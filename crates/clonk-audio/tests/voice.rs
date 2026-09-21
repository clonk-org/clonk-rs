#[cfg(not(feature = "cpal"))]
use clonk_audio::{voice_input_devices, VoiceCapture, VoiceCaptureError};
use clonk_audio::{
    VoiceCaptureOptions, VoiceInputDevice, VoiceInputDeviceId, VoiceProcessingConfig,
    VoiceProcessingSwitches, MAX_VOICE_ENCODED_BYTES, VOICE_FRAME_SAMPLES, VOICE_SAMPLE_RATE,
};

#[test]
fn input_device_ids_round_trip_without_using_display_names() {
    let first_id = "wasapi:first-endpoint"
        .parse::<VoiceInputDeviceId>()
        .expect("a persisted CPAL device ID");
    let second_id = "wasapi:second-endpoint"
        .parse::<VoiceInputDeviceId>()
        .expect("a second persisted CPAL device ID");
    let first = VoiceInputDevice {
        id: first_id.clone(),
        name: "USB Microphone".to_string(),
    };
    let second = VoiceInputDevice {
        id: second_id,
        name: "USB Microphone".to_string(),
    };

    assert_eq!(first.id.to_string(), "wasapi:first-endpoint");
    assert_eq!(first.id.to_string().parse(), Ok(first_id));
    assert_eq!(first.name, second.name);
    assert_ne!(first.id, second.id);

    let opaque = r#"coreaudio:Built-in \"Mic\":input\\one"#;
    let opaque_id = opaque
        .parse::<VoiceInputDeviceId>()
        .expect("an opaque CPAL device ID");
    assert_eq!(opaque_id.as_str(), opaque);
    assert_eq!(opaque_id.to_string(), opaque);

    let malformed = "persisted-id-without-a-host-prefix";
    let malformed_id = malformed
        .parse::<VoiceInputDeviceId>()
        .expect("a nonempty persisted ID remains an exact selection");
    assert_eq!(malformed_id.as_str(), malformed);
    assert!("".parse::<VoiceInputDeviceId>().is_err());
}

#[test]
fn capture_options_use_the_system_default_until_an_exact_device_is_selected() {
    let mut options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
        VoiceProcessingConfig::default(),
    ));
    assert_eq!(options.input_device, None);

    options.input_device = Some(
        "coreaudio:chosen-device"
            .parse()
            .expect("a persisted CPAL device ID"),
    );
    assert_eq!(
        options.input_device.as_ref().map(ToString::to_string),
        Some("coreaudio:chosen-device".to_string())
    );
}

#[test]
fn voice_codec_preserves_speech_spectrum_with_low_distortion() {
    assert_eq!(VOICE_SAMPLE_RATE, 48_000);
    assert_eq!(VOICE_FRAME_SAMPLES, 960);
    let mut encoder = clonk_audio::VoiceEncoder::new().unwrap();
    let mut decoder = clonk_audio::VoiceDecoder::new().unwrap();
    let delay = encoder.lookahead_samples().unwrap();
    let mut output = Vec::new();
    for frame in 0..100 {
        let samples = std::array::from_fn(|index| {
            let time = (frame * VOICE_FRAME_SAMPLES + index) as f64 / f64::from(VOICE_SAMPLE_RATE);
            ((std::f64::consts::TAU * 440.0 * time).sin() * 12_000.0
                + (std::f64::consts::TAU * 1_320.0 * time).sin() * 4_000.0) as i16
        });
        let packet = encoder.encode(&samples).unwrap();
        assert!(packet.len() <= MAX_VOICE_ENCODED_BYTES);
        output.extend(decoder.decode(&packet, false).unwrap());
    }
    // VoIP filtering and prediction alter phase. Pin spectral gain and added
    // distortion here; the short-utterance test separately pins startup/tail.
    let settled = &output[VOICE_FRAME_SAMPLES * 10 + delay..];
    let mut reconstruction = vec![0.0; settled.len()];
    for (frequency, expected_amplitude) in [(440.0, 12_000.0), (1_320.0, 4_000.0)] {
        let (sin, cos) = settled
            .iter()
            .enumerate()
            .fold((0.0, 0.0), |(sin, cos), (i, &sample)| {
                let phase =
                    std::f64::consts::TAU * frequency * i as f64 / f64::from(VOICE_SAMPLE_RATE);
                (
                    sin + f64::from(sample) * phase.sin(),
                    cos + f64::from(sample) * phase.cos(),
                )
            });
        let sin = 2.0 * sin / settled.len() as f64;
        let cos = 2.0 * cos / settled.len() as f64;
        assert!(
            (sin.hypot(cos) / expected_amplitude - 1.0).abs() < 0.05,
            "{frequency} Hz gain changed by more than 5%"
        );
        for (i, value) in reconstruction.iter_mut().enumerate() {
            let phase = std::f64::consts::TAU * frequency * i as f64 / f64::from(VOICE_SAMPLE_RATE);
            *value += sin * phase.sin() + cos * phase.cos();
        }
    }
    let residual = settled
        .iter()
        .zip(&reconstruction)
        .map(|(&sample, reference)| (f64::from(sample) - reference).powi(2))
        .sum::<f64>();
    let projected = reconstruction
        .iter()
        .map(|sample| sample.powi(2))
        .sum::<f64>();
    let snr = 10.0 * (projected / residual).log10();
    assert!(snr > 25.0, "speech distortion SNR was {snr:.1} dB");
}

