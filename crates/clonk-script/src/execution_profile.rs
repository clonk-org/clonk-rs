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

    pub fn ranked_reasons(&self) -> Vec<(AstFallbackReason, u64)> {
        let mut ranked = AstFallbackReason::ALL
            .into_iter()
            .map(|reason| (reason, self.reason(reason)))
            .filter(|(_, count)| *count != 0)
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
        ranked
    }
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
        Ok(())
    }
}

thread_local! {
    static PROFILE: Cell<ScriptExecutionProfile> = const { Cell::new(ScriptExecutionProfile {
        compiled: 0,
        ast_without_plan: 0,
        ast_after_runtime_guard: 0,
        reasons: [0; AstFallbackReason::ALL.len()],
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
