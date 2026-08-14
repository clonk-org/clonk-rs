use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "cpal")]
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
#[cfg(any(feature = "cpal", test))]
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;

use thiserror::Error;

/// Voice chat uses independently decodable 20 ms mono frames at 16 kHz.
pub const VOICE_SAMPLE_RATE: u32 = 16_000;
pub const VOICE_FRAME_SAMPLES: usize = 320;

const IMA_HEADER_BYTES: usize = 4;
const IMA_CODE_BYTES: usize = (VOICE_FRAME_SAMPLES - 1).div_ceil(2);

/// Two-byte predictor, one-byte IMA step index, one reserved byte, and 319
/// four-bit deltas. Every frame carries its own predictor/index state.
pub const VOICE_ENCODED_FRAME_BYTES: usize = IMA_HEADER_BYTES + IMA_CODE_BYTES;
pub type EncodedVoiceFrame = [u8; VOICE_ENCODED_FRAME_BYTES];

/// One captured frame together with how loud it was. The level is measured on
/// the PCM the encoder consumed, so a voice-activation gate never has to decode
/// a frame back just to decide whether to transmit it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceInputFrame {
    pub payload: EncodedVoiceFrame,
    /// See [`voice_activation_level`].
    pub level: f32,
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
    #[error("failed to query the microphone input format: {0}")]
    InputConfig(String),
    #[error("unsupported microphone input format: {sample_rate} Hz, {channels} channels")]
    UnsupportedInputConfig { sample_rate: u32, channels: u16 },
    #[error("failed to open the microphone input stream: {0}")]
    Stream(String),
}

/// Explicitly opened microphone capture. Merely constructing [`AudioSystem`](crate::AudioSystem)
/// never opens an input device or requests microphone permission.
pub struct VoiceCapture {
    #[cfg(feature = "cpal")]
    _stream: cpal::Stream,
    frames: Receiver<VoiceInputFrame>,
    dropped_frames: Arc<AtomicU64>,
}

impl VoiceCapture {
    /// Opens and starts the default microphone input stream. This is the only
    /// production entry point that touches a capture device.
    pub fn open() -> Result<Self, VoiceCaptureError> {
        #[cfg(feature = "cpal")]
        {
            Self::open_cpal()
        }
        #[cfg(not(feature = "cpal"))]
        {
            Err(VoiceCaptureError::Unavailable)
        }
    }

    /// Drains every complete frame currently available without waiting.
    pub fn drain_frames(&self) -> Vec<VoiceInputFrame> {
        self.frames.try_iter().collect()
    }

    /// Frames discarded because the bounded app queue was full.
    pub fn dropped_frames(&self) -> u64 {
        self.dropped_frames.load(Ordering::Relaxed)
    }

    #[cfg(feature = "cpal")]
    fn open_cpal() -> Result<Self, VoiceCaptureError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(VoiceCaptureError::NoInputDevice)?;
        let supported = device
            .default_input_config()
            .map_err(|error| VoiceCaptureError::InputConfig(error.to_string()))?;
        validate_capture_config(supported.sample_rate(), supported.channels())?;

        let (sender, frames) = mpsc::sync_channel(VOICE_CAPTURE_QUEUE_FRAMES);
        let dropped_frames = Arc::new(AtomicU64::new(0));
        let stream_config = supported.config();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::I8 => build_voice_input_stream::<i8>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::I16 => build_voice_input_stream::<i16>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::I24 => build_voice_input_stream::<cpal::I24>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::I32 => build_voice_input_stream::<i32>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::I64 => build_voice_input_stream::<i64>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::U8 => build_voice_input_stream::<u8>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::U16 => build_voice_input_stream::<u16>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::U24 => build_voice_input_stream::<cpal::U24>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::U32 => build_voice_input_stream::<u32>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::U64 => build_voice_input_stream::<u64>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::F32 => build_voice_input_stream::<f32>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            cpal::SampleFormat::F64 => build_voice_input_stream::<f64>(
                &device,
                stream_config,
                sender,
                dropped_frames.clone(),
            )?,
            _ => {
                return Err(VoiceCaptureError::Stream(
                    "unsupported non-PCM microphone sample format".to_string(),
                ));
            }
        };
        stream
            .play()
            .map_err(|error| VoiceCaptureError::Stream(error.to_string()))?;
        Ok(Self {
            _stream: stream,
            frames,
            dropped_frames,
        })
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
fn build_voice_input_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sender: SyncSender<VoiceInputFrame>,
    dropped_frames: Arc<AtomicU64>,
) -> Result<cpal::Stream, VoiceCaptureError>
where
    T: cpal::SizedSample + VoiceInputSample + Send + 'static,
{
    use cpal::traits::DeviceTrait;

    let mut processor =
        VoiceCaptureProcessor::new(config.sample_rate, config.channels, sender, dropped_frames)?;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| processor.process_interleaved(data),
            move |error| tracing::error!(%error, "cpal microphone input stream error"),
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
    samples: [i16; VOICE_FRAME_SAMPLES],
    sample_count: usize,
    sender: SyncSender<VoiceInputFrame>,
    dropped_frames: Arc<AtomicU64>,
}

