//! `cargo xtask compat verify` — the machine-readable LegacyClonk
//! compatibility-profile contract.
//!
//! `compat/profile.json` is the divergence manifest defined by
//! clonk-org/clonk-rs#581; `docs/COMPAT_PROFILE.md` is its human
//! documentation. This command is the gate that keeps the two honest: it
//! schema-checks the manifest, cross-checks the pinned engine version against
//! `clonk_core::version`, the oracle commit against the pinned snapshot, and
//! the content commit against the actual `content` submodule pin, and it
//! reports the readiness state the fail-closed `fc-readiness` rule acts on.
//!
//! The checks are deliberately fail-closed: a missing section, a bare issue
//! reference, an unowned divergence, or a content pin that has drifted are
//! all verification failures, never warnings.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use xtask::parity::workspace_dir;

/// The instrumented oracle snapshot every differential evidence refers to
/// (AGENTS.md, `parity/oracle/gen_golden.sh`).
const ORACLE_COMMIT: &str = "7d43b47b7d789b533f32d005e64596e0a07019cd";

const SCHEMA: &str = "clonk-rs/compat-profile/v1";
const PROFILE_ID: &str = "legacy-clonk";

/// The six contract areas. The promise and every divergence must name one.
const AREAS: [&str; 6] = [
    "simulation",
    "control",
    "transport",
    "content",
    "presentation",
    "save_replay",
];

const EVIDENCE_KINDS: [&str; 4] = ["command", "issue", "document", "test"];
const EVIDENCE_STATUSES: [&str; 2] = ["held", "pending"];
const DISPOSITIONS: [&str; 2] = ["accepted", "open-gap"];
const PROFILE_ACTIONS: [&str; 3] = ["reverted", "kept", "blocked"];
const FEATURE_ACTIONS: [&str; 2] = ["kept", "disabled"];

#[derive(Debug, Deserialize)]
struct Manifest {
    schema: String,
    profile: ProfileMeta,
    pinned: Pinned,
    promise: std::collections::BTreeMap<String, PromiseArea>,
    divergences: Vec<Divergence>,
    port_only_features: Vec<PortOnlyFeature>,
    fail_closed: Vec<FailClosedRule>,
}

#[derive(Debug, Deserialize)]
struct ProfileMeta {
    id: String,
    name: String,
    document: String,
}

#[derive(Debug, Deserialize)]
struct Pinned {
    engine: PinnedEngine,
    oracle_commit: String,
    content_commit: String,
}

#[derive(Debug, Deserialize)]
struct PinnedEngine {
    xver: Vec<i32>,
    build: i32,
    text: String,
}

#[derive(Debug, Deserialize)]
struct PromiseArea {
    statement: String,
    evidence: Vec<Evidence>,
}

