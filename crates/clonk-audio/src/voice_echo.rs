//! The far-end reference an acoustic echo canceller subtracts: what the mixer
//! is about to play, published for the microphone thread to read.
//!
//! The two sides are separate `cpal` streams on separate OS callback threads
//! with independent clocks, so the handoff is a lock-free single-producer,
//! single-consumer ring of atomic samples rather than a shared lock. Neither
//! audio callback may ever wait on the other.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::voice_output_reference::OutputReference;

use aec3::api::control::EchoControl;
use aec3::audio_processing::aec3::echo_canceller3::EchoCanceller3;
use aec3::audio_processing::audio_buffer::AudioBuffer;
use aec3::audio_processing::stream_config::StreamConfig;

use crate::voice::StreamingVoiceResampler;
use crate::VOICE_FRAME_SAMPLES;

/// Over a second of 48 kHz mono history. A power of two so the write
/// position masks into an index.
const ECHO_REFERENCE_SAMPLES: usize = 65_536;

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
    output: Option<Arc<Mutex<Option<Arc<OutputReference>>>>>,
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
    active_output: Option<Arc<OutputReference>>,
    output_epoch: u64,
}

/// WebRTC AEC3 tracks render-to-capture delay instead of requiring the echo
/// path to fit inside a fixed time-domain filter. Processing uses 10 ms blocks;
/// the media codec may packetize two of them together.
pub(crate) struct EchoCanceller {
    reader: Option<EchoReferenceReader>,
    processor: Option<Box<EchoProcessor>>,
    far: [f32; VOICE_FRAME_SAMPLES],
}

impl std::fmt::Debug for EchoCanceller {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EchoCanceller")
            .field("has_reference", &self.reader.is_some())
            .finish_non_exhaustive()
    }
}

impl VoiceEchoReference {
    fn new() -> Self {
        Self {
            ring: Arc::new(EchoRing {
                samples: (0..ECHO_REFERENCE_SAMPLES)
                    .map(|_| AtomicU32::new(0))
                    .collect(),
                written: AtomicU64::new(0),
            }),
            output: None,
        }
    }

    pub(crate) fn for_output() -> Self {
        Self {
            output: Some(Arc::new(Mutex::new(None))),
            ..Self::new()
        }
    }

    pub(crate) fn set_output(&self, reference: Option<Arc<OutputReference>>) {
        if let Some(output) = &self.output {
            *output.lock().unwrap() = reference;
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
            active_output: None,
            output_epoch: 0,
        }
    }

    pub(crate) fn read_at(
        &mut self,
        far: &mut [f32; VOICE_FRAME_SAMPLES],
        captured_at: Instant,
    ) -> bool {
        if let Some(output) = &self.reference.output {
            let next = output.lock().unwrap().clone();
            let epoch = next.as_ref().map_or(0, |output| output.epoch());
            let changed = match (&self.active_output, &next) {
                (Some(old), Some(new)) => !Arc::ptr_eq(old, new) || self.output_epoch != epoch,
                (None, None) => false,
                _ => true,
            };
            self.active_output = next;
            self.output_epoch = epoch;
            if let Some(output) = &self.active_output {
                output.read_at(far, captured_at);
            } else {
                far.fill(0.0);
            }
            return changed;
        }
        self.read(far);
        false
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
        let processor = reference.as_ref().map(|_| Box::new(EchoProcessor::new()));
        Self {
            reader: reference.map(EchoReferenceReader::new),
            processor,
            far: [0.0; VOICE_FRAME_SAMPLES],
        }
    }

    #[cfg(test)]
    pub(crate) fn process(&mut self, frame: &mut [f32; VOICE_FRAME_SAMPLES]) {
        self.process_at(frame, Instant::now());
    }

