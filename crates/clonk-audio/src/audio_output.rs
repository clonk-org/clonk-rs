//! Native output device ownership and callback construction.

use super::*;
use std::sync::atomic::AtomicU64;

const OUTPUT_QUEUE_FRAMES: usize = 16_384;

struct OutputBuffer {
    samples: crossbeam_queue::ArrayQueue<[f32; 2]>,
    active: AtomicBool,
    sample_rate: u32,
    callback_frames: AtomicUsize,
    underruns: AtomicU64,
    errors: crossbeam_queue::ArrayQueue<cpal::Error>,
}

impl OutputBuffer {
    fn new(sample_rate: u32) -> Arc<Self> {
        Arc::new(Self {
            samples: crossbeam_queue::ArrayQueue::new(OUTPUT_QUEUE_FRAMES),
            active: AtomicBool::new(true),
            sample_rate,
            callback_frames: AtomicUsize::new(CLASSIC_OUTPUT_BUFFER_FRAMES as usize),
            underruns: AtomicU64::new(0),
            errors: crossbeam_queue::ArrayQueue::new(16),
        })
    }
}

struct OutputCallback {
    buffer: Arc<OutputBuffer>,
}

impl OutputCallback {
    fn render<T: SampleWrite>(&mut self, data: &mut [T], channels: usize) {
        if channels == 0 {
            data.iter_mut().for_each(SampleWrite::write_zero);
            return;
        }
        self.buffer
            .callback_frames
            .fetch_max(data.len().div_ceil(channels), Ordering::Relaxed);
        if data.len().div_ceil(channels) > OUTPUT_QUEUE_FRAMES / 2 {
            self.buffer.active.store(false, Ordering::Release);
            data.iter_mut().for_each(SampleWrite::write_zero);
            return;
        }
        let mut underrun = false;
        for output in data.chunks_mut(channels) {
            let pcm = if self.buffer.active.load(Ordering::Acquire) {
                self.buffer.samples.pop().unwrap_or_else(|| {
                    underrun = true;
                    [0.0; 2]
                })
            } else {
                [0.0; 2]
            };
            write_stereo_frame(output, pcm[0], pcm[1]);
        }
        if underrun {
            self.buffer.underruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(feature = "cpal")]
pub(super) struct CpalBackend {
    control: Arc<OutputControl>,
}

#[cfg(feature = "cpal")]
fn build_cpal_output_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    buffer: Arc<OutputBuffer>,
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::SizedSample + SampleWrite + Send + 'static,
{
    use cpal::traits::DeviceTrait;

    let output_channels = usize::from(config.channels);
    let mut callback = OutputCallback {
        buffer: buffer.clone(),
    };
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                callback.render(data, output_channels);
            },
            move |error| {
                if !matches!(
                    error.kind(),
                    cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied
                ) {
                    buffer.active.store(false, Ordering::Release);
                }
                // Fatal invalidation is atomic, so an xrun flood cannot hide it.
                buffer.errors.force_push(error);
            },
            None,
        )
        .map_err(|error| AudioError::Stream(error.to_string()))
}

#[cfg(feature = "cpal")]
impl CpalBackend {
    pub(super) fn try_new(
        max_channels: usize,
        resampling_mode: ResamplingMode,
    ) -> Result<(Arc<AudioMixer>, Self), AudioError> {
        Self::with_driver(max_channels, resampling_mode, NativeOutputDriver::new)
    }

