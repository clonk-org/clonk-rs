//! `impl GameApp` — sound & music methods.
//!
//! This remains an `impl GameApp` module so it can share private application
//! state. Extracting independently owned sound and music state is tracked by
//! clonk-org/clonk-rs#1233.

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
        self.sound.runtime_music_enabled = enabled;
        if enabled {
            if let Some(path) = self
                .scenario_lifecycle
                .active
                .as_ref()
                .and_then(|scenario| scenario.path.clone())
            {
                self.sound.play_scenario_audio(&path);
            } else {
                self.sound.play_sandbox_audio();
            }
        } else if let Some(audio) = self.sound.context.as_ref() {
            audio.borrow_mut().stop_music();
        }
    }

    /// Running global F3 calls `ToggleOnOff(false)`: it changes
    /// `Game.IsMusicEnabled`/playback without changing RXMusic.
    pub(crate) fn toggle_runtime_music_playback(&mut self) -> Result<(), EngineError> {
        let enabled = self
            .sound
            .context
            .as_ref()
            .map(|audio| !audio.borrow().music_is_playing())
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
            .sound
            .context
            .as_ref()
            .map(|audio| !audio.borrow().options.music_enabled)
            .ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the in-game Options music action",
                    },
                ))
            })?;
        let flash_message = self.prepare_runtime_music_flash(next_enabled)?;

        let enabled = self
            .sound
            .context
            .as_ref()
            .map(|audio| {
                let mut audio = audio.borrow_mut();
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
            let audio = self.sound.context.as_ref().ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the running SoundToggle action",
                    },
                ))
            })?;
            let mut audio = audio.borrow_mut();
            // C4SoundSystem::ToggleOnOff changes only RXSound. The next sound
            // update releases mixer channels while retaining logical instances;
            // starts made while muted are retained channel-less as well.
            audio.options.sound_enabled = !audio.options.sound_enabled;
            audio.options.sound_enabled
        };
        self.persist_audio_option("Sound", enabled);
        Ok(())
    }

    fn persist_audio_option(&mut self, key: &'static str, enabled: bool) {
        // C4ConfigSound::CompileFunc serializes RXSound/RXMusic/FEMusic/
        // FESamples as the external [Sound] Sound/Music/MenuMusic/MenuSound
        // keys. `C4SoundSystem::ToggleOnOff` changes only the in-memory flag;
        // the file is written when the Options dialog closes or on a clean
        // shutdown, so a toggle flipped back and forth costs no disk writes and
        // an aborted run discards it.
        self.config.deferred.set("Sound", key, enabled.to_string());
    }

    pub(crate) fn toggle_frontend_music_option(&mut self) -> Result<bool, EngineError> {
        let enabled = self
            .sound
            .context
            .as_ref()
            .map(|audio| !audio.borrow().options.menu_music_enabled)
            .ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the startup MusicToggle action",
                    },
                ))
            })?;
        self.sound.set_frontend_music_option(enabled)?;
        self.persist_audio_option("MenuMusic", enabled);
        Ok(enabled)
    }

    pub(crate) fn toggle_frontend_sound_option(&mut self) -> Result<bool, EngineError> {
        let enabled = self
            .sound
            .context
            .as_ref()
            .map(|audio| !audio.borrow().options.menu_sound_enabled)
            .ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeAudioSystem {
                        action: "the startup SoundToggle action",
                    },
                ))
            })?;
        self.sound.set_frontend_sound_option(enabled)?;
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
        let Some(audio) = self.sound.context.as_ref() else {
            return false;
        };
        let mut audio = audio.borrow_mut();
        for candidate in [
            name.to_string(),
            format!("{name}.ogg"),
            format!("{name}.mp3"),
        ] {
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
                        let line = match (
                            control.message_type,
                            self.rendering.display_flags.white_chat,
                        ) {
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
                        let line = if self.rendering.display_flags.white_chat {
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
                        .players
                        .infos
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
                    self.append_control_message_log(line, 0x00ff_ffff, Some(control.by_client));
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
                    let line = if self.rendering.display_flags.white_chat {
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
                if (muted || outcome.sound_played) && self.control_message_has_lobby() {
                    self.note_control_message_sound(control.by_client, muted);
                    outcome.lobby_sound = true;
                }
            }
            MESSAGE_TYPE_ALERT => {
                outcome.attention_requested = self.request_control_message_attention();
            }
            MESSAGE_TYPE_SYSTEM if control.by_client == 0 => {
                self.append_control_message_log(
                    format!("Network: {message}"),
                    CONTROL_LOG_COLOR,
                    None,
                );
                outcome.displayed = true;
            }
            _ => {}
        }

        if check_alert && self.control_message_mentions_local_nick(&control) {
            outcome.attention_requested = self.request_control_message_attention();
        }
        outcome
    }

    pub(crate) fn play_options_sound(
        &mut self,
        sound: clonk_frontend::startup_options_dlg::SoundSheetSound,
    ) {
        self.play_ui_sound(match sound {
            clonk_frontend::startup_options_dlg::SoundSheetSound::ArrowHit => "ArrowHit",
            clonk_frontend::startup_options_dlg::SoundSheetSound::Command => "Command",
            clonk_frontend::startup_options_dlg::SoundSheetSound::Click => "Click",
        });
    }

    pub(crate) fn play_options_test_sound(
        &mut self,
        sound: clonk_frontend::startup_options_dlg::SoundSheetSound,
    ) {
        self.play_global_sound_effect(match sound {
            clonk_frontend::startup_options_dlg::SoundSheetSound::ArrowHit => "ArrowHit",
            clonk_frontend::startup_options_dlg::SoundSheetSound::Command => "Command",
            clonk_frontend::startup_options_dlg::SoundSheetSound::Click => "Click",
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
                // `C4StartupAboutDlg::OnUpdateBtn` runs a manual check
                // (`C4StartupAboutDlg.cpp:377-380`).
                AboutDlgAction::CheckForUpdates => self.check_for_updates(false)?,
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

    /// The live `(player, player info)` links a tick may retire.
    pub(crate) fn player_info_ids_by_player(&self) -> Vec<(i32, i32)> {
        self.engine
            .players()
            .map(|player| (player.id(), player.player_info_id()))
            .collect()
    }

    /// Whether a host readmits the profile an eliminated player retired with
    /// (clonk-org/clonk-rs#240). The oracle has no such policy, so an unset
    /// `Config.Network.NoRejoinAfterElimination` keeps its behaviour.
    pub(crate) fn rejoin_after_elimination_allowed(&self) -> bool {
        self.network_rejoin_after_elimination_allowed
            .unwrap_or_else(|| {
                !native_config_text(
                    &load_native_config_bytes(self.app_paths.as_ref()),
                    "Network",
                    "NoRejoinAfterElimination",
                )
                .as_deref()
                .map(parse_config_bool)
                .unwrap_or(false)
            })
    }

    /// `C4PlayerList::Retire` routes through `Remove`, which calls
    /// `C4PlayerInfo::SetRemoved` before the live player is deleted, releasing
    /// the profile for a later `ActivateNewPlayer`
    /// (src/C4PlayerList.cpp:219-267,398-409). The engine owns retirement, so
    /// mirror it into the synchronized registry once the tick has applied it.
    /// The caller captures the links before the simulation tick, so a player
    /// that disappears during `C4PlayerList::Retire` is distinguishable from
    /// a joined lobby row that has not entered this process's simulation yet.
    /// Native only retires the former (src/C4PlayerList.cpp:398-409).
    pub(crate) fn mirror_retired_player_info(&mut self, players_before_tick: &[(i32, i32)]) {
        let mut retired_player_infos = players_before_tick
            .iter()
            .filter(|(player, _)| self.engine.player(*player).is_none())
            .map(|(_, player_info)| *player_info)
            .filter(|player_info| *player_info != 0)
            .collect::<Vec<_>>();
        retired_player_infos.sort_unstable();
        retired_player_infos.dedup();
        if retired_player_infos.is_empty() {
            return;
        }

        let game_part_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
        // Resolve the rejoin policy here so it and the elimination records it
        // gates always share one lifetime: every path that replaces the
        // registry drops both together.
        let rejoin_allowed = self.rejoin_after_elimination_allowed();
        self.players
            .infos
            .set_rejoin_after_elimination_allowed(rejoin_allowed);
        let mut changed = false;
        let mut changed_remote_clients = Vec::new();
        for player_info in retired_player_infos {
            let client_id = self.players.infos.client_id_for_info(player_info);
            if self
                .players
                .infos
                .mark_retired(player_info, game_part_frame)
            {
                changed = true;
                changed_remote_clients.extend(client_id.filter(|client_id| *client_id != 0));
            }
        }
        if changed {
            self.prune_host_local_alternate_colors();
            self.publish_current_host_player_infos();
        }
        if matches!(self.runtime_network_role(), RuntimeNetworkRole::Host) {
            changed_remote_clients.sort_unstable();
            changed_remote_clients.dedup();
            for client_id in changed_remote_clients {
                let Some(info) = self.players.infos.client_packet(client_id) else {
                    continue;
                };
                // C++ mutates the shared PlayerInfo inside Player::Remove on
                // every peer. Rust owns that mirror at the app boundary, so
                // send the host's already-applied remote packet to repair a
                // client that otherwise retains Joined and refuses the file.
                if let Some(Err(error)) = self
                    .network
                    .as_ref()
                    .map(|network| network.broadcast_preexecuted_player_info(info, Vec::new()))
                {
                    tracing::error!(%client_id, %error, "failed to broadcast retired remote PlayerInfo");
                }
            }
        }
    }

    pub(crate) fn update_audio(&mut self) {
        // Script Music(nil/name) mutates Game.IsMusicEnabled before asking
        // the music system to stop/play. Fold that flag in command order so
        // a SetPlayList restart sees the state at its exact event position.
        let mut runtime_music_enabled = self.sound.runtime_music_enabled;
        let viewports = self.rendering.graphics.active_viewport_projections();
        let speech_outcomes = if let Some(audio) = self.sound.context.as_ref() {
            let mut audio = audio.borrow_mut();
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
        self.sound.runtime_music_enabled = runtime_music_enabled;
        // C4MusicSystem::Execute chooses another enabled song whenever a
        // non-looping track ends. A pending worker load counts as playback so
        // this cannot spawn replacement workers every frame.
        let restart_music = self.sound.runtime_music_enabled
            && self
                .sound
                .context
                .as_ref()
                .is_some_and(|audio| !audio.borrow().music_is_playing());
        if restart_music {
            if let Some(path) = self
                .scenario_lifecycle
                .active
                .as_ref()
                .and_then(|scenario| scenario.path.clone())
            {
                self.sound.play_scenario_audio(&path);
            } else {
                self.sound.play_sandbox_audio();
            }
        }
    }

    pub(crate) fn update_sound_instances_for_current_mode(&mut self) {
        let game_running = matches!(self.mode, AppMode::Running);
        let viewports = self.rendering.graphics.active_viewport_projections();
        if let Some(audio) = self.sound.context.as_ref() {
            let mut audio = audio.borrow_mut();
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

    pub(crate) fn reconstruct_music_system_at_preinit(&mut self) {
        // The old fade belongs to the object destroyed by MusicSystem.emplace.
        // Its replacement is immediately eligible for DoStartup playback.
        self.sound.resume_frontend_after_fade = false;
        if let Some(audio) = self.sound.context.as_ref() {
            audio
                .borrow_mut()
                .reset_music_system_generation(self.app_paths.as_ref());
        }
    }

    pub(crate) fn begin_frontend_music_entry(&mut self) {
        self.sound.frontend_attempted_for_entry = false;
        if let Some(audio) = self.sound.context.as_ref() {
            let mut audio = audio.borrow_mut();
            lock_unpoisoned(&audio.music_control).most_recently_played = None;
            if self.sound.resume_frontend_after_fade {
                audio.prepare_frontend_music();
            }
        }
        if self.sound.resume_frontend_after_fade {
            return;
        }
        self.ensure_menu_music();
    }

    pub(crate) fn ensure_menu_music(&mut self) {
        if !matches!(self.mode, AppMode::Menu)
            || self.sound.frontend_attempted_for_entry
            || self.sound.resume_frontend_after_fade
        {
            return;
        }
        self.sound.frontend_attempted_for_entry = true;
        self.sound.resume_frontend_after_fade = false;
        if let Some(audio) = self.sound.context.as_ref() {
            let mut audio = audio.borrow_mut();
            audio.prepare_frontend_music();
            if !audio.menu_music_enabled() {
                audio.stop_music();
                return;
            }
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
        self.sound.ui_log.push(name.to_owned());
        if let Some(audio) = self.sound.context.as_ref() {
            audio
                .borrow_mut()
                .play_gui_sound(name, game_running, &self.snapshot);
        }
    }

    /// Calls native `StartSoundEffect` without C4GUI's outer FESamples gate.
    pub(crate) fn play_global_sound_effect(&mut self, name: &str) {
        let game_running = matches!(self.mode, AppMode::Running);
        #[cfg(test)]
        self.sound.ui_log.push(name.to_owned());
        let Some(audio) = self.sound.context.as_ref() else {
            return;
        };
        if let Err(error) =
            audio
                .borrow_mut()
                .try_start_global_effect(name, game_running, &self.snapshot)
        {
            tracing::error!(sound = %name, %error, "failed to play global sound effect");
        }
    }

    pub(crate) fn play_viewport_feedback_sound_for_game_state(&mut self, game_running: bool) {
        #[cfg(test)]
        self.sound.ui_log.push("CloseViewport".to_owned());
        let Some(audio) = self.sound.context.as_ref() else {
            return;
        };
        if let Err(error) = audio.borrow_mut().try_start_global_effect(
            "CloseViewport",
            game_running,
            &self.snapshot,
        ) {
            tracing::error!(sound = "CloseViewport", %error, "failed to play viewport feedback");
        }
    }
}
