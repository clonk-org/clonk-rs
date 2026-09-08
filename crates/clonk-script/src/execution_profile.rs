//! Opt-in accounting for compiled C4Script execution and AST fallbacks.
//!
//! A static count of source constructs cannot identify a useful lowering
//! target: one tiny callback can run thousands of times while a large setup
//! function runs once. These counters are updated at the invocation boundary
//! and attribute every AST invocation to all syntax families that prevented
//! its function from lowering. Reason counts therefore overlap deliberately.
//!
//! Recording is compiled out of shipped builds. Enable `execution-profile`
//! only for the manual real-content engine probe.

use std::cell::Cell;
use std::fmt;

/// Syntax or signature families that can keep a function on the AST VM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AstFallbackReason {
    ReferenceSignature,
    ClassicFor,
    Foreach,
    LoopControl,
    ComplexAssignment,
    DynamicIndex,
    MethodOrOptionalCall,
    SpecialOrForwardedCall,
    UnsupportedOperator,
    LegacyOrGlobalCall,
    ParseError,
    Other,
}

impl AstFallbackReason {
    pub const ALL: [Self; 12] = [
        Self::ReferenceSignature,
        Self::ClassicFor,
        Self::Foreach,
        Self::LoopControl,
        Self::ComplexAssignment,
        Self::DynamicIndex,
        Self::MethodOrOptionalCall,
        Self::SpecialOrForwardedCall,
        Self::UnsupportedOperator,
        Self::LegacyOrGlobalCall,
        Self::ParseError,
        Self::Other,
    ];

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::ReferenceSignature => 0,
            Self::ClassicFor => 1,
            Self::Foreach => 2,
            Self::LoopControl => 3,
            Self::ComplexAssignment => 4,
            Self::DynamicIndex => 5,
            Self::MethodOrOptionalCall => 6,
            Self::SpecialOrForwardedCall => 7,
            Self::UnsupportedOperator => 8,
            Self::LegacyOrGlobalCall => 9,
            Self::ParseError => 10,
            Self::Other => 11,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ReferenceSignature => "reference_signature",
            Self::ClassicFor => "classic_for",
            Self::Foreach => "foreach",
            Self::LoopControl => "break_or_continue",
            Self::ComplexAssignment => "complex_assignment",
            Self::DynamicIndex => "dynamic_index",
            Self::MethodOrOptionalCall => "method_or_optional_call",
            Self::SpecialOrForwardedCall => "special_or_forwarded_call",
            Self::UnsupportedOperator => "concat_or_nil_coalescing",
            Self::LegacyOrGlobalCall => "legacy_or_global_call",
            Self::ParseError => "parse_error",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for AstFallbackReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// One invocation-window snapshot. Fallback-reason counts overlap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptExecutionProfile {
    pub compiled: u64,
    pub ast_without_plan: u64,
    pub ast_after_runtime_guard: u64,
    reasons: [u64; AstFallbackReason::ALL.len()],
    sole_blockers: [u64; AstFallbackReason::ALL.len()],
}

impl ScriptExecutionProfile {
    pub fn total_invocations(&self) -> u64 {
        self.compiled
            .saturating_add(self.ast_without_plan)
            .saturating_add(self.ast_after_runtime_guard)
    }

    pub fn reason(&self, reason: AstFallbackReason) -> u64 {
        self.reasons[reason.index()]
    }

    /// Invocations that this family alone kept on the AST VM: the mass a
    /// lowering change for that one family could convert.
    pub fn sole_blocker(&self, reason: AstFallbackReason) -> u64 {
        self.sole_blockers[reason.index()]
    }

    pub fn ranked_reasons(&self) -> Vec<(AstFallbackReason, u64)> {
        ranked(&self.reasons)
    }

    pub fn ranked_sole_blockers(&self) -> Vec<(AstFallbackReason, u64)> {
        ranked(&self.sole_blockers)
    }
}

/// Non-zero families, largest count first, ties in declaration order.
fn ranked(counts: &[u64; AstFallbackReason::ALL.len()]) -> Vec<(AstFallbackReason, u64)> {
    let mut ranked = AstFallbackReason::ALL
        .into_iter()
        .map(|reason| (reason, counts[reason.index()]))
        .filter(|(_, count)| *count != 0)
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    ranked
}

