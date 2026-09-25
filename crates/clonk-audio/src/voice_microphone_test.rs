//! An explicitly requested, bounded local recording followed by playback.
//! This module has no transport or network sender.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    AudioWorkerHandle, VoiceCapture, VoiceCaptureControl, VoiceCaptureError, VoiceCaptureOptions,
    VoiceInputFrame,
};

const TEST_STREAM: u64 = u64::MAX;
const RECORD_DURATION: Duration = Duration::from_secs(3);
const MAX_RECORDED_FRAMES: usize = 152;
const FLUSH_DURATION: Duration = Duration::from_millis(160);

fn next_test_stream() -> Result<u64, VoiceCaptureError> {
    // Remote voice identities occupy the lower half. Each local test keeps a
    // distinct upper-half key, including during asynchronous cancellation.
    static NEXT: AtomicU64 = AtomicU64::new(u64::MAX - 1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
        id.checked_sub(1).filter(|next| *next >= 1_u64 << 63)
    })
    .map_err(|_| VoiceCaptureError::Stream("local microphone test IDs exhausted".into()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceMicrophoneTestState {
    Opening,
    Recording,
    Finishing,
    Playing,
    Complete,
    Failed(String),
    Unavailable(crate::VoiceCaptureStatus),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoiceMicrophoneTestStatus {
    pub state: VoiceMicrophoneTestState,
    pub level: f32,
    pub remaining: Duration,
}

/// A three-second local recording followed by playback. Start only in response
/// to an explicit test action. It never exposes captured packets to a sender.
/// Cancelling or dropping this handle revokes capture immediately; cleanup and
/// native device closure stay on workers.
pub struct VoiceMicrophoneTest {
    control: VoiceCaptureControl,
    cancelled: Arc<AtomicBool>,
    status: Arc<Mutex<VoiceMicrophoneTestStatus>>,
}

impl VoiceMicrophoneTest {
    pub fn start(
        options: VoiceCaptureOptions,
        audio: AudioWorkerHandle,
    ) -> Result<Self, VoiceCaptureError> {
        Self::with_opener(options, audio, VoiceCapture::open)
    }

    fn with_opener<S: TestCapture + 'static>(
        mut options: VoiceCaptureOptions,
        audio: AudioWorkerHandle,
        open: impl FnOnce(VoiceCaptureOptions) -> Result<S, VoiceCaptureError> + Send + 'static,
    ) -> Result<Self, VoiceCaptureError> {
        let stream_id = next_test_stream()?;
        // A test never shares the live-chat capture's cancellation boundary.
        options.control = VoiceCaptureControl::default();
        let control = options.control.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(VoiceMicrophoneTestStatus {
            state: VoiceMicrophoneTestState::Opening,
            level: 0.0,
            remaining: RECORD_DURATION,
        }));
        let handle = Self {
            control: control.clone(),
            cancelled: cancelled.clone(),
            status: status.clone(),
        };
        std::thread::Builder::new()
            .name("voice-microphone-test".into())
            .spawn(move || {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                let source = match open(options) {
                    Ok(source) => source,
                    Err(error) => {
                        control.abort();
                        let mut status = status.lock().unwrap();
                        if !cancelled.load(Ordering::Acquire) {
                            status.state = VoiceMicrophoneTestState::Failed(error.to_string());
                            status.remaining = Duration::ZERO;
                        }
                        return;
                    }
                };
                let mut core = LocalTestCore::new(source, control, audio, Instant::now());
                core.cancelled = cancelled.clone();
                core.stream_id = stream_id;
                loop {
                    let now = Instant::now();
                    core.service(now);
                    let terminal = matches!(
                        core.state,
                        VoiceMicrophoneTestState::Complete
                            | VoiceMicrophoneTestState::Cancelled
                            | VoiceMicrophoneTestState::Failed(_)
                            | VoiceMicrophoneTestState::Unavailable(_)
                    );
                    let snapshot = core.status(now);
                    {
                        let mut status = status.lock().unwrap();
                        if !cancelled.load(Ordering::Acquire) {
                            *status = snapshot;
                        }
                    }
                    if terminal {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })
            .map_err(|error| VoiceCaptureError::Stream(error.to_string()))?;
        Ok(handle)
    }

    pub fn status(&self) -> VoiceMicrophoneTestStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn cancel(&self) {
        self.control.abort();
        self.cancelled.store(true, Ordering::Release);
        *self.status.lock().unwrap() = VoiceMicrophoneTestStatus {
            state: VoiceMicrophoneTestState::Cancelled,
            level: 0.0,
            remaining: Duration::ZERO,
        };
    }
}

impl Drop for VoiceMicrophoneTest {
    fn drop(&mut self) {
        self.cancel();
    }
}

trait TestCapture {
    fn status(&self) -> crate::VoiceCaptureStatus {
        crate::VoiceCaptureStatus::Active
    }
    fn stream_generation(&self) -> u64 {
        0
    }
    fn dropped_frames(&self) -> u64 {
        0
    }
    fn drain_frames(&self) -> Vec<VoiceInputFrame>;
    fn finish_at(&self, at: Instant);
    fn is_finished(&self) -> bool;
}

impl TestCapture for VoiceCapture {
    fn stream_generation(&self) -> u64 {
        self.stream_generation()
    }
    fn dropped_frames(&self) -> u64 {
        self.dropped_frames()
    }
    fn status(&self) -> crate::VoiceCaptureStatus {
        self.status()
    }
    fn drain_frames(&self) -> Vec<VoiceInputFrame> {
        self.drain_frames()
    }
    fn finish_at(&self, at: Instant) {
        self.finish_at(at);
    }
    fn is_finished(&self) -> bool {
        self.is_finished()
    }
}

struct LocalTestCore<S: TestCapture> {
    source: Option<S>,
    stream_id: u64,
    control: VoiceCaptureControl,
    cancelled: Arc<AtomicBool>,
    audio: AudioWorkerHandle,
    started: Instant,
    state: VoiceMicrophoneTestState,
    recorded: VecDeque<VoiceInputFrame>,
    decoder: Result<crate::VoiceDecoder, crate::VoiceCodecError>,
    level: f32,
    capture_generation: Option<u64>,
    playback_deadline: Option<Instant>,
}

impl<S: TestCapture> LocalTestCore<S> {
    fn new(
        source: S,
        control: VoiceCaptureControl,
        audio: AudioWorkerHandle,
        now: Instant,
    ) -> Self {
        Self {
            source: Some(source),
            stream_id: TEST_STREAM,
            control,
            cancelled: Arc::new(AtomicBool::new(false)),
            audio,
            started: now,
            state: VoiceMicrophoneTestState::Opening,
            recorded: VecDeque::with_capacity(MAX_RECORDED_FRAMES),
            decoder: crate::VoiceDecoder::new(),
            level: 0.0,
            capture_generation: None,
            playback_deadline: None,
        }
    }

    fn status(&self, now: Instant) -> VoiceMicrophoneTestStatus {
        let remaining = match self.state {
            VoiceMicrophoneTestState::Opening => RECORD_DURATION,
            VoiceMicrophoneTestState::Recording => {
                RECORD_DURATION.saturating_sub(now.saturating_duration_since(self.started))
            }
            VoiceMicrophoneTestState::Playing => {
                Duration::from_millis(self.recorded.len() as u64 * 20)
                    + self
                        .audio
                        .voice_stream_stats(self.stream_id)
                        .queued_duration
            }
            _ => Duration::ZERO,
        };
        VoiceMicrophoneTestStatus {
            state: self.state.clone(),
            level: self.level,
            remaining,
        }
    }

    fn fail(&mut self, state: VoiceMicrophoneTestState) {
        self.control.abort();
        self.source = None;
        self.recorded.clear();
        self.level = 0.0;
        self.state = state;
        self.audio.remove_voice_stream(self.stream_id);
    }

    fn service(&mut self, now: Instant) {
        if self.cancelled.load(Ordering::Acquire) {
            self.control.abort();
            self.state = VoiceMicrophoneTestState::Cancelled;
            self.source = None;
            self.recorded.clear();
            self.level = 0.0;
            self.audio.remove_voice_stream(self.stream_id);
            return;
        }
        if self.state == VoiceMicrophoneTestState::Opening {
            let status = self
                .source
                .as_ref()
                .map_or(crate::VoiceCaptureStatus::Unavailable, TestCapture::status);
            if status == crate::VoiceCaptureStatus::Active {
                self.started = now;
                self.state = VoiceMicrophoneTestState::Recording;
            } else if status == crate::VoiceCaptureStatus::Opening
                && now.saturating_duration_since(self.started) < Duration::from_secs(10)
            {
                return;
            } else {
                self.state = VoiceMicrophoneTestState::Unavailable(status);
                self.control.abort();
                self.source = None;
                return;
            }
        }
        if let Some(source) = &self.source {
            let status = source.status();
            if self.state == VoiceMicrophoneTestState::Recording
                && status != crate::VoiceCaptureStatus::Active
            {
                self.fail(VoiceMicrophoneTestState::Unavailable(status));
                return;
            }
            let finished = source.is_finished();
            let frames = source.drain_frames();
            let generation = source.stream_generation();
            if self
                .capture_generation
                .is_some_and(|previous| previous != generation)
            {
                self.fail(VoiceMicrophoneTestState::Failed(
                    "The microphone changed during the test. Record again with the selected input."
                        .into(),
                ));
                return;
            }
            self.capture_generation = Some(generation);
            if source.dropped_frames() > 0 {
                self.fail(VoiceMicrophoneTestState::Failed(
                    "The microphone recording skipped audio. Try again after reducing system load."
                        .into(),
                ));
                return;
            }
            self.recorded.extend(
                frames
                    .into_iter()
                    .filter(|frame| frame.is_fresh_at(now))
                    .filter(|frame| {
                        frame.capture_timing().is_none_or(|timing| {
                            timing.captured_at < self.started + RECORD_DURATION
                        })
                    })
                    .inspect(|frame| self.level = frame.level)
                    .take(MAX_RECORDED_FRAMES.saturating_sub(self.recorded.len())),
            );
            if self.state == VoiceMicrophoneTestState::Recording {
                if now.saturating_duration_since(self.started) >= RECORD_DURATION {
                    source.finish_at(self.started + RECORD_DURATION);
                    self.state = VoiceMicrophoneTestState::Finishing;
                }
                return;
            }
            if self.state == VoiceMicrophoneTestState::Finishing {
                if !finished
                    && now.saturating_duration_since(self.started)
                        < RECORD_DURATION + FLUSH_DURATION
                {
                    return;
                }
                self.control.abort();
                self.source = None;
                if self.recorded.is_empty() {
                    self.state = VoiceMicrophoneTestState::Failed(
                        "No microphone audio arrived. Check the selected input and try again."
                            .into(),
                    );
                    return;
                }
                self.state = VoiceMicrophoneTestState::Playing;
                self.playback_deadline = Some(
                    now + Duration::from_millis(self.recorded.len() as u64 * 20)
                        + Duration::from_secs(1),
                );
                self.level = 0.0;
            }
        }
        if self.state != VoiceMicrophoneTestState::Playing {
            return;
        }
        if self
            .playback_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            if self.recorded.is_empty()
                && self.audio.voice_stream_stats(self.stream_id).queued_frames == 0
            {
                self.state = VoiceMicrophoneTestState::Complete;
                self.audio.remove_voice_stream(self.stream_id);
            } else {
                self.fail(VoiceMicrophoneTestState::Failed(
                    "Test playback stopped. Check the output device and try again.".into(),
                ));
            }
            return;
        }
        while self.audio.voice_stream_stats(self.stream_id).queued_frames < 2 {
            let Some(frame) = self.recorded.pop_front() else {
                break;
            };
            let samples = self
                .decoder
                .as_mut()
                .map_err(|error| error.to_string())
                .and_then(|decoder| {
                    decoder
                        .decode(&frame.payload, false)
                        .map_err(|error| error.to_string())
                });
            let samples = match samples {
                Ok(samples) => samples,
                Err(error) => {
                    self.state = VoiceMicrophoneTestState::Failed(error);
                    self.audio.remove_voice_stream(self.stream_id);
                    return;
                }
            };
            self.audio.queue_voice_stream_with_mix(
                self.stream_id,
                samples,
                crate::voice_playback_gain(1.0),
                0.0,
            );
        }
        if self.recorded.is_empty()
            && self.audio.voice_stream_stats(self.stream_id).queued_frames == 0
        {
            self.state = VoiceMicrophoneTestState::Complete;
            self.audio.remove_voice_stream(self.stream_id);
        }
    }
}

impl<S: TestCapture> Drop for LocalTestCore<S> {
    fn drop(&mut self) {
        self.control.abort();
        self.audio.remove_voice_stream(self.stream_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioSystem, ResamplingMode, VOICE_FRAME_SAMPLES};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    struct Source {
        frames: Mutex<Vec<VoiceInputFrame>>,
        control: VoiceCaptureControl,
        closed: Arc<AtomicBool>,
    }
    impl Drop for Source {
        fn drop(&mut self) {
            self.closed.store(true, Ordering::Release);
        }
    }
    impl TestCapture for Source {
        fn drain_frames(&self) -> Vec<VoiceInputFrame> {
            std::mem::take(&mut *self.frames.lock().unwrap())
        }
        fn finish_at(&self, at: Instant) {
            self.control.finish_at(at);
        }
        fn is_finished(&self) -> bool {
            self.control.finish_time().is_some()
        }
    }

    #[test]
    fn closing_a_microphone_test_during_open_is_immediate_and_revokes_late_capture() {
        let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
        let options = VoiceCaptureOptions::new(crate::VoiceProcessingSwitches::new(
            crate::VoiceProcessingConfig::default(),
        ));
        let other_capture = options.control.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = closed.clone();
        let (entered, opening) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let test =
            VoiceMicrophoneTest::with_opener(options, system.worker_handle(), move |options| {
                entered.send(options.control.clone()).unwrap();
                resumed.recv().unwrap();
                Ok(Source {
                    frames: Mutex::new(Vec::new()),
                    control: options.control,
                    closed: worker_closed,
                })
            })
            .unwrap();
        let input_control = opening.recv_timeout(Duration::from_secs(2)).unwrap();
        let status = test.status.clone();
        let stopped = Instant::now();
        drop(test);
        assert!(stopped.elapsed() < Duration::from_millis(100));
        assert!(input_control.is_aborted());
        assert!(
            !other_capture.is_aborted(),
            "local testing cannot cancel a separate capture"
        );
        resume.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !closed.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(closed.load(Ordering::Acquire));
        assert_eq!(
            status.lock().unwrap().state,
            VoiceMicrophoneTestState::Cancelled
        );
    }

    #[test]
    fn microphone_test_exposes_permission_and_device_failures_without_leaving_capture_open() {
        struct UnavailableSource(crate::VoiceCaptureStatus, Source);
        impl TestCapture for UnavailableSource {
            fn status(&self) -> crate::VoiceCaptureStatus {
                self.0.clone()
            }
            fn drain_frames(&self) -> Vec<VoiceInputFrame> {
                self.1.drain_frames()
            }
            fn finish_at(&self, at: Instant) {
                self.1.finish_at(at);
            }
            fn is_finished(&self) -> bool {
                self.1.is_finished()
            }
        }
        for failure in [
            crate::VoiceCaptureStatus::PermissionDenied,
            crate::VoiceCaptureStatus::DeviceBusy,
            crate::VoiceCaptureStatus::Unavailable,
            crate::VoiceCaptureStatus::Retrying("device failed".into()),
        ] {
            let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
            let control = VoiceCaptureControl::default();
            let closed = Arc::new(AtomicBool::new(false));
            let source = UnavailableSource(
                failure.clone(),
                Source {
                    frames: Mutex::new(Vec::new()),
                    control: control.clone(),
                    closed: closed.clone(),
                },
            );
            let now = Instant::now();
            let mut test = LocalTestCore::new(source, control.clone(), system.worker_handle(), now);
            test.service(now);
            assert_eq!(test.state, VoiceMicrophoneTestState::Unavailable(failure));
            assert!(closed.load(Ordering::Acquire));
            assert!(control.is_aborted());
        }
    }

    #[test]
    fn microphone_test_flush_has_a_deadline_even_if_native_close_never_completes() {
        struct ClosingSource(Source);
        impl TestCapture for ClosingSource {
            fn drain_frames(&self) -> Vec<VoiceInputFrame> {
                self.0.drain_frames()
            }
            fn finish_at(&self, at: Instant) {
                self.0.finish_at(at);
            }
            fn is_finished(&self) -> bool {
                false
            }
        }
        let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
        let control = VoiceCaptureControl::default();
        let closed = Arc::new(AtomicBool::new(false));
        let source = ClosingSource(Source {
            frames: Mutex::new(vec![
                VoiceInputFrame::test_frame(
                    crate::test_encode_voice_frame(&[2_000; VOICE_FRAME_SAMPLES]).unwrap(),
                    0.5
                );
                3
            ]),
            control: control.clone(),
            closed: closed.clone(),
        });
        let now = Instant::now();
        let mut test = LocalTestCore::new(source, control.clone(), system.worker_handle(), now);
        test.service(now);
        test.service(now + RECORD_DURATION);
        test.service(now + RECORD_DURATION + Duration::from_millis(159));
        assert_eq!(test.state, VoiceMicrophoneTestState::Finishing);
        assert!(!closed.load(Ordering::Acquire));
        test.service(now + RECORD_DURATION + FLUSH_DURATION);
        assert_eq!(test.state, VoiceMicrophoneTestState::Playing);
        assert!(control.is_aborted());
        assert!(closed.load(Ordering::Acquire));
        test.cancelled.store(true, Ordering::Release);
        test.service(now + RECORD_DURATION + FLUSH_DURATION);
        assert_eq!(test.state, VoiceMicrophoneTestState::Cancelled);
        assert!(test.recorded.is_empty());
        assert_eq!(system.voice_stream_stats(TEST_STREAM).queued_frames, 0);
    }

    #[test]
    fn microphone_test_fails_if_output_never_consumes_the_recording() {
        let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
        let control = VoiceCaptureControl::default();
        let source = Source {
            frames: Mutex::new(vec![
                VoiceInputFrame::test_frame(
                    crate::test_encode_voice_frame(&[2_000; VOICE_FRAME_SAMPLES]).unwrap(),
                    0.5
                );
                3
            ]),
            control: control.clone(),
            closed: Arc::new(AtomicBool::new(false)),
        };
        let now = Instant::now();
        let mut test = LocalTestCore::new(source, control, system.worker_handle(), now);
        test.service(now);
        test.service(now + RECORD_DURATION);
        test.service(now + RECORD_DURATION + Duration::from_millis(20));
        test.service(now + Duration::from_secs(10));
        assert!(matches!(test.state, VoiceMicrophoneTestState::Failed(_)));
        assert!(test.recorded.is_empty());
        assert_eq!(system.voice_stream_stats(TEST_STREAM).queued_frames, 0);
    }

    #[test]
    fn microphone_test_rejects_device_changes_and_capture_overruns() {
        struct HealthSource {
            source: Source,
            status: Arc<Mutex<crate::VoiceCaptureStatus>>,
            generation: Arc<AtomicU64>,
            dropped: Arc<AtomicU64>,
        }
        impl TestCapture for HealthSource {
            fn status(&self) -> crate::VoiceCaptureStatus {
                self.status.lock().unwrap().clone()
            }
            fn stream_generation(&self) -> u64 {
                self.generation.load(Ordering::Relaxed)
            }
            fn dropped_frames(&self) -> u64 {
                self.dropped.load(Ordering::Relaxed)
            }
            fn drain_frames(&self) -> Vec<VoiceInputFrame> {
                self.source.drain_frames()
            }
            fn finish_at(&self, at: Instant) {
                self.source.finish_at(at);
            }
            fn is_finished(&self) -> bool {
                self.source.is_finished()
            }
        }
        for failure in 0..3 {
            let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
            let control = VoiceCaptureControl::default();
            let closed = Arc::new(AtomicBool::new(false));
            let status = Arc::new(Mutex::new(crate::VoiceCaptureStatus::Active));
            let generation = Arc::new(AtomicU64::new(1));
            let dropped = Arc::new(AtomicU64::new(0));
            let source = HealthSource {
                source: Source {
                    frames: Mutex::new(vec![VoiceInputFrame::test_frame(
                        crate::test_encode_voice_frame(&[2_000; VOICE_FRAME_SAMPLES]).unwrap(),
                        0.5,
                    )]),
                    control: control.clone(),
                    closed: closed.clone(),
                },
                status: status.clone(),
                generation: generation.clone(),
                dropped: dropped.clone(),
            };
            let now = Instant::now();
            let mut test = LocalTestCore::new(source, control.clone(), system.worker_handle(), now);
            test.service(now);
            assert_eq!(test.recorded.len(), 1);
            match failure {
                0 => *status.lock().unwrap() = crate::VoiceCaptureStatus::DeviceBusy,
                1 => generation.store(2, Ordering::Relaxed),
                _ => dropped.store(1, Ordering::Relaxed),
            }
            test.service(now + Duration::from_millis(20));
            assert!(matches!(
                test.state,
                VoiceMicrophoneTestState::Unavailable(_) | VoiceMicrophoneTestState::Failed(_)
            ));
            assert!(test.recorded.is_empty());
            assert!(closed.load(Ordering::Acquire));
            assert!(control.is_aborted());
        }
    }

    #[test]
    fn a_cancelled_microphone_test_cannot_remove_its_replacements_playback() {
        let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
        let control = VoiceCaptureControl::default();
        let source = Source {
            frames: Mutex::new(Vec::new()),
            control: control.clone(),
            closed: Arc::new(AtomicBool::new(false)),
        };
        let mut old = LocalTestCore::new(source, control, system.worker_handle(), Instant::now());
        old.stream_id = next_test_stream().unwrap();
        let next_stream = next_test_stream().unwrap();
        system.queue_voice_stream(next_stream, [2_000; VOICE_FRAME_SAMPLES]);
        old.cancelled.store(true, Ordering::Release);
        old.service(Instant::now());
        drop(old);
        assert_eq!(system.voice_stream_stats(next_stream).queued_frames, 1);
    }

    #[test]
    fn microphone_test_reports_when_an_open_device_never_delivers_audio() {
        let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
        let control = VoiceCaptureControl::default();
        let closed = Arc::new(AtomicBool::new(false));
        let source = Source {
            frames: Mutex::new(Vec::new()),
            control: control.clone(),
            closed: closed.clone(),
        };
        let now = Instant::now();
        let mut test = LocalTestCore::new(source, control, system.worker_handle(), now);
        test.service(now);
        test.service(now + RECORD_DURATION);
        test.service(now + RECORD_DURATION + Duration::from_millis(20));
        assert!(matches!(test.state, VoiceMicrophoneTestState::Failed(_)));
        assert!(closed.load(Ordering::Acquire));
        assert_eq!(system.voice_stream_stats(TEST_STREAM).queued_frames, 0);
    }

    #[test]
    fn microphone_test_records_three_seconds_after_the_device_becomes_ready() {
        struct OpeningSource(Arc<Mutex<crate::VoiceCaptureStatus>>, Source);
        impl TestCapture for OpeningSource {
            fn status(&self) -> crate::VoiceCaptureStatus {
                self.0.lock().unwrap().clone()
            }
            fn drain_frames(&self) -> Vec<VoiceInputFrame> {
                self.1.drain_frames()
            }
            fn finish_at(&self, at: Instant) {
                self.1.finish_at(at);
            }
            fn is_finished(&self) -> bool {
                self.1.is_finished()
            }
        }
        let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
        let control = VoiceCaptureControl::default();
        let status = Arc::new(Mutex::new(crate::VoiceCaptureStatus::Opening));
        let source = OpeningSource(
            status.clone(),
            Source {
                frames: Mutex::new(Vec::new()),
                control: control.clone(),
                closed: Arc::new(AtomicBool::new(false)),
            },
        );
        let now = Instant::now();
        let mut test = LocalTestCore::new(source, control.clone(), system.worker_handle(), now);
        test.service(now);
        assert_eq!(test.state, VoiceMicrophoneTestState::Opening);
        *status.lock().unwrap() = crate::VoiceCaptureStatus::Active;
        test.service(now + Duration::from_secs(2));
        assert_eq!(test.state, VoiceMicrophoneTestState::Recording);
        test.service(now + Duration::from_secs(3));
        assert_eq!(test.state, VoiceMicrophoneTestState::Recording);
        test.service(now + Duration::from_secs(5));
        assert_eq!(test.state, VoiceMicrophoneTestState::Finishing);
        assert_eq!(control.finish_time(), Some(now + Duration::from_secs(5)));
    }

    #[test]
    fn microphone_test_plays_the_entire_recording_after_capture_has_closed() {
        let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
        let control = VoiceCaptureControl::default();
        let source = Source {
            frames: Mutex::new(vec![
                VoiceInputFrame::test_frame(
                    crate::test_encode_voice_frame(&[2_000; VOICE_FRAME_SAMPLES]).unwrap(),
                    0.5
                );
                8
            ]),
            control: control.clone(),
            closed: Arc::new(AtomicBool::new(false)),
        };
        let now = Instant::now();
        let mut test = LocalTestCore::new(source, control, system.worker_handle(), now);
        test.service(now);
        test.service(now + RECORD_DURATION);
        test.service(now + RECORD_DURATION + Duration::from_millis(20));
        for index in 0..10 {
            system.mixer().mix_f32(&mut [0.0_f32; 1764]);
            test.service(now + RECORD_DURATION + Duration::from_millis(40 + index * 20));
        }
        assert_eq!(test.state, VoiceMicrophoneTestState::Complete);
        assert_eq!(system.voice_stream_stats(TEST_STREAM).queued_frames, 0);
    }

    #[test]
    fn microphone_test_closes_capture_before_playing_the_bounded_recording() {
        let system = AudioSystem::new_manual_with_resampling(8, ResamplingMode::Linear);
        let control = VoiceCaptureControl::default();
        let closed = Arc::new(AtomicBool::new(false));
        let source = Source {
            frames: Mutex::new(vec![
                VoiceInputFrame::test_frame(
                    crate::test_encode_voice_frame(&[2_000; VOICE_FRAME_SAMPLES]).unwrap(),
                    0.5
                );
                3
            ]),
            control: control.clone(),
            closed: closed.clone(),
        };
        let now = Instant::now();
        let mut test = LocalTestCore::new(source, control.clone(), system.worker_handle(), now);
        test.service(now);
        assert_eq!(test.state, VoiceMicrophoneTestState::Recording);
        assert_eq!(
            system.voice_stream_stats(TEST_STREAM).queued_frames,
            0,
            "recording must never feed back into the microphone"
        );
        test.service(now + RECORD_DURATION);
        assert_eq!(test.state, VoiceMicrophoneTestState::Finishing);
        assert_eq!(control.finish_time(), Some(now + RECORD_DURATION));
        test.service(now + RECORD_DURATION + Duration::from_millis(20));
        assert!(closed.load(Ordering::Acquire));
        assert_eq!(test.state, VoiceMicrophoneTestState::Playing);
        assert!(system.voice_stream_stats(TEST_STREAM).queued_frames > 0);
    }
}
