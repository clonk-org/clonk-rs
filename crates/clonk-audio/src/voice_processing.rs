//! What the microphone hears is not what anyone wants to send: the speakers
//! bleed back into it, the room hums, and every participant's level is
//! whatever their hardware happened to pick. This is the capture-side chain
//! that fixes the three, in the order they have to run — cancel the echo, then
//! suppress the noise, then set the level — each switchable on its own
//! (clonk-org/clonk-rs#421).
//!
//! It runs inside the microphone callback, on the fixed 20 ms, 16 kHz mono
//! frame the encoder is about to consume, and never changes that geometry.
//! Every buffer it needs is allocated when the capture opens, so a frame costs
//! no allocation and takes no lock.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::voice::voice_level_from_rms;
use crate::voice_echo::{EchoCanceller, VoiceEchoReference};
use crate::VOICE_FRAME_SAMPLES;

/// Which capture-processing stages are running. All three are on by default:
/// voice chat is itself opt-in, and a player who has opted in wants to be
/// heard rather than to hear their own room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceProcessingConfig {
    pub echo_cancellation: bool,
    pub noise_suppression: bool,
    pub automatic_gain_control: bool,
}

impl VoiceProcessingConfig {
    /// The unprocessed capture path: exactly what the microphone heard.
    pub const DISABLED: Self = Self {
        echo_cancellation: false,
        noise_suppression: false,
        automatic_gain_control: false,
    };
}

impl Default for VoiceProcessingConfig {
    fn default() -> Self {
        Self {
            echo_cancellation: true,
            noise_suppression: true,
            automatic_gain_control: true,
        }
    }
}

/// A [`VoiceProcessingConfig`] the microphone thread can read while the app
/// thread writes it. Settings changed mid-call take effect on the next frame,
/// in both capture modes, without closing the microphone — reopening it would
/// throw away everything the echo canceller and the noise floor had learned.
#[derive(Debug)]
pub struct VoiceProcessingSwitches {
    echo_cancellation: AtomicBool,
    noise_suppression: AtomicBool,
    automatic_gain_control: AtomicBool,
}

impl VoiceProcessingSwitches {
    pub fn new(config: VoiceProcessingConfig) -> Arc<Self> {
        let switches = Arc::new(Self {
            echo_cancellation: AtomicBool::new(false),
            noise_suppression: AtomicBool::new(false),
            automatic_gain_control: AtomicBool::new(false),
        });
        switches.set(config);
        switches
    }

    pub fn set(&self, config: VoiceProcessingConfig) {
        self.echo_cancellation
            .store(config.echo_cancellation, Ordering::Relaxed);
        self.noise_suppression
            .store(config.noise_suppression, Ordering::Relaxed);
        self.automatic_gain_control
            .store(config.automatic_gain_control, Ordering::Relaxed);
    }

    pub fn get(&self) -> VoiceProcessingConfig {
        VoiceProcessingConfig {
            echo_cancellation: self.echo_cancellation.load(Ordering::Relaxed),
            noise_suppression: self.noise_suppression.load(Ordering::Relaxed),
            automatic_gain_control: self.automatic_gain_control.load(Ordering::Relaxed),
        }
    }
}

/// The capture-side chain: [`EchoCanceller`], [`NoiseSuppressor`] and
/// [`AutomaticGainControl`], each run only while its switch is on.
#[derive(Debug)]
pub(crate) struct VoiceProcessing {
    switches: Arc<VoiceProcessingSwitches>,
    echo: EchoCanceller,
    noise: NoiseSuppressor,
    gain: AutomaticGainControl,
}

impl VoiceProcessing {
    pub(crate) fn new(
        switches: Arc<VoiceProcessingSwitches>,
        echo_reference: Option<VoiceEchoReference>,
    ) -> Self {
        Self {
            switches,
            echo: EchoCanceller::new(echo_reference),
            noise: NoiseSuppressor::new(),
            gain: AutomaticGainControl::new(),
        }
    }

