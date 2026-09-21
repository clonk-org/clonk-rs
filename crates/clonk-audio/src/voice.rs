use std::fmt;
use std::str::FromStr;
#[cfg(any(feature = "cpal", test))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(feature = "cpal", test))]
use std::sync::mpsc::Receiver;
#[cfg(any(feature = "cpal", test))]
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::VoiceCaptureControl;
use thiserror::Error;

use crate::voice_echo::VoiceEchoReference;
#[cfg(any(feature = "cpal", test))]
use crate::voice_processing::VoiceProcessing;
use crate::voice_processing::VoiceProcessingSwitches;

use crate::voice_codec::{EncodedVoiceFrame, VOICE_FRAME_SAMPLES, VOICE_SAMPLE_RATE};

/// Opaque CPAL input-endpoint identity suitable for persistence.
///
/// IDs obtained from CPAL use `<host>:<device>`. Parsing intentionally preserves
/// every nonempty string byte-for-byte: a corrupt or foreign persisted ID stays
/// an exact (unavailable) selection instead of silently becoming the default.
/// This identifies a host endpoint, not necessarily a physical device;
/// stability and routing are defined by that host.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VoiceInputDeviceId(Box<str>);

impl VoiceInputDeviceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VoiceInputDeviceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("voice input device ID cannot be empty")]
pub struct VoiceInputDeviceIdParseError;

impl FromStr for VoiceInputDeviceId {
    type Err = VoiceInputDeviceIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        (!value.is_empty())
            .then(|| Self(Box::from(value)))
            .ok_or(VoiceInputDeviceIdParseError)
    }
}

/// User-facing metadata for one selectable input endpoint.
///
/// Names are labels only and need not be unique. Persist and compare [`Self::id`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceInputDevice {
    pub id: VoiceInputDeviceId,
    pub name: String,
}

/// Enumerates the input endpoints currently exposed by CPAL's default host.
///
/// This queries metadata only; it does not build or start a capture stream.
pub fn voice_input_devices() -> Result<Vec<VoiceInputDevice>, VoiceCaptureError> {
    #[cfg(feature = "cpal")]
    {
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let devices = host.input_devices().map_err(cpal_capture_error)?;
        Ok(devices
            .filter_map(|device| {
                let id = device.id().map_err(|error| {
                    tracing::warn!(%error, "input device disappeared while reading its ID");
                });
                let description = device.description().map_err(|error| {
                    tracing::warn!(%error, "input device disappeared while reading its description");
                });
                id.ok()
                    .zip(description.ok())
                    .map(|(id, description)| VoiceInputDevice {
                        id: VoiceInputDeviceId(Box::from(id.to_string())),
                        name: description.name().to_string(),
                    })
            })
            .collect())
    }
    #[cfg(not(feature = "cpal"))]
    {
        Err(VoiceCaptureError::Unavailable)
    }
}

/// One captured frame together with how loud it was, so a voice-activation
/// gate never has to decode a frame back just to decide whether to transmit it.
///
/// The level is measured on the frame as the microphone heard it, before
/// automatic gain control — see
/// [`VoiceProcessing::process`](crate::voice_processing::VoiceProcessing::process).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceInputFrame {
    pub payload: EncodedVoiceFrame,
    /// See [`voice_activation_level`].
    pub level: f32,
    timing: Option<VoiceCaptureTiming>,
}

/// Timestamp and sample position of the first sample, before processing delays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceCaptureTiming {
    pub captured_at: Instant,
    /// Continuous 48 kHz sample count within one physical capture generation.
    pub sample_offset: u64,
}

const MAX_CAPTURE_AGE: Duration = Duration::from_millis(160);

impl VoiceInputFrame {
    pub fn capture_timing(&self) -> Option<VoiceCaptureTiming> {
        self.timing
    }

    pub fn is_fresh_at(&self, now: Instant) -> bool {
        self.timing.is_none_or(|timing| {
            now.saturating_duration_since(timing.captured_at) <= MAX_CAPTURE_AGE
        })
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_frame(payload: EncodedVoiceFrame, level: f32) -> Self {
        Self {
            payload,
            level,
            timing: None,
        }
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn with_test_timing(mut self, timing: VoiceCaptureTiming) -> Self {
        self.timing = Some(timing);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct QueuedVoiceInputFrame {
    callback_generation: u64,
    frame: VoiceInputFrame,
}

type CaptureFrameQueue = Arc<crossbeam_queue::ArrayQueue<QueuedVoiceInputFrame>>;

/// At most 160 ms of captured audio can wait for the media worker. This
/// preallocated queue evicts the oldest frame under overload, without blocking.
pub const VOICE_CAPTURE_QUEUE_FRAMES: usize = 8;
#[cfg(any(feature = "cpal", test))]
const MIN_VOICE_CAPTURE_SAMPLE_RATE: u32 = 8_000;
#[cfg(any(feature = "cpal", test))]
const MAX_VOICE_CAPTURE_SAMPLE_RATE: u32 = 192_000;
#[cfg(any(feature = "cpal", test))]
const MAX_VOICE_CAPTURE_CHANNELS: u16 = 32;

#[derive(Debug, Error)]
pub enum VoiceCaptureError {
    #[error("microphone permission was denied: {0}")]
    PermissionDenied(String),
    #[error("microphone device is busy: {0}")]
    DeviceBusy(String),
    #[error("microphone capture was cancelled")]
    Cancelled,
    #[error("microphone capture support was disabled at compile time")]
    Unavailable,
    #[error("no microphone input device is available")]
    NoInputDevice,
    #[error("selected microphone input device is not available: {0}")]
    InputDeviceUnavailable(VoiceInputDeviceId),
    #[error("failed to enumerate microphone input devices: {0}")]
    InputDevices(String),
    #[error("failed to query the microphone input format: {0}")]
    InputConfig(String),
    #[error("unsupported microphone input format: {sample_rate} Hz, {channels} channels")]
    UnsupportedInputConfig { sample_rate: u32, channels: u16 },
    #[error("failed to open the microphone input stream: {0}")]
    Stream(String),
}

#[cfg(any(feature = "cpal", test))]
const VOICE_CAPTURE_DEVICE_POLL_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(any(feature = "cpal", test))]
const VOICE_CAPTURE_EVENT_QUEUE: usize = 8;

#[cfg(any(feature = "cpal", test))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CaptureDeviceInventory {
    default: Option<VoiceInputDeviceId>,
    inputs: Vec<VoiceInputDeviceId>,
}

#[cfg(any(feature = "cpal", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
enum CaptureDeviceTarget {
    SystemDefault(VoiceInputDeviceId),
    Exact(VoiceInputDeviceId),
}

#[cfg(any(feature = "cpal", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureStreamEventAction {
    #[cfg(feature = "cpal")]
    Keep,
    Refresh,
    Invalidate,
}

#[cfg(any(feature = "cpal", test))]
#[derive(Clone, Copy, Debug)]
struct CaptureStreamEvent {
    generation: u64,
    action: CaptureStreamEventAction,
}

#[cfg(any(feature = "cpal", test))]
#[derive(Clone)]
struct CaptureStreamCallbacks {
    generation: u64,
    stopped: Arc<AtomicBool>,
    control: VoiceCaptureControl,
    frames: CaptureFrameQueue,
    dropped_frames: Arc<AtomicU64>,
    active_generation: Arc<AtomicU64>,
    invalidated_generation: Arc<AtomicU64>,
    route_changed_generation: Arc<AtomicU64>,
    events: SyncSender<CaptureStreamEvent>,
}

#[cfg(any(feature = "cpal", test))]
impl CaptureStreamCallbacks {
    fn send_frame(&self, frame: VoiceInputFrame) {
        if self.stopped.load(Ordering::Acquire)
            || self.control.is_aborted()
            || self.active_generation.load(Ordering::Acquire) != self.generation
        {
            return;
        }
        self.enqueue_frame(self.generation, frame);
    }

