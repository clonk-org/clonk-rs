use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::decoder::{decode_audio, decode_audio_for_output, AudioDecodeError, DecodedAudio};

const SDL_MIXER_MAX_VOLUME: f32 = 128.0;
const SDL_MIXER_MAX_PANNING: f32 = 255.0;
const MAXIMUM_MUSIC_VOLUME: f32 = 80.0;
const MAXIMUM_SOUND_VOLUME: f32 = 100.0;
const MAXIMUM_PANNING_VOLUME: f32 = 192.0;

/// Mixer slot plus allocation generation; stale handles cannot control a
/// later sound that reuses the same numeric slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelId(pub usize, pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SoundId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MusicId(u32);

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
        if let Ok((mixer, backend)) = CpalBackend::try_new(max_channels, resampling_mode) {
            return Ok(Self {
                mixer,
                _backend: Backend::Cpal(backend),
            });
        }

        Ok(Self::new_null_with_resampling(
            max_channels,
            resampling_mode,
        ))
    }

    /// Construct the same live mixer state without opening a platform audio
    /// device. The null backend advances playback in real time, making it
    /// suitable for deterministic tests and headless embedding.
    pub fn new_null(max_channels: usize) -> Self {
        Self::new_null_with_resampling(max_channels, ResamplingMode::Default)
    }

    pub fn new_null_with_resampling(
        max_channels: usize,
        resampling_mode: ResamplingMode,
    ) -> Self {
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

    /// A cheap `Send + Sync` handle onto the shared mixer so expensive
    /// music decodes (a full MIDI render through FluidSynth) can run off
    /// the caller's thread and start playback when ready.
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
}

struct MixerState {
    sounds: HashMap<SoundId, Arc<AudioClip>>,
    music: HashMap<MusicId, Arc<AudioClip>>,
    channels: Vec<Option<ChannelPlayback>>,
    channel_generations: Vec<u64>,
    active_music: Option<MusicPlayback>,
    next_sound_id: u32,
    next_music_id: u32,
}

#[derive(Debug)]
struct AudioClip {
    frames: Arc<Vec<[f32; 2]>>,
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

#[derive(Debug)]
struct MusicPlayback {
    clip: Arc<AudioClip>,
    position: usize,
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
        let state = MixerState {
            sounds: HashMap::new(),
            music: HashMap::new(),
            channels: (0..max_channels).map(|_| None).collect(),
            channel_generations: vec![0; max_channels],
            active_music: None,
            next_sound_id: 1,
            next_music_id: 1,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            channel_finished: Arc::new(RwLock::new(None)),
            sample_rate,
            resampling_mode,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub(crate) fn load_sound(&self, data: &[u8]) -> Result<SoundId, AudioError> {
        let decoded = decode_audio(data)?;
        let clip = self.prepare_clip(decoded);
        let mut state = self.state.lock().unwrap();
        let id = SoundId(state.next_sound_id);
        state.next_sound_id += 1;
        state.sounds.insert(id, clip);
        Ok(id)
    }

    pub(crate) fn load_music(&self, data: &[u8]) -> Result<MusicId, AudioError> {
        let decoded = decode_audio_for_output(data, self.sample_rate)?;
        let clip = self.prepare_clip(decoded);
        let mut state = self.state.lock().unwrap();
        let id = MusicId(state.next_music_id);
        state.next_music_id += 1;
        state.music.insert(id, clip);
        Ok(id)
    }

    pub(crate) fn unload_sound(&self, id: SoundId) {
        let mut state = self.state.lock().unwrap();
        state.sounds.remove(&id);
    }

    pub(crate) fn unload_music(&self, id: MusicId) {
        let mut state = self.state.lock().unwrap();
        state.music.remove(&id);
    }

    pub(crate) fn sound_duration_ms(&self, id: SoundId) -> Option<u32> {
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

    pub fn channel_set_volume_and_pan(
        &self,
        channel: ChannelId,
        volume: f32,
        pan: f32,
    ) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.channel_generations.get(channel.0) != Some(&channel.1) {
            return false;
        }
        let Some(Some(playback)) = state.channels.get_mut(channel.0) else {
            return false;
        };
        playback.volume = volume.clamp(0.0, 1.0);
        playback.pan = pan.clamp(-1.0, 1.0);
        playback.recalculate_gains();
        true
    }

    pub(crate) fn play_music(&self, id: MusicId, looped: bool) -> Result<(), AudioError> {
        let mut state = self.state.lock().unwrap();
        let clip = state
            .music
            .get(&id)
            .cloned()
            .ok_or(AudioError::InvalidChannel)?;
        state.active_music = Some(MusicPlayback {
            clip,
            position: 0,
            looping: looped,
            volume: 1.0,
            fade_out: None,
        });
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

                if let Some(music) = state.active_music.as_mut() {
                    let frames_len = music.clip.frames.len();
                    if frames_len == 0 {
                        finished_music = true;
                    } else {
                        if music.position >= frames_len {
                            if music.looping {
                                music.position = 0;
                            } else {
                                finished_music = true;
                            }
                        }
                        if !finished_music {
                            let frame = music.clip.frames[music.position];
                            music.position += 1;
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
        let volume = self.volume.clamp(0.0, 1.0);
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
