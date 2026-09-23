//! The media clock is independent of game updates and simulation locks.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::voice_chat::{
    voice_stream_id, RemoteVoicePlayoutStats, VoiceActivityTracker, VoiceChatState,
};
use crate::voice_media::{service_voice_media, VoiceMediaPolicy, VoiceMediaTransport};
use clonk_audio::{AudioWorkerHandle, VoiceCaptureControl};
use parking_lot::Mutex;
use winit::keyboard::KeyCode;

type Transport = Box<dyn VoiceMediaTransport + Send>;

struct WorkerControl {
    revision: u64,
    privacy_revision: u64,
    policy: Option<VoiceMediaPolicy>,
    capture_key: Option<KeyCode>,
    capture_control: VoiceCaptureControl,
    finish_capture_at: Option<Instant>,
    capture_suspended: bool,
    capture_revision: u64,
    clear_revision: u64,
    muted: BTreeSet<i32>,
    forgotten: BTreeSet<i32>,
    transport: Option<Transport>,
    audio: Option<AudioWorkerHandle>,
}

#[derive(Default)]
struct WorkerStatus {
    activity: VoiceActivityTracker,
    capture_active: bool,
    streams: BTreeMap<(i32, i32), RemoteVoicePlayoutStats>,
}

pub(crate) struct VoiceMediaWorker {
    control: Arc<Mutex<WorkerControl>>,
    status: Arc<Mutex<WorkerStatus>>,
    running: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

impl VoiceMediaWorker {
    pub(crate) fn spawn(
        factory: impl FnOnce() -> VoiceChatState + Send + 'static,
        policy: VoiceMediaPolicy,
        transport: impl VoiceMediaTransport + Send + 'static,
        audio: AudioWorkerHandle,
    ) -> std::io::Result<Self> {
        let control = Arc::new(Mutex::new(WorkerControl {
            revision: 1,
            privacy_revision: 1,
            policy: Some(policy),
            capture_key: None,
            capture_control: VoiceCaptureControl::default(),
            finish_capture_at: None,
            capture_suspended: false,
            capture_revision: 0,
            clear_revision: 0,
            muted: BTreeSet::new(),
            forgotten: BTreeSet::new(),
            transport: None,
            audio: None,
        }));
        let status = Arc::new(Mutex::new(WorkerStatus::default()));
        let running = Arc::new(AtomicBool::new(true));
        let thread_control = control.clone();
        let thread_status = status.clone();
        let thread_running = running.clone();
        let thread = thread::Builder::new().name("voice-media".to_owned()).spawn(move || {
            // Device/codec state is constructed here; it need not be Send.
            let mut state = factory();
            let mut transport: Transport = Box::new(transport);
            let mut audio = audio;
            let mut policy = None;
            let mut revision = 0;
            let mut privacy_revision = 0;
            let mut clear_revision = 0;
            let mut capture_revision = 0;
            let mut capture_key = None;
            let mut muted = BTreeSet::new();
            while thread_running.load(Ordering::Acquire) {
                let update = {
                    let mut control = thread_control.lock();
                    (control.revision != revision).then(|| WorkerControl {
                        revision: control.revision, privacy_revision: control.privacy_revision, policy: control.policy.clone(),
                        capture_key: control.capture_key, capture_suspended: control.capture_suspended, capture_revision: control.capture_revision,
                        capture_control: control.capture_control.clone(), finish_capture_at: control.finish_capture_at,
                        clear_revision: control.clear_revision, muted: control.muted.clone(),
                        forgotten: std::mem::take(&mut control.forgotten),
                        transport: control.transport.take(), audio: control.audio.take(),
                    })
                };
                if let Some(mut control) = update {
                        revision = control.revision;
                        privacy_revision = control.privacy_revision;
                        policy = control.policy.clone();
                        if control.capture_suspended {
                            if let Some(policy) = policy.as_mut() { policy.local_identity = None; }
                        }
                        if let Some(replacement) = control.transport.take() {
                            remove_streams(&audio, state.clear());
                            transport = replacement;
                        }
                        if let Some(replacement) = control.audio.take() {
                            if !audio.shares_mixer(&replacement) {
                                remove_streams(&audio, state.clear());
                            }
                            audio = replacement;
                        }
                        if control.clear_revision != clear_revision {
                            clear_revision = control.clear_revision;
                            remove_streams(&audio, state.clear());
                        }
                        for client in std::mem::take(&mut control.forgotten) {
                            remove_streams(&audio, state.forget_client(client));
                        }
                        for &client in muted.union(&control.muted) {
                            remove_streams(&audio, state.set_client_muted(client, control.muted.contains(&client)));
                        }
                        muted.clone_from(&control.muted);
                        if capture_revision != control.capture_revision {
                            capture_revision = control.capture_revision;
                            if let Some(at) = control.finish_capture_at {
                                state.finish_capture_at(at);
                            } else {
                                state.stop_capture();
                            }
                        }
                        if control.capture_control.is_recording() {
                            state.set_capture_control(control.capture_control.clone());
                        }
                        capture_key = control.capture_key;
                }
                if let Some(policy) = policy.as_ref() {
                    remove_streams(&audio, state.reconcile_context(policy.context));
                    let eligible = policy.microphone_enabled && policy.context.is_some()
                        && policy.local_identity.is_some() && transport.available();
                    if !eligible && capture_key.is_some() {
                        let mut control = thread_control.lock();
                        if control.capture_revision == capture_revision {
                            control.capture_key = None;
                        }
                        capture_key = None;
                        drop(control);
                        state.stop_capture();
                    }
                    if eligible {
                        if let Some(key) = capture_key.filter(|key| state.capture_key() != Some(*key)) {
                            if let Err(error) = state.start_capture_on_device(Some(key), Some(audio.voice_echo_reference()), policy.input_device.clone()) {
                                tracing::warn!(%error, "push-to-talk could not open the microphone");
                            }
                        }
                    }
                    if thread_control.lock().revision != revision {
                        continue;
                    }
                    let mut guarded = PrivacyGuardedTransport {
                        inner: &mut *transport, control: &thread_control, privacy_revision,
                    };
                    service_voice_media(&mut state, policy, &mut guarded, &audio, Instant::now());
                }
                // Never publish a status from before a mute/privacy update.
                let mut control = thread_control.lock();
                if control.revision == revision {
                    if let Some(capture_control) = state.capture_control() {
                        control.capture_control = capture_control;
                    }
                    *thread_status.lock() = WorkerStatus {
                        activity: state.activity_snapshot(),
                        capture_active: state.capture_active(),
                        streams: state.remote_streams.keys().map(|&(client, player)| {
                            ((client, player), state.remote_playout_stats(client, player))
                        }).collect(),
                    };
                }
                drop(control);
                thread::park_timeout(Duration::from_millis(5));
            }
            remove_streams(&audio, state.clear());
        })?;
        Ok(Self {
            control,
            status,
            running,
            thread,
        })
    }

