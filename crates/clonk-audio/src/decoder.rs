use std::io::Cursor;
use std::sync::Arc;

use lewton::inside_ogg::OggStreamReader;
use symphonia_bundle_mp3::{MpaDecoder, MpaReader};
use symphonia_core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia_core::codecs::CodecParameters;
use symphonia_core::errors::Error as SymphoniaError;
use symphonia_core::formats::{FormatOptions, FormatReader};
use symphonia_core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use thiserror::Error;

use crate::wav::WavStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Ogg,
    Mp3,
    Midi,
    Tracker,
}

#[derive(Debug, Error)]
pub enum AudioDecodeError {
    #[error("unsupported audio format")]
    UnsupportedFormat,
    #[error("invalid audio data: {0}")]
    InvalidData(&'static str),
    #[error("decoder error: {0}")]
    DecoderError(&'static str),
    #[error("decoder error: {0}")]
    Mp3DecoderError(&'static str),
    #[error("MIDI decoder error: {0}")]
    MidiDecoderError(String),
    #[error("tracker decoder error: {0}")]
    TrackerDecoderError(String),
    #[error("decoded audio exceeds the retained sound budget")]
    DecodedAudioTooLarge,
    #[error("failed to allocate decoded audio within the retained sound budget")]
    DecodedAudioAllocationFailed,
    #[error("failed to allocate an owned copy of the audio source")]
    AudioSourceAllocationFailed,
    #[error("io error")]
    IoError(#[from] std::io::Error),
}

impl AudioDecodeError {
    /// True when the host is missing an optional runtime (FluidSynth, a
    /// SoundFont, or libxmp). Those failures stay the same for the process
    /// lifetime, so callers should not warn on every catalog retry.
    pub fn is_missing_optional_decoder(&self) -> bool {
        match self {
            Self::MidiDecoderError(message) => {
                message.contains("FluidSynth library not found")
                    || message.contains("no SoundFont found")
            }
            Self::TrackerDecoderError(message) => message.contains("libxmp library not found"),
            _ => false,
        }
    }
}

/// Cheaply cloned ownership of compressed audio bytes. Owned callers retain
/// their `Vec` allocation, while borrowed callers get an explicitly fallible
/// copy instead of an infallible allocation hidden in `Arc::from`.
#[derive(Clone, Debug)]
pub(crate) struct SharedAudioData(Arc<Vec<u8>>);

impl SharedAudioData {
    pub(crate) fn from_owned(data: Vec<u8>) -> Self {
        Self(Arc::new(data))
    }

    pub(crate) fn try_from_slice(data: &[u8]) -> Result<Self, AudioDecodeError> {
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(data.len())
            .map_err(|_| AudioDecodeError::AudioSourceAllocationFailed)?;
        owned.extend_from_slice(data);
        Ok(Self::from_owned(owned))
    }
}

impl AsRef<[u8]> for SharedAudioData {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl std::ops::Deref for SharedAudioData {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub frames: Vec<[f32; 2]>,
    pub sample_rate: u32,
}

pub fn decode_audio(data: &[u8]) -> Result<DecodedAudio, AudioDecodeError> {
    decode_audio_for_output(data, 44_100)
}

pub(crate) fn decode_audio_for_output(
    data: &[u8],
    output_sample_rate: u32,
) -> Result<DecodedAudio, AudioDecodeError> {
    let original_error = match decode_audio_for_output_direct(data, output_sample_rate) {
        Ok(decoded) => return Ok(decoded),
        Err(error) => error,
    };
    retry_mpeg_layer3_candidates(data, original_error, |offset| decode_mp3(&data[offset..]))
}

/// Decode a sound at its mixer rate without ever collecting more than
/// `max_output_frames` of stereo PCM. The extra one-frame read distinguishes
/// an exact-fit stream from one that would exceed the caller's retained pool.
pub(crate) fn decode_audio_bounded_for_output(
    data: SharedAudioData,
    output_sample_rate: u32,
    max_output_frames: usize,
) -> Result<DecodedAudio, AudioDecodeError> {
    const READ_FRAMES: usize = 4_096;

    let mut stream = MusicStream::open_for_sound(data, output_sample_rate)?;
    let mut frames = Vec::new();
    let mut buffer = [[0.0_f32; 2]; READ_FRAMES];
    loop {
        let remaining = max_output_frames.saturating_sub(frames.len());
        let requested = remaining.saturating_add(1).min(READ_FRAMES);
        let count = stream.read_frames(&mut buffer[..requested])?;
        if count == 0 {
            return Ok(DecodedAudio {
                frames,
                sample_rate: output_sample_rate,
            });
        }
        if count > remaining {
            return Err(AudioDecodeError::DecodedAudioTooLarge);
        }

        let required = frames
            .len()
            .checked_add(count)
            .ok_or(AudioDecodeError::DecodedAudioTooLarge)?;
        if required > frames.capacity() {
            let doubled = frames.capacity().saturating_mul(2);
            let target = required.max(doubled).min(max_output_frames);
            frames
                .try_reserve_exact(target.saturating_sub(frames.len()))
                .map_err(|_| AudioDecodeError::DecodedAudioAllocationFailed)?;
            if frames.capacity() > max_output_frames {
                return Err(AudioDecodeError::DecodedAudioTooLarge);
            }
        }
        frames.extend_from_slice(&buffer[..count]);
    }
}

fn decode_audio_for_output_direct(
    data: &[u8],
    output_sample_rate: u32,
) -> Result<DecodedAudio, AudioDecodeError> {
    let format = detect_format(data)?;
    match format {
        AudioFormat::Wav => decode_wav(data),
        AudioFormat::Ogg => decode_ogg(data),
        AudioFormat::Mp3 => decode_mp3(data),
        AudioFormat::Midi => crate::fluidsynth::decode_midi(data, output_sample_rate),
        AudioFormat::Tracker => crate::tracker::decode_tracker(data, output_sample_rate),
    }
}

/// A pull-based music decoder. The compressed source stays owned for the
/// stream's lifetime, while decoded PCM is limited to the codec's current
/// packet plus the two frames needed for linear interpolation.
pub(crate) struct MusicStream {
    data: SharedAudioData,
    output_sample_rate: u32,
    purpose: StreamPurpose,
    decoder: MusicDecoder,
}

impl MusicStream {
    pub(crate) fn open(
        data: SharedAudioData,
        output_sample_rate: u32,
    ) -> Result<Self, AudioDecodeError> {
        if output_sample_rate == 0 {
            return Err(AudioDecodeError::InvalidData(
                "music output sample rate is zero",
            ));
        }
        Self::open_with_purpose(data, output_sample_rate, StreamPurpose::Music)
    }

    fn open_for_sound(
        data: SharedAudioData,
        output_sample_rate: u32,
    ) -> Result<Self, AudioDecodeError> {
        if output_sample_rate == 0 {
            return Err(AudioDecodeError::InvalidData(
                "sound output sample rate is zero",
            ));
        }
        Self::open_with_purpose(data, output_sample_rate, StreamPurpose::Sound)
    }

    fn open_with_purpose(
        data: SharedAudioData,
        output_sample_rate: u32,
        purpose: StreamPurpose,
    ) -> Result<Self, AudioDecodeError> {
        let decoder = MusicDecoder::open(data.clone(), output_sample_rate, purpose)?;
        Ok(Self {
            data,
            output_sample_rate,
            purpose,
            decoder,
        })
    }

    /// Fill `output` with stereo frames, returning zero once the stream ends.
    pub(crate) fn read_frames(
        &mut self,
        output: &mut [[f32; 2]],
    ) -> Result<usize, AudioDecodeError> {
        self.decoder.read_frames(output)
    }

    pub(crate) fn restart(&mut self) -> Result<(), AudioDecodeError> {
        if self.decoder.restart()? {
            return Ok(());
        }
        self.decoder =
            MusicDecoder::open(self.data.clone(), self.output_sample_rate, self.purpose)?;
        Ok(())
    }

    /// Number of decoded PCM frames retained by the decoder right now.
    #[cfg(test)]
    pub(crate) fn buffered_frames(&self) -> usize {
        self.decoder.buffered_frames()
    }

    /// Largest decoded-PCM working set observed by this stream. This excludes
    /// the compressed source bytes and MIDI's compact event schedule.
    #[cfg(test)]
    pub(crate) fn peak_buffered_frames(&self) -> usize {
        self.decoder.peak_buffered_frames()
    }
}

#[derive(Clone, Copy)]
enum StreamPurpose {
    Music,
    Sound,
}

const CLASSIC_SOUND_SOURCE_SAMPLE_RATE: u32 = 44_100;

fn sound_source_sample_rate(format: AudioFormat, output_sample_rate: u32) -> u32 {
    match format {
        AudioFormat::Midi | AudioFormat::Tracker => CLASSIC_SOUND_SOURCE_SAMPLE_RATE,
        AudioFormat::Wav | AudioFormat::Ogg | AudioFormat::Mp3 => output_sample_rate,
    }
}

enum MusicDecoder {
    Pcm(Box<PcmStream>),
    Midi(crate::fluidsynth::MidiStream),
}

impl MusicDecoder {
    fn open(
        data: SharedAudioData,
        output_sample_rate: u32,
        purpose: StreamPurpose,
    ) -> Result<Self, AudioDecodeError> {
        let original_error = match Self::open_direct(data.clone(), output_sample_rate, purpose) {
            Ok(decoder) => return Ok(decoder),
            Err(error) => error,
        };
        let retry_data = data.clone();
        retry_mpeg_layer3_candidates(data.as_ref(), original_error, move |offset| {
            Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Mp3(Box::new(Mp3MusicStream::new_at(
                    retry_data.clone(),
                    offset,
                    purpose,
                )?)),
                output_sample_rate,
                purpose,
            )?)))
        })
    }

    fn open_direct(
        data: SharedAudioData,
        output_sample_rate: u32,
        purpose: StreamPurpose,
    ) -> Result<Self, AudioDecodeError> {
        let format = detect_format(data.as_ref())?;
        let source_sample_rate = match purpose {
            StreamPurpose::Music => output_sample_rate,
            StreamPurpose::Sound => sound_source_sample_rate(format, output_sample_rate),
        };
        match format {
            AudioFormat::Wav => Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Wav(WavStream::new(data)?),
                output_sample_rate,
                purpose,
            )?))),
            AudioFormat::Ogg => Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Ogg(Box::new(OggMusicStream::new(data, purpose)?)),
                output_sample_rate,
                purpose,
            )?))),
            AudioFormat::Mp3 => Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Mp3(Box::new(Mp3MusicStream::new(data, purpose)?)),
                output_sample_rate,
                purpose,
            )?))),
            AudioFormat::Midi => {
                let stream = crate::fluidsynth::MidiStream::new(data.as_ref(), source_sample_rate)?;
                if source_sample_rate == output_sample_rate {
                    Ok(Self::Midi(stream))
                } else {
                    Ok(Self::Pcm(Box::new(PcmStream::new(
                        PcmSource::Midi(Box::new(MidiMusicStream::new(
                            stream,
                            source_sample_rate,
                        )?)),
                        output_sample_rate,
                        purpose,
                    )?)))
                }
            }
            AudioFormat::Tracker => Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Tracker(Box::new(crate::tracker::TrackerStream::new(
                    data.as_ref(),
                    source_sample_rate,
                )?)),
                output_sample_rate,
                purpose,
            )?))),
        }
    }

    fn read_frames(&mut self, output: &mut [[f32; 2]]) -> Result<usize, AudioDecodeError> {
        match self {
            Self::Pcm(stream) => stream.read_frames(output),
            Self::Midi(stream) => stream.read_frames(output),
        }
    }

    /// Restart stateful sources in place when doing so preserves their parsed
    /// representation. Byte-oriented codecs are reconstructed by MusicStream.
    fn restart(&mut self) -> Result<bool, AudioDecodeError> {
        match self {
            Self::Pcm(stream) => stream.restart_source(),
            Self::Midi(stream) => {
                stream.restart()?;
                Ok(true)
            }
        }
    }

    #[cfg(test)]
    fn buffered_frames(&self) -> usize {
        match self {
            Self::Pcm(stream) => stream.buffered_frames(),
            Self::Midi(stream) => stream.buffered_frames(),
        }
    }

    #[cfg(test)]
    fn peak_buffered_frames(&self) -> usize {
        match self {
            Self::Pcm(stream) => stream.peak_buffered_frames(),
            Self::Midi(stream) => stream.buffered_frames(),
        }
    }
}

