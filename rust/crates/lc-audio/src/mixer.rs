use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::decoder::{decode_audio, AudioDecodeError, DecodedAudio, MusicStream};

const SDL_MIXER_MAX_VOLUME: f32 = 128.0;
const SDL_MIXER_MAX_PANNING: f32 = 255.0;
const MAXIMUM_MUSIC_VOLUME: f32 = 80.0;
const MAXIMUM_SOUND_VOLUME: f32 = 100.0;
const MAXIMUM_PANNING_VOLUME: f32 = 192.0;
const MAXIMUM_SOUND_INPUT: f32 = SDL_MIXER_MAX_VOLUME / MAXIMUM_SOUND_VOLUME;
/// SDL_mixer pulls music in callback-sized blocks. Keep the Rust music path
/// similarly bounded instead of retaining one stereo-f32 frame per track
/// frame for the complete duration.
const MUSIC_DECODE_BUFFER_FRAMES: usize = 4_096;

/// Mixer slot plus allocation generation; stale handles cannot control a
/// later sound that reuses the same numeric slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelId(pub usize, pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SoundId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MusicId(u32);

const INERT_SOUND_ID: SoundId = SoundId(0);
const INERT_MUSIC_ID: MusicId = MusicId(0);
const INERT_CHANNEL_INDEX: usize = usize::MAX;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("decode error: {0}")]
    Decode(#[from] AudioDecodeError),
    #[error("no audio output device available")]
    NoAudioDevice,
    #[error("failed to create audio stream: {0}")]
    Stream(String),
    #[error("no free audio channels available")]
    NoFreeChannel,
    #[error("invalid audio channel")]
    InvalidChannel,
}

/// Selects the sample-rate converter used when decoded audio does not match
/// the output device. `Default` leaves the backend's established choice in
/// place; `Linear` explicitly pins the inexpensive two-point interpolator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResamplingMode {
    Default,
    Linear,
}

impl Default for ResamplingMode {
    fn default() -> Self {
        Self::Default
    }
}

/// Unloads the mixer sound when the last `SoundHandle` clone drops.
struct SoundHandleInner {
    mixer: Arc<AudioMixer>,
    id: SoundId,
}

impl Drop for SoundHandleInner {
    fn drop(&mut self) {
        self.mixer.unload_sound(self.id);
    }
}

#[derive(Clone)]
pub struct SoundHandle {
    inner: Arc<SoundHandleInner>,
}

/// Unloads the mixer music when the last `MusicHandle` clone drops.
struct MusicHandleInner {
    mixer: Arc<AudioMixer>,
    id: MusicId,
}

impl Drop for MusicHandleInner {
    fn drop(&mut self) {
        self.mixer.unload_music(self.id);
    }
}

#[derive(Clone)]
pub struct MusicHandle {
    inner: Arc<MusicHandleInner>,
}

impl SoundHandle {
    pub(crate) fn new(mixer: Arc<AudioMixer>, id: SoundId) -> Self {
        Self {
            inner: Arc::new(SoundHandleInner { mixer, id }),
        }
    }

    pub fn duration_ms(&self) -> Option<u32> {
        self.inner.mixer.sound_duration_ms(self.inner.id)
    }

    fn id(&self) -> SoundId {
        self.inner.id
    }
}

impl MusicHandle {
    pub(crate) fn new(mixer: Arc<AudioMixer>, id: MusicId) -> Self {
        Self {
            inner: Arc::new(MusicHandleInner { mixer, id }),
        }
    }

    fn id(&self) -> MusicId {
        self.inner.id
    }
}

pub struct AudioSystem {
    mixer: Arc<AudioMixer>,
    _backend: Backend,
}

#[allow(dead_code)]
enum Backend {
    #[cfg(feature = "cpal")]
    Cpal(CpalBackend),
    Inert,
    Null(NullBackend),
    DeferredNull(Arc<DeferredNullBackend>),
}

impl AudioSystem {
    pub fn new(max_channels: usize) -> Result<Self, AudioError> {
        Self::new_with_resampling(max_channels, ResamplingMode::Default)
    }

    pub fn new_with_resampling(
        max_channels: usize,
        resampling_mode: ResamplingMode,
    ) -> Result<Self, AudioError> {
        #[cfg(feature = "cpal")]
        let system = Self::open_output_or_inert(
            max_channels,
            resampling_mode,
            || {
                let (mixer, backend) = CpalBackend::try_new(max_channels, resampling_mode)?;
                Ok(Self {
                    mixer,
                    _backend: Backend::Cpal(backend),
                })
            },
            |error| tracing::error!("{error}"),
        );

        #[cfg(not(feature = "cpal"))]
        let system = Self::new_inert_with_resampling(max_channels, resampling_mode);

        Ok(system)
    }

    #[cfg(any(feature = "cpal", test))]
    fn open_output_or_inert(
        max_channels: usize,
        resampling_mode: ResamplingMode,
        open_output: impl FnOnce() -> Result<Self, AudioError>,
        log_error: impl FnOnce(&AudioError),
    ) -> Self {
        match open_output() {
            Ok(system) => system,
            Err(error) => {
                log_error(&error);
                Self::new_inert_with_resampling(max_channels, resampling_mode)
            }
        }
    }

    /// C4AudioSystemNone-compatible fallback used only when production output
    /// initialization fails. It owns no worker and its mixer creates inert
    /// placeholder handles without inspecting source bytes.
    fn new_inert_with_resampling(_max_channels: usize, resampling_mode: ResamplingMode) -> Self {
        Self {
            mixer: Arc::new(AudioMixer::new_inert(resampling_mode)),
            _backend: Backend::Inert,
        }
    }

    /// Construct the same live mixer state without opening a platform audio
    /// device. The null backend advances playback in real time, making it
    /// suitable for deterministic tests and headless embedding.
    pub fn new_null(max_channels: usize) -> Self {
        Self::new_null_with_resampling(max_channels, ResamplingMode::Default)
    }

    pub fn new_null_with_resampling(max_channels: usize, resampling_mode: ResamplingMode) -> Self {
        let mixer = Arc::new(AudioMixer::new_with_resampling(
            44_100,
            max_channels,
            resampling_mode,
        ));
        let backend = NullBackend::new(mixer.clone());
        Self {
            mixer,
            _backend: Backend::Null(backend),
        }
    }

    /// Construct a null mixer whose real-time worker starts only when audio
    /// is first played. Loading and configuration remain available while the
    /// backend is dormant, and later playback has the same behavior as
    /// [`Self::new_null`].
    pub fn new_deferred_null(max_channels: usize) -> Self {
        Self::new_deferred_null_with_resampling(max_channels, ResamplingMode::Default)
    }

    pub fn new_deferred_null_with_resampling(
        max_channels: usize,
        resampling_mode: ResamplingMode,
    ) -> Self {
        let mixer = Arc::new(AudioMixer::new_with_resampling(
            44_100,
            max_channels,
            resampling_mode,
        ));
        let backend = Arc::new(DeferredNullBackend::new(mixer.clone()));
        Self {
            mixer,
            _backend: Backend::DeferredNull(backend),
        }
    }

    fn ensure_backend_running(&self) {
        if let Backend::DeferredNull(backend) = &self._backend {
            backend.ensure_running();
        }
    }

    pub fn load_sound(&self, data: &[u8]) -> Result<SoundHandle, AudioError> {
        let id = self.mixer.load_sound(data)?;
        Ok(SoundHandle::new(self.mixer.clone(), id))
    }

    pub fn load_music(&self, data: &[u8]) -> Result<MusicHandle, AudioError> {
        let id = self.mixer.load_music(data)?;
        Ok(MusicHandle::new(self.mixer.clone(), id))
    }

    pub fn load_music_owned(&self, data: Vec<u8>) -> Result<MusicHandle, AudioError> {
        let id = self.mixer.load_music_owned(data)?;
        Ok(MusicHandle::new(self.mixer.clone(), id))
    }

    /// A cheap `Send + Sync` handle onto the shared mixer so bounded decoder
    /// initialization and source parsing can run off the caller's thread.
    pub fn worker_handle(&self) -> AudioWorkerHandle {
        AudioWorkerHandle {
            mixer: self.mixer.clone(),
            deferred_null_backend: match &self._backend {
                Backend::DeferredNull(backend) => Some(backend.clone()),
                _ => None,
            },
        }
    }

    pub fn play_sound(&self, sound: &SoundHandle, looped: bool) -> Result<ChannelId, AudioError> {
        self.ensure_backend_running();
        self.mixer.play_sound(sound.id(), looped)
    }

    pub fn halt_channel(&self, channel: ChannelId) {
        self.mixer.halt_channel(channel);
    }

    pub fn channel_is_playing(&self, channel: ChannelId) -> bool {
        self.mixer.channel_is_playing(channel)
    }

