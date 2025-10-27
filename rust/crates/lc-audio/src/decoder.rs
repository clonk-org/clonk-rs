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
    #[error("io error")]
    IoError(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub frames: Vec<[f32; 2]>,
    pub sample_rate: u32,
}

pub fn decode_audio(data: &[u8]) -> Result<DecodedAudio, AudioDecodeError> {
    let format = detect_format(data)?;
    match format {
        AudioFormat::Wav => decode_wav(data),
        AudioFormat::Ogg => decode_ogg(data),
        AudioFormat::Mp3 => decode_mp3(data),
    }
}

fn detect_format(data: &[u8]) -> Result<AudioFormat, AudioDecodeError> {
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return Ok(AudioFormat::Wav);
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
        let channel_count = frame.channels as usize;
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
