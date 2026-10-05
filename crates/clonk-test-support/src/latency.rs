//! Wall-clock product budgets, separate from correctness and hang deadlines.

use std::time::Duration;

/// Validate a complete, fresh fixture and return only its measured interval.
/// Pass no budget when instrumentation makes the timing unrepresentative.
///
/// A passing sample stops immediately. Only a completed, correct sample that
/// exceeds the inclusive budget is repeated, at most three times in total.
/// Assertions and progress deadlines inside the fixture propagate immediately;
/// this helper never catches panics or retries functional failures. Each sample
/// must own its fixture so a repeat does not reuse the previous run's state.
#[track_caller]
pub fn assert_latency_budget(
    name: &str,
    budget: Option<Duration>,
    mut sample: impl FnMut() -> Duration,
) {
    let Some(budget) = budget else {
        sample();
        return;
    };
    let mut elapsed_samples = [Duration::ZERO; 3];
    for (index, elapsed) in elapsed_samples.iter_mut().enumerate() {
        *elapsed = sample();
        eprintln!(
            "{name}: latency sample {}/3 = {elapsed:?}; budget = {budget:?}",
            index + 1
        );
        if *elapsed <= budget {
            return;
        }
    }
    panic!("{name} exceeded the inclusive {budget:?} latency budget in every completed sample: {elapsed_samples:?}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_late_sample_gets_a_fresh_attempt() {
        let mut samples = [Duration::from_millis(1001), Duration::from_millis(999)].into_iter();
        let mut attempts = 0;
        assert_latency_budget(
            "reference publication",
            Some(Duration::from_secs(1)),
            || {
                attempts += 1;
                samples
                    .next()
                    .expect("only completed slow samples may repeat")
            },
        );
        assert_eq!(attempts, 2);
        assert_eq!(samples.next(), None);
    }

    #[test]
    fn persistent_latency_regression_fails_after_three_completed_samples() {
        let mut attempts = 0;
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_latency_budget(
                "reference publication",
                Some(Duration::from_secs(1)),
                || {
                    attempts += 1;
                    Duration::from_millis(1000 + attempts)
                },
            );
        }))
        .expect_err("a repeat must not relax the budget");
        assert_eq!(attempts, 3);
        let diagnostic = failure
            .downcast_ref::<String>()
            .expect("latency failure message");
        assert!(diagnostic.contains("reference publication"), "{diagnostic}");
        assert!(
            diagnostic.contains("inclusive 1s latency budget"),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("[1.001s, 1.002s, 1.003s]"),
            "{diagnostic}"
        );
    }

    #[test]
    fn inclusive_budget_passes_without_an_extra_sample() {
        let mut attempts = 0;
        assert_latency_budget(
            "reference publication",
            Some(Duration::from_secs(1)),
            || {
                attempts += 1;
                Duration::from_secs(1)
            },
        );
        assert_eq!(attempts, 1);
    }

    #[test]
    fn instrumentation_runs_correctness_once_without_enforcing_latency() {
        let mut attempts = 0;
        assert_latency_budget("reference publication", None, || {
            attempts += 1;
            Duration::from_secs(60)
        });
        assert_eq!(attempts, 1);
    }

    #[test]
    fn functional_failure_after_a_late_sample_also_stops_immediately() {
        let mut attempts = 0;
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_latency_budget(
                "reference publication",
                Some(Duration::from_secs(1)),
                || {
                    attempts += 1;
                    assert_ne!(attempts, 2, "progress deadline expired");
                    Duration::from_millis(1001)
                },
            );
        }))
        .expect_err("a late sample must not make later failures retryable");
        assert_eq!(attempts, 2);
        let diagnostic = failure
            .downcast_ref::<String>()
            .expect("functional failure message");
        assert!(
            diagnostic.contains("progress deadline expired"),
            "{diagnostic}"
        );
    }

    #[test]
    fn functional_failure_propagates_without_a_retry() {
        let mut attempts = 0;
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_latency_budget(
                "reference publication",
                Some(Duration::from_secs(1)),
                || {
                    attempts += 1;
                    panic!("reference is incorrect");
                },
            );
        }))
        .expect_err("functional failure must propagate");
        assert_eq!(attempts, 1);
        assert_eq!(
            failure.downcast_ref::<&str>(),
            Some(&"reference is incorrect")
        );
    }
}
