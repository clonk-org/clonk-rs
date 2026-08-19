//! Per-family accounting for the VM's string-keyed identifier lookups.
//!
//! The C4Script runtime still resolves functions, constants, locals and
//! callback names through owned `String` keys, so a call pays a hash and a
//! comparison for every one. Deciding whether to intern those identifiers
//! needs a measurement of *which* families are material, not a single
//! aggregate: the compiled executor already removed some of them, and a
//! percentage recorded before that says nothing about the code as it stands.
//!
//! Recording is compiled out of a shipped VM: it is live only for this
//! crate's own tests, and behind the `lookup-profile` feature for a trace
//! driven from the engine. The counters sit on the hottest paths there are,
//! so leaving them in would both cost real time and perturb the very
//! measurement they exist to take.

use std::cell::{Cell, RefCell};
use std::fmt;

/// A family of identifier lookup, counted separately so a go/no-go decision
/// can name which one it is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LookupFamily {
    /// A named function in a script's own scope, its owner list, or the
    /// engine's global table.
    ScriptFunction,
    /// A host (native) function or host reference function.
    HostFunction,
    /// A script or global constant.
    Constant,
    /// A named entry in the engine's global variable table.
    Global,
    /// A named local, function variable, or parameter.
    Local,
    /// Definition metadata reached by name. Recorded by the host bridge,
    /// which is where definitions live.
    Definition,
    /// An effect callback name, including the ones built at runtime.
    EffectCallback,
}

impl LookupFamily {
    /// Every family, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::ScriptFunction,
        Self::HostFunction,
        Self::Constant,
        Self::Global,
        Self::Local,
        Self::Definition,
        Self::EffectCallback,
    ];

    const fn index(self) -> usize {
        match self {
            Self::ScriptFunction => 0,
            Self::HostFunction => 1,
            Self::Constant => 2,
            Self::Global => 3,
            Self::Local => 4,
            Self::Definition => 5,
            Self::EffectCallback => 6,
        }
    }

    /// Stable label for a profile report.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ScriptFunction => "script_function",
            Self::HostFunction => "host_function",
            Self::Constant => "constant",
            Self::Global => "global",
            Self::Local => "local",
            Self::Definition => "definition",
            Self::EffectCallback => "effect_callback",
        }
    }
}

impl fmt::Display for LookupFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// The call path that issued a lookup.
///
/// A family total says which table is hot; it does not say which code asks it.
/// The first measurement of clonk-org/clonk-rs#292 found 76% of lookups in the
/// two function families and, on sub-counting, that the path which looks
/// hottest from reading the code accounts for under a fifth of them. Attaching
/// a handle to the wrong path would move a minority of the cost, so the
/// instrument separates the paths as well as the tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LookupSite {
    /// The compiled executor's per-invocation prelude, which resolves every
    /// call site in a function body each time the function is entered.
    CompiledPrelude,
    /// The AST interpreter resolving a call expression's callee, per executed
    /// call.
    AstCall,
    /// The VM's generic name dispatch, which walks own script functions,
    /// engine globals, host functions and host reference functions in
    /// selection order. This is the path a host entry point takes when the
    /// engine calls into a script by name, and it also covers anything that
    /// dispatch reaches which does not mark a span of its own.
    GenericDispatch,
    /// The `->` and `Call` dispatch path, which resolves a named method
    /// against the target's own and global functions per executed call.
    ObjectCall,
    /// The static "does this call return a reference?" predicate the
    /// interpreter asks before evaluating an expression. It resolves a name
    /// only to inspect the signature and throws the answer away.
    ReferenceQuery,
    /// Anything not covered above. A large share here means the interesting
    /// path is still unattributed, not that it is cheap.
    Unattributed,
}

impl LookupSite {
    /// Every site, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::CompiledPrelude,
        Self::AstCall,
        Self::GenericDispatch,
        Self::ObjectCall,
        Self::ReferenceQuery,
        Self::Unattributed,
    ];

    const fn index(self) -> usize {
        match self {
            Self::CompiledPrelude => 0,
            Self::AstCall => 1,
            Self::GenericDispatch => 2,
            Self::ObjectCall => 3,
            Self::ReferenceQuery => 4,
            Self::Unattributed => 5,
        }
    }

    /// Stable label for a profile report.
    pub const fn label(self) -> &'static str {
        match self {
            Self::CompiledPrelude => "compiled_prelude",
            Self::AstCall => "ast_call",
            Self::GenericDispatch => "generic_dispatch",
            Self::ObjectCall => "object_call",
            Self::ReferenceQuery => "reference_query",
            Self::Unattributed => "unattributed",
        }
    }
}

