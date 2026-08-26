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
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// The contract, embedded so the runtime answer and the gated manifest are the
/// same artifact.
const PROFILE_MANIFEST: &str = include_str!("../../../compat/profile.json");

/// The manifest text, for tests that check a claim against what the profile
/// actually registers rather than against a second copy of the same list.
#[cfg(test)]
pub(crate) fn profile_manifest_for_tests() -> &'static str {
    PROFILE_MANIFEST
}

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

impl Evidence {
    /// What closes this evidence entry.
    ///
    /// An explicit `tracked_by` always wins. Failing that, evidence of kind
    /// `issue` already *is* an issue reference, so naming it is strictly
    /// better than the placeholder that used to appear beside it — a blocker
    /// reading "no tracking issue recorded" while quoting an issue number in
    /// the same line gives a player nothing to act on, which is exactly what
    /// clonk-org/clonk-rs#588 requires each blocker to carry.
    fn recovery_action(&self) -> String {
        self.tracked_by
            .clone()
            .or_else(|| (self.kind == "issue").then(|| self.value.clone()))
            .unwrap_or_else(|| {
                format!(
                    "run the {} check `{}` and record its result in compat/profile.json",
                    self.kind, self.value
                )
            })
    }
}

#[derive(Deserialize)]
struct Divergence {
    id: String,
    area: String,
    summary: String,
    /// Read by the readiness tests, which hold accepted entries in the
    /// manifest so an accepted limitation cannot silently become a claim.
    #[cfg_attr(not(test), allow(dead_code))]
    disposition: String,
    profile_action: String,
    #[serde(default)]
    cited_in: Vec<String>,
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
                recovery: evidence.recovery_action(),
            })
    }));
    blockers
}

/// The `planet/System.c4g` scripts the profile withholds.
///
/// Taken from the contract rather than a second hand-kept list: a content
/// `#appendto` divergence exists *as* a shipped script, so reverting it means
/// not loading that file, and every such divergence already names its file in
/// `cited_in`. A tenth divergence therefore withholds its script with no code
/// change, and a divergence that is retired stops withholding it.
///
/// Only `planet/System.c4g` paths are taken. The same field also cites Rust
/// sources and test files, which are not content and are never withheld.
pub fn reverted_content_scripts() -> BTreeSet<String> {
    manifest()
        .divergences
        .iter()
        .filter(|divergence| divergence.profile_action == "reverted")
        .flat_map(|divergence| divergence.cited_in.iter())
        .filter_map(|path| path.strip_prefix("planet/System.c4g/"))
        .filter(|name| name.ends_with(".c"))
        .map(str::to_string)
        .collect()
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

/// How many blockers to name individually before summarising the rest.
///
/// The lobby log is a C++-mirrored surface with a small visible area, and a
/// player cannot act on fourteen lines anyway. Naming a few by id and counting
/// the remainder keeps the message actionable while staying quotable.
const REPORTED_BLOCKERS: usize = 4;

/// The lines to show a host that asked for a profile the contract cannot back.
///
/// Returns empty when the profile is claimable, so a caller can use it as the
/// whole decision. Never phrased as a downgrade that already happened silently:
/// the first line says the profile was requested and is not being claimed, which
/// is the thing a player has to know before anyone joins.
pub fn blocked_profile_report(profile: &str) -> Vec<String> {
    let blockers = blockers();
    if blockers.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "Compatibility profile {profile} requested but NOT claimed: {} unresolved contract \
         item(s). This session runs as an ordinary one.",
        blockers.len()
    )];
    lines.extend(named_blockers(&blockers));
    lines
}

/// The lines to show a *client* that asked for a profile the contract cannot
/// back, when it is about to join someone else's session.
///
/// Deliberately not [`blocked_profile_report`]. That one states what the
/// session will be, which is the host's to say — a client announcing it would
/// assert a promise the session never made. This one is about the client's own
/// request and says only what the client can answer for. Empty when the profile
/// is claimable, so a caller can use it as the whole decision.
pub fn blocked_join_report(profile: &str) -> Vec<String> {
    let blockers = blockers();
    if blockers.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "This client requested compatibility profile {profile} but cannot honour it: {} \
         unresolved contract item(s). It joins as an ordinary client.",
        blockers.len()
    )];
    lines.extend(named_blockers(&blockers));
    lines
}

