//! `impl GameApp` — startup screens & main menu methods.
//!
//! This remains an `impl GameApp` module so it can share private application
//! state. Extracting an independently owned startup state is tracked by
//! clonk-org/clonk-rs#1238.

use super::*;

impl GameApp {
    pub(crate) fn loader_presentation_active(&self) -> bool {
        self.mode == AppMode::Loading
            || (self.mode == AppMode::Running && self.terminal_loader_frame_pending)
    }

    pub(crate) fn finish_terminal_loader_frame_presentation(&mut self) -> bool {
        std::mem::take(&mut self.terminal_loader_frame_pending)
    }

    pub(crate) fn arm_terminal_loader_frame_presentation(&mut self) {
        self.terminal_loader_frame_pending = !self.console_mode && self.loader_screen.is_some();
    }

    pub(crate) fn discard_terminal_loader_frame_for_headless_render(&mut self) -> bool {
        std::mem::take(&mut self.terminal_loader_frame_pending)
    }

    /// Hold a rebuilt `General.Participants` for the shutdown save.
    ///
    /// It is a `CFG_MaxString` escaped-string field, so the flush has to hand
    /// the native bytes to C++'s quoted writer; the raw writer would emit the
    /// list bare, and a LegacyClonk install sharing this file would not read it
    /// back as the same value.
    pub(crate) fn defer_participant_list(&mut self, participants: &str) {
        let Some(native) = clonk_resources::encode_legacy_script_text(participants) else {
            tracing::warn!(
                participants,
                "participant list is not representable in the classic Windows-1252 config"
            );
            return;
        };
        self.deferred_config
            .set_escaped("General", "Participants", participants, native);
    }

    pub(crate) fn console_startup_active(&self) -> bool {
        matches!(self.mode, AppMode::Menu | AppMode::Loading) && !self.console_game_active()
    }

    pub(crate) fn prepare_main_menu_slot_game(
        &mut self,
        requested_target: &Path,
        title_png: Option<&[u8]>,
    ) -> Result<Option<save_worker::PreparedNativeSave>> {
        self.prepare_native_c4_game(
            ConsoleSaveKind::Savegame,
            Some(requested_target),
            false,
            title_png,
        )
    }

    pub(crate) fn configure_native_startup_fonts(&mut self, scale: f32, point_filtering: bool) {
        match LoaderRenderConfig::new(scale, point_filtering) {
            Ok(config) => {
                let config = config.with_aspect_fill(configured_loader_aspect(
                    &load_native_config_bytes(self.app_paths.as_ref()),
                ));
                self.loader_render_config = Some(config);
                self.loader_render_error = None;
            }
            Err(error) => {
                self.loader_render_config = None;
                self.loader_render_error = Some(error.to_string());
            }
        }
        if scale <= 0.0 || !scale.is_finite() {
            self.native_startup_fonts = None;
            return;
        }
        let Some(source) = self.assets.startup_native_font_source.clone() else {
            self.native_startup_fonts = None;
            return;
        };
        let fonts = clonk_frontend::clonk_fonts::build_native_font_set_face(
            &source.bytes,
            source.face_index,
            scale,
        );
        match fonts {
            Ok(fonts) => {
                self.native_startup_fonts = Some(Arc::new(fonts));
            }
            Err(error) => {
                tracing::warn!(%error, scale, "failed to build scale-native startup fonts");
                self.native_startup_fonts = None;
            }
        }
    }

    pub(crate) fn can_defer_native_main_menu_text(&self, scale: f32) -> bool {
        self.mode == AppMode::Menu
            && self.startup_view == StartupView::MainMenu
            && !self.startup_dialog_fade_active()
            && self.message_dialogs.is_empty()
            && self.context_menu.is_none()
            && !self.startup_element_tooltip_pending()
            && self
                .native_startup_fonts
                .as_ref()
                .is_some_and(|fonts| (fonts.scale() - scale).abs() < f32::EPSILON)
    }