    fn with_driver<D: OutputDriver + 'static>(
        max_channels: usize,
        resampling_mode: ResamplingMode,
        make_driver: impl FnOnce() -> D + Send + 'static,
    ) -> Result<(Arc<AudioMixer>, Self), AudioError> {
        let mixer = Arc::new(AudioMixer::new_with_resampling(
            CLASSIC_OUTPUT_SAMPLE_RATE,
            max_channels,
            resampling_mode,
        ));
        let control = OutputControl::new();
        let backend = Self {
            control: control.clone(),
        };
        let device_control = control.clone();
        thread::Builder::new()
            .name("audio-devices".into())
            .spawn(move || {
                let mut manager = OutputManager::new(make_driver(), device_control.clone());
                while device_control.running.load(Ordering::Acquire) {
                    manager.service(Instant::now());
                    thread::sleep(Duration::from_millis(10));
                }
            })
            .map_err(|error| AudioError::Stream(error.to_string()))?;
        let render_mixer = mixer.clone();
        thread::Builder::new()
            .name("audio-render".into())
            .spawn(move || {
                render_output(render_mixer, control);
            })
            .map_err(|error| AudioError::Stream(error.to_string()))?;
        Ok((mixer, backend))
    }

    pub(super) fn status(&self) -> AudioOutputStatus {
        self.control.state.lock().unwrap().status.clone()
    }

    pub(super) fn devices(&self) -> Vec<AudioOutputDevice> {
        self.control.state.lock().unwrap().devices.clone()
    }

    pub(super) fn select(&self, selected: Option<String>) {
        let mut state = self.control.state.lock().unwrap();
        if state.selected != selected {
            state.selected = selected;
            state.invalidate();
            state.revision = state.revision.wrapping_add(1);
            state.status = AudioOutputStatus::Opening;
        }
    }

    pub(super) fn retry(&self) {
        let mut state = self.control.state.lock().unwrap();
        state.invalidate();
        state.revision = state.revision.wrapping_add(1);
        state.status = AudioOutputStatus::Opening;
    }
}

struct NativeOutputDriver {
    host: cpal::Host,
}

