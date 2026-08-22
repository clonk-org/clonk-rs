use super::*;

impl GameApp {
    pub(crate) fn voice_chat_enabled(&self) -> bool {
        self.audio
            .as_ref()
            .is_some_and(|audio| audio.borrow().options.voice_enabled)
    }

    /// The one player this client speaks as, or `None` when it is not a voice
    /// source at all. See `voice_chat::authenticated_selected_voice_crew` for
    /// the policy: an observer never opens the microphone, and a client with
    /// several local players speaks as `local_owner`.
    pub(crate) fn local_voice_identity(&self) -> Option<(i32, i32)> {
        let network = self.network.as_ref()?;
        let client_id = i32::try_from(network.local_client_id()).ok()?;
        if self.mode == AppMode::Running {
            crate::voice_chat::authenticated_selected_voice_crew(
                &self.snapshot,
                client_id,
                self.local_owner,
            )?;
            return Some((client_id, self.local_owner));
        }
        (self.network_lobby_voice_active() && self.control_clients.contains(client_id))
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

    fn authenticated_lobby_voice_client(&self, client_id: i32, player_id: i32) -> bool {
        self.network_lobby_voice_active()
            && player_id == crate::voice_chat::LOBBY_VOICE_PLAYER_ID
            && self.control_clients.contains(client_id)
    }

    fn voice_activation(&self) -> Option<crate::settings::VoiceActivation> {
        self.audio
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
        self.audio
            .as_ref()
            .map(|audio| audio.borrow().system.voice_echo_reference())
    }

    pub(crate) fn handle_voice_key(&mut self, key: VirtualKeyCode, state: ElementState) -> bool {
        let configured_key = self
            .audio
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
                .network
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
            self.engine_key_repeated,
            key,
            state,
        ) {
            crate::voice_chat::PushToTalkAction::Ignore => return false,
            crate::voice_chat::PushToTalkAction::Consume => return true,
            crate::voice_chat::PushToTalkAction::Stop => {
                self.voice_chat.stop_capture();
                return true;
            }
            crate::voice_chat::PushToTalkAction::Start => {}
        }
        if !self.voice_chat_context_active()
            || !self.window_active
            || self
                .network
                .as_ref()
                .is_none_or(|network| !network.voice_available())
            || self.local_voice_identity().is_none()
        {
            return true;
        }
        let echo_reference = self.voice_echo_reference();
        let input_device = self
            .audio
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

    /// Voice activation opens the microphone exactly where push-to-talk is
    /// allowed to keep one open — the same eligibility, resolved by the caller
    /// — only without a key to hold. It never opens one on the push-to-talk
    /// default, and it closes a capture the player stranded by switching back:
    /// no key owns that capture, so nothing else ever would.
    fn update_voice_activated_capture(&mut self, voice_activated: bool, now: Instant) {
        let echo_reference = self.voice_echo_reference();
        let input_device = self
            .audio
            .as_ref()
            .and_then(|audio| audio.borrow().options.voice_input_device.clone());
        if !voice_activated {
            if self.voice_chat.voice_activated_capture_requested() {
                self.voice_chat.stop_capture();
            } else if let Err(error) =
                self.voice_chat
                    .reconcile_capture_device_at(input_device, echo_reference, now)
            {
                tracing::warn!(%error, "the selected microphone could not be opened");
            }
            return;
        }
        if let Err(error) = self.voice_chat.reconcile_capture_device_at(
            input_device.clone(),
            echo_reference.clone(),
            now,
        ) {
            tracing::warn!(%error, "the selected microphone could not be opened");
            return;
        }
        if let Err(error) = self.voice_chat.start_voice_activated_capture_on_device_at(
            echo_reference,
            input_device,
            now,
        ) {
            tracing::warn!(%error, "voice activation could not open the microphone");
        }
    }

    pub(crate) fn remove_voice_playback(&self, speakers: impl IntoIterator<Item = (i32, i32)>) {
        let Some(audio) = self.audio.as_ref() else {
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
        let received = self
            .network
            .as_mut()
            .map(NetworkManager::poll_voice_frames)
            .unwrap_or_default();
        let voice_available = self
            .network
            .as_ref()
            .is_some_and(NetworkManager::voice_available);
        if !self.voice_chat_enabled() || !voice_available {
            let removed = self.voice_chat.clear();
            self.remove_voice_playback(removed);
            return;
        }
        let context = self.voice_chat_context();
        let removed = self.voice_chat.reconcile_context(context);
        self.remove_voice_playback(removed);
        let Some(context) = context else {
            return;
        };

        // Whatever the player has set right now, handed to a capture that may
        // already be open: the stages are switched, not reopened.
        if let Some(audio) = self.audio.as_ref() {
            let audio = audio.borrow();
            self.voice_chat
                .set_processing(audio.options.voice_processing());
        }

        let expired = self.voice_chat.expire_playback(now);
        self.remove_voice_playback(expired);
        let viewports = self.graphics.active_viewport_projections();
        // This is already a gain, not the classic `0..=100` UI value: `1.0`
        // is unity and `2.0` is the voice-only boost ceiling. The mixer accepts
        // that same contract, so applying another normalization here would
        // make the upper half of the slider quieter instead of louder.
        let voice_volume = self
            .audio
            .as_ref()
            .map_or(0.0, |audio| audio.borrow().options.voice_volume);

        for frame in received {
            let Some(client_id) = i32::try_from(frame.client_id).ok() else {
                continue;
            };
            let accepted = if context == crate::voice_chat::VoiceChatContext::Running {
                voice_source_position(&self.snapshot, client_id, frame.player_id).and_then(|_| {
                    self.voice_chat
                        .accept_remote_frame(&self.snapshot, &frame, now)
                })
            } else if self.authenticated_lobby_voice_client(client_id, frame.player_id) {
                self.voice_chat.accept_authorized_remote_frame(&frame, now)
            } else {
                None
            };
            let Some(accepted) = accepted else {
                continue;
            };
            if accepted.reset_stream {
                if let Some(audio) = self.audio.as_ref() {
                    let audio = audio.borrow();
                    audio.system.remove_voice_stream(accepted.stream_id);
                }
            }
        }

        let active_streams = self
            .voice_chat
            .remote_streams
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for (client_id, player_id) in active_streams {
            let mix = if context == crate::voice_chat::VoiceChatContext::Running {
                voice_source_position(&self.snapshot, client_id, player_id).map(|position| {
                    compute_object_positional_mix(position, &self.snapshot, &viewports)
                })
            } else {
                self.authenticated_lobby_voice_client(client_id, player_id)
                    .then_some((1.0, 0.0))
            };
            let Some((audibility, pan)) = mix else {
                self.voice_chat
                    .discard_remote_playback(client_id, player_id);
                self.remove_voice_playback([(client_id, player_id)]);
                continue;
            };
            if let Some(audio) = self.audio.as_ref() {
                let audio = audio.borrow();
                let stream_id = crate::voice_chat::voice_stream_id(client_id, player_id);
                let queued_frames = audio.system.voice_stream_stats(stream_id).queued_frames;
                let maximum_queued_frames =
                    clonk_audio::DEFAULT_VOICE_BUFFERED_FRAMES.saturating_sub(1);
                let available_frames = maximum_queued_frames.saturating_sub(queued_frames);
                for frame in self.voice_chat.drain_remote_playout(
                    client_id,
                    player_id,
                    now,
                    available_frames,
                    queued_frames,
                ) {
                    audio.system.queue_voice_stream_with_mix(
                        stream_id,
                        frame.samples,
                        audibility * voice_volume,
                        pan,
                    );
                }
                audio
                    .system
                    .update_voice_stream(stream_id, audibility * voice_volume, pan);
            }
        }

        let local_identity = (self.window_active
            && !self.runtime_gui_has_keyboard_focus()
            && !self.runtime_top_default_dialog_is_exclusive())
        .then(|| self.local_voice_identity())
        .flatten();
        if local_identity.is_none() {
            self.voice_chat.stop_capture();
        }
        let Some((client_id, player_id)) = local_identity else {
            return;
        };
        let activation = self.voice_activation();
        self.update_voice_activated_capture(activation.is_some(), now);
        for captured in self.voice_chat.drain_captured_frames(activation.as_ref()) {
            let frame = match clonk_network::VoiceFrame::outbound(
                player_id,
                captured.stream_epoch,
                captured.sequence,
                captured.payload.to_vec(),
            ) {
                Ok(frame) => frame,
                Err(error) => {
                    tracing::error!(%error, "captured voice frame violated its wire bound");
                    continue;
                }
            };
            let sent = self
                .network
                .as_ref()
                .is_some_and(|network| network.try_send_voice(frame).is_ok());
            if sent {
                self.voice_chat.note_local_frame(client_id, player_id, now);
            }
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
