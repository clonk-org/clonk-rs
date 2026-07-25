//! Sizing the lockstep lookahead from observed control delivery time.
//!
//! C++ `C4GameControlNetwork::CalcPerformance`
//! (oracle-src-pinned src/C4GameControlNetwork.cpp:382-435) derives PreSend
//! from one number: a 1/150 EWMA of the *mean* control send time. A client
//! stalls whenever a control packet misses its execution slot, and that is
//! governed by the upper tail of the delivery distribution, not by its mean.
//! On a link whose delivery time varies, the mean-only budget therefore sits
//! permanently below the value that would avoid stalling.
//!
//! This estimator instead tracks a *decaying peak envelope*: it jumps straight
//! to any sample above the current value and decays back down on C++'s slow
//! constant. Rising immediately is what matters, because until the horizon
//! covers the link every single control tick stalls, whereas an over-large
//! horizon only costs input latency. A mean-absolute-deviation term adds a
//! small margin on top for the variation the envelope has already decayed past.
//!
//! On a steady link the envelope equals the mean and the deviation collapses to
//! zero, so the budget converges on exactly what C++ would have chosen and
//! connections that are not stalling today keep the lookahead they have.

use std::time::Duration;

/// C++'s EWMA weight: `avg = (avg * 149 + sample) / 150`.
const EWMA_RETAINED: i32 = 149;
const EWMA_DIVISOR: i32 = 150;

/// Margin applied to the deviation, chosen by measurement rather than by
/// analogy: `cargo run -p clonk-network --example link_impairment` with
/// `LC_PRESEND=adaptive LC_DUP=2` over 16 seeds x 5 link profiles puts the knee
/// of the frozen-time/input-latency tradeoff at 1. Raising it to RFC 6298's 4
/// bought 0.02 percentage points of frozen time on a typical link and charged
/// 65% more input latency for it (114ms -> 188ms).
const DEVIATION_WEIGHT: i32 = 1;

/// Tracks control delivery time and reports the lookahead needed to absorb it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ControlLatencyEstimator {
    /// Decaying peak of observed delivery time, not an average.
    envelope_us: i32,
    deviation_us: i32,
    seeded: bool,
}

impl ControlLatencyEstimator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold in one control send time sample.
    ///
    /// The first sample seeds the estimator outright instead of dragging the
    /// EWMA up from zero. C++ starts at zero and needs ~150 samples — about
    /// eight seconds at ControlRate 2 — before the budget reflects the link at
    /// all, and it stalls on every control tick for that whole span.
    pub fn observe(&mut self, sample_ms: i32) {
        let sample_us = sample_ms.saturating_mul(1_000);
        if !self.seeded {
            self.seeded = true;
            self.envelope_us = sample_us;
            // Start with no variance premium. RFC 6298 seeds this at half the
            // sample because a retransmission timeout must never fire early,
            // but here the premium is paid as input latency on every frame and
            // one sample is no evidence of variation. The envelope's immediate
            // attack covers a rise on its own, and the deviation calibrates
            // within a few ticks once there is something to measure.
            self.deviation_us = 0;
            return;
        }

        let error_us = sample_us.saturating_sub(self.envelope_us);
        if error_us > 0 {
            // Attack immediately. Until the horizon covers the link, every
            // control tick stalls; an over-large horizon merely costs latency.
            // The two are not symmetric, so the estimator should not be either.
            //
            // Only this direction feeds the deviation. A sample *below* the
            // envelope never caused a stall, and charging latency for it would
            // leave a premium behind long after a link had recovered.
            self.deviation_us = ewma(self.deviation_us, error_us);
            self.envelope_us = sample_us;
        } else {
            // Decay on C++'s constant so one spike cannot pin the horizon high.
            self.deviation_us = ewma(self.deviation_us, 0);
            self.envelope_us = ewma(self.envelope_us, sample_us);
        }
    }

    /// Delivery-time budget the lookahead has to cover, in microseconds.
    pub fn budget_us(self) -> i32 {
        self.envelope_us
            .saturating_add(self.deviation_us.saturating_mul(DEVIATION_WEIGHT))
    }

    pub fn budget(self) -> Duration {
        Duration::from_micros(self.budget_us().max(0) as u64)
    }
}

fn ewma(previous_us: i32, sample_us: i32) -> i32 {
    previous_us
        .wrapping_mul(EWMA_RETAINED)
        .wrapping_add(sample_us)
        / EWMA_DIVISOR
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property that keeps healthy links untouched: with no variance the
    /// deviation term decays away and the budget settles on the plain mean,
    /// which is exactly what C++ would have budgeted.
    #[test]
    fn steady_link_budget_converges_to_the_mean() {
        let mut estimator = ControlLatencyEstimator::new();
        for _ in 0..600 {
            estimator.observe(40);
        }
        assert_eq!(estimator.envelope_us, 40_000);
        assert!(
            estimator.deviation_us < 400,
            "a steady link must not carry a deviation premium, got {}us",
            estimator.deviation_us
        );
        assert!(
            (39_000..=42_000).contains(&estimator.budget_us()),
            "budget {}us should sit on the 40ms mean",
            estimator.budget_us()
        );
    }

    /// The property that fixes jittery links: the budget has to clear the
    /// samples, not sit in the middle of them where half of them stall.
    #[test]
    fn jittery_link_budget_clears_the_upper_tail() {
        let mut estimator = ControlLatencyEstimator::new();
        // Mean 60ms, excursions to 120ms — the shape a congested link has.
        let samples = [40, 60, 120, 45, 55, 110, 50, 65, 125, 42];
        for _ in 0..60 {
            for sample in samples {
                estimator.observe(sample);
            }
        }
        let budget_ms = estimator.budget_us() / 1_000;
        assert!(
            budget_ms >= 120,
            "budget {budget_ms}ms must cover the 120ms tail, not the 67ms mean"
        );
    }

    /// C++ needs ~150 samples to notice a link got slower and stalls on every
    /// control tick until it does.
    #[test]
    fn budget_reaches_a_step_change_within_a_few_samples() {
        let mut estimator = ControlLatencyEstimator::new();
        for _ in 0..200 {
            estimator.observe(30);
        }
        estimator.observe(150);
        assert!(
            estimator.budget_us() >= 150_000,
            "one 150ms sample must already be covered, got {}us",
            estimator.budget_us()
        );
    }

    /// A single outlier must not pin the budget high forever, or the cure
    /// becomes permanent input latency.
    #[test]
    fn a_lone_spike_decays_back_down() {
        let mut estimator = ControlLatencyEstimator::new();
        for _ in 0..200 {
            estimator.observe(30);
        }
        estimator.observe(400);
        let spiked = estimator.budget_us();
        for _ in 0..600 {
            estimator.observe(30);
        }
        assert!(
            estimator.budget_us() < spiked / 2,
            "budget {}us should have decayed well below the {spiked}us spike",
            estimator.budget_us()
        );
    }

    #[test]
    fn the_first_sample_seeds_instead_of_ramping_from_zero() {
        let mut estimator = ControlLatencyEstimator::new();
        estimator.observe(80);
        assert_eq!(estimator.envelope_us, 80_000);
        assert_eq!(estimator.budget(), Duration::from_micros(80_000));
    }
}