/// Stateful linear resampling over a pull-based source. Music preserves the
/// existing exact-rational stream mapping, while bounded sound decoding uses
/// the eager converter's floating-point mapping and ceil-rounded length.
struct PcmStream {
    source: PcmSource,
    current: Option<[f32; 2]>,
    next: Option<[f32; 2]>,
    resampling: ResamplingState,
}

enum ResamplingState {
    RationalMusic {
        source_sample_rate: u32,
        output_sample_rate: u32,
        current_index: u128,
        output_index: u128,
    },
    EagerSound {
        output_to_source_ratio: f64,
        output_len: Option<usize>,
        resampling: bool,
        current_index: usize,
        output_index: usize,
    },
}

impl PcmStream {
    fn new(
        mut source: PcmSource,
        output_sample_rate: u32,
        purpose: StreamPurpose,
    ) -> Result<Self, AudioDecodeError> {
        let source_sample_rate = source.sample_rate();
        if source_sample_rate == 0 {
            return Err(AudioDecodeError::InvalidData(
                "music source sample rate is zero",
            ));
        }
        let output_to_source_ratio = f64::from(output_sample_rate) / f64::from(source_sample_rate);
        let current = source.next_frame()?;
        let next = if current.is_some() {
            source.next_frame()?
        } else {
            None
        };
        let resampling = match purpose {
            StreamPurpose::Music => ResamplingState::RationalMusic {
                source_sample_rate,
                output_sample_rate,
                current_index: 0,
                output_index: 0,
            },
            StreamPurpose::Sound => ResamplingState::EagerSound {
                output_to_source_ratio,
                output_len: initial_eager_output_len(
                    current.is_some(),
                    next.is_some(),
                    output_to_source_ratio,
                ),
                resampling: output_sample_rate != source_sample_rate,
                current_index: 0,
                output_index: 0,
            },
        };
        Ok(Self {
            source,
            current,
            next,
            resampling,
        })
    }