#[test]
fn voice_codec_preserves_a_short_utterance_when_its_lookahead_is_flushed() {
    let mut encoder = clonk_audio::VoiceEncoder::new().unwrap();
    let mut decoder = clonk_audio::VoiceDecoder::new().unwrap();
    let delay = encoder.lookahead_samples().unwrap();
    let samples = std::array::from_fn(|index| {
        let phase = index as f64 * 1_000.0 * std::f64::consts::TAU / f64::from(VOICE_SAMPLE_RATE);
        (phase.sin() * 20_000.0) as i16
    });
    let mut output = Vec::new();
    for frame in [samples, [0; VOICE_FRAME_SAMPLES], [0; VOICE_FRAME_SAMPLES]] {
        output.extend(
            decoder
                .decode(&encoder.encode(&frame).unwrap(), false)
                .unwrap(),
        );
    }
    let output_energy = output[delay..delay + VOICE_FRAME_SAMPLES]
        .iter()
        .map(|&sample| f64::from(sample).powi(2))
        .sum::<f64>();
    let input_energy = samples
        .iter()
        .map(|&sample| f64::from(sample).powi(2))
        .sum::<f64>();
    assert!(
        output_energy > input_energy * 0.5,
        "short utterance was lost in codec startup"
    );
}

#[test]
fn voice_codec_rejects_oversized_packets_and_unnegotiated_frame_durations() {
    let mut decoder = clonk_audio::VoiceDecoder::new().unwrap();
    assert!(decoder
        .decode(&vec![0; MAX_VOICE_ENCODED_BYTES + 1], false)
        .is_err());
    let mut encoder = opus::Encoder::new(
        VOICE_SAMPLE_RATE,
        opus::Channels::Mono,
        opus::Application::Voip,
    )
    .unwrap();
    for sample_count in [
        VOICE_FRAME_SAMPLES / 2,
        VOICE_FRAME_SAMPLES * 2,
        VOICE_FRAME_SAMPLES * 3,
    ] {
        let packet = encoder
            .encode_vec(&vec![1_000; sample_count], MAX_VOICE_ENCODED_BYTES)
            .unwrap();
        assert!(decoder.decode(&packet, false).is_err());
    }
}

#[cfg(not(feature = "cpal"))]
#[test]
fn feature_disabled_capture_api_fails_without_touching_a_device() {
    let options = VoiceCaptureOptions::new(VoiceProcessingSwitches::new(
        VoiceProcessingConfig::default(),
    ));
    assert!(matches!(
        VoiceCapture::open(options),
        Err(VoiceCaptureError::Unavailable)
    ));
    assert!(matches!(
        voice_input_devices(),
        Err(VoiceCaptureError::Unavailable)
    ));
}
