use super::*;

impl GameApp {
    pub(crate) fn voice_chat_enabled(&self) -> bool {
        self.sound
            .context
            .as_ref()
            .is_some_and(|audio| audio.borrow().options.voice_enabled)
    }

    /// The one player this client speaks as, or `None` when it is not a voice
    /// source at all. See `voice_chat::authenticated_selected_voice_crew` for
    /// the policy: an observer never opens the microphone, and a client with
    /// several local players speaks as `local_owner`.
    pub(crate) fn local_voice_identity(&self) -> Option<(i32, i32)> {
        let network = self.netplay.manager.as_ref()?;
        let client_id = i32::try_from(network.local_client_id()).ok()?;
        if self.mode == AppMode::Running {
            crate::voice_chat::authenticated_selected_voice_crew(
                &self.snapshot,
                client_id,
                self.players.local_owner,
            )?;
            return Some((client_id, self.players.local_owner));
        }
        (self.network_lobby_voice_active() && self.netplay.control_clients.contains(client_id))
            .then_some((client_id, crate::voice_chat::LOBBY_VOICE_PLAYER_ID))
    }

    fn network_lobby_voice_active(&self) -> bool {
        self.league_player_auth_lobby_active()
    }

    fn voice_chat_context_active(&self) -> bool {
        self.voice_chat_context().is_some()
    }

    fn voice_chat_context(&self) -> Option<crate::voice_chat::VoiceChatContext> {
        if self.mode == AppMode::Running {
            Some(crate::voice_chat::VoiceChatContext::Running)
        } else if self.network_lobby_voice_active() {
            Some(crate::voice_chat::VoiceChatContext::Lobby)
        } else {
            None
        }
    }

    fn voice_activation(&self) -> Option<crate::settings::VoiceActivation> {
        self.sound
            .context
            .as_ref()
            .and_then(|audio| audio.borrow().options.voice_activation())
    }

    /// What the mixer is playing, for the echo canceller to subtract. Every
    /// capture is handed it, whether or not echo cancellation is on at the
    /// time: the reference can only be bound while the microphone is opening,
    /// and withholding it would make this the one stage a player could not
    /// switch on mid-call. What it costs a capture that never uses it is one
    /// downmix per output frame.
    fn voice_echo_reference(&self) -> Option<clonk_audio::VoiceEchoReference> {
        self.sound
            .context
            .as_ref()
            .map(|audio| audio.borrow().system.voice_echo_reference())
    }

    pub(crate) fn handle_voice_key(&mut self, key: VirtualKeyCode, state: ElementState) -> bool {
        let configured_key = self
            .sound
            .context
            .as_ref()
            .map_or(VirtualKeyCode::Backquote, |audio| {
                audio.borrow().options.voice_push_to_talk
            });
        let keyboard_scope_available = !self.runtime_gui_has_keyboard_focus()
            && !self.runtime_top_default_dialog_is_exclusive();
        let eligible = self.voice_chat_context_active()
            && self.window_active
            && keyboard_scope_available
            && self
                .netplay
                .manager
                .as_ref()
                .is_some_and(NetworkManager::voice_available)
            && self.local_voice_identity().is_some();
        match crate::voice_chat::push_to_talk_action(
            self.voice_chat.capture_key(),
            configured_key,
            // A player on voice activation is not holding a key to talk, so the
            // configured key stays the game's to bind.
            self.voice_chat_context_active()
                && self.voice_chat_enabled()
                && keyboard_scope_available
                && self.voice_activation().is_none(),
            eligible,
            self.input_routing.engine_key_repeated,
            key,
            state,
        ) {
            crate::voice_chat::PushToTalkAction::Ignore => return false,
            crate::voice_chat::PushToTalkAction::Consume => return true,
            crate::voice_chat::PushToTalkAction::Stop => {
                if eligible && self.voice_chat_enabled() {
                    self.voice_chat.finish_capture_at(Instant::now());
                } else {
                    self.voice_chat.stop_capture();
                }
                return true;
            }
            crate::voice_chat::PushToTalkAction::Start => {}
        }
        if !self.voice_chat_context_active()
            || !self.window_active
            || self
                .netplay
                .manager
                .as_ref()
                .is_none_or(|network| !network.voice_available())
            || self.local_voice_identity().is_none()
        {
            return true;
        }
        let echo_reference = self.voice_echo_reference();
        let input_device = self
            .sound
            .context
            .as_ref()
            .and_then(|audio| audio.borrow().options.voice_input_device.clone());
        if let Err(error) =
            self.voice_chat
                .start_capture_on_device(Some(key), echo_reference, input_device)
        {
            tracing::warn!(%error, "push-to-talk could not open the microphone");
        }
        true
    }