    fn read_frames(&mut self, output: &mut [[f32; 2]]) -> Result<usize, AudioDecodeError> {
        match &mut self.resampling {
            ResamplingState::RationalMusic {
                source_sample_rate,
                output_sample_rate,
                current_index,
                output_index,
            } => Self::read_rational_music_frames(
                &mut self.source,
                &mut self.current,
                &mut self.next,
                *source_sample_rate,
                *output_sample_rate,
                current_index,
                output_index,
                output,
            ),
            ResamplingState::EagerSound {
                output_to_source_ratio,
                output_len,
                resampling,
                current_index,
                output_index,
            } => Self::read_eager_sound_frames(
                &mut self.source,
                &mut self.current,
                &mut self.next,
                *output_to_source_ratio,
                output_len,
                *resampling,
                current_index,
                output_index,
                output,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn read_rational_music_frames(
        source: &mut PcmSource,
        current: &mut Option<[f32; 2]>,
        next: &mut Option<[f32; 2]>,
        source_sample_rate: u32,
        output_sample_rate: u32,
        current_index: &mut u128,
        output_index: &mut u128,
        output: &mut [[f32; 2]],
    ) -> Result<usize, AudioDecodeError> {
        let mut written = 0;
        while written < output.len() {
            let scaled_position = output_index
                .checked_mul(u128::from(source_sample_rate))
                .ok_or(AudioDecodeError::DecoderError(
                    "music resampler position overflow",
                ))?;
            let desired_index = scaled_position / u128::from(output_sample_rate);

            while *current_index < desired_index {
                *current = next.take();
                *current_index =
                    current_index
                        .checked_add(1)
                        .ok_or(AudioDecodeError::DecoderError(
                            "music resampler position overflow",
                        ))?;
                if current.is_none() {
                    return Ok(written);
                }
                *next = source.next_frame()?;
            }

            let Some(current_frame) = *current else {
                break;
            };
            let next_frame = next.unwrap_or(current_frame);
            let remainder = scaled_position % u128::from(output_sample_rate);
            let fraction = remainder as f64 / f64::from(output_sample_rate);
            output[written] = [
                current_frame[0] + (next_frame[0] - current_frame[0]) * fraction as f32,
                current_frame[1] + (next_frame[1] - current_frame[1]) * fraction as f32,
            ];
            *output_index = output_index
                .checked_add(1)
                .ok_or(AudioDecodeError::DecoderError(
                    "music resampler output position overflow",
                ))?;
            written += 1;
        }
        Ok(written)
    }

    #[allow(clippy::too_many_arguments)]
    fn read_eager_sound_frames(
        source: &mut PcmSource,
        current: &mut Option<[f32; 2]>,
        next: &mut Option<[f32; 2]>,
        output_to_source_ratio: f64,
        output_len: &mut Option<usize>,
        resampling: bool,
        current_index: &mut usize,
        output_index: &mut usize,
        output: &mut [[f32; 2]],
    ) -> Result<usize, AudioDecodeError> {
        let mut written = 0;
        while written < output.len() {
            if output_len.is_some_and(|len| *output_index >= len) {
                break;
            }
            // Preserve the eager SFX converter's exact floating-point
            // operation order: ratio first, then output index divided by it.
            let source_position = *output_index as f64 / output_to_source_ratio;
            let desired_index = source_position.floor() as usize;

            while *current_index < desired_index {
                let Some(next_frame) = next.take() else {
                    break;
                };
                *current = Some(next_frame);
                *current_index =
                    current_index
                        .checked_add(1)
                        .ok_or(AudioDecodeError::DecoderError(
                            "sound resampler position overflow",
                        ))?;
                *next = source.next_frame()?;
                if next.is_none() {
                    let source_frames =
                        current_index
                            .checked_add(1)
                            .ok_or(AudioDecodeError::DecoderError(
                                "sound resampler position overflow",
                            ))?;
                    *output_len = Some(resampled_output_len(source_frames, output_to_source_ratio));
                }
            }

            if output_len.is_some_and(|len| *output_index >= len) {
                break;
            }
            let (current_frame, next_frame) = if *current_index == desired_index {
                let Some(current_frame) = *current else {
                    break;
                };
                (current_frame, next.unwrap_or(current_frame))
            } else {
                // The eager converter uses silence when floating-point ceil
                // produces an output position just past the final source
                // frame.
                ([0.0, 0.0], [0.0, 0.0])
            };
            let fraction = source_position - desired_index as f64;
            output[written] = if resampling {
                [
                    current_frame[0] + (next_frame[0] - current_frame[0]) * fraction as f32,
                    current_frame[1] + (next_frame[1] - current_frame[1]) * fraction as f32,
                ]
            } else {
                current_frame
            };
            *output_index = output_index
                .checked_add(1)
                .ok_or(AudioDecodeError::DecoderError(
                    "sound resampler output position overflow",
                ))?;
            written += 1;
        }
        Ok(written)
    }

    fn restart_source(&mut self) -> Result<bool, AudioDecodeError> {
        if !self.source.restart()? {
            return Ok(false);
        }
        self.current = self.source.next_frame()?;
        self.next = if self.current.is_some() {
            self.source.next_frame()?
        } else {
            None
        };
        match &mut self.resampling {
            ResamplingState::RationalMusic {
                current_index,
                output_index,
                ..
            } => {
                *current_index = 0;
                *output_index = 0;
            }
            ResamplingState::EagerSound {
                output_to_source_ratio,
                output_len,
                current_index,
                output_index,
                ..
            } => {
                *output_len = initial_eager_output_len(
                    self.current.is_some(),
                    self.next.is_some(),
                    *output_to_source_ratio,
                );
                *current_index = 0;
                *output_index = 0;
            }
        }
        Ok(true)
    }

    #[cfg(test)]
    fn buffered_frames(&self) -> usize {
        self.source
            .buffered_frames()
            .saturating_add(usize::from(self.current.is_some()))
            .saturating_add(usize::from(self.next.is_some()))
    }

    #[cfg(test)]
    fn peak_buffered_frames(&self) -> usize {
        self.source.peak_buffered_frames().saturating_add(2)
    }
}

fn initial_eager_output_len(
    has_current: bool,
    has_next: bool,
    output_to_source_ratio: f64,
) -> Option<usize> {
    match (has_current, has_next) {
        (false, _) => Some(0),
        (true, false) => Some(resampled_output_len(1, output_to_source_ratio)),
        (true, true) => None,
    }
}

fn resampled_output_len(source_frames: usize, output_to_source_ratio: f64) -> usize {
    (source_frames as f64 * output_to_source_ratio).ceil() as usize
}

enum PcmSource {
    Wav(WavStream),
    Ogg(Box<OggMusicStream>),
    Mp3(Box<Mp3MusicStream>),
    Midi(Box<MidiMusicStream>),
    Tracker(Box<crate::tracker::TrackerStream>),
}

impl PcmSource {
    fn sample_rate(&self) -> u32 {
        match self {
            Self::Wav(stream) => stream.sample_rate(),
            Self::Ogg(stream) => stream.sample_rate,
            Self::Mp3(stream) => stream.sample_rate,
            Self::Midi(stream) => stream.sample_rate,
            Self::Tracker(stream) => stream.sample_rate(),
        }
    }

    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        match self {
            Self::Wav(stream) => stream.next_frame(),
            Self::Ogg(stream) => stream.next_frame(),
            Self::Mp3(stream) => stream.next_frame(),
            Self::Midi(stream) => stream.next_frame(),
            Self::Tracker(stream) => stream.next_frame(),
        }
    }

    fn restart(&mut self) -> Result<bool, AudioDecodeError> {
        match self {
            Self::Midi(stream) => {
                stream.restart()?;
                Ok(true)
            }
            Self::Tracker(stream) => {
                stream.restart()?;
                Ok(true)
            }
            Self::Wav(_) | Self::Ogg(_) | Self::Mp3(_) => Ok(false),
        }
    }

    #[cfg(test)]
    fn buffered_frames(&self) -> usize {
        match self {
            Self::Wav(stream) => stream.buffered_frames(),
            Self::Ogg(stream) => stream.buffered_frames(),
            Self::Mp3(stream) => stream.buffered_frames(),
            Self::Midi(stream) => stream.buffered_frames(),
            Self::Tracker(stream) => stream.buffered_frames(),
        }
    }

    #[cfg(test)]
    fn peak_buffered_frames(&self) -> usize {
        match self {
            Self::Wav(stream) => stream.peak_buffered_frames(),
            Self::Ogg(stream) => stream.peak_packet_frames,
            Self::Mp3(stream) => stream.peak_packet_frames,
            Self::Midi(stream) => stream.buffered_frames(),
            Self::Tracker(stream) => stream.peak_buffered_frames(),
        }
    }
}

/// Frame-at-a-time adapter used only when a synthesized 44.1 kHz sound must
/// pass through the streaming output resampler. Refilling in blocks preserves
/// the old eager decoder's FluidSynth call shape without retaining whole PCM.
struct MidiMusicStream {
    stream: crate::fluidsynth::MidiStream,
    sample_rate: u32,
    buffer: Vec<[f32; 2]>,
    buffer_offset: usize,
    buffer_len: usize,
}

impl MidiMusicStream {
    const BUFFER_FRAMES: usize = 4_096;

    fn new(
        stream: crate::fluidsynth::MidiStream,
        sample_rate: u32,
    ) -> Result<Self, AudioDecodeError> {
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(Self::BUFFER_FRAMES)
            .map_err(|_| AudioDecodeError::DecodedAudioAllocationFailed)?;
        buffer.resize(Self::BUFFER_FRAMES, [0.0, 0.0]);
        Ok(Self {
            stream,
            sample_rate,
            buffer,
            buffer_offset: 0,
            buffer_len: 0,
        })
    }

    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        if self.buffer_offset == self.buffer_len {
            self.buffer_len = self.stream.read_frames(&mut self.buffer)?;
            self.buffer_offset = 0;
            if self.buffer_len == 0 {
                return Ok(None);
            }
        }
        let frame = self.buffer[self.buffer_offset];
        self.buffer_offset += 1;
        Ok(Some(frame))
    }

    fn restart(&mut self) -> Result<(), AudioDecodeError> {
        self.stream.restart()?;
        self.buffer_offset = 0;
        self.buffer_len = 0;
        Ok(())
    }

    #[cfg(test)]
    fn buffered_frames(&self) -> usize {
        self.buffer.capacity()
    }
}

struct OggMusicStream {
    reader: OggStreamReader<Cursor<SharedAudioData>>,
    purpose: StreamPurpose,
    sample_rate: u32,
    channels: usize,
    packet: Vec<i16>,
    packet_offset: usize,
    #[cfg(test)]
    peak_packet_frames: usize,
}

impl OggMusicStream {
    fn new(data: SharedAudioData, purpose: StreamPurpose) -> Result<Self, AudioDecodeError> {
        let reader = OggStreamReader::new(Cursor::new(data))
            .map_err(|_| AudioDecodeError::InvalidData("invalid OGG header"))?;
        let sample_rate = reader.ident_hdr.audio_sample_rate;
        if sample_rate == 0 {
            return Err(AudioDecodeError::InvalidData("OGG sample rate is zero"));
        }
        let channels = usize::from(reader.ident_hdr.audio_channels);
        if channels == 0 {
            return Err(AudioDecodeError::InvalidData("OGG channel count is zero"));
        }
        Ok(Self {
            reader,
            purpose,
            sample_rate,
            channels,
            packet: Vec::new(),
            packet_offset: 0,
            #[cfg(test)]
            peak_packet_frames: 0,
        })
    }

    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        loop {
            if self.packet.len().saturating_sub(self.packet_offset) >= self.channels {
                let start = self.packet_offset;
                self.packet_offset += self.channels;
                return Ok(Some(stereo_i16_frame(
                    &self.packet[start..start + self.channels],
                    self.purpose,
                )));
            }
            let Some(packet) = self
                .reader
                .read_dec_packet_itl()
                .map_err(|_| AudioDecodeError::DecoderError("failed to decode OGG packet"))?
            else {
                return Ok(None);
            };
            self.packet = packet;
            self.packet_offset = 0;
            #[cfg(test)]
            {
                self.peak_packet_frames = self
                    .peak_packet_frames
                    .max(frame_capacity(self.packet.capacity(), self.channels));
            }
        }
    }