    pub fn channel_set_volume_and_pan(&self, channel: ChannelId, volume: f32, pan: f32) {
        self.mixer.channel_set_volume_and_pan(channel, volume, pan);
    }

    pub fn play_music(&self, music: &MusicHandle, looped: bool) -> Result<(), AudioError> {
        self.ensure_backend_running();
        self.mixer.play_music(music.id(), looped)
    }

    pub fn halt_music(&self) {
        self.mixer.halt_music();
    }

    pub fn music_is_playing(&self) -> bool {
        self.mixer.music_is_playing()
    }

    pub fn music_set_volume(&self, volume: f32) {
        self.mixer.music_set_volume(volume);
    }

    pub fn music_fade_out(&self, duration_ms: u32) -> bool {
        self.mixer.music_fade_out(duration_ms)
    }

    pub fn sound_duration_ms(&self, sound: &SoundHandle) -> Option<u32> {
        sound.duration_ms()
    }

    pub fn set_channel_finished_callback_ffi(
        &self,
        callback: Option<extern "C" fn(i32, *mut std::ffi::c_void)>,
        user_data: *mut std::ffi::c_void,
    ) {
        self.mixer
            .set_channel_finished_callback_ffi(callback, user_data);
    }

    pub fn mixer(&self) -> &Arc<AudioMixer> {
        &self.mixer
    }

    pub fn resampling_mode(&self) -> ResamplingMode {
        self.mixer.resampling_mode
    }
}

#[cfg(feature = "cpal")]
struct CpalBackend {
    _stream: cpal::Stream,
}

#[cfg(feature = "cpal")]
fn select_cpal_stereo_output_config(
    configs: impl IntoIterator<Item = cpal::SupportedStreamConfigRange>,
) -> Option<cpal::SupportedStreamConfig> {
    let range = configs
        .into_iter()
        .filter(|range| {
            range.channels() == 2 && cpal_output_format_priority(range.sample_format()).is_some()
        })
        .max_by_key(|range| {
            (
                cpal_output_format_priority(range.sample_format()).unwrap_or_default(),
                range.contains_rate(48_000),
                range.contains_rate(44_100),
                range.max_sample_rate(),
            )
        })?;

    let config = range
        .try_with_sample_rate(48_000)
        .or_else(|| range.try_with_sample_rate(44_100))
        .unwrap_or_else(|| range.with_max_sample_rate());
    Some(config)
}

#[cfg(feature = "cpal")]
fn cpal_output_format_priority(format: cpal::SampleFormat) -> Option<u8> {
    match format {
        cpal::SampleFormat::F32 => Some(3),
        cpal::SampleFormat::I16 => Some(2),
        cpal::SampleFormat::U16 => Some(1),
        _ => None,
    }
}

#[cfg(feature = "cpal")]
impl CpalBackend {
    fn try_new(
        max_channels: usize,
        resampling_mode: ResamplingMode,
    ) -> Result<(Arc<AudioMixer>, Self), AudioError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoAudioDevice)?;
        let supported_configs = device.supported_output_configs().map_err(|err| {
            AudioError::Stream(format!("failed to enumerate output formats: {err}"))
        })?;
        let config = select_cpal_stereo_output_config(supported_configs).ok_or_else(|| {
            AudioError::Stream(
                "no supported stereo output format (tried f32, i16, and u16)".to_string(),
            )
        })?;
        let sample_rate = config.sample_rate();
        let sample_format = config.sample_format();
        let stream_config = config.config();

        let mixer = Arc::new(AudioMixer::new_with_resampling(
            sample_rate,
            max_channels,
            resampling_mode,
        ));
        let mix_i16 = mixer.clone();
        let mix_f32 = mixer.clone();
        let mix_u16 = mixer.clone();

        let err_fn = |err| {
            tracing::error!(error = %err, "cpal stream error");
        };

        let stream = match sample_format {
            cpal::SampleFormat::I16 => device
                .build_output_stream(
                    stream_config,
                    move |data: &mut [i16], _| {
                        mix_i16.mix_i16(data);
                    },
                    err_fn,
                    None,
                )
                .map_err(|err| AudioError::Stream(err.to_string()))?,
            cpal::SampleFormat::F32 => device
                .build_output_stream(
                    stream_config,
                    move |data: &mut [f32], _| {
                        mix_f32.mix_f32(data);
                    },
                    err_fn,
                    None,
                )
                .map_err(|err| AudioError::Stream(err.to_string()))?,
            cpal::SampleFormat::U16 => device
                .build_output_stream(
                    stream_config,
                    move |data: &mut [u16], _| {
                        mix_u16.mix_u16(data);
                    },
                    err_fn,
                    None,
                )
                .map_err(|err| AudioError::Stream(err.to_string()))?,
            _ => {
                return Err(AudioError::Stream(
                    "unsupported audio sample format".to_string(),
                ));
            }
        };

        stream
            .play()
            .map_err(|err| AudioError::Stream(err.to_string()))?;

        Ok((mixer, Self { _stream: stream }))
    }
}

/// See [`AudioSystem::worker_handle`]: decode-and-play from worker threads.
#[derive(Clone)]
pub struct AudioWorkerHandle {
    mixer: Arc<AudioMixer>,
    deferred_null_backend: Option<Arc<DeferredNullBackend>>,
}

impl AudioWorkerHandle {
    pub fn load_music(&self, data: &[u8]) -> Result<MusicHandle, AudioError> {
        let id = self.mixer.load_music(data)?;
        Ok(MusicHandle::new(self.mixer.clone(), id))
    }

    /// Transfers the compressed bytes into the streaming decoder without a
    /// second full-source copy.
    pub fn load_music_owned(&self, data: Vec<u8>) -> Result<MusicHandle, AudioError> {
        let id = self.mixer.load_music_owned(data)?;
        Ok(MusicHandle::new(self.mixer.clone(), id))
    }

    pub fn play_music(&self, music: &MusicHandle, looped: bool) -> Result<(), AudioError> {
        if let Some(backend) = &self.deferred_null_backend {
            backend.ensure_running();
        }
        self.mixer.play_music(music.id(), looped)
    }

    pub fn halt_music(&self) {
        self.mixer.halt_music();
    }

    pub fn music_set_volume(&self, volume: f32) {
        self.mixer.music_set_volume(volume);
    }
}

struct DeferredNullBackend {
    mixer: Arc<AudioMixer>,
    backend: OnceLock<NullBackend>,
}

impl DeferredNullBackend {
    fn new(mixer: Arc<AudioMixer>) -> Self {
        Self {
            mixer,
            backend: OnceLock::new(),
        }
    }

    fn ensure_running(&self) {
        self.backend
            .get_or_init(|| NullBackend::new(self.mixer.clone()));
    }
}

struct NullBackend {
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl NullBackend {
    fn new(mixer: Arc<AudioMixer>) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let thread_running = running.clone();
        let sample_rate = mixer.sample_rate();
        let backend_mixer = mixer;
        let handle = thread::spawn(move || {
            let frames_per_chunk = 512usize;
            let mut buffer = vec![0i16; frames_per_chunk * 2];
            let sleep_duration = if sample_rate > 0 {
                Duration::from_secs_f64(frames_per_chunk as f64 / sample_rate as f64)
            } else {
                Duration::from_millis(10)
            };
            while thread_running.load(Ordering::Acquire) {
                buffer.fill(0);
                backend_mixer.mix_i16(&mut buffer);
                thread::sleep(sleep_duration);
            }
        });

        Self {
            running,
            thread: Some(handle),
        }
    }
}

impl Drop for NullBackend {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Clone)]
pub struct AudioMixer {
    state: Arc<Mutex<MixerState>>,
    channel_finished: Arc<RwLock<Option<ChannelFinished>>>,
    sample_rate: u32,
    resampling_mode: ResamplingMode,
    inert: bool,
}

struct MixerState {
    sounds: HashMap<SoundId, Arc<AudioClip>>,
    music: HashMap<MusicId, Arc<MusicAsset>>,
    channels: Vec<Option<ChannelPlayback>>,
    channel_generations: Vec<u64>,
    active_music: Option<MusicPlayback>,
    next_sound_id: u32,
    next_music_id: u32,
    next_inert_channel_generation: u64,
}

#[derive(Debug)]
struct AudioClip {
    frames: Arc<Vec<[f32; 2]>>,
}

/// A loaded music object retains the compressed source, as C4's SDL_mixer
/// backend does. The first prepared decoder makes `play_music` cheap; replay
/// constructs another bounded decoder from the same source.
struct MusicAsset {
    source: Arc<[u8]>,
    prepared: Mutex<Option<MusicStream>>,
}