    pub(crate) fn update(
        &self,
        policy: VoiceMediaPolicy,
        transport: Option<Transport>,
        audio: AudioWorkerHandle,
    ) {
        let mut control = self.control.lock();
        let changed_context = control
            .policy
            .as_ref()
            .is_some_and(|old| old.context.is_some() && old.context != policy.context);
        if changed_context
            || transport.is_some()
            || !policy.microphone_enabled
            || policy.local_identity.is_none()
        {
            control.capture_key = None;
            control.capture_revision = control.capture_revision.wrapping_add(1);
        }
        let privacy_changed = control.policy.as_ref().is_none_or(|old| {
            old.microphone_enabled != policy.microphone_enabled
                || old.context != policy.context
                || old.local_identity != policy.local_identity
                || old.activation != policy.activation
                || old.input_device != policy.input_device
        }) || transport.is_some();
        if privacy_changed {
            control.capture_control.abort();
            control.capture_control = VoiceCaptureControl::default();
            control.finish_capture_at = None;
            control.capture_revision = control.capture_revision.wrapping_add(1);
        } else if control.capture_control.is_aborted() && control.finish_capture_at.is_none() {
            control.capture_control = VoiceCaptureControl::default();
        }
        control.capture_suspended = false;
        control.policy = Some(policy);
        if transport.is_some() {
            control.transport = transport;
        }
        control.audio = Some(audio);
        if privacy_changed {
            control.privacy_revision = control.privacy_revision.wrapping_add(1);
        }
        control.revision = control.revision.wrapping_add(1);
        drop(control);
        self.thread.thread().unpark();
    }

