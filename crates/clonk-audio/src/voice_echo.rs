//! The far-end reference an acoustic echo canceller subtracts: what the mixer
//! is about to play, published for the microphone thread to read.
//!
//! The two sides are separate `cpal` streams on separate OS callback threads
//! with independent clocks, so the handoff is a lock-free single-producer,
//! single-consumer ring of atomic samples rather than a shared lock. Neither
//! audio callback may ever wait on the other.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use crate::voice::StreamingVoiceResampler;
use crate::VOICE_FRAME_SAMPLES;

/// Just over a second of 16 kHz mono history. A power of two so the write
/// position masks into an index.
const ECHO_REFERENCE_SAMPLES: usize = 16_384;

/// How far the reader may trail the writer before it gives up on the samples in
/// between. A steady lag is harmless — the canceller simply models a shorter
/// echo path — but it eats into the delay the adaptive filter can still cover,
/// and the output callback delivers its samples in device-buffer-sized bursts,
/// so the tolerance has to be several frames wide to avoid resyncing on
/// ordinary jitter.
const MAX_REFERENCE_LAG_SAMPLES: u64 = 8 * VOICE_FRAME_SAMPLES as u64;
/// Where a resync leaves the reader: far enough back to absorb the next burst,
/// close enough that most of the real echo delay is still ahead of it.
const RESYNC_REFERENCE_LAG_SAMPLES: u64 = 2 * VOICE_FRAME_SAMPLES as u64;

#[derive(Debug)]
struct EchoRing {
    /// `f32::to_bits` of each published sample. Individual loads and stores are
    /// atomic, so a reader that the writer laps sees a mix of old and new
    /// samples — one degraded frame of cancellation, never a torn value.
    samples: Box<[AtomicU32]>,
    written: AtomicU64,
}

/// A handle on the mixer's published far-end signal, resampled to
/// [`VOICE_SAMPLE_RATE`](crate::VOICE_SAMPLE_RATE) mono. Cloning shares the
/// same ring.
#[derive(Clone, Debug)]
pub struct VoiceEchoReference {
    ring: Arc<EchoRing>,
}

/// The mixer's end of the reference: downmixes and resamples the output it is
/// about to hand the device.
#[derive(Debug)]
pub(crate) struct VoiceEchoTap {
    reference: VoiceEchoReference,
    resampler: StreamingVoiceResampler,
}

/// The capture side's cursor into a [`VoiceEchoReference`].
#[derive(Debug)]
pub(crate) struct EchoReferenceReader {
    reference: VoiceEchoReference,
    position: u64,
}

/// How much of the echo path the adaptive filter can model: 2048 taps is 128 ms
/// at [`VOICE_SAMPLE_RATE`](crate::VOICE_SAMPLE_RATE), which covers an ordinary
/// output buffer, the device's own latency, the trip across the room and the
/// capture buffer. A longer echo than that — a Bluetooth headset, say — is left
/// to the residual suppressor below, which does not care where the echo came
/// from. The filter costs about one percent of a core per 1024 taps.
const ECHO_TAIL_SAMPLES: usize = 2_048;
/// Normalized step size. Large enough to converge during the opening fraction
/// of an utterance, while remaining within the NLMS stability bound.
const ECHO_ADAPTATION_RATE: f32 = 1.0;
/// Geigel double-talk detector: a room cannot return more sound than the
/// speakers put into it, so a microphone louder than the far end that could
/// have caused it is hearing someone speak, and the filter must not adapt to
/// them. Deliberately permissive — quieter double talk only slows the filter
/// down, and the residual suppressor below is what protects the near end.
const ECHO_PATH_MAX_GAIN: f32 = 1.0;
/// How much of the filter's own output is assumed to survive as residual echo.
/// The adaptive filter never cancels a nonlinear speaker and microphone path,
/// device-clock drift, or room-path changes perfectly.
const ECHO_RESIDUAL_LEAKAGE: f32 = 0.25;
/// Conservative residual energy assumed before the adaptive filter has learned
/// enough of the room to produce its own estimate. Push-to-talk captures start
/// cold, so relying on the learned estimate alone leaves the first short
/// utterance almost completely uncancelled.
const ECHO_COLD_START_RESIDUAL_RATIO: f32 = 0.25;
/// Keep the conservative residual estimate only while the filter learns its
/// first room path. The normal learned estimate takes over after this many
/// frames that were safe to adapt, so quiet double talk is not held down for
/// the rest of the capture.
const ECHO_COLD_START_ADAPTATION_FRAMES: u32 = 12;
/// Floor of the residual suppressor's gain: 30 dB down while the far end plays
/// alone, which is quiet enough that no one hears themselves back.
const ECHO_RESIDUAL_MIN_GAIN: f32 = 0.03;
/// Share of the distance to the new residual gain covered per frame.
const ECHO_RESIDUAL_SMOOTHING: f32 = 0.5;