    #[cfg(test)]
    fn buffered_frames(&self) -> usize {
        frame_capacity(self.packet.capacity(), self.channels)
    }
}

/// symphonia's MPEG-audio demuxer paired with its decoder, bound to the first
/// audio track in the stream. Both the whole-file and the streaming MP3 paths
/// pull packets through this, so they share one notion of frame and error.
struct MpegAudioDecoder<'s> {
    reader: MpaReader<'s>,
    decoder: MpaDecoder,
    track_id: u32,
}

impl<'s> MpegAudioDecoder<'s> {
    fn open(source: Box<dyn MediaSource + 's>) -> Result<Self, AudioDecodeError> {
        let stream = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
        let reader = MpaReader::try_new(stream, FormatOptions::default())
            .map_err(|_| AudioDecodeError::Mp3DecoderError("empty MP3 data"))?;
        let (track_id, params) = reader
            .tracks()
            .iter()
            .find_map(|track| match track.codec_params.as_ref() {
                Some(CodecParameters::Audio(params)) => Some((track.id, params.clone())),
                _ => None,
            })
            .ok_or(AudioDecodeError::Mp3DecoderError("empty MP3 data"))?;
        let decoder = MpaDecoder::try_new(&params, &AudioDecoderOptions::default())
            .map_err(|_| AudioDecodeError::Mp3DecoderError("failed to decode MP3 frame"))?;
        Ok(Self {
            reader,
            decoder,
            track_id,
        })
    }

