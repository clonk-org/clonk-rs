//! `impl GameApp` — rendering, viewports & HUD methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    pub(crate) fn current_hud_graphics(&self) -> Arc<HudGraphics> {
        self.active_game_graphics
            .as_ref()
            .map(|resources| Arc::clone(&resources.hud_graphics))
            .unwrap_or_else(|| self.assets.hud_graphics())
    }

    pub(crate) fn current_hud_graphics_ref(&self) -> &HudGraphics {
        self.active_game_graphics
            .as_ref()
            .map(|resources| resources.hud_graphics.as_ref())
            .unwrap_or(self.assets.hud_graphics.as_ref())
    }

    pub(crate) fn runtime_flash_viewport_count(&self) -> usize {
        self.snapshot
            .hud
            .local_players
            .iter()
            .filter_map(|owner| {
                self.snapshot
                    .players
                    .iter()
                    .find(|player| player.id == *owner)
            })
            .map(|player| player.viewports.len())
            .sum()
    }

    fn finish_runtime_flash_draw(&mut self) {
        let Some(message) = self.runtime_flash_message.as_mut() else {
            return;
        };
        message.remaining_draws = message.remaining_draws.saturating_sub(1);
        if message.remaining_draws == 0 {
            self.runtime_flash_message = None;
        }
    }

    /// Keep the in-game menu gates synchronized with the live window mode.
    /// The process-wide display settings remain owned by the window loop;
    /// `DisplayFlags` is their presentation projection for running menus.
    pub(crate) fn set_display_mode(&mut self, mode: DisplayMode) {
        self.display_flags.is_fullscreen = matches!(mode, DisplayMode::Fullscreen);
    }

    /// Rust creates the fullscreen physical observer viewport from the
    /// absence of local player viewports. A temporary film target changes
    /// only its displayed owner, not this classification.
    pub(crate) fn primary_physical_viewport_is_no_owner(&self) -> bool {
        self.physical_viewports
            .iter()
            .any(|viewport| viewport.matches_close(OWNER_NONE))
    }

    /// GUI keyboard focus and the ownerless fullscreen menu replace the
    /// FilmView/FreeView scope. Nonexclusive overlays (scoreboard, client
    /// list, and player-owned menus) deliberately do not participate.
    pub(crate) fn viewport_cycle_scope_available(&self) -> bool {
        (!self.runtime_gui_has_keyboard_focus() || self.network_chart_elevated)
            && !self.runtime_top_default_dialog_is_exclusive()
            && !(self.primary_physical_viewport_is_no_owner() && self.ingame_menu.is_some())
    }

    pub(crate) fn viewport_scope_excludes_player_control(&self) -> bool {
        self.engine.film_replay() || self.primary_physical_viewport_is_no_owner()
    }

    fn primary_viewport_player(&self) -> Option<i32> {
        self.physical_viewports
            .first()
            .map(|viewport| viewport.displayed_player)
    }

    /// C4Viewport::NextPlayer over the app-owned first physical viewport.
    /// `wrap` is true for film replay and false for an assigned observer key.
    pub(crate) fn cycle_primary_viewport_player(&mut self, wrap: bool) -> bool {
        let Some(current) = self.primary_viewport_player() else {
            return false;
        };
        let players = self
            .engine
            .players()
            .map(|player| player.id())
            .collect::<Vec<_>>();
        let target = if let Some(index) = players.iter().position(|player| *player == current) {
            players.get(index + 1).copied().or_else(|| {
                if wrap {
                    players.first().copied()
                } else {
                    Some(OWNER_NONE)
                }
            })
        } else {
            players.first().copied()
        };
        let Some(target) = target else {
            return false;
        };
        if target == current {
            return false;
        }
        self.set_physical_film_view(target)
    }

    pub(crate) fn ensure_ingame_menu_gfx(&mut self) -> &mut IngameMenuGraphics {
        if self.ingame_menu_gfx.is_none() {
            let hud = self.current_hud_graphics();
            let throw_key = self
                .bindings
                .key_for(ControlBindingId::Throw)
                .map(format_key_label)
                .unwrap_or_default();
            let dig_key = self
                .bindings
                .key_for(ControlBindingId::Dig)
                .map(format_key_label)
                .unwrap_or_default();
            let special2_key = self
                .bindings
                .key_for(ControlBindingId::Special2)
                .map(format_key_label)
                .unwrap_or_default();
            self.ingame_menu_gfx = Some(IngameMenuGraphics {
                hud: hud.as_ref().clone(),
                menu: hud
                    .menu
                    .clone()
                    .or_else(|| self.assets.dialog_image("Menu.png")),
                options: self.current_options_graphic(),
                control: hud
                    .control
                    .clone()
                    .or_else(|| self.assets.dialog_image("Control.png")),
                gui_icons: self.assets.dialog_image("GUIIcons.png"),
                player: hud
                    .player
                    .clone()
                    .or_else(|| self.assets.dialog_image("Player.png")),
                caption_bar: self.assets.dialog_image("GUICaption.png"),
                show_commands: self.display_flags.show_commands,
                show_portraits: self.display_flags.portraits,
                show_command_keys: self.display_flags.show_command_keys,
                throw_key,
                special2_key,
                dig_key,
                ..Default::default()
            });
        }
        self.ingame_menu_gfx
            .as_mut()
            .expect("ingame menu gfx initialised above")
    }

    fn screenshot_result_message(&self, path: &Path, success: bool) -> String {
        let path = self
            .app_paths
            .as_ref()
            .and_then(|paths| path.strip_prefix(paths.install_root()).ok())
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let key = if success {
            "IDS_PRC_SCREENSHOT"
        } else {
            "IDS_PRC_SCREENSHOTERROR"
        };
        format_resource_string(self.runtime_resource_string(key), &[&path])
    }

    pub(crate) fn report_screenshot_result(
        &mut self,
        outcome: Option<ScreenshotSaveOutcome>,
    ) -> Option<String> {
        let outcome = outcome?;
        let message = self.screenshot_result_message(&outcome.path, outcome.result.is_ok());
        match outcome.result {
            Ok(()) => tracing::info!(kind = ?outcome.kind, "{message}"),
            Err(error) => {
                // SaveScreenshot uses ordinary Log() for both outcomes. Keep
                // the lower-level detail as a structured field while the
                // localized result line stays info-level.
                tracing::info!(kind = ?outcome.kind, %error, "{message}");
            }
        }
        let line = self.timestamp_log_line(message.clone());
        self.enqueue_control_message_board_line(line);
        Some(message)
    }

    pub(crate) fn ingame_viewport_region(
        &self,
        owner: i32,
        point: GuiPoint,
    ) -> Option<IngameViewportRegion> {
        // These regions are registered while drawing the same viewport HUD,
        // menu and control calls that native skips during a film replay.
        if self.engine.film_replay() {
            return None;
        }
        let viewport = self
            .graphics
            .active_viewport_projections()
            .into_iter()
            .rev()
            .find(|viewport| {
                viewport.owner == owner && viewport.contains_output_point((point.x, point.y))
            })?;
        let mouse_viewport = self
            .active_ingame_mouse_viewport()
            .is_some_and(|mouse| mouse.index == viewport.index);
        // DrawMouseButtons runs after cursor info and C4RegionList::Add
        // prepends, so these local controls win any unlikely overlap.
        if let Some(button) = clonk_frontend::hud::viewport_button_region(
            viewport.rect,
            point,
            self.display_flags.show_commands && !(self.engine.film() && self.engine.replay()),
            mouse_viewport,
            self.startup_irc_client_active(),
        ) {
            return Some(IngameViewportRegion::ViewportButton(button));
        }
        // DrawCommands registers after DrawIDList, so command pairs win the
        // remaining inventory overlap.
        if let Some(command) = self.ingame_command_region_at(owner, point) {
            return Some(IngameViewportRegion::Command(command));
        }
        self.ingame_inventory_region_target(owner, point)
            .map(IngameViewportRegion::Inventory)
    }

    fn reset_menu_positions_for_viewport_changes(&mut self) {
        let mut current = BTreeMap::new();
        for viewport in self.graphics.active_viewport_projections() {
            current.entry(viewport.owner).or_insert(viewport.rect);
        }
        let mut changed = self
            .menu_viewport_rects
            .iter()
            .filter_map(|(&owner, &previous)| {
                (current.get(&owner).copied() != Some(previous)).then_some(owner)
            })
            .collect::<Vec<_>>();
        changed.extend(current.keys().copied().filter(|owner| {
            !self.menu_viewport_rects.contains_key(owner)
                && (self.ingame_menu.contains(*owner)
                    || self.script_menu_presentations.contains_key(owner))
        }));
        if changed.is_empty() {
            self.menu_viewport_rects = current;
            return;
        }
        for &owner in &changed {
            if let Some(menu) = self.ingame_menu.get_mut(owner) {
                menu.reset_location();
            }
            if let Some(state) = self.script_menu_presentations.get_mut(&owner) {
                if state.free_aligned {
                    if let (Some((x, y)), Some(previous), Some(next)) = (
                        state.location,
                        self.menu_viewport_rects.get(&owner),
                        current.get(&owner),
                    ) {
                        state.location = Some((
                            x.saturating_add(next.x.saturating_sub(previous.x)),
                            y.saturating_add(next.y.saturating_sub(previous.y)),
                        ));
                    }
                }
                reset_script_menu_presentation_location(state);
            }
        }
        if self.menu_title_drag.is_some_and(|drag| {
            let owner = match drag {
                MenuTitleDrag::Script { owner, .. } => owner,
                MenuTitleDrag::Ingame { player, .. } => player,
            };
            changed.contains(&owner)
        }) {
            self.menu_title_drag = None;
        }
        if self
            .ingame_menu_close_pointer_capture
            .is_some_and(|owner| changed.contains(&owner))
        {
            self.ingame_menu_close_pointer_capture = None;
        }
        if self
            .script_menu_close_pointer_capture
            .is_some_and(|(owner, _)| changed.contains(&owner))
        {
            self.script_menu_close_pointer_capture = None;
        }
        self.menu_viewport_rects = current;
        self.mark_menu_dirty();
    }

    /// C4FullScreen::Close intercepts the native window close while a round
    /// is running. A refused/duplicate abort dialog still keeps the process
    /// alive; only startup/loading close requests enter ordinary teardown.
    pub(crate) fn handle_window_close_requested(&mut self) {
        if self.mode == AppMode::Running {
            let dialog_owner = if self.primary_physical_viewport_is_no_owner() {
                OWNER_NONE
            } else {
                self.local_owner
            };
            self.show_abort_dialog(dialog_owner);
            return;
        }
        self.finalize_pending_league_end_for_teardown();
        self.request_exit();
    }

    pub(crate) fn clear_physical_viewport_states(&mut self) {
        for viewport in std::mem::take(&mut self.physical_viewports) {
            self.graphics
                .drop_physical_camera(viewport.physical_identity);
        }
        self.update_film_viewport_availability();
    }

    /// `C4GraphicsSystem::RecalculateViewports` sorts the physical list by
    /// the currently displayed player's control layout. `SetFilmView` itself
    /// deliberately does not call this; create/close do.
    fn sort_physical_viewports_by_player_control(&mut self) {
        let engine = &self.engine;
        self.physical_viewports.sort_by_key(|viewport| {
            engine
                .player(viewport.displayed_player)
                .map_or(i32::MAX, |player| {
                    classic_viewport_layout_order(player.control_set())
                })
        });
    }

    fn allocate_physical_viewport_identity(&mut self) -> u64 {
        let identity = self.next_physical_viewport_identity;
        self.next_physical_viewport_identity = self.next_physical_viewport_identity.wrapping_add(1);
        identity
    }

    pub(crate) fn owned_physical_viewport_state(
        &mut self,
        player: i32,
        expand_player_slots: bool,
    ) -> PhysicalViewportState {
        let identity = self.allocate_physical_viewport_identity();
        let mut viewport = PhysicalViewportState::owned(player, expand_player_slots, identity);
        if let Some(player) = self.engine.player(player) {
            viewport.preserved_zoom = player
                .viewports()
                .first()
                .map_or(1.0, |viewport| viewport.zoom);
        }
        viewport
    }

    pub(crate) fn ownerless_physical_viewport_state(&mut self) -> PhysicalViewportState {
        let identity = self.allocate_physical_viewport_identity();
        PhysicalViewportState::ownerless(identity)
    }

    /// Reconstruct the ordinary, non-retargeted list for tests and legacy
    /// setup code that directly changes the local-control registry. This is
    /// never used after physical identity becomes observable.
    pub(crate) fn refresh_non_authoritative_physical_viewports(&mut self) {
        if self.physical_viewports_authoritative {
            return;
        }
        let mut owners = self
            .local_controls
            .owners()
            .filter(|owner| self.engine.player(*owner).is_some())
            .collect::<Vec<_>>();
        let engine = &self.engine;
        owners.sort_by_key(|owner| {
            engine.player(*owner).map_or(i32::MAX, |player| {
                classic_viewport_layout_order(player.control_set())
            })
        });
        let already_current = if owners.is_empty() {
            matches!(
                self.physical_viewports.as_slice(),
                [viewport]
                    if viewport.displayed_player == OWNER_NONE
                        && viewport.is_no_owner_viewport
            )
        } else {
            self.physical_viewports.len() == owners.len()
                && self
                    .physical_viewports
                    .iter()
                    .zip(&owners)
                    .all(|(viewport, owner)| {
                        viewport.displayed_player == *owner
                            && viewport.camera_identity_owner == *owner
                            && !viewport.is_no_owner_viewport
                            && viewport.expand_player_slots
                            && viewport.uses_live_player_presentation
                    })
        };
        if already_current {
            self.update_film_viewport_availability();
            return;
        }
        self.clear_physical_viewport_states();
        for owner in owners {
            let viewport = self.owned_physical_viewport_state(owner, true);
            self.physical_viewports.push(viewport);
        }
        self.sort_physical_viewports_by_player_control();
        if self.physical_viewports.is_empty() {
            let viewport = self.ownerless_physical_viewport_state();
            self.physical_viewports.push(viewport);
        }
        self.update_film_viewport_availability();
    }

    pub(crate) fn create_physical_viewport(
        &mut self,
        player: i32,
        silent: bool,
        game_running: bool,
        expand_player_slots: bool,
    ) -> bool {
        if player != OWNER_NONE && self.engine.player(player).is_none() {
            return false;
        }
        let viewport = if player == OWNER_NONE {
            self.ownerless_physical_viewport_state()
        } else {
            self.owned_physical_viewport_state(player, expand_player_slots)
        };
        self.physical_viewports.push(viewport);
        self.sort_physical_viewports_by_player_control();
        self.update_film_viewport_availability();
        if player != OWNER_NONE {
            self.runtime_flash_message = None;
        }
        if !silent {
            self.play_viewport_feedback_sound_for_game_state(game_running);
        }
        true
    }

    pub(crate) fn close_physical_viewports(
        &mut self,
        player: i32,
        silent: bool,
        game_running: bool,
    ) -> bool {
        let primary_removed = self
            .physical_viewports
            .first()
            .is_some_and(|viewport| viewport.matches_close(player));
        let previous_count = self.physical_viewports.len();
        let mut removed_identities = Vec::new();
        self.physical_viewports.retain(|viewport| {
            let removed = viewport.matches_close(player);
            if removed {
                removed_identities.push(viewport.physical_identity);
            }
            !removed
        });
        let closed = self.physical_viewports.len() != previous_count;
        if !closed {
            return false;
        }
        for identity in removed_identities {
            self.graphics.drop_physical_camera(identity);
        }
        if primary_removed {
            self.film_view_player = None;
        }
        self.sort_physical_viewports_by_player_control();
        self.update_film_viewport_availability();
        if !silent {
            self.play_viewport_feedback_sound_for_game_state(game_running);
        }
        true
    }

    pub(crate) fn observer_viewport_index(&self) -> Option<usize> {
        self.physical_viewports
            .iter()
            .position(|viewport| viewport.is_no_owner_viewport)
    }

    pub(crate) fn observer_viewport_player(&self) -> Option<i32> {
        self.observer_viewport_index()
            .map(|index| self.physical_viewports[index].displayed_player)
    }

    /// Fold process-local replay requests at the same app seam as viewport
    /// lifecycle operations. A request targeting a player retired inside the
    /// just-finished engine call identifies the player whose close must still
    /// run after the callback's retarget.
    pub(crate) fn apply_pending_viewport_presentation_requests(&mut self) -> Option<i32> {
        let requests = self.engine.take_viewport_presentation_requests();
        let mut removed_target = None;
        for request in requests {
            match request {
                clonk_engine::ViewportPresentationRequest::SetFilmView { player } => {
                    let _ = self.set_physical_film_view(player);
                    if player != OWNER_NONE && self.engine.player(player).is_none() {
                        removed_target = Some(player);
                    }
                }
                clonk_engine::ViewportPresentationRequest::SetViewOffset { player, offset } => {
                    if let Some(viewport) = self
                        .physical_viewports
                        .iter_mut()
                        .find(|viewport| viewport.displayed_player == player)
                    {
                        viewport.preserved_offset = offset;
                    }
                }
            }
        }
        removed_target
    }

    /// Exact fullscreen `C4FullScreen::ViewportCheck` membership behavior.
    /// Creation of an ownerless observer and its later film retarget are
    /// silent; creation directly for the first replay-film player is not.
    pub(crate) fn check_fullscreen_physical_viewports(&mut self, game_running: bool) {
        match self.physical_viewports.len() {
            0 => {
                let film_player = self
                    .engine
                    .is_replay_film()
                    .then(|| self.engine.first_player_id())
                    .flatten();
                let player = film_player.unwrap_or(OWNER_NONE);
                let _ = self.create_physical_viewport(
                    player,
                    player == OWNER_NONE,
                    game_running,
                    false,
                );
                if film_player.is_some() {
                    self.film_view_player = film_player;
                    self.physical_viewports_authoritative = true;
                }
            }
            1 => {}
            _ => {
                let _ = self.close_physical_viewports(OWNER_NONE, true, game_running);
            }
        }

        if self.engine.is_replay_film() {
            if let Some(first_player) = self.engine.first_player_id() {
                if let Some(index) = self
                    .physical_viewports
                    .iter()
                    .position(|viewport| viewport.matches_close(OWNER_NONE))
                {
                    self.physical_viewports[index].displayed_player = first_player;
                    self.physical_viewports[index].uses_live_player_presentation = false;
                    self.film_view_player = Some(first_player);
                    self.physical_viewports_authoritative = true;
                    self.runtime_flash_message = None;
                }
            }
        }
        self.update_film_viewport_availability();
    }

    /// `C4Game::InitGameFinal`: create one viewport per live local player in
    /// list/control order, then run the fullscreen fallback exactly once.
    pub(crate) fn initialize_physical_viewports(&mut self, game_running: bool) {
        self.clear_physical_viewport_states();
        self.physical_viewports_authoritative = false;
        self.film_view_player = None;
        let local_players = self
            .engine
            .players()
            .map(|player| player.id())
            .filter(|owner| self.local_controls.assignment(*owner).is_some())
            .collect::<Vec<_>>();
        for owner in local_players {
            let _ = self.create_physical_viewport(owner, false, game_running, true);
        }
        self.check_fullscreen_physical_viewports(game_running);
    }

    /// Put the physical viewport that C++ retargets through `SetFilmView`
    /// first without allocating a second per-frame list. The close projection
    /// only needs the first entry; the order of every later viewport is moot.
    pub(crate) fn move_classic_primary_viewport_first(&self, owners: &mut [i32]) {
        let Some((primary, _)) = owners.iter().enumerate().min_by_key(|(_, owner)| {
            self.local_controls
                .assignment(**owner)
                .map_or(i32::MAX, |assignment| {
                    classic_viewport_layout_order(assignment.set)
                })
        }) else {
            return;
        };
        owners[..=primary].rotate_right(1);
    }

    pub(crate) fn live_local_viewport_owners_with_primary_first(&self) -> Vec<i32> {
        let mut owners = self
            .local_controls
            .owners()
            .filter(|owner| self.engine.player(*owner).is_some())
            .collect::<Vec<_>>();
        self.move_classic_primary_viewport_first(&mut owners);
        owners
    }

    /// Remove one runtime player and mirror `C4PlayerList::Remove`'s
    /// non-silent viewport close. A remote player stays silent unless the
    /// replay film viewport is temporarily targeting it; failed/no-op
    /// removals never have a viewport to close.
    pub(crate) fn remove_runtime_player_with_viewport_feedback(
        &mut self,
        player_id: i32,
    ) -> Result<(), EngineError> {
        self.refresh_non_authoritative_physical_viewports();
        self.apply_direct_film_view_projection();
        let _ = self.apply_pending_viewport_presentation_requests();
        self.engine.remove_player(player_id)?;
        // RemovePlayer/OnOwnerRemoved callbacks run before native viewport
        // closure and may synchronously retarget the primary viewport.
        let _ = self.apply_pending_viewport_presentation_requests();
        let game_running = matches!(self.mode, AppMode::Running);
        let _ = self.close_physical_viewports(player_id, false, game_running);
        self.remove_local_control_assignment(player_id);
        self.check_fullscreen_physical_viewports(game_running);
        Ok(())
    }

    pub(crate) fn render_message_dialogs(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        if self.message_dialogs.is_empty() {
            return Ok(());
        }
        let assets = Arc::clone(&self.assets);
        let Some(resources) = assets.message_dialog_resources() else {
            tracing::error!(
                count = self.message_dialogs.len(),
                "refusing to render classic message dialog without exact resources"
            );
            anyhow::bail!(
                "classic message-dialog resources are unavailable; refusing generic fallback"
            );
        };
        let last = self.message_dialogs.len() - 1;
        let active_index = (!self.running_chat_active())
            .then(|| self.active_message_dialog_index())
            .flatten();
        let ordered_native = self.graphics.surface().is_clonk_text_capture_active();
        let now = Instant::now();
        for index in 0..=last {
            let keyboard_active = Some(index) == active_index && self.context_menu.is_none();
            let mouse_active = self.mode == AppMode::Running || Some(index) == active_index;
            self.message_dialogs[index].state.render_at(
                self.graphics.surface_mut(),
                resources,
                keyboard_active,
                mouse_active,
                gamma,
                now,
            )?;
            if ordered_native && index != last {
                self.next_pending_native_overlay();
            }
        }
        Ok(())
    }

    fn render_running_message_dialog_layer(
        &mut self,
        stack_id: u64,
        gamma: Option<&clonk_graphics::GammaRamp>,
        now: Instant,
        ordered_native: bool,
    ) -> Result<()> {
        let Some(index) = self
            .message_dialogs
            .iter()
            .position(|dialog| dialog.running_stack_id == stack_id)
        else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .message_dialog_resources()
            .context("classic message-dialog resources are unavailable")?;
        let keyboard_active = Some(index) == self.active_message_dialog_index()
            && !self.running_chat_active()
            && self.context_menu.is_none();
        self.message_dialogs[index].state.render_at(
            self.graphics.surface_mut(),
            resources,
            keyboard_active,
            true,
            gamma,
            now,
        )?;
        if ordered_native {
            self.next_pending_native_overlay();
        }
        Ok(())
    }

    fn render_league_signup_dialog(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        let Some(dialog) = self.league_signup_dialog.as_ref() else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .league_signup_resources()
            .context("classic C4LeagueSignupDialog resources are unavailable")?;
        dialog.controller.render(
            self.graphics.surface_mut(),
            resources,
            self.message_dialogs.is_empty() && self.context_menu.is_none(),
            gamma,
        )
    }

    fn render_league_signup_tooltip(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<bool> {
        let (width, height) = {
            let surface = self.graphics.surface();
            (surface.width() as i32, surface.height() as i32)
        };
        let Some((pointer, text)) = self.league_signup_tooltip(width, height) else {
            return Ok(false);
        };
        let font = self
            .assets
            .global_tooltip_font
            .clone()
            .context("classic shadowless tooltip font is unavailable")?;
        clonk_frontend::context_menu::draw_classic_tooltip(
            self.graphics.surface_mut(),
            &font,
            pointer,
            &text,
            gamma,
        );
        Ok(true)
    }

    pub(crate) fn render_game_over_tooltip(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<bool> {
        if !self.game_over_dialog_is_active() {
            return Ok(false);
        }
        let Some(pointer) = self.startup_tooltip.eligible_pointer() else {
            return Ok(false);
        };
        let (surface_width, surface_height) = {
            let surface = self.graphics.surface();
            (surface.width(), surface.height())
        };
        let text = self
            .game_over_dialog
            .as_ref()
            .map(|dialog| dialog.tooltip_at(pointer.x, pointer.y, surface_width, surface_height))
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        let Some(text) = text else {
            return Ok(false);
        };
        let font = self
            .assets
            .global_tooltip_font
            .clone()
            .context("classic shadowless tooltip font is unavailable")?;
        clonk_frontend::context_menu::draw_classic_tooltip(
            self.graphics.surface_mut(),
            font.as_ref(),
            pointer,
            &text,
            gamma,
        );
        Ok(true)
    }

    fn render_loading_league_signup_dialog(
        &self,
        surface: &mut Surface,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Result<()> {
        let Some(dialog) = self.league_signup_dialog.as_ref() else {
            return Ok(());
        };
        let resources = self
            .assets
            .league_signup_resources()
            .map_err(|error| self.loader_boundary(error.to_string()))?;
        dialog
            .controller
            .render(
                surface,
                resources,
                self.message_dialogs.is_empty() && self.context_menu.is_none(),
                Some(gamma),
            )
            .map_err(|error| self.loader_boundary(error.to_string()))
    }

    fn render_loading_league_signup_tooltip(
        &self,
        surface: &mut Surface,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Result<bool> {
        let Some((pointer, text)) =
            self.league_signup_tooltip(surface.width() as i32, surface.height() as i32)
        else {
            return Ok(false);
        };
        let font = self
            .assets
            .global_tooltip_font
            .as_deref()
            .context("classic shadowless tooltip font is unavailable")?;
        clonk_frontend::context_menu::draw_classic_tooltip(
            surface,
            font,
            pointer,
            &text,
            Some(gamma),
        );
        Ok(true)
    }

    fn render_classic_dialog_title_tooltip(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<bool> {
        let Some(pointer) = self.startup_tooltip.eligible_pointer() else {
            return Ok(false);
        };
        let Some(target) = self.classic_dialog_title_tooltip_target_at(pointer) else {
            return Ok(false);
        };
        let text = self.resolve_startup_tooltip_text(target);
        if text.is_empty() {
            return Ok(false);
        }
        let font = self
            .assets
            .global_tooltip_font
            .clone()
            .context("classic shadowless tooltip font is unavailable")?;
        clonk_frontend::context_menu::draw_classic_tooltip(
            self.graphics.surface_mut(),
            font.as_ref(),
            pointer,
            &text,
            gamma,
        );
        Ok(true)
    }

    fn render_message_dialog_tooltip(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        if self.context_menu.is_some() {
            return Ok(());
        }
        let Some(tooltip_pointer) = self.startup_tooltip.eligible_pointer() else {
            return Ok(());
        };
        if self.network_chart_is_elevated_pointer_layer()
            && self.network_chart_contains_point(tooltip_pointer)
        {
            return Ok(());
        }
        let assets = Arc::clone(&self.assets);
        let Some(resources) = assets.message_dialog_resources() else {
            return Ok(());
        };
        let (surface_width, surface_height) = {
            let surface = self.graphics.surface();
            (surface.width() as i32, surface.height() as i32)
        };
        let index = if self.mode == AppMode::Running {
            match self.top_scoreboard_message_pointer_target_cached(tooltip_pointer) {
                Some(RunningDialogStackEntry::Message(stack_id)) => {
                    self.running_message_index(stack_id)
                }
                _ => None,
            }
        } else {
            (0..self.message_dialogs.len()).rev().find(|index| {
                let dialog = &self.message_dialogs[*index].state;
                let layout = dialog.layout(surface_width, surface_height, &resources.fonts.text);
                dialog
                    .tooltip_state(Some(tooltip_pointer), &layout)
                    .is_some()
            })
        };
        let Some(index) = index else {
            return Ok(());
        };
        if self.graphics.surface().is_clonk_text_capture_active() {
            self.next_pending_native_overlay();
        }
        self.message_dialogs[index].state.render_tooltip(
            self.graphics.surface_mut(),
            resources,
            Some(tooltip_pointer),
            gamma,
        )
    }

    pub(crate) fn render_ordered_context_menu(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        let panel_count = self
            .context_menu
            .as_ref()
            .map_or(0, ClassicContextMenu::panel_count);
        for index in 0..panel_count {
            self.context_menu
                .as_ref()
                .expect("context menu panel count came from an installed menu")
                .render_panel(self.graphics.surface_mut(), index, gamma)?;
            self.next_pending_native_overlay();
        }
        Ok(())
    }

    pub(crate) fn render_context_menu_panels(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        if let Some(context_menu) = self.context_menu.as_ref() {
            context_menu.render_panels(self.graphics.surface_mut(), gamma)?;
        }
        Ok(())
    }

    fn render_context_menu_tooltip(&mut self, gamma: Option<&clonk_graphics::GammaRamp>) -> bool {
        let Some(context_menu) = self.context_menu.as_ref() else {
            return false;
        };
        context_menu.render_tooltip(self.graphics.surface_mut(), gamma)
    }

    fn render_network_chart_layer(
        &mut self,
        frame_gamma: &clonk_graphics::GammaRamp,
        ordered_native: bool,
    ) -> Result<()> {
        self.refresh_network_chart_dialog();
        let Some(dialog) = self.network_chart_dialog.as_ref() else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .network_chart_resources()
            .expect("network chart resources were preflighted before rendering");
        let preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        dialog.render(
            self.graphics.surface_mut(),
            preferred,
            resources,
            Some(frame_gamma),
        )?;
        if ordered_native {
            self.next_pending_native_overlay();
        }
        Ok(())
    }

    fn render_runtime_client_list_layer(
        &mut self,
        frame_gamma: &clonk_graphics::GammaRamp,
        ordered_native: bool,
    ) -> Result<()> {
        let Some(dialog) = self.runtime_client_list.as_ref() else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        let preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        let keyboard_active = self.runtime_client_list_draw_active();
        let mouse_active = self.runtime_client_list_mouse_active();
        if dialog.is_static_info_only() {
            let resources = assets
                .static_info_dialog_resources()
                .expect("static InfoDialog resources were preflighted before rendering");
            dialog.render_static_info(
                self.graphics.surface_mut(),
                preferred,
                resources,
                mouse_active,
                Some(frame_gamma),
            )?;
        } else {
            let resources = assets
                .runtime_client_list_resources()
                .expect("runtime client-list resources were preflighted before rendering");
            dialog.render_body_with_activity(
                self.graphics.surface_mut(),
                preferred,
                resources,
                keyboard_active,
                mouse_active,
                Some(frame_gamma),
            )?;
        }
        if ordered_native {
            self.next_pending_native_overlay();
        }
        Ok(())
    }

    fn render_runtime_client_list_tooltip(
        &mut self,
        frame_gamma: &clonk_graphics::GammaRamp,
    ) -> Result<bool> {
        let mouse_active = self.runtime_client_list_mouse_active();
        let Some(dialog) = self.runtime_client_list.as_ref() else {
            return Ok(false);
        };
        if dialog.is_static_info_only() {
            return Ok(false);
        }
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .runtime_client_list_resources()
            .expect("runtime client-list resources were preflighted before rendering");
        let preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        dialog.render_tooltip(
            self.graphics.surface_mut(),
            preferred,
            resources,
            mouse_active,
            Some(frame_gamma),
        )
    }

    pub(crate) fn runtime_client_list_draw_active(&self) -> bool {
        if self.mode == AppMode::Running {
            self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ClientList)
                && self.running_active_dialog == Some(RunningDialogStackEntry::RuntimeClientList)
                && (self.game_over_dialog.is_none() || self.runtime_client_list_above_game_over)
                && self.context_menu.is_none()
        } else {
            (self.game_over_dialog.is_none() || self.runtime_client_list_above_game_over)
                && self.message_dialogs.is_empty()
                && self.context_menu.is_none()
        }
    }

    fn render_scoreboard_layer(
        &mut self,
        font_images: &HashMap<String, ImageData>,
        frame_gamma: &clonk_graphics::GammaRamp,
        ordered_native: bool,
    ) -> Result<()> {
        let trigger = ClassicScoreboardTrigger::ScriptVisibility;
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .scoreboard_resources(font_images)
            .map_err(|error| self.scoreboard_presentation_error(trigger, error))?;
        let scoreboard = self.snapshot.hud.scoreboard.clone();
        // Shared C4GUI dialogs remain mouse-active even when another dialog
        // owns keyboard focus. Reverse stack routing clears this flag only
        // when a higher hit actually occludes the scoreboard close button.
        let close_hovered = self.scoreboard_runtime.close_hovered;
        let close_pressed = self.scoreboard_close_pointer_capture && close_hovered;
        let (layout, render_state) = {
            let presentation = self
                .scoreboard_runtime
                .presentation
                .as_mut()
                .expect("scoreboard preflight materializes retained presentation state");
            let render_state = presentation.render_state_at(
                Instant::now(),
                &resources,
                close_hovered,
                close_pressed,
            );
            (presentation.layout().clone(), render_state)
        };
        if ordered_native {
            clonk_frontend::scoreboard::render_scoreboard_body_with_layout(
                self.graphics.surface_mut(),
                &scoreboard,
                &resources,
                &layout,
                Some(frame_gamma),
            )
            .map_err(|error| self.scoreboard_presentation_error(trigger, error))?;
            self.next_pending_native_overlay();
            clonk_frontend::scoreboard::render_scoreboard_caption_with_layout(
                self.graphics.surface_mut(),
                &scoreboard,
                &resources,
                &layout,
                render_state,
                Some(frame_gamma),
            )
            .map_err(|error| self.scoreboard_presentation_error(trigger, error))?;
            self.next_pending_native_overlay();
        } else {
            clonk_frontend::scoreboard::render_scoreboard_with_layout(
                self.graphics.surface_mut(),
                &scoreboard,
                &resources,
                &layout,
                render_state,
                Some(frame_gamma),
            )
            .map_err(|error| self.scoreboard_presentation_error(trigger, error))?;
        }
        Ok(())
    }

    fn render_running_dialog_stack(
        &mut self,
        start_index: usize,
        scoreboard_font_images: Option<&HashMap<String, ImageData>>,
        frame_gamma: &clonk_graphics::GammaRamp,
        ordered_native: bool,
    ) -> Result<()> {
        let stack = self
            .running_dialog_stack
            .iter()
            .copied()
            .skip(start_index)
            .collect::<Vec<_>>();
        let now = Instant::now();
        for entry in stack {
            match entry {
                RunningDialogStackEntry::Scoreboard => {
                    if let Some(font_images) = scoreboard_font_images {
                        self.render_scoreboard_layer(font_images, frame_gamma, ordered_native)?;
                    }
                }
                RunningDialogStackEntry::RuntimeClientList => {
                    self.render_runtime_client_list_layer(frame_gamma, ordered_native)?;
                }
                RunningDialogStackEntry::Message(stack_id) => {
                    self.render_running_message_dialog_layer(
                        stack_id,
                        Some(frame_gamma),
                        now,
                        ordered_native,
                    )?;
                }
                RunningDialogStackEntry::Chat => {
                    self.render_running_chat_layer(Some(frame_gamma), ordered_native)?;
                }
            }
        }
        Ok(())
    }

    /// Renders into `frame`; returns whether the physical presentation must
    /// refresh. A raw menu-cache hit still returns `true` when a deferred
    /// monitor-gamma pass remains to be applied.
    pub(crate) fn render(&mut self, frame: &mut [u8]) -> Result<bool> {
        self.render_for_presentation(frame, false, false, false)
    }

    pub(crate) fn render_ordered_native_base(&mut self, frame: &mut [u8]) -> Result<bool> {
        self.pending_native_presentation = Some(NativePresentationPlan::default());
        self.begin_native_text_capture(false);
        if let Err(error) = self.render_for_presentation(frame, false, false, false) {
            let surface = self.graphics.surface_mut();
            let _ = surface.take_clonk_text_capture();
            let _ = surface.take_gpu_scene_capture();
            surface.clear_clip();
            self.pending_native_presentation = None;
            return Err(error);
        }
        if self.graphics.surface().is_clonk_text_capture_active() {
            let has_base = self
                .pending_native_presentation
                .as_ref()
                .is_some_and(|plan| !plan.batches.is_empty());
            if has_base {
                self.commit_pending_native_overlay();
            } else {
                self.commit_pending_native_base(frame);
            }
        }
        Ok(true)
    }

    pub(crate) fn render_for_presentation(
        &mut self,
        frame: &mut [u8],
        defer_native_main_text: bool,
        defer_native_loader_text: bool,
        defer_native_game_messages: bool,
    ) -> Result<bool> {
        self.render_for_presentation_with_monitor_defer(
            frame,
            defer_native_main_text,
            defer_native_loader_text,
            defer_native_game_messages,
            false,
        )
    }

    pub(crate) fn render_for_presentation_with_monitor_defer(
        &mut self,
        frame: &mut [u8],
        defer_native_main_text: bool,
        defer_native_loader_text: bool,
        defer_native_game_messages: bool,
        defer_monitor_gamma: bool,
    ) -> Result<bool> {
        let _renderer_config_guard = clonk_frontend::activate_advanced_renderer_config(
            self.graphics.advanced_renderer_config(),
        );
        if self.console_mode {
            self.sync_developer_console_view();
            let font = self.assets.font_arc();
            self.developer_console
                .render(self.graphics.surface_mut(), font.as_ref());
            self.render_message_dialogs(None)?;
            let surface = self.graphics.surface();
            if !surface.is_gpu_scene_capture_active() {
                if surface.pixels().len() == frame.len() {
                    frame.copy_from_slice(surface.pixels());
                } else {
                    copy_surface(surface.pixels(), surface.width(), surface.height(), frame);
                }
            }
            return Ok(true);
        }
        match self.mode {
            AppMode::Menu => {
                let menu_gamma_value = self.startup_fragment_gamma();
                let menu_gamma = &menu_gamma_value;
                let monitor_gamma = self.startup_monitor_gamma();
                let ordered_native = self.graphics.surface().is_clonk_text_capture_active();
                if ordered_native {
                    self.pending_native_presentation
                        .as_mut()
                        .expect("ordered presentation plan is active")
                        .monitor_gamma = monitor_gamma.clone();
                }
                self.advance_startup_player_portrait_thumbnail();
                if self
                    .startup_player_properties_dialog
                    .as_mut()
                    .is_some_and(|pending| pending.controller.tick_portrait_selector_scrollbar())
                {
                    // C4GUI::ScrollBar repeats held arrows from DrawElement,
                    // so advance once per presentation rather than per update.
                    self.mark_menu_dirty();
                }
                self.preflight_startup_presentation()?;
                self.preflight_visible_gui_overlay_resources()?;
                if self.startup_view == StartupView::NetworkLobby
                    && self.classic_host_lobby.is_none()
                {
                    self.close_stale_classic_lobby_team_combo();
                }
                if self.startup_view == StartupView::NetworkGame
                    && !self.startup_network_transition_active()
                    && self
                        .startup_network_dialog
                        .as_mut()
                        .is_some_and(|dialog| dialog.tick_scrollbar())
                {
                    // C4GUI::ScrollBar repeats held arrows from DrawElement,
                    // so advance once per presentation rather than per update.
                    self.mark_menu_dirty();
                }
                if self.startup_view == StartupView::PlayerSelection
                    && self
                        .startup_player_dialog
                        .as_mut()
                        .is_some_and(|dialog| dialog.tick_scrollbar())
                {
                    // Book-scrollbar arrows repeat once per presentation.
                    self.plrsel_last_click = None;
                    self.mark_menu_dirty();
                }
                if self.startup_view == StartupView::About
                    && self
                        .startup_about_dialog
                        .as_mut()
                        .is_some_and(|dialog| dialog.tick_scrollbar())
                {
                    // About TextWindow arrows repeat from ScrollBar::DrawElement.
                    self.mark_menu_dirty();
                }
                if self.startup_view == StartupView::Options {
                    let actions = self
                        .startup_options_dialog
                        .as_mut()
                        .map(|dialog| dialog.advance_frame())
                        .unwrap_or_default();
                    if !actions.is_empty() {
                        self.process_options_dialog_actions(actions)?;
                        self.mark_menu_dirty();
                    }
                }
                if self.startup_view == StartupView::NetworkLobby
                    && self.classic_host_lobby.is_some()
                {
                    self.menu_frame_cache = None;
                    self.render_classic_host_lobby()?;
                    if ordered_native {
                        self.commit_pending_native_base(frame);
                        self.begin_native_text_capture(true);
                    }
                    let gamma = Some(menu_gamma_value.clone());
                    if self.definition_selector.is_some() {
                        self.render_definition_selector(gamma.as_ref())?;
                        if ordered_native {
                            self.next_pending_native_overlay();
                        }
                    }
                    if self.game_option_input_dialog.is_some() {
                        self.render_game_option_input_dialog(gamma.as_ref())?;
                        if ordered_native {
                            self.next_pending_native_overlay();
                        }
                    }
                    if self.league_signup_dialog.is_some() {
                        self.render_league_signup_dialog(gamma.as_ref())?;
                        if ordered_native {
                            self.next_pending_native_overlay();
                        }
                    }
                    if self.external_irc_dialog_visible {
                        self.render_external_irc_dialog(gamma.as_ref())?;
                        if ordered_native {
                            self.next_pending_native_overlay();
                        }
                    }
                    if self
                        .runtime_client_list
                        .as_ref()
                        .is_some_and(|dialog| dialog.is_info_only())
                    {
                        self.render_runtime_client_list_layer(
                            gamma.as_ref().unwrap_or(menu_gamma),
                            ordered_native,
                        )?;
                    }
                    if !self.message_dialogs.is_empty() {
                        self.render_message_dialogs(gamma.as_ref())?;
                    }
                    if ordered_native
                        && self.game_option_input_dialog.is_none()
                        && self.context_menu.is_some()
                    {
                        self.next_pending_native_overlay();
                        self.render_ordered_context_menu(gamma.as_ref())?;
                    } else if !ordered_native && self.game_option_input_dialog.is_none() {
                        if let Some(context_menu) = self.context_menu.as_ref() {
                            context_menu
                                .render_panels(self.graphics.surface_mut(), gamma.as_ref())?;
                        }
                    }
                    let gui_cursor_drawn = self.draw_classic_gui_cursor(gamma.as_ref());
                    if ordered_native && gui_cursor_drawn {
                        self.next_pending_native_overlay();
                    }
                    if self.render_game_option_input_dialog_tooltip(gamma.as_ref())?
                        && ordered_native
                    {
                        self.next_pending_native_overlay();
                    }
                    if self
                        .render_runtime_client_list_tooltip(gamma.as_ref().unwrap_or(menu_gamma))?
                        && ordered_native
                    {
                        self.next_pending_native_overlay();
                    }
                    self.render_classic_host_lobby_tooltips()?;
                    if ordered_native {
                        self.next_pending_native_overlay();
                    }
                    if self.render_external_irc_dialog_tooltip(gamma.as_ref())? && ordered_native {
                        self.next_pending_native_overlay();
                    }
                    if self.render_classic_dialog_title_tooltip(gamma.as_ref())? && ordered_native {
                        self.next_pending_native_overlay();
                    }
                    self.render_message_dialog_tooltip(gamma.as_ref())?;
                    if self.render_context_menu_tooltip(gamma.as_ref()) && ordered_native {
                        self.next_pending_native_overlay();
                    }
                    if !ordered_native && !self.graphics.surface().is_gpu_scene_capture_active() {
                        let surface = self.graphics.surface();
                        if surface.pixels().len() == frame.len() {
                            frame.copy_from_slice(surface.pixels());
                        } else {
                            copy_surface(
                                surface.pixels(),
                                surface.width(),
                                surface.height(),
                                frame,
                            );
                        }
                        if !defer_monitor_gamma {
                            if let Some(gamma) = monitor_gamma.as_ref() {
                                gamma.apply_to_rgba_bytes(frame);
                            }
                        }
                    }
                    return Ok(true);
                }
                let (width, height) = {
                    let surface = self.graphics.surface();
                    (surface.width(), surface.height())
                };
                let expected_len = width as usize * height as usize * 4;
                let visible_dialog = self.visible_startup_dialog();
                let retained_fade = self.graphics.surface().is_gpu_scene_capture_active();
                let fade_compatible = self.startup_dialog_fade.as_ref().is_some_and(|fade| {
                    Some(fade.incoming) == visible_dialog
                        && (frame.len() == expected_len || retained_fade)
                        && fade.width == width
                        && fade.height == height
                        && fade.underlay.len() == expected_len
                        && fade
                            .outgoing_frame
                            .as_ref()
                            .is_none_or(|outgoing| outgoing.len() == expected_len)
                        && (!ordered_native
                            || fade
                                .outgoing_native_frame
                                .as_ref()
                                .is_none_or(|outgoing| outgoing.len() == expected_len))
                        && (!retained_fade
                            || (fade.underlay_gpu_recorder.is_some()
                                && (fade.outgoing.is_none() || fade.outgoing_gpu_plan.is_some())))
                });
                if self.startup_dialog_fade.is_some() && !fade_compatible {
                    self.startup_dialog_fade = None;
                }
                let fade_was_active = fade_compatible;
                if let Some(fade) = self.startup_dialog_fade.as_mut() {
                    fade.step = fade.step.saturating_add(1).min(STARTUP_DIALOG_FADE_STEPS);
                }
                let fade_draw_inactive = self
                    .startup_dialog_fade
                    .as_ref()
                    .is_some_and(|fade| fade.step < STARTUP_DIALOG_FADE_STEPS);
                let startup_tooltip_pending = self.startup_element_tooltip_pending();
                let cache_eligible = !ordered_native
                    && !self.graphics.surface().is_gpu_scene_capture_active()
                    && !fade_was_active
                    // A retained lobby advances held scrollbars, expires
                    // transient status icons, and matures its own 500 ms
                    // tooltip clock even without another input event.
                    && self.startup_view != StartupView::NetworkLobby
                    && self.context_menu.is_none()
                    && self.startup_player_properties_dialog.is_none()
                    && self.startup_options_advanced_dialog.is_none()
                    && self.game_option_input_dialog.is_none()
                    && self.league_signup_dialog.is_none()
                    && self.definition_selector.is_none()
                    && self.message_dialogs.is_empty()
                    && self.runtime_client_list.is_none()
                    && !self.external_irc_dialog_visible
                    && !startup_tooltip_pending;
                if cache_eligible {
                    if let Some(cache) = self.menu_frame_cache.as_ref() {
                        if cache.view == self.startup_view
                            && cache.version == self.menu_render_version
                            && cache.width == width
                            && cache.height == height
                            && cache.native_text_deferred == defer_native_main_text
                            && cache.frame.len() == frame.len()
                        {
                            frame.copy_from_slice(&cache.frame);
                            if !defer_monitor_gamma {
                                if let Some(gamma) = monitor_gamma.as_ref() {
                                    gamma.apply_to_rgba_bytes(frame);
                                }
                            }
                            // A physical post-pass still counts as refreshed:
                            // the event loop must run it after the cached raw
                            // logical frame has been copied into the presenter.
                            return Ok(defer_monitor_gamma && monitor_gamma.is_some());
                        }
                    }
                }
                let version = self.menu_render_version;
                let definition_selector_open = self.definition_selector.is_some();
                let game_option_input_open = self.game_option_input_dialog.is_some();
                let league_signup_open = self.league_signup_dialog.is_some();
                // A fading C4GUI::Dialog is inactive even when it retains its
                // focused control. Reuse the renderer's inactive-focus path.
                let context_menu_open = self.context_menu.is_some()
                    || self.startup_player_properties_dialog.is_some()
                    || league_signup_open
                    || self.external_irc_dialog_visible
                    || self.runtime_client_list.is_some()
                    || fade_draw_inactive;
                let options_draw_focus =
                    self.startup_options_dialog_has_focus_owner() && !fade_draw_inactive;
                let base_context_menu = if ordered_native || fade_was_active {
                    None
                } else {
                    Self::startup_base_context_menu(
                        self.context_menu.as_ref(),
                        game_option_input_open,
                    )
                };
                let scenario_loading_label = self.scenario_selector_loading_label();
                let network_lobby = self.network_lobby.as_mut();
                render_startup_frame(
                    &mut self.graphics,
                    self.assets.as_ref(),
                    &mut self.main_menu_state,
                    &mut self.menu_state,
                    &self.scenario_entry_enabled,
                    scenario_loading_label.as_deref(),
                    self.startup_network_dialog.as_ref(),
                    self.startup_player_dialog.as_ref(),
                    &self.startup_player_models,
                    &self.startup_crew_models,
                    self.startup_crew_rename.as_mut(),
                    base_context_menu,
                    context_menu_open,
                    definition_selector_open,
                    game_option_input_open,
                    !self.message_dialogs.is_empty() || league_signup_open,
                    &self.scenario_game_options,
                    self.scenario_selector_mode,
                    self.startup_options_dialog.as_ref(),
                    self.startup_options_advanced_dialog
                        .as_mut()
                        .map(|pending| &mut pending.controller),
                    options_draw_focus,
                    self.startup_about_dialog.as_ref(),
                    self.startup_view,
                    network_lobby,
                    self.startup_view_flags,
                    &mut self.menu_backdrop_cache,
                    defer_native_main_text && !fade_was_active,
                    menu_gamma,
                    frame,
                )?;
                if fade_was_active {
                    let fade = self
                        .startup_dialog_fade
                        .take()
                        .expect("compatible startup fade must still be present");
                    if ordered_native || retained_fade {
                        let incoming_text = if ordered_native {
                            self.graphics.surface_mut().take_clonk_text_capture()
                        } else {
                            Vec::new()
                        };
                        let incoming_gpu_recorder =
                            self.graphics.surface_mut().take_gpu_scene_capture();
                        let incoming_opacity =
                            startup_dialog_fade_opacity(fade.step.saturating_mul(10));
                        let outgoing_opacity = startup_dialog_fade_opacity(
                            100_u8.saturating_sub(fade.step.saturating_mul(10)),
                        );
                        let mut plan = NativePresentationPlan {
                            monitor_gamma: monitor_gamma.clone(),
                            ..NativePresentationPlan::default()
                        };
                        if retained_fade {
                            plan.batches.push(NativePresentationBatch {
                                logical_layer: None,
                                clip: None,
                                native_loader_text: false,
                                text: Vec::new(),
                                fonts: None,
                                gpu_recorder: fade.underlay_gpu_recorder.clone(),
                            });
                            if outgoing_opacity != 0 {
                                if let Some(outgoing) = fade.outgoing_gpu_plan.as_ref() {
                                    for mut batch in outgoing.batches.clone() {
                                        apply_startup_fade_to_batch(&mut batch, outgoing_opacity)?;
                                        plan.batches.push(batch);
                                    }
                                }
                            }
                            if incoming_opacity != 0 {
                                let mut incoming = NativePresentationBatch {
                                    logical_layer: None,
                                    clip: None,
                                    native_loader_text: false,
                                    text: incoming_text,
                                    fonts: None,
                                    gpu_recorder: incoming_gpu_recorder,
                                };
                                apply_startup_fade_to_batch(&mut incoming, incoming_opacity)?;
                                plan.batches.push(incoming);
                            }
                        } else {
                            let incoming_frame = frame.to_vec();
                            frame.copy_from_slice(&fade.underlay);
                            if outgoing_opacity != 0 {
                                if let Some(outgoing) = fade.outgoing_native_frame.as_deref() {
                                    plan.batches.push(NativePresentationBatch {
                                        logical_layer: Some(startup_fade_native_layer(
                                            outgoing,
                                            outgoing_opacity,
                                        )),
                                        clip: None,
                                        native_loader_text: false,
                                        text: startup_fade_native_text(
                                            &fade.outgoing_native_text,
                                            outgoing_opacity,
                                        ),
                                        fonts: fade.outgoing_native_fonts.clone(),
                                        gpu_recorder: None,
                                    });
                                }
                            }
                            if incoming_opacity != 0 {
                                plan.batches.push(NativePresentationBatch {
                                    logical_layer: Some(startup_fade_native_layer(
                                        &incoming_frame,
                                        incoming_opacity,
                                    )),
                                    clip: None,
                                    native_loader_text: false,
                                    text: startup_fade_native_text(
                                        &incoming_text,
                                        incoming_opacity,
                                    ),
                                    fonts: None,
                                    gpu_recorder: None,
                                });
                            }
                        }
                        self.pending_native_presentation = Some(plan);
                        if ordered_native {
                            self.begin_native_text_capture(true);
                        } else {
                            self.graphics.begin_gpu_scene_capture();
                        }
                    } else {
                        if fade.step < STARTUP_DIALOG_FADE_STEPS {
                            blend_startup_dialog_frames(
                                &fade.underlay,
                                fade.outgoing_frame.as_deref(),
                                frame,
                                fade.step * 10,
                            );
                        }
                        self.graphics
                            .surface_mut()
                            .pixels_mut()
                            .copy_from_slice(frame);
                    }
                    if fade.step < STARTUP_DIALOG_FADE_STEPS {
                        self.startup_dialog_fade = Some(fade);
                    }
                }
                if ordered_native && !fade_was_active {
                    self.commit_pending_native_base(frame);
                    self.begin_native_text_capture(true);
                }
                let startup_assets = Arc::clone(&self.assets);
                let portrait_selector_open = self
                    .startup_player_properties_dialog
                    .as_ref()
                    .is_some_and(|pending| pending.controller.portrait_selector().is_some());
                let portrait_location_popup_open = self
                    .startup_player_properties_dialog
                    .as_ref()
                    .and_then(|pending| pending.controller.portrait_selector())
                    .is_some_and(|selector| selector.is_location_popup_open());
                if let (Some(properties_assets), Some(fonts), Some(book)) = (
                    startup_assets.plrprop_assets(),
                    startup_assets.clonk_fonts.as_deref(),
                    startup_assets.options_book_fonts.as_deref(),
                ) {
                    if let Some(pending) = self.startup_player_properties_dialog.as_ref() {
                        clonk_frontend::startup_plrproperties::PlayerPropertiesScreen::render_player_form(
                            self.graphics.surface_mut(),
                            &properties_assets,
                            book,
                            &pending.controller,
                            Some(menu_gamma),
                        );
                    }
                    if ordered_native && portrait_selector_open {
                        self.next_pending_native_overlay();
                    }
                    if let Some(pending) = self
                        .startup_player_properties_dialog
                        .as_mut()
                        .filter(|_| portrait_selector_open)
                    {
                        clonk_frontend::startup_plrproperties::PlayerPropertiesScreen::render_portrait_selector_dialog(
                            self.graphics.surface_mut(),
                            &properties_assets,
                            fonts,
                            &mut pending.controller,
                            Some(menu_gamma),
                        );
                    }
                }
                if ordered_native {
                    self.commit_pending_native_overlay();
                    self.begin_native_text_capture(true);
                }
                if definition_selector_open {
                    self.render_definition_selector(Some(menu_gamma))?;
                    if ordered_native {
                        self.next_pending_native_overlay();
                    }
                }
                if game_option_input_open {
                    self.render_game_option_input_dialog(Some(menu_gamma))?;
                    if ordered_native {
                        self.next_pending_native_overlay();
                    }
                }
                if league_signup_open {
                    self.render_league_signup_dialog(Some(menu_gamma))?;
                    if ordered_native {
                        self.next_pending_native_overlay();
                    }
                }
                if self.external_irc_dialog_visible {
                    self.render_external_irc_dialog(Some(menu_gamma))?;
                    if ordered_native {
                        self.next_pending_native_overlay();
                    }
                }
                if self
                    .runtime_client_list
                    .as_ref()
                    .is_some_and(|dialog| dialog.is_info_only())
                {
                    self.render_runtime_client_list_layer(menu_gamma, ordered_native)?;
                }
                if !self.message_dialogs.is_empty() {
                    self.render_message_dialogs(Some(menu_gamma))?;
                }
                if portrait_location_popup_open {
                    if ordered_native {
                        self.next_pending_native_overlay();
                    }
                    if let (Some(properties_assets), Some(fonts), Some(pending)) = (
                        startup_assets.plrprop_assets(),
                        startup_assets.clonk_fonts.as_deref(),
                        self.startup_player_properties_dialog.as_ref(),
                    ) {
                        clonk_frontend::startup_plrproperties::PlayerPropertiesScreen::render_portrait_location_popup(
                            self.graphics.surface_mut(),
                            &properties_assets,
                            fonts,
                            &pending.controller,
                            Some(menu_gamma),
                        );
                    }
                    if ordered_native {
                        self.next_pending_native_overlay();
                    }
                }
                if ordered_native && !game_option_input_open && self.context_menu.is_some() {
                    self.next_pending_native_overlay();
                    self.render_ordered_context_menu(Some(menu_gamma))?;
                } else if fade_was_active && !game_option_input_open {
                    if let Some(context_menu) = self.context_menu.as_ref() {
                        context_menu
                            .render_panels(self.graphics.surface_mut(), Some(menu_gamma))?;
                    }
                }
                let gui_cursor_drawn = self.draw_classic_gui_cursor(Some(menu_gamma));
                if ordered_native && gui_cursor_drawn {
                    self.next_pending_native_overlay();
                }
                if self.render_game_option_input_dialog_tooltip(Some(menu_gamma))? && ordered_native
                {
                    self.next_pending_native_overlay();
                }
                if self.render_runtime_client_list_tooltip(menu_gamma)? && ordered_native {
                    self.next_pending_native_overlay();
                }
                let startup_tooltips_drawn = self.render_startup_tooltips()?;
                if ordered_native && startup_tooltips_drawn {
                    self.next_pending_native_overlay();
                }
                if self.render_league_signup_tooltip(Some(menu_gamma))? && ordered_native {
                    self.next_pending_native_overlay();
                }
                if self.render_external_irc_dialog_tooltip(Some(menu_gamma))? && ordered_native {
                    self.next_pending_native_overlay();
                }
                if self.render_classic_dialog_title_tooltip(Some(menu_gamma))? && ordered_native {
                    self.next_pending_native_overlay();
                }
                self.render_message_dialog_tooltip(Some(menu_gamma))?;
                if self.render_context_menu_tooltip(Some(menu_gamma)) && ordered_native {
                    self.next_pending_native_overlay();
                }
                if !ordered_native
                    && !self.graphics.surface().is_gpu_scene_capture_active()
                    && (fade_was_active
                        || self.startup_player_properties_dialog.is_some()
                        || definition_selector_open
                        || game_option_input_open
                        || league_signup_open
                        || self.external_irc_dialog_visible
                        || self.runtime_client_list.is_some()
                        || !self.message_dialogs.is_empty()
                        || gui_cursor_drawn
                        || startup_tooltips_drawn)
                {
                    let surface = self.graphics.surface();
                    if surface.pixels().len() == frame.len() {
                        frame.copy_from_slice(surface.pixels());
                    } else {
                        copy_surface(surface.pixels(), surface.width(), surface.height(), frame);
                    }
                }
                if cache_eligible {
                    self.menu_frame_cache = Some(MenuFrameCache {
                        view: self.startup_view,
                        version,
                        width,
                        height,
                        native_text_deferred: defer_native_main_text,
                        frame: frame.to_vec(),
                    });
                }
                if !ordered_native && !defer_monitor_gamma {
                    if let Some(gamma) = monitor_gamma.as_ref() {
                        gamma.apply_to_rgba_bytes(frame);
                    }
                }
                Ok(true)
            }
            AppMode::Loading => self
                .render_loading(frame, defer_native_loader_text, defer_monitor_gamma)
                .map(|()| true),
            AppMode::Running => self
                .render_running_for_presentation(
                    frame,
                    defer_native_game_messages,
                    defer_monitor_gamma,
                )
                .map(|()| true),
        }
    }

    pub(crate) fn ingame_selection_frame(&self) -> Option<(Vec<ObjectId>, Vector2, GuiPoint)> {
        if !self.mouse_control
            || !matches!(self.mode, AppMode::Running)
            || self.game_over_dialog.is_some()
            || self.game_option_input_dialog.is_some()
            || !self.message_dialogs.is_empty()
        {
            return None;
        }
        let motion = self
            .ingame_right_mouse_state
            .as_ref()
            .map(|state| &state.motion)
            .filter(|motion| motion.moved && motion.selection_frame)
            .or_else(|| {
                self.mouse_state
                    .as_ref()
                    .map(|state| &state.motion)
                    .filter(|motion| motion.moved && motion.selection_frame)
            })?;
        (motion.start.owner == self.local_owner).then(|| {
            (
                self.ingame_selection_candidates(*motion),
                ingame_pointer_world_pixel(motion.start),
                motion.last.screen,
            )
        })
    }

    fn render_loading(
        &mut self,
        frame: &mut [u8],
        defer_native_text: bool,
        defer_monitor_gamma: bool,
    ) -> Result<()> {
        self.reject_classic_global_gui_bootstrap()?;
        if let Some(detail) = self.loader_error.as_deref() {
            return Err(self.loader_boundary(detail));
        }
        if let Some(detail) = self.loader_render_error.as_deref() {
            return Err(self.loader_boundary(detail));
        }
        let config = self
            .loader_render_config
            .ok_or_else(|| self.loader_boundary("loader render configuration is unavailable"))?;
        let gamma_value = self.startup_fragment_gamma();
        let gamma = &gamma_value;
        let monitor_gamma = self.startup_monitor_gamma();
        let ordered_native = self.graphics.surface().is_clonk_text_capture_active();
        let retained_gpu = self.graphics.surface().is_gpu_scene_capture_active();
        if ordered_native {
            self.pending_native_presentation
                .as_mut()
                .expect("ordered presentation plan is active")
                .monitor_gamma = monitor_gamma.clone();
        }
        if config.uses_scaling_correction()
            && !defer_native_text
            && !ordered_native
            && self.message_dialogs.is_empty()
            && self.league_signup_dialog.is_none()
            && !self
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible)
        {
            return Err(self.loader_boundary(
                "scale-native loader fonts are unavailable for the configured scale",
            ));
        }
        let (width, height) = {
            let surface = self.graphics.surface();
            (surface.width(), surface.height())
        };
        let expected_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| self.loader_boundary("loader frame dimensions overflow"))?;
        if frame.len() != expected_len && !retained_gpu {
            return Err(self.loader_boundary(format!(
                "loader frame has {} bytes, expected {expected_len}",
                frame.len()
            )));
        }

        if ordered_native {
            let render = self
                .loader_screen
                .as_ref()
                .ok_or_else(|| self.loader_boundary("no selected classic loader is installed"))?
                .render_chrome(self.graphics.surface_mut(), config, Some(gamma));
            render.map_err(|error| self.loader_boundary(error.to_string()))?;

            // C4GraphicsSystem draws the startup message-board loader before
            // C4GUI. Commit that base with the loader's dedicated native-text
            // draw point, then give every later dialog its own ordered layer.
            self.commit_pending_native_loader_base(frame);
            self.begin_native_text_capture(true);
            if self
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible)
            {
                {
                    let resources = self
                        .assets
                        .network_start_wait_resources()
                        .map_err(|error| self.loader_boundary(error.to_string()))?;
                    self.network_start_wait
                        .as_ref()
                        .expect("visibility was checked above")
                        .controller
                        .render(self.graphics.surface_mut(), &resources, true, Some(gamma))
                        .map_err(|error| self.loader_boundary(error.to_string()))?;
                }
                self.next_pending_native_overlay();
            }
            self.render_league_signup_dialog(Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
            if self.league_signup_dialog.is_some() {
                self.next_pending_native_overlay();
            }
            self.render_message_dialogs(Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
            if !self.message_dialogs.is_empty() {
                self.next_pending_native_overlay();
            }
            if self.draw_classic_gui_cursor(Some(gamma)) {
                self.next_pending_native_overlay();
            }
            if self
                .render_league_signup_tooltip(Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?
            {
                self.next_pending_native_overlay();
            }
            self.render_message_dialog_tooltip(Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
            return Ok(());
        }

        if retained_gpu {
            let render = self
                .loader_screen
                .as_ref()
                .ok_or_else(|| self.loader_boundary("no selected classic loader is installed"))?
                .render_with_config(self.graphics.surface_mut(), config, Some(gamma));
            render.map_err(|error| self.loader_boundary(error.to_string()))?;
            if self
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible)
            {
                let resources = self
                    .assets
                    .network_start_wait_resources()
                    .map_err(|error| self.loader_boundary(error.to_string()))?;
                self.network_start_wait
                    .as_ref()
                    .expect("visibility was checked above")
                    .controller
                    .render(self.graphics.surface_mut(), &resources, true, Some(gamma))
                    .map_err(|error| self.loader_boundary(error.to_string()))?;
            }
            self.render_league_signup_dialog(Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
            self.render_message_dialogs(Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
            self.draw_classic_gui_cursor(Some(gamma));
            self.render_league_signup_tooltip(Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
            self.render_message_dialog_tooltip(Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
            return Ok(());
        }

        let loader = self
            .loader_screen
            .as_ref()
            .ok_or_else(|| self.loader_boundary("no selected classic loader is installed"))?;
        let mut surface = Surface::from_bytes(width, height, PixelFormat::Rgba8888, frame.to_vec())
            .map_err(|error| self.loader_boundary(error.to_string()))?;
        let render = if defer_native_text {
            loader.render_chrome(&mut surface, config, Some(gamma))
        } else {
            loader.render_with_config(&mut surface, config, Some(gamma))
        };
        render.map_err(|error| self.loader_boundary(error.to_string()))?;
        if self
            .network_start_wait
            .as_ref()
            .is_some_and(|wait| wait.visible)
        {
            let resources = self
                .assets
                .network_start_wait_resources()
                .map_err(|error| self.loader_boundary(error.to_string()))?;
            self.network_start_wait
                .as_ref()
                .expect("visibility was checked above")
                .controller
                .render(&mut surface, &resources, true, Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
        }
        self.render_loading_league_signup_dialog(&mut surface, gamma)?;
        self.render_loading_message_dialogs(&mut surface, gamma)?;
        self.draw_classic_gui_cursor_to_surface(&mut surface, Some(gamma));
        self.render_loading_league_signup_tooltip(&mut surface, gamma)?;
        self.render_loading_message_dialog_tooltip(&mut surface, gamma)?;
        frame.copy_from_slice(surface.pixels());
        if !defer_monitor_gamma {
            if let Some(gamma) = monitor_gamma.as_ref() {
                gamma.apply_to_rgba_bytes(frame);
            }
        }
        Ok(())
    }

    pub(crate) fn render_loading_message_dialogs(
        &mut self,
        surface: &mut Surface,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Result<()> {
        if self.message_dialogs.is_empty() {
            return Ok(());
        }
        let assets = Arc::clone(&self.assets);
        let resources = assets.message_dialog_resources().ok_or_else(|| {
            self.loader_boundary(
                "classic message-dialog resources are unavailable during network start wait",
            )
        })?;
        let last = self.message_dialogs.len() - 1;
        let active_index = self.active_message_dialog_index();
        let context_menu_closed = self.context_menu.is_none();
        let now = Instant::now();
        for index in 0..=last {
            let keyboard_active = Some(index) == active_index && context_menu_closed;
            let mouse_active = Some(index) == active_index;
            let result = self.message_dialogs[index].state.render_at(
                surface,
                resources,
                keyboard_active,
                mouse_active,
                Some(gamma),
                now,
            );
            if let Err(error) = result {
                return Err(self.loader_boundary(error.to_string()));
            }
        }

        Ok(())
    }

    fn render_loading_message_dialog_tooltip(
        &mut self,
        surface: &mut Surface,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Result<()> {
        if self.message_dialogs.is_empty() {
            return Ok(());
        }
        let assets = Arc::clone(&self.assets);
        let resources = assets.message_dialog_resources().ok_or_else(|| {
            self.loader_boundary(
                "classic message-dialog resources are unavailable during network start wait",
            )
        })?;

        let Some(tooltip_pointer) = self.startup_tooltip.eligible_pointer() else {
            return Ok(());
        };
        let (surface_width, surface_height) = (surface.width() as i32, surface.height() as i32);
        let Some(index) = (0..self.message_dialogs.len()).rev().find(|index| {
            let dialog = &self.message_dialogs[*index].state;
            let layout = dialog.layout(surface_width, surface_height, &resources.fonts.text);
            dialog
                .tooltip_state(Some(tooltip_pointer), &layout)
                .is_some()
        }) else {
            return Ok(());
        };
        let result = self.message_dialogs[index].state.render_tooltip(
            surface,
            resources,
            Some(tooltip_pointer),
            Some(gamma),
        );
        result.map_err(|error| self.loader_boundary(error.to_string()))
    }

    pub(crate) fn render_native_game_messages(
        &self,
        frame: &mut [u8],
        geometry: clonk_scaling::PresentationGeometry,
        gamma: &clonk_graphics::GammaRamp,
    ) -> Result<()> {
        let _renderer_config = clonk_frontend::activate_advanced_renderer_config(
            self.graphics.advanced_renderer_config(),
        );
        if self.mode != AppMode::Running || self.snapshot.hud.messages.is_empty() {
            return Ok(());
        }
        let fonts = self
            .native_startup_fonts
            .as_deref()
            .context("scale-native C4GameMessage fonts are unavailable")?;
        let (frame_width, frame_height) = geometry.physical_size();
        let expected_len = (frame_width as usize)
            .checked_mul(frame_height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .context("native C4GameMessage frame dimensions overflow")?;
        anyhow::ensure!(
            frame.len() == expected_len,
            "native C4GameMessage frame has {} bytes, expected {expected_len}",
            frame.len()
        );
        let logical = self.graphics.surface();
        anyhow::ensure!(
            geometry.logical_size() == (logical.width(), logical.height()),
            "native C4GameMessage geometry {:?} does not match its {}x{} logical target",
            geometry.logical_size(),
            logical.width(),
            logical.height(),
        );
        let mut surface = Surface::from_bytes(
            frame_width,
            frame_height,
            PixelFormat::Rgba8888,
            frame.to_vec(),
        )?;
        let viewports = self.graphics.active_viewport_projections();
        for viewport in viewports {
            for message in &self.snapshot.hud.messages {
                let target_position = match message.kind {
                    MessageKind::Global => None,
                    MessageKind::GlobalPlayer
                        if message.player.unwrap_or(OWNER_NONE) == viewport.owner =>
                    {
                        None
                    }
                    MessageKind::GlobalPlayer => continue,
                    MessageKind::Target | MessageKind::TargetPlayer => {
                        let Some(position) =
                            self.target_message_position_for_viewport(message, viewport)
                        else {
                            continue;
                        };
                        Some(position)
                    }
                };
                if !game_message::is_supported(message) {
                    continue;
                }
                let font_images = resolve_message_font_images(
                    &self.engine,
                    message,
                    self.script_text_spec_resources(),
                );
                if let Some(position) = target_position {
                    let anchor = viewport.logical_to_output(position);
                    game_message::draw_target_message_native(
                        &mut surface,
                        &fonts.text,
                        fonts.scale(),
                        geometry.logical_size(),
                        viewport.rect,
                        (anchor.0.round() as i32, anchor.1.round() as i32),
                        message,
                        &font_images,
                        Some(gamma),
                    )
                    .map_err(|detail| anyhow!("native C4GameMessage render failed: {detail}"))?;
                } else {
                    let portrait = message
                        .portrait
                        .as_deref()
                        .and_then(|spec| resolve_message_portrait(&self.engine, spec));
                    let decoration_image =
                        message.frame_decoration.as_ref().and_then(|decoration| {
                            self.engine
                                .definition_sprite_image(&decoration.source_definition, None)
                                .map(default_owner_definition_sprite)
                        });
                    game_message::draw_global_message_native(
                        &mut surface,
                        &fonts.text,
                        fonts.scale(),
                        geometry.logical_size(),
                        viewport.rect,
                        message,
                        message.frame_decoration.as_ref(),
                        decoration_image.as_ref(),
                        portrait.as_ref(),
                        &font_images,
                        Some(gamma),
                    )
                    .map_err(|detail| anyhow!("native C4GameMessage render failed: {detail}"))?;
                }
            }
        }
        frame.copy_from_slice(surface.pixels());
        Ok(())
    }

    fn viewport_player_is_eliminated(&self, owner: i32) -> bool {
        self.snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .is_some_and(|player| {
                player.surrendered
                    || matches!(
                        player.status,
                        clonk_engine::PlayerStatus::Eliminated
                            | clonk_engine::PlayerStatus::Surrendered
                    )
            })
    }

    pub(crate) fn physical_viewport_is_unsuppressed(
        &self,
        viewport: PhysicalViewportState,
    ) -> bool {
        viewport.displayed_player == OWNER_NONE
            || self.snapshot.players.iter().any(|player| {
                player.id == viewport.displayed_player
                    && !self.viewport_player_is_eliminated(player.id)
            })
    }

    pub(crate) fn menu_owner_has_unsuppressed_viewport(&self, menu_owner: i32) -> bool {
        let owner_exists = self
            .snapshot
            .players
            .iter()
            .any(|player| player.id == menu_owner);
        self.physical_viewports.iter().copied().any(|viewport| {
            let hosts_menu = if menu_owner == OWNER_NONE {
                viewport.is_no_owner_viewport
            } else {
                owner_exists && viewport.displayed_player == menu_owner
            };
            hosts_menu && self.physical_viewport_is_unsuppressed(viewport)
        })
    }

    fn viewport_elimination_notice_text(&self, owner: i32) -> Option<String> {
        let player = self
            .snapshot
            .players
            .iter()
            .find(|player| player.id == owner)?;
        if !self.viewport_player_is_eliminated(owner) {
            return None;
        }
        let surrendered =
            player.surrendered || player.status == clonk_engine::PlayerStatus::Surrendered;
        let (key, fallback) = if surrendered {
            ("IDS_PLR_SURRENDERED", "Player %s|has surrendered.")
        } else {
            ("IDS_PLR_ELIMINATED", "Player %s|eliminated.")
        };
        let template = self
            .startup_tooltip_resources
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.to_string());
        let name = c4_presentation_text(&player.name);
        Some(format_resource_string(template, &[&name]))
    }

    fn retained_gpu_frame_gamma(&self) -> clonk_graphics::GammaRamp {
        match self.mode {
            AppMode::Menu | AppMode::Loading => self.startup_active_gamma(),
            AppMode::Running => self
                .graphics
                .active_gamma_ramp(&self.snapshot.environment.gamma),
        }
    }

    pub(crate) fn render_retained_gpu_frame(
        &mut self,
        presentation: GpuPresentation,
    ) -> Result<RetainedGpuFrame> {
        let gamma = self.retained_gpu_frame_gamma();
        let renderer_config = self.graphics.advanced_renderer_config();
        let gamma_mode = retained_gpu_gamma_mode(renderer_config);
        let ordered_native = !self.console_mode
            && (self.can_present_ordered_native_text(presentation.scale)
                || self.can_defer_native_loader_text(presentation.scale));

        if ordered_native {
            self.retained_gpu_ordered_capture_active = true;
            let mut ignored_cpu_pixel = [0_u8; 4];
            let render = self.render_ordered_native_base(&mut ignored_cpu_pixel);
            self.retained_gpu_ordered_capture_active = false;
            render?;
            let plan = self
                .pending_native_presentation
                .take()
                .ok_or_else(|| anyhow!("ordered GPU presentation ended without a layer plan"))?;
            return self.retained_gpu_frame_from_native_plan(
                plan,
                presentation,
                &gamma,
                gamma_mode,
            );
        }

        self.graphics.begin_gpu_scene_capture();
        let mut ignored_cpu_pixel = [0_u8; 4];
        if let Err(error) = self.render_for_presentation_with_monitor_defer(
            &mut ignored_cpu_pixel,
            false,
            false,
            false,
            true,
        ) {
            let _ = self.graphics.finish_gpu_scene_capture(&gamma);
            return Err(error);
        }
        let mut scene = self
            .graphics
            .finish_gpu_scene_capture(&gamma)
            .ok_or_else(|| anyhow!("GPU scene capture ended before presentation"))?;
        scene.gamma_mode = gamma_mode;
        if let Some(plan) = self.pending_native_presentation.take() {
            let mut frame =
                self.retained_gpu_frame_from_native_plan(plan, presentation, &gamma, gamma_mode)?;
            if !scene.commands.is_empty() {
                frame.layers.push(RetainedGpuFrameLayer {
                    scene,
                    presentation,
                });
            }
            return Ok(frame);
        }
        Ok(RetainedGpuFrame {
            layers: vec![RetainedGpuFrameLayer {
                scene,
                presentation,
            }],
        })
    }

    fn retained_gpu_frame_from_native_plan(
        &mut self,
        plan: NativePresentationPlan,
        logical_presentation: GpuPresentation,
        gamma: &clonk_graphics::GammaRamp,
        gamma_mode: GpuGammaMode,
    ) -> Result<RetainedGpuFrame> {
        let logical_extent = [
            self.graphics.surface().width(),
            self.graphics.surface().height(),
        ];
        let [physical_width, physical_height] = logical_presentation.physical_extent;
        let default_fonts = self.native_startup_fonts.clone();
        let mut physical_surface = self
            .retained_native_capture_surface
            .take()
            .filter(|surface| {
                surface.width() == physical_width && surface.height() == physical_height
            })
            .unwrap_or_else(|| {
                Surface::new(physical_width, physical_height, PixelFormat::Rgba8888)
            });
        let mut layers = Vec::new();
        let result = (|| -> Result<()> {
            for batch in plan.batches {
                anyhow::ensure!(
                    batch.logical_layer.is_none(),
                    "ordered retained GPU capture produced a CPU logical layer"
                );
                if let Some(recorder) = batch.gpu_recorder {
                    let mut scene =
                        recorder.into_scene(logical_extent, Color::opaque(8, 12, 24), gamma);
                    scene.gamma_mode = gamma_mode;
                    layers.push(RetainedGpuFrameLayer {
                        scene,
                        presentation: logical_presentation,
                    });
                }

                if !batch.native_loader_text && batch.text.is_empty() {
                    continue;
                }
                physical_surface.clear_clip();
                debug_assert!(!physical_surface.is_gpu_scene_capture_active());
                physical_surface.begin_gpu_scene_capture();
                if batch.native_loader_text {
                    let default_fonts = default_fonts.as_deref().context(
                        "scale-native loader font bundle disappeared during GPU capture",
                    )?;
                    let loader = self.loader_screen.as_ref().ok_or_else(|| {
                        self.loader_boundary(
                            "selected classic loader disappeared during retained presentation",
                        )
                    })?;
                    loader
                        .render_native_text_to(
                            &mut physical_surface,
                            default_fonts,
                            logical_extent[0],
                            logical_extent[1],
                            Some(&self.startup_fragment_gamma()),
                        )
                        .map_err(|error| self.loader_boundary(error.to_string()))?;
                }
                if !batch.text.is_empty() {
                    let fonts = batch
                        .fonts
                        .as_deref()
                        .or(default_fonts.as_deref())
                        .context("scale-native font bundle disappeared during GPU capture")?;
                    fonts.draw_captured_text_to(
                        &mut physical_surface,
                        &batch.text,
                        (logical_extent[0], logical_extent[1]),
                    );
                }
                let recorder = physical_surface.take_gpu_scene_capture().ok_or_else(|| {
                    anyhow!("scale-native GPU text capture ended before presentation")
                })?;
                if recorder.is_empty() {
                    continue;
                }
                let mut scene = recorder.into_scene(
                    [physical_width, physical_height],
                    Color::transparent(),
                    gamma,
                );
                scene.gamma_mode = gamma_mode;
                layers.push(RetainedGpuFrameLayer {
                    scene,
                    presentation: GpuPresentation::identity(physical_width, physical_height),
                });
            }
            anyhow::ensure!(
                !layers.is_empty(),
                "retained GPU frame has no ordered layers"
            );
            Ok(())
        })();
        if physical_surface.is_gpu_scene_capture_active() {
            let _ = physical_surface.take_gpu_scene_capture();
        }
        self.retained_native_capture_surface = Some(physical_surface);
        result?;
        Ok(RetainedGpuFrame { layers })
    }

    pub(crate) fn render_running(
        &mut self,
        frame: &mut [u8],
        defer_native_game_messages: bool,
    ) -> Result<()> {
        self.render_running_for_presentation(frame, defer_native_game_messages, false)
    }

    fn render_running_for_presentation(
        &mut self,
        frame: &mut [u8],
        defer_native_game_messages: bool,
        defer_monitor_gamma: bool,
    ) -> Result<()> {
        let ordered_native = self.graphics.surface().is_clonk_text_capture_active();
        self.graphics.set_renderer_config(
            self.display_flags.show_player_hud_always,
            self.display_flags.splitscreen_dividers,
            self.display_flags.fire_particles,
        );
        // C4Viewport suppresses only its gameplay overlays for a film replay;
        // game messages and C4GraphicsSystem-owned chrome remain independent.
        let viewport_overlays_visible = !self.engine.film_replay();
        self.apply_show_commands_enable_request();
        self.sync_film_view_presentation();
        self.reject_classic_global_gui_bootstrap()?;
        self.assets
            .require_classic_hud_resources_with_hud(self.current_hud_graphics_ref())
            .map_err(report_classic_parity_boundary)?;
        self.preflight_visible_gui_overlay_resources()?;
        if let Some(dialog) = self.game_over_dialog.as_ref() {
            self.assets
                .require_classic_game_over_resources_with_hud_and_evaluation(
                    self.current_hud_graphics_ref(),
                    Some(dialog.evaluation()),
                )
                .map_err(report_classic_parity_boundary)?;
        }
        if viewport_overlays_visible && self.menu_owner_has_unsuppressed_viewport(self.local_owner)
        {
            if let Some(browser) = self.save_browser.as_ref() {
                let boundary = report_classic_parity_boundary(ClassicParityBoundary::SaveBrowser(
                    browser.mode().clone(),
                ));
                tracing::error!(%boundary, "refusing to render Rust-only save/load browser");
                return Err(anyhow::Error::new(boundary));
            }
        }
        if viewport_overlays_visible && self.menu_owner_has_unsuppressed_viewport(self.local_owner)
        {
            if let Some(menu) = self.object_menu.as_ref() {
                let boundary = report_classic_parity_boundary(
                    ClassicParityBoundary::AppObjectMenu(menu.mode()),
                );
                tracing::error!(%boundary, "refusing to render generic app-owned object menu");
                return Err(anyhow::Error::new(boundary));
            }
        }
        let mut script_menu_owners = Vec::new();
        if viewport_overlays_visible {
            let script_menu_viewports = collect_viewport_inputs_from_physical_state(
                &self.snapshot,
                &self.physical_viewports,
            )
            .map_err(|reason| {
                report_classic_parity_boundary(ClassicParityBoundary::RunningViewport(reason))
            })?;
            for viewport in script_menu_viewports {
                if self.viewport_player_is_eliminated(viewport.owner) {
                    continue;
                }
                if !script_menu_owners.contains(&viewport.owner) {
                    script_menu_owners.push(viewport.owner);
                }
            }
        }
        let has_visible_ingame_menu = viewport_overlays_visible
            && self
                .ingame_menu
                .iter()
                .any(|(owner, _)| self.ingame_menu_has_visible_surface(owner));
        let has_visible_script_menu = viewport_overlays_visible
            && script_menu_owners
                .iter()
                .any(|&owner| self.engine.cursor_object_menu(owner).is_some());
        // C4Menu::InitMenu registers every active menu as a C4GUI dialog.
        // Screen::IsActive therefore retains GUI mouse ownership even when
        // film/replay or viewport suppression omits the menu's pixels.
        let has_shown_external_menu = self.running_external_menu_is_shown();
        if viewport_overlays_visible && (has_visible_ingame_menu || has_visible_script_menu) {
            self.assets
                .require_classic_ingame_menu_resources()
                .map_err(report_classic_parity_boundary)?;
        }
        if viewport_overlays_visible {
            for &owner in &script_menu_owners {
                if let Some((_, menu)) = self.engine.cursor_object_menu(owner) {
                    if !matches!(menu.style, 0..=3) {
                        tracing::error!(
                            owner,
                            style = menu.style,
                            "refusing to render generic script-menu style fallback"
                        );
                        anyhow::bail!(
                            "classic script menu style {} is unavailable for owner {owner}; refusing generic Rust fallback",
                            menu.style
                        );
                    }
                }
            }
        }
        let runtime_help_columns = self.preflight_visible_runtime_help()?;
        let runtime_hold_message_visible = self.runtime_halt_active();
        let runtime_flash_message = self.preflight_visible_runtime_flash()?;
        let runtime_font_images = {
            let mut texts = Vec::new();
            if let Some(columns) = runtime_help_columns.as_ref() {
                texts.push(columns.left.as_str());
                texts.push(columns.right.as_str());
            }
            if let Some(message) = runtime_flash_message.as_ref() {
                texts.push(message.text.as_str());
            }
            resolve_font_images_in_texts(&self.engine, texts, self.script_text_spec_resources())
        };
        // Scoreboard reconciliation mutates presentation/refcount state. All
        // already-visible running layers must prove their exact resources or
        // typed refusal before that mutation can occur.
        self.reconcile_initial_scoreboard();
        self.sync_scoreboard_presentation();
        self.reconcile_running_mouse_after_last_gui_close(has_shown_external_menu)?;
        let scoreboard_font_images = self.preflight_visible_scoreboard()?;
        let message_board = self.advance_message_board_overlay();
        self.update_network_status_overlay();
        let viewports =
            collect_viewport_inputs_from_physical_state(&self.snapshot, &self.physical_viewports)
                .map_err(|reason| {
                report_classic_parity_boundary(ClassicParityBoundary::RunningViewport(reason))
            })?;
        // Capture CStdDDraw's installed ramp before render_frame latches any
        // runtime SetGamma controls for the next pass. C++ draws every GUI
        // overlay below with this same pre-latch ramp
        // (C4GraphicsSystem.cpp:160-199).
        let active_gamma = self
            .graphics
            .active_gamma_ramp(&self.snapshot.environment.gamma);
        let monitor_gamma = self
            .graphics
            .monitor_gamma_enabled()
            .then(|| active_gamma.clone());
        let frame_gamma = if self.graphics.fragment_gamma_enabled() {
            active_gamma
        } else {
            clonk_graphics::GammaRamp::identity()
        };
        if ordered_native {
            self.pending_native_presentation
                .as_mut()
                .expect("ordered presentation plan is active")
                .monitor_gamma = monitor_gamma.clone();
        }
        let mut value_footer_players = Vec::new();
        for &menu_owner in &script_menu_owners {
            let player = self
                .engine
                .cursor_object_menu(menu_owner)
                .and_then(|(_, menu)| {
                    (menu.extra == clonk_engine::ObjectMenuExtra::Value)
                        .then_some(menu.command_object)
                        .flatten()
                })
                .and_then(|command_object| self.engine.object_controller(command_object))
                .filter(|controller| self.engine.player(*controller).is_some());
            if let Some(player) = player.filter(|player| !value_footer_players.contains(player)) {
                value_footer_players.push(player);
            }
        }
        for owner in value_footer_players {
            self.engine
                .arm_player_view_wealth(owner)
                .map_err(anyhow::Error::new)?;
        }
        let mut players = if viewport_overlays_visible {
            collect_player_overlays_for_viewports(
                &mut self.engine,
                &self.snapshot,
                self.focus_id,
                &self.bindings,
                &self.gamepad_bindings,
                &viewports,
            )
        } else {
            Vec::new()
        };
        populate_crew_inventories(
            &self.engine,
            &self.snapshot,
            &mut players,
            self.graphics.advanced_renderer_config(),
        );
        self.populate_crew_infos(&mut players);
        self.populate_crew_portraits(&mut players);
        // Command rows for the local player's real cursor
        // (C4Viewport::DrawCursorInfo, src/C4Viewport.cpp:948-961),
        // skipped while the cursor's menu is active
        // (src/C4Object.cpp:2952).
        if viewport_overlays_visible
            && self.display_flags.show_commands
            && self.object_menu.is_none()
            && self.engine.cursor_object_menu(self.local_owner).is_none()
        {
            let cursor_id = self
                .snapshot
                .players
                .iter()
                .find(|player| player.id == self.local_owner)
                .and_then(|player| player.cursor);
            if let Some(cursor_id) = cursor_id {
                let flash_command = self
                    .snapshot
                    .object(cursor_id)
                    .and_then(|cursor| self.engine.player(cursor.owner))
                    .map(|player| player.flash_command())
                    .unwrap_or(0);
                let ctx = AppCommandContext {
                    engine: &self.engine,
                    bindings: &self.bindings,
                    snapshot: &self.snapshot,
                    resources: &self.startup_tooltip_resources,
                };
                let commands =
                    draw_commands::build_cursor_commands(&self.snapshot, cursor_id, &ctx);
                if let Some(overlay) = players
                    .iter_mut()
                    .find(|player| player.owner == self.local_owner)
                {
                    overlay.commands = commands;
                    // C4Object::DrawCommand looks up FlashCom through the
                    // command-producing object's Owner, not Controller.
                    overlay.flash_command = flash_command;
                }
            }
        }
        let crew_name_labels = self.crew_name_overlays(&viewports);
        let overlay = GraphicsOverlay {
            frame_text: &self.frame_text,
            status_text: &self.status_text,
            debug_hud: self.debug_hud,
            viewport_overlays_visible,
            players,
            game_time_seconds: self.game_time_seconds(),
            message_board,
            crew_name_labels,
            clock_text: self
                .display_flags
                .clock
                .then(|| clonk_core::chrono_util::current_timestamp(false)),
            frames_per_second: self.display_flags.fps.then_some(self.frames_per_second),
            upper_board_mode: frontend_upper_board_mode(self.display_flags.upper_board),
            // Config.Graphics.ShowPortraits/ShowCommands/ShowCommandKeys
            // from the Display menu (src/C4Config.cpp:448-450).
            show_portraits: self.display_flags.portraits,
            show_commands: self.display_flags.show_commands,
            show_command_keys: self.display_flags.show_command_keys,
        };
        self.graphics.update_overlay(&overlay);
        if ordered_native {
            let pending_hud = self.graphics.render_frame_base(&self.snapshot, &viewports);
            let surface = self.graphics.surface_mut();
            let text = surface.take_clonk_text_capture();
            if surface.pixels().len() == frame.len() {
                frame.copy_from_slice(surface.pixels());
            } else {
                copy_surface(surface.pixels(), surface.width(), surface.height(), frame);
            }
            self.pending_native_presentation
                .as_mut()
                .expect("ordered presentation plan is active")
                .batches
                .push(NativePresentationBatch {
                    logical_layer: None,
                    clip: None,
                    native_loader_text: false,
                    text,
                    fonts: None,
                    gpu_recorder: surface.take_gpu_scene_capture(),
                });
            surface.clear_clip();
            if !self.retained_gpu_ordered_capture_active {
                surface.fill(Color::transparent());
            }
            surface.begin_clonk_text_capture();
            if self.retained_gpu_ordered_capture_active {
                debug_assert!(!surface.is_gpu_scene_capture_active());
                surface.begin_gpu_scene_capture();
            }
            self.graphics.render_frame_foreground(&pending_hud);
            Self::next_native_overlay_parts(
                &mut self.graphics,
                &mut self.pending_native_presentation,
                self.retained_gpu_ordered_capture_active,
            );
            let pending_chrome = self.graphics.render_frame_hud_players(pending_hud);
            Self::next_native_overlay_parts(
                &mut self.graphics,
                &mut self.pending_native_presentation,
                self.retained_gpu_ordered_capture_active,
            );
            self.graphics
                .render_frame_hud_chrome_without_atlas_deferred_monitor_gamma(pending_chrome);
            Self::next_native_overlay_parts(
                &mut self.graphics,
                &mut self.pending_native_presentation,
                self.retained_gpu_ordered_capture_active,
            );
        } else {
            self.graphics
                .render_frame_without_atlas_deferred_monitor_gamma(&self.snapshot, &viewports);
        }
        // C4Viewport::AdjustPosition consumes ViewOffs for an ownerless
        // physical viewport after each successful draw, even when film mode
        // has temporarily assigned it a valid player.
        for viewport in &mut self.physical_viewports {
            if viewport.is_no_owner_viewport {
                viewport.preserved_offset = Vector2::ZERO;
            }
        }
        self.reset_menu_positions_for_viewport_changes();
        // Rendering latches the current C4Viewport ViewX/ViewY. Reproject a
        // stationary construction cursor before drawing its phase so camera
        // following and viewport-layout changes cannot leave one stale frame.
        self.refresh_construction_menu_drag();

        let engine = &self.engine;
        self.script_menu_presentations
            .retain(|owner, _| engine.cursor_object_menu(*owner).is_some());
        let has_visible_script_menu = script_menu_owners
            .iter()
            .any(|&owner| self.engine.cursor_object_menu(owner).is_some());
        for script_menu_owner in script_menu_owners {
            let mut script_menu = self
                .engine
                .cursor_object_menu(script_menu_owner)
                .map(|(target, menu)| (target, menu.clone()));
            if let Some((_, menu)) = script_menu.as_mut() {
                resolve_engine_script_menu_footer(&mut self.engine, menu)?;
            }
            let initial_script_menu_location = script_menu
                .as_ref()
                .and_then(|(_, menu)| self.script_menu_free_location(script_menu_owner, menu));
            let script_menu_time = script_menu
                .as_ref()
                .map(|(target, menu)| {
                    let key = ScriptMenuPresentationKey {
                        target: *target,
                        runtime_id: menu.runtime_id,
                        symbol_id: menu.symbol_id.clone(),
                        caption: menu.caption.clone(),
                        selection: menu.selection,
                        location: menu.location,
                    };
                    let progressing = menu.text_progressing;
                    match self.script_menu_presentations.remove(&script_menu_owner) {
                        Some(mut state) if state.key == key => {
                            if state.location.is_none() {
                                state.location = initial_script_menu_location;
                                state.location_needs_initialization =
                                    initial_script_menu_location.is_some();
                                state.free_aligned |= initial_script_menu_location.is_some();
                            }
                            if !progressing {
                                state.time_on_selection = state.time_on_selection.saturating_add(1);
                            }
                            let time = state.time_on_selection;
                            self.script_menu_presentations
                                .insert(script_menu_owner, state);
                            time
                        }
                        Some(mut state) if same_script_menu_presentation(&state, *target, menu) => {
                            state.key = key;
                            if state.location.is_none() {
                                state.location = initial_script_menu_location;
                                state.location_needs_initialization =
                                    initial_script_menu_location.is_some();
                                state.free_aligned |= initial_script_menu_location.is_some();
                            }
                            state.time_on_selection = u32::from(!progressing);
                            state.selection_needs_adjustment |=
                                state.scroll_selection != menu.selection;
                            let time = state.time_on_selection;
                            self.script_menu_presentations
                                .insert(script_menu_owner, state);
                            time
                        }
                        _ => {
                            let time_on_selection = u32::from(!progressing);
                            self.script_menu_presentations.insert(
                                script_menu_owner,
                                ScriptMenuPresentationState {
                                    key,
                                    time_on_selection,
                                    location: initial_script_menu_location,
                                    location_needs_initialization: initial_script_menu_location
                                        .is_some(),
                                    free_aligned: initial_script_menu_location.is_some(),
                                    scroll_y: 0,
                                    scroll_selection: menu.selection,
                                    selection_needs_adjustment: true,
                                },
                            );
                            time_on_selection
                        }
                    }
                })
                .unwrap_or_else(|| {
                    self.script_menu_presentations.remove(&script_menu_owner);
                    if self
                        .script_menu_close_pointer_capture
                        .is_some_and(|(owner, _)| owner == script_menu_owner)
                    {
                        self.script_menu_close_pointer_capture = None;
                    }
                    if matches!(
                        self.menu_title_drag,
                        Some(MenuTitleDrag::Script { owner, .. }) if owner == script_menu_owner
                    ) {
                        self.menu_title_drag = None;
                    }
                    0
                });
            if let Some((target, menu)) = script_menu.as_ref() {
                let fonts = self.assets.clonk_fonts.clone();
                let fallback = self.assets.font_arc();
                let legacy_title_id = menu.identification.to_string();
                let legacy_title_id = legacy_title_id
                    .strip_prefix('"')
                    .and_then(|id| id.strip_suffix('"'))
                    .unwrap_or(&legacy_title_id);
                let title_id = if menu.symbol_id.is_empty() {
                    legacy_title_id
                } else {
                    &menu.symbol_id
                };
                let title_icon = self
                    .engine
                    .definition_picture_image(title_id)
                    .map(definition_menu_picture);
                let item_definition_color = if !menu.user_menu
                    && matches!(
                        menu.title_symbol,
                        clonk_engine::ObjectMenuSymbol::Buy { .. }
                    ) {
                    object_menu_buying_player_color(&self.snapshot, menu.command_object)
                } else {
                    0
                };
                let text_spec_resources = self.script_text_spec_resources();
                let font_images = resolve_script_menu_font_images(
                    &self.engine,
                    menu,
                    text_spec_resources,
                )
                .map_err(|error| {
                    tracing::error!(%error, "classic menu text-image resource preflight failed");
                    error
                })?;
                let hud_graphics = self.current_hud_graphics();
                let allowed_blit_modes =
                    self.graphics.advanced_renderer_config().allowed_blit_modes;
                let item_icons = menu
                    .items
                    .iter()
                    .map(|item| {
                        object_menu_item_picture_with_renderer_modes(
                            &self.engine,
                            &self.snapshot,
                            item,
                            item_definition_color,
                            &hud_graphics,
                            menu.style,
                            text_spec_resources,
                            allowed_blit_modes,
                        )
                    })
                    .collect::<Vec<_>>();
                for (index, (item, image)) in menu.items.iter().zip(&item_icons).enumerate() {
                    // Native Buy/Sell/Exit/etc. rows retain Definition as the
                    // serde default while their non-definition symbol still
                    // supplies a valid icon. AddMenuItem picture recipes use
                    // the Definition symbol and would otherwise render blank.
                    if item.symbol == clonk_engine::ObjectMenuSymbol::Definition
                        && item.image != clonk_engine::ObjectMenuImage::None
                        && image.is_none()
                    {
                        tracing::error!(
                            index,
                            style = menu.style,
                            recipe = ?item.image,
                            "classic menu image preflight failed"
                        );
                        anyhow::bail!(
                            "unresolved classic menu image at item {index}: {:?}",
                            item.image
                        );
                    }
                }
                let selected_component_icons = usize::try_from(menu.selection)
                    .ok()
                    .and_then(|selection| menu.items.get(selection))
                    .map(|item| {
                        item.components
                            .iter()
                            .map(|component| {
                                self.engine
                                    .definition_picture_image(&component.definition_id)
                                    .map(definition_menu_picture)
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let (menu_location, retained_scroll_y, adjust_selection, initialize_location) =
                    self.script_menu_presentations
                        .get(&script_menu_owner)
                        .filter(|state| same_script_menu_presentation(state, *target, menu))
                        .map(|state| {
                            (
                                state.location,
                                state.scroll_y,
                                state.selection_needs_adjustment,
                                state.location_needs_initialization,
                            )
                        })
                        .unwrap_or((None, 0, true, false));
                let area = self
                    .graphics
                    .viewport_rect(script_menu_owner)
                    .unwrap_or_else(|| {
                        let surface = self.graphics.surface();
                        Rect::new(0, 0, surface.width(), surface.height())
                    });
                let layout_font =
                    clonk_frontend::hud::HudFont::from_set(fonts.as_deref(), fallback.as_ref());
                let (menu_location, menu_scroll_y) = if matches!(menu.style, 0..=2) {
                    let layout = if initialize_location {
                        engine_script_menu_layout_with_free_anchor(
                            area,
                            &layout_font,
                            menu,
                            self.display_flags.show_commands,
                            &font_images,
                            menu_location.expect("free anchor has a location"),
                            retained_scroll_y,
                            adjust_selection,
                        )
                    } else {
                        engine_script_menu_layout_with_presentation(
                            area,
                            &layout_font,
                            menu,
                            self.display_flags.show_commands,
                            &font_images,
                            menu_location,
                            retained_scroll_y,
                            adjust_selection,
                        )
                    };
                    (
                        initialize_location
                            .then_some((layout.bounds.x, layout.bounds.y))
                            .or(menu_location),
                        layout.scroll_y,
                    )
                } else if menu.style == 3 && initialize_location {
                    let geometry = engine_script_menu_presentation_geometry_with_free_anchor(
                        area,
                        &layout_font,
                        menu,
                        self.display_flags.show_commands,
                        &font_images,
                        menu_location.expect("free anchor has a location"),
                        retained_scroll_y,
                        adjust_selection,
                    )
                    .expect("supported dialog menu has presentation geometry");
                    (
                        Some((geometry.bounds.x, geometry.bounds.y)),
                        geometry.scroll_y,
                    )
                } else {
                    (menu_location, retained_scroll_y)
                };
                if let Some(state) = self
                    .script_menu_presentations
                    .get_mut(&script_menu_owner)
                    .filter(|state| same_script_menu_presentation(state, *target, menu))
                {
                    state.location = menu_location;
                    state.location_needs_initialization = false;
                    state.scroll_y = menu_scroll_y;
                    state.scroll_selection = menu.selection;
                    state.selection_needs_adjustment = false;
                }
                let frame_decoration = menu.decoration.as_ref().and_then(|decoration| {
                    self.engine
                        .definition_sprite_image(&decoration.source_definition, None)
                        .map(default_owner_definition_sprite)
                });
                if let Some(decoration) = menu.decoration.as_ref() {
                    if let Err(error) = validate_menu_decoration_for_area(
                        area,
                        decoration,
                        frame_decoration.as_ref(),
                    ) {
                        tracing::error!(
                            decoration = ?menu.decoration,
                            %error,
                            "classic menu decoration preflight failed"
                        );
                        anyhow::bail!("invalid classic menu frame decoration: {error}");
                    }
                }
                {
                    let show_commands = self.display_flags.show_commands;
                    let show_command_keys = self.display_flags.show_command_keys;
                    let owner_colors = self
                        .snapshot
                        .players
                        .iter()
                        .map(|player| {
                            let color = player
                                .color
                                .map(|RgbColor { r, g, b }| Color::opaque(r, g, b))
                                .unwrap_or_else(|| default_owner_color(player.id));
                            (player.id, color)
                        })
                        .collect();
                    let gfx = self.ensure_ingame_menu_gfx();
                    gfx.show_commands = show_commands;
                    gfx.show_command_keys = show_command_keys;
                    gfx.owner_colors = owner_colors;
                    gfx.font_images = font_images;
                    gfx.frame_decoration = frame_decoration;
                    gfx.menu_location = menu_location;
                    gfx.menu_scroll_y = menu_scroll_y;
                }
                let script_menu_accepts_mouse = self.mouse_control
                    && self.local_controls.mouse_owner() == Some(script_menu_owner);
                if let Some(gfx) = self.ingame_menu_gfx.as_ref() {
                    let font =
                        clonk_frontend::hud::HudFont::from_set(fonts.as_deref(), fallback.as_ref());
                    let tiny = fonts
                        .as_deref()
                        .map(|set| clonk_frontend::hud::HudFont::Clonk(&set.mini));
                    let dim_for_construction_drag =
                        self.construction_menu_drag.as_ref().is_some_and(|drag| {
                            matches!(
                                drag,
                                ConstructionMenuDrag::Active { owner, .. }
                                    if *owner == script_menu_owner
                            )
                        });
                    if dim_for_construction_drag {
                        let surface = self.graphics.surface_mut();
                        let mut menu_layer =
                            Surface::new(surface.width(), surface.height(), surface.format());
                        let capture_text = surface.is_clonk_text_capture_active();
                        if capture_text {
                            menu_layer.begin_clonk_text_capture();
                        }
                        render_engine_script_menu_with_gamma(
                            &mut menu_layer,
                            area,
                            &font,
                            fallback.as_ref(),
                            tiny.as_ref(),
                            menu,
                            gfx,
                            title_icon.as_ref(),
                            &item_icons,
                            &selected_component_icons,
                            script_menu_accepts_mouse,
                            script_menu_time,
                            Some(&frame_gamma),
                        );
                        let modulation = Color::new(255, 255, 255, 0xaf);
                        surface.blit_region_modulated(
                            &menu_layer,
                            menu_layer.bounds(),
                            SurfacePoint::new(0, 0),
                            modulation,
                        )?;
                        if capture_text {
                            surface.extend_clonk_text_capture_from_modulated(
                                &mut menu_layer,
                                SurfacePoint::new(0, 0),
                                modulation,
                            );
                        }
                    } else {
                        let surface = self.graphics.surface_mut();
                        render_engine_script_menu_with_gamma(
                            surface,
                            area,
                            &font,
                            fallback.as_ref(),
                            tiny.as_ref(),
                            menu,
                            gfx,
                            title_icon.as_ref(),
                            &item_icons,
                            &selected_component_icons,
                            script_menu_accepts_mouse,
                            script_menu_time,
                            Some(&frame_gamma),
                        );
                    }
                }
            }
        }
        if ordered_native && has_visible_script_menu {
            self.next_pending_native_overlay();
        }

        if has_visible_ingame_menu {
            let fonts = self.assets.clonk_fonts.clone();
            let fallback = self.assets.font_arc();
            let players = self
                .ingame_menu
                .iter()
                .map(|(player, _)| player)
                .filter(|&player| self.ingame_menu_has_visible_surface(player))
                .collect::<Vec<_>>();
            {
                let show_commands = self.display_flags.show_commands;
                let show_command_keys = self.display_flags.show_command_keys;
                let show_portraits = self.display_flags.portraits;
                self.hydrate_runtime_player_big_icons();
                let owner_colors = self
                    .engine
                    .players()
                    .map(|player| {
                        let color = player
                            .color()
                            .map(|RgbColor { r, g, b }| Color::opaque(r, g, b))
                            .unwrap_or_else(|| Color::opaque(0, 0, 0xff));
                        (player.id(), color)
                    })
                    .collect();
                let hostility_big_icons = self
                    .engine
                    .players()
                    .filter_map(|player| {
                        self.runtime_player_big_icons
                            .get(&player.player_info_id())
                            .cloned()
                            .map(|icon| (player.id(), icon))
                    })
                    .collect();
                let gfx = self.ensure_ingame_menu_gfx();
                gfx.show_commands = show_commands;
                gfx.show_command_keys = show_command_keys;
                gfx.show_portraits = show_portraits;
                gfx.owner_colors = owner_colors;
                gfx.hostility_big_icons = hostility_big_icons;
            }
            for player in players {
                let area = self.graphics.viewport_rect(player).unwrap_or_else(|| {
                    let surface = self.graphics.surface();
                    Rect::new(0, 0, surface.width(), surface.height())
                });
                if let Some(gfx) = self.ingame_menu_gfx.as_mut() {
                    gfx.show_close_button = self.local_controls.mouse_owner() == Some(player);
                }
                if let (Some(menu), Some(gfx)) =
                    (self.ingame_menu.get(player), self.ingame_menu_gfx.as_ref())
                {
                    // FontRegular for items, FontTiny for command-key labels
                    // (C4Menu.cpp:170; C4ObjectCom.cpp:940).
                    let font =
                        clonk_frontend::hud::HudFont::from_set(fonts.as_deref(), fallback.as_ref());
                    let tiny = fonts
                        .as_deref()
                        .map(|set| clonk_frontend::hud::HudFont::Clonk(&set.mini));
                    let surface = self.graphics.surface_mut();
                    menu.render_with_gamma(
                        surface,
                        area,
                        &font,
                        tiny.as_ref(),
                        gfx,
                        Some(&frame_gamma),
                    );
                }
            }
        }
        if ordered_native && has_visible_ingame_menu {
            self.next_pending_native_overlay();
        }

        let elimination_notices = if viewport_overlays_visible {
            self.graphics
                .active_viewport_projections()
                .into_iter()
                .filter_map(|viewport| {
                    self.viewport_elimination_notice_text(viewport.owner)
                        .map(|text| (viewport.rect, text))
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if !elimination_notices.is_empty() {
            let fonts = self
                .assets
                .clonk_fonts
                .clone()
                .expect("global GUI preflight guarantees FontRegular");
            let surface = self.graphics.surface_mut();
            let previous_clip = surface.clip();
            for (viewport, text) in elimination_notices {
                let width = i32::try_from(viewport.width).unwrap_or(i32::MAX);
                let height = i32::try_from(viewport.height).unwrap_or(i32::MAX);
                surface.set_clip(viewport);
                fonts.text.draw_with_gamma(
                    surface,
                    viewport.x.saturating_add(width / 2),
                    viewport.y.saturating_add(height.saturating_mul(2) / 3),
                    &text,
                    [255, 0, 0, 0xfa],
                    clonk_graphics::clonk_font::TextAlign::Center,
                    true,
                    Some(&frame_gamma),
                );
            }
            match previous_clip {
                Some(clip) => surface.set_clip(clip),
                None => surface.clear_clip(),
            }
            if ordered_native {
                self.next_pending_native_overlay();
            }
        }

        let message_viewports = self.graphics.active_viewport_projections();
        let mut unsupported_message_count = 0;
        for message in &self.snapshot.hud.messages {
            match self.hud_message_drawability(message, &message_viewports) {
                HudMessageDrawability::Drawable => {
                    if !game_message::is_supported(message) {
                        unsupported_message_count += 1;
                    }
                }
                HudMessageDrawability::NotDrawable => {}
            }
        }
        if unsupported_message_count != 0 {
            return Err(anyhow::Error::new(report_classic_parity_boundary(
                ClassicParityBoundary::HudGameMessage {
                    count: unsupported_message_count,
                },
            )));
        }
        self.draw_classic_game_messages(&frame_gamma, defer_native_game_messages)?;

        // C4Viewport draws this local control layer after menus and game
        // messages. Classic Chat is conditional on the process-global IRC
        // client being connecting or connected.
        if viewport_overlays_visible {
            let mouse_viewport_index = self
                .active_ingame_mouse_viewport()
                .map(|viewport| viewport.index);
            let irc_chat_active = self.startup_irc_client_active();
            self.graphics.draw_viewport_control_overlays(
                mouse_viewport_index,
                irc_chat_active,
                None,
                Some(&frame_gamma),
            );
        }

        // C4Viewport draws menus and game messages before C4MouseControl;
        // construction previews and selection frames therefore remain
        // legible over both (src/C4Viewport.cpp:836-870;
        // src/C4MouseControl.cpp:317-430,1093-1113).
        let running_world_cursor_drawable = viewport_overlays_visible
            && self.running_world_mouse_owned
            && self.window_active
            && self.pointer_inside_window;
        let construction_cursor = running_world_cursor_drawable
            .then(|| {
                self.construction_menu_drag
                    .as_ref()
                    .and_then(|drag| match drag {
                        ConstructionMenuDrag::Active {
                            definition_id,
                            viewport_index: Some(viewport_index),
                            pointer: Some(pointer),
                            site_valid,
                            ..
                        } => Some((definition_id, *viewport_index, *pointer, *site_valid)),
                        _ => None,
                    })
            })
            .flatten();
        let construction_preview =
            construction_cursor.and_then(|(definition_id, viewport_index, pointer, site_valid)| {
                self.engine
                    .definition_construction_drag_image(definition_id)
                    .map(|image| {
                        (
                            definition_menu_picture(image),
                            viewport_index,
                            pointer,
                            site_valid,
                        )
                    })
            });
        let construction_viewport_clip =
            construction_cursor.and_then(|(_, viewport_index, _, _)| {
                self.graphics
                    .active_viewport_projections()
                    .into_iter()
                    .find(|viewport| viewport.index == viewport_index)
                    .map(|viewport| viewport.rect)
            });
        let construction_primary_offset = construction_preview
            .as_ref()
            .map(|(image, _, _, _)| {
                GuiPoint::new((image.width() / 2) as f32, image.height() as f32)
            })
            .or_else(|| self.graphics.construction_cursor_primary_offset());
        let construction_cursor_drawn =
            match (construction_preview.as_ref(), construction_viewport_clip) {
                (Some((image, _, pointer, valid)), Some(viewport_clip)) => {
                    self.graphics.draw_construction_drag_preview(
                        image,
                        viewport_clip,
                        pointer.screen,
                        *valid,
                        Some(&frame_gamma),
                    )
                }
                (None, Some(viewport_clip)) => {
                    construction_cursor.is_some_and(|(_, _, pointer, _)| {
                        self.graphics.draw_construction_cursor_fallback(
                            viewport_clip,
                            pointer.screen,
                            Some(&frame_gamma),
                        )
                    })
                }
                _ => false,
            };
        if construction_cursor_drawn && self.mouse_control && self.keyboard_modifiers.shift() {
            if let (Some((_, _, pointer, _)), Some(primary_offset), Some(viewport_clip)) = (
                construction_cursor,
                construction_primary_offset,
                construction_viewport_clip,
            ) {
                self.graphics.draw_construction_add_marker(
                    viewport_clip,
                    pointer.screen,
                    primary_offset,
                    Some(&frame_gamma),
                );
            }
        }
        let selection_frame_drawn = if running_world_cursor_drawable {
            if let Some((selection, down_world, current_screen)) = self.ingame_selection_frame() {
                self.graphics.draw_mouse_selection_marks(
                    &self.snapshot,
                    self.local_owner,
                    &selection,
                    Some(&frame_gamma),
                );
                self.graphics.draw_mouse_selection_frame(
                    self.local_owner,
                    down_world,
                    current_screen,
                    Some(&frame_gamma),
                );
                true
            } else {
                false
            }
        } else {
            false
        };
        if !construction_cursor_drawn
            && !self.ingame_construction_drag_active()
            && !selection_frame_drawn
            && running_world_cursor_drawable
        {
            if let Some(pointer) = self.ingame_pointer.filter(|pointer| {
                self.window_active && self.ingame_mouse_controls_owner(pointer.owner)
            }) {
                let viewport = self.ingame_viewport_mouse.and_then(|retained| {
                    self.graphics
                        .active_viewport_projections()
                        .into_iter()
                        .find(|viewport| viewport.index == retained.viewport_index)
                });
                if let Some(viewport) = viewport {
                    let (cursor_kind, screen) = if self.ingame_help_cursor_active() {
                        (IngameMouseCursorKind::Help, pointer.screen)
                    } else if self.ingame_edge_cursor_active() {
                        self.ingame_edge_scroll.map_or(
                            (self.ingame_mouse_caption.cursor, pointer.screen),
                            |scroll| {
                                (
                                    IngameMouseCursorKind::Scrolling(scroll.edge.cursor),
                                    scroll.screen,
                                )
                            },
                        )
                    } else {
                        (self.ingame_mouse_caption.cursor, pointer.screen)
                    };
                    let phase = cursor_kind.phase();
                    let cursor_drawn = self.graphics.draw_mouse_cursor_clipped(
                        phase,
                        viewport.rect,
                        screen,
                        Some(&frame_gamma),
                    );
                    if cursor_drawn {
                        if let Some(landing) = cursor_kind.throw_landing() {
                            let (x, y) = viewport.logical_to_output(landing);
                            self.graphics.draw_mouse_cursor_clipped(
                                MouseCursorPhase::Point,
                                viewport.rect,
                                GuiPoint::new(x, y),
                                Some(&frame_gamma),
                            );
                        }
                        if self.mouse_control
                            && self.keyboard_modifiers.shift()
                            && cursor_kind.allows_add_marker()
                        {
                            if let Some(primary_offset) =
                                self.graphics.mouse_cursor_primary_offset(phase)
                            {
                                self.graphics.draw_construction_add_marker(
                                    viewport.rect,
                                    screen,
                                    primary_offset,
                                    Some(&frame_gamma),
                                );
                            }
                        }
                    }
                }
            }
        }
        let help_caption = running_world_cursor_drawable
            .then(|| {
                self.ingame_mouse_help_caption
                    .as_ref()
                    .zip(self.ingame_pointer)
                    .and_then(|(caption, pointer)| {
                        self.graphics
                            .viewport_rect(pointer.owner)
                            .map(|facet| (caption.text.clone(), pointer.screen, facet))
                    })
            })
            .flatten();
        if let (Some((caption, pointer, facet)), Some(font)) =
            (help_caption, self.assets.global_tooltip_font.clone())
        {
            let font = clonk_frontend::hud::HudFont::Clonk(font.as_ref());
            let caption = c4_presentation_text(&caption);
            ingame_menu::draw_tooltip(
                self.graphics.surface_mut(),
                &font,
                facet,
                pointer.x as i32,
                pointer.y as i32,
                &caption,
                Some(&frame_gamma),
            );
        } else if let Some((caption, viewport)) = running_world_cursor_drawable
            .then(|| {
                self.ingame_mouse_caption
                    .caption
                    .clone()
                    .and_then(|caption| {
                        self.graphics
                            .active_viewport_projections()
                            .into_iter()
                            .find(|viewport| viewport.index == caption.viewport_index)
                            .map(|viewport| (caption, viewport.rect))
                    })
            })
            .flatten()
        {
            let fonts = self.assets.clonk_fonts.clone();
            let fallback = self.assets.font_arc();
            let font = clonk_frontend::hud::HudFont::from_set(fonts.as_deref(), fallback.as_ref());
            let pointer = GuiPoint::new(
                viewport.x.saturating_add(caption.position.x) as f32,
                viewport.y.saturating_add(caption.position.y) as f32,
            );
            clonk_frontend::hud::draw_mouse_caption(
                self.graphics.surface_mut(),
                &font,
                viewport,
                pointer,
                caption
                    .caption_bottom_y
                    .map(|bottom| viewport.y.saturating_add(bottom)),
                &caption.text,
                Some(&frame_gamma),
            );
        }
        if ordered_native {
            self.next_pending_native_overlay();
        }

        // Native C4Viewport draws network status after its complete viewport
        // overlay (menus, messages, controls and mouse), but before the
        // process-global GUI layers below.
        if self.graphics.draw_network_status(Some(&frame_gamma)) && ordered_native {
            self.next_pending_native_overlay();
        }

        if let Some(columns) = runtime_help_columns.as_ref() {
            let fonts = self
                .assets
                .clonk_fonts
                .clone()
                .expect("global GUI preflight guarantees FontRegular");
            let viewport_area = self.graphics.preferred_dialog_rect(None);
            clonk_frontend::runtime_help::render_runtime_help(
                self.graphics.surface_mut(),
                &fonts.text,
                viewport_area,
                &columns.left,
                &columns.right,
                Some(&frame_gamma),
                &runtime_font_images,
            );
            if ordered_native {
                self.next_pending_native_overlay();
            }
        }

        if runtime_hold_message_visible {
            let fonts = self
                .assets
                .clonk_fonts
                .clone()
                .expect("global GUI preflight guarantees FontRegular");
            let screen_height = i32::try_from(self.graphics.surface().height()).unwrap_or(i32::MAX);
            let y = screen_height / 2 - fonts.text.line_height.saturating_mul(2);
            // DrawHoldMessages uses the same default message color, centered
            // FontRegular TextOut path as a flash message, but with a fixed
            // literal and no lifetime counter.
            clonk_frontend::flash_message::render_flash_message(
                self.graphics.surface_mut(),
                &fonts.text,
                "Pause",
                y,
                Some(&frame_gamma),
                &runtime_font_images,
            );
            if ordered_native {
                self.next_pending_native_overlay();
            }
        }

        if let Some(message) = runtime_flash_message.as_ref() {
            let fonts = self
                .assets
                .clonk_fonts
                .clone()
                .expect("global GUI preflight guarantees FontRegular");
            clonk_frontend::flash_message::render_flash_message(
                self.graphics.surface_mut(),
                &fonts.text,
                &message.text,
                message.y,
                Some(&frame_gamma),
                &runtime_font_images,
            );
            if ordered_native {
                self.next_pending_native_overlay();
            }
        }

        let use_running_dialog_stack = self.mode == AppMode::Running;
        let render_network_chart_elevated = self.network_chart_renders_elevated();
        let running_stack_split = if use_running_dialog_stack {
            self.running_dialog_stack
                .iter()
                .position(|entry| entry.z_order() > 0)
                .unwrap_or(self.running_dialog_stack.len())
        } else {
            0
        };
        let running_stack_tail = if use_running_dialog_stack {
            self.running_dialog_stack[running_stack_split..].to_vec()
        } else {
            Vec::new()
        };
        let game_over_mouse_active = self.game_over_dialog_is_mouse_active();
        let game_over_active = self.game_over_dialog_is_active();
        for dialog_kind in self.runtime_default_dialog_order_snapshot() {
            if render_network_chart_elevated && dialog_kind == RuntimeDefaultDialog::NetworkChart {
                continue;
            }
            let represented_in_running_tail = match dialog_kind {
                RuntimeDefaultDialog::Scoreboard => {
                    running_stack_tail.contains(&RunningDialogStackEntry::Scoreboard)
                }
                RuntimeDefaultDialog::ClientList => {
                    running_stack_tail.contains(&RunningDialogStackEntry::RuntimeClientList)
                }
                RuntimeDefaultDialog::NetworkChart
                | RuntimeDefaultDialog::GameOver
                | RuntimeDefaultDialog::ExternalIrc => false,
            };
            if represented_in_running_tail {
                continue;
            }
            match dialog_kind {
                RuntimeDefaultDialog::Scoreboard => {
                    if let Some(font_images) = scoreboard_font_images.as_ref() {
                        self.render_scoreboard_layer(font_images, &frame_gamma, ordered_native)?;
                    }
                }
                RuntimeDefaultDialog::NetworkChart => {
                    self.render_network_chart_layer(&frame_gamma, ordered_native)?;
                }
                RuntimeDefaultDialog::ClientList => {
                    self.render_runtime_client_list_layer(&frame_gamma, ordered_native)?;
                }
                RuntimeDefaultDialog::GameOver => {
                    if let Some(dialog) = self.game_over_dialog.as_ref() {
                        let font = self.assets.font_arc();
                        let hud = self.current_hud_graphics();
                        let classic = self
                            .assets
                            .game_over_classic_resources(hud.as_ref())
                            .expect("game-over resources were preflighted before rendering");
                        dialog.render_with_gamma_active(
                            self.graphics.surface_mut(),
                            font.as_ref(),
                            Some(classic),
                            Some(&frame_gamma),
                            game_over_active,
                            game_over_mouse_active,
                        );
                        if ordered_native {
                            self.next_pending_native_overlay();
                        }
                    }
                }
                RuntimeDefaultDialog::ExternalIrc => {
                    self.render_external_irc_dialog(Some(&frame_gamma))?;
                    if ordered_native {
                        self.next_pending_native_overlay();
                    }
                }
            }
        }
        self.render_league_signup_dialog(Some(&frame_gamma))?;
        if ordered_native && self.league_signup_dialog.is_some() {
            self.next_pending_native_overlay();
        }
        if use_running_dialog_stack {
            self.render_running_dialog_stack(
                running_stack_split,
                scoreboard_font_images.as_ref(),
                &frame_gamma,
                ordered_native,
            )?;
        } else {
            self.render_message_dialogs(Some(&frame_gamma))?;
            if ordered_native && !self.message_dialogs.is_empty() {
                self.next_pending_native_overlay();
            }
        }
        let running_chat_input_open = self.running_chat_controller().is_some();
        if running_chat_input_open && !use_running_dialog_stack {
            self.render_game_option_input_dialog(Some(&frame_gamma))?;
            if ordered_native {
                self.next_pending_native_overlay();
            }
        }
        if render_network_chart_elevated {
            self.render_network_chart_layer(&frame_gamma, ordered_native)?;
        }
        if self.context_menu.is_some()
            && (!running_chat_input_open || use_running_dialog_stack || self.network_chart_elevated)
        {
            // C4GUI::Screen draws its recursively owned context chain after
            // every dialog, so it stays above viewport menus, F1 help,
            // scoreboard, evaluation and message dialogs.
            if ordered_native {
                self.render_ordered_context_menu(Some(&frame_gamma))?;
            } else if let Some(context_menu) = self.context_menu.as_ref() {
                context_menu.render_panels(self.graphics.surface_mut(), Some(&frame_gamma))?;
            }
        }
        let gui_cursor_drawn = self.draw_classic_gui_cursor(Some(&frame_gamma));
        if ordered_native && gui_cursor_drawn {
            self.next_pending_native_overlay();
        }
        if self.render_runtime_client_list_tooltip(&frame_gamma)? && ordered_native {
            self.next_pending_native_overlay();
        }
        if self.render_league_signup_tooltip(Some(&frame_gamma))? && ordered_native {
            self.next_pending_native_overlay();
        }
        if self.render_external_irc_dialog_tooltip(Some(&frame_gamma))? && ordered_native {
            self.next_pending_native_overlay();
        }
        if self.render_game_over_tooltip(Some(&frame_gamma))? && ordered_native {
            self.next_pending_native_overlay();
        }
        if running_chat_input_open && use_running_dialog_stack {
            self.render_running_chat_tooltip(Some(&frame_gamma))?;
            if ordered_native {
                self.next_pending_native_overlay();
            }
        } else if running_chat_input_open
            && self.render_game_option_input_dialog_tooltip(Some(&frame_gamma))?
            && ordered_native
        {
            self.next_pending_native_overlay();
        }
        if self.render_classic_dialog_title_tooltip(Some(&frame_gamma))? && ordered_native {
            self.next_pending_native_overlay();
        }
        self.render_message_dialog_tooltip(Some(&frame_gamma))?;
        if self.render_context_menu_tooltip(Some(&frame_gamma)) && ordered_native {
            self.next_pending_native_overlay();
        }

        if !ordered_native {
            if !defer_monitor_gamma {
                if let Some(gamma) = monitor_gamma.as_ref() {
                    self.graphics.apply_monitor_gamma(gamma);
                }
            }
            let surface = self.graphics.surface();
            let pixels = surface.pixels();
            if pixels.len() == frame.len() {
                frame.copy_from_slice(pixels);
            } else {
                copy_surface(pixels, surface.width(), surface.height(), frame);
            }
        }
        if runtime_flash_message.is_some() {
            self.finish_runtime_flash_draw();
        }
        Ok(())
    }

    /// Resolve the target branch of `C4GameMessage::Draw` for one exact
    /// physical viewport. The returned point is the post-parallax,
    /// post-shape-offset C4Facet coordinate used by both FoW and drawing.
    pub(crate) fn target_message_position_for_viewport(
        &self,
        message: &clonk_engine::MessageSnapshot,
        viewport: ActiveViewportProjection,
    ) -> Option<Vector2> {
        if !matches!(
            message.kind,
            MessageKind::Target | MessageKind::TargetPlayer
        ) {
            return None;
        }
        if message.kind == MessageKind::TargetPlayer
            && message.player.unwrap_or(OWNER_NONE) != viewport.owner
        {
            return None;
        }
        let target_id = message.target?;
        let target = self.snapshot.object(target_id)?;
        let shape_height = self
            .engine
            .definition_shape_rect(&target.definition_id)
            .map(|shape| shape.height)
            .unwrap_or(0);
        let position = c4_message_target_position(target, message.offset, shape_height, viewport);
        if !viewport.contains_logical_point(position) {
            return None;
        }
        if message.kind == MessageKind::Target
            && !self
                .snapshot
                .object_visible_for_player(target_id, viewport.owner, false)
        {
            return None;
        }
        let fog_enabled = self
            .snapshot
            .players
            .iter()
            .find(|player| player.id == viewport.owner)
            .is_some_and(|player| player.fog_of_war);
        if fog_enabled
            && target.category & C4D_IGNORE_FOW == 0
            && !fow_point_is_visible(&self.snapshot, viewport.owner, position)
        {
            return None;
        }
        Some(position)
    }

    pub(crate) fn hud_message_drawability(
        &self,
        message: &clonk_engine::MessageSnapshot,
        viewports: &[ActiveViewportProjection],
    ) -> HudMessageDrawability {
        let drawable = match message.kind {
            MessageKind::Global => !viewports.is_empty(),
            MessageKind::GlobalPlayer => {
                let player = message.player.unwrap_or(OWNER_NONE);
                viewports.iter().any(|viewport| viewport.owner == player)
            }
            MessageKind::Target | MessageKind::TargetPlayer => viewports.iter().any(|viewport| {
                self.target_message_position_for_viewport(message, *viewport)
                    .is_some()
            }),
        };
        if drawable {
            HudMessageDrawability::Drawable
        } else {
            HudMessageDrawability::NotDrawable
        }
    }

    fn draw_classic_game_messages(
        &mut self,
        gamma: &clonk_graphics::GammaRamp,
        defer_native_messages: bool,
    ) -> Result<()> {
        if self.snapshot.hud.messages.is_empty() {
            return Ok(());
        }
        if defer_native_messages {
            // At scale >1 the complete message is committed after the
            // presenter's filtered base. Deferring frame and portrait along
            // with text preserves C++ insertion order and avoids a 150->64
            // logical portrait resample before the physical 64*scale draw.
            return Ok(());
        }
        let fonts = self
            .assets
            .clonk_fonts
            .clone()
            .context("classic C4GameMessage requires FontRegular")?;
        let mut messages = self.snapshot.hud.messages.clone();
        for message in &mut messages {
            for line in &mut message.lines {
                *line = c4_presentation_text(line);
            }
        }
        let viewports = self.graphics.active_viewport_projections();
        let ordered_native = self.graphics.surface().is_clonk_text_capture_active();
        for viewport in viewports {
            for message in &messages {
                let target_position = match message.kind {
                    MessageKind::Global => None,
                    MessageKind::GlobalPlayer
                        if message.player.unwrap_or(OWNER_NONE) == viewport.owner =>
                    {
                        None
                    }
                    MessageKind::GlobalPlayer => continue,
                    MessageKind::Target | MessageKind::TargetPlayer => {
                        let Some(position) =
                            self.target_message_position_for_viewport(message, viewport)
                        else {
                            continue;
                        };
                        Some(position)
                    }
                };
                if !game_message::is_supported(message) {
                    continue;
                }
                let font_images = resolve_message_font_images(
                    &self.engine,
                    message,
                    self.script_text_spec_resources(),
                );
                if let Some(position) = target_position {
                    let anchor = viewport.logical_to_output(position);
                    game_message::draw_target_message(
                        self.graphics.surface_mut(),
                        &fonts.text,
                        viewport.rect,
                        (anchor.0.round() as i32, anchor.1.round() as i32),
                        message,
                        &font_images,
                        Some(gamma),
                    )
                    .map_err(|detail| anyhow!("classic C4GameMessage render failed: {detail}"))?;
                } else {
                    let portrait = message
                        .portrait
                        .as_deref()
                        .and_then(|spec| resolve_message_portrait(&self.engine, spec));
                    let decoration_image =
                        message.frame_decoration.as_ref().and_then(|decoration| {
                            self.engine
                                .definition_sprite_image(&decoration.source_definition, None)
                                .map(default_owner_definition_sprite)
                        });
                    game_message::draw_global_message(
                        self.graphics.surface_mut(),
                        &fonts.text,
                        viewport.rect,
                        message,
                        message.frame_decoration.as_ref(),
                        decoration_image.as_ref(),
                        portrait.as_ref(),
                        &font_images,
                        Some(gamma),
                    )
                    .map_err(|detail| anyhow!("classic C4GameMessage render failed: {detail}"))?;
                }
                if ordered_native {
                    self.next_pending_native_overlay_with_clip(viewport.rect);
                }
            }
        }
        Ok(())
    }

    fn draw_messages(&mut self, gamma: &clonk_graphics::GammaRamp) {
        if self.snapshot.hud.messages.is_empty() {
            return;
        }

        let surface_width = self.graphics.surface().width() as f32;
        let surface_height = self.graphics.surface().height() as f32;

        struct PreparedMessage {
            anchor: (f32, f32),
            global_portrait_placement: Option<GlobalPortraitPlacement>,
            lines: Vec<MessageLineLayout>,
            has_frame: bool,
            portrait_requested: bool,
            portrait: Option<ImageData>,
            alignment: HorizontalAlignment,
            vertical_align: VerticalAlignment,
            base_color: Color,
            max_line_width: f32,
        }

        let font = self.assets.font_arc();
        let font_ref = font.as_ref();
        let line_height = 20.0;
        let frame_background = Color::new(12, 20, 36, 192);

        const FONT_SIZE: f32 = 18.0;
        const FRAME_PADDING: f32 = 8.0;
        // C4GameMessage.cpp:95-96.
        const PORTRAIT_SIZE: f32 = 64.0;
        const PORTRAIT_GAP: f32 = 10.0;

        let mut prepared: Vec<PreparedMessage> = Vec::new();
        let viewports = self.graphics.active_viewport_projections();

        for message in &self.snapshot.hud.messages {
            if self.hud_message_drawability(message, &viewports) != HudMessageDrawability::Drawable
            {
                continue;
            }

            let base_color = Color::new(
                ((message.color >> 16) & 0xff) as u8,
                ((message.color >> 8) & 0xff) as u8,
                (message.color & 0xff) as u8,
                ((message.color >> 24) & 0xff) as u8,
            );

            let mut anchor_x = if (message.flags & FLAG_X_REL) != 0 {
                surface_width * (message.offset.x as f32 / 100.0)
            } else if message.offset.x >= 0 {
                message.offset.x as f32
            } else {
                surface_width * 0.5
            };
            let mut anchor_y = if (message.flags & FLAG_Y_REL) != 0 {
                surface_height * (message.offset.y as f32 / 100.0)
            } else if message.offset.y >= 0 {
                message.offset.y as f32
            } else {
                surface_height * 0.66
            };

            if (message.flags & FLAG_HCENTER) != 0 {
                anchor_x = surface_width * 0.5;
            } else if (message.flags & FLAG_LEFT) != 0 {
                anchor_x = 32.0;
            } else if (message.flags & FLAG_RIGHT) != 0 {
                anchor_x = surface_width - 196.0;
            }

            if (message.flags & FLAG_VCENTER) != 0 {
                anchor_y = surface_height * 0.5;
            } else if (message.flags & FLAG_TOP) != 0 {
                anchor_y = 48.0;
            } else if (message.flags & FLAG_BOTTOM) != 0 {
                anchor_y = surface_height - 160.0;
            }

            let (mut anchor_x, mut anchor_y) = match message.kind {
                MessageKind::Target | MessageKind::TargetPlayer => {
                    let target_id = match message.target {
                        Some(id) => id,
                        None => continue,
                    };
                    let Some(target) = self.snapshot.object(target_id) else {
                        continue;
                    };
                    let owner = message.player.unwrap_or(self.local_owner);
                    if message.kind == MessageKind::Target
                        && !self
                            .snapshot
                            .object_visible_for_player(target_id, owner, false)
                    {
                        continue;
                    }
                    let base_position = Vector2::new(
                        target.position.x + message.offset.x,
                        target.position.y + message.offset.y,
                    );
                    match self.graphics.world_to_screen(owner, base_position) {
                        Some(coords) => coords,
                        None => continue,
                    }
                }
                MessageKind::Global | MessageKind::GlobalPlayer => (anchor_x, anchor_y),
            };

            let has_decoration = message
                .decoration
                .as_ref()
                .map(|decor| !decor.trim().is_empty())
                .unwrap_or(false);
            let portrait_requested = message
                .portrait
                .as_deref()
                .is_some_and(|spec| !spec.is_empty());
            let portrait = message
                .portrait
                .as_deref()
                .and_then(|spec| resolve_message_portrait(&self.engine, spec));
            let has_frame = portrait_requested || has_decoration;

            // TutorialMessage is a player-global portrait message. C++ draws
            // it through that player's viewport facet, whose output rectangle
            // starts at DrawX/DrawY (src/C4Viewport.cpp:852-854,1146-1149),
            // before applying the global portrait positioning rules
            // (src/C4GameMessage.cpp:103-168).
            let global_portrait_placement =
                if message.kind == MessageKind::GlobalPlayer && portrait_requested {
                    let owner = message.player.unwrap_or(self.local_owner);
                    let Some(viewport) = self.graphics.viewport_rect(owner) else {
                        continue;
                    };
                    Some(GlobalPortraitPlacement {
                        viewport,
                        offset: message.offset,
                        flags: message.flags,
                    })
                } else {
                    None
                };
            let global_portrait_geometry = global_portrait_placement.map(|placement| {
                global_message_viewport_geometry(
                    placement.viewport,
                    placement.offset,
                    message.width.unwrap_or(0),
                    placement.flags,
                )
            });
            if let Some(geometry) = global_portrait_geometry {
                anchor_x = geometry.x as f32;
                anchor_y = geometry.y as f32;
            }

            let alignment = message_horizontal_alignment(message.flags, has_frame);
            let vertical_align = if (message.flags & FLAG_TOP) != 0 {
                VerticalAlignment::Top
            } else if (message.flags & FLAG_BOTTOM) != 0 {
                VerticalAlignment::Bottom
            } else if (message.flags & FLAG_VCENTER) != 0 {
                VerticalAlignment::Center
            } else {
                VerticalAlignment::Baseline
            };

            let width_hint = if let Some(geometry) = global_portrait_geometry {
                message.width.map(|_| geometry.width as f32)
            } else {
                let mut width_hint = message.width.map(|raw| raw as f32);
                if let Some(value) = width_hint.as_mut() {
                    if (message.flags & FLAG_WIDTH_REL) != 0 {
                        *value = surface_width * (*value / 100.0);
                    }
                }
                width_hint
            };
            let available_width = global_portrait_placement
                .map(|placement| placement.viewport.width as f32)
                .unwrap_or(surface_width);

            let wrap_width = if (message.flags & FLAG_NO_BREAK) != 0 {
                None
            } else {
                let fallback = || {
                    let max_width = (available_width - 10.0).min(500.0).max(50.0);
                    if has_frame {
                        if portrait_requested {
                            Some((available_width * 0.5).clamp(50.0, max_width))
                        } else {
                            Some((available_width - 50.0).clamp(50.0, max_width))
                        }
                    } else {
                        Some((available_width - 50.0).clamp(50.0, max_width))
                    }
                };
                width_hint.or_else(fallback).filter(|value| *value > 0.0)
            };

            let mut units = Vec::new();
            for (idx, line) in message.lines.iter().enumerate() {
                let line = c4_presentation_text(line);
                let spans = parse_message_spans(&line, base_color);
                for span in spans {
                    for segment in split_span_into_segments(span, font_ref, FONT_SIZE) {
                        if !segment.text.is_empty() {
                            units.push(MessageWordUnit::Segment(segment));
                        }
                    }
                }
                if idx + 1 < message.lines.len() {
                    units.push(MessageWordUnit::ForcedBreak);
                }
            }

            let mut lines = wrap_word_units(units, wrap_width, font_ref, FONT_SIZE);
            if lines.is_empty() {
                lines.push(MessageLineLayout {
                    segments: Vec::new(),
                    width: 0.0,
                });
            }
            let max_line_width = lines.iter().fold(0.0f32, |acc, line| acc.max(line.width));

            prepared.push(PreparedMessage {
                anchor: (anchor_x, anchor_y),
                global_portrait_placement,
                lines,
                has_frame,
                portrait_requested,
                portrait,
                alignment,
                vertical_align,
                base_color,
                max_line_width,
            });
        }

        if prepared.is_empty() {
            return;
        }

        {
            let surface = self.graphics.surface_mut();
            for message in prepared {
                if message.lines.is_empty() {
                    continue;
                }

                let text_height = (message.lines.len() as f32) * line_height;
                let portrait_space = if message.portrait_requested {
                    PORTRAIT_SIZE + PORTRAIT_GAP
                } else {
                    0.0
                };
                let text_block_width = message.max_line_width;

                if message.has_frame {
                    let frame_width =
                        (text_block_width + portrait_space + FRAME_PADDING * 2.0).max(1.0);
                    let content_height = if message.portrait_requested {
                        text_height.max(PORTRAIT_SIZE)
                    } else {
                        text_height
                    };
                    let frame_height = (content_height + FRAME_PADDING * 2.0).max(1.0);
                    let frame_size = (frame_width.ceil() as u32, frame_height.ceil() as u32);
                    let (frame_x, frame_y, rect) =
                        if let Some(placement) = message.global_portrait_placement {
                            let rect = global_portrait_frame_rect(
                                placement.viewport,
                                placement.offset,
                                placement.flags,
                                frame_size,
                            );
                            (rect.x as f32, rect.y as f32, rect)
                        } else {
                            let frame_x = match message.alignment {
                                HorizontalAlignment::Left => message.anchor.0,
                                HorizontalAlignment::Center => message.anchor.0 - frame_width * 0.5,
                                HorizontalAlignment::Right => message.anchor.0 - frame_width,
                            };
                            let frame_y = match message.vertical_align {
                                VerticalAlignment::Top => message.anchor.1,
                                VerticalAlignment::Center => message.anchor.1 - frame_height * 0.5,
                                VerticalAlignment::Bottom => message.anchor.1 - frame_height,
                                VerticalAlignment::Baseline => message.anchor.1,
                            };
                            (
                                frame_x,
                                frame_y,
                                Rect::new(
                                    frame_x.floor() as i32,
                                    frame_y.floor() as i32,
                                    frame_size.0,
                                    frame_size.1,
                                ),
                            )
                        };

                    clonk_frontend::draw_color_rect(surface, rect, frame_background, Some(gamma));
                    let border = Color::new(
                        message.base_color.r.saturating_add(24),
                        message.base_color.g.saturating_add(24),
                        message.base_color.b.saturating_add(24),
                        255,
                    );
                    Self::draw_border_with_gamma(surface, rect, border, gamma);

                    if message.portrait_requested {
                        let portrait_rect = Rect::new(
                            rect.x + FRAME_PADDING as i32,
                            rect.y + FRAME_PADDING as i32,
                            PORTRAIT_SIZE as u32,
                            PORTRAIT_SIZE as u32,
                        );
                        if let Some(portrait) = message.portrait.as_ref() {
                            clonk_frontend::draw_image_with_gamma(
                                surface,
                                &GuiRect::new(
                                    portrait_rect.x as f32,
                                    portrait_rect.y as f32,
                                    portrait_rect.width as f32,
                                    portrait_rect.height as f32,
                                ),
                                portrait,
                                Some(gamma),
                            );
                        }
                    }

                    let text_base_x = frame_x + FRAME_PADDING + portrait_space;
                    let mut text_y = frame_y + FRAME_PADDING;

                    for line in &message.lines {
                        let line_offset = match message.alignment {
                            HorizontalAlignment::Left => 0.0,
                            HorizontalAlignment::Center => (text_block_width - line.width) * 0.5,
                            HorizontalAlignment::Right => text_block_width - line.width,
                        };
                        let mut cursor_x = text_base_x + line_offset;
                        for segment in &line.segments {
                            clonk_frontend::draw_text_with_gamma(
                                font_ref,
                                surface,
                                cursor_x,
                                text_y,
                                &segment.text,
                                FONT_SIZE,
                                segment.color,
                                Some(gamma),
                            );
                            cursor_x += segment.width;
                        }
                        text_y += line_height;
                    }
                } else {
                    let text_base_x = match message.alignment {
                        HorizontalAlignment::Left => message.anchor.0,
                        HorizontalAlignment::Center => message.anchor.0 - text_block_width * 0.5,
                        HorizontalAlignment::Right => message.anchor.0 - text_block_width,
                    };
                    let mut text_y = match message.vertical_align {
                        VerticalAlignment::Top => message.anchor.1,
                        VerticalAlignment::Center => message.anchor.1 - text_height * 0.5,
                        VerticalAlignment::Bottom => message.anchor.1 - text_height,
                        VerticalAlignment::Baseline => message.anchor.1,
                    };

                    for line in &message.lines {
                        let line_offset = match message.alignment {
                            HorizontalAlignment::Left => 0.0,
                            HorizontalAlignment::Center => (text_block_width - line.width) * 0.5,
                            HorizontalAlignment::Right => text_block_width - line.width,
                        };
                        let mut cursor_x = text_base_x + line_offset;
                        for segment in &line.segments {
                            clonk_frontend::draw_text_with_gamma(
                                font_ref,
                                surface,
                                cursor_x,
                                text_y,
                                &segment.text,
                                FONT_SIZE,
                                segment.color,
                                Some(gamma),
                            );
                            cursor_x += segment.width;
                        }
                        text_y += line_height;
                    }
                }
            }
        }
    }

    fn draw_border(surface: &mut Surface, rect: Rect, color: Color) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let top = Rect::new(rect.x, rect.y, rect.width, 1);
        let bottom = Rect::new(rect.x, rect.y + rect.height as i32 - 1, rect.width, 1);
        let left = Rect::new(rect.x, rect.y, 1, rect.height);
        let right = Rect::new(rect.x + rect.width as i32 - 1, rect.y, 1, rect.height);
        Self::fill_rect(surface, top, color);
        Self::fill_rect(surface, bottom, color);
        Self::fill_rect(surface, left, color);
        Self::fill_rect(surface, right, color);
    }

    fn draw_border_with_gamma(
        surface: &mut Surface,
        rect: Rect,
        color: Color,
        gamma: &clonk_graphics::GammaRamp,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        for edge in [
            Rect::new(rect.x, rect.y, rect.width, 1),
            Rect::new(rect.x, rect.y + rect.height as i32 - 1, rect.width, 1),
            Rect::new(rect.x, rect.y, 1, rect.height),
            Rect::new(rect.x + rect.width as i32 - 1, rect.y, 1, rect.height),
        ] {
            clonk_frontend::draw_color_rect(surface, edge, color, Some(gamma));
        }
    }

    pub(crate) fn loaded_game_global_gui_resolution(
        &self,
        frontend: &FrontendScenario,
        definition_load: Option<&ScenarioDefinitionLoad>,
    ) -> Result<ClassicGuiSheetResolution> {
        let paths = self
            .app_paths
            .as_ref()
            .context("application paths are unavailable for saved-game GUI resolution")?;
        let (head, catalog, graphics_registrations) =
            loaded_game_gui_registrations(frontend, definition_load, paths)?;
        let resolution = resolve_active_network_gui_resolution(
            paths,
            Some(head.font()),
            &catalog,
            &graphics_registrations,
        )?;
        Ok(ClassicGuiSheetResolution {
            overrides: resolution.overrides,
            failures: resolution.failures,
        })
    }

    /// Rebuilds the running round's registered group set from its retained
    /// activation inputs and appends the completed network definition roots
    /// that are not part of it yet. Returns `None` when every completed
    /// root is already registered (the idRegisteredMainGroupSetFiles skip:
    /// a re-run of Init would reload nothing).
    pub(crate) fn resolve_network_overloaded_gui_resolution(
        &self,
        frontend: &FrontendScenario,
    ) -> Result<Option<ActiveNetworkGuiResolution>> {
        let paths = self
            .app_paths
            .as_ref()
            .context("application paths are unavailable for the network GUI overloading")?;
        let definition_load = self.active_definition_load.as_ref();
        let (head, catalog, mut graphics_registrations) =
            if matches!(self.network_mode, Some(NetworkMode::Client(_))) {
                // A client's Extra.Init ran before the join with the pre-join
                // DefinitionFilenames (C4Game.cpp:368-381), so its set keeps
                // no per-definition Extra children; the synchronized
                // definition roots were retained as the activation's exact
                // ordered module paths.
                let path = frontend
                    .path
                    .as_deref()
                    .context("active scenario has no path for GUI resolution")?;
                let scenario_group = open_group_path_for_folder_map(path).with_context(|| {
                    format!("failed to open active scenario at {}", path.display())
                })?;
                let head = load_classic_scenario_loader_head(&scenario_group, paths)?;
                let modules = match definition_load {
                    Some(
                        ScenarioDefinitionLoad::Fixed { modules, .. }
                        | ScenarioDefinitionLoad::Seed { modules, .. },
                    ) => modules.as_slice(),
                    None => &[],
                };
                let resolver = InstallDefinitionResolver::new(Some(Arc::new(paths.clone())));
                let mut definition_groups = Vec::new();
                for module in modules {
                    definition_groups.extend(
                        resolver
                            .resolve_definition_groups(&scenario_group, module)
                            .map_err(anyhow::Error::from)?,
                    );
                }
                let (catalog, graphics_registrations) = client_network_gui_registrations(
                    frontend,
                    &scenario_group,
                    &head,
                    &definition_groups,
                    paths,
                )?;
                (head, catalog, graphics_registrations)
            } else {
                loaded_game_gui_registrations(frontend, definition_load, paths)?
            };
        let mut registered_roots = graphics_registrations
            .iter()
            .map(|registration| registration.group.root().to_path_buf())
            .collect::<HashSet<_>>();
        let mut next_registration_order = graphics_registrations
            .iter()
            .map(|registration| registration.registration_order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut appended = false;
        for (resource_id, state) in &self.admission_resources.resources {
            let AdmissionResourceState::Complete { path, .. } = state else {
                continue;
            };
            let arrived_definitions = self
                .admission_resources
                .resource_cores
                .get(resource_id)
                .is_some_and(|core| {
                    core.resource_type == clonk_network::HostResourceType::Definitions as u8
                });
            if !arrived_definitions || registered_roots.contains(path.as_path()) {
                continue;
            }
            let group = match Group::open(path) {
                Ok(group) => group,
                Err(error) => {
                    // An unopenable arrival is not a Graphics-bearing group;
                    // C++ would never register it mid-round either.
                    tracing::warn!(
                        resource_id,
                        path = %path.display(),
                        %error,
                        "ignoring an unopenable mid-round definitions resource"
                    );
                    continue;
                }
            };
            if registered_roots.insert(group.root().to_path_buf()) {
                graphics_registrations.push(LoaderGroupRegistration {
                    priority: 1,
                    registration_order: next_registration_order,
                    group,
                });
                next_registration_order = next_registration_order.saturating_add(1);
                appended = true;
            }
        }
        if !appended {
            return Ok(None);
        }
        resolve_active_network_gui_resolution(
            paths,
            Some(head.font()),
            &catalog,
            &graphics_registrations,
        )
        .map(Some)
    }

    pub(crate) fn loaded_game_graphics_resources(
        &self,
        frontend: &FrontendScenario,
        definition_load: Option<&ScenarioDefinitionLoad>,
    ) -> Result<GameGraphicsResources> {
        load_game_graphics_resources(
            self.app_paths.as_ref(),
            self.startup_game_graphics_resources(),
            self.assets.liquid_animation_enabled(),
            frontend,
            definition_load,
        )
    }
}