/// Subtracts what the mixer played from what the microphone heard.
///
/// A normalized least-mean-squares filter models the path from the speakers
/// back into the microphone and subtracts its estimate; a Wiener-style
/// suppressor then holds down whatever the filter could not model. Without a
/// reference — voice chat with no audio device, or echo cancellation switched
/// off before the capture opened — it is a no-op.
#[derive(Debug)]
pub(crate) struct EchoCanceller {
    reader: Option<EchoReferenceReader>,
    weights: Box<[f32]>,
    /// The far end from one tail before this frame up to its end, so that
    /// `history[offset + 1..offset + 1 + ECHO_TAIL_SAMPLES]` is the tap window
    /// for sample `offset` — oldest first, ending on the far-end sample that
    /// shares its instant.
    history: Box<[f32]>,
    far: [f32; VOICE_FRAME_SAMPLES],
    /// Loudest far-end sample still inside the tap window, for the double-talk
    /// detector.
    far_peaks: [f32; ECHO_TAIL_FRAMES],
    residual_gain: f32,
    adapted_frames: u32,
}

const ECHO_TAIL_FRAMES: usize = ECHO_TAIL_SAMPLES.div_ceil(VOICE_FRAME_SAMPLES);

impl VoiceEchoReference {
    fn new() -> Self {
        Self {
            ring: Arc::new(EchoRing {
                samples: (0..ECHO_REFERENCE_SAMPLES)
                    .map(|_| AtomicU32::new(0))
                    .collect(),
                written: AtomicU64::new(0),
            }),
        }
    }

    fn push(&self, sample: f32) {
        let ring = &self.ring;
        let position = ring.written.load(Ordering::Relaxed);
        let index = position as usize % ECHO_REFERENCE_SAMPLES;
        ring.samples[index].store(sample.to_bits(), Ordering::Relaxed);
        // Release so a reader that observes this count also observes the sample.
        ring.written
            .store(position.wrapping_add(1), Ordering::Release);
    }

    fn written(&self) -> u64 {
        self.ring.written.load(Ordering::Acquire)
    }

    fn sample_at(&self, position: u64) -> f32 {
        let index = position as usize % ECHO_REFERENCE_SAMPLES;
        f32::from_bits(self.ring.samples[index].load(Ordering::Relaxed))
    }
}

impl VoiceEchoTap {
    pub(crate) fn new(output_sample_rate: u32) -> Self {
        Self {
            reference: VoiceEchoReference::new(),
            resampler: StreamingVoiceResampler::new(output_sample_rate),
        }
    }

    pub(crate) fn reference(&self) -> VoiceEchoReference {
        self.reference.clone()
    }

    /// One mixed output frame, downmixed to the mono the canceller compares
    /// against. Called with the float pair the mixer has just summed, before it
    /// is converted to the device's sample format.
    pub(crate) fn push_output_frame(&mut self, left: f32, right: f32) {
        let reference = &self.reference;
        self.resampler
            .push_sample((left + right) * 0.5, |sample| reference.push(sample));
    }

    /// Publish `frames` of silence. The mixer skips its per-frame loop entirely
    /// when nothing is playing, and the reference timeline may not stop with
    /// it: the capture side reads it as a clock, and a gap would look like an
    /// abrupt change in the echo path.
    pub(crate) fn push_silence(&mut self, frames: usize) {
        for _ in 0..frames {
            self.push_output_frame(0.0, 0.0);
        }
    }
}

