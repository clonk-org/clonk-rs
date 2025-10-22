use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::decoder::{decode_audio, AudioDecodeError, DecodedAudio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelId(pub usize);

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

#[derive(Clone)]
pub struct SoundHandle {
    mixer: Arc<AudioMixer>,
    id: SoundId,
    released: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct MusicHandle {
    mixer: Arc<AudioMixer>,
    id: MusicId,
    released: Arc<AtomicBool>,
}

impl SoundHandle {
    pub fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.mixer.unload_sound(self.id);
        }
    }

    pub fn duration_ms(&self) -> Option<u32> {
        self.mixer.sound_duration_ms(self.id)
    }

    fn id(&self) -> SoundId {
        self.id
    }
}

impl Drop for SoundHandle {
    fn drop(&mut self) {
        self.release();
    }
}

impl MusicHandle {
    pub fn release(&self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.mixer.unload_music(self.id);
        }
    }

    fn id(&self) -> MusicId {
        self.id
    }
}

impl Drop for MusicHandle {
    fn drop(&mut self) {
        self.release();
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
}

impl AudioSystem {
    pub fn new(max_channels: usize) -> Result<Self, AudioError> {
        #[cfg(feature = "cpal")]
        if let Ok((mixer, backend)) = CpalBackend::try_new(max_channels) {
            return Ok(Self {
                mixer,
                _backend: Backend::Cpal(backend),
            });
        }

        let mixer = Arc::new(AudioMixer::new(44_100, max_channels));
        let backend = NullBackend::new(mixer.clone());
        Ok(Self {
            mixer,
            _backend: Backend::Null(backend),
        })
    }

    pub fn load_sound(&self, data: &[u8]) -> Result<SoundHandle, AudioError> {
        let id = self.mixer.load_sound(data)?;
        Ok(SoundHandle {
            mixer: self.mixer.clone(),
            id,
            released: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn load_music(&self, data: &[u8]) -> Result<MusicHandle, AudioError> {
        let id = self.mixer.load_music(data)?;
        Ok(MusicHandle {
            mixer: self.mixer.clone(),
            id,
            released: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn play_sound(&self, sound: &SoundHandle, looped: bool) -> Result<ChannelId, AudioError> {
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
}

#[cfg(feature = "cpal")]
struct CpalBackend {
    _stream: cpal::Stream,
}

#[cfg(feature = "cpal")]
impl CpalBackend {
    fn try_new(max_channels: usize) -> Result<(Arc<AudioMixer>, Self), AudioError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoAudioDevice)?;
        let config = device
            .default_output_config()
            .map_err(|err| AudioError::Stream(err.to_string()))?;

        let sample_rate = config.sample_rate().0;
        let stream_config: cpal::StreamConfig = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let mixer = Arc::new(AudioMixer::new(sample_rate, max_channels));
        let mix_i16 = mixer.clone();
        let mix_f32 = mixer.clone();
        let mix_u16 = mixer.clone();

        let err_fn = |err| {
            tracing::error!(error = %err, "cpal stream error");
        };

        let stream = match config.sample_format() {
            cpal::SampleFormat::I16 => device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [i16], _| {
                        mix_i16.mix_i16(data);
                    },
                    err_fn,
                    None,
                )
                .map_err(|err| AudioError::Stream(err.to_string()))?,
            cpal::SampleFormat::F32 => device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [f32], _| {
                        mix_f32.mix_f32(data);
                    },
                    err_fn,
                    None,
                )
                .map_err(|err| AudioError::Stream(err.to_string()))?,
            cpal::SampleFormat::U16 => device
                .build_output_stream(
                    &stream_config,
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
}

struct MixerState {
    sounds: HashMap<SoundId, Arc<AudioClip>>,
    music: HashMap<MusicId, Arc<AudioClip>>,
    channels: Vec<Option<ChannelPlayback>>,
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
        let state = MixerState {
            sounds: HashMap::new(),
            music: HashMap::new(),
            channels: (0..max_channels).map(|_| None).collect(),
            active_music: None,
            next_sound_id: 1,
            next_music_id: 1,
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            channel_finished: Arc::new(RwLock::new(None)),
            sample_rate,
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
        let decoded = decode_audio(data)?;
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
        state.channels[channel_index] = Some(playback);
        Ok(ChannelId(channel_index))
    }

    pub fn halt_channel(&self, channel: ChannelId) {
        let mut state = self.state.lock().unwrap();
        if let Some(slot) = state.channels.get_mut(channel.0) {
            *slot = None;
        }
    }

    pub fn channel_is_playing(&self, channel: ChannelId) -> bool {
        let state = self.state.lock().unwrap();
        state
            .channels
            .get(channel.0)
            .and_then(|slot| slot.as_ref())
            .is_some()
    }

    pub fn channel_set_volume_and_pan(&self, channel: ChannelId, volume: f32, pan: f32) {
        let mut state = self.state.lock().unwrap();
        if let Some(Some(playback)) = state.channels.get_mut(channel.0) {
            playback.volume = volume.clamp(0.0, 1.0);
            playback.pan = pan.clamp(-1.0, 1.0);
            playback.recalculate_gains();
        }
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
            resample_frames(&decoded.frames, decoded.sample_rate, self.sample_rate)
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
                    if !channel.looping && channel.position >= frames_len {
                        if !finished_channels.contains(&index) {
                            finished_channels.push(index);
                        }
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
                            let mut volume = music.volume;
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
        let left = (1.0 - pan).clamp(0.0, 2.0) * 0.5;
        let right = (1.0 + pan).clamp(0.0, 2.0) * 0.5;
        self.left_gain = left * volume;
        self.right_gain = right * volume;
    }
}

fn resample_frames(frames: &[[f32; 2]], source_rate: u32, target_rate: u32) -> Vec<[f32; 2]> {
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
        mixer.channel_set_volume_and_pan(channel, 0.5, 1.0);
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
