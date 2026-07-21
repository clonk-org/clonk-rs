use std::io::Cursor;

use hound::{SampleFormat, WavReader};
use lewton::inside_ogg::OggStreamReader;
use minimp3::Decoder as Mp3Decoder;
use thiserror::Error;

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
    let format = detect_format(data)?;
    match format {
        AudioFormat::Wav => decode_wav(data),
        AudioFormat::Ogg => decode_ogg(data),
        AudioFormat::Mp3 => decode_mp3(data),
        AudioFormat::Midi => crate::fluidsynth::decode_midi(data, output_sample_rate),
        AudioFormat::Tracker => crate::tracker::decode_tracker(data, output_sample_rate),
    }
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
    let cursor = Cursor::new(data);
    let mut reader =
        WavReader::new(cursor).map_err(|_| AudioDecodeError::InvalidData("invalid WAV header"))?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    if sample_rate == 0 {
        return Err(AudioDecodeError::InvalidData("WAV sample rate is zero"));
    }
    if spec.channels == 0 {
        return Err(AudioDecodeError::InvalidData("WAV channel count is zero"));
    }
    let channel_count = spec.channels as usize;

    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 8) => {
            // 8-bit PCM WAV is unsigned (0-255), hound converts to i8 (-128 to 127)
            // Normalize to [-1.0, 1.0): divide by 128.0
            reader
                .samples::<i8>()
                .map(|sample| sample.map(|value| value as f32 / 128.0))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| AudioDecodeError::InvalidData("invalid WAV 8-bit PCM data"))?
        }
        (SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|sample| sample.map(|value| value as f32 / i16::MAX as f32))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AudioDecodeError::InvalidData("invalid WAV PCM data"))?,
        (SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .map(|sample| sample.map(|value| value.clamp(-1.0, 1.0)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AudioDecodeError::InvalidData("invalid WAV float data"))?,
        _ => {
            return Err(AudioDecodeError::InvalidData(
                "unsupported WAV bit depth or format",
            ));
        }
    };

    let frames = convert_interleaved_to_stereo(&samples, channel_count)?;
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
    let cursor = Cursor::new(data);
    let mut decoder = Mp3Decoder::new(cursor);
    let mut frames = Vec::new();
    let mut sample_rate: Option<u32> = None;
    while let Ok(frame) = decoder.next_frame() {
        sample_rate.get_or_insert(frame.sample_rate as u32);
        let channel_count = frame.channels;
        let packet_frames = convert_interleaved_i16_to_stereo(&frame.data, channel_count)?;
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
}