impl fmt::Display for ScriptExecutionProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "compiled={} ast_without_plan={} ast_after_runtime_guard={} total={}",
            self.compiled,
            self.ast_without_plan,
            self.ast_after_runtime_guard,
            self.total_invocations(),
        )?;
        for (reason, count) in self.ranked_reasons() {
            writeln!(formatter, "{reason}: {count}")?;
        }
        for (reason, count) in self.ranked_sole_blockers() {
            writeln!(formatter, "sole {reason}: {count}")?;
        }
        Ok(())
    }
}

thread_local! {
    static PROFILE: Cell<ScriptExecutionProfile> = const { Cell::new(ScriptExecutionProfile {
        compiled: 0,
        ast_without_plan: 0,
        ast_after_runtime_guard: 0,
        reasons: [0; AstFallbackReason::ALL.len()],
        sole_blockers: [0; AstFallbackReason::ALL.len()],
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

#[inline(always)]
#[cfg(any(test, feature = "execution-profile"))]
pub(crate) fn record_ast_without_plan(reasons: &[AstFallbackReason]) {
    PROFILE.with(|profile| {
        let mut current = profile.get();
        current.ast_without_plan = current.ast_without_plan.saturating_add(1);
        for reason in reasons {
            let counter = &mut current.reasons[reason.index()];
            *counter = counter.saturating_add(1);
        }
        if let [sole] = reasons {
            let counter = &mut current.sole_blockers[sole.index()];
            *counter = counter.saturating_add(1);
        }
        profile.set(current);
    });
}

#[inline(always)]
pub(crate) fn record_ast_after_runtime_guard() {
    #[cfg(any(test, feature = "execution-profile"))]
    PROFILE.with(|profile| {
        let mut current = profile.get();
        current.ast_after_runtime_guard = current.ast_after_runtime_guard.saturating_add(1);
        profile.set(current);
    });
}

/// Which path an execution interval belongs to. An AST interval carries the
/// family that alone kept its function on the AST VM, when there is exactly
/// one, so its time can be charged to that family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionKind {
    Compiled,
    Ast(Option<AstFallbackReason>),
}

/// Exclusive execution intervals, including native host work but excluding
/// nested script bodies. This bounds interpreter cost; it is not pure VM time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecutionTiming {
    pub compiled_ns: u64,
    pub ast_ns: u64,
    ast_sole_blocker_ns: [u64; AstFallbackReason::ALL.len()],
}

impl ExecutionTiming {
    /// The part of `ast_ns` spent in invocations that this family alone kept
    /// on the AST VM. Invocations with several blockers, and resumed
    /// continuations, stay in the total only.
    pub fn ast_sole_blocker_ns(&self, reason: AstFallbackReason) -> u64 {
        self.ast_sole_blocker_ns[reason.index()]
    }

    pub fn ranked_ast_sole_blocker_ns(&self) -> Vec<(AstFallbackReason, u64)> {
        ranked(&self.ast_sole_blocker_ns)
    }
}

#[cfg(any(test, feature = "execution-profile"))]
impl ExecutionTiming {
    fn enter(self, now: u64) -> (u64, u64) {
        (now, self.compiled_ns.saturating_add(self.ast_ns))
    }

    fn exit(&mut self, (started, children_before): (u64, u64), now: u64, kind: ExecutionKind) {
        let children = self
            .compiled_ns
            .saturating_add(self.ast_ns)
            .saturating_sub(children_before);
        let exclusive = now.saturating_sub(started).saturating_sub(children);
        match kind {
            ExecutionKind::Compiled => {
                self.compiled_ns = self.compiled_ns.saturating_add(exclusive);
            }
            ExecutionKind::Ast(sole_blocker) => {
                self.ast_ns = self.ast_ns.saturating_add(exclusive);
                if let Some(reason) = sole_blocker {
                    let counter = &mut self.ast_sole_blocker_ns[reason.index()];
                    *counter = counter.saturating_add(exclusive);
                }
            }
        }
    }
}

thread_local! {
    static TIMING: Cell<ExecutionTiming> = const { Cell::new(ExecutionTiming {
        compiled_ns: 0,
        ast_ns: 0,
        ast_sole_blocker_ns: [0; AstFallbackReason::ALL.len()],
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
    pub(crate) fn enter(_kind: ExecutionKind) -> Option<Self> {
        None
    }
}

#[cfg(feature = "execution-profile")]
pub(crate) struct ExecutionTimer {
    started: std::time::Instant,
    interval: (u64, u64),
    kind: ExecutionKind,
}

#[cfg(feature = "execution-profile")]
impl ExecutionTimer {
    pub(crate) fn enter(kind: ExecutionKind) -> Option<Self> {
        TIMING_ENABLED.with(Cell::get).then(|| Self {
            started: std::time::Instant::now(),
            interval: TIMING.with(|timing| timing.get().enter(0)),
            kind,
        })
    }
}

#[cfg(feature = "execution-profile")]
impl Drop for ExecutionTimer {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        TIMING.with(|timing| {
            let mut current = timing.get();
            current.exit(self.interval, elapsed, self.kind);
            timing.set(current);
        });
    }
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    #[test]
    fn nested_execution_time_is_charged_only_to_its_own_path() {
        let mut timing = ExecutionTiming::default();
        let outer = timing.enter(10);
        let inner = timing.enter(20);
        timing.exit(inner, 50, ExecutionKind::Ast(None));
        timing.exit(outer, 90, ExecutionKind::Compiled);
        assert_eq!(timing.ast_ns, 30);
        assert_eq!(timing.compiled_ns, 50);
    }

    #[test]
    fn sole_blocker_ast_time_is_charged_to_its_family() {
        let mut timing = ExecutionTiming::default();
        let outer = timing.enter(0);
        let inner = timing.enter(10);
        timing.exit(inner, 40, ExecutionKind::Compiled);
        timing.exit(
            outer,
            100,
            ExecutionKind::Ast(Some(AstFallbackReason::Foreach)),
        );
        assert_eq!(timing.compiled_ns, 30);
        assert_eq!(timing.ast_ns, 70);
        assert_eq!(timing.ast_sole_blocker_ns(AstFallbackReason::Foreach), 70);
        assert_eq!(
            timing.ast_sole_blocker_ns(AstFallbackReason::LoopControl),
            0
        );
    }

    #[test]
    fn unattributed_ast_time_stays_in_the_total_only() {
        let mut timing = ExecutionTiming::default();
        let window = timing.enter(0);
        timing.exit(window, 25, ExecutionKind::Ast(None));
        assert_eq!(timing.ast_ns, 25);
        assert!(AstFallbackReason::ALL
            .into_iter()
            .all(|reason| timing.ast_sole_blocker_ns(reason) == 0));
    }
}

#[cfg(test)]
mod sole_blocker_tests {
    use super::*;

    #[test]
    fn only_single_reason_invocations_count_as_sole_blockers() {
        reset();
        record_ast_without_plan(&[AstFallbackReason::Foreach]);
        record_ast_without_plan(&[AstFallbackReason::Foreach, AstFallbackReason::LoopControl]);
        let profile = snapshot();
        assert_eq!(profile.reason(AstFallbackReason::Foreach), 2);
        assert_eq!(profile.reason(AstFallbackReason::LoopControl), 1);
        assert_eq!(profile.sole_blocker(AstFallbackReason::Foreach), 1);
        assert_eq!(profile.sole_blocker(AstFallbackReason::LoopControl), 0);
    }

    #[test]
    fn display_ranks_sole_blockers_beside_the_overlapping_counts() {
        reset();
        record_ast_without_plan(&[AstFallbackReason::Foreach]);
        record_ast_without_plan(&[AstFallbackReason::ComplexAssignment]);
        record_ast_without_plan(&[AstFallbackReason::ComplexAssignment]);
        let profile = snapshot();
        assert_eq!(
            profile.ranked_sole_blockers(),
            vec![
                (AstFallbackReason::ComplexAssignment, 2),
                (AstFallbackReason::Foreach, 1),
            ]
        );
        let text = profile.to_string();
        assert!(text.contains("sole complex_assignment: 2\n"), "{text}");
        assert!(text.contains("sole foreach: 1\n"), "{text}");
    }
}