    /// Decode the next audio packet into `packet` as interleaved 16-bit samples,
    /// reporting its sample rate and channel count. `None` ends the stream.
    fn next_packet(
        &mut self,
        packet: &mut Vec<i16>,
    ) -> Result<Option<(u32, usize)>, AudioDecodeError> {
        loop {
            let source_packet = match self.reader.next_packet() {
                Ok(Some(source_packet)) => source_packet,
                Ok(None) => return Ok(None),
                // A stream that stops mid-frame ends there rather than failing,
                // matching how C4AudioSystemSdl tolerates truncated music.
                Err(SymphoniaError::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(_) => {
                    return Err(AudioDecodeError::Mp3DecoderError(
                        "failed to decode MP3 frame",
                    ));
                }
            };
            if source_packet.track_id != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&source_packet) {
                Ok(decoded) => decoded,
                // Skip a corrupt frame instead of truncating the stream, the
                // way minimp3's frame search resynchronised past damage.
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(_) => {
                    return Err(AudioDecodeError::Mp3DecoderError(
                        "failed to decode MP3 frame",
                    ));
                }
            };
            let spec = decoded.spec();
            let (sample_rate, channels) = (spec.rate(), spec.channels().count());
            decoded.copy_to_vec_interleaved(packet);
            return Ok(Some((sample_rate, channels)));
        }
    }
}

/// Pull-based MP3 music. Only the current packet's PCM is retained, so memory
/// stays flat no matter how long the track runs.
struct Mp3MusicStream {
    decoder: MpegAudioDecoder<'static>,
    purpose: StreamPurpose,
    sample_rate: u32,
    channels: usize,
    packet: Vec<i16>,
    packet_offset: usize,
    #[cfg(test)]
    peak_packet_frames: usize,
}

impl Mp3MusicStream {
    fn new(data: SharedAudioData, purpose: StreamPurpose) -> Result<Self, AudioDecodeError> {
        Self::new_at(data, 0, purpose)
    }

    fn new_at(
        data: SharedAudioData,
        offset: usize,
        purpose: StreamPurpose,
    ) -> Result<Self, AudioDecodeError> {
        let mut cursor = Cursor::new(data);
        cursor.set_position(offset as u64);
        let mut stream = Self {
            decoder: MpegAudioDecoder::open(Box::new(cursor))?,
            purpose,
            sample_rate: 0,
            channels: 0,
            packet: Vec::new(),
            packet_offset: 0,
            #[cfg(test)]
            peak_packet_frames: 0,
        };
        if !stream.load_packet()? {
            return Err(AudioDecodeError::Mp3DecoderError("empty MP3 data"));
        }
        Ok(stream)
    }

    fn load_packet(&mut self) -> Result<bool, AudioDecodeError> {
        let Some((sample_rate, channels)) = self.decoder.next_packet(&mut self.packet)? else {
            return Ok(false);
        };
        self.update_packet_format(sample_rate, channels)?;
        self.packet_offset = 0;
        #[cfg(test)]
        {
            self.peak_packet_frames = self
                .peak_packet_frames
                .max(frame_capacity(self.packet.capacity(), self.channels));
        }
        Ok(true)
    }

    fn update_packet_format(
        &mut self,
        sample_rate: u32,
        channels: usize,
    ) -> Result<(), AudioDecodeError> {
        if sample_rate == 0 {
            return Err(AudioDecodeError::InvalidData("MP3 sample rate is zero"));
        }
        if channels == 0 {
            return Err(AudioDecodeError::InvalidData("MP3 channel count is zero"));
        }
        match self.sample_rate {
            0 => self.sample_rate = sample_rate,
            previous if previous != sample_rate && matches!(self.purpose, StreamPurpose::Music) => {
                return Err(AudioDecodeError::InvalidData(
                    "MP3 sample rate changes within stream",
                ));
            }
            _ => {}
        }
        self.channels = channels;
        Ok(())
    }

    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        loop {
            if self.packet.len().saturating_sub(self.packet_offset) >= self.channels {
                let start = self.packet_offset;
                self.packet_offset += self.channels;
                return Ok(Some(stereo_i16_frame(
                    &self.packet[start..start + self.channels],
                    self.purpose,
                )));
            }
            if !self.load_packet()? {
                return Ok(None);
            }
        }
    }

    #[cfg(test)]
    fn buffered_frames(&self) -> usize {
        frame_capacity(self.packet.capacity(), self.channels)
    }
}

fn stereo_i16_frame(samples: &[i16], purpose: StreamPurpose) -> [f32; 2] {
    match purpose {
        StreamPurpose::Music => {
            let mut left = f32::from(samples[0]);
            if samples.len() == 1 {
                let mono = left / f32::from(i16::MAX);
                return [mono, mono];
            }
            let mut right = f32::from(samples[1]);
            let mut left_count = 1_usize;
            let mut right_count = 1_usize;
            for (index, sample) in samples[2..].iter().enumerate() {
                if index % 2 == 0 {
                    left += f32::from(*sample);
                    left_count += 1;
                } else {
                    right += f32::from(*sample);
                    right_count += 1;
                }
            }
            [
                left / left_count as f32 / f32::from(i16::MAX),
                right / right_count as f32 / f32::from(i16::MAX),
            ]
        }
        StreamPurpose::Sound => {
            let normalize = |sample| f32::from(sample) / f32::from(i16::MAX);
            let mut left = normalize(samples[0]);
            if samples.len() == 1 {
                return [left, left];
            }
            let mut right = normalize(samples[1]);
            let mut left_count = 1_usize;
            let mut right_count = 1_usize;
            for (index, sample) in samples[2..].iter().enumerate() {
                if index % 2 == 0 {
                    left += normalize(*sample);
                    left_count += 1;
                } else {
                    right += normalize(*sample);
                    right_count += 1;
                }
            }
            if left_count > 1 {
                left /= left_count as f32;
            }
            if right_count > 1 {
                right /= right_count as f32;
            }
            [left, right]
        }
    }
}

#[cfg(test)]
fn frame_capacity(sample_capacity: usize, channels: usize) -> usize {
    sample_capacity / channels + usize::from(!sample_capacity.is_multiple_of(channels))
}

// C4AudioSystemSdl bounds its fallback search by the maximum possible MPEG
// Layer III frame size: 144 * maximum bitrate / minimum sample rate + padding.
const MAX_MPEG_LAYER3_FRAME_SIZE: usize = 144 * 320_000 / 8_000 + 1;

fn retry_mpeg_layer3_candidates<T>(
    data: &[u8],
    original_error: AudioDecodeError,
    mut load: impl FnMut(usize) -> Result<T, AudioDecodeError>,
) -> Result<T, AudioDecodeError> {
    let limit = data.len().min(MAX_MPEG_LAYER3_FRAME_SIZE);
    for offset in 0..limit.saturating_sub(4) {
        if !is_mpeg_layer3_header(&data[offset..offset + 4]) {
            continue;
        }
        if let Ok(decoded) = load(offset) {
            return Ok(decoded);
        }
    }
    Err(original_error)
}