impl EchoReferenceReader {
    pub(crate) fn new(reference: VoiceEchoReference) -> Self {
        let position = reference.written();
        Self {
            reference,
            position,
        }
    }

    /// The far-end block that lines up with the microphone frame being
    /// processed. Underruns fill the tail with silence rather than skipping
    /// ahead, so a writer running slightly slow only stretches the alignment
    /// the adaptive filter tracks anyway.
    pub(crate) fn read(&mut self, far: &mut [f32; VOICE_FRAME_SAMPLES]) {
        let written = self.reference.written();
        if self.position > written || written - self.position > MAX_REFERENCE_LAG_SAMPLES {
            self.position = written.saturating_sub(RESYNC_REFERENCE_LAG_SAMPLES);
        }
        let available = (written - self.position).min(VOICE_FRAME_SAMPLES as u64) as usize;
        for (offset, sample) in far.iter_mut().enumerate() {
            *sample = if offset < available {
                self.reference.sample_at(self.position + offset as u64)
            } else {
                0.0
            };
        }
        self.position += available as u64;
    }
}

impl EchoCanceller {
    pub(crate) fn new(reference: Option<VoiceEchoReference>) -> Self {
        Self {
            reader: reference.map(EchoReferenceReader::new),
            weights: vec![0.0; ECHO_TAIL_SAMPLES].into_boxed_slice(),
            history: vec![0.0; ECHO_TAIL_SAMPLES + VOICE_FRAME_SAMPLES].into_boxed_slice(),
            far: [0.0; VOICE_FRAME_SAMPLES],
            far_peaks: [0.0; ECHO_TAIL_FRAMES],
            residual_gain: 1.0,
            adapted_frames: 0,
        }
    }