    fn enqueue_frame(&self, generation: u64, frame: VoiceInputFrame) {
        if self
            .frames
            .force_push(QueuedVoiceInputFrame {
                callback_generation: generation,
                frame,
            })
            .is_some()
        {
            self.dropped_frames.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cfg(test)]
    fn enqueue_frame_after_activation_check(&self, frame: VoiceInputFrame) {
        self.enqueue_frame(self.generation, frame);
    }

    fn report(&self, action: CaptureStreamEventAction) {
        let generation = self.generation;
        match action {
            #[cfg(feature = "cpal")]
            CaptureStreamEventAction::Keep => {}
            CaptureStreamEventAction::Refresh => {
                self.route_changed_generation
                    .fetch_max(generation, Ordering::Release);
                let _ = self.active_generation.compare_exchange(
                    generation,
                    0,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            CaptureStreamEventAction::Invalidate => {
                self.invalidated_generation
                    .fetch_max(generation, Ordering::Release);
                let _ = self.active_generation.compare_exchange(
                    generation,
                    0,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
        let _ = self
            .events
            .try_send(CaptureStreamEvent { generation, action });
    }
}

#[cfg(any(feature = "cpal", test))]
trait VoiceCaptureBackend {
    type Stream;

    fn inventory(
        &mut self,
        selected: Option<&VoiceInputDeviceId>,
    ) -> Result<CaptureDeviceInventory, VoiceCaptureError>;

    fn open_stream(
        &mut self,
        target: &CaptureDeviceTarget,
        callbacks: CaptureStreamCallbacks,
        options: &VoiceCaptureOptions,
    ) -> Result<Self::Stream, VoiceCaptureError>;
}

#[cfg(any(feature = "cpal", test))]
struct ActiveCaptureStream<S> {
    target: CaptureDeviceTarget,
    callback_generation: u64,
    invalidated_generation: Arc<AtomicU64>,
    route_changed_generation: Arc<AtomicU64>,
    _stream: S,
}

#[cfg(any(feature = "cpal", test))]
struct VoiceCaptureManager<B: VoiceCaptureBackend> {
    backend: B,
    stopped: Arc<AtomicBool>,
    options: VoiceCaptureOptions,
    active: Option<ActiveCaptureStream<B::Stream>>,
    active_generation: Arc<AtomicU64>,
    stream_generation: Arc<AtomicU64>,
    next_callback_generation: u64,
    frames: CaptureFrameQueue,
    dropped_frames: Arc<AtomicU64>,
    event_sender: SyncSender<CaptureStreamEvent>,
    events: Receiver<CaptureStreamEvent>,
    next_poll: Instant,
}

#[cfg(any(feature = "cpal", test))]
impl<B: VoiceCaptureBackend> VoiceCaptureManager<B> {
    fn new(
        backend: B,
        options: VoiceCaptureOptions,
        frames: CaptureFrameQueue,
        dropped_frames: Arc<AtomicU64>,
    ) -> Self {
        let (event_sender, events) = std::sync::mpsc::sync_channel(VOICE_CAPTURE_EVENT_QUEUE);
        Self {
            backend,
            stopped: Arc::new(AtomicBool::new(false)),
            options,
            active: None,
            active_generation: Arc::new(AtomicU64::new(0)),
            stream_generation: Arc::new(AtomicU64::new(0)),
            next_callback_generation: 1,
            frames,
            dropped_frames,
            event_sender,
            events,
            next_poll: Instant::now() + VOICE_CAPTURE_DEVICE_POLL_INTERVAL,
        }
    }

    fn ensure_requested(&self) -> Result<(), VoiceCaptureError> {
        if self.stopped.load(Ordering::Acquire) || !self.options.control.is_recording() {
            Err(VoiceCaptureError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn open_initial(&mut self) -> Result<(), VoiceCaptureError> {
        self.ensure_requested()?;
        let inventory = self.backend.inventory(self.options.input_device.as_ref())?;
        let target = match self.options.input_device.as_ref() {
            Some(selected) if inventory.inputs.contains(selected) => {
                CaptureDeviceTarget::Exact(selected.clone())
            }
            Some(selected) => {
                return Err(VoiceCaptureError::InputDeviceUnavailable(selected.clone()));
            }
            None => inventory
                .default
                .map(CaptureDeviceTarget::SystemDefault)
                .ok_or(VoiceCaptureError::NoInputDevice)?,
        };
        self.replace_stream(target)
    }

    fn replace_stream(&mut self, target: CaptureDeviceTarget) -> Result<(), VoiceCaptureError> {
        self.ensure_requested()?;
        let callback_generation = self.next_callback_generation;
        self.next_callback_generation = callback_generation.wrapping_add(1).max(1);
        let invalidated_generation = Arc::new(AtomicU64::new(0));
        let route_changed_generation = Arc::new(AtomicU64::new(0));
        let callbacks = CaptureStreamCallbacks {
            generation: callback_generation,
            stopped: self.stopped.clone(),
            control: self.options.control.clone(),
            frames: self.frames.clone(),
            dropped_frames: self.dropped_frames.clone(),
            active_generation: self.active_generation.clone(),
            invalidated_generation: invalidated_generation.clone(),
            route_changed_generation: route_changed_generation.clone(),
            events: self.event_sender.clone(),
        };
        let stream = self
            .backend
            .open_stream(&target, callbacks, &self.options)?;
        self.ensure_requested()?;
        let previous = self.active.replace(ActiveCaptureStream {
            target,
            callback_generation,
            invalidated_generation,
            route_changed_generation,
            _stream: stream,
        });
        self.active_generation
            .store(callback_generation, Ordering::Release);
        let reported_during_open = self.active.as_ref().is_some_and(|active| {
            active.invalidated_generation.load(Ordering::Acquire) == callback_generation
                || active.route_changed_generation.load(Ordering::Acquire) == callback_generation
        });
        if reported_during_open {
            let _ = self.active_generation.compare_exchange(
                callback_generation,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        let stream_generation = self
            .stream_generation
            .load(Ordering::Relaxed)
            .saturating_add(1);
        self.stream_generation
            .store(stream_generation, Ordering::Release);
        drop(previous);
        Ok(())
    }

    /// Reconciles the active stream with a fresh device snapshot. The return
    /// value is true only after a new physical stream has opened successfully.
    fn refresh(&mut self) -> Result<bool, VoiceCaptureError> {
        let inventory = self.backend.inventory(self.options.input_device.as_ref())?;
        let target = match self.options.input_device.as_ref() {
            Some(selected) if inventory.inputs.contains(selected) => {
                Some(CaptureDeviceTarget::Exact(selected.clone()))
            }
            Some(_) => None,
            None => inventory.default.map(CaptureDeviceTarget::SystemDefault),
        };

        let Some(target) = target else {
            self.deactivate();
            return Ok(false);
        };
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.target == target)
        {
            return Ok(false);
        }

        match self.replace_stream(target) {
            Ok(()) => Ok(true),
            Err(error) => {
                self.deactivate();
                Err(error)
            }
        }
    }

    fn deactivate(&mut self) {
        self.active_generation.store(0, Ordering::Release);
        drop(self.active.take());
    }

    fn service(&mut self, now: Instant) -> Result<bool, VoiceCaptureError> {
        let poll_due = now >= self.next_poll;
        let active_invalidated = self.active.as_ref().is_some_and(|active| {
            active.invalidated_generation.swap(0, Ordering::AcqRel) == active.callback_generation
        });
        let active_route_changed = self.active.as_ref().is_some_and(|active| {
            active.route_changed_generation.swap(0, Ordering::AcqRel) == active.callback_generation
        });
        #[cfg(feature = "cpal")]
        let mut saw_recoverable_error = false;
        let mut saw_route_change = active_route_changed;
        let mut saw_invalidation = active_invalidated;
        while let Ok(event) = self.events.try_recv() {
            if self
                .active
                .as_ref()
                .map(|active| active.callback_generation)
                != Some(event.generation)
            {
                continue;
            }
            match event.action {
                #[cfg(feature = "cpal")]
                CaptureStreamEventAction::Keep => saw_recoverable_error = true,
                CaptureStreamEventAction::Refresh => saw_route_change = true,
                CaptureStreamEventAction::Invalidate => saw_invalidation = true,
            }
        }
        #[cfg(feature = "cpal")]
        if saw_recoverable_error {
            tracing::warn!("recoverable cpal microphone input stream error");
        }
        if saw_route_change {
            tracing::warn!("cpal microphone input route changed");
        }
        if saw_invalidation {
            tracing::error!("cpal microphone input stream invalidated");
        }

        if saw_invalidation || saw_route_change {
            self.deactivate();
            self.next_poll = now + VOICE_CAPTURE_DEVICE_POLL_INTERVAL;
            return self.refresh();
        }
        if !poll_due {
            return Ok(false);
        }
        self.next_poll = now + VOICE_CAPTURE_DEVICE_POLL_INTERVAL;
        self.refresh()
    }

    #[cfg(test)]
    fn drain_frames(&mut self, receiver: &CaptureFrameQueue) -> Vec<VoiceInputFrame> {
        match self.service(Instant::now()) {
            Ok(true) => {
                // A stream generation is an app-visible media boundary. No
                // frame captured before the swap may cross it.
                std::iter::from_fn(|| receiver.pop()).for_each(drop);
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "microphone stream refresh failed; capture remains idle");
            }
        }
        self.collect_active_frames(receiver, || {})
    }

    #[cfg(test)]
    fn collect_active_frames(
        &self,
        receiver: &CaptureFrameQueue,
        after_collect: impl FnOnce(),
    ) -> Vec<VoiceInputFrame> {
        let generation_before = self.active_generation.load(Ordering::Acquire);
        let now = Instant::now();
        let frames = std::iter::from_fn(|| receiver.pop())
            .filter(|queued| queued.callback_generation == generation_before)
            .map(|queued| queued.frame)
            .filter(|frame| {
                let fresh = frame.is_fresh_at(now);
                if !fresh {
                    self.dropped_frames.fetch_add(1, Ordering::Relaxed);
                }
                fresh
            })
            .collect();
        after_collect();
        if generation_before != 0
            && self.active_generation.load(Ordering::Acquire) == generation_before
        {
            frames
        } else {
            Vec::new()
        }
    }

    #[cfg(test)]
    fn stream_generation(&self) -> u64 {
        self.stream_generation.load(Ordering::Acquire)
    }
}

/// How a capture treats what it hears: which processing stages run, and the
/// far-end signal the echo canceller needs.
#[derive(Clone, Debug)]
pub struct VoiceCaptureOptions {
    /// Shared with the caller so release and privacy revocation reach the
    /// callback immediately, even while the media worker is busy.
    pub control: VoiceCaptureControl,
    /// `None` follows the system default. `Some` opens only the matching CPAL
    /// endpoint ID; this layer never substitutes another ID. An endpoint may
    /// itself be a host routing alias (notably under ALSA).
    pub input_device: Option<VoiceInputDeviceId>,
    /// Read by the microphone thread once per frame, so a settings change
    /// reaches a capture that is already open.
    pub processing: Arc<VoiceProcessingSwitches>,
    /// What the mixer is playing, from
    /// [`AudioSystem::voice_echo_reference`](crate::AudioSystem::voice_echo_reference).
    /// Without it there is nothing to cancel an echo against.
    pub echo_reference: Option<VoiceEchoReference>,
}

impl VoiceCaptureOptions {
    pub fn new(processing: Arc<VoiceProcessingSwitches>) -> Self {
        Self {
            control: VoiceCaptureControl::default(),
            input_device: None,
            processing,
            echo_reference: None,
        }
    }

    pub fn with_echo_reference(mut self, reference: VoiceEchoReference) -> Self {
        self.echo_reference = Some(reference);
        self
    }
}

/// The current input-device state. Capture is requested explicitly; failures
/// are retried on the device worker without delaying incoming voice or gameplay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceCaptureStatus {
    PermissionDenied,
    DeviceBusy,
    Opening,
    Active,
    Unavailable,
    Retrying(String),
}

impl VoiceCaptureStatus {
    #[cfg(any(feature = "cpal", test))]
    fn from_error(error: &VoiceCaptureError) -> Self {
        match error {
            VoiceCaptureError::PermissionDenied(_) => Self::PermissionDenied,
            VoiceCaptureError::DeviceBusy(_) => Self::DeviceBusy,
            VoiceCaptureError::Unavailable
            | VoiceCaptureError::NoInputDevice
            | VoiceCaptureError::InputDeviceUnavailable(_)
            | VoiceCaptureError::Cancelled => Self::Unavailable,
            _ => Self::Retrying(error.to_string()),
        }
    }
}

#[cfg(feature = "cpal")]
fn cpal_capture_error(error: cpal::Error) -> VoiceCaptureError {
    match error.kind() {
        cpal::ErrorKind::PermissionDenied => VoiceCaptureError::PermissionDenied(error.to_string()),
        cpal::ErrorKind::DeviceBusy => VoiceCaptureError::DeviceBusy(error.to_string()),
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::HostUnavailable => {
            VoiceCaptureError::NoInputDevice
        }
        _ => VoiceCaptureError::Stream(error.to_string()),
    }
}

/// Explicitly requested microphone capture. Device enumeration, opening,
/// recovery and closing run on a dedicated worker, including initial failures.
/// Merely constructing an audio system never requests microphone permission.
pub struct VoiceCapture {
    control: VoiceCaptureControl,
    frames: CaptureFrameQueue,
    dropped_frames: Arc<AtomicU64>,
    active_generation: Arc<AtomicU64>,
    last_drained_generation: AtomicU64,
    status: Arc<Mutex<VoiceCaptureStatus>>,
    #[cfg(any(feature = "cpal", test))]
    stopped: Arc<AtomicBool>,
}

impl VoiceCapture {
    /// Starts device management without waiting for a native driver or a
    /// permission prompt. Observe [`Self::status`] for asynchronous failures.
    pub fn open(options: VoiceCaptureOptions) -> Result<Self, VoiceCaptureError> {
        #[cfg(feature = "cpal")]
        {
            Self::open_with_backend(options, || CpalVoiceCaptureBackend)
        }
        #[cfg(not(feature = "cpal"))]
        {
            drop(options);
            Err(VoiceCaptureError::Unavailable)
        }
    }

    /// Drains fresh audio without performing device IO. A batch belongs to
    /// exactly one capture generation, even if a device changes concurrently.
    pub fn drain_frames(&self) -> Vec<VoiceInputFrame> {
        if self.control.is_aborted() {
            return Vec::new();
        }
        let generation = self.active_generation.load(Ordering::Acquire);
        let now = Instant::now();
        let frames = std::iter::from_fn(|| self.frames.pop())
            .filter(|queued| queued.callback_generation == generation)
            .map(|queued| queued.frame)
            .filter(|frame| {
                let fresh = frame.is_fresh_at(now);
                if !fresh {
                    self.dropped_frames.fetch_add(1, Ordering::Relaxed);
                }
                fresh
            })
            .collect();
        if generation != 0 && self.active_generation.load(Ordering::Acquire) == generation {
            self.last_drained_generation
                .store(generation, Ordering::Release);
            frames
        } else {
            Vec::new()
        }
    }

    /// Generation associated with the last successful drain. Changes are
    /// published with the batch, never halfway through its consumption.
    pub fn stream_generation(&self) -> u64 {
        self.last_drained_generation.load(Ordering::Acquire)
    }

    pub fn status(&self) -> VoiceCaptureStatus {
        self.status
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Stop acquiring samples at `at`, then drain the bounded processor tail.
    /// Dropping the capture instead aborts and discards all pending speech.
    pub fn finish_at(&self, at: Instant) {
        self.control.finish_at(at);
    }

    pub fn is_finished(&self) -> bool {
        self.control.is_finished()
    }

    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    #[cfg(any(feature = "cpal", test))]
    fn open_with_backend<B, F>(
        options: VoiceCaptureOptions,
        backend: F,
    ) -> Result<Self, VoiceCaptureError>
    where
        B: VoiceCaptureBackend + 'static,
        F: FnOnce() -> B + Send + 'static,
    {
        let frames = Arc::new(crossbeam_queue::ArrayQueue::new(VOICE_CAPTURE_QUEUE_FRAMES));
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let active_generation = Arc::new(AtomicU64::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(VoiceCaptureStatus::Opening));
        let capture = Self {
            control: options.control.clone(),
            frames: frames.clone(),
            dropped_frames: dropped_frames.clone(),
            active_generation: active_generation.clone(),
            last_drained_generation: AtomicU64::new(0),
            status: status.clone(),
            stopped: stopped.clone(),
        };
        std::thread::Builder::new()
            .name("voice-device".to_owned())
            .spawn(move || {
                let mut manager =
                    VoiceCaptureManager::new(backend(), options, frames, dropped_frames);
                manager.active_generation = active_generation;
                manager.stopped = stopped.clone();
                let set_status =
                    |result: Result<(), VoiceCaptureError>, manager: &VoiceCaptureManager<B>| {
                        let next = match result {
                            Err(error) => VoiceCaptureStatus::from_error(&error),
                            Ok(()) if manager.active_generation.load(Ordering::Acquire) != 0 => {
                                VoiceCaptureStatus::Active
                            }
                            Ok(()) => VoiceCaptureStatus::Unavailable,
                        };
                        *status
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
                    };
                set_status(manager.open_initial(), &manager);
                while !stopped.load(Ordering::Acquire) && manager.options.control.is_recording() {
                    let now = Instant::now();
                    let poll_due = now >= manager.next_poll;
                    let had_stream = manager.active.is_some();
                    let result = manager.service(now).map(|_| ());
                    if result.is_err() || poll_due || had_stream != manager.active.is_some() {
                        set_status(result, &manager);
                    }
                    std::thread::park_timeout(Duration::from_millis(5));
                }
                if manager.options.control.finish_time().is_some()
                    && !manager.options.control.is_aborted()
                {
                    // Closing the producer flushes its authorized tail. Keep
                    // this generation valid until the consumer drains it.
                    drop(manager.active.take());
                } else {
                    manager.deactivate();
                }
                manager.options.control.device_closed();
            })
            .map_err(|error| VoiceCaptureError::Stream(error.to_string()))?;
        Ok(capture)
    }
}

impl Drop for VoiceCapture {
    fn drop(&mut self) {
        self.control.abort();
        #[cfg(any(feature = "cpal", test))]
        self.stopped.store(true, Ordering::Release);
        self.active_generation.store(0, Ordering::Release);
    }
}

#[cfg(feature = "cpal")]
struct CpalVoiceCaptureBackend;

#[cfg(feature = "cpal")]
impl VoiceCaptureBackend for CpalVoiceCaptureBackend {
    type Stream = cpal::Stream;

    fn inventory(
        &mut self,
        selected: Option<&VoiceInputDeviceId>,
    ) -> Result<CaptureDeviceInventory, VoiceCaptureError> {
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let default = if selected.is_none() {
            host.default_input_device()
                .map(|device| {
                    device
                        .id()
                        .map(|id| VoiceInputDeviceId(Box::from(id.to_string())))
                        .map_err(cpal_capture_error)
                })
                .transpose()?
        } else {
            None
        };
        let inputs = if selected.is_some() {
            host.input_devices()
                .map_err(cpal_capture_error)?
                .map(|device| {
                    device
                        .id()
                        .map(|id| VoiceInputDeviceId(Box::from(id.to_string())))
                        .map_err(cpal_capture_error)
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        Ok(CaptureDeviceInventory { default, inputs })
    }

    fn open_stream(
        &mut self,
        target: &CaptureDeviceTarget,
        callbacks: CaptureStreamCallbacks,
        options: &VoiceCaptureOptions,
    ) -> Result<Self::Stream, VoiceCaptureError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = match target {
            CaptureDeviceTarget::SystemDefault(expected) => {
                let device = host
                    .default_input_device()
                    .ok_or(VoiceCaptureError::NoInputDevice)?;
                let actual = device
                    .id()
                    .map(|id| VoiceInputDeviceId(Box::from(id.to_string())))
                    .map_err(cpal_capture_error)?;
                if &actual != expected {
                    return Err(VoiceCaptureError::Stream(
                        "system default microphone changed while opening".to_string(),
                    ));
                }
                device
            }
            CaptureDeviceTarget::Exact(selected) => selected
                .as_str()
                .parse::<cpal::DeviceId>()
                .ok()
                .and_then(|id| host.device_by_id(&id))
                .ok_or_else(|| VoiceCaptureError::InputDeviceUnavailable(selected.clone()))?,
        };
        let supported = device.default_input_config().map_err(cpal_capture_error)?;
        validate_capture_config(supported.sample_rate(), supported.channels())?;

        let stream_config = supported.config();
        let processing = options.clone();
        macro_rules! input_stream {
            ($sample:ty) => {
                build_voice_input_stream::<$sample>(&device, stream_config, callbacks, processing)?
            };
        }
        let stream = match supported.sample_format() {
            cpal::SampleFormat::I8 => input_stream!(i8),
            cpal::SampleFormat::I16 => input_stream!(i16),
            cpal::SampleFormat::I24 => input_stream!(cpal::I24),
            cpal::SampleFormat::I32 => input_stream!(i32),
            cpal::SampleFormat::I64 => input_stream!(i64),
            cpal::SampleFormat::U8 => input_stream!(u8),
            cpal::SampleFormat::U16 => input_stream!(u16),
            cpal::SampleFormat::U24 => input_stream!(cpal::U24),
            cpal::SampleFormat::U32 => input_stream!(u32),
            cpal::SampleFormat::U64 => input_stream!(u64),
            cpal::SampleFormat::F32 => input_stream!(f32),
            cpal::SampleFormat::F64 => input_stream!(f64),
            _ => {
                return Err(VoiceCaptureError::Stream(
                    "unsupported non-PCM microphone sample format".to_string(),
                ));
            }
        };
        stream.play().map_err(cpal_capture_error)?;
        Ok(stream)
    }
}

#[cfg(any(feature = "cpal", test))]
fn validate_capture_config(sample_rate: u32, channels: u16) -> Result<(), VoiceCaptureError> {
    if !(MIN_VOICE_CAPTURE_SAMPLE_RATE..=MAX_VOICE_CAPTURE_SAMPLE_RATE).contains(&sample_rate)
        || !(1..=MAX_VOICE_CAPTURE_CHANNELS).contains(&channels)
    {
        return Err(VoiceCaptureError::UnsupportedInputConfig {
            sample_rate,
            channels,
        });
    }
    Ok(())
}

#[cfg(feature = "cpal")]
fn cpal_stream_error_action(kind: cpal::ErrorKind) -> CaptureStreamEventAction {
    match kind {
        cpal::ErrorKind::DeviceChanged => CaptureStreamEventAction::Refresh,
        cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied => CaptureStreamEventAction::Keep,
        _ => CaptureStreamEventAction::Invalidate,
    }
}

#[cfg(feature = "cpal")]
fn build_voice_input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    callbacks: CaptureStreamCallbacks,
    options: VoiceCaptureOptions,
) -> Result<cpal::Stream, VoiceCaptureError>
where
    T: cpal::SizedSample + VoiceInputSample + Send + 'static,
{
    use cpal::traits::DeviceTrait;

    let stopped = callbacks.stopped.clone();
    let control = options.control.clone();
    let error_callbacks = callbacks.clone();
    let raw_frames = Arc::new(crossbeam_queue::ArrayQueue::<RawCapturedFrame>::new(
        VOICE_CAPTURE_QUEUE_FRAMES,
    ));
    let raw_rx = raw_frames.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let raw_closed = closed.clone();
    let dropped_frames = callbacks.dropped_frames.clone();
    // Device callbacks only gather PCM. Codec and echo processing belong to a
    // worker, so neither expensive processing nor its allocations can miss a
    // hardware callback deadline. Dropping the stream closes this worker's
    // only raw producer; it then drains its bounded tail and exits.
    let encoder =
        crate::VoiceEncoder::new().map_err(|error| VoiceCaptureError::Stream(error.to_string()))?;
    let processing_guard = control.processing_guard();
    std::thread::Builder::new()
        .name("voice-capture".to_owned())
        .spawn(move || {
            let _processing_guard = processing_guard;
            let mut sink = ProcessedCaptureSink {
                encoder,
                processing: VoiceProcessing::new(options.processing, options.echo_reference),
                callbacks,
                last_timing: None,
            };
            loop {
                if sink.callbacks.control.is_aborted() {
                    break;
                }
                if let Some(mut frame) = raw_rx.pop() {
                    sink.process(&mut frame.samples, frame.timing);
                } else if raw_closed.load(Ordering::Acquire) {
                    // The producer publishes its final frame before closing.
                    if raw_rx.is_empty() {
                        break;
                    }
                } else {
                    std::thread::park_timeout(Duration::from_millis(2));
                }
            }
            sink.finish();
        })
        .map_err(|error| VoiceCaptureError::Stream(error.to_string()))?;
    let mut processor = VoiceCaptureProcessor::new_with_sink(
        config.sample_rate,
        config.channels,
        RawCaptureSink {
            frames: raw_frames,
            closed,
            dropped_frames,
        },
        control,
    )?;
    device
        .build_input_stream(
            config,
            move |data: &[T], info| {
                if stopped.load(Ordering::Acquire) {
                    return;
                }
                let now = Instant::now();
                let timestamp = info.timestamp();
                let delay = timestamp.callback.duration_since(timestamp.capture);
                let captured_at = now.checked_sub(delay).unwrap_or(now);
                processor.process_interleaved_at(data, captured_at);
            },
            move |error| {
                let action = cpal_stream_error_action(error.kind());
                error_callbacks.report(action);
            },
            None,
        )
        .map_err(cpal_capture_error)
}

#[cfg(any(feature = "cpal", test))]
trait VoiceInputSample: Copy {
    fn to_voice_f32(self) -> f32;
}

#[cfg(any(feature = "cpal", test))]
impl VoiceInputSample for f32 {
    fn to_voice_f32(self) -> f32 {
        self
    }
}

#[cfg(feature = "cpal")]
macro_rules! impl_voice_input_sample {
    ($($sample:ty),+ $(,)?) => {
        $(
            impl VoiceInputSample for $sample {
                fn to_voice_f32(self) -> f32 {
                    <Self as cpal::Sample>::to_sample::<f32>(self)
                }
            }
        )+
    };
}

#[cfg(feature = "cpal")]
impl_voice_input_sample!(
    i8,
    i16,
    cpal::I24,
    i32,
    i64,
    u8,
    u16,
    cpal::U24,
    u32,
    u64,
    f64,
);

#[cfg(any(feature = "cpal", test))]
trait CaptureFrameSink {
    fn process(&mut self, frame: &mut [f32; VOICE_FRAME_SAMPLES], timing: VoiceCaptureTiming);

    fn finish(&mut self) {}
}

#[cfg(any(feature = "cpal", test))]
struct ProcessedCaptureSink {
    encoder: crate::VoiceEncoder,
    processing: VoiceProcessing,
    callbacks: CaptureStreamCallbacks,
    last_timing: Option<VoiceCaptureTiming>,
}

#[cfg(any(feature = "cpal", test))]
impl CaptureFrameSink for ProcessedCaptureSink {
    fn process(&mut self, frame: &mut [f32; VOICE_FRAME_SAMPLES], timing: VoiceCaptureTiming) {
        if Instant::now().saturating_duration_since(timing.captured_at) > MAX_CAPTURE_AGE {
            self.callbacks
                .dropped_frames
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.last_timing = Some(timing);
        let level = self.processing.process_at(frame, timing.captured_at);
        let samples = std::array::from_fn(|index| voice_f32_to_i16(frame[index]));
        match self.encoder.encode(&samples) {
            Ok(payload) => self.callbacks.send_frame(VoiceInputFrame {
                payload,
                level,
                timing: Some(timing),
            }),
            Err(error) => {
                tracing::warn!(%error, "voice encoder could not process capture");
                self.callbacks.report(CaptureStreamEventAction::Invalidate);
            }
        }
    }

    fn finish(&mut self) {
        let Some(mut timing) = self.last_timing.take() else {
            return;
        };
        if self.callbacks.control.finish_time().is_none() || self.callbacks.control.is_aborted() {
            return;
        }
        let lookahead = match self.encoder.lookahead_samples() {
            Ok(samples) => samples,
            Err(error) => {
                tracing::warn!(%error, "voice encoder could not flush capture");
                return;
            }
        };
        let frames = (self.processing.delay_samples() + lookahead).div_ceil(VOICE_FRAME_SAMPLES);
        for _ in 0..frames.min(2) {
            timing.sample_offset = timing
                .sample_offset
                .saturating_add(VOICE_FRAME_SAMPLES as u64);
            // The deadline stays attached to the last real input. Padding
            // releases the DSP/codec tail; it is never new microphone audio.
            self.process(&mut [0.0; VOICE_FRAME_SAMPLES], timing);
        }
        self.last_timing = None;
    }
}

#[cfg(any(feature = "cpal", test))]
struct RawCapturedFrame {
    samples: [f32; VOICE_FRAME_SAMPLES],
    timing: VoiceCaptureTiming,
}

#[cfg(any(feature = "cpal", test))]
struct RawCaptureSink {
    frames: Arc<crossbeam_queue::ArrayQueue<RawCapturedFrame>>,
    closed: Arc<AtomicBool>,
    dropped_frames: Arc<AtomicU64>,
}

#[cfg(any(feature = "cpal", test))]
impl CaptureFrameSink for RawCaptureSink {
    fn process(&mut self, frame: &mut [f32; VOICE_FRAME_SAMPLES], timing: VoiceCaptureTiming) {
        if self
            .frames
            .force_push(RawCapturedFrame {
                samples: *frame,
                timing,
            })
            .is_some()
        {
            self.dropped_frames.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(any(feature = "cpal", test))]
impl Drop for RawCaptureSink {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}

#[cfg(any(feature = "cpal", test))]
struct VoiceCaptureProcessor<S: CaptureFrameSink> {
    control: VoiceCaptureControl,
    finished: bool,
    channels: usize,
    sample_rate: u32,
    frame_capture_time: Option<Instant>,
    sample_offset: u64,
    resampler: StreamingVoiceResampler,
    frame: [f32; VOICE_FRAME_SAMPLES],
    sample_count: usize,
    sink: S,
}

#[cfg(test)]
impl VoiceCaptureProcessor<ProcessedCaptureSink> {
    fn new(
        sample_rate: u32,
        channels: u16,
        sender: CaptureFrameQueue,
        dropped_frames: Arc<AtomicU64>,
        processing: VoiceProcessing,
    ) -> Result<Self, VoiceCaptureError> {
        let (events, _) = std::sync::mpsc::sync_channel(1);
        let control = VoiceCaptureControl::default();
        Self::new_with_sink(
            sample_rate,
            channels,
            ProcessedCaptureSink {
                encoder: crate::VoiceEncoder::new()
                    .map_err(|error| VoiceCaptureError::Stream(error.to_string()))?,
                processing,
                callbacks: CaptureStreamCallbacks {
                    generation: 1,
                    stopped: Arc::new(AtomicBool::new(false)),
                    control: control.clone(),
                    frames: sender,
                    dropped_frames,
                    active_generation: Arc::new(AtomicU64::new(1)),
                    invalidated_generation: Arc::new(AtomicU64::new(0)),
                    route_changed_generation: Arc::new(AtomicU64::new(0)),
                    events,
                },
                last_timing: None,
            },
            control,
        )
    }
}

#[cfg(any(feature = "cpal", test))]
impl<S: CaptureFrameSink> VoiceCaptureProcessor<S> {
    fn new_with_sink(
        sample_rate: u32,
        channels: u16,
        sink: S,
        control: VoiceCaptureControl,
    ) -> Result<Self, VoiceCaptureError> {
        validate_capture_config(sample_rate, channels)?;
        Ok(Self {
            control,
            finished: false,
            channels: usize::from(channels),
            sample_rate,
            frame_capture_time: None,
            sample_offset: 0,
            resampler: StreamingVoiceResampler::new(sample_rate),
            frame: [0.0; VOICE_FRAME_SAMPLES],
            sample_count: 0,
            sink,
        })
    }

    #[cfg(test)]
    fn process_interleaved<T: VoiceInputSample>(&mut self, input: &[T]) {
        self.process_interleaved_at(input, Instant::now());
    }

    fn process_interleaved_at<T: VoiceInputSample>(&mut self, input: &[T], captured_at: Instant) {
        if self.finished {
            return;
        }
        for (index, input_frame) in input.chunks_exact(self.channels).enumerate() {
            let sample_time =
                captured_at + Duration::from_secs_f64(index as f64 / f64::from(self.sample_rate));
            if !self.control.accepts_sample_at(sample_time) {
                break;
            }
            let mono = input_frame
                .iter()
                .map(|sample| sample.to_voice_f32())
                .sum::<f32>()
                / self.channels as f32;
            let Self {
                resampler,
                frame,
                sample_count,
                frame_capture_time,
                sample_offset,
                sink,
                ..
            } = self;
            resampler.push_sample(mono, |sample| {
                let captured_at = *frame_capture_time.get_or_insert(sample_time);
                frame[*sample_count] = sample;
                *sample_count += 1;
                if *sample_count == VOICE_FRAME_SAMPLES {
                    sink.process(
                        frame,
                        VoiceCaptureTiming {
                            captured_at,
                            sample_offset: *sample_offset,
                        },
                    );
                    *sample_offset = sample_offset.saturating_add(VOICE_FRAME_SAMPLES as u64);
                    *sample_count = 0;
                    *frame_capture_time = None;
                }
            });
        }
    }

    fn finish(&mut self) {
        if self.finished || self.control.is_aborted() {
            return;
        }
        self.finished = true;
        if self.sample_count == 0 && self.sample_offset == 0 {
            return;
        }
        let captured_at = self
            .frame_capture_time
            .unwrap_or_else(|| self.control.finish_time().unwrap_or_else(Instant::now));
        // Flush the causal resampler using synthetic silence, never another
        // device read. At most one extra raw frame is needed at supported rates.
        for _ in 0..VOICE_RESAMPLER_TAIL {
            let Self {
                resampler,
                frame,
                sample_count,
                sample_offset,
                sink,
                ..
            } = self;
            resampler.push_sample(0.0, |sample| {
                frame[*sample_count] = sample;
                *sample_count += 1;
                if *sample_count == VOICE_FRAME_SAMPLES {
                    sink.process(
                        frame,
                        VoiceCaptureTiming {
                            captured_at,
                            sample_offset: *sample_offset,
                        },
                    );
                    *sample_offset = sample_offset.saturating_add(VOICE_FRAME_SAMPLES as u64);
                    *sample_count = 0;
                }
            });
        }
        if self.sample_count > 0 {
            self.frame[self.sample_count..].fill(0.0);
            self.sink.process(
                &mut self.frame,
                VoiceCaptureTiming {
                    captured_at,
                    sample_offset: self.sample_offset,
                },
            );
            self.sample_offset = self
                .sample_offset
                .saturating_add(VOICE_FRAME_SAMPLES as u64);
            self.sample_count = 0;
        }
        self.sink.finish();
    }
}

#[cfg(any(feature = "cpal", test))]
impl<S: CaptureFrameSink> Drop for VoiceCaptureProcessor<S> {
    fn drop(&mut self) {
        if self.control.finish_time().is_some() {
            self.finish();
        }
    }
}

/// Streaming, band-limited conversion with an integer clock. Equal device
/// rates pass through exactly; other rates use a prepared causal sinc filter.
#[derive(Debug)]
pub(crate) struct StreamingVoiceResampler {
    source_rate: u32,
    output_rate: u32,
    filter: crate::voice_resampling::SincHistory,
    current_source_index: u128,
    next_output_position: u128,
    started: bool,
}

#[cfg(any(feature = "cpal", test))]
const VOICE_RESAMPLER_TAIL: usize = crate::voice_resampling::TAPS;

impl StreamingVoiceResampler {
    pub(crate) fn new(source_rate: u32) -> Self {
        Self::with_output_rate(source_rate, VOICE_SAMPLE_RATE)
    }

    pub(crate) fn with_output_rate(source_rate: u32, output_rate: u32) -> Self {
        let source_rate = source_rate.max(1);
        let output_rate = output_rate.max(1);
        Self {
            source_rate,
            output_rate,
            filter: crate::voice_resampling::SincHistory::new(source_rate, output_rate),
            current_source_index: 0,
            next_output_position: 0,
            started: false,
        }
    }

    pub(crate) fn push_sample(&mut self, sample: f32, mut emit: impl FnMut(f32)) {
        if self.source_rate == self.output_rate {
            emit(sample);
            return;
        }
        self.filter.push(sample);
        if !self.started {
            self.started = true;
            emit(sample);
            self.next_output_position = u128::from(self.source_rate);
            return;
        }
        self.current_source_index += 1;
        let output_rate = u128::from(self.output_rate);
        let interval_end = self.current_source_index * output_rate;
        let interval_start = interval_end - output_rate;
        while self.next_output_position <= interval_end {
            let fraction = (self.next_output_position - interval_start) as f64 / output_rate as f64;
            emit(self.filter.interpolate(fraction));
            self.next_output_position += u128::from(self.source_rate);
        }
    }
}

#[cfg_attr(not(feature = "cpal"), allow(dead_code))]
pub(crate) fn voice_f32_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32_768.0)
        .round()
        .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

/// Quietest RMS a frame can report before its activation level clamps to zero.
/// Linear amplitude would crowd every useful voice-activation threshold into the
/// bottom few percent of its range, so the level is linear in decibels instead.
const VOICE_ACTIVATION_FLOOR_DBFS: f64 = -60.0;

/// How loud one captured frame is, as `0.0..=1.0` linear in dBFS over
/// [`VOICE_ACTIVATION_FLOOR_DBFS`]`..=0`: `0.0` is silence (or anything at or
/// below the floor) and `1.0` is full scale. This is a presentation and
/// voice-activation measurement only — it never reaches the simulation.
pub fn voice_activation_level(samples: &[i16; VOICE_FRAME_SAMPLES]) -> f32 {
    let mean_square = samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum::<f64>()
        / VOICE_FRAME_SAMPLES as f64;
    voice_level_from_rms(mean_square.sqrt() / 32_768.0)
}

/// [`voice_activation_level`]'s curve, for a root mean square already in
/// `0.0..=1.0` of full scale.
pub(crate) fn voice_level_from_rms(rms: f64) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    let dbfs = 20.0 * rms.log10();
    (1.0 - dbfs / VOICE_ACTIVATION_FLOOR_DBFS).clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_processing::VoiceProcessingConfig;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeCaptureBackend {
        state: Arc<Mutex<FakeCaptureBackendState>>,
    }

    struct FakeCaptureBackendState {
        inventory: CaptureDeviceInventory,
        opens: Vec<CaptureDeviceTarget>,
        callbacks: Vec<CaptureStreamCallbacks>,
        stream_drops: Arc<AtomicU64>,
        input_enumeration_unavailable: bool,
        fail_next_inventory: bool,
        fail_next_open: bool,
        event_during_next_open: Option<CaptureStreamEventAction>,
    }

    struct FakeCaptureStream {
        stream_drops: Arc<AtomicU64>,
    }

    impl Drop for FakeCaptureStream {
        fn drop(&mut self) {
            self.stream_drops.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl FakeCaptureBackend {
        fn new(inventory: CaptureDeviceInventory) -> Self {
            Self {
                state: Arc::new(Mutex::new(FakeCaptureBackendState {
                    inventory,
                    opens: Vec::new(),
                    callbacks: Vec::new(),
                    stream_drops: Arc::new(AtomicU64::new(0)),
                    input_enumeration_unavailable: false,
                    fail_next_inventory: false,
                    fail_next_open: false,
                    event_during_next_open: None,
                })),
            }
        }

        fn opens(&self) -> Vec<CaptureDeviceTarget> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .opens
                .clone()
        }

        fn set_inventory(&self, inventory: CaptureDeviceInventory) {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .inventory = inventory;
        }

        fn callbacks(&self) -> Vec<CaptureStreamCallbacks> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .callbacks
                .clone()
        }

        fn stream_drops(&self) -> u64 {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .stream_drops
                .load(Ordering::Relaxed)
        }

        fn fail_next_open(&self) {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .fail_next_open = true;
        }

        fn fail_next_inventory(&self) {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .fail_next_inventory = true;
        }

        fn report_during_next_open(&self, action: CaptureStreamEventAction) {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .event_during_next_open = Some(action);
        }

        fn make_input_enumeration_unavailable(&self) {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .input_enumeration_unavailable = true;
        }
    }

    impl VoiceCaptureBackend for FakeCaptureBackend {
        type Stream = FakeCaptureStream;

        fn inventory(
            &mut self,
            selected: Option<&VoiceInputDeviceId>,
        ) -> Result<CaptureDeviceInventory, VoiceCaptureError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if std::mem::take(&mut state.fail_next_inventory) {
                return Err(VoiceCaptureError::InputDevices(
                    "injected inventory failure".to_string(),
                ));
            }
            if selected.is_some() && state.input_enumeration_unavailable {
                return Err(VoiceCaptureError::InputDevices(
                    "injected input enumeration failure".to_string(),
                ));
            }
            Ok(state.inventory.clone())
        }

        fn open_stream(
            &mut self,
            target: &CaptureDeviceTarget,
            callbacks: CaptureStreamCallbacks,
            _options: &VoiceCaptureOptions,
        ) -> Result<Self::Stream, VoiceCaptureError> {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.opens.push(target.clone());
            if let Some(action) = state.event_during_next_open.take() {
                callbacks.report(action);
            }
            state.callbacks.push(callbacks);
            let stream = FakeCaptureStream {
                stream_drops: state.stream_drops.clone(),
            };
            if std::mem::take(&mut state.fail_next_open) {
                drop(stream);
                return Err(VoiceCaptureError::Stream(
                    "injected open failure".to_string(),
                ));
            }
            Ok(stream)
        }
    }

    /// The unprocessed capture path, which is what these tests pin: the
    /// downmix, the resampling, the frame geometry and the encoder.
    fn raw_processing() -> VoiceProcessing {
        VoiceProcessing::new(
            VoiceProcessingSwitches::new(VoiceProcessingConfig::DISABLED),
            None,
        )
    }

    fn input_device_id(value: &str) -> VoiceInputDeviceId {
        value.parse().expect("a test CPAL device ID")
    }

    fn input_frame(marker: u8) -> VoiceInputFrame {
        VoiceInputFrame::test_frame(
            crate::test_encode_voice_frame(&[i16::from(marker); VOICE_FRAME_SAMPLES]).unwrap(),
            f32::from(marker) / 255.0,
        )
    }

    type TestCaptureManager = VoiceCaptureManager<FakeCaptureBackend>;

    fn capture_fixture(
        default: Option<VoiceInputDeviceId>,
        inputs: Vec<VoiceInputDeviceId>,
        input_device: Option<VoiceInputDeviceId>,
    ) -> (FakeCaptureBackend, CaptureFrameQueue, TestCaptureManager) {
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory { default, inputs });
        let receiver = Arc::new(crossbeam_queue::ArrayQueue::new(VOICE_CAPTURE_QUEUE_FRAMES));
        let sender = receiver.clone();
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let mut options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        options.input_device = input_device;
        let capture = VoiceCaptureManager::new(
            backend.clone(),
            options,
            sender,
            Arc::clone(&dropped_frames),
        );
        (backend, receiver, capture)
    }

    #[cfg(feature = "cpal")]
    #[test]
    fn streaming_resampling_tracks_a_new_device_clock_without_changing_the_mixer_rate() {
        for output_rate in [8_000, 44_100, 48_000, 96_000, 192_000] {
            let mut converter = StreamingVoiceResampler::with_output_rate(44_100, output_rate);
            let mut count = 0_u32;
            for _ in 0..44_101 {
                converter.push_sample(0.25, |sample| {
                    assert!((sample - 0.25).abs() < 1e-5);
                    count += 1;
                });
            }
            assert!(
                count.abs_diff(output_rate + 1) <= 1,
                "{output_rate} Hz: {count} samples"
            );
        }
    }

    #[test]
    fn saturated_raw_capture_discards_old_audio_before_processing() {
        let receiver = Arc::new(crossbeam_queue::ArrayQueue::new(2));
        let frames = receiver.clone();
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let mut sink = RawCaptureSink {
            frames,
            closed: Arc::new(AtomicBool::new(false)),
            dropped_frames: dropped_frames.clone(),
        };
        for marker in 1..=3 {
            sink.process(
                &mut [marker as f32; VOICE_FRAME_SAMPLES],
                VoiceCaptureTiming {
                    captured_at: Instant::now(),
                    sample_offset: (marker - 1) * VOICE_FRAME_SAMPLES as u64,
                },
            );
        }
        assert_eq!(
            std::iter::from_fn(|| receiver.pop())
                .map(|frame| (frame.samples[0], frame.timing.sample_offset))
                .collect::<Vec<_>>(),
            vec![
                (2.0, VOICE_FRAME_SAMPLES as u64),
                (3.0, 2 * VOICE_FRAME_SAMPLES as u64)
            ]
        );
        assert_eq!(dropped_frames.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn delayed_capture_does_not_send_expired_speech() {
        let default = input_device_id("test:default");
        let (_, receiver, mut capture) =
            capture_fixture(Some(default.clone()), vec![default], None);
        capture.open_initial().unwrap();
        let mut processor = VoiceCaptureProcessor::new(
            VOICE_SAMPLE_RATE,
            1,
            receiver.clone(),
            Arc::new(AtomicU64::new(0)),
            raw_processing(),
        )
        .unwrap();
        processor.process_interleaved(&[0.25; VOICE_FRAME_SAMPLES]);
        std::thread::sleep(Duration::from_millis(180));
        assert!(
            capture.drain_frames(&receiver).is_empty(),
            "speech older than the 160 ms media budget must be discarded"
        );
    }

    #[test]
    fn saturated_capture_retains_the_newest_speech_frames() {
        let default = input_device_id("test:default");
        let (backend, receiver, mut capture) =
            capture_fixture(Some(default.clone()), vec![default], None);
        capture.open_initial().unwrap();
        let callbacks = backend.callbacks().remove(0);
        for marker in 1..=10 {
            callbacks.send_frame(input_frame(marker));
        }
        let frames = capture.drain_frames(&receiver);
        assert_eq!(
            frames.iter().map(|frame| frame.level).collect::<Vec<_>>(),
            (3..=10)
                .map(|marker| input_frame(marker).level)
                .collect::<Vec<_>>()
        );
        assert_eq!(capture.dropped_frames.load(Ordering::Relaxed), 2);
    }

    #[cfg(feature = "cpal")]
    #[test]
    fn capture_status_preserves_actionable_native_device_failures() {
        for (kind, expected) in [
            (
                cpal::ErrorKind::PermissionDenied,
                VoiceCaptureStatus::PermissionDenied,
            ),
            (cpal::ErrorKind::DeviceBusy, VoiceCaptureStatus::DeviceBusy),
            (
                cpal::ErrorKind::DeviceNotAvailable,
                VoiceCaptureStatus::Unavailable,
            ),
        ] {
            assert_eq!(
                VoiceCaptureStatus::from_error(&cpal_capture_error(cpal::Error::new(kind))),
                expected
            );
        }
    }

    #[test]
    fn capture_failure_remains_visible_between_bounded_retries() {
        let default = input_device_id("test:default");
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(default.clone()),
            inputs: vec![default],
        });
        backend.fail_next_open();
        let observed = backend.clone();
        let options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        let capture = VoiceCapture::open_with_backend(options, move || backend).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while observed.opens().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            matches!(capture.status(), VoiceCaptureStatus::Retrying(_)),
            "failure disappeared before retry: {:?}",
            capture.status()
        );
        assert_eq!(
            observed.opens().len(),
            1,
            "device failures must not cause a busy retry loop"
        );
        drop(capture);
        std::thread::sleep(Duration::from_millis(10));
    }

    #[test]
    fn cancelling_capture_during_initialization_never_opens_a_microphone_later() {
        let default = input_device_id("test:default");
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(default.clone()),
            inputs: vec![default],
        });
        let observed = backend.clone();
        let (release, wait) = std::sync::mpsc::channel();
        let options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        let capture = VoiceCapture::open_with_backend(options, move || {
            wait.recv().unwrap();
            backend
        })
        .unwrap();
        drop(capture);
        release.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            observed.opens().is_empty(),
            "cancelled capture opened a microphone after permission was revoked"
        );
    }

    #[test]
    fn release_during_device_initialization_finishes_without_opening_late() {
        let default = input_device_id("test:default");
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(default.clone()),
            inputs: vec![default],
        });
        let observed = backend.clone();
        let (release, wait) = std::sync::mpsc::channel();
        let options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        let control = options.control.clone();
        let capture = VoiceCapture::open_with_backend(options, move || {
            wait.recv().unwrap();
            backend
        })
        .unwrap();
        control.finish_at(Instant::now());
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !capture.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(capture.is_finished());
        assert!(
            observed.opens().is_empty(),
            "a released key cannot open a microphone later"
        );
        assert!(capture.drain_frames().is_empty());
    }

    #[test]
    fn microphone_device_initialization_never_blocks_the_media_caller() {
        let default = input_device_id("test:default");
        let options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        let started = Instant::now();
        let capture = VoiceCapture::open_with_backend(options, move || {
            std::thread::sleep(Duration::from_millis(250));
            FakeCaptureBackend::new(CaptureDeviceInventory {
                default: Some(default.clone()),
                inputs: vec![default],
            })
        })
        .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "device initialization blocked the media caller"
        );
        assert!(capture.drain_frames().is_empty());
        drop(capture);
        // Let the intentionally slow fake driver finish; no detached test leak.
        std::thread::sleep(Duration::from_millis(300));
    }

    #[test]
    fn capture_opens_the_exact_selected_device_even_when_names_collide() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let (backend, _receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first, second.clone()],
            Some(second.clone()),
        );

        capture.open_initial().expect("selected input opens");

        assert_eq!(backend.opens(), [CaptureDeviceTarget::Exact(second)]);
        assert_eq!(capture.stream_generation(), 1);
    }

