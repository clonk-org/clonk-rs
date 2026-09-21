//! Game-thread controls and presentation snapshots for the independent worker.

use crate::network::NetworkVoiceEndpoint;
#[cfg(test)]
use crate::voice_chat::voice_stream_id;
use crate::voice_chat::{RemoteVoicePlayoutStats, VoiceChatState};
#[cfg(test)]
use crate::voice_media::service_voice_media;
use crate::voice_media::VoiceMediaPolicy;
use crate::voice_worker::VoiceMediaWorker;
use clonk_audio::{AudioWorkerHandle, VoiceCaptureError, VoiceEchoReference, VoiceInputDeviceId};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use winit::keyboard::KeyCode;

#[derive(Default)]
pub(crate) struct VoiceChatService {
    worker: Option<VoiceMediaWorker>,
    endpoint: Option<NetworkVoiceEndpoint>,
    pending_key: Option<KeyCode>,
    muted: BTreeSet<i32>,
    retry_at: Option<Instant>,
    #[cfg(test)]
    manual: Option<VoiceChatState>,
}

impl VoiceChatService {
    pub(crate) fn new() -> Self {
        #[cfg(test)]
        {
            Self {
                manual: Some(VoiceChatState::default()),
                ..Self::default()
            }
        }
        #[cfg(not(test))]
        {
            Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_source_opener<F, S>(opener: F) -> Self
    where
        F: FnMut(clonk_audio::VoiceCaptureOptions) -> Result<S, VoiceCaptureError> + 'static,
        S: crate::voice_chat::VoiceFrameSource + 'static,
    {
        Self {
            manual: Some(VoiceChatState::with_source_opener(opener)),
            ..Self::default()
        }
    }

    pub(crate) fn update(
        &mut self,
        policy: VoiceMediaPolicy,
        endpoint: Option<NetworkVoiceEndpoint>,
        audio: AudioWorkerHandle,
        now: Instant,
    ) {
        let replacing = self.endpoint.is_some() && endpoint.is_some();
        if let Some(endpoint) = endpoint.as_ref() {
            self.endpoint = Some(endpoint.clone());
        }
        #[cfg(test)]
        if let Some(state) = self.manual.as_mut() {
            if replacing {
                for (client, player) in state.clear() {
                    audio.remove_voice_stream(voice_stream_id(client, player));
                }
            }
            if let Some(transport) = self.endpoint.as_mut() {
                service_voice_media(state, &policy, transport, &audio, now);
            }
            return;
        }
        if replacing {
            self.pending_key = None;
        }
        if let Some(worker) = self.worker.as_ref() {
            worker.update(
                policy,
                endpoint.map(|endpoint| Box::new(endpoint) as _),
                audio,
            );
            return;
        }
        if self.retry_at.is_some_and(|retry| now < retry) {
            return;
        }
        let Some(endpoint) = self.endpoint.as_ref() else {
            return;
        };
        match VoiceMediaWorker::spawn(VoiceChatState::default, policy, endpoint.clone(), audio) {
            Ok(worker) => {
                for &client in &self.muted {
                    worker.set_client_muted(client, true);
                }
                if let Some(key) = self.pending_key {
                    worker.request_capture(Some(key));
                }
                self.worker = Some(worker);
                self.retry_at = None;
            }
            Err(error) => {
                tracing::error!(%error, "voice media worker could not start; retrying shortly");
                self.retry_at = now.checked_add(Duration::from_secs(1));
            }
        }
    }

    pub(crate) fn start_capture(
        &mut self,
        key: Option<KeyCode>,
        echo: Option<VoiceEchoReference>,
    ) -> Result<(), VoiceCaptureError> {
        self.start_capture_on_device(key, echo, None)
    }
    pub(crate) fn start_capture_on_device(
        &mut self,
        key: Option<KeyCode>,
        echo: Option<VoiceEchoReference>,
        input: Option<VoiceInputDeviceId>,
    ) -> Result<(), VoiceCaptureError> {
        #[cfg(test)]
        if let Some(state) = self.manual.as_mut() {
            return state.start_capture_on_device(key, echo, input);
        }
        drop((echo, input));
        self.pending_key = key;
        if let Some(worker) = self.worker.as_ref() {
            worker.request_capture(key);
        }
        Ok(())
    }
    pub(crate) fn stop_capture(&mut self) {
        #[cfg(test)]
        if let Some(state) = self.manual.as_mut() {
            state.stop_capture();
            return;
        }
        self.pending_key = None;
        if let Some(worker) = self.worker.as_ref() {
            worker.request_capture(None);
        }
    }
    pub(crate) fn finish_capture_at(&mut self, at: Instant) {
        #[cfg(test)]
        if let Some(state) = self.manual.as_mut() {
            state.finish_capture_at(at);
            return;
        }
        self.pending_key = None;
        if let Some(worker) = &self.worker {
            worker.finish_capture_at(at);
        }
    }
    pub(crate) fn capture_key(&self) -> Option<KeyCode> {
        #[cfg(test)]
        if let Some(state) = self.manual.as_ref() {
            return state.capture_key();
        }
        self.worker
            .as_ref()
            .map_or(self.pending_key, VoiceMediaWorker::capture_key)
    }
    pub(crate) fn capture_active(&self) -> bool {
        #[cfg(test)]
        if let Some(state) = self.manual.as_ref() {
            return state.capture_active();
        }
        self.worker
            .as_ref()
            .is_some_and(VoiceMediaWorker::capture_active)
    }
    pub(crate) fn active_speakers(&self, now: Instant) -> Vec<(i32, i32)> {
        #[cfg(test)]
        if let Some(state) = self.manual.as_ref() {
            return state.active_speakers(now);
        }
        self.worker
            .as_ref()
            .map(|worker| worker.active_speakers(now))
            .unwrap_or_default()
    }
    pub(crate) fn remote_playout_stats(&self, client: i32, player: i32) -> RemoteVoicePlayoutStats {
        #[cfg(test)]
        if let Some(state) = self.manual.as_ref() {
            return state.remote_playout_stats(client, player);
        }
        self.worker
            .as_ref()
            .map(|worker| worker.remote_playout_stats(client, player))
            .unwrap_or_default()
    }
    pub(crate) fn has_remote_stream(&self, client: i32, player: i32) -> bool {
        #[cfg(test)]
        if let Some(state) = self.manual.as_ref() {
            return state.remote_streams.contains_key(&(client, player));
        }
        self.worker
            .as_ref()
            .is_some_and(|worker| worker.stream_keys().contains(&(client, player)))
    }
    pub(crate) fn remote_streams_empty(&self) -> bool {
        #[cfg(test)]
        if let Some(state) = self.manual.as_ref() {
            return state.remote_streams.is_empty();
        }
        self.worker
            .as_ref()
            .is_none_or(|worker| worker.stream_keys().is_empty())
    }
    pub(crate) fn clear(&mut self) -> Vec<(i32, i32)> {
        #[cfg(test)]
        if let Some(state) = self.manual.as_mut() {
            return state.clear();
        }
        self.pending_key = None;
        self.worker
            .as_ref()
            .map(VoiceMediaWorker::clear)
            .unwrap_or_default()
    }
    pub(crate) fn set_client_muted(&mut self, client: i32, muted: bool) -> Vec<(i32, i32)> {
        #[cfg(test)]
        if let Some(state) = self.manual.as_mut() {
            return state.set_client_muted(client, muted);
        }
        if muted {
            self.muted.insert(client);
        } else {
            self.muted.remove(&client);
        }
        self.worker
            .as_ref()
            .map(|worker| worker.set_client_muted(client, muted))
            .unwrap_or_default()
    }
    pub(crate) fn forget_client(&mut self, client: i32) -> Vec<(i32, i32)> {
        #[cfg(test)]
        if let Some(state) = self.manual.as_mut() {
            return state.forget_client(client);
        }
        self.worker
            .as_ref()
            .map(|worker| worker.forget_client(client))
            .unwrap_or_default()
    }
}