    pub(crate) fn request_capture(&self, key: Option<KeyCode>) {
        let mut control = self.control.lock();
        control.capture_control.abort();
        control.capture_control = VoiceCaptureControl::default();
        control.finish_capture_at = None;
        control.capture_key = key;
        control.capture_suspended = key.is_none();
        control.capture_revision = control.capture_revision.wrapping_add(1);
        control.privacy_revision = control.privacy_revision.wrapping_add(1);
        control.revision = control.revision.wrapping_add(1);
        drop(control);
        self.thread.thread().unpark();
    }

    pub(crate) fn finish_capture_at(&self, at: Instant) {
        let mut control = self.control.lock();
        control.capture_control.finish_at(at);
        control.capture_key = None;
        control.capture_suspended = false;
        control.finish_capture_at.get_or_insert(at);
        control.capture_revision = control.capture_revision.wrapping_add(1);
        control.privacy_revision = control.privacy_revision.wrapping_add(1);
        control.revision = control.revision.wrapping_add(1);
        drop(control);
        self.thread.thread().unpark();
    }

    pub(crate) fn capture_key(&self) -> Option<KeyCode> {
        self.control.lock().capture_key
    }
    pub(crate) fn capture_active(&self) -> bool {
        self.control.lock().finish_capture_at.is_none() && self.status.lock().capture_active
    }
    pub(crate) fn active_speakers(&self, now: Instant) -> Vec<(i32, i32)> {
        self.status.lock().activity.active_speakers(now)
    }
    pub(crate) fn stream_keys(&self) -> Vec<(i32, i32)> {
        self.status.lock().streams.keys().copied().collect()
    }
    pub(crate) fn remote_playout_stats(&self, client: i32, player: i32) -> RemoteVoicePlayoutStats {
        self.status
            .lock()
            .streams
            .get(&(client, player))
            .copied()
            .unwrap_or_default()
    }
    pub(crate) fn clear(&self) -> Vec<(i32, i32)> {
        let streams = self.stream_keys();
        let mut control = self.control.lock();
        control.policy = None;
        control.capture_control.abort();
        control.finish_capture_at = None;
        control.capture_key = None;
        control.clear_revision = control.clear_revision.wrapping_add(1);
        control.privacy_revision = control.privacy_revision.wrapping_add(1);
        control.revision = control.revision.wrapping_add(1);
        *self.status.lock() = WorkerStatus::default();
        drop(control);
        self.thread.thread().unpark();
        streams
    }
    pub(crate) fn set_client_muted(&self, client: i32, muted: bool) -> Vec<(i32, i32)> {
        let streams = self
            .stream_keys()
            .into_iter()
            .filter(|(id, _)| *id == client && muted)
            .collect();
        let mut control = self.control.lock();
        if muted {
            control.muted.insert(client);
        } else {
            control.muted.remove(&client);
        }
        control.privacy_revision = control.privacy_revision.wrapping_add(1);
        control.revision = control.revision.wrapping_add(1);
        drop(control);
        self.thread.thread().unpark();
        streams
    }
    pub(crate) fn forget_client(&self, client: i32) -> Vec<(i32, i32)> {
        let streams = self
            .stream_keys()
            .into_iter()
            .filter(|(id, _)| *id == client)
            .collect();
        let mut control = self.control.lock();
        control.forgotten.insert(client);
        if let Some(policy) = control.policy.as_mut() {
            policy.speakers.retain(|(id, _), _| *id != client);
        }
        control.privacy_revision = control.privacy_revision.wrapping_add(1);
        control.revision = control.revision.wrapping_add(1);
        drop(control);
        self.thread.thread().unpark();
        streams
    }
}

struct PrivacyGuardedTransport<'a> {
    inner: &'a mut (dyn VoiceMediaTransport + Send),
    control: &'a Mutex<WorkerControl>,
    privacy_revision: u64,
}

