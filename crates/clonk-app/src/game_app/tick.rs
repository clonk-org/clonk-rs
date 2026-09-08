//! `impl GameApp` — the application tick that runs before the sound step.
//!
//! `GameApp::update` runs this and then the sound-instance step, mirroring
//! C4Application executing `SoundSystem::Execute` after every pass. The
//! body is the whole per-pass update, so it lives beside its caller rather
//! than in the sound module its ordering once parked it in; separating the
//! sound state itself is tracked by clonk-org/clonk-rs#1233.

use super::*;

fn advance_after_releasing_snapshot_landscape<T, E>(
    snapshot: &mut SimulationSnapshot,
    advance: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    // Rendering is complete before the next fixed simulation step. Release
    // the previous presentation landscape before invoking `advance`, even if
    // that advance later fails, so its Surface8 Arc cannot force the first
    // terrain write to copy the complete multi-megabyte plane. The renderer
    // retains a lightweight dirty-lineage anchor of its own.
    snapshot.landscape = None;
    advance()
}

impl GameApp {
    pub(crate) fn update_before_sound_instance_step(&mut self) -> Result<(), EngineError> {
        let viewports = self.rendering.graphics.active_viewport_projections();
        let game_running = matches!(self.mode, AppMode::Running);
        if let Some(audio) = self.sound.context.as_ref() {
            audio
                .borrow_mut()
                .set_synchronous_sound_state(&viewports, game_running);
        }
        // Each C4Game::Execute attempt recomputes whether Control.Prepare is
        // blocked. The scheduler reads the reason after this method returns.
        self.waiting_network_control = None;
        self.guard_classic_global_gui_bootstrap()?;
        self.poll_lobby_preload()?;
        if let Some(network) = self.network.as_ref() {
            network.refresh_current_frame(self.current_network_input_frame());
        }
        if self.mode == AppMode::Loading && self.scenario_lifecycle.loading.is_some() {
            self.poll_loading()?;
            self.guard_classic_global_gui_bootstrap()?;
            if self.mode != AppMode::Loading {
                return Ok(());
            }
        }
        self.poll_startup_game_search()?;
        self.poll_scenario_selector_discovery()?;
        self.poll_update_check()?;
        self.poll_update_download()?;
        self.poll_background_save_jobs();
        self.poll_startup_irc()?;
        self.poll_classic_direct_reference_query()?;
        self.poll_startup_network_connection()?;
        self.poll_pending_network_host_preparation()?;
        self.poll_live_masterserver_signup()?;
        self.poll_league_player_auth()?;
        self.process_network_events()?;
        self.update_voice_chat();
        self.poll_blocking_resource_wait_at(Instant::now())?;
        // C4Network2::Execute probes the runtime status target before
        // Control.Prepare on every attempted frame, including halted frames.
        self.check_runtime_network_status_reached();
        // Native runs Network.Execute before Control.Prepare, so this scan
        // must still happen when the ready-control gate returns early.
        self.deactivate_inactive_network_clients();
        // The regular game execution is the second C4Network2::Execute path
        // besides OnSec1Timer; retire an outdated runtime dynamic before any
        // control preparation can observe it (src/C4Game.cpp:776-782).
        self.execute_network_before_control_prepare();
        if !matches!(self.mode, AppMode::Menu) {
            // Dropping the backdrop while loading or in-game (game over,
            // return to menu) frees its full-screen buffer during play.
            self.menu_backdrop_cache = StartupBackdropCache::default();
            self.startup.dialog_fade = None;
        }
        match self.mode {
            AppMode::Running => {
                self.reconcile_initial_scoreboard();
                // Loading callbacks and direct console/control entrypoints may
                // have produced process-local pacing requests before this
                // frame. Native makes them visible before the next Prepare.
                self.apply_engine_network_target_fps_requests()?;
                // Console/direct script execution remains available through
                // the outer app loop while HaltCount stops Game::Execute. A
                // queued PauseGame(true) must therefore be consumed before
                // the game-over and halt returns below; Toggle's own dialog
                // guard discards it while evaluation is visible.
                self.apply_engine_pause_game_requests();
                if self.dialogs.game_over.is_some() {
                    return Ok(());
                }
                if self.pending_league_end.is_some() {
                    return Ok(());
                }
                self.reconcile_message_board_input_dialog()?;
                // C4Game::Execute returns at its HaltCount gate while the
                // application continues polling input and drawing the frozen
                // frame. Network control reaches the same gate through its
                // synchronized status barrier.
                if self.runtime_halt_active() {
                    return Ok(());
                }
                // Prepare local network input every frame. C++ looks ahead by
                // PreSend frames before its cadence gate, so the aggregate is
                // normally complete by the frame that wants to execute it.
                self.flush_pending_remove_player_controls(true)?;
                if self.network.is_none() {
                    let control_rate = u64::try_from(self.engine.control_rate())
                        .unwrap_or(1)
                        .max(1);
                    if self.engine.frame().is_multiple_of(control_rate)
                        && !self.offline_control_input.is_empty()
                    {
                        let tick = u32::try_from(self.engine.frame()).unwrap_or(u32::MAX);
                        let controls = std::mem::take(&mut self.offline_control_input);
                        self.apply_ready_controls(tick, controls)?;
                    }
                }
                if self.network.is_some() {
                    let frame = self.engine.frame();
                    let local_activated = self
                        .network
                        .as_ref()
                        .and_then(|network| i32::try_from(network.local_client_id()).ok())
                        .is_some_and(|client_id| self.control_clients.is_activated(client_id));
                    let due_ticks = match self.network_control_clock.as_mut() {
                        Some(clock) => clock
                            .take_due_ticks(frame, local_activated)
                            .into_iter()
                            .filter_map(|tick| match Tick::try_from(tick) {
                                Ok(tick) => Some(tick),
                                Err(_) => {
                                    tracing::error!(tick, "negative network presend tick");
                                    None
                                }
                            })
                            .collect::<Vec<_>>(),
                        None => vec![u32::try_from(frame).unwrap_or(u32::MAX)],
                    };
                    let Some(network) = self.network.as_ref() else {
                        return Ok(());
                    };
                    let control_tick = match self.network_control_clock {
                        None => Some(u32::try_from(frame).unwrap_or(u32::MAX)),
                        Some(clock) => match clock.tick_for_frame(frame) {
                            None => None,
                            Some(tick) => match Tick::try_from(tick) {
                                Ok(tick) => Some(tick),
                                Err(_) => {
                                    tracing::error!(tick, "negative network control tick");
                                    return Ok(());
                                }
                            },
                        },
                    };
                    // Native records iWaitStart before DoInput and queued sync
                    // controls, but PackCompleteCtrl cannot race those sync
                    // controls on another thread. Capture that instant now and
                    // arm the worker only after sync controls have supplied the
                    // live rate and target FPS.
                    let control_tick_reached_at = control_tick.map(|_| tokio::time::Instant::now());
                    for tick in due_ticks {
                        network.finalize_tick(tick);
                    }

                    if let Some(tick) = control_tick {
                        let sync_controls = self.network_sync.take_exact(tick);
                        if !sync_controls.is_empty() {
                            let control_result =
                                self.apply_synchronized_controls(tick, sync_controls);
                            // ExecQueuedSyncCtrl runs before GetControl, so a
                            // synchronized SetPreSend affects this frame's
                            // subsequent performance calculation.
                            let pacing_result = self.apply_engine_network_target_fps_requests();
                            control_result?;
                            pacing_result?;
                            if let Some(network) = self.network.as_ref() {
                                network.reset_client_performance();
                            }
                        }
                        // ExecQueuedSyncCtrl has now supplied the live deadline
                        // inputs. Arm the host once with the earlier native wait
                        // start so an old TargetFPS cannot expire concurrently
                        // before this update reaches the worker.
                        if let (Some(network), Some(clock)) =
                            (self.network.as_ref(), self.network_control_clock)
                        {
                            network.control_tick_reached(
                                tick,
                                clock.control_rate(),
                                clock.target_fps(),
                                control_tick_reached_at
                                    .expect("control tick has a captured wait start"),
                            );
                        }

                        // Network mode mirrors C4Game::Execute's Prepare gate:
                        // CtrlReady(ControlTick) must succeed or the frame returns
                        // before control/simulation (src/C4GameControl.cpp:262-265;
                        // src/C4Game.cpp:786-797). The decoded packet order is
                        // authoritative, including interleaved SyncCheck packets.
                        let pending_player_resource =
                            self.network_ticks.ready.get(&tick).and_then(|controls| {
                                pending_admission_resource(
                                    &mut self.admission_resources,
                                    &self.control_clients,
                                    controls,
                                    &self.aborted_player_resource_joins,
                                )
                            });
                        if let Some(pending) = pending_player_resource {
                            self.waiting_network_control =
                                Some(NetworkControlWait::PlayerResource {
                                    resource_id: pending.core.id,
                                });
                            let player_name = pending
                                .player_name
                                .or_else(|| {
                                    self.control_player_infos
                                        .get(pending.info_id)
                                        .map(|player| {
                                            legacy_presentation_text(player.name.as_bytes())
                                        })
                                        .filter(|name| !name.is_empty())
                                })
                                .unwrap_or_else(|| {
                                    pending.core.filename.to_string_lossy().into_owned()
                                });
                            let template = self
                                .runtime_resource_text("IDS_NET_RES_PLRFILE", "player file for %s");
                            let display_name = format_resource_string(template, &[&player_name]);
                            self.begin_blocking_resource_wait_at(
                                BlockingResourceScope::PlayerJoin,
                                pending.core.id,
                                Some(pending.info_id),
                                display_name,
                                Instant::now(),
                            )?;
                            return Ok(());
                        }
                        let Some(controls) =
                            self.network_ticks.take_exact_if_ready(tick, |controls| {
                                preflight_admission_resources(
                                    &mut self.admission_resources,
                                    &self.control_clients,
                                    controls,
                                    &self.aborted_player_resource_joins,
                                )
                            })
                        else {
                            self.waiting_network_control =
                                Some(NetworkControlWait::ReadyTick(tick));
                            self.announce_network_stall(Instant::now())?;
                            return Ok(());
                        };
                        if let Some((stalled_since, _)) = self.network_stall_since.take() {
                            self.netplay_pacing.record_stall(stalled_since.elapsed());
                        }
                        // C++ CalcPerformance runs in GetControl, before the
                        // decoded controls execute. Freeze the receiver-local
                        // wait sample at the same consumption boundary.
                        let active_client_ids = self
                            .control_clients
                            .activated_client_ids()
                            .into_iter()
                            .filter_map(|client_id| ClientId::try_from(client_id).ok())
                            .collect();
                        let Some(network) = self.network.as_ref() else {
                            return Ok(());
                        };
                        let control_tick_cost =
                            network.control_tick_consumed(tick, active_client_ids);
                        if let Some(cost) = control_tick_cost {
                            self.netplay_pacing
                                .record_control_tick(cost.lateness_ms, cost.wait_attribution);
                        }
                        // C++ GetControl::CalcPerformance precedes decoded
                        // Control.Execute. Its flash therefore precedes (and
                        // may be replaced by) a SetPreSend flash in this batch.
                        let control_mode = self
                            .runtime_network_committed_control_mode
                            .or(self.runtime_network_control_mode)
                            .unwrap_or(0);
                        // Independent of the lateness branch below: the host can
                        // give up on this client's control without this tick
                        // also being measurably late here.
                        let discarded_tick = control_tick_cost
                            .and_then(|cost| cost.wait_attribution)
                            .filter(|attribution| attribution.discarded_recipient_control)
                            .map(|attribution| attribution.tick);
                        if let Some(discarded_tick) = discarded_tick {
                            self.note_discarded_control_tick(discarded_tick);
                        }
                        if let Some(clock) = self.network_control_clock.as_mut() {
                            if let Some(cost) = control_tick_cost {
                                clock.observe_control_send_time_ms(cost.send_time_ms);
                                if let Some(lateness_ms) = cost.lateness_ms {
                                    if let Some(attribution) = cost.wait_attribution {
                                        clock.observe_attributed_control_lateness_ms(
                                            lateness_ms,
                                            attribution,
                                        );
                                    } else {
                                        clock.observe_control_lateness_ms(lateness_ms);
                                    }
                                }
                            }
                            if let Some(change) = clock.calculate_performance_for_mode(control_mode)
                            {
                                self.apply_control_presend_change(change)?;
                            }
                        }
                        let control_result = self.apply_ready_controls(tick, controls);
                        if control_result.is_ok() {
                            if let Some(clock) = self.network_control_clock.as_mut() {
                                clock.complete_control_frame_at(frame);
                            }
                        }
                        // A request is an already-performed process-local
                        // mutation even if a later control reports an error.
                        let target_result = self.apply_engine_network_target_fps_requests();
                        control_result?;
                        target_result?;
                        // A client mismatch disconnects and returns to the menu.
                        // Do not execute one extra simulation frame after the
                        // ordered SyncCheck has changed session state.
                        if !matches!(self.mode, AppMode::Running) || self.network.is_none() {
                            return Ok(());
                        }
                    }
                }
                let replay_finished = if let Some(playback) = self.records.playback.as_mut() {
                    let frame = u32::try_from(self.engine.frame()).unwrap_or(u32::MAX);
                    let controls = playback
                        .take_controls(frame)
                        .into_iter()
                        .filter_map(network::network_control_for_packet)
                        .collect::<Vec<_>>();
                    let finished = playback.is_finished();
                    if !controls.is_empty() {
                        self.apply_ready_controls_from_queue(frame, controls, false)?;
                    }
                    finished
                } else {
                    false
                };
                if replay_finished {
                    self.records.playback = None;
                    self.engine.finish_replay()?;
                }
                self.apply_direct_film_view_projection();
                let _ = self.apply_pending_viewport_presentation_requests();
                let local_viewport_owners_before_tick = self.execute_local_team_selections()?;
                self.record_network_stats_control_frame();
                let player_infos_before_tick = self.player_info_ids_by_player();
                let tick_result =
                    advance_after_releasing_snapshot_landscape(&mut self.snapshot, || {
                        self.engine.tick()
                    });
                // C4Player::Eliminate queues synchronized client deactivation
                // during the simulation frame, before its presentation
                // snapshot is consumed (src/C4Player.cpp:2015-2037).
                self.flush_pending_client_updates();
                // PauseGame is a process-local console request emitted from
                // scripts during this tick. Native applies it immediately,
                // then observes HaltCount at the start of the next Execute.
                // Drain it even when the originating script reports an error.
                self.apply_engine_pause_game_requests();
                let target_result = self.apply_engine_network_target_fps_requests();
                self.snapshot = match tick_result {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        // Script errors are nonfatal at the application
                        // boundary. Re-snapshot the live engine so the redraw
                        // after this failed execute still has a landscape.
                        self.snapshot = self.engine.snapshot();
                        return Err(error);
                    }
                };
                target_result?;
                self.mirror_retired_player_info(&player_infos_before_tick);
                self.record_network_stats_frame();
                self.reconcile_message_board_input_dialog()?;
                let retired_viewport_owner = local_viewport_owners_before_tick
                    .iter()
                    .copied()
                    .find(|owner| self.engine.player(*owner).is_none())
                    .or_else(|| {
                        self.viewports
                            .physical_viewports
                            .iter()
                            .map(|viewport| viewport.displayed_player)
                            .find(|owner| {
                                *owner != OWNER_NONE && self.engine.player(*owner).is_none()
                            })
                    });
                let requested_removed_player = self.apply_pending_viewport_presentation_requests();
                let retired_viewport_owner = retired_viewport_owner.or(requested_removed_player);
                if let Some(owner) = retired_viewport_owner {
                    // Player::Execute retires at most one player per frame.
                    // Its C4PlayerList::Remove path closes all of that
                    // player's local viewports with one feedback request.
                    let _ = self.close_physical_viewports(owner, false, true);
                    self.remove_local_control_assignment(owner);
                    self.check_fullscreen_physical_viewports(true);
                }
                // Native MouseControl::Execute runs after Players/Script on
                // every successfully executed game frame. Re-run the last
                // clamped border move even when the OS emitted no new motion.
                let moving_drag_before_move = self.ingame_moving_drag_active();
                let selection_drag_before_move = self.ingame_selection_drag_active();
                let mut repeated_mouse_move = false;
                let player_view_scroll_owner =
                    if self.input_routing.live.ingame_edge_scroll.is_some() {
                        repeated_mouse_move = true;
                        self.advance_ingame_mouse_caption_lifetime();
                        self.apply_ingame_edge_scroll()?
                    } else if self.engine.frame().is_multiple_of(5) {
                        if self.initialize_ingame_mouse_center()? {
                            None
                        } else {
                            repeated_mouse_move = true;
                            self.advance_ingame_mouse_caption_lifetime();
                            self.refresh_ingame_edge_scroll_tick5()?
                        }
                    } else {
                        None
                    };
                if let Some(owner) = player_view_scroll_owner {
                    self.refresh_snapshot_after_player_view_scroll(owner);
                }
                if repeated_mouse_move {
                    if let Some(pointer) = self.input_routing.live.ingame_pointer {
                        self.advance_ingame_mouse_caption(
                            pointer,
                            moving_drag_before_move,
                            selection_drag_before_move,
                        );
                    }
                }
                self.refresh_ingame_region_drag_cursor_for_execute();
                // DragConstruct refreshes its ConstructionCheck phase during
                // MouseControl::Execute even without a new platform motion.
                self.refresh_construction_menu_drag();
                if let Some(network) = self.network.as_ref() {
                    network.refresh_current_frame(self.current_network_input_frame());
                }
                self.apply_game_goal_menu_requests()?;
                // Requests made by simulation scripts belong to a later
                // control tick, never to the frame that just executed them.
                self.flush_pending_remove_player_controls(false)?;
                self.handle_script_player_info_updates()?;
                self.presentation.frames_since_second =
                    self.presentation.frames_since_second.wrapping_add(1);
                self.apply_scoreboard_presentation_requests();
                self.handle_menu_requests()?;
                if self.snapshot.game_over && !self.game_over_handled {
                    self.handle_game_over()?;
                }
                self.refresh_object_menu();
                // C4Menu::Execute refills every active menu whenever
                // Game.iTick35 wraps, picking up joins, removals and changed
                // visibility even when no control just executed.
                if self.engine.frame().is_multiple_of(35) {
                    self.refresh_hostility_menus();
                    self.refresh_team_menus();
                }
                // C4RoundResults::EvaluatePlayer runs inside the simulation
                // when a player is evaluated, retired or eliminated, and
                // copies its BigIcon then (src/C4RoundResults.cpp:338-344).
                self.freeze_evaluated_player_big_icons();
                // Tooltip delay counter (C4Menu::Draw, C4Menu.cpp:805).
                for menu in self.ingame_menus.players.values_mut() {
                    menu.tick();
                }
                self.refresh_focus();
                self.update_audio();
                self.maybe_emit_sync_check();
            }
            AppMode::Loading => {
                self.poll_boot_loading();
            }
            AppMode::Menu => {
                if let Some(layout) = self.definition_selector_layout() {
                    if let Some(controller) = self.definition_selection.dialog.as_mut() {
                        controller.tick_scrollbar(&layout);
                    }
                }
                self.tick_scensel_scrollbar_arrow();
                self.menu_state.search_edit.tick_blink();
                let _ = self
                    .menu_state
                    .rename_edit
                    .as_mut()
                    .is_some_and(|rename| rename.edit.tick_blink());
                let _ = self
                    .startup
                    .crew_rename
                    .as_mut()
                    .is_some_and(|rename| rename.edit.tick_blink());
                let _ = self
                    .startup
                    .options_advanced_dialog
                    .as_mut()
                    .is_some_and(|dialog| dialog.controller.tick_edit_blink());
                let _ = self
                    .startup_network
                    .dialog
                    .as_mut()
                    .is_some_and(|dialog| dialog.tick_join_address_cursor());
                let fade_finished = self.sound.resume_frontend_after_fade
                    && self
                        .sound
                        .context
                        .as_ref()
                        .is_none_or(|audio| !audio.borrow().music_is_playing());
                if fade_finished {
                    self.sound.resume_frontend_after_fade = false;
                    self.ensure_menu_music();
                }
            }
        }
        Ok(())
    }
}