/// The blocker detail shared by every report: a few named by id with their
/// recovery action, then a count of the remainder and where to read the rest.
fn named_blockers(blockers: &[CompatBlocker]) -> Vec<String> {
    let mut lines = blockers
        .iter()
        .take(REPORTED_BLOCKERS)
        .map(|blocker| format!("  [{}] {} — {}", blocker.area, blocker.id, blocker.recovery))
        .collect::<Vec<_>>();
    if let Some(remaining) = blockers
        .len()
        .checked_sub(REPORTED_BLOCKERS)
        .filter(|n| *n > 0)
    {
        lines.push(format!(
            "  ...and {remaining} more; see docs/COMPAT_PROFILE.md and compat/profile.json."
        ));
    }
    lines
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    /// A stock peer refuses the session before a single script runs, so the
    /// contract has to say so before anyone hosts.
    ///
    /// `System.c4g` is announced as a resource the host never sends -- both
    /// engines publish it `Loadable=false`, carrying only a `ContentsCRC` --
    /// so a client that does not already hold an identical group cannot obtain
    /// one and aborts (`C4Network2Res.cpp:1501-1506`). The port ships files in
    /// `planet/System.c4g` that stock LegacyClonk does not, which changes that
    /// CRC. Disabling the appends does not close it: the profile turns the
    /// scripts off, and the check compares bytes.
    ///
    /// Accepted as permanent by product decision (clonk-org/clonk-rs#1094), so
    /// it is no longer a blocker: nothing the port can *compute* differently
    /// changes the CRC, and only shipping a byte-identical group would -- which
    /// means not shipping the port's own content. It stays in the manifest, and
    /// this test holds it there: the limitation must remain stated where the
    /// contract is read, and it must not silently become a claim of
    /// compatibility.
    #[test]
    fn the_contract_keeps_the_system_group_a_stock_peer_would_refuse() {
        let divergence = manifest()
            .divergences
            .iter()
            .find(|divergence| divergence.id == "content-system-group-identity")
            .expect("the contract still records the System.c4g identity gap");

        assert_eq!(divergence.area, "content");
        assert_eq!(
            divergence.disposition, "accepted",
            "the gap is accepted, not an unfixed open gap"
        );
        assert_eq!(
            divergence.profile_action, "kept",
            "nothing the port computes can change the group's CRC, so there is \
             nothing to revert"
        );
        assert!(
            divergence.summary.contains("System.c4g"),
            "the entry names the group a peer compares: {}",
            divergence.summary
        );
        assert!(
            divergence.summary.contains("out of scope"),
            "the entry states the limitation it accepts: {}",
            divergence.summary
        );
        assert!(
            !blockers()
                .iter()
                .any(|blocker| blocker.id == "content-system-group-identity"),
            "an accepted divergence is not a readiness blocker"
        );
    }

    #[test]
    fn the_reverted_content_scripts_come_from_the_contract() {
        // The content `#appendto` divergences are the only reason the profile
        // withholds a shipped script, and each already names its file in
        // `cited_in`. Deriving the set from the manifest keeps one source of
        // truth: adding another divergence withholds its script with no code
        // change, and removing one stops withholding it.
        let scripts = reverted_content_scripts();

        for expected in [
            "BirdFlight.c",
            "EkeAirbikeSteering.c",
            "EkeGpedRemoteControl.c",
            "EkeGuidedMissile.c",
            "EkeSftRelease.c",
            "FoWReveal.c",
            "GatherMenu.c",
            "GatherTask.c",
            "MarsOrderCapsule.c",
            "MenuRangeRow.c",
        ] {
            assert!(
                scripts.contains(expected),
                "{expected} is cited by a reverted content divergence"
            );
        }

        // Shipped engine scripts are never withheld -- reverting a divergence
        // must not take C4Script's own standard library with it.
        for kept in ["C4.c", "Explode.c", "FindObject.c", "Helpers.c", "Magic.c"] {
            assert!(!scripts.contains(kept), "{kept} is not a divergence");
        }
    }

    #[test]
    fn every_blocker_names_a_recovery_action() {
        // clonk-org/clonk-rs#588: a blocker is only actionable if it carries a
        // recovery action as well as a diagnostic. Evidence of kind `issue`
        // already names the issue in its own value, so reporting "no tracking
        // issue recorded" beside it is a defect in the report rather than a
        // gap in the manifest -- and a blocker whose recovery is a placeholder
        // tells a player nothing they can act on.
        for blocker in blockers() {
            assert!(
                !blocker.recovery.starts_with("no "),
                "blocker `{}` reports no recovery action: {}",
                blocker.id,
                blocker.recovery
            );
            assert!(
                !blocker.recovery.trim().is_empty(),
                "blocker `{}` has an empty recovery action",
                blocker.id
            );
        }
    }

    #[test]
    fn a_blocked_profile_reports_what_is_wrong_and_never_claims_it() {
        // clonk-org/clonk-rs#588: the contract's fail-closed rule is only
        // useful if a host is told before anyone joins. The report must say
        // the profile is not claimed, and must stay quotable — every named
        // line carries a manifest id and its recovery.
        let lines = blocked_profile_report("LegacyClonk");
        assert!(
            !lines.is_empty(),
            "the contract records gaps, so a report is owed"
        );
        assert!(
            lines[0].contains("NOT claimed"),
            "the first line must not read as a silent downgrade: {}",
            lines[0]
        );
        assert!(
            lines[0].contains("LegacyClonk"),
            "the report names the profile that was requested"
        );

        let blockers = blockers();
        let named = lines.len().saturating_sub(1).min(REPORTED_BLOCKERS);
        for (line, blocker) in lines[1..=named].iter().zip(&blockers) {
            assert!(line.contains(&blocker.id), "{line} must quote its id");
            assert!(
                line.contains(&blocker.recovery),
                "{line} must name what closes it"
            );
        }
        if blockers.len() > REPORTED_BLOCKERS {
            assert!(
                lines
                    .last()
                    .is_some_and(|line| line.contains("more") && line.contains("COMPAT_PROFILE")),
                "the remainder must be counted and pointed at the contract"
            );
        }
    }

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
        //
        // `blocked` is zero today -- every open gap has been closed or
        // accepted -- so this deliberately does NOT require one to exist.
        // Reaching zero is the goal, not a broken fixture. The sum still pins
        // that both kinds are summed, and the per-kind checks below pin the
        // mapping itself, so a blocked divergence added later cannot go
        // unreported.
        assert!(pending > 0, "an unproven promise is still outstanding");
        assert_eq!(blockers().len(), blocked + pending);

        let reported = blockers()
            .into_iter()
            .map(|blocker| blocker.id)
            .collect::<Vec<_>>();
        for divergence in manifest
            .divergences
            .iter()
            .filter(|divergence| divergence.profile_action == "blocked")
        {
            assert!(
                reported.contains(&divergence.id),
                "a blocked divergence must reach the readiness report: {}",
                divergence.id
            );
        }
    }
}