#[derive(Debug)]
struct ChannelPlayback {
    clip: Arc<AudioClip>,
    position: usize,
    looping: bool,
    volume: f32,
    pan: f32,
    left_gain: f32,
    right_gain: f32,
}

#[derive(Debug)]
struct FadeState {
    remaining_samples: usize,
    total_samples: usize,
}

struct MusicPlayback {
    stream: MusicStream,
    decode_buffer: Box<[[f32; 2]]>,
    buffer_position: usize,
    buffer_length: usize,
    looping: bool,
    volume: f32,
    fade_out: Option<FadeState>,
}

#[derive(Clone)]
enum ChannelFinished {
    Ffi {
        callback: extern "C" fn(i32, *mut std::ffi::c_void),
        user_data: *mut std::ffi::c_void,
    },
    #[allow(dead_code)]
    Rust(Arc<dyn Fn(usize) + Send + Sync>),
}

unsafe impl Send for ChannelFinished {}
unsafe impl Sync for ChannelFinished {}

impl AudioMixer {
    pub fn new(sample_rate: u32, max_channels: usize) -> Self {
        Self::new_with_resampling(sample_rate, max_channels, ResamplingMode::Default)
    }

    pub(crate) fn new_with_resampling(
        sample_rate: u32,
        max_channels: usize,
        resampling_mode: ResamplingMode,
    ) -> Self {
        Self::new_inner(sample_rate, max_channels, resampling_mode, false)
    }

    fn new_inert(resampling_mode: ResamplingMode) -> Self {
        Self::new_inner(44_100, 0, resampling_mode, true)
    }

    fn new_inner(
        sample_rate: u32,
        max_channels: usize,
        resampling_mode: ResamplingMode,
        inert: bool,
    ) -> Self {
        let state = MixerState {
            sounds: HashMap::new(),
            music: HashMap::new(),
            channels: (0..max_channels).map(|_| None).collect(),
            channel_generations: vec![0; max_channels],
            active_music: None,
            next_sound_id: 1,
            next_music_id: 1,
            next_inert_channel_generation: 1,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            channel_finished: Arc::new(RwLock::new(None)),
            sample_rate,
            resampling_mode,
            inert,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub(crate) fn load_sound(&self, data: &[u8]) -> Result<SoundId, AudioError> {
        if self.inert {
            return Ok(INERT_SOUND_ID);
        }
        let decoded = decode_audio(data)?;
        let clip = self.prepare_clip(decoded);
        let mut state = self.state.lock().unwrap();
        let id = SoundId(state.next_sound_id);
        state.next_sound_id += 1;
        state.sounds.insert(id, clip);
        Ok(id)
    }

    pub(crate) fn load_music(&self, data: &[u8]) -> Result<MusicId, AudioError> {
        if self.inert {
            return Ok(INERT_MUSIC_ID);
        }
        self.load_music_owned(data.to_vec())
    }

    pub(crate) fn load_music_owned(&self, data: Vec<u8>) -> Result<MusicId, AudioError> {
        if self.inert {
            return Ok(INERT_MUSIC_ID);
        }
        let source: Arc<[u8]> = Arc::from(data.into_boxed_slice());
        // Opening validates the format and performs only bounded decoder
        // initialization. In particular, MIDI parses its event schedule but
        // does not synthesize a duration-sized PCM vector.
        let prepared = MusicStream::open(source.clone(), self.sample_rate)?;
        let asset = Arc::new(MusicAsset {
            source,
            prepared: Mutex::new(Some(prepared)),
        });
        let mut state = self.state.lock().unwrap();
        let id = MusicId(state.next_music_id);
        state.next_music_id += 1;
        state.music.insert(id, asset);
        Ok(id)
    }

    pub(crate) fn unload_sound(&self, id: SoundId) {
        if self.inert {
            return;
        }
        let mut state = self.state.lock().unwrap();
        state.sounds.remove(&id);
    }

    pub(crate) fn unload_music(&self, id: MusicId) {
        if self.inert {
            return;
        }
        let mut state = self.state.lock().unwrap();
        state.music.remove(&id);
    }

    pub(crate) fn sound_duration_ms(&self, id: SoundId) -> Option<u32> {
        if self.inert {
            return Some(0);
        }
        let state = self.state.lock().unwrap();
        state.sounds.get(&id).map(|clip| {
            let frames = clip.frames.len();
            if self.sample_rate == 0 {
                0
            } else {
                ((frames as u64 * 1000) / self.sample_rate as u64) as u32
            }
        })
    }

    pub(crate) fn play_sound(&self, id: SoundId, looped: bool) -> Result<ChannelId, AudioError> {
        if self.inert {
            let mut state = self.state.lock().unwrap();
            let generation = state.next_inert_channel_generation;
            state.next_inert_channel_generation = generation.wrapping_add(1);
            if state.next_inert_channel_generation == 0 {
                state.next_inert_channel_generation = 1;
            }
            return Ok(ChannelId(INERT_CHANNEL_INDEX, generation));
        }
        let mut state = self.state.lock().unwrap();
        let clip = state
            .sounds
            .get(&id)
            .cloned()
            .ok_or(AudioError::InvalidChannel)?;
        let channel_index = state
            .channels
            .iter()
            .position(|slot| slot.is_none())
            .ok_or(AudioError::NoFreeChannel)?;
        let mut playback = ChannelPlayback {
            clip,
            position: 0,
            looping: looped,
            volume: 1.0,
            pan: 0.0,
            left_gain: 1.0,
            right_gain: 1.0,
        };
        playback.recalculate_gains();
        let mut generation = state.channel_generations[channel_index].wrapping_add(1);
        if generation == 0 {
            generation = 1;
        }
        state.channel_generations[channel_index] = generation;
        state.channels[channel_index] = Some(playback);
        Ok(ChannelId(channel_index, generation))
    }

    pub fn halt_channel(&self, channel: ChannelId) {
        let mut state = self.state.lock().unwrap();
        if state.channel_generations.get(channel.0) != Some(&channel.1) {
            return;
        }
        if let Some(slot) = state.channels.get_mut(channel.0) {
            *slot = None;
        }
    }

    pub fn channel_is_playing(&self, channel: ChannelId) -> bool {
        let state = self.state.lock().unwrap();
        state.channel_generations.get(channel.0) == Some(&channel.1)
            && state
                .channels
                .get(channel.0)
                .and_then(|slot| slot.as_ref())
                .is_some()
    }

    pub fn channel_set_volume_and_pan(&self, channel: ChannelId, volume: f32, pan: f32) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.channel_generations.get(channel.0) != Some(&channel.1) {
            return false;
        }
        let Some(Some(playback)) = state.channels.get_mut(channel.0) else {
            return false;
        };
        playback.volume = volume.clamp(0.0, MAXIMUM_SOUND_INPUT);
        playback.pan = pan.clamp(-1.0, 1.0);
        playback.recalculate_gains();
        true
    }

    pub(crate) fn play_music(&self, id: MusicId, looped: bool) -> Result<(), AudioError> {
        if self.inert {
            return Ok(());
        }
        let asset = {
            let state = self.state.lock().unwrap();
            state
                .music
                .get(&id)
                .cloned()
                .ok_or(AudioError::InvalidChannel)?
        };
        let stream = asset
            .prepared
            .lock()
            .unwrap()
            .take()
            .map(Ok)
            .unwrap_or_else(|| MusicStream::open(asset.source.clone(), self.sample_rate))?;

        let mut state = self.state.lock().unwrap();
        let Some(current_asset) = state.music.get(&id) else {
            return Err(AudioError::InvalidChannel);
        };
        if !Arc::ptr_eq(current_asset, &asset) {
            return Err(AudioError::InvalidChannel);
        }
        state.active_music = Some(MusicPlayback::new(stream, looped));
        Ok(())
    }

    pub fn halt_music(&self) {
        let mut state = self.state.lock().unwrap();
        state.active_music = None;
    }

    pub fn music_is_playing(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.active_music.is_some()
    }

    #[cfg(test)]
    fn music_buffered_frame_capacity(&self) -> Option<usize> {
        let state = self.state.lock().unwrap();
        state
            .active_music
            .as_ref()
            .map(|music| music.decode_buffer.len() + music.stream.buffered_frames())
    }

    #[cfg(test)]
    fn music_peak_buffered_frame_capacity(&self) -> Option<usize> {
        let state = self.state.lock().unwrap();
        state
            .active_music
            .as_ref()
            .map(|music| music.decode_buffer.len() + music.stream.peak_buffered_frames())
    }

    pub fn music_set_volume(&self, volume: f32) {
        let mut state = self.state.lock().unwrap();
        if let Some(music) = state.active_music.as_mut() {
            music.volume = volume.clamp(0.0, 1.0);
        }
    }

    pub fn music_fade_out(&self, duration_ms: u32) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(music) = state.active_music.as_mut() else {
            return false;
        };
        if duration_ms == 0 || self.sample_rate == 0 {
            state.active_music = None;
            return true;
        }
        let total_samples = ((duration_ms as u64 * self.sample_rate as u64) / 1000) as usize;
        if total_samples == 0 {
            state.active_music = None;
            return true;
        }
        music.fade_out = Some(FadeState {
            remaining_samples: total_samples,
            total_samples,
        });
        true
    }

