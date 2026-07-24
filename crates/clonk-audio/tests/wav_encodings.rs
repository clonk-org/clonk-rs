use clonk_audio::{decode_audio, AudioDecodeError};

const SAMPLE_RATE: u32 = 8_000;
const PCM_GUID_TAIL: [u8; 12] = [
    0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];
const MS_ADPCM_COEFFICIENTS: [(i16, i16); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];

fn append_chunk(wav: &mut Vec<u8>, name: &[u8; 4], payload: &[u8]) {
    wav.extend_from_slice(name);
    wav.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    wav.extend_from_slice(payload);
    if !payload.len().is_multiple_of(2) {
        wav.push(0);
    }
}

fn riff_wav(fmt: &[u8], fact_frames: Option<u32>, data: &[u8]) -> Vec<u8> {
    let mut wav = b"RIFF\0\0\0\0WAVE".to_vec();
    append_chunk(&mut wav, b"fmt ", fmt);
    if let Some(frames) = fact_frames {
        append_chunk(&mut wav, b"fact", &frames.to_le_bytes());
    }
    append_chunk(&mut wav, b"data", data);
    let riff_len = u32::try_from(wav.len() - 8).unwrap();
    wav[4..8].copy_from_slice(&riff_len.to_le_bytes());
    wav
}