impl VoiceMediaTransport for PrivacyGuardedTransport<'_> {
    fn available(&self) -> bool {
        self.inner.available()
    }
    fn receive(&mut self) -> Vec<clonk_network::ReceivedVoiceFrame> {
        self.inner.receive()
    }
    fn send(
        &self,
        frame: clonk_network::VoiceFrame,
        captured_at: Instant,
    ) -> Result<(), clonk_network::VoiceSendError> {
        let control = self.control.lock();
        if control.privacy_revision != self.privacy_revision {
            return Err(clonk_network::VoiceSendError::Closed);
        }
        // The nonblocking enqueue and permission check share a boundary with
        // revocation, including when opening a device took several seconds.
        self.inner.send(frame, captured_at)
    }
}

impl Drop for VoiceMediaWorker {
    fn drop(&mut self) {
        let mut control = self.control.lock();
        control.capture_control.abort();
        control.privacy_revision = control.privacy_revision.wrapping_add(1);
        drop(control);
        self.running.store(false, Ordering::Release);
        self.thread.thread().unpark();
        // Device teardown belongs to the worker too; the game never waits for it.
    }
}

fn remove_streams(audio: &AudioWorkerHandle, streams: Vec<(i32, i32)>) {
    for (client, player) in streams {
        audio.remove_voice_stream(voice_stream_id(client, player));
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5"),
))]
mod tests {
    use super::*;
    use crate::settings::VoiceActivation;
    use crate::voice_chat::{VoiceChatContext, VoiceFrameSource, LOBBY_VOICE_PLAYER_ID};
    use clonk_audio::{VoiceInputFrame, VoiceProcessingConfig};
    use clonk_network::{ReceivedVoiceFrame, VoiceFrame, VoiceSendError};
    use std::cell::Cell;
    use std::sync::mpsc;

    struct TimedSource {
        next: Cell<Instant>,
        frame: VoiceInputFrame,
    }
    impl VoiceFrameSource for TimedSource {
        fn drain_frames(&self) -> Vec<VoiceInputFrame> {
            let now = Instant::now();
            if now < self.next.get() {
                return Vec::new();
            }
            self.next.set(now + Duration::from_millis(20));
            vec![self.frame]
        }
    }
    struct TestTransport(mpsc::SyncSender<VoiceFrame>);
    impl VoiceMediaTransport for TestTransport {
        fn available(&self) -> bool {
            true
        }
        fn receive(&mut self) -> Vec<ReceivedVoiceFrame> {
            Vec::new()
        }
        fn send(&self, frame: VoiceFrame, _captured_at: Instant) -> Result<(), VoiceSendError> {
            self.0.try_send(frame).map_err(|_| VoiceSendError::Full)
        }
    }
    fn policy() -> VoiceMediaPolicy {
        VoiceMediaPolicy {
            microphone_enabled: true,
            context: Some(VoiceChatContext::Lobby),
            speakers: BTreeMap::new(),
            local_identity: Some((0, LOBBY_VOICE_PLAYER_ID)),
            activation: Some(VoiceActivation {
                threshold: 0.1,
                hangover_frames: 0,
            }),
            processing: VoiceProcessingConfig::DISABLED,
            input_device: None,
        }
    }