    #[test]
    fn stream_error_reported_during_open_quarantines_before_activation() {
        for action in [
            CaptureStreamEventAction::Refresh,
            CaptureStreamEventAction::Invalidate,
        ] {
            let default = input_device_id("test:default");
            let (backend, receiver, mut capture) =
                capture_fixture(Some(default.clone()), vec![default], None);
            backend.report_during_next_open(action);

            capture
                .open_initial()
                .expect("the physical stream opened before its callback error");
            backend.callbacks()[0].send_frame(input_frame(1));

            assert!(receiver.pop().is_none(), "{action:?}");
            assert_eq!(capture.active_generation.load(Ordering::Acquire), 0);
            assert!(capture
                .service(Instant::now())
                .expect("the first service rebuilds the quarantined stream"));
            assert_eq!(capture.stream_generation(), 2);
            assert_eq!(backend.opens().len(), 2);
            assert_eq!(backend.stream_drops(), 1);
        }
    }

    #[test]
    fn missing_selected_input_is_reported_without_opening_the_default() {
        let default = input_device_id("test:default");
        let selected = input_device_id("test:missing");
        let (backend, _receiver, mut capture) =
            capture_fixture(Some(default), Vec::new(), Some(selected.clone()));

        assert!(matches!(
            capture.open_initial(),
            Err(VoiceCaptureError::InputDeviceUnavailable(id)) if id == selected
        ));
        assert!(backend.opens().is_empty());
        assert_eq!(capture.stream_generation(), 0);
    }

