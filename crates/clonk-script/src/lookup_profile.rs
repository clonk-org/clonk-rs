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

use std::cell::Cell;
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
}

impl ScriptLookupProfile {
    /// The counters for one family.
    pub fn family(&self, family: LookupFamily) -> LookupFamilyProfile {
        self.families[family.index()]
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

thread_local! {
    /// One VM executes synchronously, but tests run many in parallel; the
    /// counters follow the same thread-local convention as the rest of the
    /// VM's instrumentation.
    static PROFILE: Cell<ScriptLookupProfile> = const {
        Cell::new(ScriptLookupProfile {
            families: [LookupFamilyProfile {
                lookups: 0,
                hashed_bytes: 0,
                key_allocations: 0,
            }; LookupFamily::ALL.len()],
        })
    };
}

/// Clears this thread's counters.
pub fn reset() {
    PROFILE.with(|profile| profile.set(ScriptLookupProfile::default()));
}

/// This thread's counters since the last [`reset`].
pub fn snapshot() -> ScriptLookupProfile {
    PROFILE.with(Cell::get)
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
fn update(family: LookupFamily, apply: impl FnOnce(&mut LookupFamilyProfile)) {
    PROFILE.with(|profile| {
        let mut current = profile.get();
        apply(&mut current.families[family.index()]);
        profile.set(current);
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
