//! Explicit synthetic CPU qualification: no microphone or output device opens.

use std::time::{Duration, Instant};

use crate::voice::StreamingVoiceResampler;
use crate::voice_echo::VoiceEchoReference;
use crate::voice_output_reference::OutputReference;
use crate::voice_processing::VoiceProcessing;
use crate::{
    VoiceDecoder, VoiceEncoder, VoiceProcessingConfig, VoiceProcessingSwitches, VOICE_FRAME_SAMPLES,
};

#[test]
#[ignore = "manual timing probe; run on the target hardware"]
fn voice_full_pipeline_cpu_budget() {
    let reference = VoiceEchoReference::for_output();
    let output = OutputReference::new(96_000);
    reference.set_output(Some(output.clone()));
    let mut dsp = VoiceProcessing::new(
        VoiceProcessingSwitches::new(VoiceProcessingConfig::default()),
        Some(reference),
    );
    let mut capture = StreamingVoiceResampler::new(96_000);
    let mut device = StreamingVoiceResampler::with_output_rate(44_100, 96_000);
    let mut encoder = VoiceEncoder::new().unwrap();
    let mut decoder = VoiceDecoder::new().unwrap();
    let mut timings = Vec::with_capacity(1500);
    let mut packet_bytes = 0;
    let mut rng = 0x12345678_u32;
    let mut delayed = vec![0.0_f32; 28_800];
    let mut energy = 0.0;
    let origin = Instant::now();
    for frame_index in 0..1600 {
        let mut frame = [0.0; VOICE_FRAME_SAMPLES];
        let mut offset = 0;
        let captured_at = origin + Duration::from_millis(frame_index * 20);
        let first = output.written();
        let start = Instant::now();
        for i in 0..1920 {
            let index = frame_index as usize * 1920 + i;
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let far = (rng as i32 as f32 / i32::MAX as f32) * 0.2;
            let delay_index = index % delayed.len();
            let echo = delayed[delay_index] * 0.6;
            delayed[delay_index] = far;
            output.push(far);
            let phase = index as f64 / 96_000.0 * std::f64::consts::TAU;
            let near = if frame_index % 100 < 70 {
                (0.15 * (phase * 200.0).sin() + 0.04 * (phase * 2300.0).sin()) as f32
            } else {
                0.0
            };
            capture.push_sample(near + echo, |sample| {
                frame[offset] = sample;
                offset += 1;
            });
        }
        assert_eq!(offset, VOICE_FRAME_SAMPLES);
        output.publish_timing(first, captured_at);
        std::hint::black_box(dsp.process_at(&mut frame, captured_at));
        let pcm = frame.map(crate::voice::voice_f32_to_i16);
        let packet = encoder.encode(&pcm).unwrap();
        let decoded = decoder.decode(&packet, false).unwrap();
        energy += decoded.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>();
        // Also charge one channel's production native-output conversion. This
        // is a CPU probe, not a full game/mixer or acoustic-quality benchmark.
        for sample in decoded.iter().take(882) {
            device.push_sample(f32::from(*sample) / 32_768.0, |value| {
                std::hint::black_box(value);
            });
        }
        if frame_index >= 100 {
            timings.push(start.elapsed().as_secs_f64() * 1000.0);
            packet_bytes += packet.len();
        }
    }
    timings.sort_by(f64::total_cmp);
    let mean = timings.iter().sum::<f64>() / timings.len() as f64;
    eprintln!("VOICE_CPU frames={} mean_ms={mean:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3} mean_packet_bytes={:.1} frame_budget_ms=20",
        timings.len(), timings[timings.len()*95/100], timings[timings.len()*99/100],
        timings[timings.len()-1], packet_bytes as f64 / timings.len() as f64);
    assert!(energy > 0.0);
    assert!(
        timings[timings.len() * 99 / 100] < 20.0,
        "processing misses its frame budget"
    );
}