#[derive(Debug, Deserialize)]
struct Evidence {
    kind: String,
    value: String,
    status: String,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Divergence {
    id: String,
    area: String,
    summary: String,
    cited_in: Vec<String>,
    #[serde(default)]
    cpp_reference: Option<String>,
    disposition: String,
    profile_action: String,
    determinism_critical: bool,
    owner: String,
}

#[derive(Debug, Deserialize)]
struct PortOnlyFeature {
    id: String,
    area: String,
    summary: String,
    cited_in: Vec<String>,
    profile_action: String,
    owner: String,
}

#[derive(Debug, Deserialize)]
struct FailClosedRule {
    id: String,
    combination: String,
    behavior: String,
    #[serde(default)]
    basis: Option<String>,
}

fn is_commit_sha(text: &str) -> bool {
    text.len() == 40
        && text
            .chars()
            .all(|digit| digit.is_ascii_digit() || matches!(digit, 'a'..='f'))
}

/// A qualified issue reference (`clonk-org/clonk-rs#N`), never a bare `#N`.
fn is_qualified_issue(text: &str) -> bool {
    text.starts_with("clonk-org/clonk-rs#")
        && text["clonk-org/clonk-rs#".len()..]
            .chars()
            .all(|ch| ch.is_ascii_digit())
}

/// Structural and cross-reference validation of the manifest.
///
/// `expected_content_commit` is the `content` submodule pin the manifest must
/// agree with; `None` skips that one check (used by the in-memory tamper
/// tests, which only assert the specific rule they exercise).
pub fn validate(json: &str, expected_content_commit: Option<&str>) -> Vec<String> {
    let manifest: Manifest = match serde_json::from_str(json) {
        Ok(manifest) => manifest,
        Err(error) => return vec![format!("manifest does not parse: {error}")],
    };

    [
        check_pins(&manifest, expected_content_commit),
        check_profile_meta(&manifest.profile),
        check_promise(&manifest.promise),
        check_divergences(&manifest.divergences),
        check_port_only_features(&manifest.port_only_features, &manifest.divergences),
        check_fail_closed(&manifest.fail_closed),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Tree-level validation: every path the contract cites must resolve, and
/// every entry it carries must also appear in its human documentation.
///
/// Only the path head is resolved. Line numbers drift with every edit and a
/// stale line is not what this catches; what it catches is a rename that
/// silently orphans the divergence it documented, leaving the contract
/// describing code that is no longer there.
pub fn validate_tree(repo_root: &Path, json: &str) -> Vec<String> {
    let manifest: Manifest = match serde_json::from_str(json) {
        Ok(manifest) => manifest,
        Err(error) => return vec![format!("manifest does not parse: {error}")],
    };

    // `crates/clonk-engine/src/pxs.rs:362`, `crates/clonk-network (…tests)`,
    // and bare `parity/README.md` all reduce to their leading path.
    let resolves = |citation: &str| {
        citation
            .split_whitespace()
            .next()
            .and_then(|head| head.split(':').next())
            .filter(|head| !head.is_empty())
            .is_some_and(|head| repo_root.join(head).exists())
    };

    let mut issues = Vec::new();
    // The manifest and its document are two halves of one contract. An entry
    // that lives only in the manifest is a decision no reader can find, so the
    // document has to mention every one of them by id.
    match std::fs::read_to_string(repo_root.join(&manifest.profile.document)) {
        Err(_) => issues.push(format!(
            "the human documentation `{}` does not exist; the contract is a manifest plus the \
             document it points at, and a pointer at nothing states no promise",
            manifest.profile.document
        )),
        Ok(document) => {
            let entries = manifest
                .divergences
                .iter()
                .map(|divergence| ("divergence", &divergence.id))
                .chain(
                    manifest
                        .port_only_features
                        .iter()
                        .map(|feature| ("port-only feature", &feature.id)),
                )
                .chain(
                    manifest
                        .fail_closed
                        .iter()
                        .map(|rule| ("fail-closed rule", &rule.id)),
                );
            for (kind, id) in entries {
                if !document.contains(id.as_str()) {
                    issues.push(format!(
                        "{kind} `{id}` is in the manifest but nowhere in `{}`; a difference from \
                         C++ that is not written down for a reader is not documented",
                        manifest.profile.document
                    ));
                }
            }
        }
    }
    for (area, section) in &manifest.promise {
        for (index, entry) in section.evidence.iter().enumerate() {
            if matches!(entry.kind.as_str(), "document" | "test") && !resolves(&entry.value) {
                issues.push(format!(
                    "promise[{area}].evidence[{index}] cites `{}`, which names nothing in the tree",
                    entry.value
                ));
            }
        }
    }
    let cited = manifest
        .divergences
        .iter()
        .map(|divergence| ("divergence", &divergence.id, &divergence.cited_in))
        .chain(
            manifest
                .port_only_features
                .iter()
                .map(|feature| ("port-only feature", &feature.id, &feature.cited_in)),
        );
    for (kind, id, citations) in cited {
        for citation in citations {
            if !resolves(citation) {
                issues.push(format!(
                    "{kind} `{id}`: citation `{citation}` names nothing in the tree"
                ));
            }
        }
    }
    issues
}

fn check_pins(manifest: &Manifest, expected_content_commit: Option<&str>) -> Vec<String> {
    let mut issues = Vec::new();
    if manifest.schema != SCHEMA {
        issues.push(format!(
            "schema is `{}`, expected `{SCHEMA}`",
            manifest.schema
        ));
    }
    let pinned = &manifest.pinned;

    // The engine pin must agree with clonk-core on all three faces: the
    // numeric tuple, the build, and the exact protocol string. Drift between
    // the two would let content gating and the profile promise disagree.
    let [x1, x2, x3, x4] = match pinned.engine.xver.as_slice() {
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => {
            issues.push(format!(
                "pinned engine xver must have exactly four slots, found {}",
                pinned.engine.xver.len()
            ));
            return issues;
        }
    };
    let core = clonk_core::version::ENGINE_VERSION;
    if [x1, x2, x3, x4, pinned.engine.build] != core {
        issues.push(format!(
            "pinned engine version [{x1}, {x2}, {x3}, {x4}, {}] does not match clonk-core's \
             ENGINE_VERSION {core:?}",
            pinned.engine.build
        ));
    }
    let expected_text = format!("{x1}.{x2}.{x3}.{x4} [{}]", pinned.engine.build);
    if pinned.engine.text != expected_text {
        issues.push(format!(
            "pinned engine text `{}` does not render its own tuple as `{expected_text}`",
            pinned.engine.text
        ));
    }
    if pinned.engine.text != clonk_core::version::ENGINE_VERSION_COMPACT {
        issues.push(format!(
            "pinned engine text `{}` does not match clonk-core's `{}`",
            pinned.engine.text,
            clonk_core::version::ENGINE_VERSION_COMPACT
        ));
    }

    if pinned.oracle_commit != ORACLE_COMMIT {
        issues.push(format!(
            "pinned oracle commit `{}` is not the instrumented snapshot {ORACLE_COMMIT}",
            pinned.oracle_commit
        ));
    } else if !is_commit_sha(&pinned.oracle_commit) {
        issues.push("pinned oracle commit is not a 40-digit lowercase git name".to_string());
    }

    if !is_commit_sha(&pinned.content_commit) {
        issues.push("pinned content commit is not a 40-digit lowercase git name".to_string());
    } else if let Some(expected) = expected_content_commit {
        if pinned.content_commit != expected {
            issues.push(format!(
                "pinned content commit `{}` has drifted from the `content` submodule pin \
                 `{expected}`; restate the contract pin and re-verify it in the same change",
                pinned.content_commit
            ));
        }
    }
    issues
}

fn check_profile_meta(profile: &ProfileMeta) -> Vec<String> {
    let mut issues = Vec::new();
    if profile.id != PROFILE_ID {
        issues.push(format!(
            "profile id is `{}`, expected `{PROFILE_ID}`",
            profile.id
        ));
    }
    if profile.name.is_empty() {
        issues.push("profile name is empty".to_string());
    }
    if profile.document.is_empty() {
        issues.push(
            "profile document path is empty; the machine-readable manifest must point at its \
             human documentation"
                .to_string(),
        );
    }
    issues
}

fn check_promise(promise: &std::collections::BTreeMap<String, PromiseArea>) -> Vec<String> {
    let mut issues = Vec::new();
    for area in AREAS {
        if !promise.contains_key(area) {
            issues.push(format!(
                "promise has no `{area}` section; required evidence must be stated separately \
                 for every contract area"
            ));
        }
    }
    for area in promise.keys() {
        if !AREAS.contains(&area.as_str()) {
            issues.push(format!("promise section `{area}` is not a contract area"));
        }
    }
    for (area, section) in promise {
        if section.statement.trim().is_empty() {
            issues.push(format!("promise[{area}] has no statement"));
        }
        if section.evidence.is_empty() {
            issues.push(format!("promise[{area}] carries no evidence"));
        }
        for (index, entry) in section.evidence.iter().enumerate() {
            if !EVIDENCE_KINDS.contains(&entry.kind.as_str()) {
                issues.push(format!(
                    "promise[{area}].evidence[{index}] kind `{}` is not one of {}",
                    entry.kind,
                    EVIDENCE_KINDS.join("/")
                ));
            }
            if !EVIDENCE_STATUSES.contains(&entry.status.as_str()) {
                issues.push(format!(
                    "promise[{area}].evidence[{index}] status `{}` is not one of {}",
                    entry.status,
                    EVIDENCE_STATUSES.join("/")
                ));
            }
            if entry.value.trim().is_empty() {
                issues.push(format!("promise[{area}].evidence[{index}] has no value"));
            }
            if entry
                .note
                .as_deref()
                .is_some_and(|note| note.trim().is_empty())
            {
                issues.push(format!(
                    "promise[{area}].evidence[{index}] carries an empty note; drop the field \
                     rather than ship a note that says nothing"
                ));
            }
            if entry.status == "pending" && !is_qualified_issue(&entry.value) {
                issues.push(format!(
                    "promise[{area}].evidence[{index}] is pending but cites `{}`, which is not \
                     a qualified clonk-org/clonk-rs issue reference",
                    entry.value
                ));
            }
        }
    }
    issues
}

fn check_divergences(divergences: &[Divergence]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for divergence in divergences {
        let at = format!("divergence `{}`", divergence.id);
        if divergence.id.trim().is_empty() {
            issues.push("divergence has an empty id".to_string());
        }
        if !seen.insert(divergence.id.as_str()) {
            issues.push(format!("{at}: duplicate id"));
        }
        if !AREAS.contains(&divergence.area.as_str()) {
            issues.push(format!(
                "{at}: area `{}` is not a contract area",
                divergence.area
            ));
        }
        if divergence.summary.trim().is_empty() {
            issues.push(format!("{at}: summary is empty"));
        }
        if divergence
            .cited_in
            .iter()
            .all(|citation| citation.trim().is_empty())
            || divergence.cited_in.is_empty()
        {
            issues.push(format!(
                "{at}: has no citation; every divergence must name its source"
            ));
        }
        if !DISPOSITIONS.contains(&divergence.disposition.as_str()) {
            issues.push(format!(
                "{at}: disposition `{}` is not one of {}",
                divergence.disposition,
                DISPOSITIONS.join("/")
            ));
        }
        if !PROFILE_ACTIONS.contains(&divergence.profile_action.as_str()) {
            issues.push(format!(
                "{at}: profile action `{}` is not one of {}",
                divergence.profile_action,
                PROFILE_ACTIONS.join("/")
            ));
        }
        // An open gap may not be part of the promise: it either gets closed
        // or it blocks readiness. A stable, accepted divergence may be kept
        // or reverted, never silently blocked.
        if divergence.disposition == "open-gap" && divergence.profile_action != "blocked" {
            issues.push(format!(
                "{at}: an open-gap divergence must be `blocked`, not `{}`",
                divergence.profile_action
            ));
        }
        if divergence.disposition == "accepted" && divergence.profile_action == "blocked" {
            issues.push(format!(
                "{at}: an accepted divergence must be `kept` or `reverted`, not `blocked`"
            ));
        }
        // A divergence that can move synchronized state must name the C++ it
        // differs from. Without that the entry asserts a difference it never
        // locates, and no reader can check the claim against the oracle.
        let cpp_reference = divergence
            .cpp_reference
            .as_deref()
            .unwrap_or_default()
            .trim();
        if cpp_reference.is_empty() {
            if divergence.determinism_critical {
                issues.push(format!(
                    "{at}: is determinism-critical but names no `cpp_reference`; say what C++ \
                     does at the same point"
                ));
            } else if divergence.cpp_reference.is_some() {
                issues.push(format!("{at}: has an empty `cpp_reference`"));
            }
        }
        if divergence.owner.trim().is_empty() {
            issues.push(format!("{at}: has no owner"));
        } else if divergence.owner.contains('#') && !is_qualified_issue(&divergence.owner) {
            issues.push(format!(
                "{at}: owner `{}` cites an issue unqualified; write clonk-org/clonk-rs#N",
                divergence.owner
            ));
        }
    }
    issues
}

fn check_port_only_features(
    features: &[PortOnlyFeature],
    divergences: &[Divergence],
) -> Vec<String> {
    let mut issues = Vec::new();
    let divergence_ids: std::collections::BTreeSet<&str> =
        divergences.iter().map(|d| d.id.as_str()).collect();
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for feature in features {
        let at = format!("port-only feature `{}`", feature.id);
        if feature.id.trim().is_empty() {
            issues.push("port-only feature has an empty id".to_string());
        }
        if !seen.insert(feature.id.as_str()) {
            issues.push(format!("{at}: duplicate id"));
        }
        if divergence_ids.contains(feature.id.as_str()) {
            issues.push(format!("{at}: id collides with a divergence id"));
        }
        if !AREAS.contains(&feature.area.as_str()) {
            issues.push(format!(
                "{at}: area `{}` is not a contract area",
                feature.area
            ));
        }
        if feature.summary.trim().is_empty() {
            issues.push(format!("{at}: summary is empty"));
        }
        if feature.cited_in.is_empty() || feature.cited_in.iter().all(|c| c.trim().is_empty()) {
            issues.push(format!("{at}: has no citation"));
        }
        if !FEATURE_ACTIONS.contains(&feature.profile_action.as_str()) {
            issues.push(format!(
                "{at}: profile action `{}` is not one of {}",
                feature.profile_action,
                FEATURE_ACTIONS.join("/")
            ));
        }
        if feature.owner.trim().is_empty() {
            issues.push(format!("{at}: has no owner"));
        } else if feature.owner.contains('#') && !is_qualified_issue(&feature.owner) {
            issues.push(format!(
                "{at}: owner `{}` cites an issue unqualified; write clonk-org/clonk-rs#N",
                feature.owner
            ));
        }
    }
    issues
}

fn check_fail_closed(rules: &[FailClosedRule]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for rule in rules {
        let at = format!("fail-closed rule `{}`", rule.id);
        if rule.id.trim().is_empty() {
            issues.push("fail-closed rule has an empty id".to_string());
        }
        if !seen.insert(rule.id.as_str()) {
            issues.push(format!("{at}: duplicate id"));
        }
        if rule.combination.trim().is_empty() {
            issues.push(format!("{at}: does not name the unsupported combination"));
        }
        if rule.behavior.trim().is_empty() {
            issues.push(format!("{at}: does not state what fails and how"));
        }
        if rule.basis.as_deref().unwrap_or_default().trim().is_empty() {
            issues.push(format!(
                "{at}: names no `basis`; a refusal rule rests on a C++ line, a manifest pin, or \
                 an issue, never on assertion alone"
            ));
        }
    }
    if rules.iter().all(|rule| rule.id != "fc-readiness") {
        issues.push(
            "fail_closed has no `fc-readiness` rule; the completion rule (clonk-org/clonk-rs#498) \
             must be machine-readable"
                .to_string(),
        );
    }
    issues
}

/// The `content` submodule pin at HEAD, for the cross-check.
fn content_submodule_commit(repo_root: &Path) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(["ls-tree", "HEAD", "content"])
        .output()
        .context("running `git ls-tree HEAD content`")?;
    if !output.status.success() {
        bail!(
            "`git ls-tree HEAD content` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // `160000 commit <sha>\tcontent`
    let sha = stdout
        .split_whitespace()
        .nth(2)
        .filter(|sha| is_commit_sha(sha))
        .ok_or_else(|| {
            anyhow::anyhow!("could not read the content submodule pin from: {stdout}")
        })?;
    Ok(sha.to_string())
}

/// The readiness state the `fc-readiness` fail-closed rule acts on: counts of
/// pending evidence and blocked divergences across the whole manifest.
pub fn readiness(json: &str) -> Result<(usize, usize)> {
    let manifest: Manifest = serde_json::from_str(json).context("parsing the manifest")?;
    let pending = manifest
        .promise
        .values()
        .flat_map(|area| area.evidence.iter())
        .filter(|entry| entry.status == "pending")
        .count();
    let blocked = manifest
        .divergences
        .iter()
        .filter(|divergence| divergence.profile_action == "blocked")
        .count();
    Ok((pending, blocked))
}

pub fn command(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        println!(
            "Usage:\n  cargo xtask compat verify   Verify compat/profile.json against the schema, clonk-core, and the content pin."
        );
        return Ok(());
    }
    if args[0] != "verify" || args.len() > 1 {
        bail!(
            "unknown `compat` subcommand `{}` (try `cargo xtask compat --help`)",
            args[0]
        );
    }

    let workspace_dir = workspace_dir()?;
    let manifest_path = workspace_dir.join("compat/profile.json");
    let json = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let expected_content = content_submodule_commit(&workspace_dir)?;

    let mut issues = validate(&json, Some(&expected_content));
    issues.extend(validate_tree(&workspace_dir, &json));
    if !issues.is_empty() {
        for issue in &issues {
            println!("  {issue}");
        }
        bail!(
            "compat profile manifest failed verification with {} issue(s)",
            issues.len()
        );
    }

    xtask::presentation::verify_accepted_evidence(&workspace_dir)?;

    let manifest: Manifest = serde_json::from_str(&json)?;
    let (pending, blocked) = readiness(&json)?;
    println!("compat profile: {}", manifest_path.display());
    println!(
        "  pinned engine {} (matches clonk-core)",
        manifest.pinned.engine.text
    );
    println!("  pinned oracle {}", manifest.pinned.oracle_commit);
    println!("  pinned content {}", manifest.pinned.content_commit);
    for area in AREAS {
        let count = manifest
            .promise
            .get(area)
            .map(|section| section.evidence.len())
            .unwrap_or(0);
        println!("  promise[{area}]: {count} evidence entries");
    }
    println!("  divergences: {}", manifest.divergences.len());
    println!(
        "  port-only features: {}",
        manifest.port_only_features.len()
    );
    println!("  fail-closed rules: {}", manifest.fail_closed.len());
    if pending == 0 && blocked == 0 {
        println!("  readiness: satisfiable — the profile may be advertised as compatible");
    } else {
        println!(
            "  readiness: {pending} pending evidence entries and {blocked} blocked divergences \
             — the profile must not be advertised as compatible (fc-readiness)"
        );
    }
    println!("compat verify: OK");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    fn shipped_manifest() -> String {
        let path = repo_root().join("compat/profile.json");
        std::fs::read_to_string(&path).expect("reading the shipped manifest")
    }

    /// Parse the shipped manifest, mutate it through `serde_json::Value`, and
    /// return the tampered JSON text.
    /// The id of some `open-gap` divergence in the shipped manifest.
    ///
    /// These tamper cases care that the rule fires, not which entry it fires
    /// on, and hardcoding an id makes every one of them a landmine for whoever
    /// closes that gap: clonk-org/clonk-rs#1092 repointed two of them onto
    /// `sim-findobject-layer-bbox` days before clonk-org/clonk-rs#1095 removed
    /// it, and the collision only surfaced in the merge queue.
    const SYNTHETIC_OPEN_GAP_ID: &str = "test-synthetic-open-gap";

    /// A well-formed open gap for the tamper cases to mutate.
    ///
    /// These cases care that the rule fires, not which entry it fires on. They
    /// used to *find* an open gap in the shipped manifest, which made every one
    /// of them a landmine for whoever closed that gap -- and closing the last
    /// one (clonk-org/clonk-rs#1094) tripped exactly that. The shipped manifest
    /// now records **no** open gap: every divergence is closed or accepted.
    ///
    /// So synthesize one. The contract must not have to keep a defect on the
    /// books for its own tests' benefit, and a rule about open gaps is worth
    /// testing whether or not one currently exists.
    fn synthetic_open_gap() -> serde_json::Value {
        serde_json::json!({
            "id": SYNTHETIC_OPEN_GAP_ID,
            "area": "simulation",
            "summary": "A synthetic open gap injected by the tamper tests.",
            "cited_in": ["xtask/src/compat_profile.rs"],
            "cpp_reference": "C4Game.cpp:1",
            "disposition": "open-gap",
            "profile_action": "blocked",
            "determinism_critical": true,
            "owner": "clonk-org/clonk-rs#1094"
        })
    }

    /// `synthetic_open_gap`, mutated and appended to the shipped manifest.
    fn tampered_with_open_gap(mutate: impl FnOnce(&mut serde_json::Value)) -> String {
        tampered(|value| {
            let mut gap = synthetic_open_gap();
            mutate(&mut gap);
            value["divergences"]
                .as_array_mut()
                .expect("divergences is an array")
                .push(gap);
        })
    }

    fn tampered(mutate: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut value: serde_json::Value =
            serde_json::from_str(&shipped_manifest()).expect("parsing the shipped manifest");
        mutate(&mut value);
        serde_json::to_string(&value).expect("serializing the tampered manifest")
    }

    #[test]
    fn the_shipped_manifest_is_valid() {
        // `None` skips only the content-pin cross-check, which the next test
        // asserts against the real submodule pin.
        let issues = validate(&shipped_manifest(), None);
        assert!(
            issues.is_empty(),
            "the shipped manifest must validate clean, got: {issues:?}"
        );
    }

    #[test]
    fn the_content_pin_matches_the_submodule() {
        // The profile pins a content commit, and a content bump must drag the
        // manifest along with it: the pin in the manifest must be the pin in
        // the tree, or the contract silently describes other content.
        let expected = content_submodule_commit(&repo_root()).expect("reading the submodule pin");
        let issues = validate(&shipped_manifest(), Some(&expected));
        assert!(
            issues.is_empty(),
            "content pin must match the submodule, got: {issues:?}"
        );
    }

    #[test]
    fn a_drifted_content_pin_is_rejected() {
        let drifted = tampered(|value| {
            value["pinned"]["content_commit"] = "0000000000000000000000000000000000000000".into();
        });
        let issues = validate(&drifted, Some("1111111111111111111111111111111111111111"));
        assert!(
            issues.iter().any(|issue| issue.contains("drifted")),
            "a content pin that differs from the submodule must fail, got: {issues:?}"
        );
    }

    #[test]
    fn the_engine_pin_must_match_clonk_core() {
        let tampered = tampered(|value| {
            value["pinned"]["engine"]["xver"][2] = 12.into();
        });
        let issues = validate(&tampered, None);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("ENGINE_VERSION") && issue.contains("does not match")),
            "an engine pin that differs from clonk-core must fail, got: {issues:?}"
        );
    }

    #[test]
    fn the_oracle_commit_must_be_the_pinned_snapshot() {
        let tampered = tampered(|value| {
            let commit = value["pinned"]["oracle_commit"]
                .as_str()
                .expect("the oracle pin is a string")
                .to_string();
            value["pinned"]["oracle_commit"] = format!("0{}0", &commit[1..39]).into();
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| issue.contains("oracle commit")),
            "an oracle pin other than the instrumented snapshot must fail, got: {issues:?}"
        );
    }

    #[test]
    fn the_schema_must_be_current() {
        let tampered = tampered(|value| {
            value["schema"] = "clonk-rs/compat-profile/v2".into();
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| issue.contains("schema")),
            "an unknown schema must fail, got: {issues:?}"
        );
    }

    #[test]
    fn every_promise_area_is_required() {
        let tampered = tampered(|value| {
            value["promise"]
                .as_object_mut()
                .unwrap()
                .remove("transport");
        });
        let issues = validate(&tampered, None);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("`transport` section")),
            "a missing promise area must fail, got: {issues:?}"
        );
    }

    #[test]
    fn an_unlisted_promise_area_is_rejected() {
        let tampered = tampered(|value| {
            let area = value
                .get("promise")
                .and_then(|promise| promise.get("simulation"))
                .cloned()
                .expect("the simulation area exists to clone");
            value["promise"]["physics"] = area;
        });
        let issues = validate(&tampered, None);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("not a contract area")),
            "an extra promise area must fail, got: {issues:?}"
        );
    }

    #[test]
    fn pending_evidence_must_cite_a_qualified_issue() {
        let tampered = tampered(|value| {
            value["promise"]["save_replay"]["evidence"][2]["status"] = "pending".into();
            value["promise"]["save_replay"]["evidence"][2]["value"] = "the C++ oracle".into();
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| issue.contains("not a qualified")),
            "pending evidence without a clonk-org/clonk-rs#N reference must fail, got: {issues:?}"
        );
    }

    #[test]
    fn an_open_gap_may_not_be_part_of_the_promise() {
        let tampered = tampered_with_open_gap(|gap| {
            gap["profile_action"] = "kept".into();
        });
        let issues = validate(&tampered, None);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains(SYNTHETIC_OPEN_GAP_ID)
                    && issue.contains("must be `blocked`")),
            "an open-gap divergence marked `kept` must fail, got: {issues:?}"
        );
    }

    #[test]
    fn an_accepted_divergence_may_not_be_blocked() {
        let tampered = tampered(|value| {
            for divergence in value["divergences"].as_array_mut().unwrap() {
                if divergence["id"] == "transport-presend-envelope" {
                    divergence["profile_action"] = "blocked".into();
                }
            }
        });
        let issues = validate(&tampered, None);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("transport-presend-envelope")
                    && issue.contains("must be `kept` or `reverted`")),
            "an accepted divergence marked `blocked` must fail, got: {issues:?}"
        );
    }

    #[test]
    fn divergence_ids_are_unique() {
        let tampered = tampered(|value| {
            for divergence in value["divergences"].as_array_mut().unwrap() {
                if divergence["id"] == "sim-s2-terminal-params" {
                    divergence["id"] = "sim-pxs-syncclearance".into();
                }
            }
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| issue.contains("duplicate id")),
            "a repeated divergence id must fail, got: {issues:?}"
        );
    }

    #[test]
    fn feature_ids_may_not_collide_with_divergence_ids() {
        let tampered = tampered(|value| {
            for feature in value["port_only_features"].as_array_mut().unwrap() {
                if feature["id"] == "local-voice-chat" {
                    feature["id"] = "save-mission-access".into();
                }
            }
        });
        let issues = validate(&tampered, None);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("collides with a divergence id")),
            "a feature id reusing a divergence id must fail, got: {issues:?}"
        );
    }

    #[test]
    fn owners_must_cite_issues_qualified() {
        let tampered = tampered_with_open_gap(|gap| {
            gap["owner"] = "#384".into();
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| issue.contains("unqualified")),
            "a bare `#N` owner must fail; the repo is public and #N is ambiguous",
        );
    }

    #[test]
    fn every_divergence_needs_an_owner_and_a_citation() {
        let no_owner = tampered(|value| {
            for divergence in value["divergences"].as_array_mut().unwrap() {
                if divergence["id"] == "sim-game-tick-38fps" {
                    divergence["owner"] = "".into();
                }
            }
        });
        let issues = validate(&no_owner, None);
        assert!(
            issues.iter().any(|issue| issue.contains("no owner")),
            "a divergence without an owner must fail, got: {issues:?}"
        );

        let no_citation = tampered(|value| {
            for divergence in value["divergences"].as_array_mut().unwrap() {
                if divergence["id"] == "sim-game-tick-38fps" {
                    divergence["cited_in"] = serde_json::json!([]);
                }
            }
        });
        let issues = validate(&no_citation, None);
        assert!(
            issues.iter().any(|issue| issue.contains("no citation")),
            "a divergence without a source citation must fail, got: {issues:?}"
        );
    }

    #[test]
    fn fail_closed_rules_name_the_combination_and_the_behavior() {
        let tampered = tampered(|value| {
            for rule in value["fail_closed"].as_array_mut().unwrap() {
                if rule["id"] == "fc-save-version" {
                    rule["behavior"] = "".into();
                }
            }
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| issue.contains("fc-save-version")),
            "a fail-closed rule without a stated behavior must fail, got: {issues:?}"
        );
    }

    #[test]
    fn the_readiness_rule_is_machine_readable() {
        let tampered = tampered(|value| {
            value["fail_closed"] = value["fail_closed"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|rule| rule["id"] != "fc-readiness")
                .cloned()
                .collect::<Vec<_>>()
                .into();
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| issue.contains("fc-readiness")),
            "removing the fc-readiness rule must fail, got: {issues:?}"
        );
    }

    #[test]
    fn readiness_counts_pending_evidence_and_blocked_divergences() {
        let (pending, blocked) = readiness(&shipped_manifest()).expect("readiness");
        // The shipped contract deliberately carries pending issue evidence,
        // so it must currently report the profile as not advertisable.
        assert!(pending > 0, "pending evidence entries must be counted");
        // Every open gap is now closed or accepted (clonk-org/clonk-rs#1094), so
        // the shipped manifest reports none. That is the goal, not a broken
        // fixture -- but the counting still has to work, so prove it on a
        // manifest that does carry one rather than requiring the contract to.
        assert_eq!(blocked, 0, "the shipped manifest records no open gap");
        let (_, injected) =
            readiness(&tampered_with_open_gap(|_| {})).expect("readiness of the tampered manifest");
        assert_eq!(injected, 1, "blocked open-gap divergences must be counted");
    }

    #[test]
    fn an_evidence_note_that_says_nothing_is_rejected() {
        let tampered = tampered(|value| {
            value["promise"]["content"]["evidence"][0]["note"] = "".into();
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| issue.contains("note")),
            "an empty evidence note must fail, got: {issues:?}"
        );
    }

    #[test]
    fn a_determinism_critical_divergence_must_name_its_cpp_reference() {
        let tampered = tampered(|value| {
            for divergence in value["divergences"].as_array_mut().unwrap() {
                if divergence["id"] == "sim-pxs-syncclearance" {
                    divergence["cpp_reference"] = serde_json::Value::Null;
                }
            }
        });
        let issues = validate(&tampered, None);
        assert!(
            issues.iter().any(|issue| {
                issue.contains("sim-pxs-syncclearance") && issue.contains("cpp_reference")
            }),
            "a sync-relevant divergence that never says what C++ does must fail, got: {issues:?}"
        );
    }

    #[test]
    fn every_fail_closed_rule_must_state_its_basis() {
        let tampered = tampered(|value| {
            for rule in value["fail_closed"].as_array_mut().unwrap() {
                if rule["id"] == "fc-build-mismatch" {
                    rule["basis"] = serde_json::Value::Null;
                }
            }
        });
        let issues = validate(&tampered, None);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("fc-build-mismatch") && issue.contains("basis")),
            "a fail-closed rule resting on nothing must fail, got: {issues:?}"
        );
    }

    #[test]
    fn the_shipped_contract_resolves_against_the_tree() {
        let issues = validate_tree(&repo_root(), &shipped_manifest());
        assert!(
            issues.is_empty(),
            "every cited path must resolve and every entry be documented, got: {issues:?}"
        );
    }

    #[test]
    fn a_citation_that_names_nothing_is_rejected() {
        let tampered = tampered(|value| {
            for divergence in value["divergences"].as_array_mut().unwrap() {
                if divergence["id"] == "sim-game-tick-38fps" {
                    divergence["cited_in"] =
                        serde_json::json!(["crates/clonk-engine/src/renamed-away.rs:418"]);
                }
            }
        });
        let issues = validate_tree(&repo_root(), &tampered);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("sim-game-tick-38fps")),
            "a citation whose file was renamed away must fail, got: {issues:?}"
        );
    }

    #[test]
    fn the_human_documentation_must_exist() {
        let tampered = tampered(|value| {
            value["profile"]["document"] = "docs/NEVER_WRITTEN.md".into();
        });
        let issues = validate_tree(&repo_root(), &tampered);
        assert!(
            issues.iter().any(|issue| issue.contains("NEVER_WRITTEN")),
            "a manifest pointing at documentation that was never written must fail, \
             got: {issues:?}"
        );
    }

    #[test]
    fn a_manifest_entry_the_document_never_mentions_is_rejected() {
        let tampered = tampered(|value| {
            for divergence in value["divergences"].as_array_mut().unwrap() {
                if divergence["id"] == "sim-game-tick-38fps" {
                    divergence["id"] = "sim-difference-nobody-wrote-down".into();
                }
            }
        });
        let issues = validate_tree(&repo_root(), &tampered);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("sim-difference-nobody-wrote-down")),
            "a divergence missing from the human documentation must fail, got: {issues:?}"
        );
    }

    #[test]
    fn malformed_json_is_a_single_reported_issue_not_a_panic() {
        let issues = validate("{ not json", None);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("does not parse"));
    }
}
