//! `impl GameApp` — rendering, viewports & HUD methods.
//!
//! This remains an `impl GameApp` module so it can share private application
//! state. Extracting an independently owned rendering state is tracked by
//! clonk-org/clonk-rs#1232.

use super::*;

/// The detached viewport bars' thickness.
///
/// One constant because the drawing and the hit test have to agree: a bar the
/// pointer misses by two pixels is worse than no bar. Wide enough to hit,
/// narrow enough not to eat the view a 400x250 window starts with.
const CONSOLE_SCROLL_BAR_THICKNESS: i32 = 6;

impl GameApp {
    /// Compose the port's opt-in diagnostics overlay (`Graphics.ShowStats`,
    /// plus a default-unbound `StatsToggle` key — both off by default).
    ///
    /// C++ has no counterpart, so this is a deliberate divergence. The upper
    /// board draws exactly one frame rate under `Config.General.FPS`
    /// (C4UpperBoard.cpp:81-86), and it is `C4Game::FPS`: `cFPS++` counts
    /// executed *game* frames (C4Game.cpp:1915-1916) and `C4Game::Sec1Timer`
    /// samples it (C4Game.cpp:1758-1762). C++ presents once per tick, so there
    /// that single number is also the render rate; this port moves the present
    /// rate independently (smooth presentation, the presentation-detail
    /// governor, automatic graphics skips, the refuse-to-draw-while-inactive
    /// gate), so a presentation stall cannot reach the screen at all. Measured
    /// on `content/mods/Super_Mega_Ultra_Extrem_Wettlauf.c4s`: 35.7 simulation
    /// FPS held steady across a 9.03 -> 0.93 collapse in presentation
    /// submission rate, and establishing which half was slow cost a two-hour
    /// investigation (causes filed as clonk-org/clonk-rs#158 and #159).
    /// Composing nothing is the whole gate — with the key off no draw site
    /// exists and the frame is byte-identical to the one shipped today.
    ///
    /// Every value read here is presentation-only. Nothing on this path
    /// touches `C4Fixed`, `C4Random`, control ordering or any other
    /// determinism-critical state, so two clients with the key set differently
    /// stay in lockstep and cross-play against a stock LegacyClonk client is
    /// unaffected.
    pub(crate) fn update_diagnostics_overlay(&mut self) {
        if !self.display_flags.show_stats {
            self.graphics.set_diagnostics_overlay_text(None);
            return;
        }
        let format_ms = |duration: Duration| format!("{:.1} ms", duration.as_secs_f64() * 1_000.0);
        let stats = &self.presentation_stats;
        let mut lines = vec![
            format!(
                "Sim {} FPS, Render {} FPS",
                self.frames_per_second,
                stats.presentations_per_second(),
            ),
            format!(
                "Draw {} (p95 {}), skips {}",
                format_ms(stats.last_graphics()),
                format_ms(stats.graphics_p95()),
                stats.automatic_graphics_skips_per_second(),
            ),
        ];
        if let Some(clock) = self.network_control_clock {
            // `runtime_connections` needs a live worker, so a session without
            // one reports no route rather than an invented zero.
            let route = self
                .network
                .as_ref()
                .and_then(|network| network.runtime_connections().ok())
                .unwrap_or_default()
                .into_iter()
                .map(|connection| (connection.ping_ms, connection.packet_loss))
                .reduce(|worst, route| (worst.0.max(route.0), worst.1.max(route.1)));
            if let Some((ping_ms, packet_loss)) = route {
                lines.push(format!("Ping {ping_ms} ms, loss {packet_loss}"));
            }
            let behind = self.network_control_pacing().behind;
            let lateness = clock
                .control_lateness_ms()
                .map_or_else(|| "-".to_string(), |lateness| format!("{lateness} ms"));
            lines.push(format!(
                "Behind {behind}, PreSend {}, late {lateness}, budget {}",
                clock.control_presend(),
                format_ms(clock.control_latency_budget()),
            ));
        }
        self.graphics
            .set_diagnostics_overlay_text(Some(lines.join("|")));
    }

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
        self.request_exit("the window was closed");
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
    ///
    /// It is fullscreen-only: the function's first statement is
    /// `if (!Application.isFullScreen) return;` (C4GraphicsSystem.cpp:335-336)
    /// and the sort follows at `:339`, so in console mode every viewport keeps
    /// the position it was created at.
    fn sort_physical_viewports_by_player_control(&mut self) {
        if self.console_mode {
            return;
        }
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

    /// `C4GraphicsSystem::CloseViewport(C4Viewport *cvp)`
    /// (`C4GraphicsSystem.cpp:205-224`) — the path a viewport window's own
    /// close button takes (`C4ViewportWindow::Close`, `C4Viewport.cpp:775-778`).
    ///
    /// Two things separate it from its player-keyed sibling (`:314-331`): it
    /// erases **exactly one** viewport, found by pointer, so closing one window
    /// never takes a sibling viewport of the same player with it; and it has no
    /// `fSilent` parameter at all, so it always plays.
    pub(crate) fn close_physical_viewport_identity(&mut self, identity: u64) -> bool {
        let primary_removed = self
            .physical_viewports
            .first()
            .is_some_and(|viewport| viewport.physical_identity == identity);
        let previous_count = self.physical_viewports.len();
        self.physical_viewports
            .retain(|viewport| viewport.physical_identity != identity);
        if self.physical_viewports.len() == previous_count {
            return false;
        }
        self.graphics.drop_physical_camera(identity);
        if primary_removed {
            self.film_view_player = None;
        }
        self.sort_physical_viewports_by_player_control();
        self.update_film_viewport_availability();
        self.play_viewport_feedback_sound_for_game_state(self.mode == AppMode::Running);
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
                // Outside film mode ViewportCheck tells the user how to reach
                // the observer menu the ownerless viewport just handed them
                // (C4FullScreen.cpp:518-526). The key name follows the live
                // registration, which C4Game::InitKeyboard put on K_SPACE
                // (C4Game.cpp:3428).
                if !self.engine.is_replay_film() {
                    let key = format!(
                        "<c ffff00><{}></c>",
                        runtime_help_key_name(
                            self.runtime_key_config().ok(),
                            "FullscreenMenuOpen",
                            0,
                        )
                    );
                    self.runtime_flash_message = self.prepare_runtime_resource_flash(|resources| {
                        format_resource_string(resources.observer_menu.clone(), &[&key])
                    });
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
        self.graphics.set_pxs_graphics(self.display_flags.pxs_gfx);
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
                // C4GUI::ScrollBar repeats held arrows from DrawElement, so
                // advance once per presentation rather than per update.
                let _ = self
                    .startup_player_properties_dialog
                    .as_mut()
                    .is_some_and(|pending| pending.controller.tick_portrait_selector_scrollbar());
                self.preflight_startup_presentation()?;
                self.preflight_visible_gui_overlay_resources()?;
                if self.startup_view == StartupView::NetworkLobby
                    && self.classic_host_lobby.is_none()
                {
                    self.close_stale_classic_lobby_team_combo();
                }
                if self.startup_view == StartupView::NetworkGame
                    && !self.startup_network_transition_active()
                {
                    // C4GUI::ScrollBar repeats held arrows from DrawElement,
                    // so advance once per presentation rather than per update.
                    let _ = self
                        .startup_network_dialog
                        .as_mut()
                        .is_some_and(|dialog| dialog.tick_scrollbar());
                }
                if self.startup_view == StartupView::PlayerSelection
                    && self
                        .startup_player_dialog
                        .as_mut()
                        .is_some_and(|dialog| dialog.tick_scrollbar())
                {
                    // Book-scrollbar arrows repeat once per presentation.
                    self.plrsel_last_click = None;
                }
                if self.startup_view == StartupView::About {
                    // About TextWindow arrows repeat from ScrollBar::DrawElement.
                    let _ = self
                        .startup_about_dialog
                        .as_mut()
                        .is_some_and(|dialog| dialog.tick_scrollbar());
                }
                if self.startup_view == StartupView::Options {
                    let actions = self
                        .startup_options_dialog
                        .as_mut()
                        .map(|dialog| dialog.advance_frame())
                        .unwrap_or_default();
                    if !actions.is_empty() {
                        self.process_options_dialog_actions(actions)?;
                    }
                }
                if self.startup_view == StartupView::NetworkLobby
                    && self.classic_host_lobby.is_some()
                {
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
                    if self.chat.external_dialog_visible {
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
                let definition_selector_open = self.definition_selector.is_some();
                let game_option_input_open = self.game_option_input_dialog.is_some();
                let league_signup_open = self.league_signup_dialog.is_some();
                // A fading C4GUI::Dialog is inactive even when it retains its
                // focused control. Reuse the renderer's inactive-focus path.
                let context_menu_open = self.context_menu.is_some()
                    || self.startup_player_properties_dialog.is_some()
                    || league_signup_open
                    || self.chat.external_dialog_visible
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
                // Read before the surface is borrowed mutably below.
                let point_filtering = self.graphics.point_filtering();
                let application_scale = self.graphics.presentation_scale();
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
                    startup_assets.plrprop_assets(point_filtering, application_scale),
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
                if self.chat.external_dialog_visible {
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
                        startup_assets.plrprop_assets(point_filtering, application_scale),
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
                        || self.chat.external_dialog_visible
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
            AppMode::Running if self.terminal_loader_frame_pending => self
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
            self.loader_screen
                .as_ref()
                .ok_or_else(|| self.loader_boundary("no selected classic loader is installed"))?
                .render_chrome(self.graphics.surface_mut(), config, Some(gamma));

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
        if defer_native_text {
            loader.render_chrome(&mut surface, config, Some(gamma));
        } else {
            loader
                .render_with_config(&mut surface, config, Some(gamma))
                .map_err(|error| self.loader_boundary(error.to_string()))?;
        }
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
        if self.loader_presentation_active() {
            return self.startup_active_gamma();
        }
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
        // The monitor resolve is a second full-screen pass; the detail
        // governor drops it before it drops anything the player controls.
        let gamma_mode = match retained_gpu_gamma_mode(renderer_config) {
            GpuGammaMode::Monitor if !self.presentation_detail.resolves_monitor_gamma() => {
                GpuGammaMode::Disabled
            }
            mode => mode,
        };
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
        let (mut scene, capture_stats) = self
            .graphics
            .finish_gpu_scene_capture_with_stats(&gamma)
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
            frame.capture_stats.merge(capture_stats);
            return Ok(frame);
        }
        Ok(RetainedGpuFrame {
            layers: vec![RetainedGpuFrameLayer {
                scene,
                presentation,
            }],
            capture_stats,
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
        let mut capture_stats = clonk_graphics::GpuSceneCaptureStats::default();
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
                    capture_stats.merge(recorder.capture_stats());
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
                capture_stats.merge(recorder.capture_stats());
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
        Ok(RetainedGpuFrame {
            layers,
            capture_stats,
        })
    }

    /// A left-button press inside a console viewport window.
    ///
    /// This is `C4EditCursor::LeftButtonDown`'s Edit arm reached from a real
    /// window: `C4Viewport`'s handler converts the window-local pointer through
    /// *that viewport's* `ViewX`/`ViewY` and scale (`C4Viewport.cpp:181`),
    /// `Move` picks the target with `Game.FindObject(..., OCF_NotContained,
    /// ..., Target)` (`C4EditCursor.cpp:150`), and the press then edits the
    /// selection (`:201-229`).
    ///
    /// Returns the new selection when it actually changed, so a caller
    /// forwards at most one notification per click and a no-op stays silent.
    pub(crate) fn console_viewport_press(
        &mut self,
        identity: u64,
        local: (i32, i32),
        scale: f32,
        control: bool,
        shift: bool,
    ) -> Option<clonk_engine::developer_selection::SelectionSnapshot> {
        use clonk_engine::developer_cursor::{edit_press, edit_target, SelectionEdit};
        use clonk_engine::developer_selection::SelectionWriter;

        // Play routes to ordinary mouse control; Edit and Draw are the two
        // editor arms (`developer_viewport::route_viewport_event`).
        if self.developer_console_edit_mode == ConsoleEditMode::Draw {
            self.console_draw_press(identity, local, scale);
            return None;
        }
        if self.developer_console_edit_mode != ConsoleEditMode::Edit {
            return None;
        }
        let projection = *self.console_viewport_projections.get(&identity)?;
        let (x, y) = projection
            .pointer_projection(scale)
            .world_position(local.0, local.1);

        // One world view per gesture: `edit_target` calls the hit test
        // repeatedly to walk a shift-click stack.
        let hit_test = clonk_engine::EditCursorHitTest::new(&self.snapshot);
        let selection = self.developer_selection.objects().to_vec();
        let target = edit_target(shift, &selection, |after| hit_test.object_at(x, y, after));

        let press = edit_press(control, target, &selection);
        self.edit_cursor_hold = press.hold;
        self.edit_cursor_last_world = Some((x, y));
        match press.selection {
            Some(SelectionEdit::Replace(object)) => self
                .developer_selection
                .replace(SelectionWriter::EditCursor, object),
            Some(SelectionEdit::Remove(object)) | Some(SelectionEdit::Add(object)) => self
                .developer_selection
                .toggle(SelectionWriter::EditCursor, object),
            Some(SelectionEdit::ClearAndDragFrame) => {
                // `DragFrame = true; X2 = X; Y2 = Y` — the band is anchored at
                // the press, in world coordinates.
                self.edit_cursor_drag_frame = Some(((x, y), (x, y)));
                self.developer_selection.clear(SelectionWriter::EditCursor)
            }
            // The bare clear is the right button's alone (`right_press`).
            Some(SelectionEdit::Clear) | None => None,
        }
    }

    /// Pointer motion inside a console viewport window.
    ///
    /// `C4EditCursor::Move`'s Edit arm (`C4EditCursor.cpp:129-152`). While a
    /// rubber band is armed the band's live corner follows the pointer
    /// (`X2 = X; Y2 = Y`); otherwise the hovered target is re-picked, which is
    /// what a later shift-click resumes from.
    pub(crate) fn console_viewport_motion(
        &mut self,
        identity: u64,
        local: (i32, i32),
        scale: f32,
        control: bool,
        shift: bool,
    ) {
        use clonk_engine::developer_cursor::edit_target;

        if self.developer_console_edit_mode == ConsoleEditMode::Draw {
            self.console_draw_motion(identity, local, scale);
            return;
        }
        if self.developer_console_edit_mode != ConsoleEditMode::Edit {
            return;
        }
        let Some(projection) = self.console_viewport_projections.get(&identity).copied() else {
            return;
        };
        let (x, y) = projection
            .pointer_projection(scale)
            .world_position(local.0, local.1);

        // `UpdateDropTarget` runs on every move, before the drag arms decide
        // anything (`C4EditCursor.cpp:653-670`).
        self.edit_cursor_drop_target = self.console_drop_target(control, (x, y));
        if let Some((_, corner)) = self.edit_cursor_drag_frame.as_mut() {
            *corner = (x, y);
            return;
        }
        // `edit_move` decides between moving the selection and re-picking the
        // hovered target; the offset is the delta from the previous message.
        let previous = self.edit_cursor_last_world.replace((x, y));
        if let clonk_engine::developer_cursor::EditMove::MoveSelection { dx, dy } =
            clonk_engine::developer_cursor::edit_move(
                self.edit_cursor_hold,
                false,
                previous.map_or(0, |(px, _)| x - px),
                previous.map_or(0, |(_, py)| y - py),
                || None,
            )
        {
            self.submit_editor_move_selection(dx, dy);
            return;
        }
        let hit_test = clonk_engine::EditCursorHitTest::new(&self.snapshot);
        let selection = self.developer_selection.objects().to_vec();
        let target = edit_target(shift, &selection, |after| hit_test.object_at(x, y, after));
        self.developer_selection.set_hover(target);
    }

    /// Releasing the left button inside a console viewport window.
    ///
    /// `C4EditCursor::LeftButtonUp`'s Edit arm runs `FrameSelection()` then
    /// `PutContents()`, both optional and in that order, and clears `Hold`,
    /// `DragFrame`, `DragLine` and `DropTarget` regardless
    /// (`C4EditCursor.cpp:287-341`).
    pub(crate) fn console_viewport_release(
        &mut self,
    ) -> Option<clonk_engine::developer_selection::SelectionSnapshot> {
        use clonk_engine::developer_cursor::{
            edit_release, frame_selection, EditRelease, FrameCandidate,
        };
        use clonk_engine::developer_selection::SelectionWriter;

        let band = self.edit_cursor_drag_frame.take();
        let drop_target = self.edit_cursor_drop_target.take();
        self.edit_cursor_hold = false;
        self.edit_cursor_last_world = None;
        // `LeftButtonUp` dispatches its finish on the *current* mode but then
        // clears `Hold`, `DragFrame` and `DragLine` unconditionally
        // (`C4EditCursor.cpp:300-304`) — and C++ has one `Hold` for both arms.
        // So the tools' gesture ends here whatever the mode is now; only the
        // Line or Rect it finished is mode-dependent. The release carries no
        // coordinates of its own: C++ reads the `X`/`Y` the window's preceding
        // motion message already stored.
        let (x, y) = self.developer_tools.cursor();
        let finished_stroke = self.developer_tools.release(x, y);
        if self.developer_console_edit_mode == ConsoleEditMode::Draw {
            if let Some(control) = finished_stroke {
                self.submit_editor_draw_tool(control);
            }
            return None;
        }
        if self.developer_console_edit_mode != ConsoleEditMode::Edit {
            return None;
        }

        let mut result = None;
        for action in edit_release(band.is_some(), drop_target) {
            match action {
                EditRelease::FrameSelection => {
                    let (anchor, corner) = band?;
                    // `Game.Objects` master order, which is the reverse of the
                    // snapshot's draw order.
                    let candidates = self
                        .snapshot
                        .render_order
                        .iter()
                        .rev()
                        .filter_map(|id| {
                            let object = self.snapshot.object(*id)?;
                            Some(FrameCandidate {
                                id: *id,
                                deleted: false,
                                contained: object.container.is_some(),
                                x: object.position.x,
                                y: object.position.y,
                            })
                        })
                        .collect::<Vec<_>>();
                    let framed = frame_selection(anchor, corner, &candidates);
                    result = self
                        .developer_selection
                        .select_frame(SelectionWriter::EditCursor, framed);
                }
                // `PutContents` — `EMMoveObject(EMMO_Enter, 0, 0, DropTarget,
                // &Selection)` (`C4EditCursor.cpp:674-677`).
                EditRelease::Enter { target } => self.submit_editor_enter(target),
            }
        }
        result
    }

    /// `C4EditCursor::RightButtonDown` (`C4EditCursor.cpp:244-274`).
    ///
    /// The selection is settled *before* the menu opens, so the enablement the
    /// menu is built with already describes what its Delete would act on.
    pub(crate) fn console_viewport_right_press(
        &mut self,
        identity: u64,
        local: (i32, i32),
        scale: f32,
        control: bool,
    ) -> Option<clonk_engine::developer_selection::SelectionSnapshot> {
        use clonk_engine::developer_cursor::{right_press, SelectionEdit};
        use clonk_engine::developer_selection::SelectionWriter;

        let (x, y) = self.console_viewport_world(identity, local, scale)?;
        let hit_test = clonk_engine::EditCursorHitTest::new(&self.snapshot);
        // `fCursorIsOnSelection` — `pLnk->Obj->At(X, Y)` over the selection
        // itself, not the topmost object under the cursor (`:251-257`).
        let cursor_on_selection = self
            .developer_selection
            .objects()
            .iter()
            .any(|object| hit_test.object_covers(*object, x, y));
        let target = hit_test.object_at(x, y, None);
        match right_press(
            self.console_cursor_mode(),
            control,
            target,
            cursor_on_selection,
        ) {
            Some(SelectionEdit::Replace(object)) => self
                .developer_selection
                .replace(SelectionWriter::EditCursor, object),
            Some(SelectionEdit::Clear) => {
                self.developer_selection.clear(SelectionWriter::EditCursor)
            }
            // The right button produces neither a toggle nor a rubber band.
            Some(SelectionEdit::Remove(_))
            | Some(SelectionEdit::Add(_))
            | Some(SelectionEdit::ClearAndDragFrame)
            | None => None,
        }
    }

    /// `C4EditCursor::RightButtonUp` -> `DoContextMenu` (`:332-340`,
    /// `:582-628`).
    ///
    /// `local` is in the viewport window's *surface* coordinates, which is
    /// where the popup is drawn — C++ pops up at the screen cursor, but this
    /// menu lives on the viewport's own frame.
    /// `C4Viewport::DropFiles` (`C4Viewport.cpp:225-240`) for one dropped
    /// path.
    ///
    /// The gate is asked once per drop and reports `IDS_CNS_NONETEDIT`, which
    /// is why it is here rather than inside the ported decision: winit
    /// delivers `DroppedFile` one path at a time, so a batch arrives as
    /// several events and each one is its own `DropFiles` call.
    pub(crate) fn drop_file_on_console_viewport(
        &mut self,
        identity: u64,
        path: &std::path::Path,
        local: (i32, i32),
    ) {
        use clonk_engine::developer_drop::{drop_file, drop_world_position, DropOutcome};

        let Some(projection) = self.console_viewport_projections.get(&identity).copied() else {
            return;
        };
        let editing = self.developer_console_editing();
        let outcome = {
            // `drop_file` asks its three questions strictly in sequence — the
            // id, whether it is already loaded, and only then the load — and
            // the last one mutates what the others read. The cell scopes that
            // alternation to this call; the borrows cannot overlap, because
            // `drop_file` never holds one across another.
            let engine = std::cell::RefCell::new(&mut self.engine);
            drop_file(
                editing,
                path,
                |path| Self::dropped_definition_id(&engine.borrow(), path),
                |id| engine.borrow().definition(id).is_some(),
                // `Defs.Load(szFilename, C4D_Load_RX, …) && C4Id2Def(c_id)`
                // (`C4Game.cpp:1650`): C++ tests both, and so does `drop_file`.
                |path| engine.borrow_mut().load_definition_from_path(path),
            )
        };
        match outcome {
            DropOutcome::Refused => {
                let message =
                    self.runtime_resource_text("IDS_CNS_NONETEDIT", "No editing while replaying.");
                self.developer_console.out(&message);
            }
            // Not a definition file: C++ says nothing at all.
            DropOutcome::Ignored => {}
            DropOutcome::NoDefinition(name) => {
                let message = self.runtime_resource_text("IDS_CNS_DROPNODEF", "%s: no definition");
                self.developer_console
                    .out(&message.replacen("%s", &name, 1));
            }
            DropOutcome::Drop(id) => {
                let (x, y) = drop_world_position(projection.target_x, projection.target_y, local);
                self.submit_editor_drop_definition(&id, x, y);
            }
        }
    }

    /// `DefFileGetID` (`C4Game.cpp:1631-1639`) — the definition id a `.c4d`
    /// declares.
    ///
    /// A definition the engine already loaded from that path answers without
    /// touching the disk, which is also the only way a *packed* pack member
    /// resolves; otherwise the group's own `DefCore.txt` is read, exactly as
    /// C++ opens the group and loads the core.
    fn dropped_definition_id(
        engine: &clonk_engine::Engine,
        path: &std::path::Path,
    ) -> Option<clonk_engine::DefinitionId> {
        if let Some(id) = engine.definition_id_for_source_path(&path.to_string_lossy()) {
            return Some(id);
        }
        let group = clonk_resources::Group::open(path).ok()?;
        clonk_resources::DefCore::load(&group)
            .ok()
            .map(|core| core.id)
    }

    /// `C4Game::DropDef` — `Control.DoInput(CID_EMDropDef, …, CDT_Decide)`
    /// (`C4Game.cpp:1667`).
    fn submit_editor_drop_definition(&mut self, id: &str, x: i32, y: i32) {
        let mut packed = *b"NONE";
        let bytes = id.as_bytes();
        if bytes.len() != packed.len() {
            tracing::warn!(%id, "a dropped definition id is not four bytes");
            return;
        }
        packed.copy_from_slice(bytes);
        if let Err(error) =
            self.submit_or_execute_editor_drop_definition(clonk_engine::EmDropDefControlData {
                id: packed,
                x,
                y,
                ..Default::default()
            })
        {
            tracing::error!(%error, "failed to submit an editor definition drop");
        }
    }

    pub(crate) fn open_console_viewport_context_menu(&mut self, identity: u64, local: (i32, i32)) {
        use clonk_engine::developer_cursor::context_menu;
        use clonk_frontend::developer_context_menu::ViewportContextMenu;

        // `Target = nullptr` — the hover is dropped before the menu opens.
        self.developer_selection.set_hover(None);
        let selection = self.developer_selection.objects();
        // `Selection.GetObject()->Contents.ObjectCount()` asks the *first*
        // selected object only (`:590`).
        let contents = selection
            .first()
            .and_then(|object| self.snapshot.object(*object))
            .map_or(0, |object| object.contents.len());
        let enablement = context_menu(
            self.console_cursor_mode(),
            self.developer_console_editing(),
            !selection.is_empty(),
            contents,
        );
        let labels = self.console_viewport_context_labels();
        self.console_viewport_context_menu = Some((
            identity,
            ViewportContextMenu::new(enablement, &labels, local),
        ));
    }

    /// The resource strings `DoContextMenu` writes into the menu
    /// (`:592-595`).
    fn console_viewport_context_labels(
        &self,
    ) -> clonk_frontend::developer_context_menu::ViewportContextLabels {
        use clonk_frontend::developer_context_menu::ViewportContextLabels;

        let mut labels = ViewportContextLabels::default();
        for (target, key, fallback) in [
            (&mut labels.delete, "IDS_MNU_DELETE", "Delete"),
            (&mut labels.duplicate, "IDS_MNU_DUPLICATE", "Duplicate"),
            (&mut labels.contents, "IDS_MNU_CONTENTS", "Grab contents"),
            (&mut labels.properties, "IDS_CNS_PROPERTIES", "Properties"),
            (&mut labels.tools, "IDS_CNS_TOOLS", "Tools"),
        ] {
            *target = self.runtime_resource_text(key, fallback);
        }
        labels
    }

    /// Track the pointer over an open popup so its rows highlight.
    pub(crate) fn console_viewport_context_menu_motion(
        &mut self,
        identity: u64,
        local: (i32, i32),
    ) -> bool {
        let Some((open, menu)) = self.console_viewport_context_menu.as_mut() else {
            return false;
        };
        if *open != identity {
            return false;
        }
        menu.handle_pointer_move(clonk_frontend::GuiPoint::new(
            local.0 as f32,
            local.1 as f32,
        ));
        true
    }

    /// Whether an open popup owns this viewport's pointer events.
    ///
    /// C++'s menu is modal — `TrackPopupMenu` blocks until an item is chosen
    /// (`C4EditCursor.cpp:597`) and the GTK menu holds a pointer grab — so
    /// *no* button message reaches the viewport while it is up. The port's
    /// popup is painted rather than owned by the window system, so the grab
    /// has to be made explicit, and it covers both buttons: without it a
    /// right-click over the popup would re-pick the selection underneath it,
    /// and the release that follows a chosen item would run
    /// `LeftButtonUp` — clearing the very `Hold` Grab contents had just set.
    pub(crate) fn console_viewport_context_menu_owns_pointer(&self, identity: u64) -> bool {
        self.console_viewport_context_menu
            .as_ref()
            .is_some_and(|(open, _)| *open == identity)
    }

    /// Consume the grab a swallowed press left behind, so exactly one release
    /// is swallowed with it.
    pub(crate) fn take_console_viewport_pointer_grab(&mut self, identity: u64) -> bool {
        self.console_viewport_context_menu_grab
            .take_if(|held| *held == identity)
            .is_some()
    }

    /// A click while the popup is up. Returns whether the menu consumed it.
    pub(crate) fn console_viewport_context_menu_click(
        &mut self,
        identity: u64,
        local: (i32, i32),
        extent: (u32, u32),
    ) -> bool {
        use clonk_frontend::developer_context_menu::ViewportContextOutcome;

        let Some((open, menu)) = self.console_viewport_context_menu.as_mut() else {
            return false;
        };
        if *open != identity {
            return false;
        }
        let outcome = menu.handle_pointer_up(
            clonk_frontend::GuiPoint::new(local.0 as f32, local.1 as f32),
            extent.0,
            extent.1,
        );
        // The release completing this click belongs to the menu too, whether
        // or not the menu is still up by the time it arrives.
        self.console_viewport_context_menu_grab = Some(identity);
        match outcome {
            ViewportContextOutcome::Activate(item) => {
                self.console_viewport_context_menu = None;
                self.activate_console_viewport_context_item(item);
            }
            // A greyed row or the separator: swallowed, and the menu stays.
            ViewportContextOutcome::Ignored => {}
            ViewportContextOutcome::Dismiss => self.console_viewport_context_menu = None,
        }
        true
    }

    /// Whether a detached viewport's console popup is open, which is what
    /// decides who owns Escape.
    pub(crate) fn console_viewport_context_menu_open(&self) -> bool {
        self.console_viewport_context_menu.is_some()
    }

    /// Close the popup without running anything — the Escape key.
    pub(crate) fn dismiss_console_viewport_context_menu(&mut self) -> bool {
        self.console_viewport_context_menu.take().is_some()
    }

    /// Close the popup only if it belongs to `identity`, so one viewport's
    /// window closing never takes a sibling's menu with it.
    pub(crate) fn dismiss_console_viewport_context_menu_for(&mut self, identity: u64) -> bool {
        self.console_viewport_context_menu
            .take_if(|(open, _)| *open == identity)
            .is_some()
    }

    /// `DoContextMenu`'s switch over the chosen item (`:602-608`).
    fn activate_console_viewport_context_item(
        &mut self,
        item: clonk_frontend::developer_context_menu::ViewportContextItem,
    ) {
        use clonk_frontend::developer_context_menu::ViewportContextItem;

        match item {
            ViewportContextItem::Delete => self.console_delete_selection(),
            ViewportContextItem::Duplicate => self.console_duplicate_selection(),
            ViewportContextItem::GrabContents => self.console_grab_contents(),
            ViewportContextItem::Properties => self.open_developer_prop_tools(),
        }
    }

    /// `C4EditCursor::OpenPropTools` (`C4EditCursor.cpp:361-374`).
    ///
    /// The page follows the cursor mode, and both pages exist from the first
    /// call: `C4DevmodeDlg::AddPage` appends without showing, so the notebook
    /// holds them whichever one is switched to (`C4DevmodeDlg.cpp:53-77`).
    pub(crate) fn open_developer_prop_tools(&mut self) {
        use crate::developer_windows::ToolboxPage;
        use crate::toolbox_window_host::prop_tools_page;

        let page = prop_tools_page(self.console_cursor_mode());
        // `C4ToolsDlg::Open`'s tail on a build with no dialog of its own:
        // `Active = true` plus the ordered refresh (`C4ToolsDlg.cpp:399-408`).
        if page == ToolboxPage::Tools {
            let _ = self.developer_tools.open();
        }
        for page in [ToolboxPage::Tools, ToolboxPage::Property] {
            let effect = self.developer_toolbox.add_page(page);
            self.developer_toolbox_effects.extend(effect);
        }
        let position = self.developer_toolbox.remembered_position();
        let effect = self.developer_toolbox.switch_page(page, position);
        self.developer_toolbox_effects.extend(effect);
    }

    /// `C4EditCursor::SetMode`'s prop-tools arm (`C4EditCursor.cpp:503-518`).
    ///
    /// A mode change clears the page the mode it *left* owns and reopens the
    /// toolbox only when one of the two was already up — switching modes never
    /// opens it from nothing, which is why the console's Draw button alone
    /// still shows no window.
    pub(crate) fn apply_developer_cursor_mode_change(
        &mut self,
        previous: clonk_engine::developer_cursor::CursorMode,
    ) {
        use clonk_engine::developer_cursor::{set_mode, PropertyToolsPage};

        let change = set_mode(
            previous,
            self.console_cursor_mode(),
            self.developer_tools.active(),
            // `C4PropertyDlg::Active`, which only a *shown* property page
            // sets. Asking `current_page` instead would call a closed toolbox
            // active forever, because hiding it keeps its pages.
            self.developer_toolbox.visible()
                && self.developer_toolbox.current_page()
                    == Some(crate::developer_windows::ToolboxPage::Property),
        );
        // `Clear()` drops `Active` and nothing else, which is why re-opening
        // restores the previous selection rather than the defaults.
        if change.clear_page == Some(PropertyToolsPage::Tools) {
            self.developer_tools.clear();
        }
        if change.reopen_prop_tools {
            self.open_developer_prop_tools();
        }
    }

    /// The toolbox window closing, from its own close button.
    ///
    /// `C4DevmodeDlg`'s `delete-event` hides rather than destroys, and the
    /// shared window's `"hide"` signal is separately connected to
    /// `C4ToolsDlg::OnWindowHide`, whose whole body is `Active = false`
    /// (`C4ToolsDlg.cpp:393,1098-1101`). Dropping that second half is what
    /// would make the next mode change resurrect a toolbox the user closed —
    /// `SetMode` reopens on `ToolsDlg.Active || PropertyDlg.Active`.
    pub(crate) fn close_developer_toolbox(&mut self, position: Option<(i32, i32)>) {
        let effect = self.developer_toolbox.close(position);
        self.developer_toolbox_effects.extend(effect);
        self.developer_tools.clear();
    }

    /// `C4Console::EditScript`/`EditTitle`/`EditInfo`
    /// (`C4Console.cpp:1328-1351`).
    ///
    /// All three open with the same refusal — `if (Game.Network.isEnabled())
    /// return;` — and `EditScript` alone relinks, **unconditionally**: that
    /// statement sits outside the `#ifdef _WIN32` that guards the dialog, so
    /// it runs even when the editor never opened. The port keeps that, which
    /// is why the relink is here rather than in the commit.
    pub(crate) fn open_developer_component_editor(
        &mut self,
        component: clonk_engine::developer_components::EditableComponent,
    ) {
        use clonk_engine::developer_components::component_editor_available;

        // `ShowDialog` is modal, so a second editor cannot open over the
        // first — and letting one would discard whatever was being typed.
        if self.developer_component_editor.is_some() {
            return;
        }
        if !component_editor_available(self.network.is_some()) {
            let message = self.runtime_resource_text(
                "IDS_CNS_NONETEDIT",
                "No editing while a network game is running.",
            );
            self.developer_console.out(&message);
            return;
        }
        match self.load_developer_component(component) {
            Some(edit) => self.developer_component_editor = Some(edit),
            None => {
                let message = self.runtime_resource_text("IDS_CNS_NOSCENARIO", "No scenario open.");
                self.developer_console.out(&message);
            }
        }
        // `Game.ScriptEngine.ReLink(&Game.Defs)` past the `#endif` (`:1342`).
        if component.relinks_scripts() {
            if let Err(error) = self.engine.relink_after_component_edit() {
                tracing::error!(%error, "the component editor's relink failed");
            }
        }
    }

    /// Read a component's bytes out of the open scenario group.
    ///
    /// C++ has these already: `C4Game` holds a live `C4ComponentHost` per
    /// component for the whole round. The port keeps none — the scenario's
    /// script reaches the engine as source and is never held as bytes, and
    /// `Info.txt` is not read at all — so the editor loads from the group it
    /// will be saved back into.
    fn load_developer_component(
        &self,
        component: clonk_engine::developer_components::EditableComponent,
    ) -> Option<crate::DeveloperComponentEdit> {
        use clonk_engine::developer_components::ComponentHost;

        let filename = developer_component_filename(component);
        // A component edited earlier this round reopens on **its** bytes.
        // C++ never has to think about this: `Game.Script` and friends are
        // live hosts held for the whole round, so a second `ShowDialog` sees
        // the first edit. Re-reading the group here would show the stale
        // on-disk text and the second commit would overwrite the first.
        if let Some(host) = self
            .developer_component_hosts
            .iter()
            .rev()
            .find(|host| host.filename() == filename)
        {
            return Some(crate::DeveloperComponentEdit {
                component,
                text: crate::developer_component_editor::ComponentEditorText::opened(host.data()),
                host: host.clone(),
            });
        }
        let scenario = self.developer_component_scenario_path()?;
        let group = clonk_resources::Group::open(&scenario).ok()?;
        // A component that does not exist yet opens empty rather than
        // refusing: that is how a scenario grows one.
        let data = group.read_file(filename).unwrap_or_default();
        Some(crate::DeveloperComponentEdit {
            component,
            text: crate::developer_component_editor::ComponentEditorText::opened(&data),
            host: ComponentHost::loaded(filename, data),
        })
    }

    /// The open scenario's group path, which is what a component is read
    /// from and written back to. Both the running round and one still loading
    /// answer, as the console's own caption does.
    fn developer_component_scenario_path(&self) -> Option<std::path::PathBuf> {
        self.active_scenario
            .as_ref()
            .and_then(|scenario| scenario.path.clone())
            .or_else(|| {
                self.loading_state
                    .as_ref()
                    .and_then(|loading| loading.scenario.path.clone())
            })
    }

    /// `C4ComponentHost`'s OK arm (`C4ComponentHost.cpp:330-334`), plus the
    /// Script editor's own reload.
    pub(crate) fn commit_developer_component_editor(&mut self) {
        use clonk_engine::developer_components::EditableComponent;

        let Some(mut edit) = self.developer_component_editor.take() else {
            return;
        };
        // `Accept` replaces the bytes and sets `Modified` **without
        // comparing**, so committing unchanged text still marks the component
        // for the save.
        edit.host.accept(edit.text.bytes());
        if edit.component == EditableComponent::Script {
            // `C4Console::EditScript` reloads the scenario script into the
            // engine and relinks; it must *not* re-run Initialize, because the
            // scenario is already running and its objects exist.
            let source = clonk_script::c4_string_from_bytes(edit.host.data());
            let name = self
                .developer_component_scenario_path()
                .map(|scenario| scenario.join("Script.c").to_string_lossy().into_owned())
                .unwrap_or_else(|| "Script.c".to_owned());
            if let Err(error) = self.engine.apply_scenario_script_edit(name, &source) {
                tracing::error!(%error, "the edited scenario script did not link");
            }
        }
        // One host per component: a second commit replaces the first rather
        // than queueing a second write of the same filename at save time.
        self.developer_component_hosts
            .retain(|host| host.filename() != edit.host.filename());
        self.developer_component_hosts.push(edit.host);
    }

    /// `C4ComponentHost`'s Cancel arm, which mutates nothing — not even the
    /// modified flag.
    pub(crate) fn cancel_developer_component_editor(&mut self) {
        if let Some(mut edit) = self.developer_component_editor.take() {
            edit.host.cancel();
        }
    }

    /// Draw the open editor, if there is one.
    pub(crate) fn render_developer_component_editor(
        &mut self,
        width: u32,
        height: u32,
    ) -> Option<clonk_graphics::Surface> {
        let mut surface = clonk_graphics::Surface::new(
            width.max(1),
            height.max(1),
            clonk_graphics::PixelFormat::Rgba8888,
        );
        let font = self.assets.font_arc();
        let edit = self.developer_component_editor.as_mut()?;
        let title = format!("{}  —  Enter commits, Escape cancels", edit.host.filename());
        edit.text.render(&mut surface, font.as_ref(), &title);
        Some(surface)
    }

    /// `C4ObjectListDlg::Open` (`C4ObjectListDlg.cpp:726-787`), reached from
    /// `C4Console::EditObjects` (`C4Console.cpp:1353-1356`).
    ///
    /// Opening is idempotent: C++ creates the window only `if (window ==
    /// nullptr)`, so a second Objects click on an open list does nothing.
    pub(crate) fn open_developer_object_list(&mut self) {
        self.developer_object_list_open = true;
    }

    /// The `"destroy"` handler, which nulls the window and the model rather
    /// than hiding them (`:592-597`).
    pub(crate) fn close_developer_object_list(&mut self) {
        self.developer_object_list_open = false;
    }

    /// The console scoreboard child window's caption and natural size.
    ///
    /// The size is the dialog's own: `C4ScoreboardDlg::Update` sizes it to the
    /// spreadsheet and `Dialog::UpdateSize` resizes the console window to
    /// match (`C4GuiDialogs.cpp:445-473`), so the window follows live
    /// `SetScoreboardData` rather than the player. C++'s console dialog style
    /// is `WS_POPUP | WS_CAPTION` with no `WS_THICKFRAME`
    /// (`C4GuiDialogs.cpp:56`) — a titled, fixed-size popup.
    ///
    /// Returns `None` when the board cannot be laid out at all, which is the
    /// same condition that keeps `CreateConsoleWindow` from being reached.
    /// The console chart window's title and size.
    ///
    /// Unlike the scoreboard's, the chart's bounds are fixed
    /// (`NETWORK_CHART_DIALOG_WIDTH`/`_HEIGHT`), so the window never has to
    /// follow a live resize — `Dialog::UpdateSize` has nothing to report.
    /// The title is the dialog's own caption, which is `IDS_NET_STATISTICS`
    /// resolved through the runtime resource table.
    pub(crate) fn console_network_chart_window_chrome(&self) -> Option<(String, u32, u32)> {
        if !self.console_network_chart_window_open() {
            return None;
        }
        let dialog = self.network_chart_dialog.as_ref()?;
        let (width, height) =
            clonk_frontend::network_chart::NetworkChartDialog::console_window_extent();
        Some((dialog.caption().to_owned(), width, height))
    }

    pub(crate) fn console_scoreboard_window_chrome(&self) -> Option<(String, u32, u32)> {
        let layout = self.console_scoreboard_layout()?.0;
        Some((
            clonk_frontend::scoreboard::scoreboard_console_window_title(
                &self.snapshot.hud.scoreboard,
            ),
            layout.bounds.w.max(1) as u32,
            layout.bounds.h.max(1) as u32,
        ))
    }

    /// Lay the console scoreboard out at its own window origin.
    ///
    /// `Screen::ShowDialog` skips `DoPlacement` for a console dialog
    /// (`C4Gui.cpp:559-560`) — the window, not the screen, decides where the
    /// dialog lands — so the computed placement is discarded and only its size
    /// survives.
    ///
    /// The layout is rebuilt on every call rather than retained the way the
    /// fullscreen route caches `scoreboard_runtime.presentation`. C++ retains
    /// `piColWidths` and recomputes on `InvalidateRows`, but a console window
    /// has no drag, no cached placement and no pointer state to preserve
    /// across a rebuild, so the snapshot *is* the layout — the same reasoning
    /// `render_developer_object_list` records for its rows.
    fn console_scoreboard_layout(
        &self,
    ) -> Option<(
        clonk_frontend::scoreboard::ScoreboardLayout,
        HashMap<String, ImageData>,
    )> {
        if !self.console_scoreboard_window_open() {
            return None;
        }
        let font_images = resolve_scoreboard_font_images(
            &self.engine,
            &self.snapshot.hud.scoreboard,
            self.script_text_spec_resources(),
        );
        let resources = self.assets.scoreboard_resources(&font_images).ok()?;
        let mut layout = clonk_frontend::scoreboard::scoreboard_console_layout(
            clonk_frontend::classic_gui::IntRect::new(0, 0, 0, 0),
            &self.snapshot.hud.scoreboard,
            &resources,
        )
        .ok()?;
        layout.translate(-layout.bounds.x, -layout.bounds.y);
        Some((layout, font_images))
    }

    /// Draw the console chart at its window's extent.
    ///
    /// `Dialog::Draw` clears the separate window and draws the dialog into it
    /// (`C4GuiDialogs.cpp:479-489`), so the whole window *is* the dialog:
    /// `console_layout` takes the extent as its bounds rather than placing a
    /// dialog inside it, and the caption and close icon the chrome supplies
    /// are not drawn again.
    ///
    /// Like the object list's rows, the layout is rebuilt per call. A console
    /// window has no drag and no cached placement to preserve, so the
    /// snapshot is the layout.
    pub(crate) fn render_console_network_chart(
        &mut self,
        width: u32,
        height: u32,
    ) -> Option<clonk_graphics::Surface> {
        if !self.console_network_chart_window_open() {
            return None;
        }
        self.refresh_network_chart_dialog();
        let dialog = self.network_chart_dialog.as_ref()?;
        let assets = Arc::clone(&self.assets);
        let resources = assets.network_chart_resources()?;
        let mut surface = clonk_graphics::Surface::new(
            width.max(1),
            height.max(1),
            clonk_graphics::PixelFormat::Rgba8888,
        );
        let extent = clonk_frontend::classic_gui::IntRect::new(
            0,
            0,
            surface.width() as i32,
            surface.height() as i32,
        );
        dialog
            .render_console(&mut surface, extent, resources, None)
            .ok()?;
        Some(surface)
    }

    /// A press inside the console chart window, in its own coordinates.
    ///
    /// The window chrome owns moving and closing, so the only element the
    /// dialog still answers for is a sheet tab.
    pub(crate) fn console_network_chart_pointer_down(
        &mut self,
        point: clonk_frontend::GuiPoint,
    ) -> bool {
        if !self.console_network_chart_window_open() {
            return false;
        }
        let assets = Arc::clone(&self.assets);
        let Some(resources) = assets.network_chart_resources() else {
            return false;
        };
        let (width, height) =
            clonk_frontend::network_chart::NetworkChartDialog::console_window_extent();
        let extent = clonk_frontend::classic_gui::IntRect::new(0, 0, width as i32, height as i32);
        let Some(dialog) = self.network_chart_dialog.as_mut() else {
            return false;
        };
        !matches!(
            dialog.console_pointer_down(point, extent, resources),
            clonk_frontend::network_chart::NetworkChartDialogAction::Ignored
        )
    }

    /// Draw the console scoreboard at its window's extent.
    ///
    /// `Dialog::Draw` clears the separate window to the standard GUI
    /// background before drawing the dialog into it
    /// (`C4GuiDialogs.cpp:479-481`); `render_scoreboard_with_layout` paints
    /// the same frame and body the fullscreen route does, minus the caption
    /// the console has no widget for. `ordered_native` is always false in
    /// console mode, so this is the single-pass form.
    pub(crate) fn render_console_scoreboard(
        &mut self,
        width: u32,
        height: u32,
    ) -> Option<clonk_graphics::Surface> {
        let (layout, font_images) = self.console_scoreboard_layout()?;
        let resources = self.assets.scoreboard_resources(&font_images).ok()?;
        let mut surface = clonk_graphics::Surface::new(
            width.max(1),
            height.max(1),
            clonk_graphics::PixelFormat::Rgba8888,
        );
        clonk_frontend::scoreboard::render_scoreboard_with_layout(
            &mut surface,
            &self.snapshot.hud.scoreboard,
            &resources,
            &layout,
            clonk_frontend::scoreboard::ScoreboardRenderState::default(),
            None,
        )
        .ok()?;
        Some(surface)
    }

    /// Draw the object list at the window's extent.
    pub(crate) fn render_developer_object_list(
        &mut self,
        width: u32,
        height: u32,
    ) -> clonk_graphics::Surface {
        let mut surface = clonk_graphics::Surface::new(
            width.max(1),
            height.max(1),
            clonk_graphics::PixelFormat::Rgba8888,
        );
        let rows = self.developer_object_list_rows();
        self.reveal_developer_object_list_selection(&rows, height);
        let font = self.assets.font_arc();
        crate::developer_object_list_view::render_object_list(
            &mut surface,
            font.as_ref(),
            &rows,
            self.developer_selection.objects(),
            self.developer_object_list_scroll,
        );
        surface
    }

    /// Bring a newly selected row into view, once per selection.
    ///
    /// The list mirrors the edit cursor, so the selection also moves from a
    /// viewport click. `C4ObjectListDlg::Update` reacts to that by setting the
    /// cursor, which the scrolled window follows; it does not re-scroll while
    /// the selection stands, which is what leaves the user free to scroll
    /// away from it.
    fn reveal_developer_object_list_selection(
        &mut self,
        rows: &[crate::developer_object_list_view::ObjectListRow],
        height: u32,
    ) {
        let selected = self.developer_selection.objects().first().copied();
        if selected == self.developer_object_list_revealed {
            return;
        }
        self.developer_object_list_revealed = selected;
        let Some(row) = selected.and_then(|id| rows.iter().position(|row| row.id == id)) else {
            return;
        };
        self.developer_object_list_scroll
            .reveal(row, rows.len(), height);
    }

    /// How many rows the list currently has, for a caller sizing its view.
    pub(crate) fn developer_object_list_row_count(&self) -> usize {
        self.developer_object_list_rows().len()
    }

    /// One navigation key over the object list.
    ///
    /// Returns whether the list claimed it. `GtkTreeView` keeps the cursor
    /// apart from the selection: a plain arrow moves both, Ctrl moves only the
    /// cursor, and Shift extends a range from the anchor
    /// (`gtk_tree_selection_set_mode(GTK_SELECTION_MULTIPLE)`,
    /// `C4ObjectListDlg.cpp:777-779`).
    pub(crate) fn navigate_developer_object_list(
        &mut self,
        key: crate::developer_object_list_view::ObjectListKey,
        control: bool,
        shift: bool,
        height: u32,
    ) -> bool {
        use crate::developer_object_list_view::{
            object_list_navigate, ObjectListNavigation, ObjectListScroll,
        };
        use clonk_engine::developer_selection::SelectionWriter;

        let rows = self.developer_object_list_rows();
        let page = ObjectListScroll::capacity(height);
        let Some(navigation) =
            object_list_navigate(&rows, self.developer_object_list_cursor, key, page)
        else {
            return false;
        };
        match navigation {
            ObjectListNavigation::Expand(object) | ObjectListNavigation::Collapse(object) => {
                self.developer_object_tree_expansion.toggle(object);
            }
            ObjectListNavigation::MoveCursor(object) => {
                self.developer_object_list_cursor = Some(object);
                // The cursor is what the view scrolls to follow.
                let moved = rows.iter().position(|row| row.id == object);
                if let Some(row) = moved {
                    self.developer_object_list_scroll
                        .reveal(row, rows.len(), height);
                    self.developer_object_list_revealed = Some(object);
                }
                if control {
                    // Ctrl moves the cursor alone.
                    return true;
                }
                if shift {
                    self.extend_developer_object_list_selection(&rows, object);
                } else {
                    self.developer_object_list_anchor = Some(object);
                    self.developer_selection
                        .replace(SelectionWriter::ObjectTree, object);
                }
            }
        }
        true
    }

    /// Apply one clicked row under the live modifiers.
    ///
    /// `GTK_SELECTION_MULTIPLE` (`C4ObjectListDlg.cpp:777-778`) is what gives
    /// Ctrl and Shift meaning: plain replaces, Ctrl toggles the one row,
    /// Shift covers the anchor through it, and Ctrl+Shift adds that range to
    /// what is already selected.
    ///
    /// Whatever the gesture, the writeback is in **tree-path order** —
    /// `OnSelectionChanged` reads `gtk_tree_selection_get_selected_rows`, and
    /// that is ordered by path, not by when each row was clicked.
    fn select_developer_object_list_row(
        &mut self,
        rows: &[crate::developer_object_list_view::ObjectListRow],
        object: clonk_engine::ObjectId,
    ) {
        use clonk_engine::developer_selection::SelectionWriter;

        let control = self.keyboard_modifiers.control_key();
        let shift = self.keyboard_modifiers.shift_key();
        if !control && !shift {
            self.developer_object_list_anchor = Some(object);
            self.developer_selection
                .replace(SelectionWriter::ObjectTree, object);
            return;
        }

        let selected = self.developer_selection.objects().to_vec();
        let wanted: Vec<clonk_engine::ObjectId> = if shift {
            // The anchor stays put, so a second Shift-click re-covers from the
            // same place rather than from the last row reached.
            let anchor = self.developer_object_list_anchor.unwrap_or(object);
            let index_of = |id| rows.iter().position(|row| row.id == id);
            let (Some(from), Some(to)) = (index_of(anchor), index_of(object)) else {
                self.developer_object_list_anchor = Some(object);
                self.developer_selection
                    .replace(SelectionWriter::ObjectTree, object);
                return;
            };
            let (low, high) = if from <= to { (from, to) } else { (to, from) };
            let range = &rows[low..=high];
            rows.iter()
                .filter(|row| {
                    range.iter().any(|in_range| in_range.id == row.id)
                        // Ctrl+Shift adds to the selection; Shift alone
                        // replaces it.
                        || (control && selected.contains(&row.id))
                })
                .map(|row| row.id)
                .collect()
        } else {
            self.developer_object_list_anchor = Some(object);
            rows.iter()
                .filter(|row| {
                    if row.id == object {
                        !selected.contains(&object)
                    } else {
                        selected.contains(&row.id)
                    }
                })
                .map(|row| row.id)
                .collect()
        };
        self.developer_selection
            .select_frame(SelectionWriter::ObjectTree, wanted);
    }

    /// Select every visible row between the anchor and the cursor.
    fn extend_developer_object_list_selection(
        &mut self,
        rows: &[crate::developer_object_list_view::ObjectListRow],
        cursor: clonk_engine::ObjectId,
    ) {
        use clonk_engine::developer_selection::SelectionWriter;

        let anchor = self.developer_object_list_anchor.unwrap_or(cursor);
        let index_of = |id| rows.iter().position(|row| row.id == id);
        let (Some(from), Some(to)) = (index_of(anchor), index_of(cursor)) else {
            self.developer_selection
                .replace(SelectionWriter::ObjectTree, cursor);
            return;
        };
        let (low, high) = if from <= to { (from, to) } else { (to, from) };
        self.developer_selection.clear(SelectionWriter::ObjectTree);
        for row in &rows[low..=high] {
            self.developer_selection
                .toggle(SelectionWriter::ObjectTree, row.id);
        }
    }

    /// Ctrl+Space: select or deselect whatever the cursor is on, leaving the
    /// rest of the selection alone.
    pub(crate) fn toggle_developer_object_list_cursor_selection(&mut self) -> bool {
        use clonk_engine::developer_selection::SelectionWriter;

        let Some(cursor) = self.developer_object_list_cursor else {
            return false;
        };
        let rows = self.developer_object_list_rows();
        if !rows.iter().any(|row| row.id == cursor) {
            return false;
        }
        self.developer_object_list_anchor = Some(cursor);
        self.developer_selection
            .toggle(SelectionWriter::ObjectTree, cursor);
        true
    }

    /// A press on either pane's scroll bar.
    ///
    /// Returns whether the bar took it. A press the bar takes never reaches
    /// the pane underneath — in GTK the bar is a sibling widget inside the
    /// scrolled window, and the pane never sees the event.
    pub(crate) fn developer_pane_scroll_press(
        &mut self,
        pane: DeveloperPane,
        point: (i32, i32),
        extent: (u32, u32),
    ) -> bool {
        use clonk_frontend::developer_chrome::{pane_scroll_bar_press, PaneScrollPart};

        let Some(bar) = self.developer_pane_scroll_bar(pane, extent) else {
            return false;
        };
        let Some(part) = pane_scroll_bar_press(&bar, point) else {
            return false;
        };
        match part {
            PaneScrollPart::Thumb => {
                self.developer_pane_scroll_drag = Some(pane);
            }
            PaneScrollPart::LineBack => self.step_developer_pane_scroll(pane, -1, extent),
            PaneScrollPart::LineForward => self.step_developer_pane_scroll(pane, 1, extent),
            PaneScrollPart::PageBack => {
                let page = i32::try_from(bar.capacity).unwrap_or(i32::MAX);
                self.step_developer_pane_scroll(pane, -page, extent);
            }
            PaneScrollPart::PageForward => {
                let page = i32::try_from(bar.capacity).unwrap_or(i32::MAX);
                self.step_developer_pane_scroll(pane, page, extent);
            }
        }
        true
    }

    /// Pointer motion while a pane thumb is held.
    pub(crate) fn developer_pane_scroll_drag(
        &mut self,
        pane: DeveloperPane,
        point: (i32, i32),
        extent: (u32, u32),
    ) -> bool {
        use clonk_frontend::developer_chrome::pane_scroll_bar_line;

        if self.developer_pane_scroll_drag != Some(pane) {
            return false;
        }
        let Some(bar) = self.developer_pane_scroll_bar(pane, extent) else {
            return false;
        };
        let first = pane_scroll_bar_line(&bar, point.1);
        match pane {
            DeveloperPane::PropertyOutput => {
                let lines = self.developer_property_page_line_count();
                let capacity = crate::developer_toolbox_view::property_output_capacity(extent.1);
                self.developer_property_scroll
                    .scroll_to(first, lines, capacity);
            }
            DeveloperPane::ObjectList => {
                let rows = self.developer_object_list_rows().len();
                self.developer_object_list_scroll
                    .scroll_to(first, rows, extent.1);
            }
        }
        true
    }

    /// Release whichever pane thumb is held.
    pub(crate) fn developer_pane_scroll_release(&mut self) -> bool {
        self.developer_pane_scroll_drag.take().is_some()
    }

    fn developer_pane_scroll_bar(
        &self,
        pane: DeveloperPane,
        extent: (u32, u32),
    ) -> Option<clonk_frontend::developer_chrome::PaneScrollBar> {
        match pane {
            DeveloperPane::PropertyOutput => crate::developer_toolbox_view::property_output_bar(
                extent,
                self.developer_property_page_line_count(),
                self.developer_property_scroll,
            ),
            DeveloperPane::ObjectList => crate::developer_object_list_view::object_list_bar(
                extent,
                self.developer_object_list_rows().len(),
                self.developer_object_list_scroll,
            ),
        }
    }

    fn step_developer_pane_scroll(&mut self, pane: DeveloperPane, lines: i32, extent: (u32, u32)) {
        match pane {
            DeveloperPane::PropertyOutput => {
                self.scroll_developer_property_page(lines, extent.1);
            }
            DeveloperPane::ObjectList => {
                self.scroll_developer_object_list(lines, extent.1);
            }
        };
    }

    /// One wheel notch over the object list.
    ///
    /// The tree lives in an automatic scrolled window
    /// (`C4ObjectListDlg.cpp:747-780`), so the wheel moves the view and
    /// nothing else — it does not change the selection.
    pub(crate) fn scroll_developer_object_list(&mut self, rows_delta: i32, height: u32) -> bool {
        let rows = self.developer_object_list_rows().len();
        let before = self.developer_object_list_scroll;
        self.developer_object_list_scroll
            .scroll_by(rows_delta, rows, height);
        self.developer_object_list_scroll != before
    }

    /// The visible rows, for a test that needs to address one by index.
    #[cfg(test)]
    pub(crate) fn developer_object_list_rows_for_test(
        &self,
    ) -> Vec<crate::developer_object_list_view::ObjectListRow> {
        self.developer_object_list_rows()
    }

    /// The list's rows, rebuilt from the live snapshot.
    fn developer_object_list_rows(&self) -> Vec<crate::developer_object_list_view::ObjectListRow> {
        use clonk_engine::developer_inspection::object_tree;

        let tree = object_tree(&self.snapshot.render_order, &self.snapshot);
        crate::developer_object_list_view::object_list_rows(
            &tree,
            &self.developer_object_tree_expansion,
            |id| {
                // `name_cell_data_func` draws `object->GetName()` (`:659-664`),
                // which is the custom name when there is one and the definition's
                // otherwise.
                self.snapshot
                    .object(id)
                    .map(|object| {
                        object.custom_name.clone().unwrap_or_else(|| {
                            self.engine
                                .definition(&object.definition_id)
                                .map(|definition| definition.name().to_owned())
                                .unwrap_or_else(|| object.definition_id.clone())
                        })
                    })
                    .unwrap_or_default()
            },
            |id| {
                // `icon_cell_data_func` reads the definition's Graphics bitmap at
                // its PictureRect (`C4ObjectListDlg.cpp:667-722`), not the
                // object's mutable PictureRect or a separate incremental model.
                let object = self.snapshot.object(id)?;
                let image = self
                    .engine
                    .definition_picture_icon_image(&object.definition_id)?;
                if self
                    .engine
                    .definition(&object.definition_id)
                    .is_some_and(|definition| definition.color_by_owner())
                {
                    Some(clonk_gui::ImageData::new(
                        image.width(),
                        image.height(),
                        clonk_app_core::pictures::inventory_picture_pixels(&image, object.color),
                    ))
                } else {
                    Some(clonk_app_core::pictures::definition_menu_picture(image))
                }
            },
        )
    }

    /// `C4ObjectListDlg::OnSelectionChanged` (`:599-620`).
    ///
    /// The tree writes the edit cursor's selection wholesale — `Clear()` then
    /// one `Add` per selected row — and the `updating_selection` guard on both
    /// sides is what stops that write echoing back as a fresh tree update.
    /// The port has the same guard in a different shape:
    /// [`clonk_engine::developer_selection::SelectionWriter`] tags the writer,
    /// so a surface can recognise its own change instead of a flag having to
    /// be raised and lowered around every write.
    pub(crate) fn developer_object_list_click(&mut self, point: (i32, i32), extent: (u32, u32)) {
        use crate::developer_object_list_view::ObjectListClick;
        use clonk_engine::developer_selection::SelectionWriter;

        let rows = self.developer_object_list_rows();
        match crate::developer_object_list_view::object_list_hit(
            &rows,
            self.developer_object_list_scroll,
            extent.0,
            extent.1,
            point,
        ) {
            Some(ObjectListClick::Select(object)) => {
                // A click sets the cursor as well: GTK's `set_cursor` is what
                // a button press on a row performs.
                self.developer_object_list_cursor = Some(object);
                self.select_developer_object_list_row(&rows, object);
            }
            // The expander column consumes its own click: `GtkTreeView` opens
            // or closes the row and the selection does not follow.
            Some(ObjectListClick::Toggle(object)) => {
                self.developer_object_tree_expansion.toggle(object);
            }
            // No path under the pointer: `gtk_tree_selection_get_selected_rows`
            // returns an empty list and the handler still clears.
            None => {
                self.developer_selection.clear(SelectionWriter::ObjectTree);
            }
        }
    }

    /// Draw one toolbox page at the window's extent.
    pub(crate) fn render_developer_toolbox_page(
        &mut self,
        page: crate::developer_windows::ToolboxPage,
        width: u32,
        height: u32,
    ) -> clonk_graphics::Surface {
        use crate::developer_windows::ToolboxPage;

        let mut surface = clonk_graphics::Surface::new(
            width.max(1),
            height.max(1),
            clonk_graphics::PixelFormat::Rgba8888,
        );
        let font = self.assets.font_arc();
        match page {
            ToolboxPage::Tools => self
                .developer_tools_page_model()
                .render(&mut surface, font.as_ref()),
            ToolboxPage::Property => {
                let text = self.developer_property_page_text();
                let script = self.developer_property_script_input.clone();
                crate::developer_toolbox_view::render_property_page(
                    &mut surface,
                    font.as_ref(),
                    &text,
                    self.developer_property_scroll,
                    &script,
                    self.developer_console_editing(),
                    &self.runtime_resource_text("IDS_BTN_RELOADDEF", "Reload def"),
                );
            }
        }
        surface
    }

    /// How many lines the property pane currently shows, for a caller sizing
    /// its view.
    pub(crate) fn developer_property_page_line_count(&self) -> usize {
        self.developer_property_page_text().lines().count()
    }

    /// One wheel notch over the property page.
    ///
    /// The output is a scrolled text view (`C4PropertyDlg.cpp:128-140`), so a
    /// notch moves the retained first visible line and nothing else. Returns
    /// whether the view actually moved, so a caller only redraws when it did.
    pub(crate) fn scroll_developer_property_page(&mut self, lines: i32, height: u32) -> bool {
        use crate::developer_toolbox_view::property_output_capacity;

        let lines_available = self.developer_property_page_text().lines().count();
        let before = self.developer_property_scroll;
        self.developer_property_scroll.scroll_by(
            lines,
            lines_available,
            property_output_capacity(height),
        );
        self.developer_property_scroll != before
    }

    /// A click on whichever page the toolbox shows.
    pub(crate) fn developer_toolbox_click(&mut self, point: (i32, i32), extent: (u32, u32)) {
        use crate::developer_toolbox_view::ToolsPageAction;
        use crate::developer_windows::ToolboxPage;

        if self.developer_toolbox.current_page() == Some(ToolboxPage::Property) {
            let _ = self.developer_property_page_click(point, extent);
            return;
        }
        if self.developer_toolbox.current_page() != Some(ToolboxPage::Tools) {
            return;
        }
        let Some(action) = self
            .developer_tools_page_model()
            .hit(extent.0, extent.1, point)
        else {
            return;
        };
        match action {
            // The one control on the page that is a synchronized *control*:
            // every peer has to change landscape mode at the same tick
            // (`C4ToolsDlg.cpp:875-879`).
            ToolsPageAction::SetLandscapeMode(mode) => self.submit_editor_landscape_mode(mode),
            ToolsPageAction::SetTool(tool) => self.developer_tools.set_tool(tool, false),
            ToolsPageAction::SetIft(ift) => {
                self.developer_tools.set_ift(ift);
            }
            ToolsPageAction::SetGrade(grade) => {
                self.developer_tools.set_grade(grade);
            }
            // `C4ToolsDlg::SetMaterial` runs `AssertValidTexture` after the
            // material lands (`:565-572`), which is what stops a Static
            // landscape being handed a pair its tex map has no slot for.
            ToolsPageAction::SetMaterial(material) => {
                self.developer_tools.set_material(material);
                self.assert_valid_developer_texture();
                self.developer_tools_open_combo = None;
            }
            ToolsPageAction::SetTexture(texture) => {
                self.developer_tools.set_texture(texture);
                // Selecting closes the list, as a combo does.
                self.developer_tools_open_combo = None;
            }
            ToolsPageAction::OpenCombo(combo) => {
                self.developer_tools_open_combo = Some(combo);
            }
            ToolsPageAction::CloseCombo => {
                self.developer_tools_open_combo = None;
            }
        }
    }

    /// `C4ToolsDlg::AssertValidTexture` (`C4ToolsDlg.cpp:965-983`).
    fn assert_valid_developer_texture(&mut self) {
        let Some(state) = self.engine.developer_landscape_tool_state() else {
            return;
        };
        if let Some(texture) = clonk_engine::developer_landscape::corrected_tool_texture(
            state.texmap(),
            self.developer_tools.material(),
            self.developer_tools.texture(),
            state.mode,
        ) {
            self.developer_tools.set_texture(texture);
        }
    }

    /// `C4ToolsDlg::SetLandscapeMode(iMode, false)` (`C4ToolsDlg.cpp:865-879`).
    ///
    /// The local path *changes nothing*: it confirms the one destructive
    /// transition and enqueues `EMDT_SetMode`. The dialog state moves only
    /// when that control comes back out of the queue, which is what keeps
    /// every peer's landscape mode identical.
    pub(crate) fn submit_editor_landscape_mode(
        &mut self,
        target: clonk_engine::developer_tools::LandscapeMode,
    ) {
        use clonk_engine::developer_tools::landscape_mode_needs_confirmation;

        let Some(mode) = self.engine.landscape().map(|landscape| landscape.mode()) else {
            return;
        };
        if landscape_mode_needs_confirmation(landscape_mode_of(mode), target) {
            // A declined confirmation **aborts**: `SetLandscapeMode` returns
            // false before enqueueing anything (`C4ToolsDlg.cpp:869-874`). And
            // on the reference build it is always declined — `C4Console::
            // Message`'s two bodies are behind `_WIN32`/`WITH_DEVELOPER_MODE`
            // and past their `#endif` it is a bare `return false`
            // (`C4Console.cpp:841-853`). So the one destructive transition is
            // refused there, and is refused here. Saying so is the port's own
            // choice of surface; discarding an exact landscape on a click no
            // dialog ever confirmed would be the worse divergence.
            let message = self.runtime_resource_text(
                "IDS_CNS_EXACTTOSTATIC",
                "The exact landscape would be lost. Switching to static is refused.",
            );
            self.developer_console.out(&message);
            return;
        }
        if let Err(error) =
            self.submit_or_execute_editor_draw_tool(clonk_engine::EmDrawToolControlData {
                action: clonk_engine::EMDT_SET_MODE,
                mode: landscape_mode_value(target),
                ..Default::default()
            })
        {
            tracing::error!(%error, "failed to submit an editor landscape mode");
        }
    }

    /// Everything the Tools page draws, read fresh — C++ refreshes the same
    /// controls from `Game.Landscape` on every `Open` and `EnableControls`.
    pub(crate) fn developer_tools_page_model(
        &self,
    ) -> crate::developer_toolbox_view::ToolsPageModel {
        use crate::developer_toolbox_view::ToolsPageModel;
        use clonk_engine::developer_tools::LandscapeMode;

        let state = self.engine.developer_landscape_tool_state();
        let material = self.developer_tools.material().to_owned();
        ToolsPageModel {
            mode: state.as_ref().map_or(LandscapeMode::Undefined, |state| {
                landscape_mode_of(state.mode)
            }),
            has_map: state.as_ref().is_some_and(|state| state.has_map),
            tool: self.developer_tools.tool(),
            grade: self.developer_tools.grade(),
            ift: self.developer_tools.ift(),
            materials: state
                .as_ref()
                .map(|state| state.material_catalog())
                .unwrap_or_default(),
            textures: state
                .as_ref()
                .map(|state| state.texture_catalog(&material))
                .unwrap_or_default(),
            open_combo: self.developer_tools_open_combo,
            preview: self.developer_tools_preview_sample(&material),
            texture: self.developer_tools.texture().to_owned(),
            material,
        }
    }

    /// The rendered material sample the Tools page's preview box shows.
    ///
    /// Resolved here rather than in the view because the catalogues live on the
    /// application: the view is handed pixels, not a material library.
    /// `C4ToolsDlg::UpdatePreview` draws a disc of the *grade* radius, so the
    /// grade travels with it.
    fn developer_tools_preview_sample(&self, material: &str) -> Option<clonk_frontend::ImageData> {
        /// `IDC_PREVIEW`'s own extent in C++ (`C4ToolsDlg.cpp:604`).
        const PREVIEW_EXTENT: u32 = 64;

        clonk_frontend::material_preview_swatch_for(
            PREVIEW_EXTENT,
            PREVIEW_EXTENT,
            self.developer_tools.grade(),
            material,
            self.developer_tools.texture(),
            &self.material_render_info,
            &self.material_texture_images,
            clonk_graphics::Color::opaque(0x40, 0x40, 0x40),
        )
    }

    /// `C4PropertyDlg::Update` over the live selection
    /// (`C4PropertyDlg.cpp:169-256`).
    fn developer_property_page_text(&self) -> String {
        use clonk_engine::developer_property_text::{property_panel_text, PropertyPanelStrings};

        let mut strings = PropertyPanelStrings {
            no_object: String::new(),
            type_line: String::new(),
            owner: String::new(),
            contents: String::new(),
            action: String::new(),
            locals: String::new(),
            effects: String::new(),
            multiple_objects: String::new(),
        };
        for (target, key, fallback) in [
            (&mut strings.no_object, "IDS_CNS_NOOBJECT", "No object"),
            (&mut strings.type_line, "IDS_CNS_TYPE", "Type: %s (%s)"),
            (&mut strings.owner, "IDS_CNS_OWNER", "Owner: %s"),
            (&mut strings.contents, "IDS_CNS_CONTENTS", "Contents:"),
            (&mut strings.action, "IDS_CNS_ACTION", "Action:"),
            (&mut strings.locals, "IDS_CNS_LOCALS", "Local variables:"),
            (&mut strings.effects, "IDS_CNS_EFFECTS", "Effects:"),
            (
                &mut strings.multiple_objects,
                "IDS_CNS_MULTIPLEOBJECTS",
                "%d objects selected",
            ),
        ] {
            *target = self.runtime_resource_text(key, fallback);
        }
        let selection = self.developer_selection.objects();
        let object = selection
            .first()
            .filter(|_| selection.len() == 1)
            .and_then(|id| self.developer_property_page_object(*id));
        property_panel_text(&strings, selection.len(), object.as_ref())
    }

    /// A click on the property page.
    ///
    /// `IDC_BUTTONRELOADDEF` runs `Game.ReloadDef(idSelectedDef)`
    /// (`C4PropertyDlg.cpp:74-76`), and is enabled on `Console.Editing`
    /// alone (`:117`) — no selection condition, because the identity itself
    /// carries that: `ReloadDef` looks it up and returns false when the
    /// selection named none (`C4Game.cpp:2321-2323`).
    /// Returns whether a reload was dispatched, which is not the same as the
    /// click being consumed: the button is enabled whenever editing is, so a
    /// press with nothing or several things selected is a press on a live
    /// control that reaches `ReloadDef` with `C4ID_None` and does nothing.
    pub(crate) fn developer_property_page_click(
        &mut self,
        point: (i32, i32),
        extent: (u32, u32),
    ) -> bool {
        use crate::developer_toolbox_view::{property_page_hit, PropertyPageAction};

        let Some(PropertyPageAction::ReloadDef) = property_page_hit(extent, point) else {
            return false;
        };
        let Some(definition) = self.developer_property_reload_target() else {
            return false;
        };
        // `ReloadDef`'s own first line refuses in a network game
        // (`C4Game.cpp:2314`); the engine keeps that check rather than the
        // caller, so the live flag is what is passed.
        let reloaded = self
            .engine
            .reload_definition(&definition, self.network.is_some());
        tracing::debug!(%definition, reloaded, "property page reload dispatched");
        // `ReloadDef` updates every affected object's face on success and
        // removes every object of the definition on failure, so the page's
        // text is stale either way.
        self.snapshot = self.engine.snapshot();
        true
    }

    /// The definition a reload press would act on, if any.
    ///
    /// Both halves of C++'s behaviour in one place: the button is enabled on
    /// `Console.Editing` alone (`C4PropertyDlg.cpp:117`), and its argument is
    /// `idSelectedDef`, which only a single selection ever sets.
    pub(crate) fn developer_property_reload_target(&self) -> Option<String> {
        self.developer_console_editing()
            .then(|| self.developer_property_selected_definition())
            .flatten()
    }

    /// What has been typed into the script entry.
    pub(crate) fn developer_property_script_input(&self) -> &str {
        &self.developer_property_script_input
    }

    /// Append typed text, if the control is enabled.
    ///
    /// `EnableWindow(GetDlgItem(hDialog, IDC_COMBOINPUT), Console.Editing)`
    /// (`C4PropertyDlg.cpp:117`) is the entry's whole gate; a disabled combo
    /// box takes no keystroke. Returns whether it did.
    pub(crate) fn type_developer_property_script(&mut self, text: &str) -> bool {
        if !self.developer_console_editing() || text.is_empty() {
            return false;
        }
        self.developer_property_script_input.push_str(text);
        true
    }

    /// Remove the last character, if the control is enabled.
    pub(crate) fn backspace_developer_property_script(&mut self) -> bool {
        if !self.developer_console_editing() {
            return false;
        }
        self.developer_property_script_input.pop().is_some()
    }

    /// Enter: run what was typed on the live selection.
    ///
    /// `OnScriptActivate` calls `Console.EditCursor.In(text)` for nonempty
    /// text only (`C4PropertyDlg.cpp:394-399`); `In` wraps the selection into
    /// the `EMMO_Script` control this already had. Returns whether anything
    /// was submitted.
    pub(crate) fn submit_developer_property_script(&mut self) -> Result<bool, EngineError> {
        if !self.developer_console_editing() || self.developer_property_script_input.is_empty() {
            return Ok(false);
        }
        let script = std::mem::take(&mut self.developer_property_script_input);
        let objects = self
            .developer_selection
            .objects()
            .iter()
            .map(|id| id.as_u64() as i32)
            .collect::<Vec<_>>();
        self.submit_editor_selection_script(&script, &objects)?;
        Ok(true)
    }

    /// What the script entry offers to complete.
    ///
    /// The selected definition is the same `idSelectedDef` the reload button
    /// takes: only a single selection names one (`C4PropertyDlg.cpp:175,
    /// 248-249`), and `UpdateInputCtrl` is handed that object or nothing.
    pub(crate) fn developer_property_script_completions(&self) -> Vec<String> {
        self.engine
            .property_script_completions(self.developer_property_selected_definition().as_deref())
    }

    /// `C4PropertyDlg::idSelectedDef` — the reload button's target.
    ///
    /// `Update` clears it before its switch and assigns `cobj->id` only in the
    /// single-object arm, so an empty or multiple selection leaves it
    /// `C4ID_None` (`C4PropertyDlg.cpp:175,248-249,253-255`). That is what
    /// makes the button harmless in those cases rather than disabled:
    /// `Game.ReloadDef` looks the identity up and returns false when it names
    /// no definition (`C4Game.cpp:2321-2323`).
    pub(crate) fn developer_property_selected_definition(&self) -> Option<String> {
        let selection = self.developer_selection.objects();
        selection
            .first()
            .filter(|_| selection.len() == 1)
            .and_then(|id| self.snapshot.object(*id))
            .map(|object| object.definition_id.clone())
    }

    /// One selected object's already-formatted detail, in C++'s section order.
    fn developer_property_page_object(
        &self,
        id: clonk_engine::ObjectId,
    ) -> Option<clonk_engine::developer_property_text::PropertyPanelObject> {
        use clonk_engine::developer_inspection::{effect_lines, name_list};
        use clonk_engine::developer_locals::local_lines;
        use clonk_engine::developer_property_text::PropertyPanelObject;

        let object = self.snapshot.object(id)?;
        let definition = object.definition_id.clone();
        let name_of = |id: &str| {
            self.engine
                .definition(id)
                .map(|definition| definition.name().to_owned())
        };
        let contents = name_list(&object.contents, &self.snapshot, name_of);
        Some(PropertyPanelObject {
            name: object
                .custom_name
                .clone()
                .or_else(|| name_of(&definition))
                .unwrap_or_else(|| definition.clone()),
            id: definition.clone(),
            // `ValidPlr(cobj->Owner)` (`:190-194`) — NO_OWNER prints no line.
            owner: self
                .engine
                .player(object.owner)
                .map(|player| player.name().to_owned()),
            contents: (!contents.is_empty()).then_some(contents),
            // `Action.Act != ActIdle` (`:203-208`).
            action: object
                .action_procedure
                .clone()
                .filter(|action| !action.is_empty()),
            locals: local_lines(&object.local_vars, &self.developer_local_names(&definition)),
            effects: effect_lines(&object.effects),
        })
    }

    /// A definition's declared `local` names, which decide the named half of
    /// the panel's locals section.
    fn developer_local_names(&self, definition: &str) -> Vec<String> {
        self.engine
            .definition(definition)
            .map(|definition| {
                definition
                    .local_variable_names()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `C4EditCursor::Delete` (`:350-359`).
    pub(crate) fn console_delete_selection(&mut self) {
        if !self.console_editing_ok() {
            return;
        }
        self.submit_editor_selection_action(clonk_engine::EMMO_REMOVE, "delete");
    }

    /// `C4EditCursor::Duplicate` (`:376-380`).
    ///
    /// C++ does **not** open this one with `EditingOK`, unlike `Delete` — the
    /// menu item's own enablement is the only gate, so a caller reaching it
    /// another way would duplicate during a replay.
    fn console_duplicate_selection(&mut self) {
        self.submit_editor_selection_action(clonk_engine::EMMO_DUPLICATE, "duplicate");
    }

    /// `C4EditCursor::GrabContents` (`:640-651`).
    ///
    /// The selection is *replaced* by the first selected object's contents and
    /// only then exited, so the command acts on what was inside the container,
    /// not on the container. `Hold` is set before the control goes out, which
    /// is what lets the freed objects be dragged straight out of it.
    fn console_grab_contents(&mut self) {
        use clonk_engine::developer_selection::SelectionWriter;

        let Some(container) = self.developer_selection.objects().first().copied() else {
            return;
        };
        let Some(contents) = self
            .snapshot
            .object(container)
            .map(|object| object.contents.clone())
        else {
            return;
        };
        self.developer_selection
            .select_frame(SelectionWriter::EditCursor, contents);
        self.edit_cursor_hold = true;
        self.submit_editor_selection_action(clonk_engine::EMMO_EXIT, "grab contents");
    }

    /// `EMMoveObject(action, 0, 0, nullptr, &Selection)` — the three menu
    /// commands that carry no offset and no target object.
    fn submit_editor_selection_action(&mut self, action: u8, what: &str) {
        let objects = self
            .developer_selection
            .objects()
            .iter()
            .map(|id| id.as_u64() as i32)
            .collect::<Vec<_>>();
        if objects.is_empty() {
            return;
        }
        if let Err(error) =
            self.submit_or_execute_editor_selection_script(clonk_engine::EmMoveObjectControlData {
                action,
                objects,
                ..Default::default()
            })
        {
            tracing::error!(%error, "failed to submit an editor {what}");
        }
    }

    /// The edit cursor's mode as the ported console logic names it.
    pub(crate) fn console_cursor_mode(&self) -> clonk_engine::developer_cursor::CursorMode {
        use clonk_engine::developer_cursor::CursorMode;

        match self.developer_console_edit_mode {
            ConsoleEditMode::Play => CursorMode::Play,
            ConsoleEditMode::Edit => CursorMode::Edit,
            ConsoleEditMode::Draw => CursorMode::Draw,
        }
    }

    /// This window's pointer position in world coordinates, through the
    /// projection it was last drawn with (`C4Viewport.cpp:181`).
    fn console_viewport_world(
        &self,
        identity: u64,
        local: (i32, i32),
        scale: f32,
    ) -> Option<(i32, i32)> {
        let projection = self.console_viewport_projections.get(&identity)?;
        Some(
            projection
                .pointer_projection(scale)
                .world_position(local.0, local.1),
        )
    }

    /// `C4Viewport::PlayerLock` for one console viewport window.
    pub(crate) fn console_viewport_player_lock(&self, identity: u64) -> bool {
        self.physical_viewports
            .iter()
            .find(|viewport| viewport.physical_identity == identity)
            .is_some_and(|viewport| viewport.player_lock)
    }

    /// `C4Viewport::TogglePlayerLock` (`C4Viewport.cpp:250-267`), returning the
    /// lock the viewport now holds.
    ///
    /// The asymmetry is the whole point: unlocking always succeeds, while
    /// locking needs `ValidPlr(Player)`, so an ownerless viewport can never be
    /// locked and stays scrollable.
    pub(crate) fn toggle_console_viewport_player_lock(&mut self, identity: u64) -> bool {
        use clonk_engine::developer_viewport::toggle_player_lock;

        let players = &self.snapshot.players;
        let Some(viewport) = self
            .physical_viewports
            .iter_mut()
            .find(|viewport| viewport.physical_identity == identity)
        else {
            return false;
        };
        let has_valid_player = viewport.displayed_player != clonk_engine::OWNER_NONE
            && players
                .iter()
                .any(|state| state.id == viewport.displayed_player);
        viewport.player_lock = toggle_player_lock(viewport.player_lock, has_valid_player).locked;
        viewport.player_lock
    }

    /// One scroll step on a console viewport window, the way `WM_HSCROLL` and
    /// `WM_VSCROLL` move `cvp->ViewX`/`ViewY` (`C4Viewport.cpp:125-146`).
    ///
    /// A locked viewport refuses, because `ScrollBarsByViewPosition` returns
    /// false before it touches anything (`:272`) — there is no bar to move.
    /// The step itself is applied **unclamped**, exactly as the line buttons
    /// do (`ViewX -= ViewportScrollSpeed`, `:127-128,140-141`). An owned
    /// viewport is allowed a view outside the landscape — `UpdateViewPosition`
    /// clamps only `fIsNoOwnerViewport` (`:1234-1236`) and everything else just
    /// grows its borders — so clamping the step would be stricter than C++.
    pub(crate) fn scroll_console_viewport(&mut self, identity: u64, dx: i32, dy: i32) -> bool {
        use clonk_engine::developer_viewport::scroll_ranges;

        let Some((view_x, view_y, view_width, view_height)) =
            self.graphics.detached_viewport_view(identity)
        else {
            return false;
        };
        let Some(landscape) = self.snapshot.landscape.as_ref() else {
            return false;
        };
        // The refusal is the whole of `scroll_ranges`' role here: it returns
        // `None` exactly when `ScrollBarsByViewPosition` returns false.
        if scroll_ranges(
            self.console_viewport_player_lock(identity),
            view_x,
            view_y,
            view_width,
            view_height,
            landscape.width() as i32,
            // `GBackHgt`, which the renderer resolves the same way.
            landscape.estimated_height().max(1),
        )
        .is_none()
        {
            return false;
        }
        if (dx, dy) == (0, 0) {
            return false;
        }
        self.graphics
            .scroll_detached_viewport(identity, dx, dy)
            .is_some()
    }

    /// The bars a detached viewport window is currently showing, if any.
    ///
    /// `None` while the player lock is on, which is what hides them:
    /// `ScrollBarsByViewPosition` returns false immediately when locked
    /// (`C4Viewport.cpp:272`).
    fn console_viewport_scroll_bars(
        &self,
        identity: u64,
        extent: (u32, u32),
    ) -> Option<(
        clonk_engine::developer_viewport::ScrollBarLayout,
        clonk_engine::developer_viewport::ScrollRange,
        clonk_engine::developer_viewport::ScrollRange,
        (i32, i32),
    )> {
        use clonk_engine::developer_viewport::{scroll_bar_layout, scroll_ranges};

        let (view_x, view_y, view_width, view_height) =
            self.graphics.detached_viewport_view(identity)?;
        let landscape = self.snapshot.landscape.as_ref()?;
        let ranges = scroll_ranges(
            self.console_viewport_player_lock(identity),
            view_x,
            view_y,
            view_width,
            view_height,
            landscape.width() as i32,
            landscape.estimated_height().max(1),
        )?;
        let layout = scroll_bar_layout(
            Some(ranges),
            extent.0 as i32,
            extent.1 as i32,
            CONSOLE_SCROLL_BAR_THICKNESS,
        )?;
        Some((layout, ranges.0, ranges.1, (view_width, view_height)))
    }

    /// A press inside a detached viewport window's scroll chrome.
    ///
    /// Returns whether the chrome took it. A press it takes never reaches the
    /// gameplay or editor routing underneath — the bars are window chrome, and
    /// in C++ they are separate child controls that Windows dispatches to
    /// before the viewport ever sees a message.
    pub(crate) fn console_viewport_scroll_press(
        &mut self,
        identity: u64,
        point: (i32, i32),
        extent: (u32, u32),
    ) -> bool {
        use clonk_engine::developer_viewport::{scroll_bar_hit, scroll_bar_step, ScrollBarPart};

        let Some((layout, horizontal, vertical, view)) =
            self.console_viewport_scroll_bars(identity, extent)
        else {
            return false;
        };
        let Some(press) = scroll_bar_hit(&layout, point) else {
            return false;
        };
        if press.part == ScrollBarPart::Thumb {
            self.console_viewport_scroll_drag = Some((identity, press.axis));
            return true;
        }
        let (dx, dy) = scroll_bar_step(press.axis, press.part, view);
        let _ = (horizontal, vertical);
        self.scroll_console_viewport(identity, dx, dy);
        true
    }

    /// Pointer motion while a thumb is held.
    pub(crate) fn console_viewport_scroll_drag(
        &mut self,
        identity: u64,
        point: (i32, i32),
        extent: (u32, u32),
    ) -> bool {
        use clonk_engine::developer_viewport::{scroll_bar_thumb_position, ScrollAxis};

        let Some((held, axis)) = self.console_viewport_scroll_drag else {
            return false;
        };
        if held != identity {
            return false;
        }
        let Some((layout, horizontal, vertical, _)) =
            self.console_viewport_scroll_bars(identity, extent)
        else {
            return false;
        };
        let range = match axis {
            ScrollAxis::Horizontal => horizontal,
            ScrollAxis::Vertical => vertical,
        };
        let target = scroll_bar_thumb_position(axis, &layout, range, point);
        // `SB_THUMBTRACK` assigns the position; the port's viewport moves by a
        // delta, so the delta is whatever closes the gap.
        let delta = target - range.position;
        match axis {
            ScrollAxis::Horizontal => self.scroll_console_viewport(identity, delta, 0),
            ScrollAxis::Vertical => self.scroll_console_viewport(identity, 0, delta),
        };
        true
    }

    /// Release the thumb, whichever window holds it.
    pub(crate) fn console_viewport_scroll_release(&mut self) -> bool {
        self.console_viewport_scroll_drag.take().is_some()
    }

    /// `C4EditCursor::AltDown`/`AltUp` (`C4EditCursor.cpp:773-792`).
    ///
    /// Alt selects the picker for as long as it is held and restores the
    /// previous tool on release. Both arms are no-ops outside Draw mode, so a
    /// mode switch mid-hold can never strand the override.
    pub(crate) fn update_console_editor_modifiers(
        &mut self,
        modifiers: winit::keyboard::ModifiersState,
    ) {
        self.keyboard_modifiers = modifiers;
        if self.developer_console_edit_mode != ConsoleEditMode::Draw {
            return;
        }
        if modifiers.alt_key() {
            self.developer_tools.press_alt(true);
        } else {
            self.developer_tools.release_alt();
        }
    }

    /// `C4EditCursor::LeftButtonDown`'s Draw arm (`C4EditCursor.cpp:220-236`).
    ///
    /// Brush applies on the click itself, Line and Rect only record their
    /// anchor, Fill arms the per-frame repeat, and the picker samples the
    /// landscape into the tools instead of drawing at all.
    fn console_draw_press(&mut self, identity: u64, local: (i32, i32), scale: f32) {
        use clonk_engine::developer_tools::Tool;

        let Some((x, y)) = self.console_viewport_world(identity, local, scale) else {
            return;
        };
        match self.developer_tools.tool() {
            // `ApplyToolPicker` ends with `Hold = false` (`:731`), so a picker
            // click never arms a drag — press, sample, release.
            Tool::Picker => {
                self.developer_tools.press(x, y);
                self.console_apply_tool_picker(x, y);
                self.developer_tools.release(x, y);
            }
            // A halted game refuses Fill outright and says so, clearing Hold
            // with it so the frame repeat never starts (`:227-231`).
            Tool::Fill if self.runtime_halt_active() => {
                let message = self.runtime_resource_text(
                    "IDS_CNS_FILLNOHALT",
                    "The fill tool cannot be used in halt mode.",
                );
                self.developer_console.out(&message);
            }
            _ => {
                if let Some(control) = self.developer_tools.press(x, y) {
                    self.submit_editor_draw_tool(control);
                }
            }
        }
    }

    /// `C4EditCursor::Move`'s Draw arm (`C4EditCursor.cpp:145-154`). Only the
    /// brush draws while the button is down; every tool still moves the cursor.
    fn console_draw_motion(&mut self, identity: u64, local: (i32, i32), scale: f32) {
        let Some((x, y)) = self.console_viewport_world(identity, local, scale) else {
            return;
        };
        if let Some(control) = self.developer_tools.drag(x, y) {
            self.submit_editor_draw_tool(control);
        }
    }

    /// `C4EditCursor::LeftButtonUp`'s Draw arm (`C4EditCursor.cpp:297-306`).
    ///
    /// The release carries no coordinates of its own: C++ reads the `X`/`Y`
    /// the window's preceding motion message already stored, which is exactly
    /// what the tools' retained cursor holds.
    /// A middle-button release inside a console viewport window.
    ///
    /// `C4EditCursor::MiddleButtonUp` is `if (Hold) return; ApplyToolPicker();`
    /// (`C4EditCursor.cpp:343-348`) — the picker whatever tool is selected, and
    /// nothing at all while a drag is held. It exists only in the editor arm
    /// of `C4Viewport`'s dispatch (`C4Viewport.cpp:190`), so the Play arm never
    /// sees a middle button and neither does this.
    /// Returns whether the picker ran, which is what a window redraws for.
    pub(crate) fn console_viewport_middle_release(
        &mut self,
        identity: u64,
        local: (i32, i32),
        scale: f32,
    ) -> bool {
        if self.edit_cursor_hold || self.developer_tools.holding() {
            return false;
        }
        let Some((x, y)) = self.console_viewport_world(identity, local, scale) else {
            return false;
        };
        self.console_apply_tool_picker(x, y);
        true
    }

    /// `C4EditCursor::ApplyToolPicker` (`C4EditCursor.cpp:698-731`) — what the
    /// picker samples goes into the tools dialog, not onto the landscape.
    fn console_apply_tool_picker(&mut self, x: i32, y: i32) {
        if let Some(pick) = self.engine.developer_tool_pick(x, y) {
            self.developer_tools.apply_pick(&pick);
        }
    }

    /// `C4EditCursor::EditingOK` (`C4EditCursor.cpp:673-682`), which every
    /// `ApplyTool*` opens with.
    ///
    /// It is not a predicate: a refusal drops `Hold` and reports itself, so a
    /// drag the console may not make stops at the first stroke instead of
    /// asking again on every pointer message. `C4Console::Message` shows
    /// nothing at all on the reference build — both its arms sit behind
    /// `_WIN32`/`WITH_DEVELOPER_MODE` (`C4Console.cpp:841-853`) — so the log is
    /// the port's own choice of surface, the one the save and reload notices
    /// already use.
    pub(crate) fn console_editing_ok(&mut self) -> bool {
        if self.developer_console_editing() {
            return true;
        }
        self.developer_tools.clear_hold();
        let message =
            self.runtime_resource_text("IDS_CNS_NONETEDIT", "No editing while replaying.");
        self.developer_console.out(&message);
        false
    }

    /// Pack one finished gesture into `C4ControlEMDrawTool` and queue it
    /// (`C4EditCursor::ApplyToolBrush` and its siblings, `:551-580`).
    ///
    /// The landscape mode travels with the control because the executor
    /// refuses a packet whose mode no longer matches
    /// (`C4Control.cpp:1015-1016`).
    fn submit_editor_draw_tool(&mut self, control: clonk_engine::developer_tools::DrawControl) {
        use clonk_engine::developer_tools::DrawControl;

        if !self.console_editing_ok() {
            return;
        }
        let Some(mode) = self.engine.landscape().map(|landscape| landscape.mode()) else {
            return;
        };
        let Some(material) =
            clonk_engine::LegacyCString::from_bytes(self.developer_tools.material().into())
        else {
            tracing::warn!("the selected draw material contained an embedded NUL");
            return;
        };
        let Some(texture) =
            clonk_engine::LegacyCString::from_bytes(self.developer_tools.texture().into())
        else {
            tracing::warn!("the selected draw texture contained an embedded NUL");
            return;
        };
        let ift = self.developer_tools.ift();
        let (action, x, y, x2, y2, ift) = match control {
            DrawControl::Brush { x, y } => (clonk_engine::EMDT_BRUSH, x, y, 0, 0, ift),
            DrawControl::Line { x, y, x2, y2 } => (clonk_engine::EMDT_LINE, x, y, x2, y2, ift),
            DrawControl::Rect { x, y, x2, y2 } => (clonk_engine::EMDT_RECT, x, y, x2, y2, ift),
            // Fill passes `0` for X2 and forces IFT false (`:579`).
            DrawControl::Fill { x, y, y2 } => (clonk_engine::EMDT_FILL, x, y, 0, y2, false),
        };
        if let Err(error) =
            self.submit_or_execute_editor_draw_tool(clonk_engine::EmDrawToolControlData {
                action,
                mode,
                x,
                y,
                x2,
                y2,
                grade: self.developer_tools.grade(),
                ift,
                material,
                texture,
                ..Default::default()
            })
        {
            tracing::error!(%error, "failed to submit an editor draw tool");
        }
    }

    /// `C4EditCursor::MoveSelection` — `EMMoveObject(EMMO_Move, xoff, yoff,
    /// nullptr, &Selection)` (`C4EditCursor.cpp`).
    ///
    /// Editing is a *control*, not a direct mutation: it goes through the same
    /// queue as every other player action so a network game stays in lockstep,
    /// which is why `EMMO_Script` already takes this path.
    fn submit_editor_move_selection(&mut self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            // C++ still re-issues a zero-offset EMMO_Move every tick from
            // Execute while Hold is set (`edit_tick_move`); a *motion* message
            // that moved nothing is not that path and emits nothing.
            return;
        }
        let objects = self
            .developer_selection
            .objects()
            .iter()
            .map(|id| id.as_u64() as i32)
            .collect::<Vec<_>>();
        if objects.is_empty() {
            return;
        }
        if let Err(error) =
            self.submit_or_execute_editor_selection_script(clonk_engine::EmMoveObjectControlData {
                action: clonk_engine::EMMO_MOVE,
                tx: dx,
                ty: dy,
                objects,
                ..Default::default()
            })
        {
            tracing::error!(%error, "failed to submit an editor move");
        }
    }

    /// `C4EditCursor::UpdateDropTarget` (`C4EditCursor.cpp:653-670`).
    fn console_drop_target(
        &self,
        control: bool,
        cursor: (i32, i32),
    ) -> Option<clonk_engine::ObjectId> {
        use clonk_engine::developer_cursor::{drop_target, DropCandidate};

        let selection = self.developer_selection.objects();
        if !control || selection.is_empty() {
            return None;
        }
        let shapes = clonk_engine::EditCursorHitTest::new(&self.snapshot);
        // `Game.Objects` master order, the reverse of the draw order.
        let candidates = self
            .snapshot
            .render_order
            .iter()
            .rev()
            .filter_map(|id| {
                let object = self.snapshot.object(*id)?;
                let shape = shapes.shape_rect(*id)?;
                Some(DropCandidate {
                    id: *id,
                    deleted: false,
                    contained: object.container.is_some(),
                    // `object_live_shape_rect` is already
                    // `cobj->x + cobj->Shape.x`.
                    shape_x: shape.x,
                    shape_y: shape.y,
                    shape_width: shape.width,
                    shape_height: shape.height,
                })
            })
            .collect::<Vec<_>>();
        drop_target(control, selection, cursor, &candidates)
    }

    /// `C4EditCursor::PutContents` — `EMMoveObject(EMMO_Enter, 0, 0,
    /// DropTarget, &Selection)`.
    fn submit_editor_enter(&mut self, target: clonk_engine::ObjectId) {
        let objects = self
            .developer_selection
            .objects()
            .iter()
            .map(|id| id.as_u64() as i32)
            .collect::<Vec<_>>();
        if objects.is_empty() {
            return;
        }
        if let Err(error) =
            self.submit_or_execute_editor_selection_script(clonk_engine::EmMoveObjectControlData {
                action: clonk_engine::EMMO_ENTER,
                target_object: target.as_u64() as i32,
                objects,
                ..Default::default()
            })
        {
            tracing::error!(%error, "failed to submit an editor enter");
        }
    }

    /// `C4EditCursor::Execute`'s Edit arm (`C4EditCursor.cpp:65-69`) — while
    /// `Hold` is set it re-issues a **zero-offset** `EMMO_Move` every tick, so
    /// a stationary held selection still produces control traffic.
    ///
    /// Draw mode has its own arm in the same switch (`:60-67`): the Fill tool
    /// alone repeats, and only while the game runs and the console can edit.
    pub(crate) fn console_edit_cursor_tick(&mut self) {
        use clonk_engine::developer_cursor::{edit_tick_move, CursorMode};

        let mode = match self.developer_console_edit_mode {
            ConsoleEditMode::Play => CursorMode::Play,
            ConsoleEditMode::Edit => CursorMode::Edit,
            ConsoleEditMode::Draw => CursorMode::Draw,
        };
        if mode == CursorMode::Draw {
            self.console_draw_tools_tick();
            return;
        }
        if edit_tick_move(mode, self.edit_cursor_hold).is_none() {
            self.edit_cursor_tick_frame = None;
            return;
        }
        // Once per engine tick, not once per event-loop wake.
        let frame = self.engine.frame();
        if self.edit_cursor_tick_frame == Some(frame) {
            return;
        }
        self.edit_cursor_tick_frame = Some(frame);
        let objects = self
            .developer_selection
            .objects()
            .iter()
            .map(|id| id.as_u64() as i32)
            .collect::<Vec<_>>();
        if objects.is_empty() {
            return;
        }
        if let Err(error) =
            self.submit_or_execute_editor_selection_script(clonk_engine::EmMoveObjectControlData {
                action: clonk_engine::EMMO_MOVE,
                objects,
                ..Default::default()
            })
        {
            tracing::error!(%error, "failed to submit the held editor move");
        }
    }

    /// `C4EditCursor::Execute`'s Draw arm — `case C4TLS_Fill: if (Hold) if
    /// (!Game.HaltCount) if (Console.Editing) ApplyToolFill();` (`:60-67`).
    fn console_draw_tools_tick(&mut self) {
        let halted = self.runtime_halt_active();
        let editing = self.developer_console_editing();
        let Some(control) = self.developer_tools.execute_frame(halted, editing) else {
            self.edit_cursor_tick_frame = None;
            return;
        };
        // Once per engine tick, not once per event-loop wake: `C4Console::
        // Execute` runs the edit cursor exactly once per application tick.
        let frame = self.engine.frame();
        if self.edit_cursor_tick_frame == Some(frame) {
            return;
        }
        self.edit_cursor_tick_frame = Some(frame);
        self.submit_editor_draw_tool(control);
    }

    /// `Config.Developer.AutoFileReload`, defaulting true (`C4Config.cpp:434`).
    pub(crate) fn configured_auto_file_reload(&self) -> bool {
        crate::configured_auto_file_reload(&crate::load_native_config_bytes(
            self.app_paths.as_ref(),
        ))
    }

    /// `C4Game::InitGame`'s monitor arming plus `InitGameFinal`'s start
    /// (`C4Game.cpp:2413-2424`, `:2738`).
    ///
    /// The ordering is the whole contract: create the monitor, register every
    /// unpacked definition group, *then* start it. `C4FileMonitor::AddDirectory`
    /// on the reference backend is `if (!started) paths.emplace_back(...)`
    /// (`C4FileMonitor.cpp:299-305`), so a directory registered after the start
    /// is silently dropped — which is safe only because C++ registers during
    /// definition loading and starts afterwards.
    pub(crate) fn arm_developer_file_monitor(&mut self, auto_file_reload: bool) {
        use clonk_engine::developer_file_monitor::should_arm_file_monitor;

        // `Application.isFullScreen` is the negation of console mode: a
        // fullscreen session never watches, however the key is set.
        if !should_arm_file_monitor(
            auto_file_reload,
            !self.console_mode,
            self.file_monitor.is_some(),
        ) {
            return;
        }
        let mut monitor = clonk_platform::file_monitor::DirectoryMonitor::new();
        for directory in self.engine.monitored_definition_directories() {
            monitor.add_directory(directory);
        }
        monitor.start();
        tracing::debug!(
            watched = monitor.watched().len(),
            "armed the developer file monitor"
        );
        self.file_monitor = Some(monitor);
    }

    /// Deliver whatever the monitor saw to `C4Game::ReloadFile`'s dispatcher.
    ///
    /// `C4FileMonitor`'s callback is bound straight to `C4Game::ReloadFile`
    /// (`C4Game.cpp:2418`), which refuses in a network game, routes a matched
    /// definition to `ReloadDef`, and offers everything else to the script
    /// host — the fallback, not a sibling branch.
    pub(crate) fn poll_developer_file_monitor(&mut self) {
        use clonk_engine::developer_reload::{changed_file_route, ChangedFileRoute};

        let Some(monitor) = self.file_monitor.as_mut() else {
            return;
        };
        let changed = monitor.poll();
        if changed.is_empty() {
            return;
        }
        let network_game = self.network.is_some();
        for path in changed {
            let path = path.to_string_lossy().into_owned();
            let route = changed_file_route(network_game, &path, |candidate| {
                self.engine.definition_id_for_source_path(candidate)
            });
            match route {
                ChangedFileRoute::RefusedInNetwork => return,
                ChangedFileRoute::Definition { definition } => {
                    let reloaded = self.engine.reload_definition(&definition, network_game);
                    tracing::debug!(%definition, reloaded, "developer reload dispatched");
                }
                ChangedFileRoute::Script { relative_path } => {
                    tracing::debug!(%relative_path, "developer reload found no definition");
                }
            }
        }
    }

    /// Draw one console viewport window's frame.
    ///
    /// This is `C4Viewport::Execute` (`C4Viewport.cpp:1126-1155`) for a
    /// windowed viewport: it draws the one viewport its window owns, at that
    /// window's own extent, and hands the result back for the window to blit —
    /// `BlitOutput` page-flips immediately for a windowed viewport and defers
    /// only for a fullscreen one (`:1121-1124`).
    ///
    /// `width`/`height` are the window's logical extent. C++ derives them in
    /// `UpdateOutputSize` as `ceilf(rect.Wdt / scale)` (`:798`) from the
    /// window's own drawable, so the caller converts before calling.
    ///
    /// Returns `None` when the identity no longer has a physical viewport —
    /// a closed viewport's window goes blank rather than adopting another
    /// viewport's view.
    pub(crate) fn render_console_viewport(
        &mut self,
        identity: u64,
        width: u32,
        height: u32,
    ) -> Option<clonk_graphics::Surface> {
        self.graphics.set_pxs_graphics(self.display_flags.pxs_gfx);
        let Self {
            snapshot,
            graphics,
            physical_viewports,
            ..
        } = self;
        let inputs =
            collect_viewport_inputs_from_physical_state(snapshot, physical_viewports).ok()?;
        let mut frame =
            graphics.render_detached_viewport(snapshot, &inputs, identity, width, height)?;
        // `C4Viewport::Draw` calls `Console.EditCursor.Draw(cgo)` after the
        // foreground objects and before the per-player HUD, gated on
        // `!Application.isFullScreen` (`C4Viewport.cpp:1102-1108`). It draws
        // through the engine's own rasterizer, so it lands on this surface.
        Self::draw_console_overlay(
            &mut frame.surface,
            snapshot,
            frame.projection,
            self.developer_selection.objects(),
            self.edit_cursor_drag_frame,
        );
        // `ScrollBarsByViewPosition` is fed the view the frame was drawn with,
        // not a later one, so a bar can never describe a position the window
        // is not showing.
        if let Some((view_x, view_y, view_width, view_height)) =
            self.graphics.detached_viewport_view(identity)
        {
            let locked = self.console_viewport_player_lock(identity);
            let ranges = self.snapshot.landscape.as_ref().and_then(|landscape| {
                clonk_engine::developer_viewport::scroll_ranges(
                    locked,
                    view_x,
                    view_y,
                    view_width,
                    view_height,
                    landscape.width() as i32,
                    // `GBackHgt`, resolved the way the renderer resolves it.
                    landscape.estimated_height().max(1),
                )
            });
            Self::draw_console_scroll_bars(&mut frame.surface, ranges);
        }
        // The frame a window drew is what its pointer input must be converted
        // through; nothing else records this viewport's own ViewX/ViewY.
        self.console_viewport_projections
            .insert(identity, frame.projection);
        // The context menu is *not* part of `C4Viewport::Draw`: C++ hands it to
        // the window system, which paints it above the window entirely. With no
        // OS popup to hand it to, the port paints it last, over everything the
        // viewport just drew.
        if let Some((open, menu)) = self.console_viewport_context_menu.as_ref() {
            if *open == identity {
                let font = self.assets.font_arc();
                menu.render(&mut frame.surface, font.as_ref());
            }
        }
        Some(frame.surface)
    }

    /// Paint `C4EditCursor::Draw`'s command list onto a finished viewport
    /// frame (`clonk_engine::developer_overlay`).
    ///
    /// Selection marks are twelve individual pixels per corner, not a
    /// rectangle outline, and nothing at all when the shape is under a pixel
    /// wide or tall — `select_mark_pixels` owns that rule. Coordinates are
    /// viewport-space: `cobj->x + cobj->Shape.x - ViewX`.
    /// Draws an unlocked viewport's scroll bars onto its own surface.
    ///
    /// The thickness and the two greys are this port's own: the reference
    /// macOS build compiles `ScrollBarsByViewPosition` away entirely
    /// (`C4Viewport.cpp:634-635`), so there is no C++ presentation to mirror —
    /// only the proportions, which [`scroll_bar_layout`] takes straight from
    /// the ranges. A locked viewport is handed `None` and draws nothing, which
    /// is C++'s `if (PlayerLock) return false` (`C4Viewport.cpp:272`).
    fn draw_console_scroll_bars(
        surface: &mut clonk_graphics::Surface,
        ranges: Option<(
            clonk_engine::developer_viewport::ScrollRange,
            clonk_engine::developer_viewport::ScrollRange,
        )>,
    ) {
        use clonk_engine::developer_viewport::scroll_bar_layout;

        let Some(layout) = scroll_bar_layout(
            ranges,
            surface.width() as i32,
            surface.height() as i32,
            CONSOLE_SCROLL_BAR_THICKNESS,
        ) else {
            return;
        };
        let track = clonk_graphics::Color::opaque(24, 24, 24);
        let thumb = clonk_graphics::Color::opaque(168, 168, 168);
        let arrow = clonk_graphics::Color::opaque(104, 104, 104);
        for bar in [layout.horizontal, layout.vertical] {
            Self::fill_surface_rect(surface, bar.track, track);
            // Drawn before the thumb, so a thumb that has slid under an arrow
            // still reads as the thumb.
            for end in clonk_engine::developer_viewport::scroll_bar_arrows(bar) {
                Self::fill_surface_rect(surface, end, arrow);
            }
            Self::fill_surface_rect(surface, bar.thumb, thumb);
        }
    }

    /// Fills one rectangle, clipped to the surface.
    fn fill_surface_rect(
        surface: &mut clonk_graphics::Surface,
        rect: clonk_engine::developer_viewport::BarRect,
        color: clonk_graphics::Color,
    ) {
        for y in rect.y.max(0)..(rect.y + rect.height).min(surface.height() as i32) {
            for x in rect.x.max(0)..(rect.x + rect.width).min(surface.width() as i32) {
                let _ = surface.set_pixel(x as u32, y as u32, color);
            }
        }
    }

    fn draw_console_overlay(
        surface: &mut clonk_graphics::Surface,
        snapshot: &SimulationSnapshot,
        projection: clonk_frontend::ActiveViewportProjection,
        selection: &[clonk_engine::ObjectId],
        drag_frame: Option<((i32, i32), (i32, i32))>,
    ) {
        use clonk_engine::developer_overlay::{
            console_overlay_commands, ConsoleOverlayCommand, OverlaySelection,
        };

        // The same world view the hit test uses, so the mark frames exactly
        // the shape a click resolves against.
        let shapes = clonk_engine::EditCursorHitTest::new(snapshot);
        let entries = selection
            .iter()
            .filter_map(|id| {
                let shape = shapes.shape_rect(*id)?;
                Some(OverlaySelection {
                    object: *id,
                    // `object_live_shape_rect` already returns the shape in
                    // *world* coordinates — `cobj->x + cobj->Shape.x`, the
                    // whole left-hand side of C++'s expression — so only the
                    // view origin is subtracted here. Adding the position
                    // again would double-count it.
                    x: shape.x - projection.target_x,
                    y: shape.y - projection.target_y,
                    width: shape.width,
                    height: shape.height,
                })
            })
            .collect::<Vec<_>>();
        // `DrawFrame` normalises the corners, so the band is the same
        // rectangle whichever way the drag went (`developer_overlay`).
        let band = drag_frame.map(|(anchor, corner)| {
            (
                (
                    anchor.0 - projection.target_x,
                    anchor.1 - projection.target_y,
                ),
                (
                    corner.0 - projection.target_x,
                    corner.1 - projection.target_y,
                ),
            )
        });
        let commands = console_overlay_commands(false, false, &entries, band, None, None);

        let white = clonk_graphics::Color::opaque(255, 255, 255);
        for command in commands {
            // Only the select mark is drawn: the remaining commands need the
            // drag gestures that are not wired yet, and drawing half a rubber
            // band would be worse than drawing none.
            if let ConsoleOverlayCommand::SelectMark { pixels, .. } = command {
                for (x, y) in pixels {
                    if x >= 0 && y >= 0 {
                        let _ = surface.set_pixel(x as u32, y as u32, white);
                    }
                }
            }
        }
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
        self.graphics
            .set_scroll_smooth(self.display_flags.scroll_smooth);
        self.graphics.set_renderer_config(
            self.display_flags.show_player_hud_always,
            self.display_flags.splitscreen_dividers,
        );
        self.graphics.set_pxs_graphics(self.display_flags.pxs_gfx);
        // Only the measured-cost governor suppresses the flame draws. The
        // static `Config.Graphics.FireParticles` is honoured engine-side by
        // `Engine::set_fire_particles`, where C++ folds it into
        // `SetDefParticles`: it stops the automatic emitter without hiding
        // script-created Fire/Fire2 particles, which a renderer gate on the
        // same flag would.
        self.graphics
            .set_fire_particle_detail(self.presentation_detail.draws_fire_particles());
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
        self.update_diagnostics_overlay();
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
        let speaking_object_ids = collect_speaking_overlay_objects(
            &self.snapshot,
            &self.voice_chat.active_speakers(Instant::now()),
        );
        // The developer HUD is an additive payload for the frontend chrome.
        // `render_frame_hud_chrome` draws it before this app-owned sequence,
        // where classic network status, help, flash, dialogs and errors retain
        // their independent presentation layers and ordering.
        let overlay = GraphicsOverlay {
            frame_text: &self.frame_text,
            status_text: &self.status_text,
            debug_hud: self.debug_hud,
            viewport_overlays_visible,
            players,
            game_time_seconds: self.game_time_seconds(),
            message_board,
            crew_name_labels,
            speaking: SpeakingOverlay {
                gui_icons: (!speaking_object_ids.is_empty())
                    .then(|| self.assets.dialog_image("GUIIcons.png"))
                    .flatten(),
                object_ids: speaking_object_ids,
            },
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
                                    // A row count set before the first draw is
                                    // discarded by that draw's InitLocation
                                    // (C4Menu.cpp:713-721,796-797).
                                    explicit_lines: None,
                                    applied_menu_lines: menu.lines,
                                    applied_location_reset_generation: menu
                                        .location_reset_generation,
                                    location_reset_pending: false,
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
                let text_spec_resources = self.script_text_spec_resources();
                let font_images =
                    resolve_script_menu_font_images(&self.engine, menu, text_spec_resources);
                let item_icons = self.script_menu_item_icons(menu);
                // C4MenuItem::DrawElement blits a row symbol only while its
                // facet holds a surface (C4Menu.cpp:166), so a picture that
                // never resolved draws an empty cell. C++ renders each symbol
                // once at refill time, while these recipes resolve against the
                // frame's snapshot, so a row can outlive the object it was
                // built from; failing the frame there ends the event loop.
                for (index, (item, image)) in menu.items.iter().zip(&item_icons).enumerate() {
                    if item.symbol == clonk_engine::ObjectMenuSymbol::Definition
                        && item.image != clonk_engine::ObjectMenuImage::None
                        && image.is_none()
                    {
                        // Redraws repeat this every frame the row survives, so
                        // it stays below the default log level.
                        tracing::debug!(
                            index,
                            style = menu.style,
                            recipe = ?item.image,
                            "classic menu item drawn without its unresolved picture"
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
                // C4Menu::SetSize assigns Lines without clearing LocationSet,
                // so a SetMenuSize on an already-displayed menu keeps its
                // explicit row count (C4Menu.cpp:635-640).
                if let Some(state) = self
                    .script_menu_presentations
                    .get_mut(&script_menu_owner)
                    .filter(|state| {
                        !state.location_reset_pending && state.applied_menu_lines != menu.lines
                    })
                {
                    state.explicit_lines = (menu.lines > 0).then_some(menu.lines);
                    state.applied_menu_lines = menu.lines;
                }
                if let Some(state) = self.script_menu_presentations.get_mut(&script_menu_owner) {
                    sync_script_menu_presentation_location_reset(state, menu);
                    state.location_reset_pending = false;
                }
                let (
                    menu_location,
                    retained_scroll_y,
                    adjust_selection,
                    initialize_location,
                    explicit_lines,
                ) = self
                    .script_menu_presentations
                    .get(&script_menu_owner)
                    .filter(|state| same_script_menu_presentation(state, *target, menu))
                    .map(|state| {
                        (
                            state.location,
                            state.scroll_y,
                            state.selection_needs_adjustment,
                            state.location_needs_initialization,
                            state.explicit_lines,
                        )
                    })
                    .unwrap_or((None, 0, true, false, None));
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
                            explicit_lines,
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
                            explicit_lines,
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
                        &item_icons,
                        self.display_flags.show_commands,
                        &font_images,
                        menu_location.expect("free anchor has a location"),
                        retained_scroll_y,
                        adjust_selection,
                        explicit_lines,
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
                // No preflight: `FrameDecoration` stores whatever the
                // definition's callbacks returned and `Draw` works with it as
                // it is — the background box is unconditional, the edge
                // helpers return on an empty facet, a border wider than the
                // frame makes their loop condition false immediately, and the
                // corners are clipped by `C4Facet::Draw`
                // (C4GuiDialogs.cpp:110-135, 150-196).
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
                            explicit_lines,
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
                            explicit_lines,
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
        if construction_cursor_drawn && self.mouse_control && self.keyboard_modifiers.shift_key() {
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
                            && self.keyboard_modifiers.shift_key()
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

        // The port's own overlay follows the status it yields to. It composes
        // nothing unless `Graphics.ShowStats` is on, so with the key unset
        // this adds no draw site and the frame stays oracle-exact.
        if self.graphics.draw_diagnostics_overlay(Some(&frame_gamma)) && ordered_native {
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

/// `Game.Landscape.Mode` as the ported tools state names it.
///
/// The two spellings exist because `C4LSC_*` is an untyped `int32_t` on the
/// control wire and in the landscape, while the console's own logic is written
/// against an enum that cannot hold a fifth value.
pub(crate) fn landscape_mode_of(mode: i32) -> clonk_engine::developer_tools::LandscapeMode {
    use clonk_engine::developer_tools::LandscapeMode;
    use clonk_engine::landscape::{
        LANDSCAPE_MODE_DYNAMIC, LANDSCAPE_MODE_EXACT, LANDSCAPE_MODE_STATIC,
    };

    match mode {
        LANDSCAPE_MODE_DYNAMIC => LandscapeMode::Dynamic,
        LANDSCAPE_MODE_STATIC => LandscapeMode::Static,
        LANDSCAPE_MODE_EXACT => LandscapeMode::Exact,
        _ => LandscapeMode::Undefined,
    }
}

/// The `C4LSC_*` value an `EMDT_SetMode` control carries.
pub(crate) fn landscape_mode_value(mode: clonk_engine::developer_tools::LandscapeMode) -> i32 {
    use clonk_engine::developer_tools::LandscapeMode;
    use clonk_engine::landscape::{
        LANDSCAPE_MODE_DYNAMIC, LANDSCAPE_MODE_EXACT, LANDSCAPE_MODE_STATIC,
        LANDSCAPE_MODE_UNDEFINED,
    };

    match mode {
        LandscapeMode::Dynamic => LANDSCAPE_MODE_DYNAMIC,
        LandscapeMode::Static => LANDSCAPE_MODE_STATIC,
        LandscapeMode::Exact => LANDSCAPE_MODE_EXACT,
        LandscapeMode::Undefined => LANDSCAPE_MODE_UNDEFINED,
    }
}

/// The group entry each editable component lives in.
///
/// `Game.Script`, `Game.Title` and `Game.Info` are loaded from these names;
/// the Script editor's is the unlocalised `Script.c`, because that is the file
/// a scenario's own script is written to and the one `EditScript` reloads.
pub(crate) fn developer_component_filename(
    component: clonk_engine::developer_components::EditableComponent,
) -> &'static str {
    use clonk_engine::developer_components::EditableComponent;

    match component {
        EditableComponent::Script => "Script.c",
        EditableComponent::Title => "Title.txt",
        EditableComponent::Info => "Info.txt",
    }
}
