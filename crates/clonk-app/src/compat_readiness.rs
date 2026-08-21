//! Whether the compatibility profile may be claimed, computed from the
//! contract itself.
//!
//! `docs/COMPAT_PROFILE.md` and `compat/profile.json` are the promise; this is
//! the runtime half of the `fc-readiness` fail-closed rule the contract states:
//! a profile with an unclosed gap or an unproven promise must not be advertised
//! as compatible. Computing it from the manifest rather than from a hand-kept
//! constant means the two cannot drift — the manifest is embedded at build
//! time, so a change to the contract changes this answer in the same commit.
//!
//! Deciding this before a session starts is the whole point. A blocked profile
//! that is discovered mid-round is a desync in a lockstep engine, which costs
//! everyone in the session their round, not just the peer that was wrong.

use serde::Deserialize;
use std::sync::OnceLock;

/// The contract, embedded so the runtime answer and the gated manifest are the
/// same artifact.
const PROFILE_MANIFEST: &str = include_str!("../../../compat/profile.json");

/// Why the profile cannot be claimed, and what would close it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatBlocker {
    /// The manifest id, stable across runs and quotable in a bug report.
    pub id: String,
    /// Which contract area it belongs to.
    pub area: String,
    /// What is wrong, in the contract's own words.
    pub reason: String,
    /// What closes it — an owner reference for a gap, or the check that has
    /// not been run for unproven evidence.
    pub recovery: String,
}

#[derive(Deserialize)]
struct Manifest {
    promise: std::collections::BTreeMap<String, PromiseArea>,
    divergences: Vec<Divergence>,
}

#[derive(Deserialize)]
struct PromiseArea {
    #[serde(default)]
    evidence: Vec<Evidence>,
}

#[derive(Deserialize)]
struct Evidence {
    kind: String,
    value: String,
    status: String,
    #[serde(default)]
    tracked_by: Option<String>,
}

#[derive(Deserialize)]
struct Divergence {
    id: String,
    area: String,
    summary: String,
    profile_action: String,
    #[serde(default)]
    owner: Option<String>,
}

fn manifest() -> &'static Manifest {
    static PARSED: OnceLock<Manifest> = OnceLock::new();
    PARSED.get_or_init(|| {
        serde_json::from_str(PROFILE_MANIFEST)
            .expect("the embedded compatibility manifest is generated and gated by compat verify")
    })
}

/// Every reason the profile cannot currently be claimed.
///
/// Two kinds, and the contract keeps them apart on purpose. A divergence with
/// `profile_action: blocked` is an **open gap** — a defect that has not been
/// fixed. Evidence with `status: pending` is a promise whose **check does not
/// exist yet**: the behaviour may well be right, but nothing proves it, and an
/// unproven promise is not one worth making to a lockstep peer.
pub fn blockers() -> Vec<CompatBlocker> {
    let manifest = manifest();
    let mut blockers = manifest
        .divergences
        .iter()
        .filter(|divergence| divergence.profile_action == "blocked")
        .map(|divergence| CompatBlocker {
            id: divergence.id.clone(),
            area: divergence.area.clone(),
            reason: divergence.summary.clone(),
            recovery: divergence
                .owner
                .clone()
                .unwrap_or_else(|| "no owner recorded in the manifest".to_string()),
        })
        .collect::<Vec<_>>();

    blockers.extend(manifest.promise.iter().flat_map(|(area, promise)| {
        promise
            .evidence
            .iter()
            .filter(|evidence| evidence.status == "pending")
            .map(move |evidence| CompatBlocker {
                id: format!("{area}:{}", evidence.value),
                area: area.clone(),
                reason: format!(
                    "the {area} promise is not proven: its {} evidence `{}` is pending",
                    evidence.kind, evidence.value
                ),
                recovery: evidence
                    .tracked_by
                    .clone()
                    .unwrap_or_else(|| "no tracking issue recorded in the manifest".to_string()),
            })
    }));
    blockers
}

/// Whether the profile may be claimed at all.
///
/// Fail-closed: anything unresolved answers `false`. This never downgrades a
/// requested profile silently — callers are expected to report [`blockers`]
/// rather than quietly starting an ordinary session, which is what turns a
/// mid-round desync into a message before anyone commits.
pub fn is_ready() -> bool {
    blockers().is_empty()
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    #[test]
    fn the_profile_is_not_claimable_while_the_contract_records_gaps() {
        // The manifest is the authority, so this test states the rule rather
        // than a count that would have to be edited every time the contract
        // moves: every blocked divergence and every pending promise is a
        // blocker, and any of them makes the profile unclaimable.
        let blockers = blockers();
        assert!(
            !blockers.is_empty(),
            "the contract still records gaps, so the profile must not be claimable"
        );
        assert!(!is_ready());

        for blocker in &blockers {
            assert!(!blocker.id.is_empty(), "a blocker must be quotable by id");
            assert!(
                !blocker.reason.is_empty(),
                "a blocker must say what is wrong"
            );
            assert!(
                !blocker.recovery.is_empty(),
                "a blocker must name what closes it: {}",
                blocker.id
            );
        }
    }

    #[test]
    fn blocked_divergences_and_pending_evidence_are_both_reported() {
        let manifest = manifest();
        let blocked = manifest
            .divergences
            .iter()
            .filter(|divergence| divergence.profile_action == "blocked")
            .count();
        let pending = manifest
            .promise
            .values()
            .flat_map(|promise| &promise.evidence)
            .filter(|evidence| evidence.status == "pending")
            .count();

        // Both kinds count, and neither is allowed to mask the other: an open
        // gap and an unproven promise are different failures with different
        // fixes, which is why the contract keeps their dispositions apart.
        assert!(blocked > 0 && pending > 0, "the fixture is worth having");
        assert_eq!(blockers().len(), blocked + pending);
    }
}