    #[test]
    fn default_capture_does_not_require_full_device_enumeration() {
        let default = input_device_id("test:default");
        let (backend, _receiver, mut capture) =
            capture_fixture(Some(default.clone()), vec![default.clone()], None);
        backend.make_input_enumeration_unavailable();

        capture
            .open_initial()
            .expect("a usable default does not need enumeration");

        assert_eq!(
            backend.opens(),
            [CaptureDeviceTarget::SystemDefault(default)]
        );
    }

    #[test]
    fn exact_capture_fails_closed_when_device_enumeration_is_unavailable() {
        let selected = input_device_id("test:selected");
        let (backend, _receiver, mut capture) = capture_fixture(
            Some(selected.clone()),
            vec![selected.clone()],
            Some(selected.clone()),
        );
        backend.make_input_enumeration_unavailable();

        assert!(matches!(
            capture.open_initial(),
            Err(VoiceCaptureError::InputDevices(message))
                if message == "injected input enumeration failure"
        ));
        assert!(backend.opens().is_empty());
        assert_eq!(capture.stream_generation(), 0);
    }

    #[test]
    fn removed_selected_device_waits_for_that_device_to_return() {
        let selected = input_device_id("test:selected");
        let other = input_device_id("test:other");
        let (backend, receiver, mut capture) = capture_fixture(
            Some(other.clone()),
            vec![selected.clone(), other.clone()],
            Some(selected.clone()),
        );
        let dropped_frames = Arc::clone(&capture.dropped_frames);
        capture.open_initial().expect("selected input opens");
        backend.callbacks()[0].send_frame(input_frame(1));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(1)]);