impl fmt::Display for LookupSite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// Attributes every lookup recorded while it is alive to `site`.
///
/// Resolution helpers are shared by all the call paths and cannot tell which
/// one invoked them, so the caller marks the span instead of every helper
/// growing a parameter. Restores the enclosing site on drop, so a host entry
/// point that runs a compiled function reports each span under its own site.
#[must_use = "the site is only attributed while the guard is alive"]
pub struct SiteGuard {
    #[cfg(any(test, feature = "lookup-profile"))]
    previous: LookupSite,
}

impl Drop for SiteGuard {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "lookup-profile"))]
        ACTIVE_SITE.with(|site| site.set(self.previous));
    }
}

/// Attributes lookups to `site` until the returned guard is dropped.
#[inline(always)]
pub fn enter_site(site: LookupSite) -> SiteGuard {
    let _ = site;
    SiteGuard {
        #[cfg(any(test, feature = "lookup-profile"))]
        previous: ACTIVE_SITE.with(|active| active.replace(site)),
    }
}

/// What one family cost over the profiled window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LookupFamilyProfile {
    /// Table probes issued.
    pub lookups: u64,
    /// Key bytes fed to the hasher, and therefore also the upper bound on the
    /// bytes a successful comparison walks.
    pub hashed_bytes: u64,
    /// Lookup keys that had to be built at runtime rather than reused from a
    /// parsed identifier — the `Fx<Name>Damage` callback family and friends.
    pub key_allocations: u64,
}

/// A snapshot of every family, taken between [`reset`] and [`snapshot`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptLookupProfile {
    families: [LookupFamilyProfile; LookupFamily::ALL.len()],
    sites: [[LookupFamilyProfile; LookupSite::ALL.len()]; LookupFamily::ALL.len()],
}

impl ScriptLookupProfile {
    /// The counters for one family, across every call path.
    pub fn family(&self, family: LookupFamily) -> LookupFamilyProfile {
        self.families[family.index()]
    }

    /// The counters for one family issued from one call path.
    pub fn family_at(&self, family: LookupFamily, site: LookupSite) -> LookupFamilyProfile {
        self.sites[family.index()][site.index()]
    }

    /// Probes issued from one call path, across every family.
    pub fn site(&self, site: LookupSite) -> LookupFamilyProfile {
        LookupFamily::ALL
            .into_iter()
            .fold(LookupFamilyProfile::default(), |mut total, family| {
                let counters = self.family_at(family, site);
                total.lookups = total.lookups.saturating_add(counters.lookups);
                total.hashed_bytes = total.hashed_bytes.saturating_add(counters.hashed_bytes);
                total.key_allocations = total
                    .key_allocations
                    .saturating_add(counters.key_allocations);
                total
            })
    }

    /// Call paths ordered by probe count, heaviest first, skipping any that
    /// issued nothing.
    pub fn ranked_sites(&self) -> Vec<(LookupSite, LookupFamilyProfile)> {
        let mut ranked: Vec<_> = LookupSite::ALL
            .into_iter()
            .map(|site| (site, self.site(site)))
            .filter(|(_, profile)| profile.lookups != 0)
            .collect();
        ranked.sort_by(|left, right| {
            right
                .1
                .lookups
                .cmp(&left.1.lookups)
                .then(left.0.cmp(&right.0))
        });
        ranked
    }

    /// Probes issued across every family.
    pub fn total_lookups(&self) -> u64 {
        self.families
            .iter()
            .map(|family| family.lookups)
            .sum::<u64>()
    }

    /// Key bytes hashed across every family.
    pub fn total_hashed_bytes(&self) -> u64 {
        self.families
            .iter()
            .map(|family| family.hashed_bytes)
            .sum::<u64>()
    }