    pub(crate) fn process_at(
        &mut self,
        frame: &mut [f32; VOICE_FRAME_SAMPLES],
        captured_at: Instant,
    ) {
        let Some(reader) = &mut self.reader else {
            return;
        };
        if reader.read_at(&mut self.far, captured_at) {
            self.processor = Some(Box::new(EchoProcessor::new()));
        }
        let Some(processor) = &mut self.processor else {
            return;
        };
        const BLOCK: usize = VOICE_FRAME_SAMPLES / 2;
        for (render, capture) in self
            .far
            .chunks_exact(BLOCK)
            .zip(frame.chunks_exact_mut(BLOCK))
        {
            processor.process(render, capture);
        }
    }
}

/// Fixed buffers avoid the graph API's per-frame packet allocations and keep
/// buffers reusable. The processor is constructed on its owning capture thread.
struct EchoProcessor {
    echo: EchoCanceller3,
    render: AudioBuffer,
    capture: AudioBuffer,
    stream: StreamConfig,
}

impl EchoProcessor {
    fn new() -> Self {
        let mut config = aec3::api::config::EchoCanceller3Config::default();
        // Detect nearby speech after 8 ms instead of 48 ms of qualifying
        // blocks. Short push-to-talk utterances must survive the cold start;
        // the echo-only and double-talk regressions pin both sides of this.
        config
            .suppressor
            .dominant_nearend_detection
            .trigger_threshold = 2;
        let samples = VOICE_FRAME_SAMPLES / 2;
        Self {
            echo: EchoCanceller3::new(config, crate::VOICE_SAMPLE_RATE as i32, 1, 1),
            render: AudioBuffer::new(samples, 1, samples, 1, samples),
            capture: AudioBuffer::new(samples, 1, samples, 1, samples),
            stream: StreamConfig::new(crate::VOICE_SAMPLE_RATE as usize, 1, false),
        }
    }