    /// Processes one captured frame in place and returns how loud it is, on
    /// [`voice_activation_level`](crate::voice_activation_level)'s scale.
    ///
    /// That level is measured **before** automatic gain control, which exists
    /// precisely to erase the difference between a quiet room and a talker.
    /// Measuring after it would hand the voice-activation gate a signal in
    /// which every frame is equally loud, and the gate would hold the
    /// microphone open on a hum.
    pub(crate) fn process(&mut self, frame: &mut [f32; VOICE_FRAME_SAMPLES]) -> f32 {
        let config = self.switches.get();
        if config.echo_cancellation {
            self.echo.process(frame);
        }
        if config.noise_suppression {
            self.noise.process(frame);
        }
        let level = frame_level(frame);
        if config.automatic_gain_control {
            self.gain.process(frame);
        }
        level
    }
}

/// Half-overlapping analysis windows, each two frames long, so exactly one
/// transform runs per captured frame and the suppressor's own delay is exactly
/// one frame. A square-root Hann window on both the analysis and the synthesis
/// side sums back to unity at that overlap, so a frame nothing is subtracted
/// from comes out as it went in.
const NOISE_WINDOW_SAMPLES: usize = 2 * VOICE_FRAME_SAMPLES;
/// The window zero-padded up to the next power of two the transform needs.
const NOISE_TRANSFORM_SAMPLES: usize = 1_024;
const NOISE_BINS: usize = NOISE_TRANSFORM_SAMPLES / 2 + 1;
/// Quietest a bin may be held to. Voice sounds thin and starts to bubble when
/// the noise between the harmonics is removed completely, so the floor is 24 dB
/// rather than silence.
const NOISE_MIN_GAIN: f32 = 0.06;
/// Tracking a bin's minimum finds a noise floor well below the mean of a
/// fluctuating spectrum, so the estimate is scaled back up before it is
/// subtracted. Doubling it costs about one decibel of speech and buys around
/// thirteen decibels of noise.
const NOISE_ESTIMATE_BIAS: f32 = 2.0;
/// How fast the noise floor follows the spectrum down (a room that goes quiet)
/// and up (a fan spinning up). Down in about 0.4 s, up over several seconds, so
/// that speech — which is loud and brief — is never mistaken for the floor.
const NOISE_FLOOR_DECAY: f32 = 0.95;
const NOISE_FLOOR_RISE: f32 = 0.995;
/// Weight of the previous frame's estimate in the decision-directed a-priori
/// signal-to-noise ratio, which is what keeps the gain from flickering between
/// frames and turning residual noise into warbling tones.
const NOISE_SNR_SMOOTHING: f32 = 0.98;
/// Frames used to seed the noise floor before any suppression is applied.
const NOISE_FLOOR_SEED_FRAMES: u32 = 4;

/// Removes the part of the spectrum that stays put while speech comes and goes.
///
/// The frame is transformed, each bin is scaled by the Wiener gain implied by
/// its signal-to-noise ratio against a per-bin noise floor, and the result is
/// overlap-added back. Steady noise sits near the floor and is held down;
/// speech towers over it and is not.
#[derive(Debug)]
struct NoiseSuppressor {
    window: Box<[f32]>,
    previous_input: [f32; VOICE_FRAME_SAMPLES],
    overlap: [f32; VOICE_FRAME_SAMPLES],
    transform: DiscreteTransform,
    noise: Box<[f32]>,
    previous_gain: Box<[f32]>,
    previous_power: Box<[f32]>,
    frames: u32,
}

impl NoiseSuppressor {
    fn new() -> Self {
        Self {
            window: (0..NOISE_WINDOW_SAMPLES)
                .map(|index| {
                    let hann = 0.5
                        - 0.5
                            * (std::f32::consts::TAU * index as f32 / NOISE_WINDOW_SAMPLES as f32)
                                .cos();
                    hann.sqrt()
                })
                .collect(),
            previous_input: [0.0; VOICE_FRAME_SAMPLES],
            overlap: [0.0; VOICE_FRAME_SAMPLES],
            transform: DiscreteTransform::new(),
            noise: vec![0.0; NOISE_BINS].into_boxed_slice(),
            previous_gain: vec![1.0; NOISE_BINS].into_boxed_slice(),
            previous_power: vec![0.0; NOISE_BINS].into_boxed_slice(),
            frames: 0,
        }
    }

