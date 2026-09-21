//! The device callback publishes native-rate PCM and its playback timestamp.
//! Capture processing reads by capture time, independently of DSP scheduling.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

const SAMPLES: usize = 262_144;
use crate::voice_resampling::{coefficients, PHASES, TAPS};

#[derive(Debug)]
pub(crate) struct OutputReference {
    samples: Box<[AtomicU32]>,
    written: AtomicU64,
    sample_rate: u32,
    origin: Instant,
    timing_sequence: AtomicU64,
    anchor_position: AtomicU64,
    anchor_nanos: AtomicU64,
    coefficients: Box<[[f32; TAPS]]>,
    valid_from: AtomicU64,
    epoch: AtomicU64,
}

impl OutputReference {
    pub(crate) fn new(sample_rate: u32) -> Arc<Self> {
        Arc::new(Self {
            samples: (0..SAMPLES).map(|_| AtomicU32::new(0)).collect(),
            written: AtomicU64::new(0),
            sample_rate,
            origin: Instant::now(),
            timing_sequence: AtomicU64::new(0),
            anchor_position: AtomicU64::new(0),
            anchor_nanos: AtomicU64::new(0),
            coefficients: coefficients(sample_rate, crate::VOICE_SAMPLE_RATE),
            valid_from: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
        })
    }

    pub(crate) fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// Exactly one output callback owns a ring. Retired callbacks keep their
    /// own ring, and cannot publish samples into a replacement device's ring.
    pub(crate) fn push(&self, sample: f32) {
        let position = self.written.load(Ordering::Relaxed);
        self.samples[position as usize % SAMPLES].store(sample.to_bits(), Ordering::Relaxed);
        self.written
            .store(position.wrapping_add(1), Ordering::Release);
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    pub(crate) fn publish_timing(&self, first_position: u64, playback_at: Instant) {
        let nanos = playback_at
            .saturating_duration_since(self.origin)
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let previous = self.timing_sequence.fetch_add(1, Ordering::AcqRel);
        if previous != 0 {
            let frames =
                first_position.saturating_sub(self.anchor_position.load(Ordering::Relaxed));
            let predicted = u128::from(self.anchor_nanos.load(Ordering::Relaxed))
                + u128::from(frames) * 1_000_000_000 / u128::from(self.sample_rate);
            if predicted.abs_diff(u128::from(nanos)) > 20_000_000 {
                // A sleep/resume or a lost hardware period is a discontinuity,
                // not a new interpretation of all the PCM preceding it.
                self.valid_from.store(first_position, Ordering::Relaxed);
                self.epoch.fetch_add(1, Ordering::Release);
            }
        }
        self.anchor_position
            .store(first_position, Ordering::Relaxed);
        self.anchor_nanos.store(nanos, Ordering::Relaxed);
        self.timing_sequence.fetch_add(1, Ordering::Release);
    }
}

impl OutputReference {
    pub(crate) fn read_at(
        &self,
        far: &mut [f32; crate::VOICE_FRAME_SAMPLES],
        captured_at: Instant,
    ) {
        far.fill(0.0);
        let sequence = self.timing_sequence.load(Ordering::Acquire);
        if sequence == 0 || sequence & 1 != 0 {
            return;
        }
        let anchor = self.anchor_position.load(Ordering::Relaxed);
        let anchor_nanos = self.anchor_nanos.load(Ordering::Relaxed);
        let written = self.written();
        let oldest = written
            .saturating_sub(SAMPLES as u64)
            .max(self.valid_from.load(Ordering::Relaxed));
        if self.timing_sequence.load(Ordering::Acquire) != sequence {
            return;
        }
        let capture_nanos = if captured_at >= self.origin {
            captured_at.duration_since(self.origin).as_nanos() as f64
        } else {
            -(self.origin.duration_since(captured_at).as_nanos() as f64)
        };
        let first = anchor as f64
            + (capture_nanos - anchor_nanos as f64) * f64::from(self.sample_rate) / 1_000_000_000.0;
        let source_per_output = f64::from(self.sample_rate) / f64::from(crate::VOICE_SAMPLE_RATE);
        for (offset, sample) in far.iter_mut().enumerate() {
            let position = first + offset as f64 * source_per_output;
            let index = position.floor() as i64;
            let fraction = (position - position.floor()) as f32;
            if self.sample_rate <= crate::VOICE_SAMPLE_RATE && fraction < 0.00001 {
                *sample = self.sample_at(index, oldest, written);
                continue;
            }
            let phase = ((fraction * PHASES as f32) as usize).min(PHASES - 1);
            *sample = self.coefficients[phase]
                .iter()
                .enumerate()
                .map(|(tap, coefficient)| {
                    coefficient
                        * self.sample_at(
                            index + tap as i64 - (TAPS as i64 / 2 - 1),
                            oldest,
                            written,
                        )
                })
                .sum();
        }
    }