    /// Families ordered by probe count, heaviest first, skipping any that was
    /// never reached. A go/no-go decision reads this, not an aggregate.
    pub fn ranked(&self) -> Vec<(LookupFamily, LookupFamilyProfile)> {
        let mut ranked: Vec<_> = LookupFamily::ALL
            .into_iter()
            .map(|family| (family, self.family(family)))
            .filter(|(_, profile)| profile.lookups != 0)
            .collect();
        // Ties break on the family's declaration order, so a report of the
        // same run never reorders itself.
        ranked.sort_by(|left, right| {
            right
                .1
                .lookups
                .cmp(&left.1.lookups)
                .then(left.0.cmp(&right.0))
        });
        ranked
    }
}

impl fmt::Display for ScriptLookupProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (family, profile) in self.ranked() {
            writeln!(
                formatter,
                "{family}: lookups={} hashed_bytes={} key_allocations={}",
                profile.lookups, profile.hashed_bytes, profile.key_allocations
            )?;
        }
        Ok(())
    }
}

const EMPTY_FAMILY: LookupFamilyProfile = LookupFamilyProfile {
    lookups: 0,
    hashed_bytes: 0,
    key_allocations: 0,
};

thread_local! {
    /// One VM executes synchronously, but tests run many in parallel; the
    /// counters follow the same thread-local convention as the rest of the
    /// VM's instrumentation.
    ///
    /// A `RefCell` rather than a `Cell`: the site dimension makes the profile
    /// too large to copy in and out on every probe, and a counter that
    /// perturbs the run is not a measurement.
    static PROFILE: RefCell<ScriptLookupProfile> = const {
        RefCell::new(ScriptLookupProfile {
            families: [EMPTY_FAMILY; LookupFamily::ALL.len()],
            sites: [[EMPTY_FAMILY; LookupSite::ALL.len()]; LookupFamily::ALL.len()],
        })
    };
    /// The call path currently being attributed. Unattributed until a caller
    /// marks its span with [`enter_site`].
    static ACTIVE_SITE: Cell<LookupSite> = const { Cell::new(LookupSite::Unattributed) };
}

/// Clears this thread's counters and returns attribution to unattributed.
pub fn reset() {
    PROFILE.with(|profile| *profile.borrow_mut() = ScriptLookupProfile::default());
    ACTIVE_SITE.with(|site| site.set(LookupSite::Unattributed));
}

/// This thread's counters since the last [`reset`].
pub fn snapshot() -> ScriptLookupProfile {
    PROFILE.with(|profile| *profile.borrow())
}

/// Counts one probe of `family` for a key of `key` bytes.
///
/// Compiled out unless this crate's own tests are building or the
/// `lookup-profile` feature is on, so a shipped VM pays nothing.
#[inline(always)]
pub fn record(family: LookupFamily, key: &str) {
    let _ = (family, key);
    #[cfg(any(test, feature = "lookup-profile"))]
    update(family, |counters| {
        counters.lookups = counters.lookups.saturating_add(1);
        counters.hashed_bytes = counters.hashed_bytes.saturating_add(key.len() as u64);
    });
}

/// Counts one lookup key that had to be built at runtime.
///
/// The host bridge calls this from the callback families it formats, such as
/// `Fx<Name>Damage`, which is why it is public where [`record`]'s VM sites
/// are not.
#[inline(always)]
pub fn record_key_allocation(family: LookupFamily) {
    let _ = family;
    #[cfg(any(test, feature = "lookup-profile"))]
    update(family, |counters| {
        counters.key_allocations = counters.key_allocations.saturating_add(1);
    });
}

