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
    #[error("io error")]
    IoError(#[from] std::io::Error),
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
    data: Arc<[u8]>,
    output_sample_rate: u32,
    decoder: MusicDecoder,
}

impl MusicStream {
    pub(crate) fn open(data: Arc<[u8]>, output_sample_rate: u32) -> Result<Self, AudioDecodeError> {
        if output_sample_rate == 0 {
            return Err(AudioDecodeError::InvalidData(
                "music output sample rate is zero",
            ));
        }
        let decoder = MusicDecoder::open(data.clone(), output_sample_rate)?;
        Ok(Self {
            data,
            output_sample_rate,
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
        self.decoder = MusicDecoder::open(self.data.clone(), self.output_sample_rate)?;
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

enum MusicDecoder {
    Pcm(Box<PcmStream>),
    Midi(crate::fluidsynth::MidiStream),
}

impl MusicDecoder {
    fn open(data: Arc<[u8]>, output_sample_rate: u32) -> Result<Self, AudioDecodeError> {
        let original_error = match Self::open_direct(data.clone(), output_sample_rate) {
            Ok(decoder) => return Ok(decoder),
            Err(error) => error,
        };
        let retry_data = data.clone();
        retry_mpeg_layer3_candidates(data.as_ref(), original_error, move |offset| {
            Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Mp3(Box::new(Mp3MusicStream::new_at(
                    retry_data.clone(),
                    offset,
                )?)),
                output_sample_rate,
            )?)))
        })
    }