    fn process(&mut self, frame: &mut [f32; VOICE_FRAME_SAMPLES]) {
        self.transform.clear();
        // The analysis window spans the previous frame and this one.
        let windowed = self
            .previous_input
            .iter()
            .chain(frame.iter())
            .zip(self.window.iter());
        for (slot, (sample, window)) in self.transform.real.iter_mut().zip(windowed) {
            *slot = sample * window;
        }
        self.previous_input.copy_from_slice(frame);
        self.transform.run(TransformDirection::Forward);

        self.frames = self.frames.saturating_add(1);
        for bin in 0..NOISE_BINS {
            let real = self.transform.real[bin];
            let imaginary = self.transform.imaginary[bin];
            let power = real * real + imaginary * imaginary;
            let floor = &mut self.noise[bin];
            if self.frames <= NOISE_FLOOR_SEED_FRAMES {
                *floor = floor.max(power);
            } else if power < *floor {
                *floor = NOISE_FLOOR_DECAY * *floor + (1.0 - NOISE_FLOOR_DECAY) * power;
            } else {
                *floor = NOISE_FLOOR_RISE * *floor + (1.0 - NOISE_FLOOR_RISE) * power;
            }

            let noise = (NOISE_ESTIMATE_BIAS * *floor).max(f32::MIN_POSITIVE);
            let posterior = power / noise;
            let prior = NOISE_SNR_SMOOTHING
                * (self.previous_gain[bin] * self.previous_gain[bin] * self.previous_power[bin]
                    / noise)
                + (1.0 - NOISE_SNR_SMOOTHING) * (posterior - 1.0).max(0.0);
            let gain = (prior / (1.0 + prior)).clamp(NOISE_MIN_GAIN, 1.0);
            self.previous_gain[bin] = gain;
            self.previous_power[bin] = power;

            self.transform.real[bin] = real * gain;
            self.transform.imaginary[bin] = imaginary * gain;
            // The upper half of a real signal's spectrum is the conjugate of
            // the lower half, and the inverse transform needs both.
            if bin > 0 && bin < NOISE_TRANSFORM_SAMPLES / 2 {
                self.transform.real[NOISE_TRANSFORM_SAMPLES - bin] = self.transform.real[bin];
                self.transform.imaginary[NOISE_TRANSFORM_SAMPLES - bin] =
                    -self.transform.imaginary[bin];
            }
        }
        self.transform.run(TransformDirection::Inverse);

        // The first half of the synthesis window completes the overlap this
        // frame's predecessor left behind; the second half waits for its
        // successor. That is what delays the capture by exactly one frame.
        let (leading, trailing) = self.window.split_at(VOICE_FRAME_SAMPLES);
        let synthesized = self.transform.real.iter().zip(leading);
        for ((sample, overlap), (synthesized, window)) in frame
            .iter_mut()
            .zip(self.overlap.iter_mut())
            .zip(synthesized)
        {
            *sample = *overlap + synthesized * window;
        }
        let tail = self.transform.real[VOICE_FRAME_SAMPLES..]
            .iter()
            .zip(trailing);
        for (overlap, (synthesized, window)) in self.overlap.iter_mut().zip(tail) {
            *overlap = synthesized * window;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransformDirection {
    Forward,
    Inverse,
}

/// An in-place radix-2 discrete Fourier transform over
/// [`NOISE_TRANSFORM_SAMPLES`] points, with its twiddle factors built once.
#[derive(Debug)]
struct DiscreteTransform {
    cos: Box<[f32]>,
    sin: Box<[f32]>,
    real: Box<[f32]>,
    imaginary: Box<[f32]>,
}

impl DiscreteTransform {
    fn new() -> Self {
        let angle =
            |index: usize| std::f32::consts::TAU * index as f32 / NOISE_TRANSFORM_SAMPLES as f32;
        Self {
            cos: (0..NOISE_TRANSFORM_SAMPLES / 2)
                .map(|index| angle(index).cos())
                .collect(),
            sin: (0..NOISE_TRANSFORM_SAMPLES / 2)
                .map(|index| angle(index).sin())
                .collect(),
            real: vec![0.0; NOISE_TRANSFORM_SAMPLES].into_boxed_slice(),
            imaginary: vec![0.0; NOISE_TRANSFORM_SAMPLES].into_boxed_slice(),
        }
    }

    fn clear(&mut self) {
        self.real.fill(0.0);
        self.imaginary.fill(0.0);
    }

    fn run(&mut self, direction: TransformDirection) {
        let points = NOISE_TRANSFORM_SAMPLES;
        let mut target = 0;
        for index in 1..points {
            let mut bit = points >> 1;
            while target & bit != 0 {
                target ^= bit;
                bit >>= 1;
            }
            target |= bit;
            if index < target {
                self.real.swap(index, target);
                self.imaginary.swap(index, target);
            }
        }

        let mut span = 2;
        while span <= points {
            let half = span / 2;
            let stride = points / span;
            for start in (0..points).step_by(span) {
                for step in 0..half {
                    let twiddle = step * stride;
                    let cos = self.cos[twiddle];
                    let sin = match direction {
                        TransformDirection::Forward => -self.sin[twiddle],
                        TransformDirection::Inverse => self.sin[twiddle],
                    };
                    let (upper_real, upper_imaginary) =
                        (self.real[start + step], self.imaginary[start + step]);
                    let (lower_real, lower_imaginary) = (
                        self.real[start + step + half],
                        self.imaginary[start + step + half],
                    );
                    let (rotated_real, rotated_imaginary) = (
                        lower_real * cos - lower_imaginary * sin,
                        lower_real * sin + lower_imaginary * cos,
                    );
                    self.real[start + step] = upper_real + rotated_real;
                    self.imaginary[start + step] = upper_imaginary + rotated_imaginary;
                    self.real[start + step + half] = upper_real - rotated_real;
                    self.imaginary[start + step + half] = upper_imaginary - rotated_imaginary;
                }
            }
            span <<= 1;
        }

        if direction == TransformDirection::Inverse {
            let scale = 1.0 / points as f32;
            for (real, imaginary) in self.real.iter_mut().zip(self.imaginary.iter_mut()) {
                *real *= scale;
                *imaginary *= scale;
            }
        }
    }
}

/// Root mean square every talker is brought to: -18 dBFS leaves enough headroom
/// for the peaks of ordinary speech without ever reaching full scale.
const GAIN_TARGET_RMS: f32 = 0.126;
/// Below this the frame is a quiet room rather than a person, and the gain is
/// held where it is. Without this an empty room would be amplified until its
/// hiss was as loud as speech.
const GAIN_SPEECH_FLOOR_RMS: f32 = 0.003_16;
/// Per-frame share of the distance to the wanted gain. Coming down is quick, so
/// a shout is caught within a syllable; going up is slow, so the gain does not
/// pump between words.
const GAIN_FALL_RATE: f32 = 0.3;
const GAIN_RISE_RATE: f32 = 0.03;
/// 30 dB of lift for a distant microphone, 20 dB of cut for a loud one.
const GAIN_MIN: f32 = 0.1;
const GAIN_MAX: f32 = 31.6;
/// Loudest any sample may leave the stage, leaving the encoder a little room.
const GAIN_PEAK_CEILING: f32 = 0.99;

/// Brings every talker to the same loudness.
#[derive(Debug)]
struct AutomaticGainControl {
    gain: f32,
}

impl AutomaticGainControl {
    fn new() -> Self {
        Self { gain: 1.0 }
    }

    fn process(&mut self, frame: &mut [f32; VOICE_FRAME_SAMPLES]) {
        let mean_square =
            frame.iter().map(|sample| sample * sample).sum::<f32>() / VOICE_FRAME_SAMPLES as f32;
        let rms = mean_square.sqrt();
        let peak = frame
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let previous = self.gain;

        if rms > GAIN_SPEECH_FLOOR_RMS {
            let wanted = (GAIN_TARGET_RMS / rms).clamp(GAIN_MIN, GAIN_MAX);
            let rate = if wanted < self.gain {
                GAIN_FALL_RATE
            } else {
                GAIN_RISE_RATE
            };
            self.gain += (wanted - self.gain) * rate;
        }

        // The gain is ramped across the frame rather than stepped, so the
        // ceiling has to bind every point along the ramp: the loudest sample
        // may well sit at the start, where the previous frame's gain still
        // applies.
        let ceiling = if peak > 0.0 {
            GAIN_PEAK_CEILING / peak
        } else {
            f32::MAX
        };
        self.gain = self.gain.min(ceiling);
        for (index, sample) in frame.iter_mut().enumerate() {
            let progress = (index + 1) as f32 / VOICE_FRAME_SAMPLES as f32;
            *sample *= (previous + (self.gain - previous) * progress).min(ceiling);
        }
    }
}

fn frame_level(frame: &[f32; VOICE_FRAME_SAMPLES]) -> f32 {
    let mean_square = frame
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum::<f64>()
        / VOICE_FRAME_SAMPLES as f64;
    voice_level_from_rms(mean_square.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic stand-in for room noise. Nothing here reaches the
    /// simulation, so a plain congruential generator is all a repeatable test
    /// needs.
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

    #[test]
    fn processing_every_stage_off_leaves_the_captured_frame_exactly_as_it_was() {
        let switches = VoiceProcessingSwitches::new(VoiceProcessingConfig::DISABLED);
        let mut processing = VoiceProcessing::new(switches, None);
        let mut frame: [f32; VOICE_FRAME_SAMPLES] =
            std::array::from_fn(|index| (index as f32 / 40.0).sin() * 0.4);
        let untouched = frame;

        let level = processing.process(&mut frame);

        assert_eq!(frame, untouched);
        // A 0.4-amplitude tone is -11 dBFS, four fifths of the way up the
        // 60 dB window the level is spread over.
        assert!((level - 0.818).abs() < 0.005, "level was {level}");
    }

    #[test]
    fn the_transform_matches_a_direct_discrete_fourier_transform() {
        let mut transform = DiscreteTransform::new();
        let signal: Vec<f32> = (0..NOISE_TRANSFORM_SAMPLES)
            .map(|index| {
                let phase = index as f32;
                (phase * 0.07).sin() * 0.6 + (phase * 0.31).cos() * 0.25
            })
            .collect();
        transform.real.copy_from_slice(&signal);
        transform.imaginary.fill(0.0);

        transform.run(TransformDirection::Forward);

        for bin in [0_usize, 1, 7, 64, 511, 512] {
            let (mut real, mut imaginary) = (0.0_f64, 0.0_f64);
            for (index, sample) in signal.iter().enumerate() {
                let angle =
                    -std::f64::consts::TAU * (bin * index) as f64 / NOISE_TRANSFORM_SAMPLES as f64;
                real += f64::from(*sample) * angle.cos();
                imaginary += f64::from(*sample) * angle.sin();
            }
            assert!(
                (f64::from(transform.real[bin]) - real).abs() < 0.01
                    && (f64::from(transform.imaginary[bin]) - imaginary).abs() < 0.01,
                "bin {bin} was {} + {}i, direct sum gives {real} + {imaginary}i",
                transform.real[bin],
                transform.imaginary[bin],
            );
        }
    }

    #[test]
    fn noise_suppression_returns_a_frame_it_gates_nothing_from_one_frame_later() {
        let mut suppressor = NoiseSuppressor::new();
        let mut phase = 0.0_f32;
        let mut spoken = Vec::new();
        let mut sent = Vec::new();

        for _ in 0..16 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                phase += std::f32::consts::TAU * 700.0 / 16_000.0;
                *sample = 0.4 * phase.sin();
            }
            spoken.extend_from_slice(&frame);
            // A noise floor of zero is a signal the suppressor finds nothing
            // wrong with, which is what leaves its gains at one.
            suppressor.noise.fill(0.0);
            suppressor.process(&mut frame);
            sent.extend_from_slice(&frame);
        }

        // Skip the frames the noise floor is seeded over.
        let settled = 8 * VOICE_FRAME_SAMPLES;
        let error = spoken[settled..spoken.len() - VOICE_FRAME_SAMPLES]
            .iter()
            .zip(&sent[settled + VOICE_FRAME_SAMPLES..])
            .map(|(before, after)| (before - after).abs())
            .fold(0.0_f32, f32::max);
        // The window pair sums back to unity, so all that survives is the last
        // percent the Wiener gain holds back even at a boundless
        // signal-to-noise ratio. A mismatched window or an overlap a frame out
        // of step is off by two orders of magnitude more.
        assert!(
            error < 0.01,
            "the frame should come back one frame later unchanged, worst sample off by {error}",
        );
    }

    #[test]
    fn noise_suppression_quiets_a_steady_hiss() {
        let mut suppressor = NoiseSuppressor::new();
        let mut hiss = TestSignal(1);
        let mut sent = Vec::new();
        let mut heard = Vec::new();

        for index in 0..150 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                *sample = hiss.next() * 0.05;
            }
            let raw = frame;
            suppressor.process(&mut frame);
            if index >= 140 {
                heard.extend_from_slice(&raw);
                sent.extend_from_slice(&frame);
            }
        }

        let reduction = 20.0 * (rms(&heard) / rms(&sent)).log10();
        assert!(
            reduction >= 12.0,
            "a steady hiss should drop by at least 12 dB, got {reduction:.1} dB",
        );
    }