fn pcm_fmt(bits: u16) -> Vec<u8> {
    let bytes_per_sample = bits / 8;
    let mut fmt = Vec::with_capacity(16);
    fmt.extend_from_slice(&1_u16.to_le_bytes());
    fmt.extend_from_slice(&1_u16.to_le_bytes());
    fmt.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    fmt.extend_from_slice(&(SAMPLE_RATE * u32::from(bytes_per_sample)).to_le_bytes());
    fmt.extend_from_slice(&bytes_per_sample.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    fmt
}

fn extensible_fmt(subformat: u16, bits: u16, block_align: u16, union_value: u16) -> Vec<u8> {
    let mut fmt = Vec::with_capacity(40);
    let byte_rate = if matches!(subformat, 2 | 0x11) {
        SAMPLE_RATE * u32::from(block_align) / u32::from(union_value)
    } else {
        SAMPLE_RATE * u32::from(block_align)
    };
    fmt.extend_from_slice(&0xfffe_u16.to_le_bytes());
    fmt.extend_from_slice(&1_u16.to_le_bytes());
    fmt.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    fmt.extend_from_slice(&byte_rate.to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    fmt.extend_from_slice(&22_u16.to_le_bytes());
    fmt.extend_from_slice(&union_value.to_le_bytes());
    fmt.extend_from_slice(&4_u32.to_le_bytes());
    fmt.extend_from_slice(&u32::from(subformat).to_le_bytes());
    fmt.extend_from_slice(&PCM_GUID_TAIL);
    fmt
}

fn law_fmt(format: u16, channels: u16) -> Vec<u8> {
    let mut fmt = Vec::with_capacity(18);
    fmt.extend_from_slice(&format.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    fmt.extend_from_slice(&(SAMPLE_RATE * u32::from(channels)).to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&8_u16.to_le_bytes());
    fmt.extend_from_slice(&0_u16.to_le_bytes());
    fmt
}

fn ms_adpcm_fmt(channels: u16, block_align: u16, samples_per_block: u16) -> Vec<u8> {
    let mut fmt = Vec::with_capacity(50);
    fmt.extend_from_slice(&2_u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    fmt.extend_from_slice(
        &(SAMPLE_RATE * u32::from(block_align) / u32::from(samples_per_block)).to_le_bytes(),
    );
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&4_u16.to_le_bytes());
    fmt.extend_from_slice(&32_u16.to_le_bytes());
    fmt.extend_from_slice(&samples_per_block.to_le_bytes());
    fmt.extend_from_slice(&7_u16.to_le_bytes());
    for (coefficient1, coefficient2) in MS_ADPCM_COEFFICIENTS {
        fmt.extend_from_slice(&coefficient1.to_le_bytes());
        fmt.extend_from_slice(&coefficient2.to_le_bytes());
    }
    fmt
}

fn ima_adpcm_fmt(channels: u16) -> Vec<u8> {
    let block_align = channels * 8;
    let mut fmt = Vec::with_capacity(20);
    fmt.extend_from_slice(&0x11_u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    fmt.extend_from_slice(&(SAMPLE_RATE * u32::from(block_align) / 9).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&4_u16.to_le_bytes());
    fmt.extend_from_slice(&2_u16.to_le_bytes());
    fmt.extend_from_slice(&9_u16.to_le_bytes());
    fmt
}

fn pcm24_bytes(samples: &[i32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 3);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes()[..3]);
    }
    bytes
}

fn assert_mono_samples(wav: &[u8], expected: &[i32], denominator: f32) {
    let decoded = decode_audio(wav).expect("SDL-compatible WAV decodes");
    assert_eq!(decoded.sample_rate, SAMPLE_RATE);
    assert_eq!(decoded.frames.len(), expected.len());
    for (frame, sample) in decoded.frames.iter().zip(expected) {
        let expected = *sample as f32 / denominator;
        assert!(
            (frame[0] - expected).abs() < 1.0e-6,
            "{frame:?} != {expected}"
        );
        assert!(
            (frame[1] - expected).abs() < 1.0e-6,
            "{frame:?} != {expected}"
        );
    }
}

fn assert_stereo_samples(wav: &[u8], expected: &[[i32; 2]]) {
    let decoded = decode_audio(wav).expect("SDL-compatible stereo WAV decodes");
    assert_eq!(decoded.sample_rate, SAMPLE_RATE);
    assert_eq!(decoded.frames.len(), expected.len());
    for (frame, expected) in decoded.frames.iter().zip(expected) {
        for channel in 0..2 {
            let expected = expected[channel] as f32 / i16::MAX as f32;
            assert!((frame[channel] - expected).abs() < 1.0e-6);
        }
    }
}

#[test]
fn decodes_cpp_supported_wav_encodings() {
    assert_mono_samples(
        &riff_wav(&pcm_fmt(8), None, &[0, 128, 255]),
        &[-128, 0, 127],
        128.0,
    );
    let pcm16_samples = [i16::MIN, -1, 0, i16::MAX];
    let pcm16_data: Vec<u8> = pcm16_samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    let pcm16_expected: Vec<i32> = pcm16_samples
        .iter()
        .map(|sample| i32::from(*sample))
        .collect();
    assert_mono_samples(
        &riff_wav(&pcm_fmt(16), None, &pcm16_data),
        &pcm16_expected,
        i16::MAX as f32,
    );

    let pcm24_samples = [i32::from(-1_i16) << 23, -1, 0, 0x7f_ffff];
    let pcm24_data = pcm24_bytes(&pcm24_samples);
    assert_mono_samples(
        &riff_wav(&pcm_fmt(24), None, &pcm24_data),
        &pcm24_samples,
        8_388_607.0,
    );
    assert_mono_samples(
        &riff_wav(&extensible_fmt(1, 24, 3, 24), None, &pcm24_data),
        &pcm24_samples,
        8_388_607.0,
    );
    assert_mono_samples(
        &riff_wav(&extensible_fmt(1, 24, 3, 1), None, &pcm24_data),
        &pcm24_samples,
        8_388_607.0,
    );
    let mut extended_pcm_fmt = extensible_fmt(1, 24, 3, 24);
    extended_pcm_fmt[16..18].copy_from_slice(&24_u16.to_le_bytes());
    extended_pcm_fmt.extend_from_slice(&[0xaa, 0x55]);
    assert_mono_samples(
        &riff_wav(&extended_pcm_fmt, None, &pcm24_data),
        &pcm24_samples,
        8_388_607.0,
    );

    let pcm32_samples = [i32::MIN, -1, 0, i32::MAX];
    let pcm32_data: Vec<u8> = pcm32_samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    assert_mono_samples(
        &riff_wav(&pcm_fmt(32), None, &pcm32_data),
        &pcm32_samples,
        i32::MAX as f32,
    );
    let mut pcm32_wave_format_ex = pcm_fmt(32);
    pcm32_wave_format_ex.extend_from_slice(&0_u16.to_le_bytes());
    assert_mono_samples(
        &riff_wav(&pcm32_wave_format_ex, None, &pcm32_data),
        &pcm32_samples,
        i32::MAX as f32,
    );
    assert_mono_samples(
        &riff_wav(&extensible_fmt(1, 32, 4, 32), None, &pcm32_data),
        &pcm32_samples,
        i32::MAX as f32,
    );
    let pcm24_in_32 = [i32::MIN, -256, 0, 0x7f_ffff00];
    let pcm24_in_32_data: Vec<u8> = pcm24_in_32
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    assert_mono_samples(
        &riff_wav(&extensible_fmt(1, 32, 4, 24), None, &pcm24_in_32_data),
        &pcm24_in_32,
        i32::MAX as f32,
    );

    let float_data: Vec<u8> = [-1.5_f32, 0.25, 2.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let mut float_fmt = pcm_fmt(32);
    float_fmt[..2].copy_from_slice(&3_u16.to_le_bytes());
    let decoded = decode_audio(&riff_wav(&float_fmt, None, &float_data))
        .expect("conventional float WAV decodes");
    assert_eq!(decoded.frames, [[-1.0; 2], [0.25; 2], [1.0; 2]]);
    let decoded = decode_audio(&riff_wav(&extensible_fmt(3, 32, 4, 32), None, &float_data))
        .expect("extensible float WAV decodes");
    assert_eq!(decoded.frames, [[-1.0; 2], [0.25; 2], [1.0; 2]]);

    assert_mono_samples(
        &riff_wav(&law_fmt(6, 1), Some(4), &[0xd5, 0x55, 0x80, 0x00]),
        &[8, -8, 5_504, -5_504],
        i16::MAX as f32,
    );
    let mut compact_a_law_fmt = law_fmt(6, 1);
    compact_a_law_fmt.truncate(16);
    assert_mono_samples(
        &riff_wav(&compact_a_law_fmt, None, &[0xd5, 0x55, 0x80, 0x00]),
        &[8, -8, 5_504, -5_504],
        i16::MAX as f32,
    );
    assert_mono_samples(
        &riff_wav(&law_fmt(7, 1), Some(4), &[0xff, 0x7f, 0x80, 0x00]),
        &[0, 0, 32_124, -32_124],
        i16::MAX as f32,
    );
    assert_mono_samples(
        &riff_wav(
            &extensible_fmt(7, 8, 1, 8),
            Some(4),
            &[0xff, 0x7f, 0x80, 0x00],
        ),
        &[0, 0, 32_124, -32_124],
        i16::MAX as f32,
    );

    let ms_block = [0x00, 0x10, 0x00, 0xe8, 0x03, 0x00, 0x00, 0x1f];
    assert_mono_samples(
        &riff_wav(&ms_adpcm_fmt(1, 8, 4), Some(4), &ms_block),
        &[0, 1_000, 1_016, 1_000],
        i16::MAX as f32,
    );
    let ms_stereo_block = [
        0x00, 0x00, 0x10, 0x00, 0x10, 0x00, 0xe8, 0x03, 0x18, 0xfc, 0x00, 0x00, 0x00, 0x00, 0x11,
        0x1f,
    ];
    assert_stereo_samples(
        &riff_wav(&ms_adpcm_fmt(2, 16, 4), Some(4), &ms_stereo_block),
        &[[0, 0], [1_000, -1_000], [1_016, -984], [1_032, -1_000]],
    );

    let ima_block = [0x00, 0x00, 0x00, 0x00, 0x11, 0x11, 0x11, 0x11];
    assert_mono_samples(
        &riff_wav(&ima_adpcm_fmt(1), Some(9), &ima_block),
        &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        i16::MAX as f32,
    );
    let mut derived_ima_fmt = ima_adpcm_fmt(1);
    derived_ima_fmt[18..20].copy_from_slice(&0_u16.to_le_bytes());
    assert_mono_samples(
        &riff_wav(&derived_ima_fmt, None, &ima_block),
        &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        i16::MAX as f32,
    );
    let mut compact_ima_fmt = ima_adpcm_fmt(1);
    compact_ima_fmt.truncate(16);
    assert_mono_samples(
        &riff_wav(&compact_ima_fmt, None, &ima_block),
        &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        i16::MAX as f32,
    );
    let mut clamped_ima_block = ima_block;
    clamped_ima_block[2] = u8::MAX;
    clamped_ima_block[3] = 0xa5;
    assert_mono_samples(
        &riff_wav(&ima_adpcm_fmt(1), Some(9), &clamped_ima_block),
        &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        i16::MAX as f32,
    );

    let ima_stereo_block = [
        0x00, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11,
    ];
    let expected_stereo: Vec<[i32; 2]> = (0..=8).map(|sample| [sample, 100 + sample]).collect();
    assert_stereo_samples(
        &riff_wav(&ima_adpcm_fmt(2), Some(9), &ima_stereo_block),
        &expected_stereo,
    );

    let ima_three_channel_block = [
        0x00, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0xc8, 0x00, 0x00, 0x00, 0x11, 0x11, 0x11,
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
    ];
    let expected_three_channel: Vec<[i32; 2]> =
        (0..=8).map(|sample| [40 + sample, 120 + sample]).collect();
    assert_stereo_samples(
        &riff_wav(&ima_adpcm_fmt(3), Some(9), &ima_three_channel_block),
        &expected_three_channel,
    );

    let mut two_ima_blocks = ima_block.to_vec();
    two_ima_blocks.extend_from_slice(&[0x64, 0x00, 0x00, 0x00, 0x11, 0x11, 0x11, 0x11]);
    assert_mono_samples(
        &riff_wav(&ima_adpcm_fmt(1), Some(18), &two_ima_blocks),
        &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 100, 101, 102, 103, 104, 105, 106, 107, 108,
        ],
        i16::MAX as f32,
    );
    assert_mono_samples(
        &riff_wav(&extensible_fmt(0x11, 4, 8, 9), Some(9), &ima_block),
        &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        i16::MAX as f32,
    );

    let stereo_law = riff_wav(&law_fmt(6, 2), Some(2), &[0xd5, 0x55, 0x80, 0x00]);
    let decoded = decode_audio(&stereo_law).expect("stereo A-law decodes");
    assert_eq!(decoded.sample_rate, SAMPLE_RATE);
    assert_eq!(decoded.frames.len(), 2);
    assert_eq!(decoded.frames[0], [8.0 / 32_767.0, -8.0 / 32_767.0]);
    assert_eq!(decoded.frames[1], [5_504.0 / 32_767.0, -5_504.0 / 32_767.0]);
}

#[test]
fn terminal_odd_data_chunk_does_not_require_padding() {
    // C4AudioSystemSdl.cpp:284-286 delegates legacy sound bytes to
    // Mix_LoadWAV_RW; shipped odd data chunks omit the terminal alignment pad.
    let mut wav = riff_wav(&pcm_fmt(8), None, &[128]);
    assert_eq!(wav.pop(), Some(0), "fixture ends with RIFF padding");
    let riff_len = u32::try_from(wav.len() - 8).unwrap();
    wav[4..8].copy_from_slice(&riff_len.to_le_bytes());

    assert_mono_samples(&wav, &[0], 128.0);
}

#[test]
fn terminal_unpadded_data_still_requires_the_declared_payload() {
    // C4AudioSystemSdl.cpp:284-286 accepts the shipped alignment quirk, not a
    // truncated audio payload.
    let mut wav = riff_wav(&pcm_fmt(8), None, &[0, 128, 255]);
    assert_eq!(wav.pop(), Some(0), "fixture ends with RIFF padding");
    assert_eq!(wav.pop(), Some(255), "remove one declared payload byte");
    let riff_len = u32::try_from(wav.len() - 8).unwrap();
    wav[4..8].copy_from_slice(&riff_len.to_le_bytes());

    assert!(matches!(
        decode_audio(&wav),
        Err(AudioDecodeError::InvalidData("truncated WAV chunk"))
    ));
}

#[test]
fn malformed_or_unsupported_wav_encodings_remain_typed_errors() {
    fn assert_invalid(wav: &[u8]) {
        assert!(matches!(
            decode_audio(wav),
            Err(AudioDecodeError::InvalidData(_))
        ));
    }

    let mut misaligned_law = law_fmt(6, 2);
    misaligned_law[12..14].copy_from_slice(&1_u16.to_le_bytes());
    assert_invalid(&riff_wav(&misaligned_law, None, &[0xd5, 0x55]));

    let mut bad_coefficients = ms_adpcm_fmt(1, 8, 4);
    bad_coefficients[22..24].copy_from_slice(&255_i16.to_le_bytes());
    assert_invalid(&riff_wav(
        &bad_coefficients,
        None,
        &[0, 16, 0, 0, 0, 0, 0, 0],
    ));

    let mut bad_predictor = [0x00, 0x10, 0x00, 0xe8, 0x03, 0x00, 0x00, 0x11];
    bad_predictor[0] = 7;
    assert_invalid(&riff_wav(&ms_adpcm_fmt(1, 8, 4), None, &bad_predictor));
    assert_invalid(&riff_wav(
        &ms_adpcm_fmt(1, 8, 4),
        None,
        &[0, 16, 0, 0, 0, 0, 0],
    ));

    assert_invalid(&riff_wav(&ima_adpcm_fmt(1), None, &[0, 0, 0, 0, 0x11]));
    assert_invalid(&riff_wav(
        &extensible_fmt(2, 4, 8, 4),
        None,
        &[0, 16, 0, 0, 0, 0, 0, 0],
    ));

    let mut unsupported_fmt = pcm_fmt(16);
    unsupported_fmt[..2].copy_from_slice(&0x1234_u16.to_le_bytes());
    assert_invalid(&riff_wav(&unsupported_fmt, None, &[]));

    let mut zero_rate_fmt = law_fmt(7, 1);
    zero_rate_fmt[4..8].copy_from_slice(&0_u32.to_le_bytes());
    assert_invalid(&riff_wav(&zero_rate_fmt, None, &[0xff]));

    assert_invalid(&riff_wav(&law_fmt(7, 9), None, &[0xff; 9]));
    let mut excessive_rate_fmt = law_fmt(7, 1);
    excessive_rate_fmt[4..8].copy_from_slice(&4_194_303_u32.to_le_bytes());
    excessive_rate_fmt[8..12].copy_from_slice(&4_194_303_u32.to_le_bytes());
    assert_invalid(&riff_wav(&excessive_rate_fmt, None, &[0xff]));

    let mut truncated = riff_wav(&law_fmt(7, 1), None, &[0xff, 0x7f]);
    truncated.pop();
    assert_invalid(&truncated);
}
