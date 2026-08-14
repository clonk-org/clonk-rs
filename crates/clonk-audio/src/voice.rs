use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "cpal")]
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
#[cfg(any(feature = "cpal", test))]
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;
#[cfg(feature = "cpal")]
use std::sync::Mutex;
#[cfg(any(feature = "cpal", test))]
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::voice_echo::VoiceEchoReference;
#[cfg(any(feature = "cpal", test))]
use crate::voice_processing::VoiceProcessing;
use crate::voice_processing::VoiceProcessingSwitches;

/// Voice chat uses independently decodable 20 ms mono frames at 16 kHz.
pub const VOICE_SAMPLE_RATE: u32 = 16_000;
pub const VOICE_FRAME_SAMPLES: usize = 320;

const IMA_HEADER_BYTES: usize = 4;
const IMA_CODE_BYTES: usize = (VOICE_FRAME_SAMPLES - 1).div_ceil(2);

/// Two-byte predictor, one-byte IMA step index, one reserved byte, and 319
/// four-bit deltas. Every frame carries its own predictor/index state.
pub const VOICE_ENCODED_FRAME_BYTES: usize = IMA_HEADER_BYTES + IMA_CODE_BYTES;
pub type EncodedVoiceFrame = [u8; VOICE_ENCODED_FRAME_BYTES];

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
        let devices = host
            .input_devices()
            .map_err(|error| VoiceCaptureError::InputDevices(error.to_string()))?;
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct QueuedVoiceInputFrame {
    callback_generation: u64,
    frame: VoiceInputFrame,
}

/// At most 160 ms of captured audio can wait for the app. The CPAL callback
/// uses `try_send`, so a stalled consumer can never stall the device thread.
pub const VOICE_CAPTURE_QUEUE_FRAMES: usize = 8;
#[cfg(any(feature = "cpal", test))]
const MIN_VOICE_CAPTURE_SAMPLE_RATE: u32 = 8_000;
#[cfg(any(feature = "cpal", test))]
const MAX_VOICE_CAPTURE_SAMPLE_RATE: u32 = 192_000;
#[cfg(any(feature = "cpal", test))]
const MAX_VOICE_CAPTURE_CHANNELS: u16 = 32;

const IMA_INDEX_TABLE: [i8; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

const IMA_STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1_060, 1_166, 1_282, 1_411, 1_552, 1_707, 1_878, 2_066,
    2_272, 2_499, 2_749, 3_024, 3_327, 3_660, 4_026, 4_428, 4_871, 5_358, 5_894, 6_484, 7_132,
    7_845, 8_630, 9_493, 10_442, 11_487, 12_635, 13_899, 15_289, 16_818, 18_500, 20_350, 22_385,
    24_623, 27_086, 29_794, 32_767,
];

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum VoiceCodecError {
    #[error("voice frame has {actual} bytes; expected exactly {expected}")]
    InvalidLength { actual: usize, expected: usize },
    #[error("voice frame has invalid IMA step index {0}")]
    InvalidStepIndex(u8),
    #[error("voice frame reserved byte must be zero")]
    InvalidReservedByte,
    #[error("voice frame has nonzero padding bits")]
    InvalidPadding,
}

#[derive(Debug, Error)]
pub enum VoiceCaptureError {
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
    frames: SyncSender<QueuedVoiceInputFrame>,
    dropped_frames: Arc<AtomicU64>,
    active_generation: Arc<AtomicU64>,
    invalidated_generation: Arc<AtomicU64>,
    route_changed_generation: Arc<AtomicU64>,
    events: SyncSender<CaptureStreamEvent>,
}

#[cfg(any(feature = "cpal", test))]
impl CaptureStreamCallbacks {
    fn send_frame(&self, frame: VoiceInputFrame) {
        if self.active_generation.load(Ordering::Acquire) != self.generation {
            return;
        }
        self.enqueue_frame(self.generation, frame);
    }