fn is_mpeg_layer3_header(header: &[u8]) -> bool {
    let [byte1, byte2, byte3, byte4, ..] = header else {
        return false;
    };
    *byte1 == 0xff
        && (*byte2 & 0xe6) == 0xe2
        && (*byte2 & 0x18) != 0x08
        && (*byte3 & 0xf0) != 0xf0
        && (*byte3 & 0x0c) != 0x0c
        && (*byte4 & 0x03) != 0x02
}

fn detect_format(data: &[u8]) -> Result<AudioFormat, AudioDecodeError> {
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return Ok(AudioFormat::Wav);
    }
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"RMID" {
        return Ok(AudioFormat::Midi);
    }
    if data.len() >= 4 && &data[0..4] == b"OggS" {
        return Ok(AudioFormat::Ogg);
    }
    if data.len() >= 3 && &data[0..3] == b"ID3" {
        return Ok(AudioFormat::Mp3);
    }
    if data.len() >= 2 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0 {
        return Ok(AudioFormat::Mp3);
    }
    if data.len() >= 4 && &data[0..4] == b"MThd" {
        return Ok(AudioFormat::Midi);
    }
    if crate::tracker::looks_like_tracker(data) {
        return Ok(AudioFormat::Tracker);
    }
    if crate::tracker::probe_tracker(data)? {
        return Ok(AudioFormat::Tracker);
    }
    Err(AudioDecodeError::UnsupportedFormat)
}

fn decode_wav(data: &[u8]) -> Result<DecodedAudio, AudioDecodeError> {
    let source = SharedAudioData::try_from_slice(data)?;
    let mut stream = WavStream::new(source)?;
    let sample_rate = stream.sample_rate();
    let mut frames = Vec::new();
    while let Some(frame) = stream.next_frame()? {
        frames.push(frame);
    }
    Ok(DecodedAudio {
        frames,
        sample_rate,
    })
}

fn decode_ogg(data: &[u8]) -> Result<DecodedAudio, AudioDecodeError> {
    let cursor = Cursor::new(data);
    let mut reader = OggStreamReader::new(cursor)
        .map_err(|_| AudioDecodeError::InvalidData("invalid OGG header"))?;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    if sample_rate == 0 {
        return Err(AudioDecodeError::InvalidData("OGG sample rate is zero"));
    }
    let channel_count = reader.ident_hdr.audio_channels as usize;
    if channel_count == 0 {
        return Err(AudioDecodeError::InvalidData("OGG channel count is zero"));
    }

    let mut frames = Vec::new();
    while let Some(packet) = reader
        .read_dec_packet_itl()
        .map_err(|_| AudioDecodeError::DecoderError("failed to decode OGG packet"))?
    {
        let packet_frames = convert_interleaved_i16_to_stereo(&packet, channel_count)?;
        frames.extend(packet_frames);
    }

    Ok(DecodedAudio {
        frames,
        sample_rate,
    })
}

fn decode_mp3(data: &[u8]) -> Result<DecodedAudio, AudioDecodeError> {
    let mut decoder = MpegAudioDecoder::open(Box::new(Cursor::new(data)))?;
    let mut frames = Vec::new();
    let mut sample_rate: Option<u32> = None;
    let mut packet = Vec::new();
    while let Some((packet_rate, channel_count)) = decoder.next_packet(&mut packet)? {
        sample_rate.get_or_insert(packet_rate);
        let packet_frames = convert_interleaved_i16_to_stereo(&packet, channel_count)?;
        frames.extend(packet_frames);
    }

    let sample_rate = sample_rate.ok_or(AudioDecodeError::Mp3DecoderError("empty MP3 data"))?;
    Ok(DecodedAudio {
        frames,
        sample_rate,
    })
}

fn convert_interleaved_to_stereo(
    samples: &[f32],
    channels: usize,
) -> Result<Vec<[f32; 2]>, AudioDecodeError> {
    if channels == 0 {
        return Err(AudioDecodeError::InvalidData("channel count is zero"));
    }
    let frames = samples.len() / channels;
    let mut result = Vec::with_capacity(frames);
    for frame_idx in 0..frames {
        let base = frame_idx * channels;
        let (left, right) = match channels {
            1 => {
                let sample = samples[base];
                (sample, sample)
            }
            2 => (samples[base], samples[base + 1]),
            _ => {
                let mut left = samples[base];
                let mut right = samples[base + 1];
                let mut left_count = 1usize;
                let mut right_count = 1usize;
                for (index, sample) in samples[base + 2..base + channels].iter().enumerate() {
                    if index % 2 == 0 {
                        left += *sample;
                        left_count += 1;
                    } else {
                        right += *sample;
                        right_count += 1;
                    }
                }
                if left_count > 1 {
                    left /= left_count as f32;
                }
                if right_count > 1 {
                    right /= right_count as f32;
                }
                (left, right)
            }
        };
        result.push([left, right]);
    }
    Ok(result)
}