    #[test]
    fn noise_suppression_keeps_the_speech_that_starts_after_the_hiss() {
        let mut suppressor = NoiseSuppressor::new();
        let mut hiss = TestSignal(13);
        let mut phase = 0.0_f32;
        let mut spoken = Vec::new();
        let mut sent = Vec::new();

        for index in 0..160 {
            let talking = index >= 140;
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            let mut speech = [0.0; VOICE_FRAME_SAMPLES];
            for (offset, sample) in frame.iter_mut().enumerate() {
                phase += std::f32::consts::TAU * 300.0 / 16_000.0;
                speech[offset] = if talking {
                    0.25 * phase.sin() + 0.12 * (2.7 * phase).sin()
                } else {
                    0.0
                };
                *sample = hiss.next() * 0.02 + speech[offset];
            }
            suppressor.process(&mut frame);
            if index >= 143 {
                spoken.extend_from_slice(&speech);
                sent.extend_from_slice(&frame);
            }
        }

        let kept = rms(&sent) / rms(&spoken);
        assert!(
            kept > 0.8,
            "speech over the hiss must survive it, kept {kept:.2} of it",
        );
    }

    #[test]
    fn automatic_gain_control_lifts_a_quiet_talker_to_the_target() {
        let mut gain = AutomaticGainControl::new();
        let mut phase = 0.0_f32;
        let mut sent = Vec::new();

        for index in 0..250 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                phase += std::f32::consts::TAU * 300.0 / 16_000.0;
                *sample = 0.01 * phase.sin();
            }
            gain.process(&mut frame);
            if index >= 240 {
                sent.extend_from_slice(&frame);
            }
        }

        let level = rms(&sent);
        assert!(
            (level - GAIN_TARGET_RMS).abs() < 0.01,
            "a distant talker should arrive at the target level, got {level:.4}",
        );
    }

    #[test]
    fn automatic_gain_control_holds_its_gain_in_a_quiet_room() {
        let mut gain = AutomaticGainControl::new();
        let mut room = TestSignal(3);
        let mut sent = Vec::new();

        for index in 0..250 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                *sample = room.next() * 0.001;
            }
            gain.process(&mut frame);
            if index >= 240 {
                sent.extend_from_slice(&frame);
            }
        }

        assert_eq!(gain.gain, 1.0, "an empty room must not be amplified");
        assert!(rms(&sent) < 0.001);
    }

    #[test]
    fn automatic_gain_control_cannot_clip_while_its_gain_comes_back_down() {
        let mut gain = AutomaticGainControl::new();
        let mut phase = 0.0_f32;
        let mut quiet = |gain: &mut AutomaticGainControl| {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                phase += std::f32::consts::TAU * 300.0 / 16_000.0;
                *sample = 0.01 * phase.sin();
            }
            gain.process(&mut frame);
        };
        for _ in 0..250 {
            quiet(&mut gain);
        }
        assert!(
            gain.gain > 8.0,
            "the quiet talker earned a large gain first"
        );

        // The loudest sample sits at the very start of the frame, where the
        // ramp still carries the previous frame's much larger gain.
        let mut frame = [0.0; VOICE_FRAME_SAMPLES];
        frame[0] = 1.0;
        frame[1] = -1.0;
        gain.process(&mut frame);

        let peak = frame
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(
            peak <= GAIN_PEAK_CEILING,
            "a full-scale sample left the stage at {peak}, above the headroom the encoder is left",
        );
    }

    #[test]
    fn the_activation_level_is_measured_before_gain_control() {
        let switches = VoiceProcessingSwitches::new(VoiceProcessingConfig {
            echo_cancellation: false,
            noise_suppression: false,
            automatic_gain_control: true,
        });
        let mut processing = VoiceProcessing::new(switches, None);
        let mut phase = 0.0_f32;
        let mut level = 0.0;
        let mut sent = [0.0; VOICE_FRAME_SAMPLES];

        for _ in 0..250 {
            let mut frame = [0.0; VOICE_FRAME_SAMPLES];
            for sample in frame.iter_mut() {
                phase += std::f32::consts::TAU * 300.0 / 16_000.0;
                *sample = 0.01 * phase.sin();
            }
            level = processing.process(&mut frame);
            sent = frame;
        }

        // -43 dBFS in, close to the -18 dBFS target out. A gate reading the
        // amplified frame could not tell this apart from someone shouting.
        assert!((level - 0.283).abs() < 0.01, "level was {level}");
        assert!(
            frame_level(&sent) > 0.6,
            "the frame itself was amplified to {}",
            frame_level(&sent),
        );
    }

    #[test]
    fn the_echo_stage_cancels_only_while_its_own_switch_is_on() {
        use crate::voice_echo::VoiceEchoTap;

        let switches = VoiceProcessingSwitches::new(VoiceProcessingConfig {
            echo_cancellation: true,
            ..VoiceProcessingConfig::DISABLED
        });
        let mut tap = VoiceEchoTap::new(16_000);
        let mut processing = VoiceProcessing::new(switches.clone(), Some(tap.reference()));
        let mut signal = TestSignal(31);
        let delay = 500;
        let mut played = vec![0.0; delay + VOICE_FRAME_SAMPLES];
        let mut heard = Vec::new();
        let mut sent = Vec::new();

        // The same room the canceller's own tests use: the microphone hears
        // half of what the speakers played, 500 samples ago.
        let mut speak =
            |processing: &mut VoiceProcessing, played: &mut Vec<f32>, signal: &mut TestSignal| {
                let mut frame = [0.0; VOICE_FRAME_SAMPLES];
                for sample in frame.iter_mut() {
                    *sample = signal.next() * 0.3;
                    tap.push_output_frame(*sample, *sample);
                }
                played.extend_from_slice(&frame);
                let mut microphone = [0.0; VOICE_FRAME_SAMPLES];
                for (offset, sample) in microphone.iter_mut().enumerate() {
                    *sample = 0.5 * played[played.len() - VOICE_FRAME_SAMPLES + offset - delay];
                }
                let raw = microphone;
                processing.process(&mut microphone);
                (raw, microphone)
            };

        for index in 0..400 {
            let (raw, processed) = speak(&mut processing, &mut played, &mut signal);
            if index >= 390 {
                heard.extend_from_slice(&raw);
                sent.extend_from_slice(&processed);
            }
        }
        assert!(
            rms(&sent) < rms(&heard) * 0.1,
            "with the switch on the chain cancels the echo",
        );

        switches.set(VoiceProcessingConfig::DISABLED);
        let (raw, processed) = speak(&mut processing, &mut played, &mut signal);

        assert_eq!(
            processed, raw,
            "with it off the chain hands the microphone through untouched",
        );
    }

    #[test]
    fn a_stage_switched_off_stops_running_on_the_next_frame() {
        let switches = VoiceProcessingSwitches::new(VoiceProcessingConfig::default());
        let mut processing = VoiceProcessing::new(switches.clone(), None);
        let mut hiss = TestSignal(5);
        let mut frame: [f32; VOICE_FRAME_SAMPLES] = std::array::from_fn(|_| hiss.next() * 0.05);
        let heard = frame;

        processing.process(&mut frame);
        assert_ne!(frame, heard, "the chain was running");

        switches.set(VoiceProcessingConfig::DISABLED);
        let mut frame = heard;
        processing.process(&mut frame);

        assert_eq!(
            frame, heard,
            "switching the stages off takes effect without reopening the microphone",
        );
    }
}