    fn enqueue_frame(&self, generation: u64, frame: VoiceInputFrame) {
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.frames.try_send(QueuedVoiceInputFrame {
                callback_generation: generation,
                frame,
            })
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
    options: VoiceCaptureOptions,
    active: Option<ActiveCaptureStream<B::Stream>>,
    active_generation: Arc<AtomicU64>,
    stream_generation: Arc<AtomicU64>,
    next_callback_generation: u64,
    frames: SyncSender<QueuedVoiceInputFrame>,
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
        frames: SyncSender<QueuedVoiceInputFrame>,
        dropped_frames: Arc<AtomicU64>,
    ) -> Self {
        let (event_sender, events) = std::sync::mpsc::sync_channel(VOICE_CAPTURE_EVENT_QUEUE);
        Self {
            backend,
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

    fn open_initial(&mut self) -> Result<(), VoiceCaptureError> {
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
        let callback_generation = self.next_callback_generation;
        self.next_callback_generation = callback_generation.wrapping_add(1).max(1);
        let invalidated_generation = Arc::new(AtomicU64::new(0));
        let route_changed_generation = Arc::new(AtomicU64::new(0));
        let callbacks = CaptureStreamCallbacks {
            generation: callback_generation,
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

    fn drain_frames(&mut self, receiver: &Receiver<QueuedVoiceInputFrame>) -> Vec<VoiceInputFrame> {
        match self.service(Instant::now()) {
            Ok(true) => {
                // A stream generation is an app-visible media boundary. No
                // frame captured before the swap may cross it.
                receiver.try_iter().for_each(drop);
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "microphone stream refresh failed; capture remains idle");
            }
        }
        self.collect_active_frames(receiver, || {})
    }

    fn collect_active_frames(
        &self,
        receiver: &Receiver<QueuedVoiceInputFrame>,
        after_collect: impl FnOnce(),
    ) -> Vec<VoiceInputFrame> {
        let generation_before = self.active_generation.load(Ordering::Acquire);
        let frames = receiver
            .try_iter()
            .filter(|queued| queued.callback_generation == generation_before)
            .map(|queued| queued.frame)
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

    fn stream_generation(&self) -> u64 {
        self.stream_generation.load(Ordering::Acquire)
    }
}

/// How a capture treats what it hears: which processing stages run, and the
/// far-end signal the echo canceller needs.
#[derive(Clone, Debug)]
pub struct VoiceCaptureOptions {
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

/// Explicitly opened microphone capture. Merely constructing [`AudioSystem`](crate::AudioSystem)
/// never opens an input device or requests microphone permission.
pub struct VoiceCapture {
    #[cfg(feature = "cpal")]
    manager: Mutex<VoiceCaptureManager<CpalVoiceCaptureBackend>>,
    frames: Receiver<QueuedVoiceInputFrame>,
    dropped_frames: Arc<AtomicU64>,
}

impl VoiceCapture {
    /// Opens and starts the configured microphone input endpoint. This is the
    /// only production entry point that touches a capture device.
    pub fn open(options: VoiceCaptureOptions) -> Result<Self, VoiceCaptureError> {
        #[cfg(feature = "cpal")]
        {
            Self::open_cpal(options)
        }
        #[cfg(not(feature = "cpal"))]
        {
            drop(options);
            Err(VoiceCaptureError::Unavailable)
        }
    }

    /// Drains every complete frame currently available without waiting.
    ///
    /// This also performs the throttled hotplug check. Call
    /// [`Self::stream_generation`] after draining when the frames need to be
    /// associated with a physical-stream generation.
    pub fn drain_frames(&self) -> Vec<VoiceInputFrame> {
        #[cfg(feature = "cpal")]
        {
            self.manager
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .drain_frames(&self.frames)
        }
        #[cfg(not(feature = "cpal"))]
        {
            self.frames.try_iter().map(|queued| queued.frame).collect()
        }
    }

    /// Generation of the most recently opened physical capture stream.
    ///
    /// The initial stream is generation 1. A successful hotplug replacement
    /// advances it exactly once; scans, failures and idle time leave it alone.
    pub fn stream_generation(&self) -> u64 {
        #[cfg(feature = "cpal")]
        {
            self.manager
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .stream_generation()
        }
        #[cfg(not(feature = "cpal"))]
        {
            0
        }
    }

    /// Frames discarded because the bounded app queue was full.
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    #[cfg(feature = "cpal")]
    fn open_cpal(options: VoiceCaptureOptions) -> Result<Self, VoiceCaptureError> {
        let (sender, frames) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let mut manager = VoiceCaptureManager::new(
            CpalVoiceCaptureBackend,
            options,
            sender,
            dropped_frames.clone(),
        );
        manager.open_initial()?;
        Ok(Self {
            manager: Mutex::new(manager),
            frames,
            dropped_frames,
        })
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
                        .map_err(|error| VoiceCaptureError::InputDevices(error.to_string()))
                })
                .transpose()?
        } else {
            None
        };
        let inputs = if selected.is_some() {
            host.input_devices()
                .map_err(|error| VoiceCaptureError::InputDevices(error.to_string()))?
                .map(|device| {
                    device
                        .id()
                        .map(|id| VoiceInputDeviceId(Box::from(id.to_string())))
                        .map_err(|error| VoiceCaptureError::InputDevices(error.to_string()))
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
                    .map_err(|error| VoiceCaptureError::InputConfig(error.to_string()))?;
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
        let supported = device
            .default_input_config()
            .map_err(|error| VoiceCaptureError::InputConfig(error.to_string()))?;
        validate_capture_config(supported.sample_rate(), supported.channels())?;

        let stream_config = supported.config();
        let processing =
            VoiceProcessing::new(options.processing.clone(), options.echo_reference.clone());
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
        stream
            .play()
            .map_err(|error| VoiceCaptureError::Stream(error.to_string()))?;
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
    processing: VoiceProcessing,
) -> Result<cpal::Stream, VoiceCaptureError>
where
    T: cpal::SizedSample + VoiceInputSample + Send + 'static,
{
    use cpal::traits::DeviceTrait;

    let error_callbacks = callbacks.clone();
    let mut processor = VoiceCaptureProcessor::new_managed(
        config.sample_rate,
        config.channels,
        callbacks,
        processing,
    )?;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| processor.process_interleaved(data),
            move |error| {
                let action = cpal_stream_error_action(error.kind());
                error_callbacks.report(action);
            },
            None,
        )
        .map_err(|error| VoiceCaptureError::Stream(error.to_string()))
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
struct VoiceCaptureProcessor {
    channels: usize,
    resampler: StreamingVoiceResampler,
    /// The frame is gathered as floats because the processing chain works in
    /// them; quantizing to the encoder's integers is the last step.
    frame: [f32; VOICE_FRAME_SAMPLES],
    sample_count: usize,
    processing: VoiceProcessing,
    callbacks: CaptureStreamCallbacks,
}

#[cfg(any(feature = "cpal", test))]
impl VoiceCaptureProcessor {
    #[cfg(test)]
    fn new(
        sample_rate: u32,
        channels: u16,
        sender: SyncSender<QueuedVoiceInputFrame>,
        dropped_frames: Arc<AtomicU64>,
        processing: VoiceProcessing,
    ) -> Result<Self, VoiceCaptureError> {
        let (events, _) = std::sync::mpsc::sync_channel(1);
        Self::new_with_callbacks(
            sample_rate,
            channels,
            CaptureStreamCallbacks {
                generation: 1,
                frames: sender,
                dropped_frames,
                active_generation: Arc::new(AtomicU64::new(1)),
                invalidated_generation: Arc::new(AtomicU64::new(0)),
                route_changed_generation: Arc::new(AtomicU64::new(0)),
                events,
            },
            processing,
        )
    }

    #[cfg(feature = "cpal")]
    fn new_managed(
        sample_rate: u32,
        channels: u16,
        callbacks: CaptureStreamCallbacks,
        processing: VoiceProcessing,
    ) -> Result<Self, VoiceCaptureError> {
        Self::new_with_callbacks(sample_rate, channels, callbacks, processing)
    }

    fn new_with_callbacks(
        sample_rate: u32,
        channels: u16,
        callbacks: CaptureStreamCallbacks,
        processing: VoiceProcessing,
    ) -> Result<Self, VoiceCaptureError> {
        validate_capture_config(sample_rate, channels)?;
        Ok(Self {
            channels: usize::from(channels),
            resampler: StreamingVoiceResampler::new(sample_rate),
            frame: [0.0; VOICE_FRAME_SAMPLES],
            sample_count: 0,
            processing,
            callbacks,
        })
    }

    fn process_interleaved<T: VoiceInputSample>(&mut self, input: &[T]) {
        for input_frame in input.chunks_exact(self.channels) {
            let mono = input_frame
                .iter()
                .map(|sample| sample.to_voice_f32())
                .sum::<f32>()
                / self.channels as f32;
            let Self {
                resampler,
                frame,
                sample_count,
                processing,
                callbacks,
                ..
            } = self;
            resampler.push_sample(mono, |sample| {
                frame[*sample_count] = sample;
                *sample_count += 1;
                if *sample_count == VOICE_FRAME_SAMPLES {
                    let level = processing.process(frame);
                    let mut samples = [0_i16; VOICE_FRAME_SAMPLES];
                    for (quantized, processed) in samples.iter_mut().zip(frame.iter()) {
                        *quantized = voice_f32_to_i16(*processed);
                    }
                    let captured = VoiceInputFrame {
                        payload: encode_voice_frame(&samples),
                        level,
                    };
                    callbacks.send_frame(captured);
                    *sample_count = 0;
                }
            });
        }
    }
}

/// Streaming conversion to [`VOICE_SAMPLE_RATE`], with a low-pass filter before
/// downsampling and linear interpolation between source samples. Capture uses
/// it on the microphone's own rate; the echo reference uses it on the mixer's
/// output rate, so both sides of the canceller see the same conversion.
#[derive(Debug)]
pub(crate) struct StreamingVoiceResampler {
    source_per_output: f64,
    anti_alias: Option<VoiceAntiAliasFilter>,
    previous: Option<f32>,
    current_source_index: u64,
    next_output_position: f64,
}

/// A causal low-pass ahead of downsampling. Linear interpolation alone is not
/// a sample-rate converter when the source is faster: it aliases everything
/// above 8 kHz back into the speech band, where it sounds like noise and no
/// longer matches the echo reference produced by another device rate.
#[derive(Debug)]
struct VoiceAntiAliasFilter {
    coefficients: Box<[f32]>,
    history: Box<[f32]>,
    newest: usize,
    primed: bool,
}

const VOICE_ANTI_ALIAS_TAPS: usize = 127;

impl StreamingVoiceResampler {
    pub(crate) fn new(source_rate: u32) -> Self {
        Self {
            source_per_output: f64::from(source_rate) / f64::from(VOICE_SAMPLE_RATE),
            anti_alias: VoiceAntiAliasFilter::new(source_rate),
            previous: None,
            current_source_index: 0,
            next_output_position: 0.0,
        }
    }

    pub(crate) fn push_sample(&mut self, mut sample: f32, mut emit: impl FnMut(f32)) {
        if let Some(filter) = self.anti_alias.as_mut() {
            sample = filter.process(sample);
        }
        let Some(previous) = self.previous else {
            self.previous = Some(sample);
            emit(sample);
            self.next_output_position = self.source_per_output;
            return;
        };

        self.current_source_index = self.current_source_index.saturating_add(1);
        let interval_end = self.current_source_index as f64;
        let interval_start = interval_end - 1.0;
        while self.next_output_position <= interval_end {
            let fraction = (self.next_output_position - interval_start).clamp(0.0, 1.0) as f32;
            emit(previous + (sample - previous) * fraction);
            self.next_output_position += self.source_per_output;
        }
        self.previous = Some(sample);
    }
}

impl VoiceAntiAliasFilter {
    fn new(source_rate: u32) -> Option<Self> {
        if source_rate <= VOICE_SAMPLE_RATE {
            return None;
        }
        let cutoff = 0.45 * VOICE_SAMPLE_RATE as f64 / f64::from(source_rate);
        let center = (VOICE_ANTI_ALIAS_TAPS - 1) as f64 * 0.5;
        let mut coefficients = (0..VOICE_ANTI_ALIAS_TAPS)
            .map(|index| {
                let offset = index as f64 - center;
                let sinc = if offset == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * offset).sin()
                        / (std::f64::consts::PI * offset)
                };
                let phase =
                    std::f64::consts::TAU * index as f64 / (VOICE_ANTI_ALIAS_TAPS - 1) as f64;
                let blackman = 0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos();
                (sinc * blackman) as f32
            })
            .collect::<Vec<_>>();
        let sum = coefficients.iter().sum::<f32>();
        for coefficient in &mut coefficients {
            *coefficient /= sum;
        }
        Some(Self {
            coefficients: coefficients.into_boxed_slice(),
            history: vec![0.0; VOICE_ANTI_ALIAS_TAPS].into_boxed_slice(),
            newest: VOICE_ANTI_ALIAS_TAPS - 1,
            primed: false,
        })
    }

    fn process(&mut self, sample: f32) -> f32 {
        if !self.primed {
            self.history.fill(sample);
            self.primed = true;
            return sample;
        }
        self.newest = (self.newest + 1) % self.history.len();
        self.history[self.newest] = sample;
        let oldest = (self.newest + 1) % self.history.len();
        let (early, late) = self.history.split_at(oldest);
        self.coefficients
            .iter()
            .zip(late.iter().chain(early))
            .map(|(coefficient, sample)| coefficient * sample)
            .sum()
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

/// Encodes one complete voice frame as self-contained IMA ADPCM.
pub fn encode_voice_frame(samples: &[i16; VOICE_FRAME_SAMPLES]) -> [u8; VOICE_ENCODED_FRAME_BYTES] {
    let mut encoded = [0_u8; VOICE_ENCODED_FRAME_BYTES];
    encoded[..2].copy_from_slice(&samples[0].to_le_bytes());

    let mut predictor = i32::from(samples[0]);
    let mut step_index = initial_ima_step_index(samples);
    encoded[2] = step_index;

    for (code_index, sample) in samples[1..].iter().enumerate() {
        let code = encode_ima_sample(i32::from(*sample), &mut predictor, &mut step_index);
        let byte = &mut encoded[IMA_HEADER_BYTES + code_index / 2];
        if code_index.is_multiple_of(2) {
            *byte = code;
        } else {
            *byte |= code << 4;
        }
    }
    encoded
}

/// Chooses the self-contained frame's initial quantizer state. Reusing index
/// zero every 20 ms forces IMA to reacquire the signal at a 50 Hz cadence, so
/// evaluate every legal state and keep the one with the least frame error.
fn initial_ima_step_index(samples: &[i16; VOICE_FRAME_SAMPLES]) -> u8 {
    (0..IMA_STEP_TABLE.len())
        .min_by_key(|candidate| {
            let mut predictor = i32::from(samples[0]);
            let mut step_index = *candidate as u8;
            samples[1..]
                .iter()
                .map(|sample| {
                    encode_ima_sample(i32::from(*sample), &mut predictor, &mut step_index);
                    i64::from(i32::from(*sample) - predictor)
                        .unsigned_abs()
                        .pow(2)
                })
                .sum::<u64>()
        })
        .unwrap_or_default() as u8
}

/// Decodes one complete self-contained voice frame.
pub fn decode_voice_frame(encoded: &[u8]) -> Result<[i16; VOICE_FRAME_SAMPLES], VoiceCodecError> {
    if encoded.len() != VOICE_ENCODED_FRAME_BYTES {
        return Err(VoiceCodecError::InvalidLength {
            actual: encoded.len(),
            expected: VOICE_ENCODED_FRAME_BYTES,
        });
    }
    if encoded[2] as usize >= IMA_STEP_TABLE.len() {
        return Err(VoiceCodecError::InvalidStepIndex(encoded[2]));
    }
    if encoded[3] != 0 {
        return Err(VoiceCodecError::InvalidReservedByte);
    }
    if encoded[VOICE_ENCODED_FRAME_BYTES - 1] & 0xf0 != 0 {
        return Err(VoiceCodecError::InvalidPadding);
    }

    let mut decoded = [0_i16; VOICE_FRAME_SAMPLES];
    decoded[0] = i16::from_le_bytes([encoded[0], encoded[1]]);
    let mut predictor = i32::from(decoded[0]);
    let mut step_index = encoded[2];
    for code_index in 0..VOICE_FRAME_SAMPLES - 1 {
        let packed = encoded[IMA_HEADER_BYTES + code_index / 2];
        let code = if code_index.is_multiple_of(2) {
            packed & 0x0f
        } else {
            packed >> 4
        };
        decoded[code_index + 1] = decode_ima_sample(code, &mut predictor, &mut step_index);
    }
    Ok(decoded)
}

fn encode_ima_sample(sample: i32, predictor: &mut i32, step_index: &mut u8) -> u8 {
    let step = IMA_STEP_TABLE[usize::from(*step_index)];
    let mut difference = sample - *predictor;
    let mut code = 0_u8;
    if difference < 0 {
        code = 8;
        difference = -difference;
    }

    let mut reconstructed_difference = step >> 3;
    if difference >= step {
        code |= 4;
        difference -= step;
        reconstructed_difference += step;
    }
    if difference >= step >> 1 {
        code |= 2;
        difference -= step >> 1;
        reconstructed_difference += step >> 1;
    }
    if difference >= step >> 2 {
        code |= 1;
        reconstructed_difference += step >> 2;
    }

    if code & 8 != 0 {
        *predictor -= reconstructed_difference;
    } else {
        *predictor += reconstructed_difference;
    }
    *predictor = (*predictor).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
    update_step_index(code, step_index);
    code
}

fn decode_ima_sample(code: u8, predictor: &mut i32, step_index: &mut u8) -> i16 {
    let step = IMA_STEP_TABLE[usize::from(*step_index)];
    let mut difference = step >> 3;
    if code & 4 != 0 {
        difference += step;
    }
    if code & 2 != 0 {
        difference += step >> 1;
    }
    if code & 1 != 0 {
        difference += step >> 2;
    }
    if code & 8 != 0 {
        *predictor -= difference;
    } else {
        *predictor += difference;
    }
    *predictor = (*predictor).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
    update_step_index(code, step_index);
    *predictor as i16
}

fn update_step_index(code: u8, step_index: &mut u8) {
    let next = i16::from(*step_index) + i16::from(IMA_INDEX_TABLE[usize::from(code & 0x0f)]);
    *step_index = next.clamp(0, (IMA_STEP_TABLE.len() - 1) as i16) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_processing::VoiceProcessingConfig;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Mutex};

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
        VoiceInputFrame {
            payload: [marker; VOICE_ENCODED_FRAME_BYTES],
            level: f32::from(marker) / 255.0,
        }
    }

    #[test]
    fn capture_opens_the_exact_selected_device_even_when_names_collide() {
        let first = input_device_id("test:first");
        let second = input_device_id("test:second");
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first, second.clone()],
        });
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        options.input_device = Some(second.clone());
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            options,
            sender,
            Arc::new(AtomicU64::new(0)),
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
            let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
                default: Some(default.clone()),
                inputs: vec![default],
            });
            backend.report_during_next_open(action);
            let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
            let mut capture = VoiceCaptureManager::new(
                backend.clone(),
                VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                    VoiceProcessingConfig::DISABLED,
                )),
                sender,
                Arc::new(AtomicU64::new(0)),
            );

            capture
                .open_initial()
                .expect("the physical stream opened before its callback error");
            backend.callbacks()[0].send_frame(input_frame(1));

            assert!(receiver.try_recv().is_err(), "{action:?}");
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(default),
            inputs: Vec::new(),
        });
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        options.input_device = Some(selected.clone());
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            options,
            sender,
            Arc::new(AtomicU64::new(0)),
        );

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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(default.clone()),
            inputs: vec![default.clone()],
        });
        backend.make_input_enumeration_unavailable();
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
        );

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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(selected.clone()),
            inputs: vec![selected.clone()],
        });
        backend.make_input_enumeration_unavailable();
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        options.input_device = Some(selected);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            options,
            sender,
            Arc::new(AtomicU64::new(0)),
        );

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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(other.clone()),
            inputs: vec![selected.clone(), other.clone()],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let mut options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        options.input_device = Some(selected.clone());
        let mut capture =
            VoiceCaptureManager::new(backend.clone(), options, sender, dropped_frames.clone());
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first.clone(), second.clone()],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(default.clone()),
            inputs: vec![default],
        });
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
        );
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(selected.clone()),
            inputs: vec![selected.clone()],
        });
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
            VoiceProcessingConfig::DISABLED,
        ));
        options.input_device = Some(selected.clone());
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            options,
            sender,
            Arc::new(AtomicU64::new(0)),
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(selected.clone()),
            inputs: vec![selected],
        });
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
        );
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first.clone(), second.clone()],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first.clone(), second.clone()],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first.clone(), second.clone()],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first.clone(), second.clone()],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(selected.clone()),
            inputs: vec![selected],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
        );
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first.clone(), second.clone()],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
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
        assert!(receiver.try_recv().is_err());

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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first.clone(), second.clone()],
        });
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(selected.clone()),
            inputs: vec![selected],
        });
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
        );
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(selected.clone()),
            inputs: vec![selected],
        });
        let (sender, _receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
        );
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(selected.clone()),
            inputs: vec![selected],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
        );
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
        let backend = FakeCaptureBackend::new(CaptureDeviceInventory {
            default: Some(first.clone()),
            inputs: vec![first.clone(), second.clone()],
        });
        let (sender, receiver) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let mut capture = VoiceCaptureManager::new(
            backend.clone(),
            VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
                VoiceProcessingConfig::DISABLED,
            )),
            sender,
            Arc::new(AtomicU64::new(0)),
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
        let (sender, receiver) = mpsc::sync_channel(2);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut processor =
            VoiceCaptureProcessor::new(48_000, 2, sender, dropped.clone(), raw_processing())
                .expect("48 kHz stereo capture should be supported");
        let stereo = [1_000.0 / 32_768.0, 3_000.0 / 32_768.0].repeat(960);

        processor.process_interleaved(&stereo[..734]);
        assert!(receiver.try_recv().is_err());
        processor.process_interleaved(&stereo[734..]);

        let frame = receiver.try_recv().expect("one 20 ms frame").frame;
        let decoded =
            decode_voice_frame(&frame.payload).expect("captured frame should be canonical");
        assert!(decoded.iter().all(|sample| sample.abs_diff(2_000) <= 1));
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn capture_resampling_rejects_frequencies_above_voice_nyquist() {
        let resampled_rms = |frequency_hz: f32| {
            let mut resampler = StreamingVoiceResampler::new(48_000);
            let mut output = Vec::new();
            for index in 0..4_800 {
                let phase = std::f32::consts::TAU * frequency_hz * index as f32 / 48_000.0;
                resampler.push_sample(phase.sin(), |sample| output.push(sample));
            }
            let settled = &output[400..];
            (settled.iter().map(|sample| sample * sample).sum::<f32>() / settled.len() as f32)
                .sqrt()
        };

        let speech = resampled_rms(1_000.0);
        let ultrasonic = resampled_rms(12_000.0);

        assert!(speech > 0.65, "the speech band was attenuated to {speech}");
        assert!(
            ultrasonic < speech * 0.01,
            "12 kHz aliased into the 16 kHz voice signal at {ultrasonic}, versus {speech} in-band",
        );
    }

    #[test]
    fn capture_processor_uses_bounded_try_send_without_blocking() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut processor =
            VoiceCaptureProcessor::new(16_000, 1, sender, dropped.clone(), raw_processing())
                .expect("16 kHz mono capture should be supported");

        processor.process_interleaved(&vec![0.25_f32; VOICE_FRAME_SAMPLES * 2]);

        assert_eq!(receiver.try_iter().count(), 1);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn captured_frames_carry_the_input_level_the_gate_needs() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut processor =
            VoiceCaptureProcessor::new(16_000, 1, sender, dropped, raw_processing())
                .expect("16 kHz mono capture should be supported");

        processor.process_interleaved(&[0.0_f32; VOICE_FRAME_SAMPLES]);
        processor.process_interleaved(&[0.25_f32; VOICE_FRAME_SAMPLES]);

        let silent = receiver
            .try_recv()
            .expect("a silent frame is still captured")
            .frame;
        assert_eq!(silent.level, 0.0);
        let loud = receiver.try_recv().expect("a loud frame").frame;
        assert!(
            (loud.level - 0.799).abs() < 0.01,
            "a quarter of full scale is -12 dBFS, got {}",
            loud.level,
        );
        assert!(decode_voice_frame(&loud.payload).is_ok());
    }

    #[test]
    fn capture_processor_rejects_unbounded_device_shapes() {
        let make = |sample_rate, channels| {
            let (sender, _) = mpsc::sync_channel(1);
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
