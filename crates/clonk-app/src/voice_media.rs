//! Presentation policy from the game; media servicing independent of simulation.

use std::collections::BTreeMap;
use std::time::Instant;

use clonk_audio::{AudioWorkerHandle, VoiceInputDeviceId, VoiceProcessingConfig};
use clonk_network::{ReceivedVoiceFrame, VoiceFrame, VoiceSendError};

use crate::settings::VoiceActivation;
use crate::voice_chat::{voice_stream_id, VoiceChatContext, VoiceChatState};

#[derive(Clone)]
pub(crate) struct VoiceMediaPolicy {
    pub(crate) enabled: bool,
    pub(crate) context: Option<VoiceChatContext>,
    pub(crate) speakers: BTreeMap<(i32, i32), (f32, f32)>,
    pub(crate) local_identity: Option<(i32, i32)>,
    pub(crate) activation: Option<VoiceActivation>,
    pub(crate) processing: VoiceProcessingConfig,
    pub(crate) input_device: Option<VoiceInputDeviceId>,
}

pub(crate) trait VoiceMediaTransport {
    fn available(&self) -> bool;
    fn receive(&mut self) -> Vec<ReceivedVoiceFrame>;
    fn send(&self, frame: VoiceFrame, captured_at: Instant) -> Result<(), VoiceSendError>;
}

impl VoiceMediaTransport for crate::network::NetworkVoiceEndpoint {
    fn available(&self) -> bool {
        self.is_available()
    }
    fn receive(&mut self) -> Vec<ReceivedVoiceFrame> {
        self.receive()
    }
    fn send(&self, frame: VoiceFrame, captured_at: Instant) -> Result<(), VoiceSendError> {
        self.try_send_at(frame, captured_at)
    }
}

pub(crate) fn service_voice_media(
    state: &mut VoiceChatState,
    policy: &VoiceMediaPolicy,
    transport: &mut (impl VoiceMediaTransport + ?Sized),
    audio: &AudioWorkerHandle,
    now: Instant,
) {
    let received = transport.receive();
    let remove = |speakers: Vec<(i32, i32)>| {
        for (client, player) in speakers {
            audio.remove_voice_stream(voice_stream_id(client, player));
        }
    };
    if !transport.available() {
        remove(state.clear());
        return;
    }
    remove(state.reconcile_context(policy.context));
    if policy.context.is_none() {
        return;
    }
    state.set_processing(policy.processing);
    remove(state.expire_playback(now));
    for received in received {
        let frame = received.frame;
        let Some(client_id) = i32::try_from(frame.client_id).ok() else {
            continue;
        };
        if !policy.speakers.contains_key(&(client_id, frame.player_id)) {
            continue;
        }
        if let Some(accepted) = state.accept_authorized_remote_frame(&frame, received.received_at) {
            if accepted.reset_stream {
                audio.remove_voice_stream(accepted.stream_id);
            }
        }
    }
    let active_streams = state.remote_streams.keys().copied().collect::<Vec<_>>();
    for (client_id, player_id) in active_streams {
        let stream_id = voice_stream_id(client_id, player_id);
        let Some(&(volume, pan)) = policy.speakers.get(&(client_id, player_id)) else {
            state.discard_remote_playback(client_id, player_id);
            audio.remove_voice_stream(stream_id);
            continue;
        };
        let playback = audio.voice_stream_stats(stream_id);
        let rate = state.update_playout_clock(client_id, player_id, now, playback.queued_duration);
        audio.set_voice_playout_rate(stream_id, rate);
        let queued = playback.queued_frames;
        let available = clonk_audio::DEFAULT_VOICE_BUFFERED_FRAMES
            .saturating_sub(1)
            .saturating_sub(queued);
        for frame in state.drain_remote_playout(client_id, player_id, now, available, queued) {
            audio.queue_voice_stream_with_mix(stream_id, frame.samples, volume, pan);
        }
        audio.update_voice_stream(stream_id, volume, pan);
    }
    // `enabled` is the microphone opt-in and nothing else: a player who never
    // took it still hears the players who did. This is the one gate every
    // capture below passes, so a disabled player's microphone stays closed.
    let Some((client_id, player_id)) = policy.local_identity.filter(|_| policy.enabled) else {
        state.stop_capture();
        return;
    };
    let echo_reference = Some(audio.voice_echo_reference());
    if policy.activation.is_none() && state.voice_activated_capture_requested() {
        state.stop_capture();
    } else if let Err(error) =
        state.reconcile_capture_device_at(policy.input_device.clone(), echo_reference.clone(), now)
    {
        tracing::warn!(%error, "the selected microphone could not be opened");
        if policy.activation.is_some() {
            return;
        }
    }
    if policy.activation.is_some() {
        if let Err(error) = state.start_voice_activated_capture_on_device_at(
            echo_reference,
            policy.input_device.clone(),
            now,
        ) {
            tracing::warn!(%error, "voice activation could not open the microphone");
        }
    }
    for captured in state.drain_captured_frames(policy.activation.as_ref()) {
        let frame = match VoiceFrame::outbound(
            player_id,
            captured.stream_epoch,
            captured.sequence,
            captured
                .payload
                .map_or_else(Vec::new, |payload| payload.to_vec()),
        ) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::error!(%error, "captured voice frame violated its wire bound");
                continue;
            }
        };
        if transport.send(frame, captured.captured_at).is_ok() && captured.payload.is_some() {
            state.note_local_frame(client_id, player_id, now);
        }
    }
}