    fn sample_at(&self, position: i64, oldest: u64, written: u64) -> f32 {
        let Ok(position) = u64::try_from(position) else {
            return 0.0;
        };
        if position >= written || position < oldest {
            return 0.0;
        }
        f32::from_bits(self.samples[position as usize % SAMPLES].load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timed_reference_preserves_high_speech_frequencies_across_device_rates() {
        for rate in [44_100, 48_000, 96_000, 192_000] {
            let reference = OutputReference::new(rate);
            let start = Instant::now();
            for index in 0..rate / 10 {
                reference.push(
                    (std::f64::consts::TAU * 15_000.0 * f64::from(index) / f64::from(rate)).sin()
                        as f32
                        * 0.2,
                );
            }
            reference.publish_timing(0, start);
            let mut far = [0.0_f32; crate::VOICE_FRAME_SAMPLES];
            reference.read_at(&mut far, start + Duration::from_millis(40));
            let rms =
                (far.iter().map(|sample| sample * sample).sum::<f32>() / far.len() as f32).sqrt();
            assert!(
                (rms - 0.2 / 2.0_f32.sqrt()).abs() < 0.007,
                "{rate} Hz output lost speech bandwidth: {rms}"
            );
        }
    }

    #[test]
    fn a_device_clock_gap_cannot_turn_old_output_into_a_live_echo_reference() {
        let reference = OutputReference::new(48_000);
        let start = Instant::now();
        for _ in 0..24_000 {
            reference.push(0.3);
        }
        reference.publish_timing(0, start);
        for _ in 0..960 {
            reference.push(0.1);
        }
        reference.publish_timing(24_000, start + Duration::from_millis(900));
        let mut far = [1.0; crate::VOICE_FRAME_SAMPLES];
        reference.read_at(&mut far, start + Duration::from_millis(850));
        assert!(far.iter().all(|sample| *sample == 0.0));
        reference.read_at(&mut far, start + Duration::from_millis(900));
        assert!(far[128..832]
            .iter()
            .all(|sample| (*sample - 0.1).abs() < 0.0001));
    }

    #[test]
    fn native_output_reference_does_not_alias_ultrasound_into_the_echo_band() {
        for rate in [96_000, 192_000] {
            let reference = OutputReference::new(rate);
            let start = Instant::now();
            for index in 0..rate / 10 {
                reference.push(
                    (std::f32::consts::TAU * 30_000.0 * index as f32 / rate as f32).sin() * 0.2,
                );
            }
            reference.publish_timing(0, start);
            let mut far = [0.0_f32; crate::VOICE_FRAME_SAMPLES];
            reference.read_at(&mut far, start + Duration::from_millis(40));
            let rms =
                (far.iter().map(|sample| sample * sample).sum::<f32>() / far.len() as f32).sqrt();
            assert!(
                rms < 0.005,
                "{rate} Hz output aliased into the echo band: {rms}"
            );
        }
    }
}