    #[test]
    fn push_to_talk_release_flushes_and_ends_without_a_game_update() {
        let (tx, rx) = mpsc::sync_channel(32);
        let audio = clonk_audio::AudioSystem::new_manual_with_resampling(
            8,
            clonk_audio::ResamplingMode::Linear,
        );
        let mut policy = policy();
        policy.activation = None;
        let worker = VoiceMediaWorker::spawn(
            || {
                VoiceChatState::with_source_opener(|_| {
                    Ok(TimedSource {
                        next: Cell::new(Instant::now()),
                        frame: VoiceInputFrame::test_frame(
                            clonk_audio::test_encode_voice_frame(
                                &[1_000; clonk_audio::VOICE_FRAME_SAMPLES],
                            )
                            .unwrap(),
                            1.0,
                        ),
                    })
                })
            },
            policy,
            TestTransport(tx),
            audio.worker_handle(),
        )
        .unwrap();
        worker.request_capture(Some(KeyCode::Backquote));
        assert!(!rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .payload
            .is_empty());
        worker.finish_capture_at(Instant::now());
        let deadline = Instant::now() + Duration::from_millis(200);
        loop {
            let frame = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("release must flush an end marker without a game update");
            if frame.payload.is_empty() {
                break;
            }
        }
        assert!(
            rx.recv_timeout(Duration::from_millis(40)).is_err(),
            "capture continued after its end marker"
        );
    }
    #[test]
    fn disabling_voice_during_a_slow_microphone_open_is_nonblocking_and_sends_nothing() {
        let (tx, rx) = mpsc::sync_channel(32);
        let (opening_tx, opening_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let audio = clonk_audio::AudioSystem::new_manual_with_resampling(
            8,
            clonk_audio::ResamplingMode::Linear,
        );
        let worker = VoiceMediaWorker::spawn(
            move || {
                VoiceChatState::with_source_opener(move |_| {
                    opening_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    Ok(TimedSource {
                        next: Cell::new(Instant::now()),
                        frame: VoiceInputFrame::test_frame(
                            clonk_audio::test_encode_voice_frame(
                                &[12_000; clonk_audio::VOICE_FRAME_SAMPLES],
                            )
                            .unwrap(),
                            0.5,
                        ),
                    })
                })
            },
            policy(),
            TestTransport(tx),
            audio.worker_handle(),
        )
        .unwrap();
        opening_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut disabled = policy();
        disabled.microphone_enabled = false;
        let update_at = Instant::now();
        worker.update(disabled, None, audio.worker_handle());
        assert!(
            update_at.elapsed() < Duration::from_millis(100),
            "policy publication must not wait for a device operation"
        );
        release_tx.send(()).unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "opening completion cannot resurrect revoked microphone permission"
        );
        drop(worker);
    }

    #[test]
    fn dropping_voice_during_a_slow_microphone_open_cannot_send_after_shutdown() {
        let (tx, rx) = mpsc::sync_channel(32);
        let (opening_tx, opening_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let audio = clonk_audio::AudioSystem::new_manual_with_resampling(
            8,
            clonk_audio::ResamplingMode::Linear,
        );
        let worker = VoiceMediaWorker::spawn(
            move || {
                VoiceChatState::with_source_opener(move |_| {
                    opening_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    Ok(TimedSource {
                        next: Cell::new(Instant::now()),
                        frame: VoiceInputFrame::test_frame(
                            clonk_audio::test_encode_voice_frame(
                                &[12_000; clonk_audio::VOICE_FRAME_SAMPLES],
                            )
                            .unwrap(),
                            0.5,
                        ),
                    })
                })
            },
            policy(),
            TestTransport(tx),
            audio.worker_handle(),
        )
        .unwrap();
        opening_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let closing_at = Instant::now();
        drop(worker);
        assert!(closing_at.elapsed() < Duration::from_millis(100));
        release_tx.send(()).unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "opening completion cannot resurrect revoked microphone permission"
        );
    }