#[cfg(any(test, feature = "lookup-profile"))]
fn update(family: LookupFamily, apply: impl Fn(&mut LookupFamilyProfile)) {
    let site = ACTIVE_SITE.with(Cell::get);
    PROFILE.with(|profile| {
        // A borrow held across `apply` would deadlock if the VM ever recorded
        // a lookup from inside one; `apply` is a plain counter bump, so it
        // cannot, and the borrow stays inside this call.
        let mut profile = profile.borrow_mut();
        apply(&mut profile.families[family.index()]);
        apply(&mut profile.sites[family.index()][site.index()]);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loop that calls one script function and one host function per
    /// iteration, so every family a call path touches is exercised.
    fn driver_engine() -> crate::Engine {
        let mut engine = crate::Engine::new();
        engine.register_host_function("HostDouble", |args: &[crate::Value]| {
            let value = args.first().and_then(crate::Value::as_c4_int).unwrap_or(0);
            Ok(crate::Value::Int(value * 2))
        });
        engine
            .load_script(
                "global func Helper(value) { return value + 1; }\n\
                 global func Driver(count) {\n\
                     var total = 0;\n\
                     var index = 0;\n\
                     while (index < count) {\n\
                         total = total + Helper(HostDouble(index));\n\
                         index = index + 1;\n\
                     }\n\
                     return total;\n\
                 }",
            )
            .expect("profile driver script loads");
        engine
    }

    #[test]
    fn a_ranked_profile_puts_the_heaviest_family_first_and_drops_untouched_ones() {
        let mut profile = ScriptLookupProfile::default();
        profile.families[LookupFamily::Local.index()] = LookupFamilyProfile {
            lookups: 9,
            hashed_bytes: 45,
            key_allocations: 0,
        };
        profile.families[LookupFamily::ScriptFunction.index()] = LookupFamilyProfile {
            lookups: 12,
            hashed_bytes: 96,
            key_allocations: 0,
        };
        assert_eq!(
            profile
                .ranked()
                .into_iter()
                .map(|(family, _)| family)
                .collect::<Vec<_>>(),
            vec![LookupFamily::ScriptFunction, LookupFamily::Local],
            "an untouched family is not a measurement and must not be reported"
        );
        assert_eq!(profile.total_lookups(), 21);
        assert_eq!(profile.total_hashed_bytes(), 141);
    }

    #[test]
    fn executing_a_script_attributes_each_lookup_to_its_own_family() {
        // The point of the instrument: a decision about interning has to name
        // which family it is about, so a run that touches script functions,
        // host functions and locals must report three distinct counts rather
        // than one aggregate.
        let engine = driver_engine();

        reset();
        engine
            .call("Driver", &[crate::Value::Int(8)])
            .expect("profile driver script runs");
        let profile = snapshot();

        for family in [
            LookupFamily::ScriptFunction,
            LookupFamily::HostFunction,
            LookupFamily::Local,
        ] {
            let counters = profile.family(family);
            assert!(
                counters.lookups > 0,
                "{family} lookups must be counted: {profile}"
            );
            assert!(
                counters.hashed_bytes >= counters.lookups,
                "{family} hashes at least one byte per probe: {counters:?}"
            );
        }
        assert_eq!(
            profile.total_lookups(),
            LookupFamily::ALL
                .into_iter()
                .map(|family| profile.family(family).lookups)
                .sum::<u64>(),
            "the total must be the sum of the families it separates"
        );
    }

    #[test]
    fn function_name_lookups_do_not_scale_with_the_work_a_call_does() {
        // The measurement that decides where interning is worth anything.
        // The compiled executor resolves each call site once per invocation,
        // so name lookups stay flat however long the callee loops; the
        // callee's own identifiers are looked up by string on every access.
        // Any interning work has to keep the first property and is only
        // worth doing for the second.
        let engine = driver_engine();
        let profile_for = |iterations: i32| {
            reset();
            engine
                .call("Driver", &[crate::Value::Int(iterations)])
                .expect("profile driver script runs");
            snapshot()
        };

        let short = profile_for(4);
        let long = profile_for(64);
        assert_eq!(
            short.family(LookupFamily::ScriptFunction).lookups,
            long.family(LookupFamily::ScriptFunction).lookups,
            "resolving a call site is per invocation, not per executed call:\n{short}\n{long}"
        );
        assert_eq!(
            short.family(LookupFamily::HostFunction).lookups,
            long.family(LookupFamily::HostFunction).lookups,
            "the same holds for host call sites:\n{short}\n{long}"
        );
        assert!(
            long.family(LookupFamily::Local).lookups > short.family(LookupFamily::Local).lookups,
            "the callee's own identifiers are still resolved by string per access:\n{long}"
        );
    }

    #[test]
    fn resetting_clears_every_family() {
        record(LookupFamily::ScriptFunction, "GetX");
        assert_ne!(snapshot(), ScriptLookupProfile::default());
        reset();
        assert_eq!(snapshot(), ScriptLookupProfile::default());
    }

    #[test]
    fn a_site_guard_attributes_only_the_span_it_covers() {
        reset();
        record(LookupFamily::ScriptFunction, "Before");
        {
            let _span = enter_site(LookupSite::AstCall);
            record(LookupFamily::ScriptFunction, "Inside");
        }
        record(LookupFamily::ScriptFunction, "After");
        let profile = snapshot();

        assert_eq!(
            profile
                .family_at(LookupFamily::ScriptFunction, LookupSite::AstCall)
                .lookups,
            1,
            "only the guarded probe belongs to the span"
        );
        assert_eq!(
            profile
                .family_at(LookupFamily::ScriptFunction, LookupSite::Unattributed)
                .lookups,
            2,
            "dropping the guard restores the enclosing site"
        );
        reset();
    }

    #[test]
    fn nested_spans_each_keep_their_own_site() {
        reset();
        let _outer = enter_site(LookupSite::GenericDispatch);
        record(LookupFamily::ScriptFunction, "Outer");
        {
            let _inner = enter_site(LookupSite::CompiledPrelude);
            record(LookupFamily::ScriptFunction, "Inner");
        }
        // A host entry point that runs a compiled function must not have the
        // compiled prelude's probes charged to it, nor lose its own after.
        record(LookupFamily::ScriptFunction, "OuterAgain");
        let profile = snapshot();

        assert_eq!(
            profile
                .family_at(LookupFamily::ScriptFunction, LookupSite::CompiledPrelude)
                .lookups,
            1
        );
        assert_eq!(
            profile
                .family_at(LookupFamily::ScriptFunction, LookupSite::GenericDispatch)
                .lookups,
            2
        );
        reset();
    }

    #[test]
    fn a_family_total_is_the_sum_of_its_call_paths() {
        reset();
        record(LookupFamily::HostFunction, "Unguarded");
        {
            let _span = enter_site(LookupSite::ObjectCall);
            record(LookupFamily::HostFunction, "Guarded");
            record(LookupFamily::HostFunction, "AlsoGuarded");
        }
        let profile = snapshot();

        // The two views must never disagree; a site breakdown that does not
        // add up to its family would silently misdirect the decision it exists
        // to inform.
        assert_eq!(
            profile.family(LookupFamily::HostFunction).lookups,
            LookupSite::ALL
                .into_iter()
                .map(|site| profile.family_at(LookupFamily::HostFunction, site).lookups)
                .sum::<u64>(),
        );
        assert_eq!(
            profile.family(LookupFamily::HostFunction).hashed_bytes,
            LookupSite::ALL
                .into_iter()
                .map(|site| {
                    profile
                        .family_at(LookupFamily::HostFunction, site)
                        .hashed_bytes
                })
                .sum::<u64>(),
        );
        reset();
    }

    /// A driver the compiled executor refuses (a reference parameter), so its
    /// body runs on the AST path where the reference-returning predicate lives.
    fn interpreted_driver_engine() -> crate::Engine {
        let mut engine = crate::Engine::new();
        engine
            .load_script(
                "global func Helper(value) { return value + 1; }\n\
                 global func Interpreted(&out, count) {\n\
                     var total = 0;\n\
                     var index = 0;\n\
                     while (index < count) {\n\
                         total = Helper(index);\n\
                         index = index + 1;\n\
                     }\n\
                     out = total;\n\
                     return total;\n\
                 }",
            )
            .expect("interpreted driver script loads");
        engine
    }

    #[test]
    fn a_compiled_host_call_site_walks_the_host_tables_once() {
        // The prelude asked `host_reference_functions` whether the callee is a
        // reference function and then asked `host_functions` for the value
        // target, walking the host tables twice for every host call site.
        // Registration keeps a name out of the table it is not in, so one walk
        // answers both.
        let engine = driver_engine();
        const ITERATIONS: i32 = 8;
        reset();
        engine
            .call("Driver", &[crate::Value::Int(ITERATIONS)])
            .expect("compiled driver runs");
        let profile = snapshot();
        let host = profile
            .family_at(LookupFamily::HostFunction, LookupSite::CompiledPrelude)
            .lookups;

        assert!(
            host > 0,
            "the compiled prelude must resolve this script's host call site"
        );
        assert_eq!(
            host, 1,
            "one host call site is one walk of the host tables, not two"
        );
    }

    #[test]
    fn registering_a_host_function_removes_the_same_name_from_the_other_table() {
        // The invariant the single walk above rests on. If a name could sit in
        // both tables at once, probing values first would silently select a
        // value function where the reference guard used to bail.
        let mut engine = crate::Engine::new();
        engine
            .load_script("global func Ask() { return Ambiguous(); }")
            .expect("ambiguity probe script loads");

        engine.register_host_function("Ambiguous", |_: &[crate::Value]| Ok(crate::Value::Int(1)));
        assert_eq!(
            engine
                .call("Ask", &[])
                .expect("value host function answers"),
            crate::Value::Int(1)
        );

        engine
            .register_host_reference_function("Ambiguous", [0_usize], |_| Ok(crate::Value::Int(7)));
        assert_eq!(
            engine
                .call("Ask", &[])
                .expect("reference host function answers"),
            crate::Value::Int(7),
            "registering a reference function must evict the same-named value function"
        );

        engine.register_host_function("Ambiguous", |_: &[crate::Value]| Ok(crate::Value::Int(2)));
        assert_eq!(
            engine
                .call("Ask", &[])
                .expect("re-registered value host function answers"),
            crate::Value::Int(2),
            "registering a value function must evict the same-named reference function"
        );
    }

    #[test]
    fn one_reference_query_resolves_its_callee_once() {
        // `direct_value_call_has_materialized_result` asked
        // `call_expression_returns_reference` whether the result is a
        // reference and then resolved the same callee again to decide whether
        // it is materialized — two walks of the same tables to answer one
        // question about one call site.
        //
        // The remaining probe per call belongs to `set_no_ref_keeps_reference`
        // on a separate evaluator entry point. Sharing it would mean threading
        // a resolution between two entry points that encode C++'s SetNoRef
        // decision, so it is deliberately left to its own change.
        let engine = interpreted_driver_engine();
        const ITERATIONS: i32 = 32;
        reset();
        let (_, _) = engine
            .call_with_ref_args(
                "Interpreted",
                &[crate::Value::Nil, crate::Value::Int(ITERATIONS)],
            )
            .expect("interpreted driver runs");
        let profile = snapshot();
        let query = profile
            .family_at(LookupFamily::ScriptFunction, LookupSite::ReferenceQuery)
            .lookups;
        let budget = u64::try_from(ITERATIONS).expect("iteration count fits u64") * 2;

        assert!(query > 0, "the interpreted path must reach the predicate");
        assert!(
            query <= budget,
            "a reference query resolved the callee {query} times over {ITERATIONS} calls, \
             over the budget of {budget}; it was 3 per call before the duplicate was removed"
        );
    }

    #[test]
    fn executing_a_script_attributes_its_call_paths() {
        // The instrument's whole point: a family total does not name a call
        // site, and the sites have to add up to the families they split.
        let engine = driver_engine();
        reset();
        engine
            .call("Driver", &[crate::Value::Int(8)])
            .expect("profile driver script runs");
        let profile = snapshot();

        assert!(
            profile.site(LookupSite::CompiledPrelude).lookups > 0,
            "the compiled prelude resolves this script's call sites:\n{profile}"
        );
        assert_eq!(
            profile.total_lookups(),
            LookupSite::ALL
                .into_iter()
                .map(|site| profile.site(site).lookups)
                .sum::<u64>(),
            "every probe lands in exactly one call path"
        );
    }

    #[test]
    fn equal_families_rank_in_declaration_order_so_a_report_is_stable() {
        let mut profile = ScriptLookupProfile::default();
        for family in [LookupFamily::EffectCallback, LookupFamily::HostFunction] {
            profile.families[family.index()] = LookupFamilyProfile {
                lookups: 4,
                hashed_bytes: 20,
                key_allocations: 0,
            };
        }
        assert_eq!(
            profile
                .ranked()
                .into_iter()
                .map(|(family, _)| family)
                .collect::<Vec<_>>(),
            vec![LookupFamily::HostFunction, LookupFamily::EffectCallback],
        );
    }
}