    pub fn mix_i16(&self, output: &mut [i16]) {
        self.mix_into(output);
    }

    pub fn mix_f32(&self, output: &mut [f32]) {
        self.mix_into(output);
    }

    pub fn mix_u16(&self, output: &mut [u16]) {
        self.mix_into(output);
    }

    pub fn set_channel_finished_callback_ffi(
        &self,
        callback: Option<extern "C" fn(i32, *mut std::ffi::c_void)>,
        user_data: *mut std::ffi::c_void,
    ) {
        let mut guard = self.channel_finished.write().unwrap();
        if let Some(callback) = callback {
            *guard = Some(ChannelFinished::Ffi {
                callback,
                user_data,
            });
        } else {
            *guard = None;
        }
    }

    #[cfg(test)]
    pub fn set_channel_finished_callback_rust(
        &self,
        callback: Option<Arc<dyn Fn(usize) + Send + Sync>>,
    ) {
        let mut guard = self.channel_finished.write().unwrap();
        if let Some(cb) = callback {
            *guard = Some(ChannelFinished::Rust(cb));
        } else {
            *guard = None;
        }
    }

    fn prepare_clip(&self, decoded: DecodedAudio) -> Arc<AudioClip> {
        let frames = if decoded.sample_rate == self.sample_rate || decoded.sample_rate == 0 {
            decoded.frames
        } else {
            resample_frames(
                &decoded.frames,
                decoded.sample_rate,
                self.sample_rate,
                self.resampling_mode,
            )
        };
        Arc::new(AudioClip {
            frames: Arc::new(frames),
        })
    }

    fn mix_into<T>(&self, output: &mut [T])
    where
        T: SampleWrite,
    {
        let frames = output.len() / 2;
        if frames == 0 {
            return;
        }

        let mut finished_channels: Vec<usize> = Vec::new();
        let mut finished_music = false;

        for sample in output.iter_mut() {
            sample.write_zero();
        }

        let (callback, finished_list) = {
            let mut state = self.state.lock().unwrap();
            for frame_index in 0..frames {
                let mut left = 0.0f32;
                let mut right = 0.0f32;

                for (index, slot) in state.channels.iter_mut().enumerate() {
                    let Some(channel) = slot.as_mut() else {
                        continue;
                    };
                    let frames_len = channel.clip.frames.len();
                    if frames_len == 0 {
                        if !finished_channels.contains(&index) {
                            finished_channels.push(index);
                        }
                        continue;
                    }
                    if channel.position >= frames_len {
                        if channel.looping {
                            channel.position = 0;
                        } else {
                            if !finished_channels.contains(&index) {
                                finished_channels.push(index);
                            }
                            continue;
                        }
                    }
                    if channel.position >= frames_len {
                        continue;
                    }
                    let frame = channel.clip.frames[channel.position];
                    channel.position += 1;
                    left += frame[0] * channel.left_gain;
                    right += frame[1] * channel.right_gain;
                    if !channel.looping
                        && channel.position >= frames_len
                        && !finished_channels.contains(&index)
                    {
                        finished_channels.push(index);
                    }
                }

                if !finished_music {
                    if let Some(music) = state.active_music.as_mut() {
                        if let Some(frame) = music.next_frame() {
                            let mut volume =
                                music.volume * (MAXIMUM_MUSIC_VOLUME / SDL_MIXER_MAX_VOLUME);
                            if let Some(fade) = music.fade_out.as_mut() {
                                if fade.remaining_samples > 0 {
                                    let ratio =
                                        fade.remaining_samples as f32 / fade.total_samples as f32;
                                    volume *= ratio.clamp(0.0, 1.0);
                                    fade.remaining_samples -= 1;
                                    if fade.remaining_samples == 0 {
                                        finished_music = true;
                                    }
                                } else {
                                    finished_music = true;
                                }
                            }
                            left += frame[0] * volume;
                            right += frame[1] * volume;
                        } else {
                            finished_music = true;
                        }
                    }
                }

                let offset = frame_index * 2;
                output[offset].write_sample(left);
                output[offset + 1].write_sample(right);
            }

            finished_channels.sort_unstable();
            finished_channels.dedup();

            for index in &finished_channels {
                if let Some(slot) = state.channels.get_mut(*index) {
                    if slot.is_some() {
                        slot.take();
                    }
                }
            }

            if finished_music {
                state.active_music = None;
            }

            (
                self.channel_finished.read().unwrap().clone(),
                finished_channels.clone(),
            )
        };

        if let Some(callback) = callback {
            if !finished_list.is_empty() {
                match callback {
                    ChannelFinished::Ffi {
                        callback,
                        user_data,
                    } => {
                        for index in finished_list {
                            callback(index as i32, user_data);
                        }
                    }
                    ChannelFinished::Rust(handler) => {
                        for index in finished_list {
                            handler(index);
                        }
                    }
                }
            }
        }
    }
}

impl MusicPlayback {
    fn new(stream: MusicStream, looping: bool) -> Self {
        Self {
            stream,
            decode_buffer: vec![[0.0, 0.0]; MUSIC_DECODE_BUFFER_FRAMES].into_boxed_slice(),
            buffer_position: 0,
            buffer_length: 0,
            looping,
            volume: 1.0,
            fade_out: None,
        }
    }

    fn next_frame(&mut self) -> Option<[f32; 2]> {
        if self.buffer_position >= self.buffer_length && !self.refill() {
            return None;
        }
        let frame = self.decode_buffer[self.buffer_position];
        self.buffer_position += 1;
        Some(frame)
    }

    fn refill(&mut self) -> bool {
        self.buffer_position = 0;
        self.buffer_length = 0;
        let mut restarted = false;
        loop {
            match self.stream.read_frames(&mut self.decode_buffer) {
                Ok(0) if self.looping && !restarted => {
                    if let Err(error) = self.stream.restart() {
                        tracing::warn!(%error, "music stream restart failed");
                        return false;
                    }
                    // An empty looping source must terminate instead of
                    // spinning forever inside an audio callback.
                    restarted = true;
                }
                Ok(0) => return false,
                Ok(read) => {
                    self.buffer_length = read;
                    return true;
                }
                Err(error) => {
                    // SDL_mixer treats a pull-time decoder failure as an ended
                    // music stream. Do the same without poisoning the mixer.
                    tracing::warn!(%error, "music stream decode failed");
                    return false;
                }
            }
        }
    }
}

trait SampleWrite {
    fn write_sample(&mut self, value: f32);
    fn write_zero(&mut self);
}

impl SampleWrite for i16 {
    fn write_sample(&mut self, value: f32) {
        let clamped = value.clamp(-1.0, 1.0);
        *self = (clamped * i16::MAX as f32) as i16;
    }

    fn write_zero(&mut self) {
        *self = 0;
    }
}

impl SampleWrite for u16 {
    fn write_sample(&mut self, value: f32) {
        let clamped = value.clamp(-1.0, 1.0);
        let normalized = (clamped * 0.5 + 0.5) * u16::MAX as f32;
        *self = normalized.clamp(0.0, u16::MAX as f32) as u16;
    }

    fn write_zero(&mut self) {
        *self = u16::MAX / 2;
    }
}

impl SampleWrite for f32 {
    fn write_sample(&mut self, value: f32) {
        *self = value.clamp(-1.0, 1.0);
    }

    fn write_zero(&mut self) {
        *self = 0.0;
    }
}

impl ChannelPlayback {
    fn recalculate_gains(&mut self) {
        let pan = self.pan.clamp(-1.0, 1.0);
        // C4's instance volume is intentionally not a percentage cap:
        // SoundLevel may exceed 100. SDL_mixer clamps the resulting
        // Mix_Volume argument at 128, so normalized Rust input saturates at
        // 128 / MaximumSoundVolume while level 100 retains its headroom.
        let volume = self.volume.clamp(0.0, MAXIMUM_SOUND_INPUT);
        let volume_gain = volume * (MAXIMUM_SOUND_VOLUME / SDL_MIXER_MAX_VOLUME);
        let left = ((1.0 - pan) * MAXIMUM_PANNING_VOLUME).clamp(0.0, MAXIMUM_PANNING_VOLUME)
            / SDL_MIXER_MAX_PANNING;
        let right = ((1.0 + pan) * MAXIMUM_PANNING_VOLUME).clamp(0.0, MAXIMUM_PANNING_VOLUME)
            / SDL_MIXER_MAX_PANNING;
        self.left_gain = left * volume_gain;
        self.right_gain = right * volume_gain;
    }
}

