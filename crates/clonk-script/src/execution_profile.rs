//! Opt-in invocation counts and execution timing for the C4Script bytecode VM.
//! Recording is compiled out of shipped builds; enable `execution-profile`
//! for the manual real-content engine probe.

use std::cell::Cell;
use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptExecutionProfile {
    pub compiled: u64,
}

impl ScriptExecutionProfile {
    pub fn total_invocations(&self) -> u64 {
        self.compiled
    }
}

impl fmt::Display for ScriptExecutionProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "compiled={}", self.compiled)
    }
}

thread_local! {
    static PROFILE: Cell<ScriptExecutionProfile> = const { Cell::new(ScriptExecutionProfile {
        compiled: 0,
    }) };
}

pub fn reset() {
    PROFILE.with(|profile| profile.set(ScriptExecutionProfile::default()));
}

pub fn snapshot() -> ScriptExecutionProfile {
    PROFILE.with(Cell::get)
}

#[inline(always)]
pub(crate) fn record_compiled() {
    #[cfg(any(test, feature = "execution-profile"))]
    PROFILE.with(|profile| {
        let mut current = profile.get();
        current.compiled = current.compiled.saturating_add(1);
        profile.set(current);
    });
}

/// Execution intervals include native host work. Nested script bodies are
/// counted once, so this is elapsed script time rather than pure VM time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutionTiming {
    pub compiled_ns: u64,
}

#[cfg(any(test, feature = "execution-profile"))]
impl ExecutionTiming {
    fn enter(self, now: u64) -> (u64, u64) {
        (now, self.compiled_ns)
    }

    fn exit(&mut self, (started, children_before): (u64, u64), now: u64) {
        let children = self.compiled_ns.saturating_sub(children_before);
        let exclusive = now.saturating_sub(started).saturating_sub(children);
        self.compiled_ns = self.compiled_ns.saturating_add(exclusive);
    }
}

thread_local! {
    static TIMING: Cell<ExecutionTiming> = const { Cell::new(ExecutionTiming {
        compiled_ns: 0,
    }) };
    static TIMING_ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// Start a fresh timing window, or disable timing. Call outside script execution.
/// Requires `execution-profile`; normal builds never read the clock in the VM.
pub fn set_timing_enabled(enabled: bool) {
    TIMING.with(|timing| timing.set(ExecutionTiming::default()));
    TIMING_ENABLED.with(|current| current.set(enabled));
}

pub fn timing_snapshot() -> ExecutionTiming {
    TIMING.with(Cell::get)
}

/// Shipped builds never read the clock in the VM: entering a timer is a
/// constant `None`, so the call sites carry no `cfg` of their own.
#[cfg(not(feature = "execution-profile"))]
pub(crate) struct ExecutionTimer;

#[cfg(not(feature = "execution-profile"))]
impl ExecutionTimer {
    #[inline(always)]
    pub(crate) fn enter() -> Option<Self> {
        None
    }
}

#[cfg(feature = "execution-profile")]
pub(crate) struct ExecutionTimer {
    started: std::time::Instant,
    interval: (u64, u64),
}

#[cfg(feature = "execution-profile")]
impl ExecutionTimer {
    pub(crate) fn enter() -> Option<Self> {
        TIMING_ENABLED.with(Cell::get).then(|| Self {
            started: std::time::Instant::now(),
            interval: TIMING.with(|timing| timing.get().enter(0)),
        })
    }
}

#[cfg(feature = "execution-profile")]
impl Drop for ExecutionTimer {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        TIMING.with(|timing| {
            let mut current = timing.get();
            current.exit(self.interval, elapsed);
            timing.set(current);
        });
    }
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    #[test]
    fn nested_execution_time_is_counted_once() {
        let mut timing = ExecutionTiming::default();
        let outer = timing.enter(10);
        let inner = timing.enter(20);
        timing.exit(inner, 50);
        assert_eq!(timing.compiled_ns, 30);
        timing.exit(outer, 90);
        assert_eq!(timing.compiled_ns, 80);
    }
}
