use super::*;

impl GameApp {
    pub(crate) fn voice_chat_enabled(&self) -> bool {
        self.audio
            .as_ref()
            .is_some_and(|audio| audio.options.voice_enabled)
    }

    fn local_voice_identity(&self) -> Option<(i32, i32)> {
        let network = self.network.as_ref()?;
        let client_id = i32::try_from(network.local_client_id()).ok()?;
        crate::voice_chat::authenticated_selected_voice_crew(
            &self.snapshot,
            client_id,
            self.local_owner,
        )?;
        Some((client_id, self.local_owner))
    }

    pub(crate) fn handle_voice_key(&mut self, key: VirtualKeyCode, state: ElementState) -> bool {
        let configured_key = self
            .audio
            .as_ref()
            .map_or(VirtualKeyCode::Backquote, |audio| {
                audio.options.voice_push_to_talk
            });
        let keyboard_scope_available = !self.runtime_gui_has_keyboard_focus()
            && !self.runtime_top_default_dialog_is_exclusive();
        let eligible = self.mode == AppMode::Running
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
            self.mode == AppMode::Running && self.voice_chat_enabled() && keyboard_scope_available,
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
        if self.mode != AppMode::Running
            || !self.window_active
            || self
                .network
                .as_ref()
                .is_none_or(|network| !network.voice_available())
            || self.local_voice_identity().is_none()
        {
            return true;
        }
        if let Err(error) = self.voice_chat.start_capture(Some(key)) {
            tracing::warn!(%error, "push-to-talk could not open the microphone");
        }
        true
    }

    fn remove_voice_playback(&self, speakers: impl IntoIterator<Item = (i32, i32)>) {
        let Some(audio) = self.audio.as_ref() else {
            return;
        };
        for (client_id, player_id) in speakers {
            audio
                .system
                .remove_voice_stream(crate::voice_chat::voice_stream_id(client_id, player_id));
        }
    }

    pub(crate) fn update_voice_chat(&mut self) {
        let now = Instant::now();
        let received = self
            .network
            .as_mut()
            .map(NetworkManager::poll_voice_frames)
            .unwrap_or_default();
        let voice_available = self
            .network
            .as_ref()
            .is_some_and(NetworkManager::voice_available);
        if self.mode != AppMode::Running || !self.voice_chat_enabled() || !voice_available {
            let removed = self.voice_chat.clear();
            self.remove_voice_playback(removed);
            return;
        }

        let expired = self.voice_chat.expire_playback(now);
        self.remove_voice_playback(expired);
        let viewports = self.graphics.active_viewport_projections();
        let voice_volume = self
            .audio
            .as_ref()
            .map_or(0.0, |audio| audio.options.voice_volume);

        for frame in received {
            let Some(position) = i32::try_from(frame.client_id).ok().and_then(|client_id| {
                voice_source_position(&self.snapshot, client_id, frame.player_id)
            }) else {
                continue;
            };
            let Some(accepted) = self
                .voice_chat
                .accept_remote_frame(&self.snapshot, &frame, now)
            else {
                continue;
            };
            let (audibility, pan) =
                compute_object_positional_mix(position, &self.snapshot, &viewports);
            if let Some(audio) = self.audio.as_ref() {
                if accepted.reset_stream {
                    audio.system.remove_voice_stream(accepted.stream_id);
                }
                audio.system.queue_voice_stream_with_mix(
                    accepted.stream_id,
                    accepted.samples,
                    audibility * voice_volume,
                    pan,
                );
            }
        }

        let active_streams = self
            .voice_chat
            .remote_streams
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for (client_id, player_id) in active_streams {
            let Some(position) = voice_source_position(&self.snapshot, client_id, player_id) else {
                self.remove_voice_playback([(client_id, player_id)]);
                continue;
            };
            let (audibility, pan) =
                compute_object_positional_mix(position, &self.snapshot, &viewports);
            if let Some(audio) = self.audio.as_ref() {
                audio.system.update_voice_stream(
                    crate::voice_chat::voice_stream_id(client_id, player_id),
                    audibility * voice_volume,
                    pan,
                );
            }
        }

        let local_identity = (self.window_active
            && !self.runtime_gui_has_keyboard_focus()
            && !self.runtime_top_default_dialog_is_exclusive())
        .then(|| self.local_voice_identity())
        .flatten();
        if self.voice_chat.capture_active() && local_identity.is_none() {
            self.voice_chat.stop_capture();
        }
        let Some((client_id, player_id)) = local_identity else {
            return;
        };
        for captured in self.voice_chat.drain_captured_frames() {
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