fn convert_interleaved_i16_to_stereo(
    samples: &[i16],
    channels: usize,
) -> Result<Vec<[f32; 2]>, AudioDecodeError> {
    let float_samples: Vec<f32> = samples
        .iter()
        .map(|sample| *sample as f32 / i16::MAX as f32)
        .collect();
    convert_interleaved_to_stereo(&float_samples, channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent_mpeg_layer3_frame() -> [u8; 72] {
        // One independently decodable MPEG-2.5 Layer III mono frame: 576
        // samples at 8 kHz in 72 compressed bytes.
        let mut frame = [0x55_u8; 72];
        frame[..13].copy_from_slice(&[
            0xff, 0xe3, 0x18, 0xc4, 0x00, 0x00, 0x00, 0x03, 0x48, 0x00, 0x00, 0x00, 0x00,
        ]);
        frame[13..22].copy_from_slice(b"LAME3.100");
        frame[53..62].copy_from_slice(b"LAME3.100");
        frame
    }

    /// The pull-based music path and the whole-file path run the same demuxer
    /// and decoder, so they must agree sample for sample. A decoder swap that
    /// desynchronised them would otherwise only show up as audible drift.
    #[test]
    fn mp3_streaming_matches_whole_file_decode() {
        let data = silent_mpeg_layer3_frame().repeat(4);
        let decoded = decode_audio(&data).expect("whole-file MP3 decode");
        assert_eq!(decoded.sample_rate, 8_000);
        assert!(!decoded.frames.is_empty(), "fixture decoded to no audio");
        assert!(
            decoded.frames.iter().flatten().all(|sample| *sample == 0.0),
            "silent fixture decoded to non-silent PCM"
        );

        let source = SharedAudioData::from_owned(data);
        let mut music =
            MusicStream::open(source, decoded.sample_rate).expect("MP3 music stream opens");
        let mut streamed = Vec::new();
        let mut output = [[0.0_f32; 2]; 64];
        loop {
            let count = music.read_frames(&mut output).expect("MP3 stream reads");
            if count == 0 {
                break;
            }
            streamed.extend_from_slice(&output[..count]);
        }
        assert_eq!(
            streamed, decoded.frames,
            "streamed MP3 PCM diverged from the whole-file decode"
        );
    }

    #[test]
    fn mp3_stream_retains_the_first_packet_rate_and_updates_channels() {
        let data = silent_mpeg_layer3_frame().repeat(4);
        let mut stream =
            Mp3MusicStream::new(SharedAudioData::from_owned(data), StreamPurpose::Sound)
                .expect("MP3 stream opens");
        assert_eq!(stream.sample_rate, 8_000);
        assert_eq!(stream.channels, 1);

        stream
            .update_packet_format(11_025, 2)
            .expect("later nonzero packet format is accepted");

        assert_eq!(stream.sample_rate, 8_000);
        assert_eq!(stream.channels, 2);
    }

    #[test]
    fn mp3_music_stream_rejects_a_later_sample_rate_change() {
        let data = silent_mpeg_layer3_frame().repeat(4);
        let mut stream =
            Mp3MusicStream::new(SharedAudioData::from_owned(data), StreamPurpose::Music)
                .expect("MP3 stream opens");

        assert!(matches!(
            stream.update_packet_format(11_025, 2),
            Err(AudioDecodeError::InvalidData(
                "MP3 sample rate changes within stream"
            ))
        ));
    }

    #[test]
    fn bounded_sound_decode_rejects_the_first_frame_past_its_limit() {
        let data = mono_pcm16_wav(44_100, &[0, 1, 2, 3]);
        let result = decode_audio_bounded_for_output(SharedAudioData::from_owned(data), 44_100, 3);

        assert!(matches!(
            result,
            Err(AudioDecodeError::DecodedAudioTooLarge)
        ));
    }

    #[test]
    fn bounded_sound_uses_the_classic_render_rate_for_synthesized_formats() {
        for format in [AudioFormat::Midi, AudioFormat::Tracker] {
            assert_eq!(sound_source_sample_rate(format, 48_000), 44_100);
        }
        for format in [AudioFormat::Wav, AudioFormat::Ogg, AudioFormat::Mp3] {
            assert_eq!(sound_source_sample_rate(format, 48_000), 48_000);
        }
    }

    #[test]
    fn owned_audio_source_shares_the_original_vec_allocation() {
        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(b"owned decoder source");
        let allocation = data.as_ptr();

        let shared = SharedAudioData::from_owned(data);

        assert_eq!(shared.as_ref().as_ptr(), allocation);
    }

    #[test]
    fn streaming_multichannel_i16_downmix_matches_eager_conversion() {
        let samples = [i16::MAX, i16::MAX, i16::MIN, i16::MIN, 1, 1];
        let eager = convert_interleaved_i16_to_stereo(&samples, 6).unwrap();

        assert_eq!(stereo_i16_frame(&samples, StreamPurpose::Sound), eager[0]);
    }

    #[test]
    fn music_multichannel_i16_downmix_preserves_streaming_rounding() {
        let samples = [i16::MIN, 0, i16::MIN, 0, -30_000, 0];
        let expected_left = (f32::from(samples[0]) + f32::from(samples[2]) + f32::from(samples[4]))
            / 3.0
            / f32::from(i16::MAX);

        assert_eq!(
            stereo_i16_frame(&samples, StreamPurpose::Music)[0],
            expected_left
        );
    }

    fn mono_pcm16_wav(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
        let data_len = u32::try_from(samples.len() * 2).unwrap();
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        wav
    }

    fn mono_ima_adpcm_wav(sample_rate: u32, initial: i16, payload: [u8; 4]) -> Vec<u8> {
        let mut fmt = Vec::with_capacity(20);
        fmt.extend_from_slice(&0x11_u16.to_le_bytes());
        fmt.extend_from_slice(&1_u16.to_le_bytes());
        fmt.extend_from_slice(&sample_rate.to_le_bytes());
        fmt.extend_from_slice(&(sample_rate * 8 / 9).to_le_bytes());
        fmt.extend_from_slice(&8_u16.to_le_bytes());
        fmt.extend_from_slice(&4_u16.to_le_bytes());
        fmt.extend_from_slice(&2_u16.to_le_bytes());
        fmt.extend_from_slice(&9_u16.to_le_bytes());

        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&initial.to_le_bytes());
        data.extend_from_slice(&[0, 0]);
        data.extend_from_slice(&payload);

        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        wav.extend_from_slice(&fmt);
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        let riff_len = u32::try_from(wav.len() - 8).unwrap();
        wav[4..8].copy_from_slice(&riff_len.to_le_bytes());
        wav
    }

    #[test]
    fn detects_standard_midi_header() {
        // C4MusicSystem.cpp:31-32,101-113 accepts MID data for SDL_mixer.
        assert!(matches!(
            detect_format(b"MThd\0\0\0\x06\0\0\0\x01\0\x60"),
            Ok(AudioFormat::Midi)
        ));
    }

    #[test]
    fn detects_riff_wrapped_midi_header() {
        // C4AudioSystemSdl.cpp:280-282 delegates RMID recognition to SDL_mixer.
        assert!(matches!(
            detect_format(b"RIFF\x10\0\0\0RMIDdata\x04\0\0\0MThd"),
            Ok(AudioFormat::Midi)
        ));
    }

    #[test]
    fn malformed_tracker_data_returns_typed_decode_error() {
        let mut s3m = vec![0_u8; 48];
        s3m[44..48].copy_from_slice(b"SCRM");
        let mut module = vec![0_u8; 1_084];
        module[1_080..1_084].copy_from_slice(b"M.K.");

        for malformed in [
            b"IMPM".as_slice(),
            b"Extended Module: ".as_slice(),
            s3m.as_slice(),
            module.as_slice(),
        ] {
            assert!(matches!(
                decode_audio_for_output(malformed, 8_000),
                Err(AudioDecodeError::TrackerDecoderError(_))
            ));
        }
        assert!(matches!(
            decode_audio_for_output(b"not audio", 8_000),
            Err(AudioDecodeError::UnsupportedFormat)
        ));
    }

    #[test]
    fn mp3_leading_junk_recovers_first_valid_layer3_frame() {
        let clean_mp3 = silent_mpeg_layer3_frame().repeat(2);
        let clean = decode_audio(&clean_mp3).expect("frame-zero MP3 decodes directly");
        let mut data = b"legacy-prefix".to_vec();
        let invalid_headers = [
            [0xfe, 0xe3, 0x18, 0xc4], // incomplete frame sync
            [0xff, 0xe5, 0x18, 0xc4], // not Layer III
            [0xff, 0xea, 0x18, 0xc4], // reserved MPEG version
            [0xff, 0xe3, 0xf8, 0xc4], // invalid bitrate index
            [0xff, 0xe3, 0x1c, 0xc4], // reserved sample-rate index
            [0xff, 0xe3, 0x18, 0xc6], // reserved emphasis
        ];
        for header in invalid_headers {
            assert!(!is_mpeg_layer3_header(&header));
            data.extend_from_slice(&header);
        }
        assert!(is_mpeg_layer3_header(&clean_mp3[..4]));
        for byte2 in [0xe2, 0xe3, 0xf2, 0xf3, 0xfa, 0xfb] {
            assert!(is_mpeg_layer3_header(&[0xff, byte2, 0x18, 0xc4]));
        }
        assert!(
            is_mpeg_layer3_header(&[0xff, 0xe3, 0x08, 0xc4]),
            "native prefilter permits the free-bitrate index"
        );
        for emphasis in [0, 1, 3] {
            assert!(is_mpeg_layer3_header(&[0xff, 0xe3, 0x18, 0xc4 | emphasis,]));
        }
        data.extend_from_slice(&clean_mp3);

        let decoded = decode_audio(&data).expect("sound decoder recovers later MP3 frame");
        assert_eq!(decoded.sample_rate, clean.sample_rate);
        assert_eq!(decoded.frames, clean.frames);

        let source = SharedAudioData::from_owned(data);
        let mut music = MusicStream::open(source, 8_000)
            .expect("music stream recovers the same later MP3 frame");
        let mut output = [[0.0_f32; 2]; 1];
        assert_eq!(music.read_frames(&mut output).unwrap(), 1);
        music.restart().expect("recovered MP3 stream restarts");
        assert_eq!(music.read_frames(&mut output).unwrap(), 1);

        let false_sync = b"junk\xff\xe3\x18\xc4\0";
        assert!(matches!(
            decode_audio(false_sync),
            Err(AudioDecodeError::UnsupportedFormat)
        ));
        assert!(matches!(
            MusicStream::open(SharedAudioData::try_from_slice(false_sync).unwrap(), 8_000),
            Err(AudioDecodeError::UnsupportedFormat)
        ));

        let mut malformed_wav = b"RIFF\0\0\0\0WAVE".to_vec();
        malformed_wav.extend_from_slice(false_sync);
        assert!(matches!(
            decode_audio(&malformed_wav),
            Err(AudioDecodeError::InvalidData("invalid WAV header"))
        ));
        assert!(matches!(
            MusicStream::open(SharedAudioData::from_owned(malformed_wav), 8_000),
            Err(AudioDecodeError::InvalidData("invalid WAV header"))
        ));

        let mut candidate_data = vec![0_u8; 20];
        for offset in [2, 8, 14] {
            candidate_data[offset..offset + 4].copy_from_slice(&clean_mp3[..4]);
        }
        let mut attempted = Vec::new();
        let recovered_offset = retry_mpeg_layer3_candidates(
            &candidate_data,
            AudioDecodeError::UnsupportedFormat,
            |offset| {
                attempted.push(offset);
                if offset == 8 {
                    Ok(offset)
                } else {
                    Err(AudioDecodeError::Mp3DecoderError("synthetic failure"))
                }
            },
        )
        .expect("second candidate succeeds");
        assert_eq!(recovered_offset, 8);
        assert_eq!(attempted, [2, 8]);

        let mut last_included = vec![0_u8; MAX_MPEG_LAYER3_FRAME_SIZE - 5];
        last_included.extend_from_slice(&clean_mp3);
        assert!(decode_audio(&last_included).is_ok());
        let mut first_excluded = vec![0_u8; MAX_MPEG_LAYER3_FRAME_SIZE - 4];
        first_excluded.extend_from_slice(&clean_mp3);
        assert!(matches!(
            decode_audio(&first_excluded),
            Err(AudioDecodeError::UnsupportedFormat)
        ));

        let id3_payload_len = MAX_MPEG_LAYER3_FRAME_SIZE;
        let mut id3 = b"ID3\x04\0\0".to_vec();
        id3.extend_from_slice(&[
            ((id3_payload_len >> 21) & 0x7f) as u8,
            ((id3_payload_len >> 14) & 0x7f) as u8,
            ((id3_payload_len >> 7) & 0x7f) as u8,
            (id3_payload_len & 0x7f) as u8,
        ]);
        id3.resize(10 + id3_payload_len, 0);
        id3.extend_from_slice(&clean_mp3);
        assert!(decode_audio(&id3).is_ok());
        assert!(MusicStream::open(SharedAudioData::from_owned(id3), 8_000).is_ok());

        for len in 0..=4 {
            let short = vec![0_u8; len];
            let direct = decode_audio_for_output_direct(&short, 44_100).unwrap_err();
            let recovered = decode_audio(&short).unwrap_err();
            assert_eq!(
                std::mem::discriminant(&recovered),
                std::mem::discriminant(&direct)
            );
            assert_eq!(recovered.to_string(), direct.to_string());
        }
    }

    #[test]
    fn music_stream_resamples_in_bounded_chunks_and_restarts() {
        let source = SharedAudioData::from_owned(mono_pcm16_wav(2, &[0, i16::MAX]));
        let mut stream = MusicStream::open(source, 4).unwrap();
        assert_eq!(stream.buffered_frames(), 2);
        assert_eq!(stream.peak_buffered_frames(), 2);

        let mut output = [[0.0_f32; 2]; 8];
        assert_eq!(stream.read_frames(&mut output).unwrap(), 4);
        assert_eq!(output[..4], [[0.0; 2], [0.5; 2], [1.0; 2], [1.0; 2]]);
        assert_eq!(stream.read_frames(&mut output).unwrap(), 0);

        stream.restart().unwrap();
        assert_eq!(stream.read_frames(&mut output[..2]).unwrap(), 2);
        assert_eq!(output[..2], [[0.0; 2], [0.5; 2]]);
    }

    #[test]
    fn music_stream_preserves_rational_resampling_at_non_integral_rates() {
        let mut samples = vec![0_i16; 443];
        samples[440] = i16::MIN;
        samples[441] = -1;
        let source = SharedAudioData::from_owned(mono_pcm16_wav(44_100, &samples));
        let mut stream = MusicStream::open(source, 48_000).unwrap();
        let mut output = [[0.0_f32; 2]; 481];

        assert_eq!(stream.read_frames(&mut output).unwrap(), output.len());
        let expected = f32::from(-1_i16) / f32::from(i16::MAX);
        assert_eq!(output[480], [expected; 2]);

        let source = SharedAudioData::from_owned(mono_pcm16_wav(11_025, &[0; 147]));
        let mut stream = MusicStream::open(source, 24_000).unwrap();
        let mut output = [[0.0_f32; 2]; 321];
        assert_eq!(stream.read_frames(&mut output).unwrap(), 320);
        assert_eq!(stream.read_frames(&mut output).unwrap(), 0);
    }

    #[test]
    fn compressed_wav_music_streams_in_bounded_blocks_and_resamples() {
        let source = SharedAudioData::from_owned(mono_ima_adpcm_wav(2, 0, [0x11; 4]));
        let mut stream = MusicStream::open(source, 4).unwrap();
        assert_eq!(stream.buffered_frames(), 11);
        assert_eq!(stream.peak_buffered_frames(), 11);

        let mut output = [[0.0_f32; 2]; 4];
        assert_eq!(stream.read_frames(&mut output).unwrap(), 4);
        let scale = f32::from(i16::MAX);
        for (frame, expected) in output.iter().zip([0.0, 0.5, 1.0, 1.5]) {
            assert!((frame[0] - expected / scale).abs() < 1.0e-7);
            assert_eq!(frame[0], frame[1]);
        }

        stream.restart().unwrap();
        assert_eq!(stream.read_frames(&mut output[..2]).unwrap(), 2);
        assert!((output[1][0] - 0.5 / scale).abs() < 1.0e-7);
    }
}