        backend.set_inventory(CaptureDeviceInventory {
            default: Some(other.clone()),
            inputs: vec![other.clone()],
        });
        assert!(!capture
            .service(capture.next_poll)
            .expect("the removal poll is handled"));
        assert_eq!(
            backend.opens(),
            [CaptureDeviceTarget::Exact(selected.clone())]
        );
        assert_eq!(backend.stream_drops(), 1);
        assert_eq!(capture.stream_generation(), 1);

        let unrelated = input_device_id("test:unrelated");
        backend.set_inventory(CaptureDeviceInventory {
            default: Some(other),
            inputs: vec![unrelated],
        });
        assert!(!capture
            .service(capture.next_poll)
            .expect("the unrelated addition poll is ignored"));
        assert_eq!(backend.opens().len(), 1);

        backend.set_inventory(CaptureDeviceInventory {
            default: None,
            inputs: vec![selected.clone()],
        });
        assert!(capture
            .service(capture.next_poll)
            .expect("the readdition poll reopens the selected input"));
        assert_eq!(
            backend.opens(),
            [
                CaptureDeviceTarget::Exact(selected.clone()),
                CaptureDeviceTarget::Exact(selected),
            ]
        );
        assert_eq!(capture.stream_generation(), 2);
        backend.callbacks()[1].send_frame(input_frame(2));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(2)]);
        assert_eq!(dropped_frames.load(Ordering::Relaxed), 0);

        drop(capture);
        assert_eq!(backend.stream_drops(), 2);
    }

    #[test]
    fn system_default_capture_follows_a_changed_default_device() {
        let first = input_device_id("test:first-default");
        let second = input_device_id("test:second-default");
        let (backend, receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first.clone(), second.clone()],
            None,
        );
        capture.open_initial().expect("default input opens");

        backend.set_inventory(CaptureDeviceInventory {
            default: Some(second.clone()),
            inputs: vec![first, second.clone()],
        });
        assert!(capture.refresh().expect("new default opens"));

        assert_eq!(
            backend.opens(),
            [
                CaptureDeviceTarget::SystemDefault(input_device_id("test:first-default")),
                CaptureDeviceTarget::SystemDefault(second),
            ]
        );
        assert_eq!(backend.stream_drops(), 1);
        assert_eq!(capture.stream_generation(), 2);
        backend.callbacks()[1].send_frame(input_frame(2));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(2)]);
    }

    #[test]
    fn unchanged_default_identity_does_not_advance_the_stream_generation() {
        let default = input_device_id("test:default-alias");
        let (backend, _receiver, mut capture) =
            capture_fixture(Some(default.clone()), vec![default], None);
        capture.open_initial().expect("default input opens");

        assert!(!capture
            .service(capture.next_poll)
            .expect("an unchanged default scan keeps the stream"));

        assert_eq!(capture.stream_generation(), 1);
        assert_eq!(backend.opens().len(), 1);
        assert_eq!(backend.stream_drops(), 0);
    }

    #[test]
    fn route_change_reopens_an_exact_selection_instead_of_accepting_a_reroute() {
        let selected = input_device_id("test:selected");
        let (backend, _receiver, mut capture) = capture_fixture(
            Some(selected.clone()),
            vec![selected.clone()],
            Some(selected.clone()),
        );
        capture.open_initial().expect("selected input opens");
        let callbacks = backend.callbacks()[0].clone();
        let generation = callbacks.generation;
        for _ in 0..VOICE_CAPTURE_EVENT_QUEUE {
            let _ = callbacks.events.try_send(CaptureStreamEvent {
                generation,
                action: CaptureStreamEventAction::Refresh,
            });
        }
        callbacks.report(CaptureStreamEventAction::Refresh);

        assert!(capture
            .service(Instant::now())
            .expect("route change reopens the exact input"));

        assert_eq!(capture.stream_generation(), 2);
        assert_eq!(
            backend.opens(),
            [
                CaptureDeviceTarget::Exact(selected.clone()),
                CaptureDeviceTarget::Exact(selected),
            ]
        );
        assert_eq!(backend.stream_drops(), 1);
    }

    #[test]
    fn route_event_drained_after_the_atomic_snapshot_still_rebuilds() {
        let selected = input_device_id("test:selected");
        let (backend, _receiver, mut capture) =
            capture_fixture(Some(selected.clone()), vec![selected], None);
        capture.open_initial().expect("default input opens");
        let callbacks = backend.callbacks()[0].clone();

        // A generation-filtered event without its sticky flag models the
        // callback landing between service's atomic snapshot and queue drain.
        callbacks
            .events
            .try_send(CaptureStreamEvent {
                generation: callbacks.generation,
                action: CaptureStreamEventAction::Refresh,
            })
            .expect("the route event queues");

        assert!(capture
            .service(Instant::now())
            .expect("the queued route event rebuilds the stream"));
        assert_eq!(capture.stream_generation(), 2);
        assert_eq!(backend.opens().len(), 2);
        assert_eq!(backend.stream_drops(), 1);
    }

    #[test]
    fn default_route_change_reopens_the_new_default_with_a_clean_generation() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let (backend, receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first.clone(), second.clone()],
            None,
        );
        capture.open_initial().expect("default input opens");
        let callbacks = backend.callbacks()[0].clone();
        callbacks.send_frame(input_frame(1));

        backend.set_inventory(CaptureDeviceInventory {
            default: Some(second.clone()),
            inputs: vec![first, second.clone()],
        });
        callbacks.report(CaptureStreamEventAction::Refresh);

        assert!(capture.drain_frames(&receiver).is_empty());
        assert_eq!(capture.stream_generation(), 2);
        assert_eq!(backend.opens().len(), 2);
        assert_eq!(backend.stream_drops(), 1);
        assert_eq!(
            capture.active.as_ref().map(|active| &active.target),
            Some(&CaptureDeviceTarget::SystemDefault(second))
        );

        callbacks.enqueue_frame_after_activation_check(input_frame(1));
        backend.callbacks()[1].send_frame(input_frame(2));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(2)]);
    }

    #[test]
    fn default_route_inventory_failure_quarantines_frames_and_retries() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let (backend, receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first.clone(), second.clone()],
            None,
        );
        capture.open_initial().expect("default input opens");
        let callbacks = backend.callbacks()[0].clone();
        backend.set_inventory(CaptureDeviceInventory {
            default: Some(second.clone()),
            inputs: vec![first, second.clone()],
        });
        backend.fail_next_inventory();
        callbacks.send_frame(input_frame(1));
        callbacks.report(CaptureStreamEventAction::Refresh);
        callbacks.send_frame(input_frame(2));

        assert!(capture.drain_frames(&receiver).is_empty());
        assert_eq!(capture.stream_generation(), 1);
        assert_eq!(backend.opens().len(), 1);
        assert_eq!(backend.stream_drops(), 1);
        assert!(capture.active.is_none());

        assert!(capture
            .service(capture.next_poll)
            .expect("the default inventory retry succeeds"));
        assert_eq!(capture.stream_generation(), 2);
        assert_eq!(backend.opens().len(), 2);
        assert_eq!(
            capture.active.as_ref().map(|active| &active.target),
            Some(&CaptureDeviceTarget::SystemDefault(second))
        );
        backend.callbacks()[1].send_frame(input_frame(3));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(3)]);
    }

    #[test]
    fn late_callbacks_from_a_replaced_stream_cannot_affect_the_new_generation() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let (backend, receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first.clone(), second.clone()],
            None,
        );
        capture.open_initial().expect("first default opens");
        let old_callbacks = backend.callbacks()[0].clone();

        backend.set_inventory(CaptureDeviceInventory {
            default: Some(second.clone()),
            inputs: vec![first, second],
        });
        assert!(capture.refresh().expect("second default opens"));
        let new_callbacks = backend.callbacks()[1].clone();

        old_callbacks.send_frame(input_frame(1));
        new_callbacks.send_frame(input_frame(2));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(2)]);

        old_callbacks.report(CaptureStreamEventAction::Invalidate);
        assert!(!capture
            .service(Instant::now())
            .expect("a stale error is ignored"));
        assert_eq!(capture.stream_generation(), 2);
        assert_eq!(backend.opens().len(), 2);
        assert_eq!(backend.stream_drops(), 1);
    }

    #[test]
    fn frame_enqueued_by_an_old_callback_after_the_swap_is_filtered() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let (backend, receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first.clone(), second.clone()],
            None,
        );
        capture.open_initial().expect("first default opens");
        let old_callbacks = backend.callbacks()[0].clone();

        backend.set_inventory(CaptureDeviceInventory {
            default: Some(second.clone()),
            inputs: vec![first, second],
        });
        assert!(capture.refresh().expect("second default opens"));
        capture.drain_frames(&receiver);

        // Models preemption after the old callback's active-generation check
        // but before its nonblocking queue send.
        old_callbacks.enqueue_frame_after_activation_check(input_frame(1));
        backend.callbacks()[1].send_frame(input_frame(2));

        assert_eq!(capture.drain_frames(&receiver), [input_frame(2)]);
        assert_eq!(capture.stream_generation(), 2);
    }

    #[test]
    fn quarantine_between_generation_snapshot_and_return_discards_the_drain() {
        let selected = input_device_id("test:selected");
        let (backend, receiver, mut capture) =
            capture_fixture(Some(selected.clone()), vec![selected], None);
        capture.open_initial().expect("default input opens");
        let callbacks = backend.callbacks()[0].clone();
        callbacks.send_frame(input_frame(1));

        let frames = capture.collect_active_frames(&receiver, || {
            callbacks.report(CaptureStreamEventAction::Refresh);
        });

        assert!(frames.is_empty());
        assert_eq!(capture.active_generation.load(Ordering::Acquire), 0);
    }

    #[test]
    fn failed_replacement_keeps_capture_idle_and_retries_without_leaking() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let (backend, receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first.clone(), second.clone()],
            None,
        );
        capture.open_initial().expect("first default opens");
        let stale_callbacks = backend.callbacks()[0].clone();

        backend.set_inventory(CaptureDeviceInventory {
            default: Some(second.clone()),
            inputs: vec![first, second.clone()],
        });
        backend.fail_next_open();
        assert!(matches!(
            capture.refresh(),
            Err(VoiceCaptureError::Stream(message)) if message == "injected open failure"
        ));
        assert_eq!(capture.stream_generation(), 1);
        assert_eq!(backend.stream_drops(), 2);
        stale_callbacks.send_frame(input_frame(1));
        assert!(receiver.pop().is_none());

        assert!(capture.refresh().expect("later retry succeeds"));
        assert_eq!(capture.stream_generation(), 2);
        backend
            .callbacks()
            .last()
            .expect("replacement callbacks")
            .send_frame(input_frame(2));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(2)]);

        drop(capture);
        assert_eq!(backend.stream_drops(), 3);
    }

    #[test]
    fn callback_from_a_failed_open_cannot_invalidate_the_later_retry() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let (backend, _receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first.clone(), second.clone()],
            None,
        );
        capture.open_initial().expect("first default opens");

        backend.set_inventory(CaptureDeviceInventory {
            default: Some(second.clone()),
            inputs: vec![first, second],
        });
        backend.fail_next_open();
        assert!(capture.refresh().is_err());
        let failed_callbacks = backend.callbacks()[1].clone();
        assert!(capture.refresh().expect("retry opens"));
        assert_eq!(capture.stream_generation(), 2);

        failed_callbacks.report(CaptureStreamEventAction::Invalidate);
        assert!(!capture
            .service(Instant::now())
            .expect("the failed attempt's callback is stale"));
        assert_eq!(capture.stream_generation(), 2);
        assert_eq!(backend.opens().len(), 3);
    }

    #[test]
    fn fatal_stream_error_is_not_lost_when_the_event_queue_is_full() {
        let selected = input_device_id("test:selected");
        let (backend, _receiver, mut capture) =
            capture_fixture(Some(selected.clone()), vec![selected], None);
        capture.open_initial().expect("default input opens");
        let callbacks = backend.callbacks()[0].clone();
        for _ in 0..VOICE_CAPTURE_EVENT_QUEUE {
            callbacks.report(CaptureStreamEventAction::Refresh);
        }
        callbacks.report(CaptureStreamEventAction::Invalidate);

        assert!(capture
            .service(Instant::now())
            .expect("fatal error rebuilds the stream"));
        assert_eq!(capture.stream_generation(), 2);
        assert_eq!(backend.opens().len(), 2);
        assert_eq!(backend.stream_drops(), 1);
    }

    #[test]
    fn failed_future_attempt_cannot_mask_the_active_streams_fatal_error() {
        let selected = input_device_id("test:selected");
        let target = CaptureDeviceTarget::SystemDefault(selected.clone());
        let (backend, _receiver, mut capture) =
            capture_fixture(Some(selected.clone()), vec![selected], None);
        capture.open_initial().expect("default input opens");
        let active_callbacks = backend.callbacks()[0].clone();
        for _ in 0..VOICE_CAPTURE_EVENT_QUEUE {
            active_callbacks.report(CaptureStreamEventAction::Refresh);
        }

        backend.fail_next_open();
        assert!(capture.replace_stream(target).is_err());
        let failed_callbacks = backend.callbacks()[1].clone();
        failed_callbacks.report(CaptureStreamEventAction::Invalidate);
        active_callbacks.report(CaptureStreamEventAction::Invalidate);

        assert!(capture
            .service(Instant::now())
            .expect("the active stream's fatal error rebuilds it"));
        assert_eq!(capture.stream_generation(), 2);
        assert_eq!(backend.opens().len(), 3);
        assert_eq!(backend.stream_drops(), 2);
    }

    #[test]
    fn transient_inventory_failure_leaves_a_healthy_stream_active() {
        let selected = input_device_id("test:selected");
        let (backend, receiver, mut capture) =
            capture_fixture(Some(selected.clone()), vec![selected], None);
        capture.open_initial().expect("default input opens");
        let callbacks = backend.callbacks()[0].clone();

        backend.fail_next_inventory();
        assert!(matches!(
            capture.refresh(),
            Err(VoiceCaptureError::InputDevices(message)) if message == "injected inventory failure"
        ));

        callbacks.send_frame(input_frame(1));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(1)]);
        assert_eq!(capture.stream_generation(), 1);
        assert_eq!(backend.opens().len(), 1);
        assert_eq!(backend.stream_drops(), 0);
    }

    #[test]
    fn successful_replacement_clears_frames_queued_by_the_previous_stream() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let (backend, receiver, mut capture) = capture_fixture(
            Some(first.clone()),
            vec![first.clone(), second.clone()],
            None,
        );
        capture.open_initial().expect("first default opens");
        let first_callbacks = backend.callbacks()[0].clone();
        first_callbacks.send_frame(input_frame(1));

        backend.set_inventory(CaptureDeviceInventory {
            default: Some(second),
            inputs: vec![first],
        });
        first_callbacks.report(CaptureStreamEventAction::Refresh);
        assert!(capture.drain_frames(&receiver).is_empty());
        assert_eq!(capture.stream_generation(), 2);

        backend.callbacks()[1].send_frame(input_frame(2));
        assert_eq!(capture.drain_frames(&receiver), [input_frame(2)]);
    }

    #[test]
    fn activation_level_spans_the_sixty_decibel_window_above_silence() {
        assert_eq!(voice_activation_level(&[0; VOICE_FRAME_SAMPLES]), 0.0);

        let mut full_scale = [0; VOICE_FRAME_SAMPLES];
        for (index, sample) in full_scale.iter_mut().enumerate() {
            *sample = if index.is_multiple_of(2) {
                i16::MAX
            } else {
                i16::MIN + 1
            };
        }
        assert!(
            voice_activation_level(&full_scale) > 0.999,
            "a full-scale signal sits at the top of the window",
        );

        // 328/32768 is -39.99 dBFS, which is 20.01 dB above the -60 dBFS floor.
        let level = voice_activation_level(&[328; VOICE_FRAME_SAMPLES]);
        assert!(
            (level - 0.3335).abs() < 0.001,
            "-40 dBFS should land a third of the way up, got {level}",
        );

        assert_eq!(
            voice_activation_level(&[16; VOICE_FRAME_SAMPLES]),
            0.0,
            "anything at or below the -60 dBFS floor clamps to zero",
        );
    }

    #[test]
    fn capture_processor_downmixes_and_stream_resamples_across_callbacks() {
        #[derive(Default)]
        struct SampleSink(Vec<[f32; VOICE_FRAME_SAMPLES]>);
        impl CaptureFrameSink for SampleSink {
            fn process(
                &mut self,
                frame: &mut [f32; VOICE_FRAME_SAMPLES],
                _timing: VoiceCaptureTiming,
            ) {
                self.0.push(*frame);
            }
        }
        let mut processor = VoiceCaptureProcessor::new_with_sink(
            96_000,
            2,
            SampleSink::default(),
            VoiceCaptureControl::default(),
        )
        .unwrap();
        let stereo = [1_000.0 / 32_768.0, 3_000.0 / 32_768.0].repeat(1_920);
        processor.process_interleaved(&stereo[..734]);
        assert!(processor.sink.0.is_empty());
        processor.process_interleaved(&stereo[734..]);
        assert_eq!(processor.sink.0.len(), 1);
        assert!(processor.sink.0[0]
            .iter()
            .all(|sample| (sample * 32_768.0 - 2_000.0).abs() <= 1.0));
    }

    #[test]
    fn releasing_capture_flushes_the_partial_frame_without_recording_after_release() {
        #[derive(Default)]
        struct SampleSink(Vec<[f32; VOICE_FRAME_SAMPLES]>);
        impl CaptureFrameSink for SampleSink {
            fn process(
                &mut self,
                frame: &mut [f32; VOICE_FRAME_SAMPLES],
                _timing: VoiceCaptureTiming,
            ) {
                self.0.push(*frame);
            }
        }
        let control = VoiceCaptureControl::default();
        let start = Instant::now();
        let cutoff = start + Duration::from_millis(25);
        control.finish_at(cutoff);
        let mut processor = VoiceCaptureProcessor::new_with_sink(
            VOICE_SAMPLE_RATE,
            1,
            SampleSink::default(),
            control,
        )
        .unwrap();
        // A delayed callback contains speech from before and after the key
        // release. Only the first 25 ms is authorized.
        let mut samples = vec![0.25; VOICE_FRAME_SAMPLES * 2];
        samples[1_200..].fill(0.75);
        processor.process_interleaved_at(&samples, start);
        processor.finish();
        processor.finish();
        assert_eq!(processor.sink.0.len(), 2, "flush is bounded and idempotent");
        assert!(processor.sink.0[0].iter().all(|&sample| sample == 0.25));
        assert!(processor.sink.0[1][..240]
            .iter()
            .all(|&sample| sample == 0.25));
        assert!(processor.sink.0[1][240..]
            .iter()
            .all(|&sample| sample == 0.0));
    }

    #[test]
    fn released_speech_survives_noise_suppression_and_codec_lookahead() {
        let frames = Arc::new(crossbeam_queue::ArrayQueue::new(8));
        let mut processor = VoiceCaptureProcessor::new(
            VOICE_SAMPLE_RATE,
            1,
            frames.clone(),
            Arc::new(AtomicU64::new(0)),
            VoiceProcessing::new(
                VoiceProcessingSwitches::new(VoiceProcessingConfig::default()),
                None,
            ),
        )
        .unwrap();
        let start = Instant::now();
        let samples = (0..1_200)
            .map(|i| {
                (std::f32::consts::TAU * 800.0 * i as f32 / VOICE_SAMPLE_RATE as f32).sin() * 0.25
            })
            .collect::<Vec<_>>();
        processor.process_interleaved_at(&samples, start);
        processor
            .control
            .finish_at(start + Duration::from_millis(25));
        let lookahead = processor.sink.encoder.lookahead_samples().unwrap();
        processor.finish();
        let mut decoder = crate::VoiceDecoder::new().unwrap();
        let decoded = std::iter::from_fn(|| frames.pop())
            .flat_map(|frame| decoder.decode(&frame.frame.payload, false).unwrap())
            .collect::<Vec<_>>();
        let end = VOICE_FRAME_SAMPLES + lookahead + samples.len();
        assert!(
            decoded.len() >= end,
            "processor delays must not truncate the final syllable"
        );
        let tail_rms = (decoded[end - 200..end]
            .iter()
            .map(|&s| f64::from(s).powi(2))
            .sum::<f64>()
            / 200.0)
            .sqrt();
        assert!(
            tail_rms > 1_000.0,
            "the final speech was lost: RMS {tail_rms}"
        );
        assert!(
            decoded.len() <= VOICE_FRAME_SAMPLES * 4,
            "the tail must stay bounded"
        );
    }

    #[test]
    fn device_rate_conversion_preserves_fullband_speech_without_alias_images() {
        for source_rate in [44_100, 48_000, 96_000, 192_000] {
            let mut converter = StreamingVoiceResampler::new(source_rate);
            let mut output = Vec::new();
            for index in 0..source_rate / 2 {
                let phase =
                    std::f64::consts::TAU * 15_000.0 * f64::from(index) / f64::from(source_rate);
                converter.push_sample(phase.sin() as f32, |sample| output.push(sample));
            }
            let settled = &output[4_800..];
            let (sin, cos) =
                settled
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(sin, cos), (i, value)| {
                        let phase = std::f64::consts::TAU * 15_000.0 * i as f64 / 48_000.0;
                        (
                            sin + f64::from(*value) * phase.sin(),
                            cos + f64::from(*value) * phase.cos(),
                        )
                    });
            let sin = 2.0 * sin / settled.len() as f64;
            let cos = 2.0 * cos / settled.len() as f64;
            let gain = sin.hypot(cos);
            let error = settled
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    let phase = std::f64::consts::TAU * 15_000.0 * i as f64 / 48_000.0;
                    (f64::from(*value) - sin * phase.sin() - cos * phase.cos()).powi(2)
                })
                .sum::<f64>()
                / settled.len() as f64;
            assert!((gain - 1.0).abs() < 0.05, "{source_rate} Hz gain {gain}");
            assert!(
                error.sqrt() < 0.002,
                "{source_rate} Hz interpolation noise {}",
                error.sqrt()
            );
        }
    }

    #[test]
    fn capture_resampling_rejects_frequencies_above_voice_nyquist() {
        let resampled_rms = |frequency_hz: f32| {
            let mut resampler = StreamingVoiceResampler::new(96_000);
            let mut output = Vec::new();
            for index in 0..4_800 {
                let phase = std::f32::consts::TAU * frequency_hz * index as f32 / 96_000.0;
                resampler.push_sample(phase.sin(), |sample| output.push(sample));
            }
            let settled = &output[400..];
            (settled.iter().map(|sample| sample * sample).sum::<f32>() / settled.len() as f32)
                .sqrt()
        };

        let speech = resampled_rms(1_000.0);
        let ultrasonic = resampled_rms(36_000.0);

        assert!(speech > 0.65, "the speech band was attenuated to {speech}");
        assert!(
            ultrasonic < speech * 0.01,
            "36 kHz aliased into the 48 kHz voice signal at {ultrasonic}, versus {speech} in-band",
        );
    }

    #[test]
    fn capture_processor_uses_bounded_try_send_without_blocking() {
        let receiver = Arc::new(crossbeam_queue::ArrayQueue::new(1));
        let sender = receiver.clone();
        let dropped = Arc::new(AtomicU64::new(0));
        let mut processor = VoiceCaptureProcessor::new(
            VOICE_SAMPLE_RATE,
            1,
            sender,
            dropped.clone(),
            raw_processing(),
        )
        .expect("native-rate mono capture should be supported");

        processor.process_interleaved(&vec![0.25_f32; VOICE_FRAME_SAMPLES * 2]);

        assert_eq!(std::iter::from_fn(|| receiver.pop()).count(), 1);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn captured_frames_carry_the_input_level_the_gate_needs() {
        let receiver = Arc::new(crossbeam_queue::ArrayQueue::new(2));
        let sender = receiver.clone();
        let dropped = Arc::new(AtomicU64::new(0));
        let mut processor =
            VoiceCaptureProcessor::new(VOICE_SAMPLE_RATE, 1, sender, dropped, raw_processing())
                .expect("native-rate mono capture should be supported");

        processor.process_interleaved(&[0.0_f32; VOICE_FRAME_SAMPLES]);
        processor.process_interleaved(&[0.25_f32; VOICE_FRAME_SAMPLES]);

        let silent = receiver
            .pop()
            .expect("a silent frame is still captured")
            .frame;
        assert_eq!(silent.level, 0.0);
        let loud = receiver.pop().expect("a loud frame").frame;
        assert!(
            (loud.level - 0.799).abs() < 0.01,
            "a quarter of full scale is -12 dBFS, got {}",
            loud.level,
        );
        assert!(crate::VoiceDecoder::new()
            .unwrap()
            .decode(&loud.payload, false)
            .is_ok());
    }

    #[test]
    fn capture_processor_rejects_unbounded_device_shapes() {
        let make = |sample_rate, channels| {
            let sender = Arc::new(crossbeam_queue::ArrayQueue::new(1));
            VoiceCaptureProcessor::new(
                sample_rate,
                channels,
                sender,
                Arc::new(AtomicU64::new(0)),
                raw_processing(),
            )
        };
        assert!(matches!(
            make(7_999, 1),
            Err(VoiceCaptureError::UnsupportedInputConfig { .. })
        ));
        assert!(matches!(
            make(16_000, 0),
            Err(VoiceCaptureError::UnsupportedInputConfig { .. })
        ));
        assert!(matches!(
            make(16_000, 33),
            Err(VoiceCaptureError::UnsupportedInputConfig { .. })
        ));
    }

    #[cfg(feature = "cpal")]
    #[test]
    fn cpal_stream_errors_only_rebuild_when_the_stream_is_invalid() {
        assert_eq!(
            cpal_stream_error_action(cpal::ErrorKind::DeviceChanged),
            CaptureStreamEventAction::Refresh,
        );
        for kind in [cpal::ErrorKind::Xrun, cpal::ErrorKind::RealtimeDenied] {
            assert_eq!(
                cpal_stream_error_action(kind),
                CaptureStreamEventAction::Keep,
                "{kind:?}",
            );
        }
        for kind in [
            cpal::ErrorKind::DeviceBusy,
            cpal::ErrorKind::DeviceNotAvailable,
            cpal::ErrorKind::HostUnavailable,
            cpal::ErrorKind::InvalidInput,
            cpal::ErrorKind::PermissionDenied,
            cpal::ErrorKind::ResourceExhausted,
            cpal::ErrorKind::StreamInvalidated,
            cpal::ErrorKind::UnsupportedConfig,
            cpal::ErrorKind::UnsupportedOperation,
            cpal::ErrorKind::BackendError,
            cpal::ErrorKind::Other,
        ] {
            assert_eq!(
                cpal_stream_error_action(kind),
                CaptureStreamEventAction::Invalidate,
                "{kind:?}",
            );
        }
    }
}
