//! Native output device ownership and callback construction.

use super::*;

#[cfg(feature = "cpal")]
pub(super) struct CpalBackend {
    _stream: cpal::Stream,
}

#[cfg(feature = "cpal")]
fn build_cpal_output_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mixer: Arc<AudioMixer>,
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::SizedSample + SampleWrite + Send + 'static,
{
    use cpal::traits::DeviceTrait;

    let output_channels = usize::from(config.channels);
    let stream_error_started = Instant::now();
    let mut stream_error_reporter = CpalStreamErrorReporter::default();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                mixer.mix_into_channels(data, output_channels);
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

        let stream = try_cpal_stream_configs(stream_configs, |stream_config| {
            let stream = match sample_format {
                cpal::SampleFormat::I8 => {
                    build_cpal_output_stream::<i8>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::I16 => {
                    build_cpal_output_stream::<i16>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::I24 => {
                    build_cpal_output_stream::<cpal::I24>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::I32 => {
                    build_cpal_output_stream::<i32>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::I64 => {
                    build_cpal_output_stream::<i64>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::U8 => {
                    build_cpal_output_stream::<u8>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::U16 => {
                    build_cpal_output_stream::<u16>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::U24 => {
                    build_cpal_output_stream::<cpal::U24>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::U32 => {
                    build_cpal_output_stream::<u32>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::U64 => {
                    build_cpal_output_stream::<u64>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::F32 => {
                    build_cpal_output_stream::<f32>(device, stream_config, mixer.clone())?
                }
                cpal::SampleFormat::F64 => {
                    build_cpal_output_stream::<f64>(device, stream_config, mixer.clone())?
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

        Ok((mixer, Self { _stream: stream }))
    }
}