    pub(crate) fn can_defer_native_loader_text(&self, scale: f32) -> bool {
        self.loader_presentation_active()
            && self.message_dialogs.is_empty()
            && !self
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible)
            && scale > 0.0
            && scale.is_finite()
            && self
                .loader_render_config
                .is_some_and(|config| config.application_scale() == scale)
            && self
                .native_startup_fonts
                .as_ref()
                .is_some_and(|fonts| fonts.scale() == scale)
    }

    pub(crate) fn commit_pending_native_loader_base(&mut self, frame: &mut [u8]) {
        self.commit_pending_native_base(frame);
        self.pending_native_presentation
            .as_mut()
            .and_then(|plan| plan.batches.last_mut())
            .expect("loader base batch was just committed")
            .native_loader_text = true;
    }

    fn classic_loader_render_preconditions_ready(&self) -> bool {
        if self.loader_error.is_some()
            || self.loader_render_error.is_some()
            || self.loader_screen.is_none()
        {
            return false;
        }
        let Some(config) = self.loader_render_config else {
            return false;
        };
        config.application_scale() == 1.0
            || self
                .native_startup_fonts
                .as_ref()
                .is_some_and(|fonts| fonts.scale() == config.application_scale())
    }

    pub(crate) fn startup_network_transition_active(&self) -> bool {
        self.mode != AppMode::Running && self.startup_network_connection.is_some()
    }

    fn startup_network_join_progress_active(&self) -> bool {
        self.startup_network_connection
            .as_ref()
            .is_some_and(|connection| connection.purpose == StartupNetworkPurpose::Join)
            && self.message_dialogs.iter().any(|dialog| {
                matches!(
                    dialog.continuation,
                    MessageDialogContinuation::StartupNetworkConnectProgress
                )
            })
    }

    pub(crate) fn startup_network_transition_blocks_input(&self) -> bool {
        self.startup_network_transition_active() && !self.startup_network_join_progress_active()
    }

    pub(crate) fn startup_dialog_fade_active(&self) -> bool {
        self.mode == AppMode::Menu && self.startup_dialog_fade.is_some()
    }

    fn begin_startup_host_league_auth(
        &mut self,
        mode: NetworkMode,
        manager: NetworkManager,
        selected_scenario: Option<(String, String)>,
        purpose: StartupNetworkPurpose,
    ) -> Result<LeaguePlayerAuthStatus, EngineError> {
        let (players, server_name) = match &mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => (
                prepared
                    .pending_initial_league_players()
                    .unwrap_or_default()
                    .to_vec(),
                prepared
                    .league_config()
                    .map(|league| league_server_name(&league.endpoint))
                    .unwrap_or_default(),
            ),
            NetworkMode::Host(_) | NetworkMode::Client(_) => (Vec::new(), String::new()),
        };
        self.continue_league_player_auth(LeaguePlayerAuthContinuation::StartupHost {
            mode,
            manager,
            selected_scenario,
            purpose,
            players,
            index: 0,
            server_name,
        })
    }

    pub(crate) fn startup_game_graphics_resources(&self) -> GameGraphicsResources {
        GameGraphicsResources {
            cursor_atlas: self.assets.cursor_atlas(),
            hud_graphics: self.assets.hud_graphics(),
            options: self
                .assets
                .startup_dialog_images
                .get("Options.png")
                .cloned()
                .map(Arc::new),
            palette: self.assets.game_palette(),
            liquid_animation: self.assets.liquid_animation().map(Arc::new),
        }
    }

    pub(crate) fn startup_crew_rename_rect(&self) -> Option<clonk_frontend::classic_gui::IntRect> {
        let rename = self.startup_crew_rename.as_ref()?;
        let current = self.startup_crew_files.get(rename.index)?;
        if current.file_name != rename.file_name || current.player_path != rename.player_path {
            return None;
        }
        let dialog = self
            .startup_player_dialog
            .as_ref()
            .filter(|dialog| dialog.is_crew_mode())?;
        let layout = dialog.layout();
        let row = i32::try_from(rename.index).unwrap_or(i32::MAX);
        Some(clonk_frontend::classic_gui::IntRect::new(
            layout.list_viewport.x + (layout.item_height + 2) * 2,
            layout.list_viewport.y + layout.item_pitch.saturating_mul(row)
                - dialog.list_scroll_offset()
                + 2,
            (layout.item_width - (layout.item_height + 2) * 2 - 2).max(1),
            (layout.item_height - 4).max(1),
        ))
    }

    pub(crate) fn startup_crew_rename_char_pos(
        &self,
        point: GuiPoint,
        require_inside: bool,
    ) -> Option<usize> {
        let rename = self.startup_crew_rename.as_ref()?;
        let dialog = self.startup_player_dialog.as_ref()?;
        let layout = dialog.layout();
        let rect = self.startup_crew_rename_rect()?;
        let inside_rect = point.x >= rect.x as f32
            && point.x < (rect.x + rect.w) as f32
            && point.y >= rect.y as f32
            && point.y < (rect.y + rect.h) as f32;
        let inside_viewport = point.x >= layout.list_viewport.x as f32
            && point.x < (layout.list_viewport.x + layout.list_viewport.w) as f32
            && point.y >= layout.list_viewport.y as f32
            && point.y < (layout.list_viewport.y + layout.list_viewport.h) as f32;
        if require_inside && !(inside_rect && inside_viewport) {
            return None;
        }
        let font = &self.assets.clonk_fonts.as_deref()?.text;
        Some(rename.edit.character_at_x(point.x, rect, font))
    }

    pub(crate) fn handle_startup_crew_rename_pointer_down(&mut self, point: GuiPoint) -> bool {
        let Some(position) = self.startup_crew_rename_char_pos(point, true) else {
            return false;
        };
        let now = Instant::now();
        if let Some(rename) = self.startup_crew_rename.as_mut() {
            let double_click = rename
                .last_click
                .is_some_and(|last| now.duration_since(last) < CPP_DOUBLE_CLICK_INTERVAL);
            if double_click {
                rename.edit.select_word_at(position);
                rename.last_click = None;
                rename.ignore_pointer_up = true;
            } else {
                rename.edit.begin_pointer_selection(position);
                rename.last_click = Some(now);
                rename.ignore_pointer_up = false;
            }
        }
        true
    }

    pub(crate) fn handle_startup_crew_rename_pointer_move(&mut self, point: GuiPoint) -> bool {
        if !self
            .startup_crew_rename
            .as_ref()
            .is_some_and(|rename| rename.edit.is_dragging())
        {
            return false;
        }
        if let Some(position) = self.startup_crew_rename_char_pos(point, false) {
            if let Some(rename) = self.startup_crew_rename.as_mut() {
                rename.edit.drag_pointer_selection(position);
            }
        }
        true
    }

    pub(crate) fn handle_startup_crew_rename_pointer_up(&mut self, point: GuiPoint) -> bool {
        if self
            .startup_crew_rename
            .as_mut()
            .is_some_and(|rename| std::mem::take(&mut rename.ignore_pointer_up))
        {
            return true;
        }
        if !self
            .startup_crew_rename
            .as_ref()
            .is_some_and(|rename| rename.edit.is_dragging())
        {
            return false;
        }
        let position = self
            .startup_crew_rename_char_pos(point, false)
            .or_else(|| {
                self.startup_crew_rename
                    .as_ref()
                    .map(|rename| rename.edit.caret())
            })
            .unwrap_or(0);
        if let Some(rename) = self.startup_crew_rename.as_mut() {
            rename.edit.end_pointer_selection(position);
        }
        true
    }

    pub(crate) fn handle_startup_crew_rename_middle_down(
        &mut self,
        point: GuiPoint,
        primary: Option<&str>,
    ) -> bool {
        let Some(position) = self.startup_crew_rename_char_pos(point, true) else {
            return false;
        };
        if let Some(rename) = self.startup_crew_rename.as_mut() {
            rename.edit.begin_pointer_selection(position);
            rename.edit.end_pointer_selection(position);
            if let Some(primary) = primary {
                rename.edit.insert_text(primary);
            }
        }
        true
    }

    pub(crate) fn handle_startup_dialog_key(
        &mut self,
        key: KeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        match self.startup_view {
            StartupView::NetworkGame => {
                let actions = self
                    .startup_network_dialog
                    .as_mut()
                    .map(|dialog| match state {
                        ElementState::Pressed => dialog.handle_key_down(key),
                        ElementState::Released => dialog.handle_key_up(key),
                    })
                    .unwrap_or_default();
                self.process_network_dialog_actions(actions)?;
            }
            StartupView::PlayerSelection => {
                let actions = self
                    .startup_player_dialog
                    .as_mut()
                    .map(|dialog| match state {
                        ElementState::Pressed => dialog.handle_key_down(key),
                        ElementState::Released => dialog.handle_key_up(key),
                    })
                    .unwrap_or_default();
                self.process_player_dialog_actions(actions)?;
            }
            StartupView::Options => {
                let actions = self
                    .startup_options_dialog
                    .as_mut()
                    .map(|dialog| match state {
                        ElementState::Pressed => dialog.handle_key_down(key),
                        ElementState::Released => dialog.handle_key_up(key),
                    })
                    .unwrap_or_default();
                self.process_options_dialog_actions(actions)?;
            }
            StartupView::About => {
                let actions = self
                    .startup_about_dialog
                    .as_mut()
                    .map(|dialog| match state {
                        ElementState::Pressed => dialog.handle_key_down(key),
                        ElementState::Released => dialog.handle_key_up(key),
                    })
                    .unwrap_or_default();
                self.process_about_dialog_actions(actions)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn handle_startup_tab_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || key != VirtualKeyCode::Tab
            || !matches!(
                self.startup_view,
                StartupView::NetworkGame | StartupView::PlayerSelection | StartupView::About
            )
        {
            return Ok(false);
        }
        if self.startup_view == StartupView::PlayerSelection && self.startup_crew_rename.is_some() {
            // RenameEdit owns Tab as a focus-loss commit. Its FinishRename
            // restores the saved control and cancels this traversal.
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if self.startup_view == StartupView::NetworkGame
            && (modifiers == ModifiersState::CONTROL
                || modifiers == (ModifiersState::CONTROL | ModifiersState::SHIFT))
            && self.startup_network_dialog.as_ref().is_some_and(|dialog| {
                dialog.mode() == clonk_frontend::startup_netdlg::NetDlgMode::Chat
                    && dialog.chat_page() == clonk_frontend::startup_netdlg::NetDlgChatPage::Chats
            })
        {
            let actions = if state == ElementState::Pressed {
                self.startup_network_dialog
                    .as_mut()
                    .map(|dialog| {
                        dialog.cycle_chat_sheet(modifiers.contains(ModifiersState::SHIFT))
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            self.process_network_dialog_actions(actions)?;
            return Ok(true);
        }
        let backwards = if modifiers.is_empty() {
            false
        } else if modifiers == ModifiersState::SHIFT {
            true
        } else {
            // C4GUI binds only exact Tab and Shift+Tab. Consume every other
            // modified Tab before the modifier-blind KeyCode mapping.
            return Ok(true);
        };
        if state == ElementState::Released {
            return Ok(true);
        }
        match self.startup_view {
            StartupView::NetworkGame => {
                let actions = self
                    .startup_network_dialog
                    .as_mut()
                    .map(|dialog| {
                        dialog.handle_key_down_with_tab_direction(KeyCode::Tab, backwards)
                    })
                    .unwrap_or_default();
                self.process_network_dialog_actions(actions)?;
            }
            StartupView::PlayerSelection => {
                let actions = self
                    .startup_player_dialog
                    .as_mut()
                    .map(|dialog| {
                        dialog.handle_key_down_with_tab_direction(KeyCode::Tab, backwards)
                    })
                    .unwrap_or_default();
                self.process_player_dialog_actions(actions)?;
            }
            StartupView::About => {
                let actions = self
                    .startup_about_dialog
                    .as_mut()
                    .map(|dialog| {
                        dialog.handle_key_down_with_tab_direction(KeyCode::Tab, backwards)
                    })
                    .unwrap_or_default();
                self.process_about_dialog_actions(actions)?;
            }
            _ => unreachable!("startup Tab view checked above"),
        }
        Ok(true)
    }

    pub(crate) fn handle_startup_hotkey(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.game_over_dialog.is_some()
            || !matches!(
                self.startup_view,
                StartupView::MainMenu
                    | StartupView::PlayerSelection
                    | StartupView::Options
                    | StartupView::About
                    | StartupView::NetworkGame
            )
        {
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if modifiers != ModifiersState::ALT
            && modifiers != (ModifiersState::ALT | ModifiersState::SHIFT)
        {
            return Ok(modifiers.alt_key() && map_key_code(key).is_some());
        }
        // Plain dialog bindings compare the same exact modifier mask, so an
        // unmatched Alt key must not fall through to modifier-blind Rust
        // navigation. Key-up has no C++ mnemonic callback either.
        let suppress_plain_gui_key = map_key_code(key).is_some();
        if state != ElementState::Pressed {
            return Ok(suppress_plain_gui_key);
        }
        let Some(character) = startup_dialog_hotkey(key) else {
            return Ok(suppress_plain_gui_key);
        };
        let captured = match self.startup_view {
            StartupView::MainMenu => {
                let Some(actions) = self.main_menu_state.menu.handle_hotkey(character) else {
                    return Ok(suppress_plain_gui_key);
                };
                // Button::OnHotkey calls OnPress directly, bypassing the
                // SetDown/SetUp sounds used by pointer and focus activation.
                self.process_main_menu_actions_with_sound(actions, false)?;
                true
            }
            StartupView::PlayerSelection => {
                let Some(actions) = self
                    .startup_player_dialog
                    .as_mut()
                    .and_then(|dialog| dialog.handle_hotkey(character))
                else {
                    return Ok(suppress_plain_gui_key);
                };
                self.process_player_dialog_actions(actions)?;
                true
            }
            StartupView::Options => {
                if !self.startup_options_dialog_is_active() {
                    return Ok(suppress_plain_gui_key);
                }
                let Some(actions) = self
                    .startup_options_dialog
                    .as_mut()
                    .and_then(|dialog| dialog.handle_hotkey(character))
                else {
                    // Options has its own exact-Alt combo bindings below the
                    // mnemonic layer. Let an unmatched hotkey reach those;
                    // options_modified_gui_key_is_inert still suppresses every
                    // modifier-blind GUI fallback.
                    return Ok(false);
                };
                self.process_options_dialog_actions(actions)?;
                true
            }
            StartupView::About => {
                let Some(actions) = self
                    .startup_about_dialog
                    .as_mut()
                    .and_then(|dialog| dialog.handle_hotkey(character))
                else {
                    return Ok(suppress_plain_gui_key);
                };
                // See the Button::OnHotkey sound rule above.
                self.process_about_dialog_actions_with_sound(actions, false)?;
                true
            }
            StartupView::NetworkGame => {
                let Some(actions) = self
                    .startup_network_dialog
                    .as_mut()
                    .and_then(|dialog| dialog.handle_hotkey(character))
                else {
                    return Ok(suppress_plain_gui_key);
                };
                self.process_network_dialog_actions(actions)?;
                true
            }
            _ => unreachable!("startup mnemonic view checked above"),
        };
        Ok(captured || suppress_plain_gui_key)
    }

    pub(crate) fn handle_startup_crew_rename_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.startup_view != StartupView::PlayerSelection || self.startup_crew_rename.is_none() {
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if state == ElementState::Pressed
            && key == VirtualKeyCode::ContextMenu
            && modifiers.is_empty()
        {
            if let Some(rect) = self.startup_crew_rename_rect() {
                let anchor =
                    GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32);
                self.open_startup_crew_rename_context_menu(anchor)?;
            }
            return Ok(true);
        }
        if state == ElementState::Pressed
            && key == VirtualKeyCode::Tab
            && (modifiers.is_empty() || modifiers == ModifiersState::SHIFT)
        {
            self.commit_startup_crew_rename(true)?;
            return Ok(true);
        }
        if state == ElementState::Released {
            return Ok(true);
        }
        let ctrl = modifiers.control_key();
        let shift = modifiers.shift_key();
        let cursor_modifiers = !modifiers.alt_key();
        match key {
            VirtualKeyCode::F2 if modifiers.is_empty() => {
                let index = self
                    .startup_crew_rename
                    .as_ref()
                    .map(|rename| rename.index)
                    .expect("active crew rename remains installed");
                self.abort_startup_crew_rename();
                self.start_startup_crew_rename(index)?;
            }
            VirtualKeyCode::Escape if modifiers.is_empty() => {
                self.abort_startup_crew_rename();
            }
            VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter if modifiers.is_empty() => {
                self.commit_startup_crew_rename(false)?;
            }
            VirtualKeyCode::Backspace if cursor_modifiers => {
                if let Some(rename) = self.startup_crew_rename.as_mut() {
                    rename.edit.backspace(ctrl, shift);
                }
            }
            VirtualKeyCode::Delete if cursor_modifiers => {
                if let Some(rename) = self.startup_crew_rename.as_mut() {
                    rename.edit.delete(ctrl, shift);
                }
            }
            VirtualKeyCode::ArrowLeft if cursor_modifiers => {
                if let Some(rename) = self.startup_crew_rename.as_mut() {
                    rename
                        .edit
                        .move_cursor(RenameEditCursorOperation::Left, ctrl, shift);
                }
            }
            VirtualKeyCode::ArrowRight if cursor_modifiers => {
                if let Some(rename) = self.startup_crew_rename.as_mut() {
                    rename
                        .edit
                        .move_cursor(RenameEditCursorOperation::Right, ctrl, shift);
                }
            }
            VirtualKeyCode::Home if cursor_modifiers => {
                if let Some(rename) = self.startup_crew_rename.as_mut() {
                    rename
                        .edit
                        .move_cursor(RenameEditCursorOperation::Home, ctrl, shift);
                }
            }
            VirtualKeyCode::End if cursor_modifiers => {
                if let Some(rename) = self.startup_crew_rename.as_mut() {
                    rename
                        .edit
                        .move_cursor(RenameEditCursorOperation::End, ctrl, shift);
                }
            }
            VirtualKeyCode::KeyA if modifiers == ModifiersState::CONTROL => {
                if let Some(rename) = self.startup_crew_rename.as_mut() {
                    rename.edit.select_all();
                }
            }
            VirtualKeyCode::KeyC if modifiers == ModifiersState::CONTROL => {
                let result = self.startup_crew_rename.as_mut().map(|rename| {
                    transfer_edit_selection(&mut rename.edit, false, |selected| {
                        arboard::Clipboard::new()
                            .and_then(|mut clipboard| clipboard.set_text(selected.to_string()))
                    })
                });
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "failed to copy startup crew rename text");
                }
            }
            VirtualKeyCode::KeyX if modifiers == ModifiersState::CONTROL => {
                let result = self.startup_crew_rename.as_mut().map(|rename| {
                    transfer_edit_selection(&mut rename.edit, true, |selected| {
                        arboard::Clipboard::new()
                            .and_then(|mut clipboard| clipboard.set_text(selected.to_string()))
                    })
                });
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "failed to cut startup crew rename text");
                }
            }
            VirtualKeyCode::KeyV if modifiers == ModifiersState::CONTROL => {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                    Ok(text) => {
                        if let Some(rename) = self.startup_crew_rename.as_mut() {
                            rename.edit.insert_text(&text);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to paste startup crew rename text")
                    }
                }
            }
            _ => {}
        }
        Ok(true)
    }

    pub(crate) fn startup_options_dialog_has_focus_owner(&self) -> bool {
        self.mode == AppMode::Menu
            && self.startup_view == StartupView::Options
            && self.startup_options_dialog.is_some()
            && self.startup_options_advanced_dialog.is_none()
            && self.message_dialogs.is_empty()
            && self.context_menu.is_none()
            && self.definition_selector.is_none()
            && self.game_option_input_dialog.is_none()
            && self.league_signup_dialog.is_none()
    }

    pub(crate) fn startup_options_dialog_is_active(&self) -> bool {
        self.startup_options_dialog_has_focus_owner() && !self.startup_dialog_fade_active()
    }

    /// Mouse-region `COM_PlayerMenu` calls `C4Player::ActivateMenuMain`
    /// directly: an existing menu is reinitialized to its main page instead
    /// of following the keyboard command's open/close toggle.
    pub(crate) fn activate_ingame_main_menu_for_player(
        &mut self,
        player: i32,
    ) -> Result<(), EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(());
        }
        if player == self.local_owner {
            self.close_object_menu();
        }
        self.ingame_menu.replace(
            player,
            IngameMenuState::main_menu(
                &self.main_menu_conditions_for(player),
                &self.ingame_menu_labels(),
            ),
        );
        Ok(())
    }

    /// `C4MainMenu::ActivateMain` conditions (C4MainMenu.cpp:643-715) from
    /// the running app state.
    pub(crate) fn main_menu_conditions(&self) -> MainMenuConditions {
        self.main_menu_conditions_for(self.local_owner)
    }

    pub(crate) fn main_menu_conditions_for(&self, player: i32) -> MainMenuConditions {
        let players = &self.snapshot.players;
        MainMenuConditions {
            has_player: players.iter().any(|state| state.id == player),
            player_count: players.len(),
            max_players: self.network_max_players,
            is_league: self.network_is_league,
            network_enabled: self.network.is_some(),
            network_host: matches!(self.network_mode, Some(NetworkMode::Host(_))),
            network_has_clients: self.network.is_some(),
            is_fullscreen: self.display_flags.is_fullscreen,
            team_switch_allowed: self.engine.team_configuration().allow_team_switch,
        }
    }

    pub(crate) fn set_startup_game_music_option(
        &mut self,
        enabled: bool,
    ) -> Result<(), EngineError> {
        let audio = self.sound.context.as_ref().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup game-music option",
                },
            ))
        })?;
        let mut audio = audio.borrow_mut();
        // The startup BoolConfig writes RXMusic only. There is no running
        // game whose playback state should be changed here.
        audio.options.music_enabled = enabled;
        Ok(())
    }

    pub(crate) fn set_startup_game_sound_option(
        &mut self,
        enabled: bool,
    ) -> Result<(), EngineError> {
        let audio = self.sound.context.as_ref().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup game-sound option",
                },
            ))
        })?;
        let mut audio = audio.borrow_mut();
        audio.options.sound_enabled = enabled;
        Ok(())
    }

    pub(crate) fn set_startup_music_volume(&mut self, value: i32) -> Result<(), EngineError> {
        let audio = self.sound.context.as_ref().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup music-volume slider",
                },
            ))
        })?;
        let mut audio = audio.borrow_mut();
        audio.set_music_volume_percent(value);
        Ok(())
    }

    pub(crate) fn set_startup_sound_volume(&mut self, value: i32) -> Result<(), EngineError> {
        let audio = self.sound.context.as_ref().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup sound-volume slider",
                },
            ))
        })?;
        let mut audio = audio.borrow_mut();
        audio.set_sound_volume_percent(value);
        Ok(())
    }

    /// `Config.Voice.Enabled` from the port-only Audio-sheet row
    /// (clonk-org/clonk-rs#452). Like the classic rows this only edits the live
    /// options; the sheet's close-save writes them.
    ///
    /// A session's `NetworkManager` snapshots this flag when it is constructed
    /// (`main.rs`, `game_app::network`), so toggling it here reaches the next
    /// hosted or joined game rather than one already running -- which the
    /// startup Options dialog can never be open over.
    pub(crate) fn set_startup_voice_enabled(&mut self, enabled: bool) -> Result<(), EngineError> {
        let audio = self.sound.context.as_ref().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup voice-chat option",
                },
            ))
        })?;
        let mut audio = audio.borrow_mut();
        audio.options.voice_enabled = enabled;
        Ok(())
    }

    /// `Config.Voice.ActivationMode` from the port-only Audio-sheet checkbox
    /// (clonk-org/clonk-rs#422): checked is `VoiceActivated`, unchecked the
    /// default `PushToTalk`.
    ///
    /// The mode never opens a microphone by itself -- `Voice.Enabled` remains
    /// the single opt-in, and this sheet is a startup surface, so no capture
    /// can be running while it is toggled.
    pub(crate) fn set_startup_voice_activation_mode(
        &mut self,
        voice_activated: bool,
    ) -> Result<(), EngineError> {
        let audio = self.sound.context.as_ref().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup voice-activation option",
                },
            ))
        })?;
        let mut audio = audio.borrow_mut();
        use crate::settings::VoiceActivationMode;
        audio.options.voice_activation_mode = if voice_activated {
            VoiceActivationMode::VoiceActivated
        } else {
            VoiceActivationMode::PushToTalk
        };
        Ok(())
    }

    /// `Config.Voice.Volume` from the port-only Audio-sheet bar. Voice alone
    /// has a `0..=200` range: `100` is unity and the upper half lets quiet
    /// speech be boosted. The positional mix multiplies this in every frame,
    /// so it is live immediately.
    pub(crate) fn set_startup_voice_volume(&mut self, value: i32) -> Result<(), EngineError> {
        let audio = self.sound.context.as_ref().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup voice-volume slider",
                },
            ))
        })?;
        let mut audio = audio.borrow_mut();
        audio.options.set_voice_volume_percent(value);
        Ok(())
    }

    pub(crate) fn process_main_menu_actions(
        &mut self,
        actions: Vec<MainMenuAction>,
    ) -> Result<(), EngineError> {
        self.process_main_menu_actions_with_sound(actions, true)
    }

    fn process_main_menu_actions_with_sound(
        &mut self,
        actions: Vec<MainMenuAction>,
        play_activation_sound: bool,
    ) -> Result<(), EngineError> {
        if self.game_over_dialog.is_some() {
            return Ok(());
        }
        for action in actions {
            match action {
                MainMenuAction::SelectionChanged(_) => {
                    self.play_ui_sound("Command");
                }
                MainMenuAction::Activate(item) => {
                    if play_activation_sound {
                        self.play_ui_sound("Click");
                    }
                    self.handle_main_menu_activation(item)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn startup_network_reference_row(
        reference: &clonk_network::NetworkGameReference,
    ) -> clonk_frontend::startup_netdlg::NetDlgGameEntry {
        Self::startup_network_reference_row_with_config(
            &embedded_runtime_language_table().entries,
            false,
            reference,
        )
    }

    fn startup_network_reference_identity_eq(
        left: &clonk_network::NetworkGameReference,
        right: &clonk_network::NetworkGameReference,
    ) -> bool {
        left.host_name == right.host_name
            && if left.addresses.is_empty() || right.addresses.is_empty() {
                left.tcp_addresses
                    .iter()
                    .any(|address| right.tcp_addresses.contains(address))
            } else {
                left.addresses
                    .iter()
                    .any(|address| right.addresses.contains(address))
            }
    }

    /// The name C4StartupNetListEntry gives an advertised game: its title over
    /// the client hosting it (oracle-src-pinned 7d43b47b
    /// src/C4StartupNetDlg.cpp:454).
    pub(crate) fn startup_network_reference_title(
        resources: &HashMap<String, String>,
        reference: &clonk_network::NetworkGameReference,
    ) -> String {
        let host = if reference.host_name.is_empty() {
            "unknown"
        } else {
            reference.host_name.as_str()
        };
        format_resource_string(
            runtime_resource_text_from_table(resources, "IDS_NET_REFONCLIENT", "%s on %s"),
            &[&reference.title, host],
        )
    }

    pub(crate) fn startup_network_reference_row_with_config(
        resources: &HashMap<String, String>,
        use_alternate_server: bool,
        reference: &clonk_network::NetworkGameReference,
    ) -> clonk_frontend::startup_netdlg::NetDlgGameEntry {
        use clonk_frontend::startup_netdlg::{NetDlgGameEntry, NetDlgRowIcon, NetDlgStatusIcon};

        let title = Self::startup_network_reference_title(resources, reference);
        let goals = if reference.goals.is_empty() {
            runtime_resource_text_from_table(resources, "IDS_CTL_NOGOAL", "No game goal")
        } else {
            format!(
                "{}: {}",
                runtime_resource_text_from_table(resources, "IDS_MENU_CPGOALS", "Goals"),
                reference.goals.join(", ")
            )
        };
        let state = match reference.state.to_ascii_lowercase().as_str() {
            "none" => {
                runtime_resource_text_from_table(resources, "IDS_DESC_NOTINITED", "Not initialised")
            }
            "init" => runtime_resource_text_from_table(
                resources,
                "IDS_DESC_WAITFORHOST",
                "Waiting for host connection",
            ),
            "lobby" => runtime_resource_text_from_table(
                resources,
                "IDS_DESC_EXPECTING",
                "Awaiting participants.",
            ),
            "paused" => {
                runtime_resource_text_from_table(resources, "IDS_DESC_GAMEPAUSED", "Game is paused")
            }
            "running" => runtime_resource_text_from_table(
                resources,
                "IDS_DESC_GAMERUNNING",
                "Game is running",
            ),
            _ => runtime_resource_text_from_table(
                resources,
                "IDS_DESC_UNKNOWNGAMESTATE",
                "Game is in an unknown state",
            ),
        };
        let player_count = reference.player_names.len().to_string();
        let max_players = reference.max_players.to_string();
        let mut details = format_resource_string(
            runtime_resource_text_from_table(
                resources,
                "IDS_NET_INFOPLRSGOALDESC",
                "%d/%d players - %s - %s",
            ),
            &[&player_count, &max_players, &goals, &state],
        );
        if reference.time > 0 {
            let duration = format!(
                "{:02}:{:02}:{:02}",
                reference.time / 3_600,
                (reference.time % 3_600) / 60,
                reference.time % 60
            );
            details.push_str(" - ");
            details.push_str(&duration);
        }
        let version =
            network_game_version_string(&reference.game, reference.version, reference.build);
        let version_line = format_resource_string(
            runtime_resource_text_from_table(resources, "IDS_DESC_VERSION", "Engine version: %s"),
            &[&version],
        );
        let comment_line = format!(
            "{}: {}",
            runtime_resource_text_from_table(resources, "IDS_CTL_COMMENT", "Comment"),
            reference.comment
        );
        let players = if reference.player_names.is_empty() {
            runtime_resource_text_from_table(resources, "IDS_CTL_NONE", "none")
        } else {
            reference.player_names.join(", ")
        };
        let players_line = format!(
            "{}: {players}",
            runtime_resource_text_from_table(resources, "IDS_CTL_PLAYER", "Player")
        );

        let mut status_icons = Vec::new();
        if reference.password_needed {
            status_icons.push(NetDlgStatusIcon::PasswordNeeded);
        }
        if !reference.league_address.is_empty() {
            status_icons.push(NetDlgStatusIcon::League);
        }
        if reference.is_lobby_active() {
            status_icons.push(NetDlgStatusIcon::LobbyActive);
        }
        if reference.is_past_lobby() {
            status_icons.push(NetDlgStatusIcon::Running);
            if reference.join_allowed {
                status_icons.push(NetDlgStatusIcon::RuntimeJoin);
            }
        }
        if reference.use_fair_crew {
            status_icons.push(NetDlgStatusIcon::FairCrew);
        }
        if reference.official_server && !use_alternate_server {
            status_icons.push(NetDlgStatusIcon::OfficialServer);
        }

        NetDlgGameEntry {
            title,
            details,
            address: reference.tcp_addresses.first().map(ToString::to_string),
            joinable: reference.is_joinable(),
            extra_lines: vec![version_line, comment_line, players_line],
            status_icons,
            row_icon: NetDlgRowIcon::Scenario(reference.icon),
        }
    }

    fn startup_direct_reference_query_row(
        &self,
        query: &StartupDirectReferenceQuery,
    ) -> clonk_frontend::startup_netdlg::NetDlgGameEntry {
        self.startup_reference_query_row(
            &query.address,
            &query.state,
            "IDS_NET_QUERY_DIRECTJOIN",
            "Direct join",
        )
    }

    fn startup_discovery_reference_query_row(
        &self,
        query: &StartupDiscoveryReferenceQuery,
    ) -> clonk_frontend::startup_netdlg::NetDlgGameEntry {
        self.startup_reference_query_row(
            &query.address.to_string(),
            &query.state,
            "IDS_NET_QUERY_LOCALNET",
            "Local network",
        )
    }

    fn startup_reference_query_row(
        &self,
        address: &str,
        state: &StartupDirectReferenceQueryState,
        query_resource: &str,
        query_fallback: &str,
    ) -> clonk_frontend::startup_netdlg::NetDlgGameEntry {
        let query_name = self.runtime_resource_text(query_resource, query_fallback);
        let title_template = self.runtime_resource_text("IDS_NET_CLIENTONNET", "%s on %s");
        let title = format_resource_string(title_template, &[&query_name, address.trim()]);
        let details = match state {
            StartupDirectReferenceQueryState::Pending => {
                self.runtime_resource_text("IDS_NET_INFOQUERY", "Querying game infos...")
            }
            StartupDirectReferenceQueryState::Empty => {
                self.runtime_resource_text("IDS_NET_INFONOGAME", "No games found.")
            }
            StartupDirectReferenceQueryState::Failed(error) => error.clone(),
        };
        clonk_frontend::startup_netdlg::NetDlgGameEntry {
            title,
            details,
            address: Some(address.to_string()),
            joinable: true,
            extra_lines: Vec::new(),
            status_icons: Vec::new(),
            row_icon: match state {
                StartupDirectReferenceQueryState::Pending => {
                    clonk_frontend::startup_netdlg::NetDlgRowIcon::Query
                }
                StartupDirectReferenceQueryState::Empty => {
                    clonk_frontend::startup_netdlg::NetDlgRowIcon::QueryStatic
                }
                StartupDirectReferenceQueryState::Failed(_) => {
                    clonk_frontend::startup_netdlg::NetDlgRowIcon::Error
                }
            },
        }
    }

    fn startup_masterserver_display_name(address: &str) -> String {
        let authority = address
            .trim()
            .split_once("://")
            .map_or(address.trim(), |(_, remainder)| remainder)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .rsplit('@')
            .next()
            .unwrap_or_default();
        if let Some(ipv6) = authority.strip_prefix('[') {
            return ipv6
                .split_once(']')
                .map_or(ipv6, |(host, _)| host)
                .to_string();
        }
        authority
            .rsplit_once(':')
            .filter(|(_, port)| !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()))
            .map_or(authority, |(host, _)| host)
            .to_string()
    }

    pub(crate) fn startup_masterserver_query_entry(
        resources: &HashMap<String, String>,
        address: &str,
    ) -> clonk_frontend::startup_netdlg::NetDlgMasterserverEntry {
        let query_name = runtime_resource_text_from_table(
            resources,
            "IDS_NET_QUERY_MASTERSRV",
            "Internet server",
        );
        let server_name = Self::startup_masterserver_display_name(address);
        clonk_frontend::startup_netdlg::NetDlgMasterserverEntry {
            title: format_resource_string(
                runtime_resource_text_from_table(resources, "IDS_NET_CLIENTONNET", "%s on %s"),
                &[&query_name, &server_name],
            ),
            details: runtime_resource_text_from_table(
                resources,
                "IDS_NET_INFOQUERY",
                "Querying game infos...",
            ),
            extra_lines: Vec::new(),
            row_icon: clonk_frontend::startup_netdlg::NetDlgRowIcon::Query,
        }
    }

    pub(crate) fn startup_masterserver_reply_entry(
        resources: &HashMap<String, String>,
        address: &str,
        reply: &clonk_network::MasterserverReplyInfo,
    ) -> clonk_frontend::startup_netdlg::NetDlgMasterserverEntry {
        use clonk_frontend::startup_netdlg::{NetDlgRowIcon, NetDlgTextLine};

        let mut entry = Self::startup_masterserver_query_entry(resources, address);
        let game_count = reply.game_count.to_string();
        let player_count = reply.player_count.to_string();
        entry.details = if reply.game_count == 0 {
            runtime_resource_text_from_table(resources, "IDS_NET_INFONOGAME", "No games found.")
        } else {
            format_resource_string(
                runtime_resource_text_from_table(
                    resources,
                    "IDS_NET_INFOGAMES",
                    "%d game(s) found.",
                ),
                &[&game_count, &player_count],
            )
        };
        entry.extra_lines.clear();
        if !reply.motd.is_empty() {
            entry
                .extra_lines
                .push(NetDlgTextLine::Plain(format_resource_string(
                    runtime_resource_text_from_table(
                        resources,
                        "IDS_NET_MOTD",
                        "Message of the day: %s",
                    ),
                    &[&reply.motd],
                )));
        }
        if !reply.motd_url.is_empty() {
            entry.extra_lines.push(NetDlgTextLine::Hyperlink {
                label: reply.motd_url.clone(),
                url: reply.motd_url.clone(),
            });
        }
        entry.row_icon = NetDlgRowIcon::QueryStatic;
        entry
    }

    pub(crate) fn reset_startup_masterserver_entry(&mut self) {
        self.reset_startup_masterserver_entry_at(Instant::now());
    }

    /// `C4StartupNetListEntry::QueryReferences` arms iRequestTimeout in the same
    /// step that puts the row back on IDS_NET_INFOQUERY
    /// (src/C4StartupNetDlg.cpp:168-184,199-204).
    pub(crate) fn reset_startup_masterserver_entry_at(&mut self, now: Instant) {
        let settings = load_network_search_settings(self.app_paths.as_ref());
        let entry = Self::startup_masterserver_query_entry(
            &self.startup_tooltip_resources,
            &settings.master_server_url,
        );
        // UpdateMasterserver only keeps the row — and therefore its request
        // deadline — while MasterServerSignUp is set (src/C4StartupNetDlg.cpp:851-866).
        self.startup_masterserver_request_timeout_at = self
            .startup_network_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.config().masterserver_signup)
            .then(|| now.checked_add(clonk_network::REFERENCE_QUERY_TIMEOUT))
            .flatten();
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.set_masterserver_entry(entry);
        }
    }

    /// The masterserver branch of `C4StartupNetListEntry::Execute` re-queries
    /// without calling `UpdateText`/`UpdateSmallState`, so the labels keep the
    /// previous reply — its game count, message of the day and hyperlink — and
    /// only the icon returns to the animated `fctNetGetRef` facet. The re-query
    /// still arms `iRequestTimeout` through `QueryReferences`
    /// (src/C4StartupNetDlg.cpp:182,191-207).
    pub(crate) fn begin_startup_masterserver_requery_at(&mut self, now: Instant) {
        self.startup_masterserver_request_timeout_at =
            now.checked_add(clonk_network::REFERENCE_QUERY_TIMEOUT);
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.set_masterserver_row_icon(clonk_frontend::startup_netdlg::NetDlgRowIcon::Query);
        }
    }

    pub(crate) fn set_startup_masterserver_error(&mut self, message: String) {
        self.set_startup_masterserver_error_at(Instant::now(), message);
    }

    pub(crate) fn set_startup_masterserver_error_at(&mut self, now: Instant, message: String) {
        self.startup_masterserver_next_query_at =
            now.checked_add(clonk_network::GAME_SEARCH_INTERVAL);
        self.startup_masterserver_request_timeout_at = None;
        let settings = load_network_search_settings(self.app_paths.as_ref());
        let mut entry = Self::startup_masterserver_query_entry(
            &self.startup_tooltip_resources,
            &settings.master_server_url,
        );
        entry.details = message;
        entry.row_icon = clonk_frontend::startup_netdlg::NetDlgRowIcon::Error;
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.set_masterserver_entry(entry);
        }
    }

    fn apply_startup_masterserver_reply(
        &mut self,
        reply: clonk_network::MasterserverReplyInfo,
    ) -> Result<(), EngineError> {
        self.startup_masterserver_next_query_at =
            Instant::now().checked_add(clonk_network::GAME_SEARCH_INTERVAL);
        self.startup_masterserver_request_timeout_at = None;
        let settings = load_network_search_settings(self.app_paths.as_ref());
        let entry = Self::startup_masterserver_reply_entry(
            &self.startup_tooltip_resources,
            &settings.master_server_url,
            &reply,
        );
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.set_masterserver_entry(entry);
        }

        if self.startup_network_dialog.is_none()
            || self.startup_network_ignore_redirect
            || reply.league_server_redirect.trim().is_empty()
            || self.message_dialogs.iter().any(|dialog| {
                matches!(
                    &dialog.continuation,
                    MessageDialogContinuation::NetworkServerRedirect { .. }
                )
            })
        {
            return Ok(());
        }
        let Some(paths) = self.app_paths.as_ref() else {
            return Ok(());
        };
        let config = load_native_config_bytes(Some(paths));
        let use_alternate_server = native_config_text(&config, "Network", "UseAlternateServer")
            .as_deref()
            .map(parse_config_bool)
            .unwrap_or(false);
        let server_address = native_config_text(&config, "Network", "ServerAddress")
            .map(|address| address.trim().to_string())
            .filter(|address| !address.is_empty())
            .unwrap_or_else(|| OFFICIAL_LEAGUE_SERVER.to_string());
        let alternate_server_address =
            native_config_text(&config, "Network", "AlternateServerAddress")
                .map(|address| address.trim().to_string())
                .filter(|address| !address.is_empty())
                .unwrap_or_else(|| OFFICIAL_LEAGUE_SERVER.to_string());
        let redirect = reply.league_server_redirect.trim().to_string();
        if redirect == server_address
            || (use_alternate_server && server_address != alternate_server_address)
        {
            return Ok(());
        }

        let message = format_resource_string(
            self.runtime_resource_text(
                "IDS_NET_SERVERREDIRECTMSG",
                "The configured server is no longer active and offers the following server redirection:||%s||Do you want to switch to the new server?",
            ),
            &[&redirect],
        );
        let caption = self.runtime_resource_text("IDS_NET_SERVERREDIRECT", "Server Redirection");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(44),
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::NetworkServerRedirect { address: redirect },
        )
    }

    pub(crate) fn sync_startup_network_game_rows(&mut self) {
        let use_alternate_server =
            load_network_search_settings(self.app_paths.as_ref()).use_alternate_server;
        let mut games = self
            .startup_game_references
            .iter()
            .map(|reference| {
                Self::startup_network_reference_row_with_config(
                    &self.startup_tooltip_resources,
                    use_alternate_server,
                    reference,
                )
            })
            .collect::<Vec<_>>();
        games.extend(
            self.startup_discovery_reference_queries
                .iter()
                .map(|query| self.startup_discovery_reference_query_row(query)),
        );
        games.extend(
            self.startup_direct_reference_queries
                .iter()
                .map(|query| self.startup_direct_reference_query_row(query)),
        );
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.set_games(games);
        }
    }

    fn selected_startup_direct_reference_query_id(&self) -> Option<u64> {
        let selected = self.startup_network_dialog.as_ref()?.selected_game()?;
        let query_index = selected
            .checked_sub(self.startup_game_references.len())?
            .checked_sub(self.startup_discovery_reference_queries.len())?;
        self.startup_direct_reference_queries
            .get(query_index)
            .map(|query| query.id)
    }

    fn selected_startup_game_reference(&self) -> Option<clonk_network::NetworkGameReference> {
        let selected = self.startup_network_dialog.as_ref()?.selected_game()?;
        self.startup_game_references.get(selected).cloned()
    }

    pub(crate) fn focus_startup_game_reference(
        &mut self,
        reference: &clonk_network::NetworkGameReference,
    ) {
        let index = self
            .startup_game_references
            .iter()
            .position(|candidate| candidate == reference)
            .or_else(|| {
                self.startup_game_references.iter().position(|candidate| {
                    Self::startup_network_reference_identity_eq(candidate, reference)
                })
            });
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            if let Some(index) = index {
                let _ = dialog.focus_game(index);
            }
        }
    }

    pub(crate) fn selected_startup_discovery_reference_query_id(&self) -> Option<u64> {
        let selected = self.startup_network_dialog.as_ref()?.selected_game()?;
        let query_index = selected.checked_sub(self.startup_game_references.len())?;
        self.startup_discovery_reference_queries
            .get(query_index)
            .map(|query| query.id)
    }

    pub(crate) fn focus_startup_discovery_reference_query(&mut self, id: u64) -> bool {
        let Some(query_index) = self
            .startup_discovery_reference_queries
            .iter()
            .position(|query| query.id == id)
        else {
            return false;
        };
        let row = self.startup_game_references.len() + query_index;
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            let _ = dialog.focus_game(row);
        }
        true
    }

    pub(crate) fn focus_startup_direct_reference_query(&mut self, id: u64) -> bool {
        let Some(query_index) = self
            .startup_direct_reference_queries
            .iter()
            .position(|query| query.id == id)
        else {
            return false;
        };
        let row = self.startup_game_references.len()
            + self.startup_discovery_reference_queries.len()
            + query_index;
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            let _ = dialog.focus_game(row);
        }
        true
    }

    pub(crate) fn begin_startup_direct_reference_query(&mut self, address: String) {
        if let Some(existing) = self
            .startup_direct_reference_queries
            .iter()
            .find(|query| {
                !matches!(query.state, StartupDirectReferenceQueryState::Failed(_))
                    && query.address.eq_ignore_ascii_case(&address)
            })
            .map(|query| query.id)
        {
            self.focus_startup_direct_reference_query(existing);
            return;
        }

        self.next_startup_direct_reference_query_id =
            self.next_startup_direct_reference_query_id.wrapping_add(1);
        let id = self.next_startup_direct_reference_query_id;
        self.startup_direct_reference_queries
            .push(StartupDirectReferenceQuery {
                id,
                address: address.clone(),
                state: StartupDirectReferenceQueryState::Pending,
                expires_at: None,
            });
        let default_port = load_network_reference_port(self.app_paths.as_ref());
        let submitted = self
            .startup_game_search
            .as_ref()
            .is_some_and(|search| search.query_direct(id, address, default_port).is_ok());
        if !submitted {
            if let Some(query) = self
                .startup_direct_reference_queries
                .iter_mut()
                .find(|query| query.id == id)
            {
                query.state = StartupDirectReferenceQueryState::Failed(
                    "Unable to start direct reference query".to_string(),
                );
                query.expires_at = Instant::now().checked_add(STARTUP_NETWORK_QUERY_ERROR_LIFETIME);
            }
        }
        self.sync_startup_network_game_rows();
        self.focus_startup_direct_reference_query(id);
    }

    pub(crate) fn startup_network_join_target(
        &self,
        index: usize,
    ) -> Option<StartupNetworkJoinTarget> {
        if let Some(reference) = self.startup_game_references.get(index) {
            return Some(StartupNetworkJoinTarget::Reference(reference.clone()));
        }
        let query_index = index.checked_sub(self.startup_game_references.len())?;
        if let Some(query) = self.startup_discovery_reference_queries.get(query_index) {
            return Some(match &query.state {
                StartupDirectReferenceQueryState::Failed(error) => {
                    StartupNetworkJoinTarget::QueryError(error.clone())
                }
                StartupDirectReferenceQueryState::Pending
                | StartupDirectReferenceQueryState::Empty => {
                    StartupNetworkJoinTarget::DirectAddress(query.address.to_string())
                }
            });
        }
        let query = self
            .startup_direct_reference_queries
            .get(query_index.checked_sub(self.startup_discovery_reference_queries.len())?)?;
        Some(match &query.state {
            StartupDirectReferenceQueryState::Failed(error) => {
                StartupNetworkJoinTarget::QueryError(error.clone())
            }
            StartupDirectReferenceQueryState::Pending | StartupDirectReferenceQueryState::Empty => {
                StartupNetworkJoinTarget::DirectAddress(query.address.clone())
            }
        })
    }

    pub(crate) fn begin_startup_discovery_reference_query(&mut self, address: SocketAddr) {
        if self
            .startup_discovery_reference_queries
            .iter()
            .any(|query| {
                query.address == address
                    && !matches!(query.state, StartupDirectReferenceQueryState::Failed(_))
            })
        {
            return;
        }
        let selected_direct_query = self.selected_startup_direct_reference_query_id();
        self.next_startup_direct_reference_query_id =
            self.next_startup_direct_reference_query_id.wrapping_add(1);
        let id = self.next_startup_direct_reference_query_id;
        self.startup_discovery_reference_queries
            .push(StartupDiscoveryReferenceQuery {
                id,
                address,
                state: StartupDirectReferenceQueryState::Pending,
                expires_at: None,
            });
        self.sync_startup_network_game_rows();
        if let Some(id) = selected_direct_query {
            self.focus_startup_direct_reference_query(id);
        }
    }

    pub(crate) fn finish_startup_discovery_reference_query(
        &mut self,
        address: SocketAddr,
        references: Vec<clonk_network::NetworkGameReference>,
        resolved_reference: bool,
    ) {
        let selected_reference = self.selected_startup_game_reference();
        let selected_direct_query = self.selected_startup_direct_reference_query_id();
        let selected_discovery_query = self.selected_startup_discovery_reference_query_id();
        let Some(query_index) =
            self.startup_discovery_reference_queries
                .iter()
                .rposition(|query| {
                    query.address == address
                        && !matches!(query.state, StartupDirectReferenceQueryState::Failed(_))
                })
        else {
            return;
        };
        let query_id = self.startup_discovery_reference_queries[query_index].id;
        self.startup_game_references = references;
        if resolved_reference {
            self.startup_discovery_reference_queries.remove(query_index);
        } else {
            self.startup_discovery_reference_queries[query_index].state =
                StartupDirectReferenceQueryState::Empty;
            self.startup_discovery_reference_queries[query_index].expires_at =
                Instant::now().checked_add(STARTUP_NETWORK_QUERY_ERROR_LIFETIME);
        }
        // Unlike NRQT_DirectJoin, NRQT_GameDiscovery never explicitly selects
        // a returned reference; preserve the controller's existing selection.
        self.sync_startup_network_game_rows();
        if let Some(reference) = selected_reference.as_ref() {
            self.focus_startup_game_reference(reference);
        } else if let Some(id) = selected_direct_query {
            self.focus_startup_direct_reference_query(id);
        } else if let Some(selected_id) = selected_discovery_query {
            if selected_id != query_id {
                self.focus_startup_discovery_reference_query(selected_id);
            }
        }
    }

    pub(crate) fn fail_startup_discovery_reference_query(
        &mut self,
        address: SocketAddr,
        message: String,
    ) {
        let expires_at = Instant::now().checked_add(STARTUP_NETWORK_QUERY_ERROR_LIFETIME);
        if let Some(query) = self
            .startup_discovery_reference_queries
            .iter_mut()
            .rev()
            .find(|query| {
                query.address == address
                    && !matches!(query.state, StartupDirectReferenceQueryState::Failed(_))
            })
        {
            query.state = StartupDirectReferenceQueryState::Failed(message);
            query.expires_at = expires_at;
        } else {
            // Started is queued before every result. Missing means this is a
            // stale duplicate completion whose same-address row was already
            // resolved or cleared; native suppresses duplicate live queries.
            return;
        }
        self.sync_startup_network_game_rows();
    }

    pub(crate) fn finish_startup_direct_reference_query(
        &mut self,
        request_id: u64,
        references: Vec<clonk_network::NetworkGameReference>,
        selected_index: Option<usize>,
    ) {
        let selected_reference = self.selected_startup_game_reference();
        let selected_query = self.selected_startup_direct_reference_query_id();
        let selected_discovery_query = self.selected_startup_discovery_reference_query_id();
        let Some(query_index) = self
            .startup_direct_reference_queries
            .iter()
            .position(|query| query.id == request_id)
        else {
            return;
        };
        self.startup_game_references = references;
        if let Some(selected_index) = selected_index {
            self.startup_direct_reference_queries.remove(query_index);
            self.sync_startup_network_game_rows();
            match selected_query {
                Some(id) if id == request_id => {
                    if let Some(dialog) = self.startup_network_dialog.as_mut() {
                        let _ = dialog.focus_game(selected_index);
                    }
                }
                Some(id) => {
                    self.focus_startup_direct_reference_query(id);
                }
                None => {}
            }
        } else {
            self.startup_direct_reference_queries[query_index].state =
                StartupDirectReferenceQueryState::Empty;
            self.startup_direct_reference_queries[query_index].expires_at =
                Instant::now().checked_add(STARTUP_NETWORK_QUERY_ERROR_LIFETIME);
            self.sync_startup_network_game_rows();
            if let Some(id) = selected_query {
                self.focus_startup_direct_reference_query(id);
            }
        }
        if selected_query.is_none() {
            if let Some(reference) = selected_reference.as_ref() {
                self.focus_startup_game_reference(reference);
            } else if let Some(id) = selected_discovery_query {
                self.focus_startup_discovery_reference_query(id);
            }
        }
    }

    fn fail_startup_direct_reference_query(&mut self, request_id: u64, message: String) {
        let selected_query = self.selected_startup_direct_reference_query_id();
        let Some(query) = self
            .startup_direct_reference_queries
            .iter_mut()
            .find(|query| query.id == request_id)
        else {
            return;
        };
        query.state = StartupDirectReferenceQueryState::Failed(message);
        query.expires_at = Instant::now().checked_add(STARTUP_NETWORK_QUERY_ERROR_LIFETIME);
        self.sync_startup_network_game_rows();
        if let Some(id) = selected_query {
            self.focus_startup_direct_reference_query(id);
        }
    }

    pub(crate) fn tick_startup_network_query_rows_at(&mut self, now: Instant) {
        let selected_query = self.selected_startup_direct_reference_query_id();
        let selected_discovery_query = self.selected_startup_discovery_reference_query_id();
        let query_count = self.startup_discovery_reference_queries.len()
            + self.startup_direct_reference_queries.len();
        self.startup_discovery_reference_queries
            .retain(|query| query.expires_at.is_none_or(|expires_at| now < expires_at));
        self.startup_direct_reference_queries
            .retain(|query| query.expires_at.is_none_or(|expires_at| now < expires_at));
        if self.startup_discovery_reference_queries.len()
            + self.startup_direct_reference_queries.len()
            != query_count
        {
            self.sync_startup_network_game_rows();
            if let Some(id) = selected_query {
                self.focus_startup_direct_reference_query(id);
            } else if let Some(id) = selected_discovery_query {
                self.focus_startup_discovery_reference_query(id);
            }
        }

        let masterserver_enabled = self
            .startup_network_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.config().masterserver_signup);
        if !masterserver_enabled {
            return;
        }
        if self
            .startup_masterserver_next_query_at
            .is_some_and(|next_query_at| now >= next_query_at)
        {
            // C4StartupNetListEntry::Execute clears iTimeout, re-queries and
            // returns before the iRequestTimeout check, so the fresh request
            // gets its full deadline (src/C4StartupNetDlg.cpp:191-207).
            self.startup_masterserver_next_query_at = None;
            self.begin_startup_masterserver_requery_at(now);
        } else if self
            .startup_masterserver_request_timeout_at
            .is_some_and(|timeout_at| now >= timeout_at)
        {
            // A request still outstanding after C4NetRefRequestTimeout is
            // cancelled and reported, and TT_RefReqWait schedules the next
            // masterserver query (src/C4StartupNetDlg.cpp:216-223,525).
            let message = self
                .runtime_resource_text("IDS_NET_ERR_REFREQTIMEOUT", "Reference request timed out");
            self.set_startup_masterserver_error_at(now, message);
        }
    }

    fn show_startup_discovery_error(&mut self, detail: &str) -> Result<(), EngineError> {
        let message = format_resource_string(
            self.runtime_resource_text("IDS_NET_NODISCOVERY_DESC", "Search failed: %s"),
            &[detail],
        );
        let caption = self.runtime_resource_text("IDS_NET_NODISCOVERY", "Search Error");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::None,
        )
    }

    pub(crate) fn request_startup_network_refresh(&mut self) -> Result<(), EngineError> {
        self.request_startup_network_refresh_at(Instant::now())
    }

    pub(crate) fn request_startup_network_refresh_at(
        &mut self,
        now: Instant,
    ) -> Result<(), EngineError> {
        if self.startup_network_last_refresh.is_some_and(|last| {
            now.saturating_duration_since(last) < STARTUP_NETWORK_MIN_REFRESH_INTERVAL
        }) {
            self.play_ui_sound("Error");
            return Ok(());
        }

        self.startup_network_last_refresh = Some(now);
        let masterserver_enabled = self
            .startup_network_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.config().masterserver_signup);
        self.startup_masterserver_next_query_at = if masterserver_enabled {
            now.checked_add(clonk_network::GAME_SEARCH_INTERVAL)
        } else {
            None
        };
        self.startup_game_references.clear();
        self.startup_discovery_reference_queries.clear();
        self.startup_direct_reference_queries.clear();
        self.netdlg_last_click = None;
        self.netdlg_join_edit_last_click = None;
        self.netdlg_edit_consumed_keys.clear();
        self.sync_startup_network_game_rows();
        self.reset_startup_masterserver_entry();
        self.status_text.clear();

        self.startup_network_refresh_waiting_for_clear = true;
        let refresh_error = match self.startup_game_search.as_ref() {
            Some(search) => {
                // The old generation may already have results queued. C++
                // deletes those clients synchronously; discard their queued
                // projections before asking the worker for a fresh Cleared.
                search.events().try_iter().for_each(drop);
                search.refresh().err().map(|error| error.to_string())
            }
            None => Some("network game search is not running".to_string()),
        };
        if let Some(error) = refresh_error {
            self.startup_network_refresh_waiting_for_clear = false;
            self.show_startup_discovery_error(&error)?;
        }
        Ok(())
    }

    fn sync_startup_irc_projection(
        &mut self,
        snapshot: clonk_frontend::startup_netdlg::NetDlgChatSnapshot,
    ) {
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.sync_chat_snapshot(snapshot.clone());
        }
        if let Some(dialog) = self.chat.external_dialog.as_mut() {
            dialog.sync_chat_snapshot(snapshot);
        }
    }

    pub(crate) fn startup_irc_client_active(&self) -> bool {
        self.chat
            .client
            .as_ref()
            .is_some_and(clonk_network::IrcClientHandle::is_active)
    }

    fn startup_irc_chat_visible(&self) -> bool {
        self.chat.external_dialog_visible
            || (self.mode == AppMode::Menu
                && self.startup_view == StartupView::NetworkGame
                && self.startup_network_dialog.as_ref().is_some_and(|dialog| {
                    dialog.mode() == clonk_frontend::startup_netdlg::NetDlgMode::Chat
                }))
    }

    fn show_startup_irc_error(&mut self, message: String) -> Result<(), EngineError> {
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

    pub(crate) fn show_startup_irc_validation_error(
        &mut self,
        field: clonk_frontend::startup_netdlg::NetDlgChatLoginField,
    ) -> Result<(), EngineError> {
        let (key, fallback) = match field {
            clonk_frontend::startup_netdlg::NetDlgChatLoginField::Nick => {
                ("IDS_ERR_INVALIDNICKNAME", "Invalid nickname.")
            }
            clonk_frontend::startup_netdlg::NetDlgChatLoginField::Password => (
                "IDS_ERR_INVALIDPASSWORDMAX31CHARA",
                "Invalid password. Maximum 31 characters. No spaces allowed.",
            ),
            clonk_frontend::startup_netdlg::NetDlgChatLoginField::RealName => return Ok(()),
            clonk_frontend::startup_netdlg::NetDlgChatLoginField::Channel => {
                ("IDS_ERR_INVALIDCHANNELNAME", "Invalid channel name.")
            }
        };
        self.show_startup_irc_error(self.runtime_resource_text(key, fallback))
    }

    fn show_startup_irc_connect_failure(&mut self, error: &str) -> Result<(), EngineError> {
        let template =
            self.runtime_resource_text("IDS_ERR_IRCCONNECTIONFAILED", "IRC connection failed: %s");
        self.show_startup_irc_error(format_resource_string(template, &[error]))
    }

    pub(crate) fn request_startup_irc_disconnect_confirmation(
        &mut self,
    ) -> Result<(), EngineError> {
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                self.runtime_resource_text(
                    "IDS_MSG_DISCONNECTFROMSERVER",
                    "Disconnect from server?",
                ),
                self.runtime_resource_text("IDS_DLG_CHAT", "Chat"),
                clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::StartupIrcDisconnectConfirm,
        )
    }

    pub(crate) fn sync_startup_irc_snapshot(&mut self) {
        let Some(client) = self.chat.client.as_ref() else {
            return;
        };
        let snapshot = if self.startup_irc_chat_visible() {
            client.snapshot_and_mark_message_log_read()
        } else {
            client.snapshot()
        };
        let snapshot = project_startup_irc_snapshot(&self.chat.server, snapshot);
        self.sync_startup_irc_projection(snapshot);
    }

    pub(crate) fn request_startup_irc_connection(
        &mut self,
        login: clonk_frontend::startup_netdlg::NetDlgChatLogin,
    ) -> Result<(), EngineError> {
        // C4ChatControl::OnConnectBtn stores these three editable values
        // before the disclaimer. Server2 stays config-owned and the password
        // remains transient even when the user cancels the warning.
        if let Some(paths) = self.app_paths.as_ref() {
            if let Err(error) = persist_irc_login_settings(paths, &login) {
                tracing::warn!(%error, "failed to persist IRC login settings");
                self.status_text = format!("Unable to save IRC login settings: {error}");
            }
        }

        if load_irc_settings(self.app_paths.as_ref()).hide_dangerous_warning {
            return self.connect_startup_irc(login);
        }

        let message = format_resource_string(
            self.runtime_resource_text(
                "IDS_MSG_YOUAREABOUTTOCONNECTTOAPU",
                "You are about to connect to a public chat server (%s). RedWolf Design cannot assume liability for the contents of any public chat. Additional rules can be found as part of the server messages in the server channel. Proceed?",
            ),
            &[&login.server],
        );
        let caption = self.runtime_resource_text("IDS_MSG_CHATDISCLAIMER", "Chat - Disclaimer");
        let checkbox = self.runtime_resource_text(
            "IDS_MSG_DONTSHOW",
            "&Don't display this message in the future.",
        );
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL,
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            )
            .with_checkbox(checkbox, false),
            MessageDialogContinuation::StartupIrcConnectWarning { login },
        )?;
        Ok(())
    }

    pub(crate) fn connect_startup_irc(
        &mut self,
        login: clonk_frontend::startup_netdlg::NetDlgChatLogin,
    ) -> Result<(), EngineError> {
        if let Some(mut client) = self.chat.client.take() {
            if let Err(error) = client.close() {
                tracing::warn!(%error, "failed to close the previous IRC connection");
            }
        }
        self.chat.initial_connect_pending = false;

        self.chat.server.clone_from(&login.server);
        self.sync_startup_irc_projection(clonk_frontend::startup_netdlg::NetDlgChatSnapshot {
            server: login.server.clone(),
            nick: login.nick.clone(),
            ..Default::default()
        });
        let encode = |text: &str| {
            encode_startup_irc_text(text).ok_or_else(|| EngineError::ClassicMenuParityBoundary {
                detail: "validated IRC text is not representable in the native byte charset"
                    .to_string(),
            })
        };
        let mut config = clonk_network::IrcConnectConfig::new(
            login.server.clone(),
            encode(&login.nick)?,
            encode(&login.real_name)?,
        );
        config.password = if login.password.is_empty() {
            None
        } else {
            Some(encode(&login.password)?)
        };
        config.auto_join = if login.channel.is_empty() {
            None
        } else {
            Some(encode(&login.channel)?)
        };
        config.status_templates = self.localized_irc_status_templates();
        match clonk_network::IrcClientHandle::connect(config) {
            Ok(client) => {
                self.chat.initial_connect_pending = true;
                self.chat.client = Some(client);
                self.sync_startup_irc_snapshot();
            }
            Err(error) => {
                let error = error.to_string();
                let snapshot = clonk_frontend::startup_netdlg::NetDlgChatSnapshot {
                    server: login.server,
                    nick: login.nick,
                    ..Default::default()
                };
                self.sync_startup_irc_projection(snapshot);
                self.show_irc_login_on_all_controllers();
                self.show_startup_irc_connect_failure(&error)?;
            }
        }
        Ok(())
    }

    pub(crate) fn disconnect_startup_irc(&mut self) {
        self.chat.initial_connect_pending = false;
        let Some(mut client) = self.chat.client.take() else {
            return;
        };
        let close_error = client.close().err().map(|error| error.to_string());
        let mut snapshot = project_startup_irc_snapshot(&self.chat.server, client.snapshot());
        if close_error.is_some() {
            snapshot.last_error = close_error;
        }
        self.sync_startup_irc_projection(snapshot);
    }

    pub(crate) fn dispatch_startup_irc_command(
        &mut self,
        command: clonk_frontend::startup_netdlg::NetDlgChatCommand,
    ) {
        let Some(command) = project_startup_irc_command(command) else {
            return;
        };
        let result = self
            .chat
            .client
            .as_ref()
            .ok_or(clonk_network::IrcClientError::NotConnected)
            .and_then(|client| client.queue_command(command));
        if let Err(error) = result {
            tracing::warn!(%error, "failed to queue IRC command");
            if let Some(client) = self.chat.client.as_ref() {
                let snapshot = if self.startup_irc_chat_visible() {
                    client.snapshot_and_mark_message_log_read()
                } else {
                    client.snapshot()
                };
                let mut snapshot = project_startup_irc_snapshot(&self.chat.server, snapshot);
                snapshot.last_error = Some(error.to_string());
                self.sync_startup_irc_projection(snapshot);
            }
        }
    }

    pub(crate) fn poll_startup_irc(&mut self) -> Result<(), EngineError> {
        let events = self
            .chat
            .client
            .as_ref()
            .map(|client| client.events().try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        let changed = !events.is_empty();
        let mut initial_failure = None;
        for event in events {
            match event {
                clonk_network::IrcClientEvent::Connected => {
                    self.chat.initial_connect_pending = false;
                }
                clonk_network::IrcClientEvent::Disconnected { reason }
                    if self.chat.initial_connect_pending =>
                {
                    self.chat.initial_connect_pending = false;
                    initial_failure = Some(reason);
                }
                clonk_network::IrcClientEvent::Closed => {
                    self.chat.initial_connect_pending = false;
                }
                clonk_network::IrcClientEvent::Notification
                | clonk_network::IrcClientEvent::Disconnected { .. } => {}
            }
        }
        if changed {
            self.sync_startup_irc_snapshot();
        }
        if let Some(error) = initial_failure {
            self.chat.client = None;
            self.show_irc_login_on_all_controllers();
            self.show_startup_irc_connect_failure(&error)?;
        }
        Ok(())
    }

    pub(crate) fn begin_startup_network_connection(
        &mut self,
        receiver: Receiver<StartupNetworkResult>,
        purpose: StartupNetworkPurpose,
        selected_scenario: Option<(String, String)>,
        join_target: Option<StartupJoinTarget>,
    ) -> Result<(), EngineError> {
        self.install_startup_network_connection(
            StartupNetworkConnection::new(receiver, selected_scenario, purpose),
            join_target,
        )
    }

    pub(crate) fn begin_cancellable_startup_network_connection(
        &mut self,
        receiver: Receiver<StartupNetworkResult>,
        attempt: StartupNetworkAttempt,
        purpose: StartupNetworkPurpose,
        selected_scenario: Option<(String, String)>,
        join_target: Option<StartupJoinTarget>,
    ) -> Result<(), EngineError> {
        self.install_startup_network_connection(
            StartupNetworkConnection::new(receiver, selected_scenario, purpose)
                .with_attempt(attempt),
            join_target,
        )
    }

    fn install_startup_network_connection(
        &mut self,
        connection: StartupNetworkConnection,
        join_target: Option<StartupJoinTarget>,
    ) -> Result<(), EngineError> {
        let purpose = connection.purpose;
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            // Transition guards suppress subsequent input, so release every
            // net-dialog press/capture before installing that guard.
            dialog.cancel_interaction();
        }
        if purpose == StartupNetworkPurpose::Join {
            let target = join_target
                .filter(|target| !target.is_blank())
                .unwrap_or_else(|| StartupJoinTarget::Addresses("network game".to_string()));
            let (resource_key, fallback, name) = target.message_parts();
            let message =
                format_resource_string(self.runtime_resource_text(resource_key, fallback), &[name]);
            let caption = self.runtime_resource_text("IDS_NET_JOINGAME", "Joining network game");
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::new(
                    message,
                    caption,
                    clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                    clonk_frontend::message_dialog::MessageDialogIcon::Standard(3),
                    clonk_frontend::message_dialog::MessageDialogSize::Regular,
                    false,
                ),
                MessageDialogContinuation::StartupNetworkConnectProgress,
            )?;
        }
        self.startup_network_connection = Some(connection);
        if purpose == StartupNetworkPurpose::StagedHost {
            let initial_fonts = self
                .staged_network_host_scenario
                .as_ref()
                .and_then(|staged| staged.loader_screen.as_ref())
                .map(|loader| loader.resources().fonts().clone());
            let initial_tooltip = self
                .staged_network_host_scenario
                .as_ref()
                .map(|staged| staged.loader_initial_tooltip_font.clone());
            let initial_native_source = self
                .staged_network_host_scenario
                .as_ref()
                .and_then(|staged| staged.loader_initial_native_font_source.clone());
            if let (Some(fonts), Some(tooltip)) = (initial_fonts, initial_tooltip) {
                self.install_active_classic_fonts(fonts, Some(tooltip), initial_native_source);
            }
            // C4Game opens the scenario and installs its loader before
            // InitNetworkHost. Show that exact full loader while the socket
            // worker starts; the lobby later retains only its background.
            self.cancel_underlying_interaction();
            self.replace_startup_view(StartupView::NetworkGame);
            if let Some(mut loader) = self
                .staged_network_host_scenario
                .as_mut()
                .and_then(|staged| staged.loader_screen.take())
            {
                // OpenScenario sets 4 before InitNetworkHost and the loader
                // reads that retained game progress on its first draw
                // (src/C4Game.cpp:124-270,421-440).
                loader.update(LoaderUpdate::SetProgress(4));
                self.loader_screen = Some(loader);
                self.loader_error = None;
            }
            self.status_text.clear();
            self.mode = AppMode::Loading;
        } else {
            self.status_text.clear();
        }
        Ok(())
    }

    pub(crate) fn dismiss_startup_network_connect_progress(&mut self) {
        let Some(index) = self.message_dialogs.iter().rposition(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::StartupNetworkConnectProgress
            )
        }) else {
            return;
        };
        self.remove_message_dialog_at(index);
    }

    pub(crate) fn finish_startup_network_failure(
        &mut self,
        purpose: StartupNetworkPurpose,
        message: String,
    ) -> Result<(), EngineError> {
        tracing::error!(?purpose, error = %message, "startup network session failed");
        self.startup_restart_diagnostics.mark_quit_with_error();
        self.startup_restart_diagnostics.add_fatal_error(message);
        self.finish_startup_network_restart(purpose)
    }

    pub(crate) fn present_startup_restart_diagnostics(&mut self) -> Result<(), EngineError> {
        self.status_text.clear();
        let caption = self.runtime_resource_text("IDS_DLG_LOG", "Error Log");
        self.runtime_client_list = None;
        self.runtime_client_list_consumed_keys.clear();
        self.runtime_client_list_above_game_over = false;
        let presentation = self
            .startup_restart_diagnostics
            .take_presentation()
            .expect("startup restart is entered only after an error flag or fatal diagnostic");
        let entries = match presentation {
            StartupRestartPresentation::Fatal(message) => {
                return self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                        message,
                        caption,
                        clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                    ),
                    MessageDialogContinuation::None,
                );
            }
            StartupRestartPresentation::Empty => {
                return self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                        "(no error)",
                        caption,
                        clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                    ),
                    MessageDialogContinuation::None,
                );
            }
            StartupRestartPresentation::Ringbuffer(entries) => entries,
        };

        Self::guard_gui_overlay_result(
            "C4GUI::InfoDialog",
            self.assets
                .static_info_dialog_resources()
                .context("exact C4GUI::InfoDialog resource set is absent")
                .and_then(|resources| resources.validate()),
        )?;
        let text = entries.join("|");
        let close_label = self.runtime_resource_text("IDS_DLG_CLOSE", "&Close");
        self.cancel_underlying_interaction();
        self.runtime_client_list = Some(
            clonk_frontend::runtime_client_list::RuntimeClientListDialog::new_static_info(
                caption,
                10,
                &text,
                close_label,
            ),
        );
        Ok(())
    }

    pub(crate) fn finish_startup_network_restart(
        &mut self,
        purpose: StartupNetworkPurpose,
    ) -> Result<(), EngineError> {
        // Failed C4Game::Init returns through QuitGame and constructs the
        // remembered startup dialog again in the same process. Clear every
        // partially installed network-game projection before doing likewise.
        self.restore_startup_fonts();
        self.restore_startup_gui_sheets();
        self.active_global_gui_failures.clear();
        self.clear_lobby_preload();
        self.classic_host_lobby = None;
        self.network_lobby = None;
        self.network_start_wait = None;
        self.staged_network_host_scenario = None;
        self.network_lobby_min_players = None;
        self.reinitialize_startup_loader_screen();
        self.abandon_live_masterserver_signup();
        self.clear_pending_league_player_auth();
        self.network_game_advertiser = None;
        self.advertised_game_reference = None;
        self.host_reference_paused = false;
        self.runtime_network_control_mode = None;
        self.runtime_network_committed_control_mode = None;
        self.runtime_network_committed_status = None;
        self.runtime_network_join_allowed = None;
        self.network = None;
        self.network_mode = None;
        self.host_join_snapshot = None;
        self.pending_runtime_dynamic_request = None;
        self.host_lobby_countdown = None;
        self.pending_local_lobby_countdown_echoes.clear();
        self.network_ticks.clear();
        self.network_sync.clear();
        self.offline_control_input.clear();
        self.sync_checks.clear();
        self.network_control_clock = None;
        self.host_local_alternate_colors_by_resource.clear();
        self.host_local_player_info_ids.clear();
        self.network_is_league = false;
        self.network_league_name.clear();
        self.network_stream_address = LegacyCString::default();
        self.control_player_infos = ControlPlayerInfoRegistry::default();
        self.clear_blocking_resource_wait();
        self.admission_resources.clear();
        seed_engine_player_info_parameters(
            &mut self.engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        self.network_control_running = true;
        self.runtime_network_status_barrier = None;
        self.control_clients = initial_control_clients(None, None);
        self.network_client_activity.clear();
        self.pending_network_join = None;
        self.pending_network_join_data = None;
        self.pending_round_restart_join_data = false;
        self.initial_lobby_status_ack_pending = false;
        self.client_start_barrier = ClientStartBarrier::default();
        self.pending_client_start_status = None;
        self.client_combined_scenario_path = None;
        self.client_combined_preload_file.clear();
        self.network_material_resource_groups = None;
        self.loading_state = None;
        self.active_scenario = None;
        self.active_definition_load = None;
        self.active_description_definition_modules.clear();
        self.mode = AppMode::Menu;
        self.status_text.clear();

        // C4Application::QuitGame returns an ordinary failed startup host/join
        // through PreInit before C4Startup::DoStartup reconstructs the
        // remembered dialog. Explicit command-line and developer-console
        // starts have no such second application generation.
        self.resume_startup_music_after_failed_open_game();

        match purpose {
            StartupNetworkPurpose::StagedHost => {
                self.startup_game_search = None;
                self.startup_network_last_refresh = None;
                self.startup_masterserver_next_query_at = None;
                self.startup_masterserver_request_timeout_at = None;
                self.startup_network_dialog = None;
                self.restore_startup_dialog(StartupDialog::ScenarioBrowser(
                    ScenarioSelectorMode::NetworkHost,
                ));
            }
            StartupNetworkPurpose::Join => {
                self.restore_startup_dialog(StartupDialog::NetworkGame);
            }
        }
        // NetDlg discovery startup has its own native row presentation. It
        // must not displace the fatal diagnostic with a generic overlay.
        self.present_startup_restart_diagnostics()
    }

    /// Issues the next reconnect to a restarting host, or ends the attempt when
    /// the window it announced has closed.
    pub(crate) fn poll_pending_host_rejoin(&mut self) -> Result<(), EngineError> {
        // The notice is armed while the host is still connected, and a host
        // that announces a restart it then abandons must cost this client
        // nothing. Only the session actually going away starts the clock.
        if self.network.is_some() {
            return Ok(());
        }
        let now = Instant::now();
        let Some(rejoin) = self.pending_host_rejoin.as_ref() else {
            return Ok(());
        };
        if now >= rejoin.deadline {
            let targets = startup_network_connect_targets(&rejoin.settings);
            self.pending_host_rejoin = None;
            return self.finish_startup_network_failure(
                StartupNetworkPurpose::Join,
                format!("The restarting host at {targets} did not come back in time"),
            );
        }
        if rejoin.next_attempt_at.is_some_and(|next| now < next) {
            return Ok(());
        }
        let settings = rejoin.settings.clone();
        if let Some(rejoin) = self.pending_host_rejoin.as_mut() {
            rejoin.next_attempt_at = Some(now + HOST_REJOIN_RETRY_INTERVAL);
        }
        self.pending_network_join = Some(settings);
        self.launch_pending_network_join()
    }

    /// Absorbs a reconnect failure that a still-open rejoin window should
    /// retry rather than report. Answers whether it did.
    ///
    /// A window that closed while this attempt was in flight is dropped here
    /// rather than deferred, so the failure below is reported once instead of
    /// being followed next frame by a second teardown from the expiry branch.
    fn defer_pending_host_rejoin(&mut self) -> bool {
        let now = Instant::now();
        let Some(rejoin) = self.pending_host_rejoin.as_mut() else {
            return false;
        };
        if now >= rejoin.deadline {
            self.pending_host_rejoin = None;
            return false;
        }
        rejoin.next_attempt_at = Some(now + HOST_REJOIN_RETRY_INTERVAL);
        true
    }

    pub(crate) fn poll_startup_network_connection(&mut self) -> Result<(), EngineError> {
        let Some(connection) = self.startup_network_connection.as_ref() else {
            return self.poll_pending_host_rejoin();
        };
        let selected_scenario = connection.selected_scenario.clone();
        let purpose = connection.purpose;
        let result = match connection
            .receiver
            .as_ref()
            .expect("installed startup network connection retains its receiver")
            .try_recv()
        {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => None,
        };
        let mut connection = self
            .startup_network_connection
            .take()
            .expect("completed startup network connection remains installed");
        let mut authenticated_league_players = connection.authenticated_league_players.take();
        connection.finish_attempt();
        if purpose == StartupNetworkPurpose::Join {
            // Resolution owns silent dismissal. Finishing the dialog would
            // run its continuation and incorrectly turn success into abort.
            self.dismiss_startup_network_connect_progress();
        }
        let Some(result) = result else {
            let message = "network worker disconnected before reporting readiness";
            if self.defer_pending_host_rejoin() {
                tracing::info!(
                    error = message,
                    "reconnect to the restarting host ended; retrying"
                );
                return Ok(());
            }
            tracing::error!(?purpose, error = message, "startup network session failed");
            self.startup_restart_diagnostics.add_log_entry(message);
            self.startup_restart_diagnostics.mark_quit_with_error();
            return self.finish_startup_network_restart(purpose);
        };
        match result {
            Ok((mut mode, mut manager)) => {
                // Whatever this session turns out to be, the reconnect that a
                // restart notice asked for is over. An armed window left
                // running would later expire under a live lobby.
                self.pending_host_rejoin = None;
                if let Some(response) = manager.take_league_start_response() {
                    if let NetworkMode::Host(HostSettings {
                        prepared: Some(prepared),
                        ..
                    }) = &mut mode
                    {
                        if let Err(error) = prepared.apply_league_start_response(&response) {
                            return self.finish_startup_network_failure(
                                purpose,
                                format!("Unable to apply league registration: {error}"),
                            );
                        }
                    }
                    if response.max_players != 0 {
                        if let Some(staged) = self.staged_network_host_scenario.as_mut() {
                            staged.lobby.max_players = response.max_players;
                        }
                    }
                }
                // The worker already ran the `DeinitLeague` half of a refused
                // registration; this is `LeagueStart`'s modal, whose answer is
                // all `pCancel` ever carried (src/C4Network2.cpp:259-272).
                // Queueing it here is safe because the user answers it long
                // after the session below is installed.
                if let Some(error) = manager.take_league_start_failure() {
                    self.present_refused_league_registration(error)?;
                }
                let needs_initial_host_league_auth = purpose == StartupNetworkPurpose::StagedHost
                    && authenticated_league_players.is_none()
                    && matches!(
                        &mode,
                        NetworkMode::Host(HostSettings {
                            prepared: Some(prepared),
                            ..
                        }) if prepared.pending_initial_league_players().is_some()
                            && prepared
                                .host_config()
                                .initial_join_snapshot
                                .as_ref()
                                .is_some_and(|snapshot| synchronized_parameters_are_league(
                                    &snapshot.parameters
                                ))
                    );
                if needs_initial_host_league_auth {
                    match self.begin_startup_host_league_auth(
                        mode,
                        manager,
                        selected_scenario,
                        purpose,
                    )? {
                        LeaguePlayerAuthStatus::Pending
                        | LeaguePlayerAuthStatus::Completed(true) => return Ok(()),
                        LeaguePlayerAuthStatus::Completed(false) => {
                            return self.finish_startup_network_failure(
                                purpose,
                                "Unable to finalize initial host player authentication".to_string(),
                            );
                        }
                    }
                }
                if purpose == StartupNetworkPurpose::StagedHost {
                    if let NetworkMode::Host(HostSettings {
                        prepared: Some(prepared),
                        ..
                    }) = &mut mode
                    {
                        if let Some(staged) = self.staged_network_host_scenario.as_mut() {
                            if let Some(parameters) = prepared
                                .host_config()
                                .initial_join_snapshot
                                .as_ref()
                                .map(|snapshot| &snapshot.parameters)
                            {
                                staged.lobby.max_players = parameters.max_players;
                                staged.lobby.fair_crew = parameters.use_fair_crew;
                                staged.lobby.fair_crew_forced = parameters.fair_crew_forced;
                                staged.lobby.fair_crew_strength = parameters.fair_crew_strength;
                            } else {
                                staged.lobby.max_players = prepared.admission().max_players();
                            }
                        }
                        let is_league = prepared
                            .host_config()
                            .initial_join_snapshot
                            .as_ref()
                            .is_some_and(|snapshot| {
                                synchronized_parameters_are_league(&snapshot.parameters)
                            });
                        if prepared.pending_initial_league_players().is_some() {
                            if let Err(error) = self.finalize_prepared_host_players(
                                &manager,
                                prepared,
                                is_league,
                                authenticated_league_players.take(),
                            ) {
                                return self.finish_startup_network_failure(
                                    purpose,
                                    format!("Unable to finalize initial host players: {error}"),
                                );
                            }
                            if let Some(snapshot) =
                                prepared.host_config().initial_join_snapshot.clone()
                            {
                                if let Err(error) = manager.publish_join_snapshot(snapshot) {
                                    return self.finish_startup_network_failure(
                                        purpose,
                                        format!("Unable to publish initial host players: {error}"),
                                    );
                                }
                            }
                        }
                    }
                }
                if purpose == StartupNetworkPurpose::Join {
                    self.pending_network_join = None;
                    self.host_local_alternate_colors_by_resource.clear();
                    self.host_local_player_info_ids.clear();
                }
                // InitNetwork constructs a fresh C4Network2Client list; no
                // activity timestamp survives into the new socket session.
                self.network_client_activity.clear();
                if purpose == StartupNetworkPurpose::StagedHost {
                    let control_clients = initial_control_clients(Some(&manager), Some(&mode));
                    let network_control_clock = initial_network_control_clock(Some(&mode));
                    let mut previous_player_infos = None;
                    let mut previous_admission_resources = None;
                    let admission_ready = match &mode {
                        NetworkMode::Host(settings) => match settings.prepared.as_ref() {
                            Some(prepared) => {
                                // C4Network2Players::Init executes the host's
                                // Initial PlayerInfo directly before C4Game opens
                                // joining (src/C4Game.cpp:3869-3876;
                                // src/C4Network2Players.cpp:38-49,78-123).
                                previous_player_infos =
                                    Some(std::mem::take(&mut self.control_player_infos));
                                previous_admission_resources =
                                    Some(std::mem::take(&mut self.admission_resources));
                                let player_infos = &mut self.control_player_infos;
                                let resources = &mut self.admission_resources;
                                match prepared.install_initial_host_player_state(
                                    player_infos,
                                    |core, path| {
                                        resources.register_lobby_resource(core);
                                        resources.mark_complete(core.id, path.to_path_buf())
                                    },
                                ) {
                                    Ok(ready) => Some(ready),
                                    Err(error) => {
                                        self.control_player_infos = previous_player_infos
                                            .take()
                                            .expect("prepared install saved the previous registry");
                                        self.admission_resources =
                                            previous_admission_resources.take().expect(
                                                "prepared install saved the previous resources",
                                            );
                                        return self.finish_startup_network_failure(
                                            purpose,
                                            format!(
                                                "Unable to install prepared host PlayerInfo/resources: {error}"
                                            ),
                                        );
                                    }
                                }
                            }
                            None => None,
                        },
                        NetworkMode::Client(_) => None,
                    };
                    if let Some(admission_ready) = admission_ready {
                        if let Err(error) =
                            manager.set_join_allowed(admission_ready.lobby_join_allowed())
                        {
                            if let Some(previous_player_infos) = previous_player_infos.take() {
                                self.control_player_infos = previous_player_infos;
                            }
                            if let Some(previous_admission_resources) =
                                previous_admission_resources.take()
                            {
                                self.admission_resources = previous_admission_resources;
                            }
                            return self.finish_startup_network_failure(
                                purpose,
                                format!("Unable to open prepared host admission: {error}"),
                            );
                        }
                    }
                    self.host_local_alternate_colors_by_resource =
                        initial_host_local_alternate_colors(Some(&mode));
                    self.host_local_player_info_ids =
                        initial_host_local_player_info_ids(Some(&mode));
                    self.prune_host_local_alternate_colors();
                    if self.staged_network_host_scenario.is_none()
                        && matches!(
                            &mode,
                            NetworkMode::Host(HostSettings {
                                prepared: Some(_),
                                ..
                            })
                        )
                    {
                        if let Some((identifier, title)) = selected_scenario.as_ref() {
                            // Keep the prepared host's selected scenario and
                            // admission state alive in the established network
                            // lobby projection.
                            let mut lobby = NetworkLobbyState::new(
                                manager.local_client_id(),
                                self.player_name.clone(),
                                true,
                            )
                            .with_external_chat(self.startup_irc_client_active())
                            .with_preloading(
                                load_options_program_state(
                                    self.app_paths.as_ref(),
                                    Some(&self.startup_tooltip_resources),
                                )
                                .preloading,
                                self.classic_lobby_labels(),
                            );
                            lobby.select_scenario(identifier, title);
                            self.scenario_label = lobby.scenario_label();
                            if let NetworkMode::Host(HostSettings {
                                prepared: Some(prepared),
                                ..
                            }) = &mode
                            {
                                self.start_prepared_network_game_advertiser(prepared, &manager);
                            }
                            self.network_max_players = initial_network_max_players(Some(&mode));
                            self.engine.set_max_players(
                                i32::try_from(self.network_max_players).unwrap_or(i32::MAX),
                            );
                            self.control_clients = control_clients;
                            self.host_join_snapshot = initial_host_join_snapshot(Some(&mode));
                            self.network_is_league = initial_network_is_league(Some(&mode));
                            self.network_league_name = initial_network_league_name(Some(&mode));
                            self.network_stream_address =
                                initial_network_stream_address(Some(&mode));
                            seed_engine_player_info_parameters(
                                &mut self.engine,
                                &self.network_league_name,
                                &self.control_player_infos,
                            );
                            self.network_team_assignment = initial_network_team_assignment(
                                Some(&mode),
                                &self.generated_team_name_template,
                            );
                            self.network_mode = Some(mode);
                            self.network = Some(manager);
                            self.network_control_running = false;
                            self.network_control_clock = network_control_clock;
                            self.network_lobby = Some(lobby);
                            self.classic_host_lobby = None;
                            self.replace_startup_view(StartupView::NetworkLobby);
                            self.mode = AppMode::Menu;
                            self.status_text.clear();
                            self.restore_startup_fonts();
                            self.finish_classic_command_line_host_entry()?;
                            return Ok(());
                        }
                    }
                    match self.build_classic_host_lobby(&mode, &manager) {
                        Ok((lobby, options)) => {
                            match &mode {
                                NetworkMode::Host(HostSettings {
                                    prepared: Some(prepared),
                                    ..
                                }) => {
                                    self.start_prepared_network_game_advertiser(prepared, &manager)
                                }
                                NetworkMode::Host(_) | NetworkMode::Client(_) => {
                                    self.network_game_advertiser = None;
                                    self.advertised_game_reference = None;
                                    self.host_reference_paused = false;
                                }
                            }
                            self.network_max_players = initial_network_max_players(Some(&mode));
                            self.engine.set_max_players(
                                i32::try_from(self.network_max_players).unwrap_or(i32::MAX),
                            );
                            self.control_clients = control_clients;
                            self.host_join_snapshot = initial_host_join_snapshot(Some(&mode));
                            self.network_is_league = initial_network_is_league(Some(&mode));
                            self.network_league_name = initial_network_league_name(Some(&mode));
                            self.network_stream_address =
                                initial_network_stream_address(Some(&mode));
                            seed_engine_player_info_parameters(
                                &mut self.engine,
                                &self.network_league_name,
                                &self.control_player_infos,
                            );
                            self.network_team_assignment = initial_network_team_assignment(
                                Some(&mode),
                                &self.generated_team_name_template,
                            );
                            self.network_mode = Some(mode);
                            self.network = Some(manager);
                            self.network_control_running = false;
                            self.network_control_clock = network_control_clock;
                            self.network_lobby = None;
                            self.classic_host_lobby = Some(lobby);
                            self.sync_classic_lobby_roster();
                            self.sync_classic_lobby_resource_ready();
                            self.scenario_game_options = options;
                            self.replace_startup_view(StartupView::NetworkLobby);
                            self.mode = AppMode::Menu;
                            self.status_text.clear();
                            if let Some(audio) = self.sound.context.as_ref() {
                                let mut audio = audio.borrow_mut();
                                audio.stop_music();
                            }
                            self.finish_classic_command_line_host_entry()?;
                            return Ok(());
                        }
                        Err(error) => {
                            if let Some(previous_player_infos) = previous_player_infos.take() {
                                self.control_player_infos = previous_player_infos;
                            }
                            if let Some(previous_admission_resources) =
                                previous_admission_resources.take()
                            {
                                self.admission_resources = previous_admission_resources;
                            }
                            tracing::error!(%error, "cannot enter exact classic host lobby");
                            return self.finish_startup_network_failure(
                                purpose,
                                format!("Network lobby unavailable: {error}"),
                            );
                        }
                    }
                } else {
                    let control_clients = initial_control_clients(Some(&manager), Some(&mode));
                    let local_name = i32::try_from(manager.local_client_id())
                        .ok()
                        .and_then(|client_id| control_clients.state(client_id))
                        .filter(|client| !client.name.is_empty())
                        .map(|client| legacy_presentation_text(client.name.as_bytes()))
                        .unwrap_or_else(|| self.player_name.clone());
                    let lobby =
                        NetworkLobbyState::new(manager.local_client_id(), local_name, false)
                            .with_external_chat(self.startup_irc_client_active())
                            .with_preloading(
                                load_options_program_state(
                                    self.app_paths.as_ref(),
                                    Some(&self.startup_tooltip_resources),
                                )
                                .preloading,
                                self.classic_lobby_labels(),
                            );
                    self.network_game_advertiser = None;
                    self.advertised_game_reference = None;
                    self.host_reference_paused = false;
                    self.network_max_players = initial_network_max_players(Some(&mode));
                    self.engine.set_max_players(
                        i32::try_from(self.network_max_players).unwrap_or(i32::MAX),
                    );
                    self.network_is_league = initial_network_is_league(Some(&mode));
                    self.network_league_name = initial_network_league_name(Some(&mode));
                    self.network_stream_address = initial_network_stream_address(Some(&mode));
                    seed_engine_player_info_parameters(
                        &mut self.engine,
                        &self.network_league_name,
                        &self.control_player_infos,
                    );
                    self.network_control_clock = initial_network_control_clock(Some(&mode));
                    self.control_clients = control_clients;
                    self.host_join_snapshot = initial_host_join_snapshot(Some(&mode));
                    self.network_mode = Some(mode);
                    self.network = Some(manager);
                    self.network_control_running = false;
                    self.network_team_assignment = None;
                    self.network_lobby = Some(lobby);
                    self.classic_host_lobby = None;
                    self.host_lobby_countdown = None;
                    self.pending_local_lobby_countdown_echoes.clear();
                    self.mode = AppMode::Menu;
                    self.open_network_lobby();
                    return Ok(());
                }
            }
            Err(NetworkStartError::WrongPassword { .. })
                if purpose == StartupNetworkPurpose::Join
                    && self.pending_network_join.is_some() =>
            {
                self.mode = AppMode::Menu;
                if let Err(error) = self.open_network_join_password_dialog() {
                    return self.finish_startup_network_failure(
                        purpose,
                        format!("Unable to reopen the network password prompt: {error}"),
                    );
                }
            }
            Err(error) => {
                // A host that is still re-binding refuses connections; that is
                // the expected first answer to a restart notice, not a failure
                // to show the player.
                if self.defer_pending_host_rejoin() {
                    tracing::info!(%error, "reconnect to the restarting host was refused; retrying");
                    return Ok(());
                }
                return self.finish_startup_network_failure(
                    purpose,
                    format!("Unable to start network session: {error}"),
                );
            }
        }
        Ok(())
    }

    pub(crate) fn open_startup_player_context_menu(
        &mut self,
        keyboard_trigger: bool,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::PlayerSelection
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
            || self.startup_player_properties_dialog.is_some()
            || self.game_option_input_dialog.is_some()
            || self.definition_selector.is_some()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }
        let Some((index, anchor, crew_mode)) =
            self.startup_player_dialog.as_ref().and_then(|dialog| {
                if keyboard_trigger {
                    let (index, anchor) = dialog.keyboard_context_target()?;
                    Some((index, anchor, dialog.is_crew_mode()))
                } else {
                    let point = dialog.pointer_position()?;
                    dialog
                        .context_index_at(point)
                        .map(|index| (index, point, dialog.is_crew_mode()))
                }
            })
        else {
            return Ok(false);
        };
        if let Some(rename_index) = self.startup_crew_rename.as_ref().map(|rename| rename.index) {
            if keyboard_trigger {
                return Ok(false);
            }
            if !crew_mode || index != rename_index {
                self.abort_startup_crew_rename();
            }
        }
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "C4GUI context menu",
            self.assets.context_menu_resources().map(|_| ()),
        )?;
        if !keyboard_trigger
            && !self
                .startup_player_dialog
                .as_mut()
                .is_some_and(|dialog| dialog.select_for_context(index))
        {
            tracing::error!(index, "startup player context menu references a stale row");
            return Ok(false);
        }

        let entries = if crew_mode {
            clonk_frontend::startup_plrsel::PlrSelCrewContextMenu::for_crew(index)
                .entries
                .into_iter()
                .map(|entry| {
                    let icon = match entry.icon {
                        clonk_frontend::startup_plrsel::PlrSelCrewContextIcon::None => {
                            ContextMenuIcon::None
                        }
                    };
                    let mut item = ContextMenuEntry::new(entry.label)
                        .with_icon(icon)
                        .with_action(AppContextMenuCommand::StartupCrew(entry.command));
                    if let Some(tooltip) = entry.tooltip {
                        item = item.with_tooltip(tooltip);
                    }
                    if let Some(hotkey) = entry.hotkey {
                        item = item.with_hotkey(hotkey);
                    }
                    item
                })
                .collect()
        } else {
            clonk_frontend::startup_plrsel::PlrSelPlayerContextMenu::for_player(index)
                .entries
                .into_iter()
                .map(|entry| {
                    let icon = match entry.icon {
                        clonk_frontend::startup_plrsel::PlrSelPlayerContextIcon::None => {
                            ContextMenuIcon::None
                        }
                    };
                    let mut item = ContextMenuEntry::new(entry.label)
                        .with_icon(icon)
                        .with_action(AppContextMenuCommand::StartupPlayer(entry.command));
                    if let Some(tooltip) = entry.tooltip {
                        item = item.with_tooltip(tooltip);
                    }
                    if let Some(hotkey) = entry.hotkey {
                        item = item.with_hotkey(hotkey);
                    }
                    item
                })
                .collect()
        };
        let opened = self.open_context_menu_at(entries, anchor)?;
        if keyboard_trigger && opened {
            if let Some(menu) = self.context_menu.as_mut() {
                menu.note_non_pointer_input();
            }
        }
        Ok(opened)
    }

    pub(crate) fn open_startup_participants_context_menu(&mut self) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::MainMenu
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
        {
            return Ok(false);
        }
        let Some(anchor) = self
            .main_menu_state
            .pointer_position()
            .filter(|point| self.main_menu_state.participants_contains(*point))
        else {
            return Ok(false);
        };
        let Some(paths) = self.app_paths.clone() else {
            return Err(Self::gui_overlay_engine_error(
                "startup participants context menu",
                "application paths are unavailable",
            ));
        };
        let add_paths = paths.clone();
        let remove_paths = paths;
        let entries = vec![
            ContextMenuEntry::new("Add")
                .with_tooltip("Add participant")
                .with_lazy_submenu(move || startup_participant_add_entries(&add_paths)),
            ContextMenuEntry::new("Remove")
                .with_tooltip("Remove participant")
                .with_lazy_submenu(move || startup_participant_remove_entries(&remove_paths)),
        ];
        self.open_context_menu_at(entries, anchor)
    }

    pub(crate) fn set_startup_participant(&mut self, reference: &str, active: bool) {
        let Some(paths) = self.app_paths.as_ref() else {
            tracing::error!(
                reference,
                "cannot update participant without application paths"
            );
            self.status_text = "Unable to update participants".to_string();
            return;
        };
        let result = update_startup_participant_config(paths, |entries| {
            if active {
                if !entries
                    .iter()
                    .any(|entry| entry.eq_ignore_ascii_case(reference))
                {
                    entries.push(reference.to_string());
                }
            } else {
                entries.retain(|entry| !entry.eq_ignore_ascii_case(reference));
            }
        });
        self.finish_startup_participant_update(result, reference);
    }

    pub(crate) fn remove_startup_participant(&mut self, raw_index: usize) {
        let Some(paths) = self.app_paths.as_ref() else {
            tracing::error!(
                raw_index,
                "cannot remove participant without application paths"
            );
            self.status_text = "Unable to update participants".to_string();
            return;
        };
        match remove_startup_participant_config(paths, raw_index) {
            Ok(removed) => self.finish_startup_participant_update(
                Ok(()),
                removed.as_deref().unwrap_or("<stale participant>"),
            ),
            Err(error) => self.finish_startup_participant_update(
                Err(error),
                "<unreadable participant configuration>",
            ),
        }
    }

    fn finish_startup_participant_update(&mut self, result: io::Result<()>, reference: &str) {
        match result {
            Ok(()) => {
                self.sync_startup_participant_models();
                self.refresh_participants_label();
                self.status_text.clear();
            }
            Err(error) => {
                tracing::error!(reference, %error, "failed to update startup participants");
                self.status_text = format!("Unable to update participants: {error}");
            }
        }
    }

    pub(crate) fn apply_startup_game_search_event(
        &mut self,
        event: clonk_network::StartupGameSearchEvent,
    ) -> Result<(), EngineError> {
        if self.startup_network_refresh_waiting_for_clear {
            if !matches!(&event, clonk_network::StartupGameSearchEvent::Cleared) {
                return Ok(());
            }
            self.startup_network_refresh_waiting_for_clear = false;
        }
        match event {
            clonk_network::StartupGameSearchEvent::Cleared => {
                let selected_query = self.selected_startup_direct_reference_query_id();
                let selected_discovery_query = self.selected_startup_discovery_reference_query_id();
                self.startup_game_references.clear();
                self.sync_startup_network_game_rows();
                self.reset_startup_masterserver_entry();
                let masterserver_enabled = self
                    .startup_network_dialog
                    .as_ref()
                    .is_some_and(|dialog| dialog.config().masterserver_signup);
                self.startup_masterserver_next_query_at = if masterserver_enabled {
                    Instant::now().checked_add(clonk_network::GAME_SEARCH_INTERVAL)
                } else {
                    None
                };
                if let Some(id) = selected_query {
                    self.focus_startup_direct_reference_query(id);
                } else if let Some(id) = selected_discovery_query {
                    self.focus_startup_discovery_reference_query(id);
                }
            }
            clonk_network::StartupGameSearchEvent::ReferencesUpdated(references) => {
                let selected_reference = self.selected_startup_game_reference();
                let selected_query = self.selected_startup_direct_reference_query_id();
                let selected_discovery_query = self.selected_startup_discovery_reference_query_id();
                self.startup_game_references = references;
                self.sync_startup_network_game_rows();
                if let Some(reference) = selected_reference.as_ref() {
                    self.focus_startup_game_reference(reference);
                } else if let Some(id) = selected_query {
                    self.focus_startup_direct_reference_query(id);
                } else if let Some(id) = selected_discovery_query {
                    self.focus_startup_discovery_reference_query(id);
                }
            }
            clonk_network::StartupGameSearchEvent::GameDiscoveryQueryStarted { address } => {
                self.begin_startup_discovery_reference_query(address);
            }
            clonk_network::StartupGameSearchEvent::GameDiscoveryQueryResolved {
                address,
                references,
                selected_index,
            } => {
                self.finish_startup_discovery_reference_query(
                    address,
                    references,
                    selected_index.is_some(),
                );
            }
            clonk_network::StartupGameSearchEvent::GameDiscoveryQueryFailed {
                address,
                message,
            } => {
                self.fail_startup_discovery_reference_query(address, message);
            }
            clonk_network::StartupGameSearchEvent::DirectQueryResolved {
                request_id,
                references,
                selected_index,
            } => {
                self.finish_startup_direct_reference_query(request_id, references, selected_index);
            }
            clonk_network::StartupGameSearchEvent::DirectQueryFailed {
                request_id,
                message,
            } => {
                self.fail_startup_direct_reference_query(request_id, message);
            }
            clonk_network::StartupGameSearchEvent::MasterserverReply(reply) => {
                self.apply_startup_masterserver_reply(reply)?;
            }
            clonk_network::StartupGameSearchEvent::SearchError { source, message } => {
                tracing::warn!(?source, %message, "network game search failed");
                if source == Some(clonk_network::ReferenceQuerySource::GameDiscovery) {
                    self.show_startup_discovery_error(&message)?;
                } else if source == Some(clonk_network::ReferenceQuerySource::Masterserver) {
                    self.set_startup_masterserver_error(message);
                } else {
                    self.status_text = message;
                }
            }
        }
        Ok(())
    }

    fn startup_network_dialog_is_covered_by_message(&self) -> bool {
        self.startup_view == StartupView::NetworkGame
            && self.startup_network_dialog.is_some()
            && !self.message_dialogs.is_empty()
    }

    fn next_startup_game_search_event(&mut self) -> Option<clonk_network::StartupGameSearchEvent> {
        #[cfg(test)]
        if let Some(event) = self.startup_game_search_test_events.pop_front() {
            return Some(event);
        }
        self.startup_game_search
            .as_ref()
            .and_then(|search| search.events().try_recv().ok())
    }

    pub(crate) fn poll_startup_game_search(&mut self) -> Result<(), EngineError> {
        // C4StartupNetDlg::OnSec1Timer stops before both DiscoverClient.Execute
        // and UpdateList while a message dialog owns the active-dialog slot.
        // Leave worker events queued as well as presentation timers untouched
        // until the network dialog becomes active again.
        if self.startup_network_dialog_is_covered_by_message() {
            return Ok(());
        }
        // Advance presentation timers before applying completions from the
        // query that began at this interval, so an immediate failure/success
        // remains visible instead of being overwritten by Query state.
        self.tick_startup_network_query_rows_at(Instant::now());
        loop {
            // Applying one event may itself open a message (for example a
            // masterserver redirect). Do not pre-drain later events: native
            // leaves the remainder pending as soon as NetDlg loses activity.
            if self.startup_network_dialog_is_covered_by_message() {
                break;
            }
            let Some(event) = self.next_startup_game_search_event() else {
                break;
            };
            self.apply_startup_game_search_event(event)?;
        }
        Ok(())
    }

    fn sync_startup_participant_models(&mut self) {
        let active = self
            .app_paths
            .as_ref()
            .and_then(|paths| startup_participant_references(paths).ok())
            .unwrap_or_default();
        for (file, model) in self
            .startup_player_files
            .iter_mut()
            .zip(self.startup_player_models.iter_mut())
        {
            let enabled = active
                .iter()
                .any(|entry| entry.eq_ignore_ascii_case(&file.file_name));
            file.set_activated(enabled);
            model.activated = enabled;
        }
        self.selected_player_file = active.iter().find_map(|reference| {
            self.startup_player_files
                .iter()
                .find(|player| player.file_name.eq_ignore_ascii_case(reference))
                .map(|player| player.player_file.clone())
        });
        if let Some(dialog) = self.startup_player_dialog.as_mut() {
            dialog.set_player_activations(
                self.startup_player_models
                    .iter()
                    .map(|player| player.activated)
                    .collect(),
            );
        }
    }

    /// Name the new-player dialog starts from. `C4PlayerInfoCore::Default`
    /// hardcodes the German "Neuling" (C4InfoCore.cpp:69) and C++ shows it to
    /// English players too; "Neuling" is rank 0 of `IDS_RANKS_PLAYER`, so take
    /// the seed from the localized ladder instead. Deliberate presentation-only
    /// divergence: the `Player.txt` omit-if-equal write default and the
    /// missing-`Name=` read fallback both stay "Neuling", so the file still
    /// round-trips byte-identically with C++.
    pub(crate) fn new_player_default_name(&self) -> String {
        const FALLBACK: &str = "Novice";
        let ladder = self.runtime_resource_text("IDS_RANKS_PLAYER", FALLBACK);
        ladder
            .split('|')
            .next()
            .filter(|rank| !rank.is_empty())
            .unwrap_or(FALLBACK)
            .to_string()
    }

    pub(crate) fn new_startup_player_properties_controller(
        &self,
        color_index: usize,
        portrait_index: usize,
    ) -> clonk_frontend::startup_plrproperties::PlayerPropertiesController {
        use clonk_frontend::startup_plrproperties::{PlayerPropertiesController, PLAYER_COLORS};

        let color_index = color_index % 8;
        let player = PlayerFile {
            name: self.new_player_default_name(),
            pref_color: color_index as i32,
            pref_color_dw: PLAYER_COLORS[color_index],
            pref_control: 0,
            pref_control_style: true,
            pref_auto_context_menu: true,
            ..PlayerFile::default()
        };
        let comment = self.runtime_resource_text("IDS_PLR_NEWCOMMENT", "I'm new.");
        let portrait = self
            .assets
            .dialog_image(&format!("Portrait{}.png", portrait_index % 5 + 1))
            .or_else(|| {
                (1..=5).find_map(|index| self.assets.dialog_image(&format!("Portrait{index}.png")))
            });
        let big_icon = portrait
            .as_ref()
            .and_then(|portrait| startup_player_big_icon(portrait, player.pref_color_dw));
        let mut controller = PlayerPropertiesController::new_player(
            player,
            comment,
            portrait.and_then(|portrait| materialize_startup_player_image(&portrait, 150)),
            big_icon,
        );
        controller.resize(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
        );
        controller
    }

    pub(crate) fn open_new_startup_player_properties(&mut self) {
        self.open_new_startup_player_properties_from(StartupPlayerPropertiesOrigin::SelectionNew);
    }

    pub(crate) fn open_new_startup_player_properties_from(
        &mut self,
        origin: StartupPlayerPropertiesOrigin,
    ) {
        self.close_context_menu_silently();
        self.startup_tooltip.pointer_left();
        let controller = self.new_startup_player_properties_controller(
            classic_safe_random(8),
            classic_safe_random(5),
        );
        self.startup_player_properties_dialog =
            Some(PendingStartupPlayerProperties { origin, controller });
        self.status_text.clear();
    }

    pub(crate) fn open_existing_startup_player_properties(&mut self, index: usize) {
        let Some((path, was_activated, player, comment, portrait, big_icon)) =
            self.startup_player_files.get(index).map(|entry| {
                (
                    entry.path.clone(),
                    entry.render_model.activated,
                    entry.player_file.clone(),
                    entry.render_model.comment.clone(),
                    entry.render_model.portrait.clone(),
                    entry.render_model.big_icon.clone(),
                )
            })
        else {
            tracing::error!(index, "player-properties action references a stale row");
            return;
        };
        self.close_context_menu_silently();
        self.startup_tooltip.pointer_left();
        let mut controller =
            clonk_frontend::startup_plrproperties::PlayerPropertiesController::edit_player(
                index, player, comment, portrait, big_icon,
            );
        controller.resize(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
        );
        self.startup_player_properties_dialog = Some(PendingStartupPlayerProperties {
            origin: StartupPlayerPropertiesOrigin::SelectionEdit {
                path,
                was_activated,
            },
            controller,
        });
        self.status_text.clear();
    }

    pub(crate) fn process_startup_player_properties_actions(
        &mut self,
        actions: Vec<clonk_frontend::startup_plrproperties::PlayerPropertiesAction>,
    ) {
        use clonk_frontend::startup_plrproperties::PlayerPropertiesAction;

        for action in actions {
            match action {
                PlayerPropertiesAction::Cancel => {
                    self.startup_tooltip.pointer_left();
                    self.startup_player_properties_dialog = None;
                    self.status_text.clear();
                }
                PlayerPropertiesAction::ChoosePicture => {
                    self.open_startup_player_portrait_selector();
                }
                PlayerPropertiesAction::PortraitLocationChanged { index, path } => {
                    self.reload_startup_player_portrait_location(index, &path);
                }
                PlayerPropertiesAction::PortraitSelectorClosed { location_index } => {
                    self.startup_last_portrait_folder_index = Some(location_index);
                    if let Some(paths) = self.app_paths.as_ref() {
                        if let Err(error) = persist_startup_portrait_location(paths, location_index)
                        {
                            tracing::warn!(%error, "failed to persist portrait location");
                        }
                    }
                }
                PlayerPropertiesAction::PortraitSelectionRequired => {
                    let message = self.runtime_resource_text(
                        "IDS_ERR_PLEASESELECTAFILEFIRST",
                        "Please select a file first!",
                    );
                    self.show_startup_portrait_error(message);
                }
                PlayerPropertiesAction::GuiSound(sound) => {
                    use clonk_frontend::startup_portraitsel::PortraitSelSound;
                    self.play_ui_sound(match sound {
                        PortraitSelSound::ArrowHit => "ArrowHit",
                        PortraitSelSound::DoorOpen => "DoorOpen",
                        PortraitSelSound::DoorClose => "DoorClose",
                        PortraitSelSound::Command => "Command",
                        PortraitSelSound::Click => "Click",
                    });
                }
                PlayerPropertiesAction::ApplyPicture(commit) => {
                    self.apply_startup_player_portrait_selection(commit);
                }
                PlayerPropertiesAction::Submit => self.save_open_startup_player_properties(),
            }
        }
    }

    pub(crate) fn reload_startup_player_portrait_location(&mut self, index: usize, path: &Path) {
        match clonk_frontend::startup_portraitsel::portrait_files_in_location(path) {
            Ok(entries) => {
                if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
                    pending
                        .controller
                        .replace_portrait_location_entries(index, entries);
                }
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "failed to scan portrait location");
                if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
                    pending.controller.fail_portrait_location_entries(
                        index,
                        format!("failed to scan {}: {error}", path.display()),
                    );
                }
            }
        }
    }

    fn show_startup_portrait_error(&mut self, message: String) {
        let caption = self.runtime_resource_text("IDS_DLG_ERROR", "Error");
        self.status_text.clear();
        if let Err(dialog_error) = self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                message.clone(),
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            ),
            MessageDialogContinuation::None,
        ) {
            tracing::error!(%dialog_error, "failed to show portrait-selector error dialog");
            self.status_text = message;
        }
    }

    pub(crate) fn apply_startup_player_portrait_selection(
        &mut self,
        commit: clonk_frontend::startup_portraitsel::PortraitSelCommit,
    ) {
        use clonk_frontend::startup_portraitsel::PortraitChoice;

        let color = self
            .startup_player_properties_dialog
            .as_ref()
            .map(|pending| pending.controller.player().pref_color_dw)
            .unwrap_or_default();
        let (portrait, big_icon) = match commit.choice {
            PortraitChoice::None => (None, None),
            PortraitChoice::File(path) => {
                if !commit.set_picture && !commit.set_big_icon {
                    (None, None)
                } else {
                    let image = match load_startup_portrait_image(&path) {
                        Ok(image) => image,
                        Err(detail) => {
                            let message = format_resource_string_with_opaque_arguments(
                                self.runtime_resource_text(
                                    "IDS_PRC_NOGFXFILE",
                                    "Error at graphics file %s: %s",
                                ),
                                &[&path.display().to_string(), &detail],
                            );
                            self.show_startup_portrait_error(message);
                            return;
                        }
                    };
                    let portrait = commit
                        .set_picture
                        .then(|| materialize_startup_player_image(&image, 150))
                        .flatten();
                    let big_icon = commit
                        .set_big_icon
                        .then(|| startup_player_big_icon(&image, color))
                        .flatten();
                    (portrait, big_icon)
                }
            }
        };
        if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
            pending.controller.apply_picture_selection(
                portrait,
                big_icon,
                commit.set_picture,
                commit.set_big_icon,
            );
            pending.controller.close_portrait_selector();
            pending.controller.clear_validation_error();
        }
    }

    pub(crate) fn advance_startup_player_portrait_thumbnail(&mut self) {
        let request = self
            .startup_player_properties_dialog
            .as_mut()
            .and_then(|pending| pending.controller.advance_portrait_selector_idle());
        let Some(request) = request else {
            return;
        };
        let thumbnail = load_startup_portrait_image(&request.path)
            .map(|image| resize_startup_player_image(&image, 100));
        if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
            pending
                .controller
                .complete_portrait_thumbnail(&request, thumbnail);
        }
    }

    fn save_open_startup_player_properties(&mut self) {
        let Some((origin, player, comment, portrait, big_icon)) = self
            .startup_player_properties_dialog
            .as_ref()
            .map(|pending| {
                (
                    pending.origin.clone(),
                    pending.controller.player().clone(),
                    pending.controller.comment().to_string(),
                    startup_player_image_write(pending.controller.portrait_update()),
                    startup_player_image_write(pending.controller.big_icon_update()),
                )
            })
        else {
            return;
        };
        let existing_path = match &origin {
            StartupPlayerPropertiesOrigin::MainMenuFirstPlayer
            | StartupPlayerPropertiesOrigin::SelectionNew => None,
            StartupPlayerPropertiesOrigin::SelectionEdit { path, .. } => Some(path.as_path()),
        };
        let Some(paths) = self.app_paths.as_ref() else {
            let error = "application paths are unavailable".to_string();
            tracing::error!(%error, "failed to save startup player properties");
            self.finish_startup_player_properties_save_failure(error, &origin, &player.name);
            return;
        };
        let result = save_player_properties(
            paths,
            existing_path,
            &player,
            &comment,
            &portrait,
            &big_icon,
            self.process_group_maker.as_bytes(),
        );
        match result {
            Ok(saved) => self.finish_startup_player_properties_save(saved, origin),
            Err(
                error @ (PlayerPropertiesSaveError::EmptyName
                | PlayerPropertiesSaveError::NameTaken { .. }),
            ) => {
                tracing::error!(%error, "startup player name validation failed");
                let message = match &error {
                    PlayerPropertiesSaveError::EmptyName => self.runtime_resource_text(
                        "IDS_ERR_PLRNAME_EMPTY",
                        "You must specify a player name!",
                    ),
                    PlayerPropertiesSaveError::NameTaken { name, .. } => format_resource_string(
                        self.runtime_resource_text("IDS_ERR_PLRNAME_TAKEN", "%s is already taken"),
                        &[name],
                    ),
                    _ => unreachable!("guarded player-name validation error"),
                };
                if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
                    pending.controller.clear_validation_error();
                }
                self.status_text.clear();
                if let Err(dialog_error) = self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                        message.clone(),
                        "",
                        clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                    ),
                    MessageDialogContinuation::None,
                ) {
                    tracing::error!(%dialog_error, "failed to show player-name validation dialog");
                    self.record_startup_player_properties_save_failure(message);
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to save startup player properties");
                let message = self.startup_player_properties_save_failure_text(&error);
                self.finish_startup_player_properties_save_failure(message, &origin, &player.name);
            }
        }
    }

    /// Composes the classic modal body for a post-validation save failure.
    /// Each storage step keeps C++'s own string for that branch
    /// (`C4StartupPlrPropertiesDlg::OnClosed`: `IDS_FAIL_RENAME`,
    /// `IDS_FAIL_MODIFY`, and the `ShowErrorMessage`d group step errors) and
    /// names the affected path like the startup rename neighbors do with
    /// `IDS_ERR_RENAMEFILE`/`IDS_ERR_OPENFILE`.
    pub(crate) fn startup_player_properties_save_failure_text(
        &self,
        error: &PlayerPropertiesSaveError,
    ) -> String {
        match error {
            PlayerPropertiesSaveError::Rename { from, to, detail } => {
                let step = self.runtime_resource_text("IDS_FAIL_RENAME", "Rename failure.");
                let body = format_resource_string_with_opaque_arguments(
                    self.runtime_resource_text(
                        "IDS_ERR_RENAMEFILE",
                        "Error renaming file \"%s\" to \"%s\".",
                    ),
                    &[&from.display().to_string(), &to.display().to_string()],
                );
                format!("{step}\n{body}\n{detail}")
            }
            PlayerPropertiesSaveError::Open { path, detail } => {
                format_resource_string_with_opaque_arguments(
                    self.runtime_resource_text("IDS_ERR_OPENFILE", "Error opening file \"%s\": %s"),
                    &[&path.display().to_string(), detail],
                )
            }
            PlayerPropertiesSaveError::WriteCore {
                path,
                entry,
                detail,
            } => {
                let step =
                    self.runtime_resource_text("IDS_FAIL_MODIFY", "File modification failure.");
                format!("{step}\n\"{}/{entry}\": {detail}", path.display())
            }
            PlayerPropertiesSaveError::WriteImage {
                path,
                entry,
                detail,
            } => format_resource_string_with_opaque_arguments(
                self.runtime_resource_text("IDS_PRC_NOGFXFILE", "Error at graphics file %s: %s"),
                &[&format!("{}/{entry}", path.display()), detail],
            ),
            // C++ surfaces a failed group flush as the raw "Close:"-step
            // C4Group error; carry the affected path with it.
            PlayerPropertiesSaveError::Close { path, detail } => {
                format!("Close: \"{}\": {detail}", path.display())
            }
            PlayerPropertiesSaveError::EmptyName | PlayerPropertiesSaveError::NameTaken { .. } => {
                error.to_string()
            }
        }
    }

    fn finish_startup_player_properties_save_failure(
        &mut self,
        error: String,
        origin: &StartupPlayerPropertiesOrigin,
        submitted_name: &str,
    ) {
        // C4GUI removes C4StartupPlrPropertiesDlg before OnClosed performs
        // persistence. The error dialog therefore belongs to the screen and
        // never leaves the properties form alive underneath it.
        self.startup_tooltip.pointer_left();
        self.startup_player_properties_dialog = None;
        self.reconcile_startup_player_list_after_properties_failure(origin, submitted_name);
        self.status_text.clear();

        let caption = self.runtime_resource_text("IDS_DLG_ERROR", "Error");
        if let Err(dialog_error) = self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                error.clone(),
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            ),
            MessageDialogContinuation::None,
        ) {
            tracing::error!(%dialog_error, "failed to show player-properties save error dialog");
            self.record_startup_player_properties_save_failure(error);
        }
    }

    fn reconcile_startup_player_list_after_properties_failure(
        &mut self,
        origin: &StartupPlayerPropertiesOrigin,
        submitted_name: &str,
    ) {
        let previous_players = self
            .startup_player_files
            .iter()
            .map(|player| {
                (
                    player.path.clone(),
                    player.file_name.clone(),
                    player.render_model.activated,
                )
            })
            .collect::<Vec<_>>();
        let previous_selected = self
            .startup_player_dialog
            .as_ref()
            .and_then(|dialog| dialog.selected_index())
            .and_then(|index| self.startup_player_files.get(index))
            .map(|player| (player.path.clone(), player.file_name.clone()));
        let submitted_filename = player_group_filename(submitted_name).ok();
        let edit_paths = match (origin, submitted_filename.as_deref()) {
            (StartupPlayerPropertiesOrigin::SelectionEdit { path, .. }, Some(filename)) => {
                Some((path.clone(), path.with_file_name(filename)))
            }
            _ => None,
        };

        let Some(paths) = self.app_paths.as_ref() else {
            return;
        };
        let config_file = paths.config_file().to_path_buf();
        let mut players = match discover_player_files(paths) {
            Ok(players) => players,
            Err(error) => {
                tracing::error!(%error, "failed to reconcile startup players after properties save failure");
                return;
            }
        };

        let submitted_index = match (&edit_paths, submitted_filename.as_deref()) {
            (Some((old_path, target_path)), _) => players
                .iter()
                .position(|player| player.path == *old_path)
                .or_else(|| {
                    players
                        .iter()
                        .position(|player| player.path == *target_path)
                }),
            (None, Some(filename)) => players.iter().position(|player| {
                player
                    .path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(filename))
            }),
            (None, None) => None,
        };

        for (index, player) in players.iter_mut().enumerate() {
            // A file first seen by this reconciliation keeps its discover-time
            // activation, which C4StartupPlrSelDlg::UpdatePlayerList reads
            // from Config.General.Participants.
            let previous_activation = previous_players
                .iter()
                .find(|(path, file_name, _)| {
                    player.path == *path || player.file_name.eq_ignore_ascii_case(file_name)
                })
                .map(|(_, _, activated)| *activated)
                .unwrap_or(player.render_model.activated);
            let activated = if Some(index) == submitted_index {
                match origin {
                    StartupPlayerPropertiesOrigin::MainMenuFirstPlayer
                    | StartupPlayerPropertiesOrigin::SelectionNew => true,
                    StartupPlayerPropertiesOrigin::SelectionEdit { was_activated, .. } => {
                        *was_activated
                    }
                }
            } else {
                previous_activation
            };
            player.set_activated(activated);
        }

        let activation_refusals = match persist_activations(&config_file, &mut players) {
            Ok(refusals) => refusals,
            Err(error) => {
                tracing::warn!(%error, "failed to reconcile participants after properties save failure");
                Vec::new()
            }
        };
        // A failed edit leaves C++'s untouched list widget selected where it
        // was (following the item across a completed rename), while a failed
        // NewPlayer save rebuilds the list and reselects like
        // UpdatePlayerList: the saved file, else the first activated entry,
        // else the first entry.
        let previous_selected_index = previous_selected.as_ref().and_then(|(path, file_name)| {
            players.iter().position(|player| {
                player.path == *path || player.file_name.eq_ignore_ascii_case(file_name)
            })
        });
        let selected_index = match origin {
            StartupPlayerPropertiesOrigin::SelectionEdit { .. } => {
                previous_selected_index.or(submitted_index)
            }
            StartupPlayerPropertiesOrigin::MainMenuFirstPlayer
            | StartupPlayerPropertiesOrigin::SelectionNew => submitted_index,
        }
        .or_else(|| {
            players
                .iter()
                .position(|player| player.render_model.activated)
        })
        .or_else(|| (!players.is_empty()).then_some(0));
        self.startup_player_models = players
            .iter()
            .map(|player| player.render_model.clone())
            .collect();
        self.startup_player_files = players;
        self.selected_player_file = selected_index
            .and_then(|index| self.startup_player_files.get(index))
            .filter(|player| player.render_model.activated)
            .or_else(|| {
                self.startup_player_files
                    .iter()
                    .find(|player| player.render_model.activated)
            })
            .map(|player| player.player_file.clone());
        if let Some(dialog) = self.startup_player_dialog.as_mut() {
            dialog.set_player_count(self.startup_player_models.len());
            dialog.set_player_activations(
                self.startup_player_models
                    .iter()
                    .map(|player| player.activated)
                    .collect(),
            );
            dialog.set_selected_index(selected_index);
        }
        self.plrsel_last_click = None;
        self.refresh_participants_label();
        if let Err(error) = self.show_startup_player_activation_refusals(&activation_refusals) {
            tracing::error!(%error, "failed to show participant overflow after properties save failure");
        }
    }

    fn record_startup_player_properties_save_failure(&mut self, error: String) {
        if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
            pending.controller.set_validation_error(Some(error.clone()));
        }
        self.status_text = error;
    }

    fn finish_startup_player_properties_save(
        &mut self,
        saved: SavedStartupPlayer,
        origin: StartupPlayerPropertiesOrigin,
    ) {
        let previous_activations = self
            .startup_player_files
            .iter()
            .map(|player| (player.file_name.clone(), player.render_model.activated))
            .collect::<Vec<_>>();
        // `C4StartupPlrSelDlg` rebuilds `Config.General.Participants` in memory
        // and never saves — that file contains no `Config.Save()` at all, and
        // `UpdateActivatedPlayers` (:824-833) just re-runs `SAddModule`. A memory
        // rebuild cannot fail, so unlike the previous eager write there is no
        // error to report here.
        let forced_first_participant =
            matches!(&origin, StartupPlayerPropertiesOrigin::MainMenuFirstPlayer);
        if forced_first_participant {
            self.defer_participant_list(&saved.file_name);
        }
        let Some(paths) = self.app_paths.as_ref() else {
            self.startup_tooltip.pointer_left();
            self.startup_player_properties_dialog = None;
            self.status_text = "Player saved, but application paths are unavailable".to_string();
            return;
        };
        let mut players = match discover_player_files(paths) {
            Ok(players) => players,
            Err(error) => {
                tracing::error!(%error, "failed to refresh startup players after save");
                self.startup_tooltip.pointer_left();
                self.startup_player_properties_dialog = None;
                self.refresh_participants_label();
                self.status_text =
                    format!("Player saved, but the list could not be refreshed: {error}");
                return;
            }
        };
        let is_saved = |player: &StartupPlayerFile| {
            player.path == saved.path || player.file_name.eq_ignore_ascii_case(&saved.file_name)
        };
        let saved_index = players.iter().position(&is_saved);
        for player in &mut players {
            let was_activated = previous_activations
                .iter()
                .find(|(reference, _)| reference.eq_ignore_ascii_case(&player.file_name))
                .is_some_and(|(_, activated)| *activated);
            let activated = match &origin {
                StartupPlayerPropertiesOrigin::MainMenuFirstPlayer => is_saved(player),
                StartupPlayerPropertiesOrigin::SelectionNew => is_saved(player) || was_activated,
                StartupPlayerPropertiesOrigin::SelectionEdit {
                    was_activated: saved_was_activated,
                    ..
                } => {
                    if is_saved(player) {
                        *saved_was_activated
                    } else {
                        was_activated
                    }
                }
            };
            player.set_activated(activated);
        }
        let persistence = if forced_first_participant {
            Ok(Vec::new())
        } else {
            persist_activations(&paths.config_file(), &mut players)
        };
        let (persistence_error, activation_refusals) = match persistence {
            Ok(refusals) => (None, refusals),
            Err(error) => (Some(error), Vec::new()),
        };
        self.startup_player_models = players
            .iter()
            .map(|player| player.render_model.clone())
            .collect();
        self.startup_player_files = players;
        self.selected_player_file = saved_index
            .and_then(|index| self.startup_player_files.get(index))
            .filter(|player| player.render_model.activated)
            .or_else(|| {
                self.startup_player_files
                    .iter()
                    .find(|player| player.render_model.activated)
            })
            .map(|player| player.player_file.clone());
        if let Some(dialog) = self.startup_player_dialog.as_mut() {
            dialog.set_player_count(self.startup_player_models.len());
            dialog.set_player_activations(
                self.startup_player_models
                    .iter()
                    .map(|player| player.activated)
                    .collect(),
            );
            dialog.set_selected_index(saved_index);
        }
        self.startup_tooltip.pointer_left();
        self.startup_player_properties_dialog = None;
        self.refresh_participants_label();
        if let Some(error) = persistence_error {
            tracing::error!(%error, "failed to refresh participants after player save");
            self.status_text = format!("Player saved, but participant selection failed: {error}");
        } else {
            self.status_text.clear();
        }
        if let Err(error) = self.show_startup_player_activation_refusals(&activation_refusals) {
            tracing::error!(%error, "failed to show participant overflow after player save");
        }
    }

    pub(crate) fn show_startup_player_activation_refusals(
        &mut self,
        refusals: &[PlayerActivationRefusal],
    ) -> Result<(), EngineError> {
        if refusals.is_empty() {
            return Ok(());
        }
        let template = self.runtime_resource_text(
            "IDS_ERR_PLAYERSTOOLONG",
            "Player \"%s\" has been deactivated: Too many activated players or path too long!",
        );
        let caption = self.runtime_resource_text("IDS_ERR_TITLE", "Error");
        for refusal in refusals {
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    format_resource_string(template.clone(), &[&refusal.player_name]),
                    caption.clone(),
                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                ),
                MessageDialogContinuation::None,
            )?;
        }
        Ok(())
    }

    pub(crate) fn open_startup_player_delete_dialog(
        &mut self,
        index: usize,
    ) -> Result<(), EngineError> {
        let delete = self
            .startup_player_files
            .get(index)
            .zip(self.startup_player_models.get(index))
            .map(|(player_file, player)| {
                (
                    player_file.path.clone(),
                    clonk_frontend::startup_plrsel::player_delete_warning(player),
                )
            });
        let Some((path, warning)) = delete else {
            tracing::error!(index, "player-delete action references a stale row");
            return Ok(());
        };
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                warning,
                "Delete",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::DeleteStartupPlayer { path },
        )
    }

    pub(crate) fn enter_startup_crew_mode(
        &mut self,
        player_index: usize,
    ) -> Result<(), EngineError> {
        self.abort_startup_crew_rename();
        let Some(player) = self.startup_player_files.get(player_index) else {
            tracing::error!(player_index, "crew action references a stale player row");
            return Ok(());
        };
        let player_name = player.render_model.name.clone();
        let mut crew = match discover_crew_files(player) {
            Ok(crew) => crew,
            Err(error) => {
                tracing::error!(%error, path = %player.path.display(), "failed to open startup crew");
                return Ok(());
            }
        };
        self.hydrate_startup_crew_models(&mut crew);
        if crew.is_empty() {
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    format!("{player_name} does not have a crew yet!"),
                    format!("Crew: {player_name}"),
                    clonk_frontend::message_dialog::MessageDialogIcon::PLAYER,
                ),
                MessageDialogContinuation::None,
            )?;
            return Ok(());
        }

        let participations = crew
            .iter()
            .map(|entry| entry.render_model.participating)
            .collect();
        let entered = self.startup_player_dialog.as_mut().is_some_and(|dialog| {
            dialog.enter_crew_mode(player_index, player_name, participations)
        });
        if !entered {
            tracing::error!(player_index, "startup crew mode transition was rejected");
            return Ok(());
        }
        // UpdatePlayerList replaces the row controls on this same dialog.
        self.startup_tooltip.pointer_left();
        self.startup_crew_models = crew
            .iter()
            .map(|entry| entry.render_model.clone())
            .collect();
        self.startup_crew_files = crew;
        self.startup_crew_player_index = Some(player_index);
        self.plrsel_last_click = None;
        self.status_text.clear();
        self.play_ui_sound("DoorOpen");
        Ok(())
    }

    pub(crate) fn leave_startup_crew_mode(&mut self) {
        self.abort_startup_crew_rename();
        self.close_context_menu_silently();
        if self
            .startup_player_dialog
            .as_mut()
            .and_then(|dialog| dialog.leave_crew_mode())
            .is_none()
        {
            return;
        }
        self.startup_tooltip.pointer_left();
        self.startup_crew_files.clear();
        self.startup_crew_models.clear();
        self.startup_crew_player_index = None;
        self.plrsel_last_click = None;
        self.play_ui_sound("DoorClose");
    }

    fn reload_startup_crew_list(&mut self, select_file: Option<&str>) -> io::Result<()> {
        let player_index = self.startup_crew_player_index.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "startup crew player is unavailable",
            )
        })?;
        let player = self.startup_player_files.get(player_index).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "startup crew player row is stale")
        })?;
        let mut crew = discover_crew_files(player)?;
        self.hydrate_startup_crew_models(&mut crew);
        self.startup_tooltip.pointer_left();
        let selected = select_file.and_then(|file_name| {
            crew.iter()
                .position(|entry| entry.file_name.eq_ignore_ascii_case(file_name))
        });
        self.startup_crew_models = crew
            .iter()
            .map(|entry| entry.render_model.clone())
            .collect();
        self.startup_crew_files = crew;
        if let Some(dialog) = self.startup_player_dialog.as_mut() {
            dialog.set_crew_participations(
                self.startup_crew_models
                    .iter()
                    .map(|entry| entry.participating)
                    .collect(),
            );
            dialog.set_selected_index(
                selected.or_else(|| (!self.startup_crew_models.is_empty()).then_some(0)),
            );
        }
        self.plrsel_last_click = None;
        Ok(())
    }

    fn show_startup_crew_error(
        &mut self,
        message: impl Into<String>,
        caption: impl Into<String>,
    ) -> Result<(), EngineError> {
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            ),
            MessageDialogContinuation::None,
        )
    }

    pub(crate) fn set_startup_crew_participation(
        &mut self,
        index: usize,
        participating: bool,
    ) -> Result<(), EngineError> {
        let Some((player_path, file_name)) = self
            .startup_crew_files
            .get(index)
            .map(|entry| (entry.player_path.clone(), entry.file_name.clone()))
        else {
            tracing::error!(index, "crew-participation action references a stale row");
            return Ok(());
        };
        match set_crew_participation(
            &player_path,
            &file_name,
            participating,
            self.process_group_maker.as_bytes(),
        ) {
            Ok(()) => {
                if let Some(entry) = self.startup_crew_files.get_mut(index) {
                    entry.crew_info.participation = i32::from(participating);
                    entry.render_model.participating = participating;
                }
                if let Some(model) = self.startup_crew_models.get_mut(index) {
                    model.participating = participating;
                }
            }
            Err(error) => {
                tracing::error!(%error, path = %player_path.display(), %file_name, "failed to rewrite crew participation");
                if let Some(dialog) = self.startup_player_dialog.as_mut() {
                    dialog.set_crew_participations(
                        self.startup_crew_models
                            .iter()
                            .map(|entry| entry.participating)
                            .collect(),
                    );
                    dialog.set_selected_index(Some(index));
                }
                self.show_startup_crew_error("File modification failure.", "")?;
            }
        }
        Ok(())
    }

    pub(crate) fn open_startup_crew_delete_dialog(
        &mut self,
        index: usize,
    ) -> Result<(), EngineError> {
        let delete = self.startup_crew_files.get(index).map(|entry| {
            (
                entry.player_path.clone(),
                entry.file_name.clone(),
                clonk_frontend::startup_plrsel::crew_delete_warning(&entry.render_model),
            )
        });
        let Some((player_path, file_name, warning)) = delete else {
            tracing::error!(index, "crew-delete action references a stale row");
            return Ok(());
        };
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                warning,
                "Delete",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::DeleteStartupCrew {
                player_path,
                file_name,
            },
        )
    }

    pub(crate) fn startup_crew_rename_context_entries(
        &self,
        clipboard_available: bool,
    ) -> Vec<ContextMenuEntry<AppContextMenuCommand>> {
        use clonk_frontend::startup_netdlg::NetDlgEditContextCommand as Command;

        let Some(edit) = self.startup_crew_rename.as_ref().map(|rename| &rename.edit) else {
            return Vec::new();
        };
        let item = |command, label_key, label, tooltip_key, tooltip| {
            ContextMenuEntry::new(self.runtime_resource_text(label_key, label))
                .with_tooltip(self.runtime_resource_text(tooltip_key, tooltip))
                .with_icon(ContextMenuIcon::None)
                .with_action(AppContextMenuCommand::StartupCrewRename(command))
        };
        let has_selection = edit.selection_range().is_some();
        let mut entries = Vec::new();
        if has_selection {
            entries.push(item(
                Command::Cut,
                "IDS_DLG_CUT",
                "Cut",
                "IDS_DLGTIP_CUT",
                "Moves the selection to the clipboard.",
            ));
            entries.push(item(
                Command::Copy,
                "IDS_DLG_COPY",
                "Copy",
                "IDS_DLGTIP_COPY",
                "Copies the selection to the clipboard.",
            ));
        }
        if clipboard_available {
            entries.push(item(
                Command::Paste,
                "IDS_DLG_PASTE",
                "Paste",
                "IDS_DLGTIP_PASTE",
                "Inserts the contents of the clipboard.",
            ));
        }
        if has_selection {
            entries.push(item(
                Command::Clear,
                "IDS_DLG_CLEAR",
                "Clear",
                "IDS_DLGTIP_CLEAR",
                "Clears the selection.",
            ));
        }
        let whole_text_selected = edit
            .selection_range()
            .is_some_and(|range| range.start == 0 && range.end == edit.text().len());
        if !edit.text().is_empty() && !whole_text_selected {
            entries.push(item(
                Command::SelectAll,
                "IDS_DLG_SELALL",
                "Select all",
                "IDS_DLGTIP_SELALL",
                "Selects the complete text",
            ));
        }
        entries
    }

    pub(crate) fn open_startup_crew_rename_context_menu(
        &mut self,
        anchor: GuiPoint,
    ) -> Result<bool, EngineError> {
        if self.startup_crew_rename.is_none() {
            return Ok(false);
        }
        let entries = self.startup_crew_rename_context_entries(clipboard_text_available());
        self.open_context_menu_at(entries, anchor)
    }

    pub(crate) fn execute_startup_crew_rename_context_command(
        &mut self,
        command: clonk_frontend::startup_netdlg::NetDlgEditContextCommand,
    ) {
        use clonk_frontend::startup_netdlg::NetDlgEditContextCommand as Command;

        let Some(rename) = self.startup_crew_rename.as_mut() else {
            tracing::error!(?command, "stale startup crew rename context command");
            return;
        };
        match command {
            Command::Cut | Command::Copy => {
                let cut = command == Command::Cut;
                if let Err(error) = transfer_edit_selection(&mut rename.edit, cut, |selected| {
                    arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(selected.to_string()))
                }) {
                    tracing::warn!(%error, "failed to copy startup crew rename text");
                }
            }
            Command::Paste => {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                    Ok(text) => {
                        rename.edit.insert_text(&text);
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to paste startup crew rename text");
                    }
                }
            }
            Command::Clear => {
                rename.edit.delete_selection();
            }
            Command::SelectAll => rename.edit.select_all(),
        }
    }

    pub(crate) fn start_startup_crew_rename(&mut self, index: usize) -> Result<(), EngineError> {
        if self.startup_crew_rename.is_some() {
            return Ok(());
        }
        let Some((initial_text, player_path, file_name)) = self
            .startup_crew_models
            .get(index)
            .zip(self.startup_crew_files.get(index))
            .map(|(model, file)| {
                (
                    model.name.clone(),
                    file.player_path.clone(),
                    file.file_name.clone(),
                )
            })
        else {
            tracing::error!(index, "crew-rename action references a stale row");
            return Ok(());
        };
        let Some(previous_focus) = self
            .startup_player_dialog
            .as_ref()
            .filter(|dialog| dialog.is_crew_mode())
            .map(|dialog| dialog.focused_control())
        else {
            tracing::error!(index, "crew-rename action received outside crew mode");
            return Ok(());
        };
        self.close_context_menu_silently();
        self.startup_tooltip.pointer_left();
        self.startup_crew_rename = Some(StartupCrewRenameState {
            index,
            player_path,
            file_name,
            edit: RenameEdit::new(initial_text, Some(previous_focus)),
            last_click: None,
            ignore_pointer_up: false,
        });
        Ok(())
    }

    pub(crate) fn restore_startup_crew_focus(&mut self, focus: Option<PlrSelControl>) {
        if let (Some(dialog), Some(focus)) = (self.startup_player_dialog.as_mut(), focus) {
            if dialog.is_crew_mode() {
                dialog.restore_focus(focus);
            }
        }
    }

    pub(crate) fn abort_startup_crew_rename(&mut self) -> bool {
        let Some(mut rename) = self.startup_crew_rename.take() else {
            return false;
        };
        rename.edit.abort();
        let previous_focus = rename.edit.take_previous_focus();
        self.restore_startup_crew_focus(previous_focus);
        true
    }

    fn resolve_startup_crew_rename(&mut self, result: RenameEditResult) -> bool {
        let Some(rename) = self.startup_crew_rename.as_mut() else {
            return false;
        };
        if rename.edit.resolve(result) == RenameEditResolution::KeepEditing {
            return false;
        }
        let mut rename = self
            .startup_crew_rename
            .take()
            .expect("finished crew rename state remains installed");
        let previous_focus = rename.edit.take_previous_focus();
        self.restore_startup_crew_focus(previous_focus);
        true
    }

    pub(crate) fn commit_startup_crew_rename(
        &mut self,
        focus_lost: bool,
    ) -> Result<bool, EngineError> {
        let Some((index, player_path, old_file_name, original_name, action)) =
            self.startup_crew_rename.as_mut().map(|rename| {
                (
                    rename.index,
                    rename.player_path.clone(),
                    rename.file_name.clone(),
                    rename.edit.label_text().to_string(),
                    if focus_lost {
                        rename.edit.focus_lost()
                    } else {
                        rename.edit.finish_input()
                    },
                )
            })
        else {
            return Ok(false);
        };
        let RenameEditAction::Submit(new_name) = action else {
            self.abort_startup_crew_rename();
            return Ok(true);
        };
        if new_name == original_name {
            self.resolve_startup_crew_rename(RenameEditResult::Accepted);
            return Ok(true);
        }

        let new_file_name = crew_file_name_for_title(&new_name);
        match rename_crew(
            &player_path,
            &old_file_name,
            &new_name,
            self.process_group_maker.as_bytes(),
        ) {
            Ok(persisted_file_name) => {
                self.resolve_startup_crew_rename(RenameEditResult::Accepted);
                let reload_error = self
                    .reload_startup_crew_list(Some(persisted_file_name.as_str()))
                    .err();
                self.reconcile_startup_crew_rename_model(
                    index,
                    &player_path,
                    &old_file_name,
                    &persisted_file_name,
                    &new_name,
                    reload_error.is_some(),
                );
                if let Some(error) = reload_error {
                    tracing::error!(%error, "failed to reload renamed startup crew");
                    let message =
                        self.runtime_resource_text("IDS_FAIL_MODIFY", "File modification failure.");
                    self.show_startup_crew_error(message, "")?;
                }
                Ok(true)
            }
            Err(StartupCrewMutationError::NameCollision { file_name }) => {
                self.resolve_startup_crew_rename(RenameEditResult::Invalid);
                let template = self.runtime_resource_text(
                    "IDS_ERR_CLONKCOLLISION",
                    "A Clonk with the file name \"%s\" exists already.",
                );
                let caption = self.runtime_resource_text("IDS_FAIL_RENAME", "Rename failure.");
                self.show_startup_crew_error(
                    format_resource_string(template, &[&file_name]),
                    caption,
                )?;
                Ok(false)
            }
            Err(StartupCrewMutationError::RenameAcceptedCoreRewriteFailed {
                file_name,
                source,
            }) => {
                tracing::error!(
                    error = %source,
                    path = %player_path.display(),
                    %old_file_name,
                    persisted_file_name = %file_name,
                    "startup crew filename changed but its core rewrite failed"
                );
                self.accept_startup_crew_rename_after_rewrite_failure(
                    index,
                    &player_path,
                    &old_file_name,
                    &file_name,
                    &new_name,
                )?;
                Ok(true)
            }
            Err(error) => {
                tracing::error!(%error, path = %player_path.display(), %old_file_name, %new_file_name, "failed to rename startup crew");
                self.resolve_startup_crew_rename(RenameEditResult::Invalid);
                let template = self.runtime_resource_text(
                    "IDS_ERR_RENAMEFILE",
                    "Error renaming file \"%s\" to \"%s\".",
                );
                let caption = self.runtime_resource_text("IDS_FAIL_RENAME", "Rename failure.");
                self.show_startup_crew_error(
                    format_resource_string(template, &[&old_file_name, &new_file_name]),
                    caption,
                )?;
                Ok(false)
            }
        }
    }

    pub(crate) fn accept_startup_crew_rename_after_rewrite_failure(
        &mut self,
        original_index: usize,
        player_path: &Path,
        old_file_name: &str,
        persisted_file_name: &str,
        new_name: &str,
    ) -> Result<(), EngineError> {
        self.resolve_startup_crew_rename(RenameEditResult::Accepted);
        let reload_error = self
            .reload_startup_crew_list(Some(persisted_file_name))
            .err();
        if let Some(error) = reload_error.as_ref() {
            tracing::error!(%error, "failed to reload startup crew after partial rename");
        }
        self.reconcile_startup_crew_rename_model(
            original_index,
            player_path,
            old_file_name,
            persisted_file_name,
            new_name,
            reload_error.is_some(),
        );

        let message = self.runtime_resource_text("IDS_FAIL_MODIFY", "File modification failure.");
        self.show_startup_crew_error(message, "")?;
        Ok(())
    }

    fn reconcile_startup_crew_rename_model(
        &mut self,
        original_index: usize,
        player_path: &Path,
        old_file_name: &str,
        persisted_file_name: &str,
        new_name: &str,
        allow_index_fallback: bool,
    ) {
        let row = self
            .startup_crew_files
            .iter()
            .position(|entry| {
                entry.player_path == player_path
                    && entry.file_name.eq_ignore_ascii_case(persisted_file_name)
            })
            .or_else(|| {
                self.startup_crew_files.iter().position(|entry| {
                    entry.player_path == player_path
                        && entry.file_name.eq_ignore_ascii_case(old_file_name)
                })
            })
            .or_else(|| {
                (allow_index_fallback && original_index < self.startup_crew_files.len())
                    .then_some(original_index)
            });

        if let Some(index) = row {
            let mut core_name = clonk_script::c4_string_bytes(new_name);
            if let Some(nul) = core_name.iter().position(|byte| *byte == 0) {
                core_name.truncate(nul);
            }
            core_name.truncate(30);
            if let Some(entry) = self.startup_crew_files.get_mut(index) {
                entry.file_name = persisted_file_name.to_string();
                entry.crew_info.name = clonk_script::c4_string_from_bytes(&core_name);
                entry.render_model.name = new_name.to_string();
            }
            if let Some(model) = self.startup_crew_models.get_mut(index) {
                model.name = new_name.to_string();
            }
            if let Some(dialog) = self.startup_player_dialog.as_mut() {
                dialog.set_selected_index(Some(index));
            }
        } else {
            tracing::error!(
                path = %player_path.display(),
                %old_file_name,
                %persisted_file_name,
                "renamed startup crew row could not be reconciled"
            );
        }
    }

    pub(crate) fn open_startup_crew_death_message_dialog(
        &mut self,
        index: usize,
    ) -> Result<(), EngineError> {
        let Some(initial_text) = self.startup_crew_files.get(index).map(|entry| {
            clonk_resources::decode_legacy_script_text(&clonk_script::c4_string_bytes(
                &entry.crew_info.death_message,
            ))
        }) else {
            tracing::error!(index, "crew death-message action references a stale row");
            return Ok(());
        };
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "Crew death-message input",
            self.assets.input_dialog_resources().map(|_| ()),
        )?;
        self.close_context_menu_silently();
        let controller = InputDialogController::new(
            "Enter new death message:",
            "Set death message",
            InputDialogIcon::COMMENT,
        )
        .with_max_text(75)
        .with_input_text(&initial_text);
        self.startup_tooltip.pointer_left();
        self.game_option_input_dialog = Some(PendingGameOptionInputDialog {
            purpose: PendingInputDialogPurpose::StartupCrew(
                PendingCrewInputAction::SetDeathMessage { index },
            ),
            controller,
        });
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        Ok(())
    }

    pub(crate) fn complete_startup_crew_input(
        &mut self,
        action: PendingCrewInputAction,
        text: String,
    ) -> Result<(), EngineError> {
        match action {
            PendingCrewInputAction::SetDeathMessage { index } => {
                let Some((player_path, file_name)) = self
                    .startup_crew_files
                    .get(index)
                    .map(|entry| (entry.player_path.clone(), entry.file_name.clone()))
                else {
                    tracing::error!(index, "accepted crew death message references a stale row");
                    return Ok(());
                };
                let result = set_crew_death_message(
                    &player_path,
                    &file_name,
                    &text,
                    self.process_group_maker.as_bytes(),
                );
                if result.is_ok() {
                    if let Err(error) = self.reload_startup_crew_list(Some(&file_name)) {
                        tracing::error!(%error, "failed to reload startup crew death message");
                    }
                }
                // Native feedback is unconditional after RewriteCore.
                self.play_ui_sound("Connect");
                if let Err(error) = result {
                    tracing::error!(%error, path = %player_path.display(), %file_name, "failed to rewrite crew death message");
                    self.show_startup_crew_error("File modification failure.", "")?;
                }
            }
        }
        Ok(())
    }

    fn hydrate_startup_crew_models(&self, crew: &mut [StartupCrewFile]) {
        let rank_names = self.default_rank_names.as_deref().unwrap_or_default();
        let rank_sheet = self.assets.hud_graphics.rank.as_ref();
        for entry in crew {
            let next_rank =
                entry
                    .crew_info
                    .core
                    .next_rank_info(entry.crew_info.rank, rank_names, 1_000);
            entry.render_model.next_rank = (next_rank.promotion_possible())
                .then(|| {
                    next_rank
                        .name
                        .map(|name| clonk_frontend::startup_plrsel::PlrSelCrewPromotion {
                            rank_name: clonk_resources::decode_legacy_script_text(
                                &clonk_script::c4_string_bytes(name),
                            ),
                            experience: next_rank.experience,
                        })
                })
                .flatten();
            entry.render_model.birthday = format_startup_crew_birthday(entry.crew_info.birthday);
            if entry.render_model.rank_icon.is_none() {
                entry.render_model.rank_icon =
                    rank_sheet.and_then(|sheet| startup_rank_icon(sheet, entry.crew_info.rank));
            }
        }
    }

    pub(crate) fn handle_main_menu_activation(
        &mut self,
        item: MainMenuItem,
    ) -> Result<(), EngineError> {
        match item {
            MainMenuItem::LocalGame => {
                self.begin_startup_dialog_fade(StartupDialog::ScenarioBrowser(
                    ScenarioSelectorMode::Local,
                ));
                self.open_scenario_browser();
            }
            MainMenuItem::NetworkGame => {
                if self.network_mode.is_some() && self.network_lobby.is_some() {
                    self.open_network_lobby();
                } else {
                    self.begin_startup_dialog_fade(StartupDialog::NetworkGame);
                    self.open_network_game_dialog();
                }
            }
            MainMenuItem::PlayerSelection => {
                self.begin_startup_dialog_fade(StartupDialog::PlayerSelection);
                self.open_player_selection_dialog();
            }
            MainMenuItem::Options => {
                self.begin_startup_dialog_fade(StartupDialog::Options);
                self.open_options_menu();
            }
            MainMenuItem::About => {
                self.begin_startup_dialog_fade(StartupDialog::About);
                self.open_about_dialog();
            }
            MainMenuItem::Quit => {
                self.request_exit("the main menu Quit item");
            }
        }
        Ok(())
    }

    pub(crate) fn visible_startup_dialog(&self) -> Option<StartupDialog> {
        match self.startup_view {
            StartupView::MainMenu => Some(StartupDialog::MainMenu),
            StartupView::ScenarioBrowser => Some(StartupDialog::ScenarioBrowser(self.scensel.mode)),
            StartupView::NetworkGame => Some(StartupDialog::NetworkGame),
            StartupView::Options => Some(StartupDialog::Options),
            StartupView::About => Some(StartupDialog::About),
            StartupView::PlayerSelection => Some(StartupDialog::PlayerSelection),
            // C4GameLobby is a game state, not a C4Startup dialog.
            StartupView::NetworkLobby => None,
        }
    }

    fn render_inactive_startup_dialog_layer(&mut self, frame: &mut [u8]) -> Result<()> {
        let scenario_loading_label = self.scenario_selector_loading_label();
        let gamma = self.startup_fragment_gamma();
        render_startup_frame(
            &mut self.graphics,
            self.assets.as_ref(),
            &mut self.main_menu_state,
            &mut self.menu_state,
            &self.scensel.entry_enabled,
            scenario_loading_label.as_deref(),
            self.startup_network_dialog.as_ref(),
            self.startup_player_dialog.as_ref(),
            &self.startup_player_models,
            &self.startup_crew_models,
            self.startup_crew_rename.as_mut(),
            None,
            true,
            true,
            true,
            true,
            &self.scenario_game_options,
            self.scensel.mode,
            self.startup_options_dialog.as_ref(),
            None,
            false,
            self.startup_about_dialog.as_ref(),
            self.startup_view,
            None,
            self.startup_view_flags,
            &mut self.menu_backdrop_cache,
            false,
            &gamma,
            frame,
        )
    }

    fn capture_startup_dialog_fade_layers(&mut self) -> Result<StartupDialogFadeLayers> {
        let (width, height) = {
            let surface = self.graphics.surface();
            (surface.width(), surface.height())
        };
        let mut underlay = vec![0_u8; width as usize * height as usize * 4];
        let gamma = self.startup_fragment_gamma();
        render_startup_underlay(
            &mut self.graphics,
            self.assets.as_ref(),
            &gamma,
            &mut underlay,
        );
        let mut outgoing = vec![0_u8; underlay.len()];
        self.render_inactive_startup_dialog_layer(&mut outgoing)?;

        let (outgoing_native, outgoing_native_text) = if self.native_startup_fonts.is_some() {
            self.begin_native_text_capture(false);
            let mut outgoing_native = vec![0_u8; underlay.len()];
            let native_result = self.render_inactive_startup_dialog_layer(&mut outgoing_native);
            let outgoing_native_text = self.graphics.surface_mut().take_clonk_text_capture();
            native_result?;
            (outgoing_native, outgoing_native_text)
        } else {
            // A headless/logical CPU fade never replays physical glyphs. Do
            // not precompute a semantic layer that cannot be presented; an
            // actual retained request will still fail its capture preflight.
            (outgoing.clone(), Vec::new())
        };

        self.graphics.begin_gpu_scene_capture();
        let mut ignored_underlay_pixel = [0_u8; 4];
        render_startup_underlay(
            &mut self.graphics,
            self.assets.as_ref(),
            &gamma,
            &mut ignored_underlay_pixel,
        );
        let underlay_gpu_recorder = self.graphics.surface_mut().take_gpu_scene_capture();

        self.pending_native_presentation = None;
        let mut ignored_outgoing_pixel = [0_u8; 4];
        let outgoing_gpu_plan = if self.native_startup_fonts.is_some() {
            self.retained_gpu_ordered_capture_active = true;
            let retained_result = self.render_ordered_native_base(&mut ignored_outgoing_pixel);
            self.retained_gpu_ordered_capture_active = false;
            if let Err(error) = retained_result {
                self.pending_native_presentation = None;
                return Err(error);
            }
            self.pending_native_presentation.take()
        } else {
            None
        };

        Ok(StartupDialogFadeLayers {
            width,
            height,
            underlay,
            outgoing_frame: outgoing,
            outgoing_native_frame: outgoing_native,
            outgoing_native_text,
            underlay_gpu_recorder,
            outgoing_gpu_plan,
        })
    }

    pub(crate) fn begin_startup_dialog_fade(&mut self, incoming: StartupDialog) {
        let Some(outgoing) = self.visible_startup_dialog() else {
            return;
        };
        if self.mode != AppMode::Menu {
            return;
        }
        self.pointer_left_unchecked();
        let layers = match self.capture_startup_dialog_fade_layers() {
            Ok(layers) => layers,
            Err(error) => {
                tracing::error!(%error, "failed to capture outgoing startup dialog fade layer");
                return;
            }
        };
        self.startup_dialog_fade = Some(StartupDialogFade {
            outgoing: Some(outgoing),
            incoming,
            step: 0,
            width: layers.width,
            height: layers.height,
            underlay: layers.underlay,
            outgoing_frame: Some(layers.outgoing_frame),
            outgoing_native_frame: Some(layers.outgoing_native_frame),
            outgoing_native_text: layers.outgoing_native_text,
            outgoing_native_fonts: self.native_startup_fonts.clone(),
            underlay_gpu_recorder: layers.underlay_gpu_recorder,
            outgoing_gpu_plan: layers.outgoing_gpu_plan,
        });
        self.startup_tooltip.pointer_left();
    }

    pub(crate) fn begin_startup_dialog_fade_in(&mut self) {
        let Some(incoming) = self.visible_startup_dialog() else {
            return;
        };
        if self.mode != AppMode::Menu {
            return;
        }
        self.pointer_left_unchecked();
        let (width, height) = {
            let surface = self.graphics.surface();
            (surface.width(), surface.height())
        };
        let mut underlay = vec![0_u8; width as usize * height as usize * 4];
        let gamma = self.startup_fragment_gamma();
        render_startup_underlay(
            &mut self.graphics,
            self.assets.as_ref(),
            &gamma,
            &mut underlay,
        );
        self.graphics.begin_gpu_scene_capture();
        let mut ignored_underlay_pixel = [0_u8; 4];
        render_startup_underlay(
            &mut self.graphics,
            self.assets.as_ref(),
            &gamma,
            &mut ignored_underlay_pixel,
        );
        let underlay_gpu_recorder = self.graphics.surface_mut().take_gpu_scene_capture();
        self.startup_dialog_fade = Some(StartupDialogFade {
            outgoing: None,
            incoming,
            step: 0,
            width,
            height,
            underlay,
            outgoing_frame: None,
            outgoing_native_frame: None,
            outgoing_native_text: Vec::new(),
            outgoing_native_fonts: None,
            underlay_gpu_recorder,
            outgoing_gpu_plan: None,
        });
        self.startup_tooltip.pointer_left();
    }

    pub(crate) fn replace_startup_view(&mut self, view: StartupView) {
        // Destroying/replacing a native dialog clears CMouse's owned
        // pMouseOverElement. Retaining only the screen coordinate would
        // otherwise invent a target in the new view without mouse input.
        self.startup_tooltip.pointer_left();
        if view != StartupView::Options {
            self.startup_options_advanced_dialog = None;
            self.gamepads.set_options_open_slot(None);
        }
        if view != StartupView::ScenarioBrowser {
            self.cancel_scenario_selector_discovery();
        }
        self.startup_view = view;
        let keeps_pending_fade = self
            .visible_startup_dialog()
            .zip(self.startup_dialog_fade.as_ref())
            .is_some_and(|(dialog, fade)| dialog == fade.incoming);
        if !keeps_pending_fade {
            self.startup_dialog_fade = None;
        }
    }

    pub(crate) fn replace_startup_dialog(&mut self, view: StartupView, dialog: StartupDialog) {
        self.last_startup_dialog = dialog;
        self.replace_startup_view(view);
    }

    pub(crate) fn restore_startup_dialog(&mut self, dialog: StartupDialog) {
        if !matches!(dialog, StartupDialog::MainMenu) {
            // `show_main_menu` is also the teardown hub and may have opened
            // its first-player prompt. Native DoStartup constructs only the
            // remembered non-Main dialog, so that Main-only child must not
            // leak above the restored screen.
            self.startup_player_properties_dialog = None;
        }
        match dialog {
            StartupDialog::MainMenu => {}
            StartupDialog::ScenarioBrowser(mode) => {
                self.open_scenario_browser_with_mode(mode);
                self.startup_scenario_back_dialog = None;
            }
            StartupDialog::NetworkGame => self.open_network_game_dialog(),
            StartupDialog::Options => self.open_options_menu(),
            StartupDialog::About => self.open_about_dialog(),
            StartupDialog::PlayerSelection => self.open_player_selection_dialog(),
        }
    }

    pub(crate) fn show_main_menu(&mut self) {
        if let Some(audio) = self.sound.context.as_ref() {
            let mut audio = audio.borrow_mut();
            audio.stop_lobby_elevator();
        }
        self.restore_startup_fonts();
        self.restore_startup_gui_sheets();
        self.active_global_gui_failures.clear();
        self.chat.running = None;
        self.runtime_client_list = None;
        self.running_dialog_stack.clear();
        self.running_active_dialog = None;
        self.runtime_client_list_consumed_keys.clear();
        self.hide_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
        self.message_input_history.clear();
        self.close_context_menu_silently();
        self.abort_startup_crew_rename();
        self.startup_player_properties_dialog = None;
        self.game_option_input_dialog = None;
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        self.game_option_pointer_capture = false;
        self.game_option_consumed_keys.clear();
        self.scenario_game_options.cancel_interaction();
        self.definition_selector = None;
        self.pending_definition_selection = None;
        self.pending_lobby_player_selection = None;
        self.definition_selector_last_click = None;
        self.definition_selector_consumed_keys.clear();
        self.definition_selector_pointer_capture = false;
        self.clear_pending_league_player_auth();
        self.startup_network_connection = None;
        self.startup_game_search = None;
        self.startup_network_last_refresh = None;
        self.startup_masterserver_next_query_at = None;
        self.startup_masterserver_request_timeout_at = None;
        self.startup_network_refresh_waiting_for_clear = false;
        self.startup_game_references.clear();
        self.startup_discovery_reference_queries.clear();
        self.startup_direct_reference_queries.clear();
        self.netdlg_last_click = None;
        self.netdlg_join_edit_last_click = None;
        self.netdlg_edit_consumed_keys.clear();
        self.pending_network_join = None;
        self.network_game_advertiser = None;
        self.advertised_game_reference = None;
        self.host_reference_paused = false;
        self.runtime_network_control_mode = None;
        self.runtime_network_committed_control_mode = None;
        self.runtime_network_committed_status = None;
        self.runtime_network_join_allowed = None;
        if self.startup_view == StartupView::NetworkLobby {
            self.control_messages.clear_clients();
            self.network_lobby = None;
            self.classic_host_lobby = None;
            self.network_start_wait = None;
            self.staged_network_host_scenario = None;
            self.network_lobby_min_players = None;
            self.clear_lobby_preload();
            self.host_lobby_countdown = None;
            self.pending_local_lobby_countdown_echoes.clear();
            self.reinitialize_startup_loader_screen();
            self.abandon_live_masterserver_signup();
            self.network = None;
            self.network_mode = None;
            self.host_join_snapshot = None;
            self.pending_runtime_dynamic_request = None;
            self.network_ticks.clear();
            self.network_sync.clear();
            self.offline_control_input.clear();
            self.sync_checks.clear();
            self.clear_blocking_resource_wait();
            self.admission_resources.clear();
            self.host_local_alternate_colors_by_resource.clear();
            self.host_local_player_info_ids.clear();
            self.executing_ready_tick = None;
            self.control_player_infos = ControlPlayerInfoRegistry::default();
            self.network_is_league = false;
            self.network_league_name.clear();
            self.network_stream_address = LegacyCString::default();
            seed_engine_player_info_parameters(
                &mut self.engine,
                &self.network_league_name,
                &self.control_player_infos,
            );
            self.network_control_running = true;
            self.runtime_network_status_barrier = None;
            self.control_clients = initial_control_clients(None, None);
            self.network_client_activity.clear();
            let values = self.scenario_game_option_values();
            self.scenario_game_options =
                GameOptionButtons::new(GameOptionContext::LocalSelector, values);
            self.sync_scenario_game_option_bounds();
        }
        self.startup_scenario_back_dialog = None;
        self.replace_startup_dialog(StartupView::MainMenu, StartupDialog::MainMenu);
        self.scensel.mode = ScenarioSelectorMode::Local;
        self.main_menu_state.pointer_left();
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.pointer_left();
        }
        let participants_validation = self
            .app_paths
            .as_ref()
            .map(validate_startup_participant_config);
        match participants_validation {
            Some(Ok(())) => self.sync_startup_participant_models(),
            // Preserve legacy-byte configuration instead of corrupting it
            // through the UTF-8-only convenience model.
            Some(Err(error)) if error.kind() == io::ErrorKind::InvalidData => {}
            Some(Err(error)) => {
                tracing::warn!(%error, "failed to validate startup participants");
            }
            None => {}
        }
        self.refresh_participants_label();
        self.scenario_label = self.menu_state.label_path();
        self.status_text.clear();
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.pointer_left();
        }
        if let Some(dialog) = self.startup_player_dialog.as_mut() {
            dialog.pointer_left();
        }
        if let Some(dialog) = self.startup_options_dialog.as_mut() {
            dialog.pointer_left();
        }
        if let Some(dialog) = self.startup_about_dialog.as_mut() {
            dialog.pointer_left();
        }
        match self.app_paths.as_ref().map(startup_player_file_exists) {
            Some(Ok(false)) => self.open_new_startup_player_properties_from(
                StartupPlayerPropertiesOrigin::MainMenuFirstPlayer,
            ),
            Some(Err(error)) => {
                tracing::warn!(%error, "failed to scan startup player files");
                self.open_new_startup_player_properties_from(
                    StartupPlayerPropertiesOrigin::MainMenuFirstPlayer,
                );
            }
            Some(Ok(true)) | None => {}
        }
    }

    pub(crate) fn apply_scenario_loader_frame(&mut self, progress: i32, log: Option<Vec<String>>) {
        let Some(state) = self.loading_state.as_mut() else {
            return;
        };
        let (progress, log) = state.accept_loader_frame(progress, log);
        // `C4Game::SetInitProgress` mirrors each increasing percentage to the
        // window, which publishes it on the taskbar (C4Game.cpp:4102-4105).
        self.taskbar_progress
            .report(u32::try_from(progress).unwrap_or(0));
        if let Some(loader) = self.loader_screen.as_mut() {
            loader.update(LoaderUpdate::SetProgress(progress));
            if let Some(lines) = log {
                loader.update(LoaderUpdate::ReplaceLog(lines));
            }
        }
    }

    pub(crate) fn advance_scenario_loader(&mut self, progress: i32, line: &'static str) {
        let mut log = self
            .loading_state
            .as_ref()
            .map(|state| state.log.clone())
            .unwrap_or_default();
        if !line.is_empty() {
            if log.len() == SCENARIO_LOADING_LOG_CAPACITY {
                log.remove(0);
            }
            log.push(line.to_string());
        }
        self.apply_scenario_loader_frame(progress, Some(log));
    }

    pub(crate) fn poll_boot_loading(&mut self) {
        let mut material_library: Option<Option<Arc<MaterialSet>>> = None;
        if let Some(state) = self.boot_loading.as_mut() {
            match state.receiver.try_recv() {
                Ok(BootLoadingEvent::Finished(library)) => {
                    material_library = Some(library);
                }
                Err(TryRecvError::Empty) => {
                    // Still loading, do nothing
                }
                Err(TryRecvError::Disconnected) => {
                    tracing::warn!("boot loading channel disconnected");
                    material_library = Some(None);
                }
            }
        }

        if let Some(library) = material_library {
            self.boot_loading = None;
            self.material_library = library;
            self.apply_material_library();
            if !self.console_mode
                && !self.headless
                && self.loading_state.is_none()
                && !self.classic_loader_render_preconditions_ready()
            {
                // A fast boot worker must not bypass a failed loader before
                // the first redraw. Stay in Loading so render reports the
                // logged typed boundary.
                //
                // A dedicated server has no loader screen to report on:
                // `C4Application::PreInit` builds one only for a
                // startup-dialog run (C4Application.cpp:239), and a
                // `USE_CONSOLE` build has no `C4FontLoader` to build it with
                // (C4Game.h:132-135).
                return;
            }
            if self.auto_start_classic_command_line_scenario {
                self.auto_start_classic_command_line_scenario = false;
                let mut failed = false;
                if let Err(error) = self.launch_classic_command_line_scenario() {
                    tracing::error!(?error, "failed to start command-line scenario");
                    self.status_text = format!("Unable to start command-line scenario: {error}");
                    failed = true;
                }
                if failed && !self.startup_dialog_in_use() {
                    // ParseCommandLine disables the startup dialog for an
                    // explicit scenario, direct join or record stream
                    // (C4Game.cpp:3299), so their failed start unwinds
                    // `QuitGame` into `Quit()` rather than exposing a main menu
                    // this run never came from (C4Application.cpp:373-405).
                    self.request_exit("a command-line scenario failed to start");
                    return;
                }
                if failed
                    || (self.startup_network_connection.is_none() && self.loading_state.is_none())
                {
                    self.mode = AppMode::Menu;
                    self.show_main_menu();
                    self.begin_startup_dialog_fade_in();
                }
                return;
            }
            // A scenario load can be started before boot finishes (mode is
            // already `Loading`). Boot completion must NOT yank the app back to
            // the menu in that case: the `Menu` update arm does not poll scenario
            // loading, so doing so would strand the in-flight load forever. Stay
            // in `Loading` and let `poll_loading` carry the scenario to `Running`.
            if self.loading_state.is_none()
                && self.startup_network_connection.is_none()
                && self.classic_direct_reference_query.is_none()
            {
                self.mode = AppMode::Menu;
                if self.network_mode.is_some() && self.network_lobby.is_some() {
                    // A command-line host/client has already completed network
                    // initialization. C++ proceeds directly into DoLobby here;
                    // returning to the main menu would leave GS_Lobby unacked
                    // (src/C4Game.cpp:366-409; src/C4Network2.cpp:445-461).
                    self.open_network_lobby();
                } else {
                    self.show_main_menu();
                    self.begin_startup_dialog_fade_in();
                }
                self.begin_frontend_music_entry();
                // `C4StartupMainDlg::OnShown` (cpp:258-276). Both are consumed
                // unconditionally: C++ applies an incoming package and *then*
                // honours a requested check — the incoming branch is not an
                // `else if` — so neither may survive into the next boot.
                let incoming = self.incoming_update.take();
                let check_requested = std::mem::take(&mut self.update_check_requested);
                let had_incoming = incoming.is_some();
                if let Some(package) = incoming {
                    if let Err(err) = self.refuse_incoming_update(&package) {
                        tracing::warn!(error = ?err, "failed to report the refused update package");
                    }
                }
                // Manual by command line or url (cpp:265-269), otherwise the
                // once-a-day automatic check (cpp:270-275).
                if check_requested
                    || (self.automatic_update_check_allowed && self.automatic_update_enabled())
                {
                    if let Err(err) = self.check_for_updates(!check_requested) {
                        tracing::warn!(error = ?err, "failed to start the update check");
                    }
                }
                if had_incoming || check_requested {
                    return;
                }
                // `--sandbox`: jump straight into the built-in sandbox once boot
                // completes, so the in-game scene can be launched/captured without
                // navigating the menu. One-shot, so return_to_menu works after.
                if self.auto_start_sandbox {
                    self.auto_start_sandbox = false;
                    if let Err(err) = self.start_sandbox_scenario(FrontendScenario::fallback()) {
                        tracing::warn!(error = ?err, "failed to auto-start sandbox scenario");
                    }
                }
            }
        }
    }

    pub(crate) fn delete_startup_player_and_refresh(
        &mut self,
        path: &Path,
    ) -> Result<(), EngineError> {
        let deletion = delete_player_file(path);
        if let Err(error) = deletion.as_ref() {
            tracing::error!(path = %path.display(), %error, "failed to delete player file");
        }
        self.refresh_startup_player_list();
        if deletion.is_err() {
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Delete failure.",
                    "Clear",
                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                ),
                MessageDialogContinuation::None,
            )?;
        }
        Ok(())
    }

    pub(crate) fn delete_startup_crew_and_refresh(
        &mut self,
        player_path: &Path,
        file_name: &str,
    ) -> Result<(), EngineError> {
        let deletion =
            delete_crew_file(player_path, file_name, self.process_group_maker.as_bytes());
        if let Err(error) = deletion.as_ref() {
            tracing::error!(path = %player_path.display(), %file_name, %error, "failed to delete crew file");
        }
        if let Err(error) = self.reload_startup_crew_list(None) {
            tracing::error!(%error, "failed to refresh startup crew after deletion");
        }
        if deletion.is_err() {
            self.show_startup_crew_error("Delete failure.", "Clear")?;
        }
        Ok(())
    }

    pub(crate) fn refresh_startup_player_list(&mut self) {
        self.close_context_menu_silently();
        let Some(paths) = self.app_paths.as_ref() else {
            tracing::error!("cannot refresh startup players without application paths");
            return;
        };
        let mut players = match discover_player_files(paths) {
            Ok(players) => players,
            Err(error) => {
                tracing::error!(%error, "failed to rediscover startup players after deletion");
                Vec::new()
            }
        };
        self.startup_tooltip.pointer_left();
        let activation_refusals = match persist_activations(&paths.config_file(), &mut players) {
            Ok(refusals) => refusals,
            Err(error) => {
                tracing::warn!(%error, "failed to rebuild participants after player deletion");
                Vec::new()
            }
        };
        self.startup_player_models = players
            .iter()
            .map(|player| player.render_model.clone())
            .collect();
        self.selected_player_file = players
            .iter()
            .find(|player| player.render_model.activated)
            .map(|player| player.player_file.clone());
        self.startup_player_files = players;
        if let Some(dialog) = self.startup_player_dialog.as_mut() {
            dialog.set_player_activations(
                self.startup_player_models
                    .iter()
                    .map(|player| player.activated)
                    .collect(),
            );
        }
        self.plrsel_last_click = None;
        self.refresh_participants_label();
        self.status_text.clear();
        if let Err(error) = self.show_startup_player_activation_refusals(&activation_refusals) {
            tracing::error!(%error, "failed to show participant overflow after player deletion");
        }
    }

    pub(crate) fn startup_base_context_menu(
        context_menu: Option<&ClassicContextMenu<AppContextMenuCommand>>,
        game_option_input_open: bool,
    ) -> Option<&ClassicContextMenu<AppContextMenuCommand>> {
        context_menu.filter(|_| !game_option_input_open)
    }

    pub(crate) fn startup_element_tooltip_target_at(
        &self,
        point: GuiPoint,
    ) -> Option<StartupTooltip> {
        if self.mode != AppMode::Menu
            || self.startup_network_transition_active()
            || self.context_menu.is_some()
            || !self.message_dialogs.is_empty()
            || self
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible)
            || self.definition_selector.is_some()
            || self.game_option_input_dialog.is_some()
            || self.league_signup_dialog.is_some()
            || self.startup_options_advanced_dialog.is_some()
            || self.chat.external_dialog_visible
            || self.runtime_client_list.is_some()
        {
            return None;
        }
        if let Some(properties) = self.startup_player_properties_dialog.as_ref() {
            let book = self.assets.options_book_fonts.as_deref()?;
            return properties.controller.tooltip_at(point, &book.book_small);
        }
        if self.startup_dialog_fade_active() {
            return None;
        }

        match self.startup_view {
            StartupView::MainMenu => self.main_menu_state.tooltip_at(point),
            StartupView::ScenarioBrowser => self.scenario_browser_tooltip_target_at(point),
            StartupView::NetworkGame => self.network_game_tooltip_target_at(point),
            StartupView::Options => self.options_tooltip_target_at(point),
            StartupView::PlayerSelection => self.player_selection_tooltip_target_at(point),
            StartupView::About => self.about_tooltip_target_at(point),
            // The lobby keeps its existing controller-owned tooltip model.
            StartupView::NetworkLobby => None,
        }
    }

    pub(crate) fn startup_element_tooltip_pending(&self) -> bool {
        self.startup_tooltip
            .pending_pointer()
            .and_then(|point| self.startup_element_tooltip_target_at(point))
            .is_some()
    }

    fn render_startup_element_tooltip(&mut self) -> Result<bool> {
        let Some(pointer) = self.startup_tooltip.eligible_pointer() else {
            return Ok(false);
        };
        let Some(target) = self.startup_element_tooltip_target_at(pointer) else {
            return Ok(false);
        };
        let text = self.resolve_startup_tooltip_text(target);
        if text.is_empty() {
            return Ok(false);
        }
        let tooltip_font = self
            .assets
            .global_tooltip_font
            .as_deref()
            .context("classic shadowless tooltip font is unavailable")?;
        let gamma = self.startup_fragment_gamma();
        clonk_frontend::context_menu::draw_classic_tooltip(
            self.graphics.surface_mut(),
            tooltip_font,
            pointer,
            &text,
            Some(&gamma),
        );
        Ok(true)
    }

    pub(crate) fn render_startup_tooltips(&mut self) -> Result<bool> {
        let mut rendered = self.render_startup_element_tooltip()?;
        match self.startup_view {
            StartupView::NetworkLobby
                if self.classic_host_lobby.is_none()
                    && self.runtime_client_list.is_none()
                    && self.context_menu.is_none()
                    && self.definition_selector.is_none()
                    && self.game_option_input_dialog.is_none()
                    && self.league_signup_dialog.is_none()
                    && self.message_dialogs.is_empty()
                    && self.startup_player_properties_dialog.is_none()
                    && !self.chat.external_dialog_visible
                    && self
                        .network_start_wait
                        .as_ref()
                        .is_none_or(|wait| !wait.visible) =>
            {
                let assets = Arc::clone(&self.assets);
                let gamma = self.startup_fragment_gamma();
                if let Some(lobby) = self.network_lobby.as_mut() {
                    lobby.render_classic_tooltips(
                        self.graphics.surface_mut(),
                        assets.as_ref(),
                        &self.scenario_game_options,
                        &gamma,
                    )?;
                    rendered = true;
                }
            }
            _ => {}
        }
        Ok(rendered)
    }

    pub(crate) fn startup_active_gamma(&self) -> clonk_graphics::GammaRamp {
        if self.graphics.advanced_renderer_config().disable_gamma {
            startup_identity_gamma().clone()
        } else {
            self.loader_gamma
                .clone()
                .unwrap_or_else(|| startup_gamma().clone())
        }
    }

    pub(crate) fn startup_fragment_gamma(&self) -> clonk_graphics::GammaRamp {
        if self.graphics.fragment_gamma_enabled() {
            self.startup_active_gamma()
        } else {
            startup_identity_gamma().clone()
        }
    }

    pub(crate) fn startup_monitor_gamma(&self) -> Option<clonk_graphics::GammaRamp> {
        self.graphics
            .monitor_gamma_enabled()
            .then(|| self.startup_active_gamma())
    }

    pub(crate) fn render_native_main_menu_text(
        &self,
        frame: &mut [u8],
        frame_width: u32,
        frame_height: u32,
    ) -> Result<()> {
        let _renderer_config = clonk_frontend::activate_advanced_renderer_config(
            self.graphics.advanced_renderer_config(),
        );
        let gamma = self.startup_fragment_gamma();
        self.preflight_startup_presentation()?;
        self.preflight_visible_gui_overlay_resources()?;
        if self.startup_view != StartupView::MainMenu {
            return Ok(());
        }
        let Some(fonts) = self.native_startup_fonts.as_deref() else {
            return Ok(());
        };
        let Some(expected_len) = (frame_width as usize)
            .checked_mul(frame_height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return Ok(());
        };
        let Some(pixels) = frame.get(..expected_len) else {
            return Ok(());
        };
        let Ok(mut surface) = Surface::from_bytes(
            frame_width,
            frame_height,
            clonk_graphics::PixelFormat::Rgba8888,
            pixels.to_vec(),
        ) else {
            return Ok(());
        };
        let logical = self.graphics.surface();
        let viewport_width = scaled_viewport_extent(logical.width(), fonts.scale())
            .context("native main-menu viewport width overflow")?;
        let viewport_height = scaled_viewport_extent(logical.height(), fonts.scale())
            .context("native main-menu viewport height overflow")?;
        if frame_width > viewport_width || frame_height > viewport_height {
            anyhow::bail!(
                "native main-menu framebuffer {frame_width}x{frame_height} exceeds its {viewport_width}x{viewport_height} scaled viewport"
            );
        }
        let clipped_top = i32::try_from(viewport_height - frame_height)
            .context("native main-menu viewport offset exceeds C++ integers")?;
        let physical_offset = (0, -clipped_top);
        self.main_menu_state
            .render_native_text(&mut surface, fonts, physical_offset, Some(&gamma));

        let (width, height) = (logical.width() as i32, logical.height() as i32);
        if let Some(logo) = self.assets.logo() {
            let logo_height = (0.4 * logo.height() as f32) as i32;
            fonts.text.draw_to_physical_surface_with_offset(
                &mut surface,
                width * 39 / 40,
                height / 18 + logo_height,
                &format!("Version {}", clonk_core::version::PORT_VERSION),
                [255, 255, 255, 255],
                clonk_graphics::clonk_font::TextAlign::Right,
                true,
                physical_offset,
                Some(&gamma),
            );
        }
        frame[..expected_len].copy_from_slice(surface.pixels());
        Ok(())
    }

    pub(crate) fn preflight_startup_presentation(&self) -> Result<()> {
        if self.game_over_dialog.is_some() {
            return Err(anyhow::Error::new(report_classic_parity_boundary(
                ClassicParityBoundary::StartupGameOver {
                    view: self.startup_view,
                },
            )));
        }
        self.reject_classic_global_gui_bootstrap()?;
        self.reject_classic_startup_bootstrap()?;
        self.reject_generic_startup_view()?;
        self.reject_missing_startup_model()?;
        self.reject_unported_startup_subscreen()?;
        self.reject_generic_startup_status()
    }

    fn reject_classic_startup_bootstrap(&self) -> Result<()> {
        let mut issues = self.assets.classic_startup_bootstrap_issues();
        // A plain logical CPU render uses the already loaded CStdFont cells
        // directly and is also the headless/menu-dump oracle. Scale-native
        // atlases become mandatory only when retained/semantic capture must
        // lower text to stable textured glyph quads.
        let retained_text_capture = self.graphics.surface().is_clonk_text_capture_active()
            || self.graphics.surface().is_gpu_scene_capture_active();
        let scaled_output = self
            .loader_render_config
            .as_ref()
            .map(|config| (*config).application_scale())
            .filter(|scale| scale.is_finite() && *scale > 0.0 && retained_text_capture);
        if let Some(scale) = scaled_output {
            match self.native_startup_fonts.as_deref() {
                None => issues.push(ClassicStartupBootstrapIssue::missing(
                    "ScaleNativeStartupFonts",
                )),
                Some(fonts) if fonts.scale() != scale => {
                    issues.push(ClassicStartupBootstrapIssue::malformed(
                        "ScaleNativeStartupFonts",
                        "a font atlas matching the application scale",
                        format!("font scale {} for application scale {scale}", fonts.scale()),
                    ));
                }
                Some(_) => {}
            }
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(anyhow::Error::new(report_classic_parity_boundary(
                ClassicParityBoundary::StartupBootstrapResources { issues },
            )))
        }
    }

    fn reject_missing_startup_model(&self) -> Result<()> {
        let missing = match self.startup_view {
            StartupView::NetworkGame if self.startup_network_dialog.is_none() => {
                Some("C4StartupNetDlg")
            }
            StartupView::PlayerSelection if self.startup_player_dialog.is_none() => {
                Some("C4StartupPlrSelDlg")
            }
            StartupView::Options if self.startup_options_dialog.is_none() => {
                Some("C4StartupOptionsDlg")
            }
            StartupView::About if self.startup_about_dialog.is_none() => Some("C4StartupAboutDlg"),
            _ => None,
        };
        let Some(missing) = missing else {
            return Ok(());
        };
        Err(anyhow::Error::new(report_classic_parity_boundary(
            ClassicParityBoundary::StartupModel {
                view: self.startup_view,
                missing,
            },
        )))
    }

    fn reject_unported_startup_subscreen(&self) -> Result<()> {
        if self.startup_view == StartupView::Options
            && self.sound.context.is_none()
            && self.startup_options_dialog.as_ref().is_some_and(|dialog| {
                dialog.active_sheet() == clonk_frontend::startup_options_dlg::OptionsSheet::Sound
            })
        {
            return Err(anyhow::Error::new(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup Options Audio sheet",
                },
            )));
        }
        let subscreen = match self.startup_view {
            StartupView::Options => None,
            StartupView::About => None,
            StartupView::NetworkGame => None,
            _ => None,
        };
        let Some(subscreen) = subscreen else {
            return Ok(());
        };
        Err(anyhow::Error::new(report_classic_parity_boundary(
            ClassicParityBoundary::StartupSubscreen(subscreen),
        )))
    }

    fn reject_generic_startup_status(&self) -> Result<()> {
        if self.status_text.is_empty() {
            return Ok(());
        }
        Err(anyhow::Error::new(report_classic_parity_boundary(
            ClassicParityBoundary::StartupStatusOverlay {
                view: self.startup_view,
                status: self.status_text.clone(),
            },
        )))
    }

    fn reject_generic_startup_view(&self) -> Result<()> {
        if self.startup_view != StartupView::NetworkLobby
            || self.classic_host_lobby.is_some()
            || self.network_lobby.is_some()
        {
            return Ok(());
        }
        Err(anyhow::Error::new(report_classic_parity_boundary(
            ClassicParityBoundary::StartupScreen {
                view: self.startup_view,
            },
        )))
    }

    /// `C4Application::PreInit` re-runs
    /// `InitLoaderScreen(C4CFN_StartupBackgroundMain)` on every return to the
    /// startup dialog, so an abandoned or failed game never leaves the engine
    /// without a loader (`src/C4Application.cpp:242-247,373-389,418-421`).
    /// `C4Game::Init` relies on that: its join branch builds one only when
    /// `pLoaderScreen` is null (`src/C4Game.cpp:371-381`), and a joining
    /// client otherwise loads behind the retained startup background.
    /// `InitLoaderScreen` replaces the live screen only on success
    /// (`src/C4GraphicsSystem.cpp:301-311`); a failure is fatal there and is
    /// reported here through the same loader boundary the initial launch uses.
    pub(crate) fn reinitialize_startup_loader_screen(&mut self) {
        let Some(paths) = self.app_paths.as_ref() else {
            // Path-less state fixtures have no install to re-init from.
            self.loader_screen = None;
            self.loader_error = None;
            return;
        };
        match build_startup_loader(paths, self.assets.as_ref()) {
            Ok(setup) => {
                self.loader_screen = Some(setup.screen);
                self.loader_error = None;
            }
            Err(error) => {
                tracing::error!(%error, "classic startup loader reinitialization failed");
                self.loader_error = Some(error.to_string());
            }
        }
    }

    pub(crate) fn loader_boundary(&self, detail: impl Into<String>) -> anyhow::Error {
        let context = if self.loading_state.is_some() || self.terminal_loader_frame_pending {
            "scenario loading"
        } else {
            "startup loading"
        };
        anyhow::Error::new(report_classic_parity_boundary(
            ClassicParityBoundary::LoaderScreen {
                context,
                detail: detail.into(),
            },
        ))
    }

    pub(crate) fn render_native_loader_text(
        &self,
        frame: &mut [u8],
        frame_width: u32,
        frame_height: u32,
    ) -> Result<()> {
        let _renderer_config = clonk_frontend::activate_advanced_renderer_config(
            self.graphics.advanced_renderer_config(),
        );
        self.reject_classic_global_gui_bootstrap()?;
        let gamma = self.startup_fragment_gamma();
        let loader = self
            .loader_screen
            .as_ref()
            .ok_or_else(|| self.loader_boundary("no selected classic loader is installed"))?;
        let fonts = self
            .native_startup_fonts
            .as_deref()
            .ok_or_else(|| self.loader_boundary("scale-native loader fonts are unavailable"))?;
        let config = self
            .loader_render_config
            .ok_or_else(|| self.loader_boundary("loader render configuration is unavailable"))?;
        if fonts.scale() != config.application_scale() {
            return Err(self.loader_boundary(format!(
                "native font scale {} does not match loader scale {}",
                fonts.scale(),
                config.application_scale()
            )));
        }
        let expected_len = (frame_width as usize)
            .checked_mul(frame_height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| self.loader_boundary("native loader frame dimensions overflow"))?;
        if frame.len() != expected_len {
            return Err(self.loader_boundary(format!(
                "native loader frame has {} bytes, expected {expected_len}",
                frame.len()
            )));
        }
        let mut surface = Surface::from_bytes(
            frame_width,
            frame_height,
            PixelFormat::Rgba8888,
            frame.to_vec(),
        )
        .map_err(|error| self.loader_boundary(error.to_string()))?;
        let logical = self.graphics.surface();
        loader
            .render_native_text(
                &mut surface,
                fonts,
                logical.width(),
                logical.height(),
                Some(&gamma),
            )
            .map_err(|error| self.loader_boundary(error.to_string()))?;
        frame.copy_from_slice(surface.pixels());
        Ok(())
    }

    /// `C4GUI::Resource::Clear` + `CloseFiles`: the next load happens
    /// against the global-only group set, so every rebound sheet returns
    /// to its pristine startup surface.
    pub(crate) fn restore_startup_gui_sheets(&mut self) {
        self.install_active_gui_sheet_overrides(&[]);
    }

    pub(crate) fn restore_startup_fonts(&mut self) {
        let fonts = self.assets.startup_clonk_fonts.clone();
        let tooltip = self.assets.startup_global_tooltip_font.clone();
        {
            let assets = Arc::make_mut(&mut self.assets);
            assets.clonk_fonts = fonts.clone();
            assets.global_tooltip_font = tooltip;
        }
        self.graphics.set_clonk_fonts(fonts.clone());
        self.main_menu_state.menu.set_clonk_fonts(fonts);
        self.native_startup_fonts = None;
        if let Some(config) = self.loader_render_config {
            self.configure_native_startup_fonts(
                config.application_scale(),
                config.point_filtering(),
            );
        }
    }

    pub(crate) fn failed_open_game_returns_to_startup(&self) -> bool {
        // C4Game::ParseCommandLine suppresses UseStartupDialog for an explicit
        // scenario, direct join or record stream. Their failed OpenGame quits;
        // console `/open` returns directly to C4AS_Startup. Only an ordinary
        // fullscreen startup lineage reaches QuitGame -> PreInit -> DoStartup.
        //
        // Reachable from `run_headless_server`: its console event loop drives
        // `GameApp::update`, whose `poll_loading` reports an asynchronous
        // scenario-load failure through `finish_scenario_loading_failure`.
        // `!console_mode` is nonetheless the whole condition — `UseStartupDialog`
        // is `isFullScreen && ...` (C4Game.cpp:3321) and only `/console` clears
        // `isFullScreen` (C4Game.cpp:3317-3318), so a dedicated server keeps the
        // fullscreen lineage and the gate takes no `headless` term.
        !self.console_mode
            && self
                .classic_command_line
                .scenario
                .as_ref()
                .is_none_or(|path| path.as_os_str().is_empty())
            && self
                .classic_command_line
                .direct_join
                .as_deref()
                .is_none_or(str::is_empty)
            && self
                .classic_command_line
                .record_stream
                .as_ref()
                .is_none_or(|path| path.as_os_str().is_empty())
    }

    pub(crate) fn resume_startup_music_after_failed_open_game(&mut self) {
        if !self.failed_open_game_returns_to_startup() {
            return;
        }
        self.reconstruct_music_system_at_preinit();
        self.begin_frontend_music_entry();
    }

    pub(crate) fn startup_tooltip_resource_string(&self, key: &str) -> String {
        self.startup_tooltip_resources
            .get(key)
            .cloned()
            .unwrap_or_else(|| format!("[Undefined: {key}]"))
    }

    pub(crate) fn startup_tooltip_resource_no_amp(&self, key: &str) -> String {
        self.startup_tooltip_resource_string(key).replace('&', "")
    }

    pub(crate) fn resolve_startup_tooltip_text(&self, tooltip: StartupTooltip) -> String {
        match tooltip {
            StartupTooltip::Resource { key } => self.startup_tooltip_resource_string(key),
            StartupTooltip::FormattedResource { key, arguments } => {
                let template = self.startup_tooltip_resource_string(key);
                let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
                format_resource_string(template, &arguments)
            }
            StartupTooltip::Text(text) => text,
        }
    }

    pub(crate) fn scenario_loader_head_for_start(
        &self,
        scenario: &FrontendScenario,
    ) -> std::result::Result<Option<ScenarioLoaderHead>, ClassicParityBoundary> {
        if !matches!(scenario.kind, ScenarioKind::Scenario) {
            return Ok(None);
        }
        let Some(path) = scenario.path.as_deref() else {
            return Ok(None);
        };
        let Some(paths) = self.app_paths.as_ref() else {
            // Pathless test/sandbox apps do not represent C4Startup's
            // installed scenario browser and retain their existing route.
            return Ok(None);
        };
        let inspect_error = |error: &dyn fmt::Display| {
            report_classic_parity_boundary(ClassicParityBoundary::ScenarioStartInspection {
                path: path.to_path_buf(),
                detail: error.to_string(),
            })
        };
        let group = Group::open(path).map_err(|error| inspect_error(&error))?;
        let head = load_classic_scenario_loader_head(&group, paths)
            .map_err(|error| inspect_error(&error))?;
        Ok(Some(head))
    }

    /// Whether this session has already been told to stop showing a warning.
    ///
    /// `ShowMessageModal` tests the flag it was handed by pointer — `if
    /// (pbConfigDontShowAgainSetting && *pbConfigDontShowAgainSetting) return
    /// true;` (C4GuiDialogs.cpp:1060-1065) — so a tick suppresses the dialog
    /// immediately, from memory. The pending change therefore outranks the
    /// file, which only catches up at a save surface.
    pub(crate) fn startup_message_hidden(&self, key: &str) -> bool {
        if let Some(pending) = self.deferred_config.get("Startup", key) {
            return parse_config_bool(pending);
        }
        self.app_paths
            .as_ref()
            .and_then(|paths| Config::load(paths.config_file()).ok())
            .and_then(|config| config.get_in(Some("Startup"), key).map(parse_config_bool))
            .unwrap_or(false)
    }
}
