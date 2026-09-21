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
}

impl OutputBuffer {
    fn new(sample_rate: u32) -> Arc<Self> {
        Arc::new(Self {
            samples: crossbeam_queue::ArrayQueue::new(OUTPUT_QUEUE_FRAMES),
            active: AtomicBool::new(true),
            sample_rate,
            callback_frames: AtomicUsize::new(CLASSIC_OUTPUT_BUFFER_FRAMES as usize),
            underruns: AtomicU64::new(0),
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
    _stream: cpal::Stream,
    buffer: Arc<OutputBuffer>,
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
    let mut callback = OutputCallback { buffer };
    let stream_error_started = Instant::now();
    let mut stream_error_reporter = CpalStreamErrorReporter::default();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                callback.render(data, output_channels);
            },
            move |error| match stream_error_reporter
                .record(error.kind(), stream_error_started.elapsed())
            {
                Some(CpalStreamErrorReport::RecoveredXrun {
                    total_occurrences,
                    occurrences_since_previous_report,
                }) => tracing::warn!(
                    %error,
                    total_occurrences,
                    occurrences_since_previous_report,
                    "cpal output stream buffer underrun or overrun"
                ),
                Some(CpalStreamErrorReport::Immediate) => {
                    tracing::error!(%error, "cpal stream error");
                }
                None => {}
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
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoAudioDevice)?;
        let supported_configs = device.supported_output_configs().map_err(|err| {
            AudioError::Stream(format!("failed to enumerate output formats: {err}"))
        })?;
        let configs = cpal_output_config_candidates(supported_configs);
        if configs.is_empty() {
            return Err(AudioError::Stream(
                "no safely convertible PCM output configuration with 1 to 8 channels".to_string(),
            ));
        }

        try_cpal_output_candidates(configs, |config| {
            Self::try_config(&device, config, max_channels, resampling_mode)
        })
    }

    fn try_config(
        device: &cpal::Device,
        config: cpal::SupportedStreamConfig,
        max_channels: usize,
        resampling_mode: ResamplingMode,
    ) -> Result<(Arc<AudioMixer>, Self), AudioError> {
        use cpal::traits::StreamTrait;

        let sample_rate = config.sample_rate();
        let sample_format = config.sample_format();
        let stream_configs = cpal_output_stream_config_candidates(config);

        let mixer = Arc::new(AudioMixer::new_with_resampling(
            sample_rate,
            max_channels,
            resampling_mode,
        ));

        let buffer = OutputBuffer::new(sample_rate);
        let stream = try_cpal_stream_configs(stream_configs, |stream_config| {
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
            Ok(stream)
        })?;

        let render_mixer = mixer.clone();
        let render_buffer = buffer.clone();
        thread::Builder::new()
            .name("audio-render".into())
            .spawn(move || render_output(render_mixer, render_buffer))
            .map_err(|error| AudioError::Stream(error.to_string()))?;
        Ok((
            mixer,
            Self {
                _stream: stream,
                buffer,
            },
        ))
    }
}

impl Drop for CpalBackend {
    fn drop(&mut self) {
        self.buffer.active.store(false, Ordering::Release);
    }
}

/// Rendering may lock mixer state or decode music. Only this worker does so;
/// the hardware callback consumes pre-rendered PCM from its bounded queue.
fn render_output(mixer: Arc<AudioMixer>, buffer: Arc<OutputBuffer>) {
    let mut pcm = [0.0_f32; 256];
    while buffer.active.load(Ordering::Acquire) {
        let target = buffer
            .callback_frames
            .load(Ordering::Relaxed)
            .saturating_add(buffer.sample_rate as usize / 500)
            .min(OUTPUT_QUEUE_FRAMES - pcm.len() / 2);
        while buffer.active.load(Ordering::Acquire) && buffer.samples.len() < target {
            let frames = (target - buffer.samples.len()).min(pcm.len() / 2);
            mixer.mix_f32(&mut pcm[..frames * 2]);
            for pair in pcm[..frames * 2].chunks_exact(2) {
                let _ = buffer.samples.push([pair[0], pair[1]]);
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
