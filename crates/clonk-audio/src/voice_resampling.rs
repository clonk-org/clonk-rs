//! Prepared interpolation kernels for voice and device sample rates.

pub(crate) const TAPS: usize = 64;
pub(crate) const PHASES: usize = 128;

/// Prepared off the hardware callback. The capture worker uses a bounded
/// windowed-sinc conversion, so 96/192 kHz output cannot alias into the AEC.
pub(crate) fn coefficients(source_rate: u32, output_rate: u32) -> Box<[[f32; TAPS]]> {
    let cutoff = if source_rate <= output_rate {
        0.5
    } else {
        0.48 * f64::from(output_rate) / f64::from(source_rate)
    };
    (0..=PHASES)
        .map(|phase| {
            let mut coefficients = std::array::from_fn::<_, TAPS, _>(|index| {
                let offset = index as f64 - (TAPS / 2 - 1) as f64 - phase as f64 / PHASES as f64;
                let sinc = if offset.abs() < f64::EPSILON {
                    2.0 * cutoff
                } else {
                    (std::f64::consts::TAU * cutoff * offset).sin()
                        / (std::f64::consts::PI * offset)
                };
                let window_phase = std::f64::consts::TAU * index as f64 / (TAPS - 1) as f64;
                (sinc * (0.42 - 0.5 * window_phase.cos() + 0.08 * (2.0 * window_phase).cos()))
                    as f32
            });
            let sum = coefficients.iter().sum::<f32>();
            for coefficient in &mut coefficients {
                *coefficient /= sum;
            }
            coefficients
        })
        .collect()
}

/// Causal 64-tap interpolation with 31 source samples of delay. History is
/// primed to the first sample so a constant input has no startup amplitude dip.
/// Coefficients and storage are prepared before processing; each output has
/// bounded work and interpolation between phases avoids quantizing pitch.
#[derive(Debug)]
pub(crate) struct SincHistory {
    coefficients: Box<[[f32; TAPS]]>,
    history: [f32; TAPS],
    newest: usize,
    primed: bool,
}

impl SincHistory {
    pub(crate) fn new(source_rate: u32, output_rate: u32) -> Self {
        Self {
            coefficients: coefficients(source_rate, output_rate),
            history: [0.0; TAPS],
            newest: TAPS - 1,
            primed: false,
        }
    }

    pub(crate) fn push(&mut self, sample: f32) {
        if !self.primed {
            self.history.fill(sample);
            self.primed = true;
        }
        self.newest = (self.newest + 1) % TAPS;
        self.history[self.newest] = sample;
    }

    pub(crate) fn interpolate(&self, fraction: f64) -> f32 {
        let phase = fraction.clamp(0.0, 1.0) * PHASES as f64;
        let index = (phase as usize).min(PHASES - 1);
        let blend = (phase - index as f64) as f32;
        let (early, late) = self.history.split_at((self.newest + 1) % TAPS);
        self.coefficients[index]
            .iter()
            .zip(&self.coefficients[index + 1])
            .zip(late.iter().chain(early))
            .map(|((a, b), sample)| (a + (b - a) * blend) * sample)
            .sum()
    }
}
