use clonk_audio::{
    decode_voice_frame, encode_voice_frame, VoiceCodecError, VOICE_ENCODED_FRAME_BYTES,
    VOICE_FRAME_SAMPLES, VOICE_SAMPLE_RATE,
};
#[cfg(not(feature = "cpal"))]
use clonk_audio::{
    VoiceCapture, VoiceCaptureError, VoiceCaptureOptions, VoiceProcessingConfig,
    VoiceProcessingSwitches,
};

#[test]
fn voice_codec_encodes_fixed_independently_decodable_twenty_millisecond_frames() {
    assert_eq!(VOICE_SAMPLE_RATE, 16_000);
    assert_eq!(VOICE_FRAME_SAMPLES, 320);

    let first = std::array::from_fn(|index| {
        let phase = index as f32 * 440.0 * std::f32::consts::TAU / VOICE_SAMPLE_RATE as f32;
        (phase.sin() * 12_000.0) as i16
    });
    let second = [-9_000; VOICE_FRAME_SAMPLES];

    let first_encoded = encode_voice_frame(&first);
    let second_encoded = encode_voice_frame(&second);
    assert_eq!(first_encoded.len(), VOICE_ENCODED_FRAME_BYTES);
    assert_eq!(second_encoded.len(), VOICE_ENCODED_FRAME_BYTES);

    let first_decoded = decode_voice_frame(&first_encoded).expect("first frame should decode");
    let second_decoded = decode_voice_frame(&second_encoded).expect("second frame should decode");
    assert_eq!(first_decoded[0], first[0]);
    assert_eq!(second_decoded[0], second[0]);
    assert!(second_decoded.iter().all(|sample| *sample == -9_000));

    let mean_error = first
        .iter()
        .zip(first_decoded)
        .map(|(expected, actual)| i32::from(*expected).abs_diff(i32::from(actual)) as u64)
        .sum::<u64>()
        / VOICE_FRAME_SAMPLES as u64;
    assert!(mean_error < 1_500, "IMA ADPCM mean error was {mean_error}");
}

#[test]
fn voice_codec_chooses_an_initial_step_that_avoids_a_frame_start_transient() {
    let samples = std::array::from_fn(|index| {
        let phase = index as f32 * 1_000.0 * std::f32::consts::TAU / VOICE_SAMPLE_RATE as f32;
        (phase.sin() * 28_000.0) as i16
    });

    let encoded = encode_voice_frame(&samples);
    let decoded = decode_voice_frame(&encoded).expect("the encoded voice frame should decode");
    let transient_samples = 32;
    let (signal_energy, error_energy) = samples[..transient_samples]
        .iter()
        .zip(&decoded[..transient_samples])
        .fold((0.0_f64, 0.0_f64), |(signal, error), (expected, actual)| {
            let expected = f64::from(*expected);
            let difference = expected - f64::from(*actual);
            (
                signal + expected * expected,
                error + difference * difference,
            )
        });
    let transient_snr_db = 10.0 * (signal_energy / error_energy).log10();

    assert!(
        transient_snr_db >= 25.0,
        "the first two milliseconds had only {transient_snr_db:.1} dB SNR",
    );
    assert_ne!(
        encoded[2], 0,
        "a steep frame start must advertise a useful initial IMA step index",
    );
}

#[test]
fn voice_codec_sizes_its_initial_step_from_more_than_one_flat_transition() {
    let samples = std::array::from_fn(|index| {
        if index < 2 {
            return 0;
        }
        let phase = (index - 1) as f32 * 1_000.0 * std::f32::consts::TAU / VOICE_SAMPLE_RATE as f32;
        (phase.sin() * 28_000.0) as i16
    });

    let decoded = decode_voice_frame(&encode_voice_frame(&samples))
        .expect("the encoded voice frame should decode");
    let (signal_energy, error_energy) = samples[..34].iter().zip(&decoded[..34]).fold(
        (0.0_f64, 0.0_f64),
        |(signal, error), (expected, actual)| {
            let expected = f64::from(*expected);
            let difference = expected - f64::from(*actual);
            (
                signal + expected * expected,
                error + difference * difference,
            )
        },
    );
    let transient_snr_db = 10.0 * (signal_energy / error_energy).log10();

    assert!(
        transient_snr_db >= 25.0,
        "a flat first transition hid the frame's required IMA step; SNR was {transient_snr_db:.1} dB",
    );
}

#[test]
fn voice_codec_rejects_every_noncanonical_frame_shape() {
    let encoded = encode_voice_frame(&[0; VOICE_FRAME_SAMPLES]);
    assert!(matches!(
        decode_voice_frame(&encoded[..encoded.len() - 1]),
        Err(VoiceCodecError::InvalidLength { .. })
    ));

    let mut oversized = encoded.to_vec();
    oversized.push(0);
    assert!(matches!(
        decode_voice_frame(&oversized),
        Err(VoiceCodecError::InvalidLength { .. })
    ));

    let mut invalid_index = encoded;
    invalid_index[2] = 89;
    assert_eq!(
        decode_voice_frame(&invalid_index),
        Err(VoiceCodecError::InvalidStepIndex(89))
    );

    let mut invalid_reserved = encoded;
    invalid_reserved[3] = 1;
    assert_eq!(
        decode_voice_frame(&invalid_reserved),
        Err(VoiceCodecError::InvalidReservedByte)
    );

    let mut invalid_padding = encoded;
    invalid_padding[VOICE_ENCODED_FRAME_BYTES - 1] = 0x10;
    assert_eq!(
        decode_voice_frame(&invalid_padding),
        Err(VoiceCodecError::InvalidPadding)
    );
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
}