    fn process(&mut self, render: &[f32], capture: &mut [f32]) {
        self.render.copy_from(&[render], &self.stream);
        self.render.split_into_frequency_bands();
        self.echo.analyze_render(&mut self.render);
        self.capture.copy_from(&[capture], &self.stream);
        self.echo.analyze_capture(&mut self.capture);
        self.capture.split_into_frequency_bands();
        self.echo.process_capture(&mut self.capture, false);
        self.capture.merge_frequency_bands();
        self.capture.copy_to_stream(&self.stream, &mut [capture]);
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
    fn timed_device_reference_cancels_a_three_hundred_millisecond_echo() {
        let output = OutputReference::new(VOICE_SAMPLE_RATE);
        let reference = VoiceEchoReference::for_output();
        reference.set_output(Some(output.clone()));
        let mut canceller = EchoCanceller::new(Some(reference));
        let mut signal = TestSignal(11);
        let delay = VOICE_SAMPLE_RATE as usize * 300 / 1_000;
        let mut played = vec![0.0; delay + 64 + VOICE_FRAME_SAMPLES];
        let mut heard = Vec::new();
        let mut sent = Vec::new();
        let start = Instant::now();
        for index in 0..400 {
            let first = output.written();
            let capture_time = start + std::time::Duration::from_millis(index * 20);
            let frame = std::array::from_fn::<_, VOICE_FRAME_SAMPLES, _>(|_| {
                let sample = signal.next() * 0.3;
                output.push(sample);
                sample
            });
            output.publish_timing(first, capture_time);
            played.extend_from_slice(&frame);
            let mut microphone = std::array::from_fn(|offset| {
                echo_of(&played, played.len() - VOICE_FRAME_SAMPLES + offset, delay)
            });
            let raw = microphone;
            canceller.process_at(&mut microphone, capture_time);
            if index >= 380 {
                heard.extend_from_slice(&raw);
                sent.extend_from_slice(&microphone);
            }
        }
        let reduction = 20.0 * (rms(&heard) / rms(&sent).max(1e-9)).log10();
        assert!(
            reduction >= 20.0,
            "timed output reference reduced echo by only {reduction:.2} dB"
        );
    }

    #[test]
    fn echo_reference_resets_on_output_replacement_removal_and_clock_discontinuity() {
        let reference = VoiceEchoReference::for_output();
        let mut reader = EchoReferenceReader::new(reference.clone());
        let mut far = [0.0; VOICE_FRAME_SAMPLES];
        let start = Instant::now();
        let first = OutputReference::new(48_000);
        reference.set_output(Some(first.clone()));
        assert!(reader.read_at(&mut far, start));
        assert!(!reader.read_at(&mut far, start));
        let second = OutputReference::new(96_000);
        reference.set_output(Some(second.clone()));
        let time = Instant::now();
        for _ in 0..1920 {
            second.push(0.1);
        }
        second.publish_timing(0, time);
        assert!(reader.read_at(&mut far, time));
        // The former output can still finish its callback on another thread.
        for _ in 0..960 {
            first.push(0.8);
        }
        first.publish_timing(0, time);
        assert!(!reader.read_at(&mut far, time));
        assert!(far[128..832]
            .iter()
            .all(|sample| (*sample - 0.1).abs() < 0.0001));
        second.publish_timing(1920, time + std::time::Duration::from_secs(1));
        assert!(reader.read_at(&mut far, time));
        assert!(!reader.read_at(&mut far, time));
        reference.set_output(None);
        assert!(reader.read_at(&mut far, time));
        assert!(far.iter().all(|sample| *sample == 0.0));
        assert!(!reader.read_at(&mut far, time));
    }

    #[test]
    fn output_echo_reference_uses_capture_time_after_a_dsp_scheduling_delay() {
        let output = OutputReference::new(VOICE_SAMPLE_RATE);
        let start = Instant::now();
        for frame in 0..10 {
            let position = output.written();
            for _ in 0..VOICE_FRAME_SAMPLES {
                output.push(frame as f32 * 0.05);
            }
            output.publish_timing(
                position,
                start + std::time::Duration::from_millis(frame * 20),
            );
        }
        let reference = VoiceEchoReference::for_output();
        reference.set_output(Some(output));
        let mut reader = EchoReferenceReader::new(reference);
        let mut far = [0.0; VOICE_FRAME_SAMPLES];
        reader.read_at(&mut far, start + std::time::Duration::from_millis(40));
        assert!(far[128..832]
            .iter()
            .all(|sample| (*sample - 0.1).abs() < 0.0001));
    }

    #[test]
    fn echo_cancellation_rejects_a_three_hundred_millisecond_echo_path() {
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
        let mut canceller = EchoCanceller::new(Some(tap.reference()));
        let mut signal = TestSignal(11);
        let delay = VOICE_SAMPLE_RATE as usize * 300 / 1_000;
        let mut played = vec![0.0; delay + 64 + VOICE_FRAME_SAMPLES];
        let mut heard = Vec::new();
        let mut sent = Vec::new();
        for index in 0..400 {
            let frame = std::array::from_fn::<_, VOICE_FRAME_SAMPLES, _>(|_| {
                let sample = signal.next() * 0.3;
                tap.push_output_frame(sample, sample);
                sample
            });
            played.extend_from_slice(&frame);
            let mut microphone = std::array::from_fn(|offset| {
                echo_of(&played, played.len() - VOICE_FRAME_SAMPLES + offset, delay)
            });
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
            "a long device path must still cancel echo, got {reduction:.1} dB"
        );
    }

    #[test]
    fn echo_cancellation_removes_a_delayed_copy_of_what_the_mixer_played() {
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
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
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
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
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
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
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
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
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
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
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
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
        assert!(
            (far[VOICE_FRAME_SAMPLES - 1]
                - (VOICE_FRAME_SAMPLES - 1) as f32 / VOICE_FRAME_SAMPLES as f32)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn a_reader_ahead_of_the_mixer_reads_silence_and_keeps_the_samples_it_missed() {
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
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
        let mut tap = VoiceEchoTap::new(VOICE_SAMPLE_RATE);
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