impl NativeOutputDriver {
    fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }

    fn try_config(
        device: &cpal::Device,
        config: cpal::SupportedStreamConfig,
    ) -> Result<(cpal::Stream, Arc<OutputBuffer>), AudioError> {
        use cpal::traits::StreamTrait;

        let sample_rate = config.sample_rate();
        let sample_format = config.sample_format();
        let stream_configs = cpal_output_stream_config_candidates(config);

        try_cpal_stream_configs(stream_configs, |stream_config| {
            let buffer = OutputBuffer::new(sample_rate);
            if let cpal::BufferSize::Fixed(frames) = stream_config.buffer_size {
                buffer
                    .callback_frames
                    .store(frames as usize, Ordering::Relaxed);
            }
            let stream = match sample_format {
                cpal::SampleFormat::I8 => {
                    build_cpal_output_stream::<i8>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::I16 => {
                    build_cpal_output_stream::<i16>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::I24 => {
                    build_cpal_output_stream::<cpal::I24>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::I32 => {
                    build_cpal_output_stream::<i32>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::I64 => {
                    build_cpal_output_stream::<i64>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::U8 => {
                    build_cpal_output_stream::<u8>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::U16 => {
                    build_cpal_output_stream::<u16>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::U24 => {
                    build_cpal_output_stream::<cpal::U24>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::U32 => {
                    build_cpal_output_stream::<u32>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::U64 => {
                    build_cpal_output_stream::<u64>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::F32 => {
                    build_cpal_output_stream::<f32>(device, stream_config, buffer.clone())?
                }
                cpal::SampleFormat::F64 => {
                    build_cpal_output_stream::<f64>(device, stream_config, buffer.clone())?
                }
                _ => {
                    return Err(AudioError::Stream(
                        "unsupported audio sample format".to_string(),
                    ));
                }
            };
            stream
                .play()
                .map_err(|err| AudioError::Stream(err.to_string()))?;
            Ok((stream, buffer))
        })
    }
}

impl OutputDriver for NativeOutputDriver {
    type Stream = cpal::Stream;

    fn inventory(&mut self) -> Result<OutputInventory, AudioError> {
        use cpal::traits::{DeviceTrait, HostTrait};
        let default = self
            .host
            .default_output_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());
        let devices = self
            .host
            .output_devices()
            .map_err(|error| AudioError::Stream(error.to_string()))?
            .filter_map(|device| device.id().ok().zip(device.description().ok()))
            .map(|(id, description)| AudioOutputDevice {
                id: id.to_string(),
                name: description.name().into(),
            })
            .collect();
        Ok(OutputInventory { default, devices })
    }

    fn open(&mut self, id: &str) -> Result<(Self::Stream, Arc<OutputBuffer>), AudioError> {
        use cpal::traits::{DeviceTrait, HostTrait};
        let id = id.parse().map_err(|_| AudioError::NoAudioDevice)?;
        let device = self
            .host
            .device_by_id(&id)
            .ok_or(AudioError::NoAudioDevice)?;
        let supported = device
            .supported_output_configs()
            .map_err(|error| AudioError::Stream(error.to_string()))?;
        let configs = cpal_output_config_candidates(supported)
            .into_iter()
            .filter(|config| (8_000..=192_000).contains(&config.sample_rate()))
            .collect::<Vec<_>>();
        if configs.is_empty() {
            return Err(AudioError::Stream(
                "no convertible output format between 8 and 192 kHz with 1 to 8 channels".into(),
            ));
        }
        try_cpal_output_candidates(configs, |config| Self::try_config(&device, config))
    }

    fn play(&mut self, _stream: &Self::Stream) -> Result<(), AudioError> {
        // Native candidate selection includes play: some drivers accept a
        // configuration at build time but reject it when the stream starts.
        Ok(())
    }
}

impl Drop for CpalBackend {
    fn drop(&mut self) {
        self.control.running.store(false, Ordering::Release);
        let mut state = self.control.state.lock().unwrap();
        state.invalidate();
        state.status = AudioOutputStatus::Headless;
    }
}

/// Rendering may lock mixer state or decode music. Only this worker does so;
/// the hardware callback consumes pre-rendered PCM from its bounded queue.
fn render_output(mixer: Arc<AudioMixer>, control: Arc<OutputControl>) {
    let mut pcm = [0.0_f32; 256];
    let mut current: Option<Arc<OutputBuffer>> = None;
    let mut converter = StereoOutputResampler::new(mixer.sample_rate(), mixer.sample_rate());
    let mut idle_until = Instant::now();
    while control.running.load(Ordering::Acquire) {
        let next = control.state.lock().unwrap().buffer.clone();
        let changed = match (&current, &next) {
            (Some(old), Some(new)) => !Arc::ptr_eq(old, new),
            (None, None) => false,
            _ => true,
        };
        if changed {
            converter = StereoOutputResampler::new(
                mixer.sample_rate(),
                next.as_ref()
                    .map_or(mixer.sample_rate(), |buffer| buffer.sample_rate),
            );
            current = next;
            idle_until = Instant::now();
        }
        if let Some(buffer) = current
            .as_ref()
            .filter(|buffer| buffer.active.load(Ordering::Acquire))
        {
            let target = buffer
                .callback_frames
                .load(Ordering::Relaxed)
                .saturating_add(buffer.sample_rate as usize / 500)
                .min(OUTPUT_QUEUE_FRAMES - pcm.len());
            while control.running.load(Ordering::Acquire)
                && buffer.active.load(Ordering::Acquire)
                && buffer.samples.len() < target
            {
                let needed = target - buffer.samples.len();
                let frames = (needed * mixer.sample_rate() as usize / buffer.sample_rate as usize)
                    .clamp(1, pcm.len() / 2);
                mixer.mix_f32(&mut pcm[..frames * 2]);
                for pair in pcm[..frames * 2].chunks_exact(2) {
                    converter.push([pair[0], pair[1]], |frame| {
                        let _ = buffer.samples.push(frame);
                    });
                }
            }
            idle_until = Instant::now();
        } else {
            // Advance sources while output is missing. A native driver blocked
            // in open/close cannot accumulate old speech for the next device.
            let now = Instant::now();
            let elapsed = now
                .saturating_duration_since(idle_until)
                .min(Duration::from_millis(160));
            let frames = (elapsed.as_secs_f64() * f64::from(mixer.sample_rate())) as usize;
            for count in (0..frames).step_by(pcm.len() / 2) {
                let count = (frames - count).min(pcm.len() / 2);
                mixer.mix_f32(&mut pcm[..count * 2]);
            }
            idle_until += Duration::from_secs_f64(frames as f64 / f64::from(mixer.sample_rate()));
            if now.saturating_duration_since(idle_until) > Duration::from_millis(160) {
                idle_until = now;
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

struct StereoOutputResampler {
    left: crate::voice::StreamingVoiceResampler,
    right: crate::voice::StreamingVoiceResampler,
}

impl StereoOutputResampler {
    fn new(source_rate: u32, output_rate: u32) -> Self {
        Self {
            left: crate::voice::StreamingVoiceResampler::with_output_rate(source_rate, output_rate),
            right: crate::voice::StreamingVoiceResampler::with_output_rate(
                source_rate,
                output_rate,
            ),
        }
    }

    fn push(&mut self, frame: [f32; 2], mut emit: impl FnMut([f32; 2])) {
        // The mixer is 44.1 kHz, native rates are capped at 192 kHz.
        let mut left = [0.0_f32; 5];
        let mut count = 0;
        self.left.push_sample(frame[0], |sample| {
            left[count] = sample;
            count += 1;
        });
        let mut index = 0;
        self.right.push_sample(frame[1], |sample| {
            emit([left[index], sample]);
            index += 1;
        });
        debug_assert_eq!(index, count);
    }
}

const OUTPUT_DEVICE_POLL: Duration = Duration::from_secs(1);

#[derive(Default)]
struct OutputInventory {
    default: Option<String>,
    devices: Vec<AudioOutputDevice>,
}

struct OutputControl {
    running: AtomicBool,
    state: Mutex<OutputState>,
}

struct OutputState {
    selected: Option<String>,
    revision: u64,
    buffer: Option<Arc<OutputBuffer>>,
    status: AudioOutputStatus,
    devices: Vec<AudioOutputDevice>,
}

impl OutputState {
    fn invalidate(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            buffer.active.store(false, Ordering::Release);
        }
    }
}

impl OutputControl {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            running: AtomicBool::new(true),
            state: Mutex::new(OutputState {
                selected: None,
                revision: 0,
                buffer: None,
                status: AudioOutputStatus::Opening,
                devices: Vec::new(),
            }),
        })
    }
}

trait OutputDriver {
    type Stream;
    fn inventory(&mut self) -> Result<OutputInventory, AudioError>;
    fn open(&mut self, id: &str) -> Result<(Self::Stream, Arc<OutputBuffer>), AudioError>;
    fn play(&mut self, stream: &Self::Stream) -> Result<(), AudioError>;
}

struct OutputManager<D: OutputDriver> {
    driver: D,
    control: Arc<OutputControl>,
    stream: Option<D::Stream>,
    device: Option<String>,
    revision: u64,
    next_poll: Option<Instant>,
    error_reporter: CpalStreamErrorReporter,
    started: Instant,
}

impl<D: OutputDriver> OutputManager<D> {
    fn new(driver: D, control: Arc<OutputControl>) -> Self {
        Self {
            driver,
            control,
            stream: None,
            device: None,
            revision: 0,
            next_poll: None,
            error_reporter: CpalStreamErrorReporter::default(),
            started: Instant::now(),
        }
    }

    fn service(&mut self, now: Instant) {
        if !self.control.running.load(Ordering::Acquire) {
            return;
        }
        let (requested_revision, buffer) = {
            let state = self.control.state.lock().unwrap();
            (state.revision, state.buffer.clone())
        };
        let revision_changed = self.revision != requested_revision;
        let mut failed = false;
        let mut failure = None;
        if let Some(buffer) = buffer {
            while let Some(error) = buffer.errors.pop() {
                if error.kind() != cpal::ErrorKind::Xrun {
                    failure = Some(error.to_string());
                }
                match self
                    .error_reporter
                    .record(error.kind(), now.saturating_duration_since(self.started))
                {
                    Some(CpalStreamErrorReport::RecoveredXrun {
                        total_occurrences,
                        occurrences_since_previous_report,
                    }) => {
                        tracing::warn!(%error, total_occurrences, occurrences_since_previous_report, "cpal output stream buffer underrun or overrun")
                    }
                    Some(CpalStreamErrorReport::Immediate) => {
                        tracing::warn!(%error, "cpal output stream error")
                    }
                    None => {}
                }
            }
            failed = !buffer.active.load(Ordering::Acquire);
        }
        let mut state = self.control.state.lock().unwrap();
        if !self.control.running.load(Ordering::Acquire) || state.revision != requested_revision {
            return;
        }
        if failed || revision_changed {
            state.invalidate();
            self.device = None;
            self.revision = requested_revision;
            if failed {
                state.status = AudioOutputStatus::Retrying(
                    failure.unwrap_or_else(|| "output stream stopped; reconnecting".into()),
                );
                self.next_poll = Some(now + OUTPUT_DEVICE_POLL);
            } else {
                self.next_poll = None;
            }
            drop(state);
            self.stream = None;
        } else {
            drop(state);
        }
        if self.next_poll.is_some_and(|next| now < next) {
            return;
        }
        self.next_poll = Some(now + OUTPUT_DEVICE_POLL);
        let inventory = match self.driver.inventory() {
            Ok(inventory) => inventory,
            Err(error) => {
                self.failed(requested_revision, now, error);
                return;
            }
        };
        let mut state = self.control.state.lock().unwrap();
        if !self.control.running.load(Ordering::Acquire) || state.revision != requested_revision {
            return;
        }
        state.devices = inventory.devices;
        let target = state
            .selected
            .as_ref()
            .filter(|id| state.devices.iter().any(|device| &device.id == *id))
            .cloned()
            .or_else(|| {
                state
                    .selected
                    .is_none()
                    .then_some(inventory.default)
                    .flatten()
            });
        self.revision = state.revision;
        if self.stream.is_some() && self.device == target {
            return;
        }
        if let Some(buffer) = state.buffer.take() {
            buffer.active.store(false, Ordering::Release);
        }
        self.device = None;
        state.status = if target.is_some() {
            AudioOutputStatus::Opening
        } else {
            AudioOutputStatus::Unavailable
        };
        drop(state);
        self.stream = None;
        let Some(id) = target else {
            return;
        };
        match self.driver.open(&id) {
            Ok((stream, buffer)) => {
                if let Err(error) = self.driver.play(&stream) {
                    self.failed(requested_revision, now, error);
                    return;
                }
                let mut state = self.control.state.lock().unwrap();
                if !self.control.running.load(Ordering::Acquire)
                    || state.revision != requested_revision
                {
                    buffer.active.store(false, Ordering::Release);
                    return;
                }
                state.status = AudioOutputStatus::Active {
                    device: id.clone(),
                    sample_rate: buffer.sample_rate,
                };
                state.buffer = Some(buffer);
                self.device = Some(id);
                self.stream = Some(stream);
            }
            Err(error) => self.failed(requested_revision, now, error),
        }
    }
    fn failed(&mut self, revision: u64, requested_at: Instant, error: AudioError) {
        let mut state = self.control.state.lock().unwrap();
        if self.control.running.load(Ordering::Acquire) && state.revision == revision {
            state.status = AudioOutputStatus::Retrying(error.to_string());
            self.next_poll = Some(Instant::now().max(requested_at) + OUTPUT_DEVICE_POLL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestOutputDriver {
        available: bool,
        opened: usize,
    }
    impl OutputDriver for TestOutputDriver {
        type Stream = ();
        fn inventory(&mut self) -> Result<OutputInventory, AudioError> {
            Ok(if self.available {
                OutputInventory {
                    default: Some("speakers".into()),
                    devices: vec![AudioOutputDevice {
                        id: "speakers".into(),
                        name: "Speakers".into(),
                    }],
                }
            } else {
                OutputInventory::default()
            })
        }
        fn open(&mut self, _: &str) -> Result<((), Arc<OutputBuffer>), AudioError> {
            self.opened += 1;
            Ok(((), OutputBuffer::new(48_000)))
        }
        fn play(&mut self, _: &()) -> Result<(), AudioError> {
            Ok(())
        }
    }

    #[test]
    fn missing_output_recovers_when_a_device_appears() {
        let control = OutputControl::new();
        let mut manager = OutputManager::new(
            TestOutputDriver {
                available: false,
                opened: 0,
            },
            control.clone(),
        );
        let now = Instant::now();
        manager.service(now);
        assert_eq!(
            control.state.lock().unwrap().status,
            AudioOutputStatus::Unavailable
        );
        manager.driver.available = true;
        manager.service(now + OUTPUT_DEVICE_POLL);
        assert!(matches!(
            control.state.lock().unwrap().status,
            AudioOutputStatus::Active { .. }
        ));
        assert_eq!(manager.driver.opened, 1);
    }

    #[test]
    fn output_poll_keeps_a_healthy_stream_and_retires_a_removed_device() {
        let control = OutputControl::new();
        let mut manager = OutputManager::new(
            TestOutputDriver {
                available: true,
                opened: 0,
            },
            control.clone(),
        );
        let now = Instant::now();
        manager.service(now);
        let original = control.state.lock().unwrap().buffer.clone().unwrap();
        manager.service(now + OUTPUT_DEVICE_POLL);
        assert_eq!(
            manager.driver.opened, 1,
            "healthy streams must not be reopened on every inventory poll"
        );
        manager.driver.available = false;
        manager.service(now + 2 * OUTPUT_DEVICE_POLL);
        assert!(!original.active.load(Ordering::Acquire));
        assert!(control.state.lock().unwrap().buffer.is_none());
        assert!(manager.stream.is_none());
    }

    #[test]
    fn failed_output_retries_without_losing_invalidation_to_xruns() {
        let control = OutputControl::new();
        let mut manager = OutputManager::new(
            TestOutputDriver {
                available: true,
                opened: 0,
            },
            control.clone(),
        );
        let now = Instant::now();
        manager.service(now);
        let original = control.state.lock().unwrap().buffer.clone().unwrap();
        original.active.store(false, Ordering::Release);
        original
            .errors
            .force_push(cpal::ErrorKind::StreamInvalidated.into());
        for _ in 0..32 {
            original.errors.force_push(cpal::ErrorKind::Xrun.into());
        }
        manager.service(now + Duration::from_millis(10));
        assert!(matches!(
            control.state.lock().unwrap().status,
            AudioOutputStatus::Retrying(_)
        ));
        assert!(control.state.lock().unwrap().buffer.is_none());
        manager.service(now + Duration::from_millis(500));
        assert_eq!(
            manager.driver.opened, 1,
            "failed devices use bounded retry intervals"
        );
        manager.service(now + OUTPUT_DEVICE_POLL + Duration::from_millis(10));
        assert_eq!(manager.driver.opened, 2);
        assert!(!Arc::ptr_eq(
            &original,
            control.state.lock().unwrap().buffer.as_ref().unwrap()
        ));
    }

    #[test]
    fn oversized_native_callbacks_fail_closed_instead_of_permanently_underrunning() {
        let buffer = OutputBuffer::new(44_100);
        let mut callback = OutputCallback {
            buffer: buffer.clone(),
        };
        let mut pcm = vec![1.0_f32; OUTPUT_QUEUE_FRAMES * 2];
        callback.render(&mut pcm, 2);
        assert!(!buffer.active.load(Ordering::Acquire));
        assert!(pcm.iter().all(|sample| *sample == 0.0));
    }

    struct DelayedOutputDriver {
        entered: std::sync::mpsc::SyncSender<Arc<OutputBuffer>>,
        resume: std::sync::mpsc::Receiver<()>,
        inner: TestOutputDriver,
    }

    impl OutputDriver for DelayedOutputDriver {
        type Stream = ();
        fn inventory(&mut self) -> Result<OutputInventory, AudioError> {
            self.inner.inventory()
        }
        fn open(&mut self, id: &str) -> Result<((), Arc<OutputBuffer>), AudioError> {
            let result = self.inner.open(id)?;
            self.entered.send(result.1.clone()).unwrap();
            self.resume.recv().unwrap();
            Ok(result)
        }
        fn play(&mut self, _: &()) -> Result<(), AudioError> {
            Ok(())
        }
    }

    #[test]
    fn slow_output_open_does_not_block_playback_progress_or_shutdown() {
        let (entered, opening) = std::sync::mpsc::sync_channel(1);
        let (resume, resumed) = std::sync::mpsc::channel();
        let started = Instant::now();
        let (mixer, backend) =
            CpalBackend::with_driver(8, ResamplingMode::Linear, move || DelayedOutputDriver {
                entered,
                resume: resumed,
                inner: TestOutputDriver {
                    available: true,
                    opened: 0,
                },
            })
            .unwrap();
        assert!(started.elapsed() < Duration::from_millis(100));
        let late_buffer = opening.recv_timeout(Duration::from_secs(2)).unwrap();
        mixer.queue_voice_stream_with_mix(7, [100; VOICE_FRAME_SAMPLES], 1.0, 0.0);
        let deadline = Instant::now() + Duration::from_secs(2);
        while mixer.voice_stream_stats(7).queued_frames != 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            mixer.voice_stream_stats(7).queued_frames,
            0,
            "speech expires while native output is unavailable"
        );
        backend.select(Some("missing headphones".into()));
        let control = backend.control.clone();
        let stopped = Instant::now();
        drop(backend);
        assert!(stopped.elapsed() < Duration::from_millis(100));
        resume.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while late_buffer.active.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!late_buffer.active.load(Ordering::Acquire));
        assert!(control.state.lock().unwrap().buffer.is_none());
        assert_eq!(
            control.state.lock().unwrap().status,
            AudioOutputStatus::Headless
        );
    }

    #[test]
    fn selecting_a_missing_output_never_substitutes_the_default_device() {
        let control = OutputControl::new();
        let mut manager = OutputManager::new(
            TestOutputDriver {
                available: true,
                opened: 0,
            },
            control.clone(),
        );
        let backend = CpalBackend {
            control: control.clone(),
        };
        let now = Instant::now();
        manager.service(now);
        let previous = control.state.lock().unwrap().buffer.clone().unwrap();
        backend.select(Some("missing headphones".into()));
        assert!(!previous.active.load(Ordering::Acquire));
        manager.service(now + Duration::from_millis(10));
        assert_eq!(backend.status(), AudioOutputStatus::Unavailable);
        assert_eq!(manager.driver.opened, 1);
        backend.select(None);
        manager.service(now + Duration::from_millis(20));
        assert_eq!(manager.driver.opened, 2);
        let next = control.state.lock().unwrap().buffer.clone().unwrap();
        next.samples.push([0.25, -0.25]).unwrap();
        let mut obsolete = OutputCallback { buffer: previous };
        let mut pcm = [1.0_f32; 2];
        obsolete.render(&mut pcm, 2);
        assert_eq!(pcm, [0.0; 2]);
        assert_eq!(
            next.samples.len(),
            1,
            "an obsolete callback cannot steal from the new device"
        );
    }

    #[test]
    fn an_obsolete_output_failure_cannot_overwrite_shutdown_status() {
        struct CancelDuringOpen(Arc<OutputControl>);
        impl OutputDriver for CancelDuringOpen {
            type Stream = ();
            fn inventory(&mut self) -> Result<OutputInventory, AudioError> {
                TestOutputDriver {
                    available: true,
                    opened: 0,
                }
                .inventory()
            }
            fn open(&mut self, _: &str) -> Result<((), Arc<OutputBuffer>), AudioError> {
                self.0.running.store(false, Ordering::Release);
                self.0.state.lock().unwrap().status = AudioOutputStatus::Headless;
                Err(AudioError::NoAudioDevice)
            }
            fn play(&mut self, _: &()) -> Result<(), AudioError> {
                Ok(())
            }
        }
        let control = OutputControl::new();
        let mut manager = OutputManager::new(CancelDuringOpen(control.clone()), control.clone());
        manager.service(Instant::now());
        assert_eq!(
            control.state.lock().unwrap().status,
            AudioOutputStatus::Headless
        );
    }

    #[test]
    fn output_callback_never_waits_for_the_mixer_lock() {
        let mixer = Arc::new(AudioMixer::new_with_resampling(
            CLASSIC_OUTPUT_SAMPLE_RATE,
            8,
            ResamplingMode::Linear,
        ));
        let buffer = OutputBuffer::new(CLASSIC_OUTPUT_SAMPLE_RATE);
        for _ in 0..2 {
            assert!(buffer.samples.push([0.25, -0.25]).is_ok());
        }
        let mut callback = OutputCallback { buffer };
        let held = mixer.state.lock().unwrap();
        let (ready, started) = std::sync::mpsc::channel();
        let (done, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            let mut samples = [0.0_f32; 4];
            callback.render(&mut samples, 2);
            done.send(samples).unwrap();
        });
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        let samples = result.recv_timeout(Duration::from_millis(100));
        drop(held);
        worker.join().unwrap();
        assert_eq!(
            samples.expect("a hardware callback must never wait on the game or render mutex"),
            [0.25, -0.25, 0.25, -0.25]
        );
    }
}