fn resample_frames(
    frames: &[[f32; 2]],
    source_rate: u32,
    target_rate: u32,
    mode: ResamplingMode,
) -> Vec<[f32; 2]> {
    match mode {
        // The Rust backend's established converter is currently linear. Keep
        // that backend default separate from the explicit selection so its
        // quality can change without changing PreferLinearResampling=true.
        ResamplingMode::Default => resample_frames_linear(frames, source_rate, target_rate),
        ResamplingMode::Linear => resample_frames_linear(frames, source_rate, target_rate),
    }
}

fn resample_frames_linear(
    frames: &[[f32; 2]],
    source_rate: u32,
    target_rate: u32,
) -> Vec<[f32; 2]> {
    if target_rate == 0 || source_rate == 0 || source_rate == target_rate {
        return frames.to_vec();
    }
    let ratio = target_rate as f64 / source_rate as f64;
    let output_len = (frames.len() as f64 * ratio).ceil() as usize;
    let mut result = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let src_position = i as f64 / ratio;
        let base_index = src_position.floor() as usize;
        let frac = src_position - base_index as f64;
        let current = frames.get(base_index).copied().unwrap_or([0.0, 0.0]);
        let next = frames.get(base_index + 1).copied().unwrap_or(current);
        let interpolated = [
            current[0] + (next[0] - current[0]) * frac as f32,
            current[1] + (next[1] - current[1]) * frac as f32,
        ];
        result.push(interpolated);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn generate_sine_wave(duration_ms: u32, frequency_hz: f32, sample_rate: u32) -> Vec<u8> {
        let total_samples = (duration_ms as u64 * sample_rate as u64 / 1000) as usize;
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut cursor,
                hound::WavSpec {
                    channels: 2,
                    sample_rate,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for n in 0..total_samples {
                let t = n as f32 / sample_rate as f32;
                let sample = (frequency_hz * t * std::f32::consts::TAU).sin();
                let value = (sample * i16::MAX as f32 * 0.25) as i16;
                writer.write_sample(value).unwrap();
                writer.write_sample(value).unwrap();
            }
        }
        cursor.into_inner()
    }

    fn write_bytes(target: &mut [u8], offset: usize, value: &[u8]) {
        target[offset..offset + value.len()].copy_from_slice(value);
    }

    fn write_le_u16(target: &mut [u8], offset: usize, value: u16) {
        write_bytes(target, offset, &value.to_le_bytes());
    }

    fn write_le_u32(target: &mut [u8], offset: usize, value: u32) {
        write_bytes(target, offset, &value.to_le_bytes());
    }

    fn write_be_u16(target: &mut [u8], offset: usize, value: u16) {
        write_bytes(target, offset, &value.to_be_bytes());
    }

    fn minimal_mod_music() -> Vec<u8> {
        // The original 15-sample Soundtracker layout has no magic signature,
        // so this fixture also proves that format selection probes the bytes
        // with the decoder instead of relying on a filename or fixed tag.
        const PATTERN_OFFSET: usize = 600;
        const SAMPLE_OFFSET: usize = PATTERN_OFFSET + 1_024;
        let mut module = vec![0; SAMPLE_OFFSET + 32];

        write_bytes(&mut module, 0, b"Synthetic MOD");
        let sample = 20;
        write_bytes(&mut module, sample, b"square");
        write_be_u16(&mut module, sample + 22, 16); // 32 bytes, in 16-bit words
        module[sample + 25] = 64;
        write_be_u16(&mut module, sample + 28, 1);
        module[470] = 1; // one order
        module[472] = 0; // order zero uses pattern zero

        // Sample 1, C-3 (period 428), no effect, followed by empty rows.
        write_bytes(&mut module, PATTERN_OFFSET, &[0x01, 0xac, 0x10, 0]);
        for (index, value) in module[SAMPLE_OFFSET..].iter_mut().enumerate() {
            *value = if index % 2 == 0 { 127 } else { 128 };
        }
        module
    }

    fn minimal_xm_music() -> Vec<u8> {
        const PATTERN_OFFSET: usize = 336;
        const PATTERN_BYTES: usize = 64 * 4 * 5;
        const INSTRUMENT_OFFSET: usize = PATTERN_OFFSET + 9 + PATTERN_BYTES;
        const SAMPLE_HEADER_OFFSET: usize = INSTRUMENT_OFFSET + 263;
        const SAMPLE_OFFSET: usize = SAMPLE_HEADER_OFFSET + 40;
        let mut module = vec![0; SAMPLE_OFFSET + 32];

        write_bytes(&mut module, 0, b"Extended Module: ");
        write_bytes(&mut module, 17, b"Synthetic XM");
        module[37] = 0x1a;
        write_bytes(&mut module, 38, b"lc-audio tests");
        write_le_u16(&mut module, 58, 0x0104);
        write_le_u32(&mut module, 60, 276);
        write_le_u16(&mut module, 64, 1); // song length
        write_le_u16(&mut module, 68, 4); // channels
        write_le_u16(&mut module, 70, 1); // patterns
        write_le_u16(&mut module, 72, 1); // instruments
        write_le_u16(&mut module, 76, 6); // initial speed
        write_le_u16(&mut module, 78, 125); // initial tempo

        write_le_u32(&mut module, PATTERN_OFFSET, 9);
        write_le_u16(&mut module, PATTERN_OFFSET + 5, 64);
        write_le_u16(&mut module, PATTERN_OFFSET + 7, PATTERN_BYTES as u16);
        // C-4 on instrument 1; the rest of the uncompressed pattern is empty.
        write_bytes(&mut module, PATTERN_OFFSET + 9, &[49, 1, 0, 0, 0]);

        write_le_u32(&mut module, INSTRUMENT_OFFSET, 263);
        write_bytes(&mut module, INSTRUMENT_OFFSET + 4, b"square");
        write_le_u16(&mut module, INSTRUMENT_OFFSET + 27, 1);
        write_le_u32(&mut module, INSTRUMENT_OFFSET + 29, 40);
        write_le_u32(&mut module, SAMPLE_HEADER_OFFSET, 32);
        module[SAMPLE_HEADER_OFFSET + 12] = 64;
        module[SAMPLE_HEADER_OFFSET + 15] = 128;
        write_bytes(&mut module, SAMPLE_HEADER_OFFSET + 18, b"square");
        // XM stores 8-bit samples as deltas: these decode to +64, -64, ...
        for (index, value) in module[SAMPLE_OFFSET..].iter_mut().enumerate() {
            *value = if index % 2 == 0 { 64 } else { 128 };
        }
        module
    }

    fn minimal_s3m_music() -> Vec<u8> {
        const INSTRUMENT_OFFSET: usize = 8 * 16;
        const PATTERN_OFFSET: usize = 13 * 16;
        const SAMPLE_OFFSET: usize = 20 * 16;
        let mut module = vec![0; SAMPLE_OFFSET + 32];

        write_bytes(&mut module, 0, b"Synthetic S3M");
        module[28] = 0x1a;
        module[29] = 0x10;
        write_le_u16(&mut module, 32, 1); // orders
        write_le_u16(&mut module, 34, 1); // instruments
        write_le_u16(&mut module, 36, 1); // patterns
        write_le_u16(&mut module, 40, 0x1320);
        write_le_u16(&mut module, 42, 1); // signed sample data
        write_bytes(&mut module, 44, b"SCRM");
        module[48] = 64;
        module[49] = 6;
        module[50] = 125;
        module[51] = 0xc0;
        module[64..96].fill(255);
        module[64] = 0;
        module[96] = 0;
        write_le_u16(&mut module, 97, 8);
        write_le_u16(&mut module, 99, 13);

        module[INSTRUMENT_OFFSET] = 1;
        write_bytes(&mut module, INSTRUMENT_OFFSET + 1, b"SQUARE.RAW");
        write_le_u16(&mut module, INSTRUMENT_OFFSET + 14, 20);
        write_le_u32(&mut module, INSTRUMENT_OFFSET + 16, 32);
        module[INSTRUMENT_OFFSET + 28] = 64;
        write_le_u32(&mut module, INSTRUMENT_OFFSET + 32, 8_363);
        write_bytes(&mut module, INSTRUMENT_OFFSET + 48, b"square");
        write_bytes(&mut module, INSTRUMENT_OFFSET + 76, b"SCRS");

        // Channel 0, C-4, instrument 1, then this and 63 more row terminators.
        let packed_pattern_length = 4 + 63;
        write_le_u16(&mut module, PATTERN_OFFSET, packed_pattern_length as u16);
        write_bytes(&mut module, PATTERN_OFFSET + 2, &[0x20, 0x40, 1, 0]);
        for (index, value) in module[SAMPLE_OFFSET..].iter_mut().enumerate() {
            *value = if index % 2 == 0 { 127 } else { 128 };
        }
        module
    }

    fn minimal_it_music() -> Vec<u8> {
        const SAMPLE_HEADER_OFFSET: usize = 208;
        const PATTERN_OFFSET: usize = 288;
        const SAMPLE_OFFSET: usize = 400;
        let mut module = vec![0; SAMPLE_OFFSET + 32];

        write_bytes(&mut module, 0, b"IMPM");
        write_bytes(&mut module, 4, b"Synthetic IT");
        write_le_u16(&mut module, 32, 1); // orders
        write_le_u16(&mut module, 36, 1); // samples
        write_le_u16(&mut module, 38, 1); // patterns
        write_le_u16(&mut module, 40, 0x0214);
        write_le_u16(&mut module, 42, 0x0200);
        module[48] = 128;
        module[49] = 48;
        module[50] = 6;
        module[51] = 125;
        module[52] = 128;
        module[64..128].fill(32);
        module[128..192].fill(64);
        module[192] = 0;
        write_le_u32(&mut module, 193, SAMPLE_HEADER_OFFSET as u32);
        write_le_u32(&mut module, 197, PATTERN_OFFSET as u32);

        write_bytes(&mut module, SAMPLE_HEADER_OFFSET, b"IMPS");
        write_bytes(&mut module, SAMPLE_HEADER_OFFSET + 4, b"SQUARE.RAW");
        module[SAMPLE_HEADER_OFFSET + 17] = 64;
        module[SAMPLE_HEADER_OFFSET + 18] = 1;
        module[SAMPLE_HEADER_OFFSET + 19] = 64;
        write_bytes(&mut module, SAMPLE_HEADER_OFFSET + 20, b"square");
        module[SAMPLE_HEADER_OFFSET + 46] = 1; // signed sample conversion
        module[SAMPLE_HEADER_OFFSET + 47] = 128;
        write_le_u32(&mut module, SAMPLE_HEADER_OFFSET + 48, 32);
        write_le_u32(&mut module, SAMPLE_HEADER_OFFSET + 60, 8_363);
        write_le_u32(&mut module, SAMPLE_HEADER_OFFSET + 72, SAMPLE_OFFSET as u32);

        // New mask for channel 1: note and sample, C-5, sample 1, row end.
        let packed_pattern_length = 5 + 63;
        write_le_u16(&mut module, PATTERN_OFFSET, packed_pattern_length as u16);
        write_le_u16(&mut module, PATTERN_OFFSET + 2, 64);
        write_bytes(&mut module, PATTERN_OFFSET + 8, &[0x81, 0x03, 60, 1, 0]);
        for (index, value) in module[SAMPLE_OFFSET..].iter_mut().enumerate() {
            *value = if index % 2 == 0 { 127 } else { 128 };
        }
        module
    }

    fn i16_energy(samples: &[i16]) -> u64 {
        samples
            .iter()
            .map(|sample| i64::from(*sample).unsigned_abs())
            .sum()
    }

    fn long_silent_mp3() -> Vec<u8> {
        // One independently decodable MPEG-2.5 Layer III mono frame: 576
        // samples at 8 kHz in 72 compressed bytes. Repetition creates a valid
        // track just beyond 15 minutes while keeping the fixture below 1 MiB.
        let mut frame = [0x55_u8; 72];
        frame[..13].copy_from_slice(&[
            0xff, 0xe3, 0x18, 0xc4, 0x00, 0x00, 0x00, 0x03, 0x48, 0x00, 0x00, 0x00, 0x00,
        ]);
        frame[13..22].copy_from_slice(b"LAME3.100");
        frame[53..62].copy_from_slice(b"LAME3.100");
        let frames_for_more_than_fifteen_minutes = (15 * 60 * 8_000 / 576) + 1;
        frame.repeat(frames_for_more_than_fifteen_minutes)
    }

    #[test]
    fn l040_explicit_linear_resampling_mode_uses_linear_interpolation() {
        let frames = [[0.0, 0.0], [1.0, -1.0]];

        assert_eq!(
            resample_frames(&frames, 2, 4, ResamplingMode::Linear),
            vec![[0.0, 0.0], [0.5, -0.5], [1.0, -1.0], [1.0, -1.0]]
        );
        assert_eq!(
            AudioSystem::new_null_with_resampling(1, ResamplingMode::Linear).resampling_mode(),
            ResamplingMode::Linear
        );
        assert_eq!(
            AudioSystem::new_null(1).resampling_mode(),
            ResamplingMode::Default
        );
    }

    fn rms(samples: &[f32]) -> f64 {
        let mean_square = samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / samples.len() as f64;
        mean_square.sqrt()
    }

    #[cfg(feature = "cpal")]
    fn cpal_config_range(
        channels: u16,
        sample_format: cpal::SampleFormat,
        min_sample_rate: u32,
        max_sample_rate: u32,
    ) -> cpal::SupportedStreamConfigRange {
        cpal::SupportedStreamConfigRange::new(
            channels,
            min_sample_rate,
            max_sample_rate,
            cpal::SupportedBufferSize::Unknown,
            sample_format,
        )
    }

    #[cfg(feature = "cpal")]
    #[test]
    fn cpal_output_config_prefers_supported_mixer_formats_in_order() {
        let selected = select_cpal_stereo_output_config([
            cpal_config_range(2, cpal::SampleFormat::I32, 48_000, 48_000),
            cpal_config_range(2, cpal::SampleFormat::U16, 48_000, 48_000),
            cpal_config_range(2, cpal::SampleFormat::I16, 48_000, 48_000),
            cpal_config_range(2, cpal::SampleFormat::F32, 22_050, 32_000),
        ])
        .expect("a supported stereo mixer format should be selected");

        assert_eq!(selected.channels(), 2);
        assert_eq!(selected.sample_format(), cpal::SampleFormat::F32);
        assert_eq!(selected.sample_rate(), 32_000);
    }

    #[cfg(feature = "cpal")]
    #[test]
    fn cpal_output_config_prefers_48khz_then_44_1khz() {
        let selected = select_cpal_stereo_output_config([
            cpal_config_range(2, cpal::SampleFormat::F32, 44_100, 44_100),
            cpal_config_range(2, cpal::SampleFormat::F32, 48_000, 96_000),
        ])
        .expect("a supported stereo mixer format should be selected");

        assert_eq!(selected.sample_format(), cpal::SampleFormat::F32);
        assert_eq!(selected.sample_rate(), 48_000);
    }

    #[cfg(feature = "cpal")]
    #[test]
    fn cpal_output_config_falls_back_to_44_1khz_then_range_maximum() {
        let cd_rate = select_cpal_stereo_output_config([cpal_config_range(
            2,
            cpal::SampleFormat::I16,
            32_000,
            44_100,
        )])
        .expect("44.1 kHz should be selected when 48 kHz is unavailable");
        assert_eq!(cd_rate.sample_rate(), 44_100);

        let range_max = select_cpal_stereo_output_config([cpal_config_range(
            2,
            cpal::SampleFormat::I16,
            22_050,
            32_000,
        )])
        .expect("the range maximum should be selected without a standard rate");
        assert_eq!(range_max.sample_rate(), 32_000);
    }

    #[cfg(feature = "cpal")]
    #[test]
    fn cpal_output_config_rejects_mono_and_unsupported_stereo_formats() {
        assert!(select_cpal_stereo_output_config([
            cpal_config_range(1, cpal::SampleFormat::F32, 48_000, 48_000),
            cpal_config_range(2, cpal::SampleFormat::I32, 48_000, 48_000),
        ])
        .is_none());
    }

    #[test]
    fn device_failure_uses_logged_inert_none_backend() {
        let mut logged = Vec::new();
        let system = AudioSystem::open_output_or_inert(
            2,
            ResamplingMode::Linear,
            || {
                Err(AudioError::Stream(
                    "injected output-open failure".to_string(),
                ))
            },
            |error| logged.push(error.to_string()),
        );

        assert_eq!(
            logged,
            ["failed to create audio stream: injected output-open failure"]
        );
        assert!(matches!(&system._backend, Backend::Inert));
        assert!(system.mixer.inert);
        assert_eq!(system.resampling_mode(), ResamplingMode::Linear);

        let sound = system
            .load_sound(b"not decodable audio")
            .expect("inert backend creates a placeholder sound");
        assert_eq!(sound.duration_ms(), Some(0));
        assert_eq!(system.sound_duration_ms(&sound), Some(0));
        let channel = system
            .play_sound(&sound, true)
            .expect("inert backend creates a placeholder channel");
        let second_channel = system
            .play_sound(&sound, false)
            .expect("each inert play creates another placeholder channel");
        assert_eq!(channel.0, INERT_CHANNEL_INDEX);
        assert_eq!(second_channel.0, INERT_CHANNEL_INDEX);
        assert_ne!(channel, second_channel);
        assert!(!system.channel_is_playing(channel));
        assert!(!system.channel_is_playing(second_channel));
        system.channel_set_volume_and_pan(channel, 0.75, -0.5);
        system.halt_channel(channel);
        assert!(!system.channel_is_playing(channel));

        let music = system
            .load_music(b"not decodable music")
            .expect("inert backend creates a placeholder music file");
        let owned_music = system
            .load_music_owned(b"also not decodable music".to_vec())
            .expect("inert backend accepts owned placeholder music");
        for handle in [&music, &owned_music] {
            system
                .play_music(handle, true)
                .expect("inert music play is a no-op success");
            assert!(!system.music_is_playing());
        }
        system.music_set_volume(0.25);
        assert!(!system.music_fade_out(250));
        system.halt_music();

        let worker = system.worker_handle();
        let worker_music = worker
            .load_music_owned(b"worker does not decode this".to_vec())
            .expect("inert worker creates a placeholder music file");
        worker
            .play_music(&worker_music, false)
            .expect("inert worker music play is a no-op success");
        worker.music_set_volume(0.5);
        worker.halt_music();
        assert!(!system.music_is_playing());

        let active_null = AudioSystem::new_null(1);
        assert!(matches!(&active_null._backend, Backend::Null(_)));
        assert!(matches!(
            active_null.load_sound(b"not decodable audio"),
            Err(AudioError::Decode(_))
        ));
    }

    #[test]
    fn deferred_null_backend_starts_on_first_sound_playback() {
        let system = AudioSystem::new_deferred_null(2);
        let Backend::DeferredNull(backend) = &system._backend else {
            panic!("deferred constructor must retain a deferred backend");
        };
        assert!(backend.backend.get().is_none());

        let data = generate_sine_wave(50, 440.0, 44_100);
        let sound = system.load_sound(&data).unwrap();
        assert!(backend.backend.get().is_none(), "loading stays dormant");

        system.play_sound(&sound, false).unwrap();
        assert!(backend.backend.get().is_some(), "playback starts backend");
    }

    #[test]
    fn deferred_null_backend_starts_through_music_worker_handle() {
        let system = AudioSystem::new_deferred_null(2);
        let Backend::DeferredNull(backend) = &system._backend else {
            panic!("deferred constructor must retain a deferred backend");
        };
        let worker = system.worker_handle();
        let data = generate_sine_wave(50, 440.0, 44_100);
        let music = worker.load_music(&data).unwrap();
        assert!(backend.backend.get().is_none(), "loading stays dormant");

        worker.play_music(&music, false).unwrap();
        assert!(backend.backend.get().is_some(), "playback starts backend");
    }

    #[test]
    fn plays_sound_to_completion() {
        let data = generate_sine_wave(200, 440.0, 44_100);
        let mixer = AudioMixer::new(44_100, 4);
        let sound_id = mixer.load_sound(&data).unwrap();
        let channel = mixer.play_sound(sound_id, false).unwrap();
        let mut buffer = vec![0i16; 512 * 2];
        for _ in 0..200 {
            mixer.mix_i16(&mut buffer);
        }
        assert!(!mixer.channel_is_playing(channel));
    }

    #[test]
    fn looping_sound_remains_active() {
        let data = generate_sine_wave(50, 440.0, 44_100);
        let mixer = AudioMixer::new(44_100, 2);
        let sound_id = mixer.load_sound(&data).unwrap();
        let channel = mixer.play_sound(sound_id, true).unwrap();
        let mut buffer = vec![0i16; 256 * 2];
        for _ in 0..200 {
            mixer.mix_i16(&mut buffer);
        }
        assert!(mixer.channel_is_playing(channel));
        mixer.halt_channel(channel);
        assert!(!mixer.channel_is_playing(channel));
    }

    #[test]
    fn non_looping_music_stops_after_one_pass() {
        // C4AudioSystemSdl.cpp:212-214 passes 1 to Mix_PlayMusic when loop is false;
        // SDL_mixer 2.8 treats that as one total play for its music backends.
        let data = generate_sine_wave(10, 440.0, 1_000);
        let mixer = AudioMixer::new(1_000, 2);
        let music_id = mixer.load_music(&data).unwrap();
        mixer.play_music(music_id, false).unwrap();

        let mut first_pass_and_one_frame = vec![0i16; 11 * 2];
        mixer.mix_i16(&mut first_pass_and_one_frame);
        assert!(!mixer.music_is_playing());
    }

    #[test]
    fn decodes_and_plays_cpp_tracker_music_formats() {
        let formats = [
            ("IT", minimal_it_music()),
            ("MOD", minimal_mod_music()),
            ("S3M", minimal_s3m_music()),
            ("XM", minimal_xm_music()),
        ];

        for (format, data) in formats {
            let mixer = AudioMixer::new(8_000, 1);
            let music_id = mixer
                .load_music(&data)
                .unwrap_or_else(|error| panic!("failed to decode synthetic {format}: {error}"));

            mixer.play_music(music_id, true).unwrap();
            let mut loop_energy = 0;
            let mut output = vec![0i16; 1_024 * 2];
            // These fixtures contain one 64-row pattern (about 7.7 seconds
            // at 8 kHz); eighty native-sized pulls cross the first EOF and
            // exercise libxmp's in-place restart.
            for _ in 0..80 {
                mixer.mix_i16(&mut output);
                loop_energy += i16_energy(&output);
            }
            assert!(loop_energy > 0, "{format} playback produced only silence");
            assert!(
                mixer.music_is_playing(),
                "{format} did not loop after its streamed PCM was exhausted"
            );
            assert!(
                mixer.music_peak_buffered_frame_capacity().unwrap()
                    <= MUSIC_DECODE_BUFFER_FRAMES + 1_024 + 2,
                "{format} retained an unbounded PCM working set"
            );

            mixer.play_music(music_id, true).unwrap();
            let mut full_volume = vec![0i16; 2_048 * 2];
            mixer.mix_i16(&mut full_volume);
            let full_energy = i16_energy(&full_volume);
            mixer.play_music(music_id, true).unwrap();
            mixer.music_set_volume(0.25);
            let mut quarter_volume = vec![0i16; 2_048 * 2];
            mixer.mix_i16(&mut quarter_volume);
            let quarter_energy = i16_energy(&quarter_volume);
            assert!(full_energy > 0, "{format} full-volume output was silent");
            assert!(
                quarter_energy > 0 && quarter_energy < full_energy / 2,
                "{format} music volume did not attenuate output: full={full_energy}, quarter={quarter_energy}"
            );

            mixer.play_music(music_id, true).unwrap();
            assert!(mixer.music_fade_out(1));
            let mut fade = [0i16; 8 * 2];
            mixer.mix_i16(&mut fade);
            assert!(
                !mixer.music_is_playing(),
                "{format} fade did not stop playback"
            );

            mixer.play_music(music_id, true).unwrap();
            mixer.halt_music();
            assert!(
                !mixer.music_is_playing(),
                "{format} explicit halt did not stop playback"
            );
        }
    }

    #[test]
    fn long_music_streams_with_bounded_memory_and_no_prerender_limit() {
        let mixer = AudioMixer::new(1_000, 1);
        let music_id = mixer
            .load_music_owned(long_silent_mp3())
            .expect("long compressed music opens without a full decode");
        mixer.play_music(music_id, false).unwrap();

        let initial_capacity = mixer.music_buffered_frame_capacity().unwrap();
        assert!(
            initial_capacity <= MUSIC_DECODE_BUFFER_FRAMES + minimp3::MAX_SAMPLES_PER_FRAME + 2,
            "stream retains {initial_capacity} decoded frames"
        );

        let mut output = vec![0.0_f32; MUSIC_DECODE_BUFFER_FRAMES * 2];
        let mut frames_remaining = 15 * 60 * 1_000 + 1;
        while frames_remaining != 0 {
            let frames = frames_remaining.min(MUSIC_DECODE_BUFFER_FRAMES);
            mixer.mix_f32(&mut output[..frames * 2]);
            frames_remaining -= frames;
        }
        assert!(
            mixer.music_is_playing(),
            "compressed stream ended at the former 15-minute ceiling"
        );
        assert!(
            mixer.music_peak_buffered_frame_capacity().unwrap()
                <= MUSIC_DECODE_BUFFER_FRAMES + minimp3::MAX_SAMPLES_PER_FRAME + 2
        );
    }

    #[test]
    fn music_fade_out_finishes_playback() {
        let data = generate_sine_wave(400, 440.0, 44_100);
        let mixer = AudioMixer::new(44_100, 2);
        let music_id = mixer.load_music(&data).unwrap();
        mixer.play_music(music_id, true).unwrap();
        assert!(mixer.music_is_playing());
        assert!(mixer.music_fade_out(100));
        let mut buffer = vec![0i16; 256 * 2];
        for _ in 0..200 {
            mixer.mix_i16(&mut buffer);
        }
        assert!(!mixer.music_is_playing());
    }

    #[test]
    fn streaming_music_loops_replays_and_outlives_its_loaded_handle() {
        let data = generate_sine_wave(10, 440.0, 1_000);
        let mixer = Arc::new(AudioMixer::new(1_000, 1));
        let music_id = mixer.load_music(&data).unwrap();
        let handle = MusicHandle::new(mixer.clone(), music_id);

        mixer.play_music(music_id, true).unwrap();
        let mut beyond_one_pass = vec![0.0_f32; 25 * 2];
        mixer.mix_f32(&mut beyond_one_pass);
        assert!(mixer.music_is_playing(), "loop rewinds the pull decoder");

        mixer.play_music(music_id, false).unwrap();
        drop(handle);
        assert!(
            mixer.music_is_playing(),
            "active stream owns its source bytes"
        );
        assert!(matches!(
            mixer.play_music(music_id, false),
            Err(AudioError::InvalidChannel)
        ));

        let mut replay_and_one_frame = vec![0.0_f32; 11 * 2];
        mixer.mix_f32(&mut replay_and_one_frame);
        assert!(!mixer.music_is_playing());
    }

    #[test]
    fn volume_and_pan_affect_output() {
        let data = generate_sine_wave(200, 220.0, 44_100);
        let mixer = AudioMixer::new(44_100, 2);
        let sound_id = mixer.load_sound(&data).unwrap();
        let channel = mixer.play_sound(sound_id, true).unwrap();
        assert!(mixer.channel_set_volume_and_pan(channel, 0.5, 1.0));
        let mut buffer = vec![0i16; 128 * 2];
        mixer.mix_i16(&mut buffer);
        let left_energy: i64 = buffer
            .iter()
            .step_by(2)
            .map(|sample| (*sample as i32).abs() as i64)
            .sum();
        let right_energy: i64 = buffer
            .iter()
            .skip(1)
            .step_by(2)
            .map(|sample| (*sample as i32).abs() as i64)
            .sum();
        assert!(right_energy > left_energy);
    }

    #[test]
    fn equal_script_volumes_match_sdl_music_to_centered_sound_balance() {
        let data = generate_sine_wave(200, 440.0, 44_100);
        let mixer = AudioMixer::new(44_100, 2);

        let sound_id = mixer.load_sound(&data).unwrap();
        let channel = mixer.play_sound(sound_id, true).unwrap();
        mixer.channel_set_volume_and_pan(channel, 1.0, 0.0);
        let mut sound_output = vec![0.0f32; 1_024 * 2];
        mixer.mix_f32(&mut sound_output);
        mixer.halt_channel(channel);

        let music_id = mixer.load_music(&data).unwrap();
        mixer.play_music(music_id, true).unwrap();
        mixer.music_set_volume(1.0);
        let mut music_output = vec![0.0f32; 1_024 * 2];
        mixer.mix_f32(&mut music_output);

        let ratio = rms(&music_output) / rms(&sound_output);
        let expected = 17.0 / 16.0;
        assert!(
            (ratio - expected).abs() < 0.001,
            "music:sound RMS ratio {ratio} differs from SDL ratio {expected}"
        );
    }

    #[test]
    fn fully_panned_sound_uses_sdl_side_gain_cap() {
        let data = generate_sine_wave(20, 440.0, 44_100);
        let mixer = AudioMixer::new(44_100, 1);
        let sound_id = mixer.load_sound(&data).unwrap();
        let channel = mixer.play_sound(sound_id, true).unwrap();
        mixer.channel_set_volume_and_pan(channel, 1.0, 1.0);

        let state = mixer.state.lock().unwrap();
        let playback = state.channels[channel.0].as_ref().unwrap();
        let expected_loud_side = 10.0f32 / 17.0;
        assert_eq!(playback.left_gain, 0.0);
        assert!(
            (playback.right_gain - expected_loud_side).abs() < 1.0e-6,
            "hard-pan gain {} exceeds SDL cap {expected_loud_side}",
            playback.right_gain
        );
    }

    #[test]
    fn sound_level_above_100_reaches_sdl_max_gain() {
        let data = generate_sine_wave(20, 440.0, 44_100);
        let mixer = AudioMixer::new(44_100, 1);
        let sound_id = mixer.load_sound(&data).unwrap();
        let channel = mixer.play_sound(sound_id, true).unwrap();
        let centered_pan_gain = MAXIMUM_PANNING_VOLUME / SDL_MIXER_MAX_PANNING;

        assert!(mixer.channel_set_volume_and_pan(channel, 1.0, 0.0));
        {
            let state = mixer.state.lock().unwrap();
            let playback = state.channels[channel.0].as_ref().unwrap();
            let expected = centered_pan_gain * MAXIMUM_SOUND_VOLUME / SDL_MIXER_MAX_VOLUME;
            assert_eq!(playback.volume, 1.0);
            assert!((playback.left_gain - expected).abs() < 1.0e-6);
            assert!((playback.right_gain - expected).abs() < 1.0e-6);
        }

        assert!(mixer.channel_set_volume_and_pan(channel, 1.4, 0.0));
        let state = mixer.state.lock().unwrap();
        let playback = state.channels[channel.0].as_ref().unwrap();
        assert_eq!(playback.volume, MAXIMUM_SOUND_INPUT);
        assert!((playback.left_gain - centered_pan_gain).abs() < 1.0e-6);
        assert!((playback.right_gain - centered_pan_gain).abs() < 1.0e-6);
    }

    #[test]
    fn stale_channel_generation_cannot_control_a_reused_slot() {
        let data = generate_sine_wave(200, 220.0, 44_100);
        let mixer = AudioMixer::new(44_100, 1);
        let sound_id = mixer.load_sound(&data).unwrap();
        let stale = mixer.play_sound(sound_id, false).unwrap();
        mixer.halt_channel(stale);
        let replacement = mixer.play_sound(sound_id, true).unwrap();

        assert_eq!(stale.0, replacement.0, "the sole slot is reused");
        assert_ne!(stale, replacement, "slot reuse advances its generation");
        assert!(!mixer.channel_is_playing(stale));
        assert!(!mixer.channel_set_volume_and_pan(stale, 0.0, 0.0));
        mixer.halt_channel(stale);
        assert!(mixer.channel_is_playing(replacement));
    }

    #[test]
    fn dropping_transient_clone_keeps_sound_loaded() {
        // lc-app caches one SoundHandle per effect and plays transient clones
        // of it; C4SoundSystem keeps samples loaded until the system clears
        // them (C4SoundSystem.cpp GetEffect/Play), so dropping a clone must
        // not unload the shared sample.
        let data = generate_sine_wave(60, 330.0, 44_100);
        let mixer = Arc::new(AudioMixer::new(44_100, 2));
        let id = mixer.load_sound(&data).unwrap();
        let cached = SoundHandle::new(mixer.clone(), id);
        drop(cached.clone());
        assert!(mixer.play_sound(id, false).is_ok());
    }

    #[test]
    fn dropping_last_handle_unloads_sound() {
        let data = generate_sine_wave(60, 330.0, 44_100);
        let mixer = Arc::new(AudioMixer::new(44_100, 2));
        let id = mixer.load_sound(&data).unwrap();
        let handle = SoundHandle::new(mixer.clone(), id);
        let clone = handle.clone();
        drop(handle);
        drop(clone);
        assert!(matches!(
            mixer.play_sound(id, false),
            Err(AudioError::InvalidChannel)
        ));
    }

    #[test]
    fn invokes_channel_finished_callback() {
        let data = generate_sine_wave(60, 330.0, 44_100);
        let mixer = AudioMixer::new(44_100, 1);
        let sound_id = mixer.load_sound(&data).unwrap();
        let channel = mixer.play_sound(sound_id, false).unwrap();
        let notified = Arc::new(AtomicBool::new(false));
        let notify_clone = notified.clone();
        let channel_index = channel.0;
        mixer.set_channel_finished_callback_rust(Some(Arc::new(move |finished| {
            if finished == channel_index {
                notify_clone.store(true, Ordering::Release);
            }
        })));
        let mut buffer = vec![0i16; 256 * 2];
        for _ in 0..200 {
            mixer.mix_i16(&mut buffer);
            if !mixer.channel_is_playing(channel) {
                break;
            }
        }
        assert!(notified.load(Ordering::Acquire));
    }
}