    #[test]
    fn stopping_voice_activation_waits_for_fresh_game_permission_before_reopening() {
        let (tx, rx) = mpsc::sync_channel(32);
        let audio = clonk_audio::AudioSystem::new_manual_with_resampling(
            8,
            clonk_audio::ResamplingMode::Linear,
        );
        let worker = VoiceMediaWorker::spawn(
            || {
                VoiceChatState::with_source_opener(|_| {
                    Ok(TimedSource {
                        next: Cell::new(Instant::now()),
                        frame: VoiceInputFrame::test_frame(
                            clonk_audio::test_encode_voice_frame(
                                &[12_000; clonk_audio::VOICE_FRAME_SAMPLES],
                            )
                            .unwrap(),
                            0.5,
                        ),
                    })
                })
            },
            policy(),
            TestTransport(tx),
            audio.worker_handle(),
        )
        .unwrap();
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.request_capture(None);
        rx.try_iter().for_each(drop);
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "focus loss stops voice activation before the next game update"
        );
        worker.update(policy(), None, audio.worker_handle());
        assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
        drop(worker);
    }

    #[test]
    fn replacing_a_voice_session_cancels_the_old_push_to_talk_request() {
        let (tx, rx) = mpsc::sync_channel(32);
        let audio = clonk_audio::AudioSystem::new_manual_with_resampling(
            8,
            clonk_audio::ResamplingMode::Linear,
        );
        let mut ptt = policy();
        ptt.activation = None;
        let worker = VoiceMediaWorker::spawn(
            || {
                VoiceChatState::with_source_opener(|_| {
                    Ok(TimedSource {
                        next: Cell::new(Instant::now()),
                        frame: VoiceInputFrame::test_frame(
                            clonk_audio::test_encode_voice_frame(
                                &[12_000; clonk_audio::VOICE_FRAME_SAMPLES],
                            )
                            .unwrap(),
                            0.5,
                        ),
                    })
                })
            },
            ptt.clone(),
            TestTransport(tx),
            audio.worker_handle(),
        )
        .unwrap();
        worker.request_capture(Some(KeyCode::Backquote));
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (replacement_tx, replacement_rx) = mpsc::sync_channel(32);
        worker.update(
            ptt,
            Some(Box::new(TestTransport(replacement_tx))),
            audio.worker_handle(),
        );
        assert!(
            replacement_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "a held key from the old session cannot open the new session's microphone"
        );
        worker.request_capture(Some(KeyCode::Backquote));
        assert!(replacement_rx.recv_timeout(Duration::from_secs(2)).is_ok());
        drop(worker);
    }

    #[test]
    fn voice_capture_keeps_transmitting_without_game_updates() {
        let (tx, rx) = mpsc::sync_channel(32);
        let audio = clonk_audio::AudioSystem::new_manual_with_resampling(
            8,
            clonk_audio::ResamplingMode::Linear,
        );
        let worker = VoiceMediaWorker::spawn(
            || {
                VoiceChatState::with_source_opener(|_| {
                    Ok(TimedSource {
                        next: Cell::new(Instant::now()),
                        frame: VoiceInputFrame::test_frame(
                            clonk_audio::test_encode_voice_frame(
                                &[12_000; clonk_audio::VOICE_FRAME_SAMPLES],
                            )
                            .unwrap(),
                            0.5,
                        ),
                    })
                })
            },
            policy(),
            TestTransport(tx),
            audio.worker_handle(),
        )
        .unwrap();
        let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let started = Instant::now();
        for offset in 1..=20 {
            // No game updates or manual media pumping during this simulated stall.
            let frame = rx.recv_timeout(Duration::from_millis(250)).unwrap();
            assert_eq!(frame.stream_epoch, first.stream_epoch);
            assert_eq!(frame.sequence, first.sequence + offset);
        }
        assert!(started.elapsed() >= Duration::from_millis(350));
        drop(worker);
    }
}
