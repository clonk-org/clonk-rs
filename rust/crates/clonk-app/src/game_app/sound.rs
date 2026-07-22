//! `impl GameApp` — sound & music methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    fn prepare_runtime_music_flash(
        &self,
        enabled: bool,
    ) -> Result<Option<RuntimeFlashMessage>, EngineError> {
        let (charset, message_text) = self
            .runtime_flash_resources()
            .map(|resources| (resources.charset, resources.music_on_off(enabled)))
            .map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeFlashResources {
                        detail: error.to_string(),
                    },
                ))
            })?;
        self.prepare_runtime_flash_message(&message_text, charset)
            .map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeFlashResources {
                        detail: error.to_string(),
                    },
                ))
            })
    }

    fn set_runtime_music_playback(&mut self, enabled: bool) {
        self.runtime_music_enabled = enabled;
        if enabled {
            if let Some(path) = self
                .active_scenario
                .as_ref()
                .and_then(|scenario| scenario.path.clone())
            {
                self.play_scenario_audio(&path);
            } else {
                self.play_sandbox_audio();
            }
        } else if let Some(audio) = self.audio.as_mut() {
            audio.stop_music();
        }
    }

    /// Running global F3 calls `ToggleOnOff(false)`: it changes
    /// `Game.IsMusicEnabled`/playback without changing RXMusic.
    pub(crate) fn toggle_runtime_music_playback(&mut self) -> Result<(), EngineError> {
        let enabled = self
            .audio
            .as_ref()
            .map(|audio| !audio.music_is_playing())
            .ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the running MusicToggle action",
                    },
                ))
            })?;
        let flash_message = self.prepare_runtime_music_flash(enabled)?;
        self.set_runtime_music_playback(enabled);
        self.runtime_flash_message = flash_message;
        Ok(())
    }

    /// In-game Options calls default `ToggleOnOff(true)`, changing RXMusic
    /// and the current game's playback flag together (C4MainMenu.cpp:837-840).
    pub(crate) fn toggle_music_option(&mut self) -> Result<(), EngineError> {
        let next_enabled = self
            .audio
            .as_ref()
            .map(|audio| !audio.options.music_enabled)
            .ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the in-game Options music action",
                    },
                ))
            })?;
        let flash_message = self.prepare_runtime_music_flash(next_enabled)?;

        let enabled = self
            .audio
            .as_mut()
            .map(|audio| {
                audio.options.music_enabled = !audio.options.music_enabled;
                audio.options.music_enabled
            })
            .expect("audio availability preflighted above");
        self.persist_audio_option("Music", enabled);
        self.set_runtime_music_playback(enabled);
        self.runtime_flash_message = flash_message;
        Ok(())
    }

    /// `Application.SoundSystem->ToggleOnOff()` (C4MainMenu.cpp:842-845).
    pub(crate) fn toggle_sound_option(&mut self) -> Result<(), EngineError> {
        let enabled = {
            let audio = self.audio.as_mut().ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the running SoundToggle action",
                    },
                ))
            })?;
            // C4SoundSystem::ToggleOnOff changes only RXSound. The next sound
            // update releases mixer channels while retaining logical instances;
            // starts made while muted are retained channel-less as well.
            audio.options.sound_enabled = !audio.options.sound_enabled;
            audio.options.sound_enabled
        };
        self.persist_audio_option("Sound", enabled);
        Ok(())
    }

    fn persist_audio_option(&self, key: &'static str, enabled: bool) {
        let Some(paths) = self.app_paths.as_ref() else {
            return;
        };
        // C4ConfigSound::CompileFunc serializes RXSound/RXMusic/FEMusic/
        // FESamples as the external [Sound] Sound/Music/MenuMusic/MenuSound
        // keys. Rust saves eagerly; a write failure must not roll back the
        // live toggle, just as C++ ignores a later Config.Save failure during
        // normal application shutdown.
        if let Err(error) = persist_config_value(paths, "Sound", key, enabled.to_string()) {
            tracing::warn!(%error, key, "failed to persist audio option");
        }
    }

    pub(crate) fn set_frontend_music_option(&mut self, enabled: bool) -> Result<(), EngineError> {
        self.resume_frontend_music_after_fade = false;
        let audio = self.audio.as_mut().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup frontend-music option",
                },
            ))
        })?;
        self.frontend_music_attempted_for_entry = true;
        audio.options.menu_music_enabled = enabled;
        if enabled {
            match audio.play_frontend_music() {
                Ok(true) => {}
                Ok(false) => audio.stop_music(),
                Err(error) => {
                    tracing::warn!(%error, "failed to start frontend music after option change");
                    audio.stop_music();
                }
            }
        } else {
            audio.stop_music();
        }
        Ok(())
    }

    pub(crate) fn toggle_frontend_music_option(&mut self) -> Result<bool, EngineError> {
        let enabled = self
            .audio
            .as_ref()
            .map(|audio| !audio.options.menu_music_enabled)
            .ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the startup MusicToggle action",
                    },
                ))
            })?;
        self.set_frontend_music_option(enabled)?;
        self.persist_audio_option("MenuMusic", enabled);
        Ok(enabled)
    }

    pub(crate) fn set_frontend_sound_option(&mut self, enabled: bool) -> Result<(), EngineError> {
        let audio = self.audio.as_mut().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::RuntimeAudioSystem {
                    action: "the startup frontend-sound option",
                },
            ))
        })?;
        audio.options.menu_sound_enabled = enabled;
        Ok(())
    }

    pub(crate) fn toggle_frontend_sound_option(&mut self) -> Result<bool, EngineError> {
        let enabled = self
            .audio
            .as_ref()
            .map(|audio| !audio.options.menu_sound_enabled)
            .ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the startup SoundToggle action",
                    },
                ))
            })?;
        self.set_frontend_sound_option(enabled)?;
        self.persist_audio_option("MenuSound", enabled);
        Ok(enabled)
    }

    fn note_control_message_sound(&mut self, client_id: i32, muted: bool) {
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.note_client_sound(client_id, muted);
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.note_client_sound(client_id, muted);
        }
    }

    pub(crate) fn play_control_message_sound(&mut self, name: &str) -> bool {
        let Some(audio) = self.audio.as_mut() else {
            return false;
        };
        for candidate in [name.to_string(), format!("{name}.ogg"), format!("{name}.mp3")] {
            match audio.try_start_sound(
                &candidate,
                None,
                100,
                false,
                true,
                None,
                &self.snapshot,
                &[],
            ) {
                Ok(true) => return true,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(sound = %candidate, %error, "failed to play control message sound");
                }
            }
        }
        false
    }

    pub(crate) fn execute_message_control_with_sound_at<F>(
        &mut self,
        control: MessageControlData,
        now: Instant,
        mut play_sound: F,
    ) -> MessageControlOutcome
    where
        F: FnMut(&mut Self, &str) -> bool,
    {
        let mut outcome = MessageControlOutcome::default();
        let sender = (control.player >= 0)
            .then(|| self.engine.player(control.player))
            .flatten()
            .map(|player| {
                let color = player.color().map_or(0, |color| {
                    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
                });
                (
                    player.id(),
                    player.at_client().get(),
                    c4_presentation_text(player.name()),
                    color,
                )
            });
        if sender
            .as_ref()
            .is_some_and(|(_, at_client, _, _)| *at_client != control.by_client)
        {
            outcome.rejected = true;
            return outcome;
        }

        let message = legacy_presentation_text(control.message.as_bytes());
        let mut check_alert = false;
        match control.message_type {
            MESSAGE_TYPE_NORMAL | MESSAGE_TYPE_ME => {
                let (line, color) = match sender.as_ref() {
                    Some((_, _, name, color)) => {
                        let line = match (control.message_type, self.display_flags.white_chat) {
                            (MESSAGE_TYPE_NORMAL, true) => {
                                format!("<c {color:x}><{name}></c> {message}")
                            }
                            (MESSAGE_TYPE_NORMAL, false) => {
                                format!("<c {color:x}><{name}> {message}")
                            }
                            (MESSAGE_TYPE_ME, true) => {
                                format!("<c {color:x}> * {name}</c> {message}")
                            }
                            (MESSAGE_TYPE_ME, false) => {
                                format!("<c {color:x}> * {name} {message}")
                            }
                            _ => unreachable!(),
                        };
                        (line, *color)
                    }
                    None => {
                        let nick = self
                            .control_clients
                            .state(control.by_client)
                            .map(|client| legacy_presentation_text(client.nick.as_bytes()))
                            .unwrap_or_else(|| "???".to_string());
                        let white = self.control_message_has_lobby() && self.white_lobby_chat;
                        let line = match (control.message_type, white) {
                            (MESSAGE_TYPE_NORMAL, true) => {
                                format!("<{nick}> <c ffffff>{message}")
                            }
                            (MESSAGE_TYPE_NORMAL, false) => format!("<{nick}> {message}"),
                            (MESSAGE_TYPE_ME, true) => {
                                format!(" * {nick} <c ffffff>{message}")
                            }
                            (MESSAGE_TYPE_ME, false) => format!(" * {nick} {message}"),
                            _ => unreachable!(),
                        };
                        (line, 0x00ff_ffff)
                    }
                };
                self.append_control_message_log(line, color, Some(control.by_client));
                outcome.displayed = true;
                check_alert = true;
            }
            MESSAGE_TYPE_SAY => {
                outcome.say_displayed = self.engine.execute_message_control_say(&control);
            }
            MESSAGE_TYPE_TEAM => {
                if let Some((sender_id, _, name, color)) = sender.as_ref() {
                    let local_players = self.engine.snapshot().hud.local_players;
                    let visible = local_players.iter().any(|local_id| {
                        self.engine.player(*local_id).is_some_and(|local| {
                            !local.is_hostile_towards(*sender_id)
                                && self
                                    .engine
                                    .player(*sender_id)
                                    .is_some_and(|sender| !sender.is_hostile_towards(*local_id))
                        })
                    });
                    if visible {
                        let line = if self.display_flags.white_chat {
                            format!("<c {color:x}>{{{name}}}</c> {message}")
                        } else {
                            format!("<c {color:x}>{{{name}}} {message}")
                        };
                        self.append_control_message_log(line, CONTROL_LOG_COLOR, None);
                        outcome.displayed = true;
                    }
                    check_alert = true;
                } else if self.control_message_has_lobby() {
                    let local_client = self
                        .network
                        .as_ref()
                        .and_then(|network| i32::try_from(network.local_client_id()).ok())
                        .unwrap_or(0);
                    if !self
                        .control_player_infos
                        .has_same_team_players(control.by_client, local_client)
                    {
                        return outcome;
                    }
                    let nick = self
                        .control_clients
                        .state(control.by_client)
                        .map(|client| legacy_presentation_text(client.nick.as_bytes()))
                        .unwrap_or_else(|| "???".to_string());
                    let line = if self.white_lobby_chat {
                        format!("{{{nick}}} <c ffffff>{message}")
                    } else {
                        format!("{{{nick}}} {message}")
                    };
                    self.append_control_message_log(
                        line,
                        0x00ff_ffff,
                        Some(control.by_client),
                    );
                    outcome.displayed = true;
                    check_alert = true;
                } else {
                    check_alert = true;
                }
            }
            MESSAGE_TYPE_PRIVATE => {
                let Some((_, _, name, color)) = sender.as_ref() else {
                    return outcome;
                };
                let visible = self
                    .engine
                    .snapshot()
                    .hud
                    .local_players
                    .contains(&control.to_player);
                if visible {
                    let line = if self.display_flags.white_chat {
                        format!("<c {color:x}>[{name}]</c> {message}")
                    } else {
                        format!("<c {color:x}>[{name}] {message}")
                    };
                    self.append_control_message_log(line, CONTROL_LOG_COLOR, None);
                    outcome.displayed = true;
                }
                check_alert = true;
            }
            MESSAGE_TYPE_SOUND => {
                if self.control_clients.state(control.by_client).is_none()
                    || !self.control_messages.try_allow_sound_at(now)
                {
                    return outcome;
                }
                let muted = self.control_messages.is_muted(control.by_client);
                if !muted {
                    outcome.sound_attempted = true;
                    outcome.sound_played = play_sound(self, &message);
                }
                if muted || outcome.sound_played {
                    if self.control_message_has_lobby() {
                        self.note_control_message_sound(control.by_client, muted);
                        outcome.lobby_sound = true;
                    }
                }
            }
            MESSAGE_TYPE_ALERT => {
                outcome.attention_requested = self.request_control_message_attention();
            }
            MESSAGE_TYPE_SYSTEM => {
                if control.by_client == 0 {
                    self.append_control_message_log(
                        format!("Network: {message}"),
                        CONTROL_LOG_COLOR,
                        None,
                    );
                    outcome.displayed = true;
                }
            }
            _ => {}
        }

        if check_alert && self.control_message_mentions_local_nick(&control) {
            outcome.attention_requested = self.request_control_message_attention();
        }
        outcome
    }

    pub(crate) fn play_options_sound(&mut self, sound: clonk_frontend::startup_options_dlg::SoundSheetSound) {
        self.play_ui_sound(match sound {
            clonk_frontend::startup_options_dlg::SoundSheetSound::ArrowHit => "ArrowHit",
            clonk_frontend::startup_options_dlg::SoundSheetSound::Command => "Command",
        });
    }

    pub(crate) fn play_options_test_sound(
        &mut self,
        sound: clonk_frontend::startup_options_dlg::SoundSheetSound,
    ) {
        self.play_global_sound_effect(match sound {
            clonk_frontend::startup_options_dlg::SoundSheetSound::ArrowHit => "ArrowHit",
            clonk_frontend::startup_options_dlg::SoundSheetSound::Command => "Command",
        });
    }

    pub(crate) fn process_about_dialog_actions_with_sound(
        &mut self,
        actions: Vec<clonk_frontend::startup_about_dlg::AboutDlgAction>,
        play_activation_sound: bool,
    ) -> Result<(), EngineError> {
        use clonk_frontend::startup_about_dlg::AboutDlgAction;

        for action in actions {
            match action {
                AboutDlgAction::Back => {
                    self.begin_startup_dialog_fade(StartupDialog::MainMenu);
                    self.show_main_menu();
                }
                AboutDlgAction::CheckForUpdates => self.open_launcher_update_dialog()?,
                AboutDlgAction::PageChanged(_) if play_activation_sound => {
                    self.play_ui_sound("Click");
                }
                AboutDlgAction::PageChanged(_) => {}
                AboutDlgAction::LicenseChanged(_) => self.play_ui_sound("Command"),
                AboutDlgAction::GuiSound(sound) => self.play_ui_sound(match sound {
                    clonk_frontend::startup_about_dlg::AboutDlgSound::ArrowHit => "ArrowHit",
                    clonk_frontend::startup_about_dlg::AboutDlgSound::Command => "Command",
                }),
            }
        }
        Ok(())
    }

    pub(crate) fn update_before_sound_instance_step(&mut self) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.poll_lobby_preload()?;
        if let Some(network) = self.network.as_ref() {
            network.refresh_current_frame(self.current_network_input_frame());
        }
        if self.mode == AppMode::Loading && self.loading_state.is_some() {
            self.poll_loading()?;
            self.guard_classic_global_gui_bootstrap()?;
            if self.mode != AppMode::Loading {
                return Ok(());
            }
        }
        self.poll_startup_game_search()?;
        self.poll_scenario_selector_discovery()?;
        self.poll_startup_irc()?;
        self.poll_classic_direct_reference_query()?;
        self.poll_startup_network_connection()?;
        self.poll_live_masterserver_signup()?;
        self.poll_league_player_auth()?;
        self.process_network_events()?;
        self.poll_blocking_resource_wait_at(Instant::now())?;
        // C4Network2::Execute probes the runtime status target before
        // Control.Prepare on every attempted frame, including halted frames.
        self.check_runtime_network_status_reached();
        // Native runs Network.Execute before Control.Prepare, so this scan
        // must still happen when the ready-control gate returns early.
        self.deactivate_inactive_network_clients();
        if !matches!(self.mode, AppMode::Menu) {
            // Whatever happens while loading or in-game (game over, return
            // to menu) must not replay a stale pre-game menu frame; dropping
            // the backdrop also frees its full-screen buffer during play.
            self.menu_frame_cache = None;
            self.menu_backdrop_cache = StartupBackdropCache::default();
            self.startup_dialog_fade = None;
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
                if self.game_over_dialog.is_some() {
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
                    if self.engine.frame() % control_rate == 0
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
                    let control_tick_reached_at =
                        control_tick.map(|_| tokio::time::Instant::now());
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
                            if let Err(error) = control_result {
                                return Err(error);
                            }
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
                        let pending_player_resource = self
                            .network_ticks
                            .ready
                            .get(&tick)
                            .and_then(|controls| {
                                pending_admission_resource(
                                    &mut self.admission_resources,
                                    &self.control_clients,
                                    controls,
                                    &self.aborted_player_resource_joins,
                                )
                        });
                        if let Some(pending) = pending_player_resource {
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
                            let template = self.runtime_resource_text(
                                "IDS_NET_RES_PLRFILE",
                                "player file for %s",
                            );
                            let display_name =
                                format_resource_string(template, &[&player_name]);
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
                            return Ok(());
                        };
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
                        network.control_tick_consumed(tick, active_client_ids);
                        // C++ GetControl::CalcPerformance precedes decoded
                        // Control.Execute. Its flash therefore precedes (and
                        // may be replaced by) a SetPreSend flash in this batch.
                        if let Some(change) = self
                            .network_control_clock
                            .as_mut()
                            .and_then(NetworkControlClock::calculate_performance)
                        {
                            self.apply_control_presend_change(change)?;
                        }
                        let control_result = self.apply_ready_controls(tick, controls);
                        if control_result.is_ok() {
                            if let Some(clock) = self.network_control_clock.as_mut() {
                                clock.complete_control_frame();
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
                let replay_finished = if let Some(playback) = self.control_playback.as_mut() {
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
                    self.control_playback = None;
                    self.engine.finish_replay()?;
                }
                self.apply_direct_film_view_projection();
                let _ = self.apply_pending_viewport_presentation_requests();
                let local_viewport_owners_before_tick = self.execute_local_team_selections()?;
                self.record_network_stats_control_frame();
                let tick_result = self.engine.tick();
                // PauseGame is a process-local console request emitted from
                // scripts during this tick. Native applies it immediately,
                // then observes HaltCount at the start of the next Execute.
                // Drain it even when the originating script reports an error.
                self.apply_engine_pause_game_requests();
                let target_result = self.apply_engine_network_target_fps_requests();
                self.snapshot = tick_result?;
                target_result?;
                self.record_network_stats_frame();
                self.reconcile_message_board_input_dialog()?;
                let retired_viewport_owner = local_viewport_owners_before_tick
                    .iter()
                    .copied()
                    .find(|owner| self.engine.player(*owner).is_none())
                    .or_else(|| {
                        self.physical_viewports
                            .iter()
                            .map(|viewport| viewport.displayed_player)
                            .find(|owner| {
                                *owner != OWNER_NONE && self.engine.player(*owner).is_none()
                            })
                    });
                let requested_removed_player =
                    self.apply_pending_viewport_presentation_requests();
                let retired_viewport_owner =
                    retired_viewport_owner.or(requested_removed_player);
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
                let player_view_scrolled = if self.ingame_edge_scroll.is_some() {
                    repeated_mouse_move = true;
                    self.advance_ingame_mouse_caption_lifetime();
                    self.apply_ingame_edge_scroll()?
                } else if self.engine.frame() % 5 == 0 {
                    if self.initialize_ingame_mouse_center()? {
                        false
                    } else {
                        repeated_mouse_move = true;
                        self.advance_ingame_mouse_caption_lifetime();
                        self.refresh_ingame_edge_scroll_tick5()?
                    }
                } else {
                    false
                };
                if player_view_scrolled {
                    let audio = std::mem::take(&mut self.snapshot.audio);
                    self.snapshot = self.engine.snapshot();
                    self.snapshot.audio = audio;
                }
                if repeated_mouse_move {
                    if let Some(pointer) = self.ingame_pointer {
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
                self.frames_since_second = self.frames_since_second.wrapping_add(1);
                self.apply_scoreboard_presentation_requests();
                self.handle_menu_requests()?;
                if self.snapshot.game_over && !self.game_over_handled {
                    self.handle_game_over()?;
                }
                self.refresh_object_menu();
                // C4Menu::Execute refills permanent hostility pages whenever
                // Game.iTick35 wraps, picking up joins, removals and changed
                // visibility even when no hostility control just executed.
                if self.engine.frame() % 35 == 0 {
                    self.refresh_hostility_menus();
                }
                // Tooltip delay counter (C4Menu::Draw, C4Menu.cpp:805).
                for menu in self.ingame_menu.values_mut() {
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
                let definition_scroll_changed = self
                    .definition_selector_layout()
                    .and_then(|layout| {
                        self.definition_selector.as_mut().map(|controller| {
                            let before = controller.scroll_y();
                            controller.tick_scrollbar(&layout);
                            controller.scroll_y() != before
                        })
                    })
                    .unwrap_or(false);
                let scrollbar_changed = self.tick_scensel_scrollbar_arrow();
                let search_blink_changed = self.menu_state.search_edit.tick_blink();
                let rename_blink_changed = self
                    .menu_state
                    .rename_edit
                    .as_mut()
                    .is_some_and(|rename| rename.edit.tick_blink());
                let crew_rename_blink_changed = self
                    .startup_crew_rename
                    .as_mut()
                    .is_some_and(|rename| rename.edit.tick_blink());
                let advanced_blink_changed = self
                    .startup_options_advanced_dialog
                    .as_mut()
                    .is_some_and(|dialog| dialog.controller.tick_edit_blink());
                let netdlg_blink_changed = self
                    .startup_network_dialog
                    .as_mut()
                    .is_some_and(|dialog| dialog.tick_join_address_cursor());
                if definition_scroll_changed
                    || scrollbar_changed
                    || search_blink_changed
                    || rename_blink_changed
                    || crew_rename_blink_changed
                    || advanced_blink_changed
                    || netdlg_blink_changed
                {
                    self.mark_menu_dirty();
                }
                let fade_finished = self.resume_frontend_music_after_fade
                    && self
                        .audio
                        .as_ref()
                        .is_none_or(|audio| !audio.music_is_playing());
                if fade_finished {
                    self.resume_frontend_music_after_fade = false;
                    self.ensure_menu_music();
                }
            }
        }
        Ok(())
    }

    pub(crate) fn update_audio(&mut self) {
        // Script Music(nil/name) mutates Game.IsMusicEnabled before asking
        // the music system to stop/play. Fold that flag in command order so
        // a SetPlayList restart sees the state at its exact event position.
        let mut runtime_music_enabled = self.runtime_music_enabled;
        let viewports = self.graphics.active_viewport_projections();
        let speech_outcomes = if let Some(audio) = self.audio.as_mut() {
            audio.process_audio_with_viewports(
                &self.snapshot,
                &viewports,
                &mut runtime_music_enabled,
            )
        } else {
            let mut outcomes = Vec::new();
            for event in &self.snapshot.audio {
                match event {
                    AudioCommand::PlaySpeech {
                        fallback: Some(fallback),
                        ..
                    } => outcomes.push(SpeechPlaybackOutcome::Rejected(fallback.clone())),
                    AudioCommand::PlayMusic { .. } => runtime_music_enabled = true,
                    AudioCommand::StopMusic => runtime_music_enabled = false,
                    _ => {}
                }
            }
            outcomes
        };
        if !speech_outcomes.is_empty() {
            self.snapshot.hud.messages =
                self.engine.apply_speech_playback_outcomes(speech_outcomes);
        }
        self.runtime_music_enabled = runtime_music_enabled;
        // C4MusicSystem::Execute chooses another enabled song whenever a
        // non-looping track ends. A pending worker load counts as playback so
        // this cannot spawn replacement workers every frame.
        let restart_music = self.runtime_music_enabled
            && self
                .audio
                .as_ref()
                .is_some_and(|audio| !audio.music_is_playing());
        if restart_music {
            if let Some(path) = self
                .active_scenario
                .as_ref()
                .and_then(|scenario| scenario.path.clone())
            {
                self.play_scenario_audio(&path);
            } else {
                self.play_sandbox_audio();
            }
        }
    }

    pub(crate) fn update_sound_instances_for_current_mode(&mut self) {
        let game_running = matches!(self.mode, AppMode::Running);
        let viewports = self.graphics.active_viewport_projections();
        if let Some(audio) = self.audio.as_mut() {
            audio.pump_queued_music_starts();
            audio.update_channels(&self.snapshot, &viewports, game_running);
        }
    }

    pub(crate) fn play_game_over_sound_events(&mut self, events: Vec<GameOverSound>) {
        for event in events {
            self.play_ui_sound(match event {
                GameOverSound::ArrowHit => "ArrowHit",
                GameOverSound::Click => "Click",
            });
        }
    }

    pub(crate) fn play_game_option_sound_events(&mut self, events: Vec<GameOptionSound>) {
        for event in events {
            self.play_ui_sound(match event {
                GameOptionSound::ArrowHit => "ArrowHit",
                GameOptionSound::Click => "Click",
                GameOptionSound::Connect => "Connect",
            });
        }
    }

    pub(crate) fn play_input_dialog_sound_events(&mut self, events: Vec<InputDialogSound>) {
        for event in events {
            self.play_ui_sound(match event {
                InputDialogSound::ArrowHit => "ArrowHit",
                InputDialogSound::Click => "Click",
            });
        }
    }

    pub(crate) fn play_message_dialog_sound_events(
        &mut self,
        events: Vec<clonk_frontend::message_dialog::MessageDialogSound>,
    ) {
        for event in events {
            self.play_ui_sound(match event {
                clonk_frontend::message_dialog::MessageDialogSound::ArrowHit => "ArrowHit",
                clonk_frontend::message_dialog::MessageDialogSound::Click => "Click",
            });
        }
    }

    pub(crate) fn fade_out_game_music(&mut self) -> bool {
        let fading = self
            .audio
            .as_mut()
            .is_some_and(|audio| audio.fade_out_music(GAME_MUSIC_FADE_OUT_MS));
        self.resume_frontend_music_after_fade = fading;
        fading
    }

    pub(crate) fn reconstruct_music_system_at_preinit(&mut self) {
        // The old fade belongs to the object destroyed by MusicSystem.emplace.
        // Its replacement is immediately eligible for DoStartup playback.
        self.resume_frontend_music_after_fade = false;
        if let Some(audio) = self.audio.as_mut() {
            audio.reset_music_system_generation(self.app_paths.as_ref());
        }
    }

    pub(crate) fn begin_frontend_music_entry(&mut self) {
        self.frontend_music_attempted_for_entry = false;
        if let Some(audio) = self.audio.as_mut() {
            lock_unpoisoned(&audio.music_control).most_recently_played = None;
            if self.resume_frontend_music_after_fade {
                audio.prepare_frontend_music();
            }
        }
        if self.resume_frontend_music_after_fade {
            return;
        }
        self.ensure_menu_music();
    }

    pub(crate) fn ensure_menu_music(&mut self) {
        if !matches!(self.mode, AppMode::Menu)
            || self.frontend_music_attempted_for_entry
            || self.resume_frontend_music_after_fade
        {
            return;
        }
        self.frontend_music_attempted_for_entry = true;
        if let Some(audio) = self.audio.as_mut() {
            audio.prepare_frontend_music();
            if !audio.menu_music_enabled() {
                self.resume_frontend_music_after_fade = false;
                audio.stop_music();
                return;
            }
            self.resume_frontend_music_after_fade = false;
            match audio.play_default_music(false) {
                Ok(true) => {}
                Ok(false) => audio.stop_music(),
                Err(err) => {
                    tracing::warn!(error = %err, "failed to start menu music");
                    audio.stop_music();
                }
            }
        }
    }

    pub(crate) fn play_ui_sound(&mut self, name: &str) {
        let game_running = matches!(self.mode, AppMode::Running);
        #[cfg(test)]
        self.ui_sound_log.push(name.to_owned());
        if let Some(audio) = self.audio.as_mut() {
            audio.play_gui_sound(name, game_running, &self.snapshot);
        }
    }

    /// Calls native `StartSoundEffect` without C4GUI's outer FESamples gate.
    pub(crate) fn play_global_sound_effect(&mut self, name: &str) {
        let game_running = matches!(self.mode, AppMode::Running);
        #[cfg(test)]
        self.ui_sound_log.push(name.to_owned());
        let Some(audio) = self.audio.as_mut() else {
            return;
        };
        if let Err(error) = audio.try_start_global_effect(name, game_running, &self.snapshot) {
            tracing::error!(sound = %name, %error, "failed to play global sound effect");
        }
    }

    fn play_viewport_feedback_sound(&mut self) {
        self.play_viewport_feedback_sound_for_game_state(matches!(self.mode, AppMode::Running));
    }

    pub(crate) fn play_viewport_feedback_sound_for_game_state(&mut self, game_running: bool) {
        #[cfg(test)]
        self.ui_sound_log.push("CloseViewport".to_owned());
        let Some(audio) = self.audio.as_mut() else {
            return;
        };
        if let Err(error) =
            audio.try_start_global_effect("CloseViewport", game_running, &self.snapshot)
        {
            tracing::error!(sound = "CloseViewport", %error, "failed to play viewport feedback");
        }
    }

    pub(crate) fn play_scenario_audio(&mut self, path: &Path) {
        self.resume_frontend_music_after_fade = false;
        let runtime_music_enabled = self.runtime_music_enabled;
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(Some(path));
            if !runtime_music_enabled {
                audio.stop_music();
                return;
            }
            // C4MusicSystem::PlayScenarioMusic calls Play() with its
            // non-looping default. Do not repeat one asset forever.
            match audio.play_default_music(false) {
                Ok(true) => {}
                Ok(false) => audio.stop_music(),
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "failed to load music"
                    );
                }
            }
        }
    }

    pub(crate) fn play_sandbox_audio(&mut self) {
        self.resume_frontend_music_after_fade = false;
        let runtime_music_enabled = self.runtime_music_enabled;
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(None);
            audio.set_music_playlist(None);
            if !runtime_music_enabled {
                audio.stop_music();
                return;
            }
            if let Err(err) = audio.play_music(sandbox_music_bytes(), true) {
                tracing::warn!(error = %err, "failed to start sandbox music");
                audio.stop_music();
            }
        }
    }
}