    fn open_direct(data: Arc<[u8]>, output_sample_rate: u32) -> Result<Self, AudioDecodeError> {
        match detect_format(data.as_ref())? {
            AudioFormat::Wav => Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Wav(WavStream::new(data)?),
                output_sample_rate,
            )?))),
            AudioFormat::Ogg => Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Ogg(Box::new(OggMusicStream::new(data)?)),
                output_sample_rate,
            )?))),
            AudioFormat::Mp3 => Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Mp3(Box::new(Mp3MusicStream::new(data)?)),
                output_sample_rate,
            )?))),
            AudioFormat::Midi => Ok(Self::Midi(crate::fluidsynth::MidiStream::new(
                data.as_ref(),
                output_sample_rate,
            )?)),
            AudioFormat::Tracker => Ok(Self::Pcm(Box::new(PcmStream::new(
                PcmSource::Tracker(Box::new(crate::tracker::TrackerStream::new(
                    data.as_ref(),
                    output_sample_rate,
                )?)),
                output_sample_rate,
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

/// Stateful linear resampling over a pull-based source. Keeping the current
/// and next source frames is sufficient to exactly match the eager converter,
/// including its last-frame extension and ceil-rounded output length.
struct PcmStream {
    source: PcmSource,
    source_sample_rate: u32,
    output_sample_rate: u32,
    current: Option<[f32; 2]>,
    next: Option<[f32; 2]>,
    current_index: u128,
    output_index: u128,
}

impl PcmStream {
    fn new(mut source: PcmSource, output_sample_rate: u32) -> Result<Self, AudioDecodeError> {
        let source_sample_rate = source.sample_rate();
        if source_sample_rate == 0 {
            return Err(AudioDecodeError::InvalidData(
                "music source sample rate is zero",
            ));
        }
        let current = source.next_frame()?;
        let next = if current.is_some() {
            source.next_frame()?
        } else {
            None
        };
        Ok(Self {
            source,
            source_sample_rate,
            output_sample_rate,
            current,
            next,
            current_index: 0,
            output_index: 0,
        })
    }

    fn read_frames(&mut self, output: &mut [[f32; 2]]) -> Result<usize, AudioDecodeError> {
        let mut written = 0;
        while written < output.len() {
            let scaled_position = self
                .output_index
                .checked_mul(u128::from(self.source_sample_rate))
                .ok_or(AudioDecodeError::DecoderError(
                    "music resampler position overflow",
                ))?;
            let desired_index = scaled_position / u128::from(self.output_sample_rate);

            while self.current_index < desired_index {
                self.current = self.next.take();
                self.current_index += 1;
                let Some(_) = self.current else {
                    return Ok(written);
                };
                self.next = self.source.next_frame()?;
            }

            let Some(current) = self.current else {
                break;
            };
            let next = self.next.unwrap_or(current);
            let remainder = scaled_position % u128::from(self.output_sample_rate);
            let fraction = remainder as f64 / f64::from(self.output_sample_rate);
            output[written] = [
                current[0] + (next[0] - current[0]) * fraction as f32,
                current[1] + (next[1] - current[1]) * fraction as f32,
            ];
            self.output_index =
                self.output_index
                    .checked_add(1)
                    .ok_or(AudioDecodeError::DecoderError(
                        "music resampler output position overflow",
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
        self.current_index = 0;
        self.output_index = 0;
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

enum PcmSource {
    Wav(WavStream),
    Ogg(Box<OggMusicStream>),
    Mp3(Box<Mp3MusicStream>),
    Tracker(Box<crate::tracker::TrackerStream>),
}

impl PcmSource {
    fn sample_rate(&self) -> u32 {
        match self {
            Self::Wav(stream) => stream.sample_rate(),
            Self::Ogg(stream) => stream.sample_rate,
            Self::Mp3(stream) => stream.sample_rate,
            Self::Tracker(stream) => stream.sample_rate(),
        }
    }

    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        match self {
            Self::Wav(stream) => stream.next_frame(),
            Self::Ogg(stream) => stream.next_frame(),
            Self::Mp3(stream) => stream.next_frame(),
            Self::Tracker(stream) => stream.next_frame(),
        }
    }

    fn restart(&mut self) -> Result<bool, AudioDecodeError> {
        match self {
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
            Self::Tracker(stream) => stream.buffered_frames(),
        }
    }

    #[cfg(test)]
    fn peak_buffered_frames(&self) -> usize {
        match self {
            Self::Wav(stream) => stream.peak_buffered_frames(),
            Self::Ogg(stream) => stream.peak_packet_frames,
            Self::Mp3(stream) => stream.peak_packet_frames,
            Self::Tracker(stream) => stream.peak_buffered_frames(),
        }
    }
}

struct OggMusicStream {
    reader: OggStreamReader<Cursor<Arc<[u8]>>>,
    sample_rate: u32,
    channels: usize,
    packet: Vec<i16>,
    packet_offset: usize,
    #[cfg(test)]
    peak_packet_frames: usize,
}

impl OggMusicStream {
    fn new(data: Arc<[u8]>) -> Result<Self, AudioDecodeError> {
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
    sample_rate: u32,
    channels: usize,
    packet: Vec<i16>,
    packet_offset: usize,
    #[cfg(test)]
    peak_packet_frames: usize,
}

impl Mp3MusicStream {
    fn new(data: Arc<[u8]>) -> Result<Self, AudioDecodeError> {
        Self::new_at(data, 0)
    }

    fn new_at(data: Arc<[u8]>, offset: usize) -> Result<Self, AudioDecodeError> {
        let mut cursor = Cursor::new(data);
        cursor.set_position(offset as u64);
        let mut stream = Self {
            decoder: MpegAudioDecoder::open(Box::new(cursor))?,
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
        if sample_rate == 0 {
            return Err(AudioDecodeError::InvalidData("MP3 sample rate is zero"));
        }
        if self.sample_rate != 0 && self.sample_rate != sample_rate {
            return Err(AudioDecodeError::InvalidData(
                "MP3 sample rate changes within stream",
            ));
        }
        if channels == 0 {
            return Err(AudioDecodeError::InvalidData("MP3 channel count is zero"));
        }
        self.sample_rate = sample_rate;
        self.channels = channels;
        self.packet_offset = 0;
        #[cfg(test)]
        {
            self.peak_packet_frames = self
                .peak_packet_frames
                .max(frame_capacity(self.packet.capacity(), self.channels));
        }
        Ok(true)
    }

    fn next_frame(&mut self) -> Result<Option<[f32; 2]>, AudioDecodeError> {
        loop {
            if self.packet.len().saturating_sub(self.packet_offset) >= self.channels {
                let start = self.packet_offset;
                self.packet_offset += self.channels;
                return Ok(Some(stereo_i16_frame(
                    &self.packet[start..start + self.channels],
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

fn stereo_i16_frame(samples: &[i16]) -> [f32; 2] {
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
    let source: Arc<[u8]> = Arc::from(data);
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

        let source: Arc<[u8]> = Arc::from(data.into_boxed_slice());
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

        let source: Arc<[u8]> = Arc::from(data.into_boxed_slice());
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
            MusicStream::open(Arc::from(false_sync.as_slice()), 8_000),
            Err(AudioDecodeError::UnsupportedFormat)
        ));

        let mut malformed_wav = b"RIFF\0\0\0\0WAVE".to_vec();
        malformed_wav.extend_from_slice(false_sync);
        assert!(matches!(
            decode_audio(&malformed_wav),
            Err(AudioDecodeError::InvalidData("invalid WAV header"))
        ));
        assert!(matches!(
            MusicStream::open(Arc::from(malformed_wav.into_boxed_slice()), 8_000),
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
        assert!(MusicStream::open(Arc::from(id3.into_boxed_slice()), 8_000).is_ok());

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
        let source: Arc<[u8]> = Arc::from(mono_pcm16_wav(2, &[0, i16::MAX]).into_boxed_slice());
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
    fn compressed_wav_music_streams_in_bounded_blocks_and_resamples() {
        let source: Arc<[u8]> = Arc::from(mono_ima_adpcm_wav(2, 0, [0x11; 4]).into_boxed_slice());
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