    pub(crate) fn process(&mut self, frame: &mut [f32; VOICE_FRAME_SAMPLES]) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        reader.read(&mut self.far);
        self.history.copy_within(VOICE_FRAME_SAMPLES.., 0);
        self.history[ECHO_TAIL_SAMPLES..].copy_from_slice(&self.far);
        self.far_peaks.rotate_left(1);
        self.far_peaks[ECHO_TAIL_FRAMES - 1] = self
            .far
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));

        // One normalization per frame rather than per sample: the tap window
        // moves by a single sample between them, and a frame-constant step is
        // the standard block form of the update.
        let energy = self.history[VOICE_FRAME_SAMPLES..]
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>();
        let far_peak = self
            .far_peaks
            .iter()
            .fold(0.0_f32, |peak, value| peak.max(*value));
        let microphone_peak = frame
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let near_end_present = microphone_peak > far_peak * ECHO_PATH_MAX_GAIN;
        let adapt = energy > 1e-6 && !near_end_present;
        let cold_start = self.adapted_frames < ECHO_COLD_START_ADAPTATION_FRAMES;

        let mut echo_energy = 0.0;
        let mut error_energy = 0.0;
        for (offset, sample) in frame.iter_mut().enumerate() {
            let window = &self.history[offset + 1..offset + 1 + ECHO_TAIL_SAMPLES];
            let estimate = self
                .weights
                .iter()
                .zip(window)
                .map(|(weight, far)| weight * far)
                .sum::<f32>();
            let error = *sample - estimate;
            if adapt {
                let step = ECHO_ADAPTATION_RATE * error / energy;
                for (weight, far) in self.weights.iter_mut().zip(window) {
                    *weight += step * far;
                }
            }
            echo_energy += estimate * estimate;
            error_energy += error * error;
            *sample = error;
        }
        if adapt {
            self.adapted_frames = self.adapted_frames.saturating_add(1);
        }

        // What the filter could not model is proportional to what it did model,
        // so hold the frame down by the Wiener gain that would leave only the
        // near end behind.
        let far_frame_energy = self.far.iter().map(|sample| sample * sample).sum::<f32>();
        let cold_start_residual = if cold_start && !near_end_present {
            ECHO_COLD_START_RESIDUAL_RATIO * far_frame_energy
        } else {
            0.0
        };
        let residual = (ECHO_RESIDUAL_LEAKAGE * echo_energy).max(cold_start_residual);
        let target =
            (error_energy / (error_energy + residual + 1e-12)).clamp(ECHO_RESIDUAL_MIN_GAIN, 1.0);
        self.residual_gain += (target - self.residual_gain) * ECHO_RESIDUAL_SMOOTHING;
        for sample in frame.iter_mut() {
            *sample *= self.residual_gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VOICE_SAMPLE_RATE;

    /// A deterministic broadband signal to stand in for game audio. Nothing
    /// here reaches the simulation, so a plain congruential generator is all a
    /// repeatable test needs.
    struct TestSignal(u32);

    impl TestSignal {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (self.0 >> 8) as f32 / 8_388_608.0 - 1.0
        }
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// Speaker to microphone: a delay, an attenuation, and two reflections.
    fn echo_of(history: &[f32], end: usize, delay: usize) -> f32 {
        0.5 * history[end - delay]
            + 0.25 * history[end - delay - 17]
            + 0.12 * history[end - delay - 53]
    }

    #[test]
    fn echo_cancellation_removes_a_delayed_copy_of_what_the_mixer_played() {
        let mut tap = VoiceEchoTap::new(16_000);
        let mut canceller = EchoCanceller::new(Some(tap.reference()));
        let mut signal = TestSignal(11);
        let delay = 700;
        let mut played = vec![0.0; delay + 64 + VOICE_FRAME_SAMPLES];
        let mut heard = Vec::new();
        let mut sent = Vec::new();

        for index in 0..400 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                *sample = signal.next() * 0.3;
                tap.push_output_frame(*sample, *sample);
            }
            played.extend_from_slice(&frame);
            let mut microphone = [0.0; VOICE_FRAME_SAMPLES];
            for (offset, sample) in microphone.iter_mut().enumerate() {
                *sample = echo_of(&played, played.len() - VOICE_FRAME_SAMPLES + offset, delay);
            }
            let raw = microphone;
            canceller.process(&mut microphone);
            if index >= 380 {
                heard.extend_from_slice(&raw);
                sent.extend_from_slice(&microphone);
            }
        }

        let reduction = 20.0 * (rms(&heard) / rms(&sent).max(1e-9)).log10();
        assert!(
            reduction >= 20.0,
            "the speaker bleed should be at least 20 dB quieter, got {reduction:.1} dB",
        );
    }

    #[test]
    fn echo_cancellation_quiets_speaker_bleed_within_a_short_utterance() {
        let mut tap = VoiceEchoTap::new(16_000);
        let mut canceller = EchoCanceller::new(Some(tap.reference()));
        let mut signal = TestSignal(17);
        let delay = 700;
        let mut played = vec![0.0; delay + 64 + VOICE_FRAME_SAMPLES];
        let mut heard = Vec::new();
        let mut sent = Vec::new();

        for index in 0..12 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                *sample = signal.next() * 0.3;
                tap.push_output_frame(*sample, *sample);
            }
            played.extend_from_slice(&frame);
            let mut microphone = [0.0; VOICE_FRAME_SAMPLES];
            for (offset, sample) in microphone.iter_mut().enumerate() {
                *sample = echo_of(&played, played.len() - VOICE_FRAME_SAMPLES + offset, delay);
            }
            let raw = microphone;
            canceller.process(&mut microphone);
            if index >= 7 {
                heard.extend_from_slice(&raw);
                sent.extend_from_slice(&microphone);
            }
        }

        let reduction = 20.0 * (rms(&heard) / rms(&sent).max(1e-9)).log10();
        assert!(
            reduction >= 15.0,
            "push-to-talk is often shorter than a second; speaker bleed fell only {reduction:.1} dB",
        );
    }

    #[test]
    fn echo_cancellation_still_lets_someone_talk_over_the_game() {
        let mut tap = VoiceEchoTap::new(16_000);
        let mut canceller = EchoCanceller::new(Some(tap.reference()));
        let mut signal = TestSignal(23);
        let delay = 400;
        let mut played = vec![0.0; delay + 64 + VOICE_FRAME_SAMPLES];
        let mut phase = 0.0_f32;
        let mut spoken = Vec::new();
        let mut sent = Vec::new();

        for index in 0..400 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                *sample = signal.next() * 0.3;
                tap.push_output_frame(*sample, *sample);
            }
            played.extend_from_slice(&frame);
            // The player starts talking once the filter has had time to settle.
            let talking = index >= 300;
            let mut microphone = [0.0; VOICE_FRAME_SAMPLES];
            let mut speech = [0.0; VOICE_FRAME_SAMPLES];
            for (offset, sample) in microphone.iter_mut().enumerate() {
                phase += std::f32::consts::TAU * 220.0 / VOICE_SAMPLE_RATE as f32;
                speech[offset] = if talking { 0.25 * phase.sin() } else { 0.0 };
                *sample = echo_of(&played, played.len() - VOICE_FRAME_SAMPLES + offset, delay)
                    + speech[offset];
            }
            canceller.process(&mut microphone);
            if index >= 390 {
                spoken.extend_from_slice(&speech);
                sent.extend_from_slice(&microphone);
            }
        }

        let kept = rms(&sent) / rms(&spoken);
        assert!(
            kept > 0.5,
            "speech over the game must still get through, kept {kept:.2} of it",
        );
    }

    #[test]
    fn converged_echo_cancellation_keeps_a_quiet_talker_over_loud_game_audio() {
        let mut tap = VoiceEchoTap::new(16_000);
        let mut canceller = EchoCanceller::new(Some(tap.reference()));
        let mut signal = TestSignal(31);
        let delay = 400;
        let mut played = vec![0.0; delay + VOICE_FRAME_SAMPLES];
        let mut phase = 0.0_f32;
        let mut spoken = Vec::new();
        let mut sent = Vec::new();

        for index in 0..60 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in &mut frame {
                *sample = signal.next() * 0.3;
                tap.push_output_frame(*sample, *sample);
            }
            played.extend_from_slice(&frame);
            let talking = index >= 40;
            let mut microphone = [0.0; VOICE_FRAME_SAMPLES];
            let mut speech = [0.0; VOICE_FRAME_SAMPLES];
            for (offset, sample) in microphone.iter_mut().enumerate() {
                phase += std::f32::consts::TAU * 220.0 / VOICE_SAMPLE_RATE as f32;
                speech[offset] = if talking { 0.04 * phase.sin() } else { 0.0 };
                let at = played.len() - VOICE_FRAME_SAMPLES + offset;
                *sample = 0.1 * played[at - delay] + speech[offset];
            }
            canceller.process(&mut microphone);
            if index >= 50 {
                spoken.extend_from_slice(&speech);
                sent.extend_from_slice(&microphone);
            }
        }

        let kept = rms(&sent) / rms(&spoken);
        assert!(
            kept > 0.75,
            "the startup fallback stayed on after convergence and kept only {kept:.2} of quiet speech",
        );
    }

    #[test]
    fn cold_echo_cancellation_keeps_speech_over_the_game() {
        let mut tap = VoiceEchoTap::new(16_000);
        let mut canceller = EchoCanceller::new(Some(tap.reference()));
        let mut signal = TestSignal(29);
        let delay = 400;
        let mut played = vec![0.0; delay + 64 + VOICE_FRAME_SAMPLES];
        let mut phase = 0.0_f32;
        let mut spoken = Vec::new();
        let mut sent = Vec::new();

        for index in 0..12 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                *sample = signal.next() * 0.3;
                tap.push_output_frame(*sample, *sample);
            }
            played.extend_from_slice(&frame);
            let mut microphone = [0.0; VOICE_FRAME_SAMPLES];
            let mut speech = [0.0; VOICE_FRAME_SAMPLES];
            for (offset, sample) in microphone.iter_mut().enumerate() {
                phase += std::f32::consts::TAU * 220.0 / VOICE_SAMPLE_RATE as f32;
                speech[offset] = 0.25 * phase.sin();
                *sample = echo_of(&played, played.len() - VOICE_FRAME_SAMPLES + offset, delay)
                    + speech[offset];
            }
            canceller.process(&mut microphone);
            if index >= 7 {
                spoken.extend_from_slice(&speech);
                sent.extend_from_slice(&microphone);
            }
        }

        let kept = rms(&sent) / rms(&spoken);
        assert!(
            kept > 0.5,
            "cold-start echo suppression must not mute the nearby talker, kept {kept:.2}",
        );
    }

    #[test]
    fn echo_cancellation_without_a_reference_leaves_the_microphone_alone() {
        let mut canceller = EchoCanceller::new(None);
        let mut frame = std::array::from_fn(|index| index as f32 / VOICE_FRAME_SAMPLES as f32);
        let untouched = frame;

        canceller.process(&mut frame);

        assert_eq!(frame, untouched);
    }

    #[test]
    fn the_echo_reference_hands_the_capture_side_what_the_mixer_wrote() {
        let mut tap = VoiceEchoTap::new(16_000);
        let mut reader = EchoReferenceReader::new(tap.reference());

        for index in 0..VOICE_FRAME_SAMPLES {
            let sample = index as f32 / VOICE_FRAME_SAMPLES as f32;
            tap.push_output_frame(sample, -sample);
        }

        let mut far = [1.0; VOICE_FRAME_SAMPLES];
        reader.read(&mut far);
        assert!(
            far.iter().all(|sample| sample.abs() < 1e-6),
            "a hard-panned pair cancels to a silent mono reference",
        );

        for index in 0..VOICE_FRAME_SAMPLES {
            let sample = index as f32 / VOICE_FRAME_SAMPLES as f32;
            tap.push_output_frame(sample, sample);
        }
        reader.read(&mut far);
        assert_eq!(far[0], 0.0);
        assert!((far[VOICE_FRAME_SAMPLES - 1] - 319.0 / 320.0).abs() < 1e-6);
    }

    #[test]
    fn a_reader_ahead_of_the_mixer_reads_silence_and_keeps_the_samples_it_missed() {
        let mut tap = VoiceEchoTap::new(16_000);
        let mut reader = EchoReferenceReader::new(tap.reference());
        let mut far = [1.0; VOICE_FRAME_SAMPLES];

        reader.read(&mut far);
        assert_eq!(
            far, [0.0; VOICE_FRAME_SAMPLES],
            "silence until output starts"
        );

        for _ in 0..VOICE_FRAME_SAMPLES / 2 {
            tap.push_output_frame(0.5, 0.5);
        }
        reader.read(&mut far);
        assert!(
            far[..VOICE_FRAME_SAMPLES / 2]
                .iter()
                .all(|sample| *sample == 0.5),
            "what the mixer did render is read in order",
        );
        assert!(
            far[VOICE_FRAME_SAMPLES / 2..]
                .iter()
                .all(|sample| *sample == 0.0),
            "the tail it has not rendered yet reads as silence",
        );

        for _ in 0..VOICE_FRAME_SAMPLES / 2 {
            tap.push_output_frame(0.25, 0.25);
        }
        reader.read(&mut far);
        assert!(
            far[..VOICE_FRAME_SAMPLES / 2]
                .iter()
                .all(|sample| *sample == 0.25),
            "an underrun does not skip the samples that arrive next",
        );
    }

    #[test]
    fn a_reader_that_falls_far_behind_jumps_back_to_the_live_signal() {
        let mut tap = VoiceEchoTap::new(16_000);
        let mut reader = EchoReferenceReader::new(tap.reference());

        for index in 0..MAX_REFERENCE_LAG_SAMPLES + VOICE_FRAME_SAMPLES as u64 {
            tap.push_output_frame(index as f32, index as f32);
        }

        let mut far = [0.0; VOICE_FRAME_SAMPLES];
        reader.read(&mut far);
        let written = MAX_REFERENCE_LAG_SAMPLES + VOICE_FRAME_SAMPLES as u64;
        assert_eq!(
            far[0],
            (written - RESYNC_REFERENCE_LAG_SAMPLES) as f32,
            "the reader resumes near the live end of the reference, not where it stalled",
        );
    }
}