    pub(crate) fn remove_voice_playback(&self, speakers: impl IntoIterator<Item = (i32, i32)>) {
        let Some(audio) = self.sound.context.as_ref() else {
            return;
        };
        let audio = audio.borrow();
        for (client_id, player_id) in speakers {
            audio
                .system
                .remove_voice_stream(crate::voice_chat::voice_stream_id(client_id, player_id));
        }
    }

    pub(crate) fn update_voice_chat(&mut self) {
        self.update_voice_chat_at(Instant::now());
    }

    pub(crate) fn update_voice_chat_at(&mut self, now: Instant) {
        self.update_voice_setup();
        self.service_voice_media_at(now);
    }

    fn service_voice_media_at(&mut self, now: Instant) {
        let Some(audio) = self.sound.context.as_ref() else {
            self.voice_chat.clear();
            return;
        };
        let context = self.voice_chat_context();
        let viewports = self.rendering.graphics.active_viewport_projections();
        let voice_volume = audio.borrow().options.voice_volume;
        let speakers = match context {
            Some(crate::voice_chat::VoiceChatContext::Running) => self
                .snapshot
                .players
                .iter()
                .filter_map(|player| {
                    let client_id = player.at_client.get();
                    let position = voice_source_position(&self.snapshot, client_id, player.id)?;
                    let (gain, pan) =
                        compute_object_positional_mix(position, &self.snapshot, &viewports);
                    Some(((client_id, player.id), (gain * voice_volume, pan)))
                })
                .collect(),
            Some(crate::voice_chat::VoiceChatContext::Lobby) => self
                .netplay
                .control_clients
                .snapshot()
                .into_iter()
                .map(|client| {
                    (
                        (client.client_id, crate::voice_chat::LOBBY_VOICE_PLAYER_ID),
                        (voice_volume, 0.0),
                    )
                })
                .collect(),
            None => std::collections::BTreeMap::new(),
        };
        let local_identity = (self.window_active
            && !self.runtime_gui_has_keyboard_focus()
            && !self.runtime_top_default_dialog_is_exclusive())
        .then(|| self.local_voice_identity())
        .flatten();
        let audio = audio.borrow();
        if audio.options.voice_enabled {
            audio.system.prepare_voice_output();
        }
        let policy = crate::voice_media::VoiceMediaPolicy {
            enabled: audio.options.voice_enabled,
            context,
            speakers,
            local_identity,
            activation: audio.options.voice_activation(),
            processing: audio.options.voice_processing(),
            input_device: audio.options.voice_input_device.clone(),
        };
        let audio_worker = audio.system.worker_handle();
        drop(audio);
        if let Some(network) = self.netplay.manager.as_mut() {
            let endpoint = network.take_voice_endpoint();
            self.voice_chat.update(policy, endpoint, audio_worker, now);
        } else {
            let removed = self.voice_chat.clear();
            self.remove_voice_playback(removed);
        }
    }
}

fn voice_source_position(
    snapshot: &SimulationSnapshot,
    client_id: i32,
    player_id: i32,
) -> Option<Vector2> {
    crate::voice_chat::authenticated_selected_voice_crew(snapshot, client_id, player_id)
        .map(|object| object.position)
}