#[cfg(any(feature = "cpal", test))]
impl VoiceCaptureProcessor {
    fn new(
        sample_rate: u32,
        channels: u16,
        sender: SyncSender<VoiceInputFrame>,
        dropped_frames: Arc<AtomicU64>,
    ) -> Result<Self, VoiceCaptureError> {
        validate_capture_config(sample_rate, channels)?;
        Ok(Self {
            channels: usize::from(channels),
            resampler: StreamingVoiceResampler::new(sample_rate),
            samples: [0; VOICE_FRAME_SAMPLES],
            sample_count: 0,
            sender,
            dropped_frames,
        })
    }

    fn process_interleaved<T: VoiceInputSample>(&mut self, input: &[T]) {
        for frame in input.chunks_exact(self.channels) {
            let mono = frame
                .iter()
                .map(|sample| sample.to_voice_f32())
                .sum::<f32>()
                / self.channels as f32;
            let Self {
                resampler,
                samples,
                sample_count,
                sender,
                dropped_frames,
                ..
            } = self;
            resampler.push_sample(mono, |sample| {
                samples[*sample_count] = voice_f32_to_i16(sample);
                *sample_count += 1;
                if *sample_count == VOICE_FRAME_SAMPLES {
                    let captured = VoiceInputFrame {
                        payload: encode_voice_frame(samples),
                        level: voice_activation_level(samples),
                    };
                    if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
                        sender.try_send(captured)
                    {
                        dropped_frames.fetch_add(1, Ordering::Relaxed);
                    }
                    *sample_count = 0;
                }
            });
        }
    }
}

#[cfg(any(feature = "cpal", test))]
struct StreamingVoiceResampler {
    source_per_output: f64,
    previous: Option<f32>,
    current_source_index: u64,
    next_output_position: f64,
}

#[cfg(any(feature = "cpal", test))]
impl StreamingVoiceResampler {
    fn new(source_rate: u32) -> Self {
        Self {
            source_per_output: f64::from(source_rate) / f64::from(VOICE_SAMPLE_RATE),
            previous: None,
            current_source_index: 0,
            next_output_position: 0.0,
        }
    }

    fn push_sample(&mut self, sample: f32, mut emit: impl FnMut(f32)) {
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

#[cfg(any(feature = "cpal", test))]
fn voice_f32_to_i16(sample: f32) -> i16 {
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
    let rms = mean_square.sqrt() / 32_768.0;
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
    let mut step_index = 0_u8;
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Arc};

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
        let mut processor = VoiceCaptureProcessor::new(48_000, 2, sender, dropped.clone())
            .expect("48 kHz stereo capture should be supported");
        let stereo = [1_000.0 / 32_768.0, 3_000.0 / 32_768.0].repeat(960);

        processor.process_interleaved(&stereo[..734]);
        assert!(receiver.try_recv().is_err());
        processor.process_interleaved(&stereo[734..]);

        let frame = receiver.try_recv().expect("one 20 ms frame");
        let decoded =
            decode_voice_frame(&frame.payload).expect("captured frame should be canonical");
        assert!(decoded.iter().all(|sample| sample.abs_diff(2_000) <= 1));
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn capture_processor_uses_bounded_try_send_without_blocking() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut processor = VoiceCaptureProcessor::new(16_000, 1, sender, dropped.clone())
            .expect("16 kHz mono capture should be supported");

        processor.process_interleaved(&vec![0.25_f32; VOICE_FRAME_SAMPLES * 2]);

        assert_eq!(receiver.try_iter().count(), 1);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn captured_frames_carry_the_input_level_the_gate_needs() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let dropped = Arc::new(AtomicU64::new(0));
        let mut processor = VoiceCaptureProcessor::new(16_000, 1, sender, dropped)
            .expect("16 kHz mono capture should be supported");

        processor.process_interleaved(&[0.0_f32; VOICE_FRAME_SAMPLES]);
        processor.process_interleaved(&[0.25_f32; VOICE_FRAME_SAMPLES]);

        let silent = receiver
            .try_recv()
            .expect("a silent frame is still captured");
        assert_eq!(silent.level, 0.0);
        let loud = receiver.try_recv().expect("a loud frame");
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
            VoiceCaptureProcessor::new(sample_rate, channels, sender, Arc::new(AtomicU64::new(0)))
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
}
