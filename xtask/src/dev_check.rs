use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const HELP: &str = "\
Usage: cargo dev-check [options]\n\
       cargo xtask dev-check [options]\n\
\n\
Options:\n\
  --base REF             Include committed changes since merge-base(REF, HEAD).\n\
  --changed PATH         Add an explicit changed path (repeatable).\n\
  --plan                 Print and write the plan without executing it.\n\
  --full                 Run the broad workspace/snapshot/parity gates.\n\
  --keep-going           Continue after failed ordinary checks.\n\
  --budget-seconds N     Stop between commands when the budget is exhausted.\n\
  --artifacts PATH       Store report and command logs here.\n\
  -h, --help             Show this help.\n";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Options {
    base: Option<String>,
    changed: Vec<PathBuf>,
    plan_only: bool,
    full: bool,
    keep_going: bool,
    budget_seconds: Option<u64>,
    artifacts: Option<PathBuf>,
    help: bool,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut result = Self::default();
        let mut index = 0;
        while index < args.len() {
            let arg = &args[index];
            match arg.as_str() {
                "-h" | "--help" => result.help = true,
                "--plan" => result.plan_only = true,
                "--full" => result.full = true,
                "--keep-going" => result.keep_going = true,
                "--base" => {
                    index += 1;
                    let value = args.get(index).context("--base requires a ref")?;
                    set_once(&mut result.base, value.clone(), "--base")?;
                }
                "--changed" => {
                    index += 1;
                    let value = args.get(index).context("--changed requires a path")?;
                    require_nonempty(value, "--changed")?;
                    result.changed.push(PathBuf::from(value));
                }
                "--budget-seconds" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .context("--budget-seconds requires an integer")?;
                    let seconds = positive_seconds(value)?;
                    set_once(&mut result.budget_seconds, seconds, "--budget-seconds")?;
                }
                "--artifacts" => {
                    index += 1;
                    let value = args.get(index).context("--artifacts requires a path")?;
                    require_nonempty(value, "--artifacts")?;
                    set_once(&mut result.artifacts, PathBuf::from(value), "--artifacts")?;
                }
                _ if arg.starts_with("--base=") => set_once(
                    &mut result.base,
                    assignment(arg, "--base")?.to_string(),
                    "--base",
                )?,
                _ if arg.starts_with("--changed=") => {
                    result
                        .changed
                        .push(PathBuf::from(assignment(arg, "--changed")?));
                }
                _ if arg.starts_with("--budget-seconds=") => {
                    let seconds = positive_seconds(assignment(arg, "--budget-seconds")?)?;
                    set_once(&mut result.budget_seconds, seconds, "--budget-seconds")?;
                }
                _ if arg.starts_with("--artifacts=") => set_once(
                    &mut result.artifacts,
                    PathBuf::from(assignment(arg, "--artifacts")?),
                    "--artifacts",
                )?,
                _ => bail!("unknown dev-check argument '{arg}' (try --help)"),
            }
            index += 1;
        }
        Ok(result)
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<()> {
    if slot.is_some() {
        bail!("{flag} may only be specified once");
    }
    *slot = Some(value);
    Ok(())
}

fn assignment<'a>(arg: &'a str, flag: &str) -> Result<&'a str> {
    let value = arg.split_once('=').map(|(_, value)| value).unwrap_or("");
    require_nonempty(value, flag)?;
    Ok(value)
}

fn require_nonempty(value: &str, flag: &str) -> Result<()> {
    if value.is_empty() {
        bail!("{flag} value must not be empty");
    }
    Ok(())
}

fn positive_seconds(value: &str) -> Result<u64> {
    let seconds = value
        .parse::<u64>()
        .with_context(|| format!("invalid --budget-seconds value '{value}'"))?;
    if seconds == 0 {
        bail!("--budget-seconds must be greater than zero");
    }
    Ok(seconds)
}

#[derive(Debug, Clone)]
struct WorkspacePaths {
    repo_root: PathBuf,
    workspace_dir: PathBuf,
}

impl WorkspacePaths {
    fn detect() -> Result<Self> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_dir = manifest
            .parent()
            .context("xtask manifest has no workspace parent")?
            .to_path_buf();
        // The workspace was hoisted to the repository root, so the two coincide.
        let repo_root = workspace_dir.clone();
        Ok(Self {
            repo_root,
            workspace_dir,
        })
    }

    fn artifact_dir(&self, configured: Option<&Path>) -> Result<PathBuf> {
        let path = match configured {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => self.repo_root.join(path),
            None => {
                let seconds = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                self.workspace_dir
                    .join("target/dev-check")
                    .join(format!("{seconds}-{}", std::process::id()))
            }
        };
        fs::create_dir_all(&path)
            .with_context(|| format!("creating artifact directory {}", path.display()))?;
        Ok(path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChangeSet {
    paths: Vec<String>,
    diff_base: String,
    resolved_base: Option<String>,
}

fn collect_changes(options: &Options, roots: &WorkspacePaths) -> Result<ChangeSet> {
    let mut paths = BTreeSet::new();
    for path in &options.changed {
        paths.insert(normalize_repo_path(path, &roots.repo_root)?);
    }

    let explicit_only = !options.changed.is_empty() && options.base.is_none();
    let mut diff_base = "HEAD".to_string();
    let mut resolved_base = None;
    if !explicit_only {
        if let Some(base) = options.base.as_deref() {
            let rev_arg = format!("{base}^{{commit}}");
            let resolved = git_text(&roots.repo_root, &["rev-parse", "--verify", &rev_arg])?;
            let merge_base = git_text(&roots.repo_root, &["merge-base", "HEAD", resolved.trim()])?;
            let merge_base = merge_base.trim().to_string();
            if merge_base.is_empty() {
                bail!("no merge base between HEAD and '{base}'");
            }
            let range = format!("{merge_base}..HEAD");
            paths.extend(git_changed(
                &roots.repo_root,
                &["diff", "--name-status", "-z", &range],
            )?);
            diff_base = merge_base;
            resolved_base = Some(resolved.trim().to_string());
        }
        paths.extend(git_changed(
            &roots.repo_root,
            &["diff", "--name-status", "-z"],
        )?);
        paths.extend(git_changed(
            &roots.repo_root,
            &["diff", "--cached", "--name-status", "-z"],
        )?);
        paths.extend(git_nul_paths(
            &roots.repo_root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )?);
    }
    Ok(ChangeSet {
        paths: paths.into_iter().collect(),
        diff_base,
        resolved_base,
    })
}

fn normalize_repo_path(path: &Path, repo_root: &Path) -> Result<String> {
    let relative = if path.is_absolute() {
        path.strip_prefix(repo_root).with_context(|| {
            format!(
                "changed path {} is outside repository {}",
                path.display(),
                repo_root.display()
            )
        })?
    } else {
        path
    };
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                bail!("changed path may not contain '..': {}", path.display())
            }
            Component::RootDir | Component::Prefix(_) => {
                bail!("could not normalize changed path {}", path.display())
            }
        }
    }
    if parts.is_empty() {
        bail!("changed path must name a file or directory");
    }
    Ok(parts.join("/"))
}

fn git_output(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn git_text(repo_root: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(git_output(repo_root, args)?).context("git emitted non-UTF-8 text")
}

fn git_changed(repo_root: &Path, args: &[&str]) -> Result<Vec<String>> {
    parse_name_status_z(&git_output(repo_root, args)?)
}

fn git_nul_paths(repo_root: &Path, args: &[&str]) -> Result<Vec<String>> {
    parse_nul_strings(&git_output(repo_root, args)?)
}

fn parse_nul_strings(bytes: &[u8]) -> Result<Vec<String>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8(field.to_vec()).context("git emitted a non-UTF-8 path"))
        .collect()
}

fn parse_name_status_z(bytes: &[u8]) -> Result<Vec<String>> {
    let fields = parse_nul_strings(bytes)?;
    let mut paths = BTreeSet::new();
    let mut index = 0;
    while index < fields.len() {
        let status_field = &fields[index];
        index += 1;
        let (status, embedded) = status_field
            .split_once('\t')
            .map(|(status, path)| (status, Some(path)))
            .unwrap_or((status_field.as_str(), None));
        let first = embedded.map(str::to_string).unwrap_or_else(|| {
            let path = fields.get(index).cloned().unwrap_or_default();
            index += 1;
            path
        });
        if first.is_empty() {
            bail!("git name-status output is missing a path after '{status}'");
        }
        paths.insert(first);
        if status.starts_with('R') || status.starts_with('C') {
            let second = fields
                .get(index)
                .cloned()
                .ok_or_else(|| anyhow!("git rename/copy is missing its destination"))?;
            index += 1;
            paths.insert(second);
        }
    }
    Ok(paths.into_iter().collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckKind {
    Replay,
    RenderProbe,
    Hygiene,
    Unit,
    Integration,
    Headless,
    Snapshot,
    Parity,
    Workspace,
}

impl CheckKind {
    fn label(self) -> &'static str {
        match self {
            Self::Replay => "deterministic-replay",
            Self::RenderProbe => "render-probe",
            Self::Hygiene => "hygiene",
            Self::Unit => "unit",
            Self::Integration => "integration",
            Self::Headless => "headless-scenario",
            Self::Snapshot => "snapshot",
            Self::Parity => "parity",
            Self::Workspace => "workspace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckCwd {
    Repo,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedCommand {
    id: String,
    kind: CheckKind,
    cwd: CheckCwd,
    program: String,
    args: Vec<String>,
    reasons: BTreeSet<String>,
    diagnostic_after_failure: bool,
}

impl PlannedCommand {
    fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_word)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn shell_word(word: &str) -> String {
    if word
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-._/:".contains(&byte))
    {
        word.to_string()
    } else {
        format!("'{word}'", word = word.replace('\'', "'\\''"))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CheckPlan {
    commands: Vec<PlannedCommand>,
}

impl CheckPlan {
    fn add(
        &mut self,
        id: impl Into<String>,
        kind: CheckKind,
        cwd: CheckCwd,
        program: &str,
        args: &[&str],
        reason: impl Into<String>,
    ) {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        if let Some(existing) = self
            .commands
            .iter_mut()
            .find(|check| check.cwd == cwd && check.program == program && check.args == args)
        {
            existing.reasons.insert(reason.into());
            return;
        }
        self.commands.push(PlannedCommand {
            id: id.into(),
            kind,
            cwd,
            program: program.to_string(),
            args,
            reasons: BTreeSet::from([reason.into()]),
            diagnostic_after_failure: kind == CheckKind::RenderProbe,
        });
    }

    #[cfg(test)]
    fn has_args(&self, args: &[&str]) -> bool {
        let expected: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        self.commands
            .iter()
            .any(|check| check.program == "cargo" && check.args == expected)
    }
}

fn plan_checks(changes: &ChangeSet, options: &Options) -> CheckPlan {
    let mut plan = CheckPlan::default();
    let gameplay = options.full || changes.paths.iter().any(|path| gameplay_path(path));
    if gameplay {
        add_replay_and_render(&mut plan, "gameplay-affecting changes");
    }
    plan.add(
        "diff-check",
        CheckKind::Hygiene,
        CheckCwd::Repo,
        "git",
        &["diff", "--check", &changes.diff_base],
        "always check changed text for whitespace errors",
    );

    let content_changed = changes.paths.iter().any(|path| {
        path == "content" || path.starts_with("content/") || path.starts_with("planet/System.c4g/")
    });
    if options.full {
        add_workspace(&mut plan, "--full requested");
        if content_changed {
            add_sweep(&mut plan, None, "--full with content changes");
        }
        return plan;
    }

    let workspace = changes.paths.iter().any(|path| {
        matches!(
            path.as_str(),
            "Cargo.toml" | "Cargo.lock" | ".cargo/config.toml"
        ) || path.starts_with(".cargo/")
    });
    if workspace {
        add_workspace(&mut plan, "Rust workspace configuration changed");
    }

    for path in &changes.paths {
        let reason = format!("changed {path}");
        if path.starts_with("testdata/engine/") || path == "crates/clonk-engine/src/fixtures.rs" {
            add_snapshots(&mut plan, &reason);
        }
        if path.starts_with("parity/") || path == "crates/clonk-engine/src/parity_differential.rs" {
            add_parity(&mut plan, &reason);
        }
        plan_content(&mut plan, path, &reason);
        if workspace || plan_test_path(&mut plan, path, &reason) {
            continue;
        }
        if path.starts_with("xtask/") {
            add_package(&mut plan, "xtask", &reason);
        } else if path.starts_with("crates/clonk-engine/src/") {
            add_engine_checks(&mut plan, path, &reason);
        } else if path.starts_with("crates/clonk-script/src/") {
            add_script_checks(&mut plan, &reason);
        } else if path.starts_with("crates/clonk-resources/src/") {
            add_package(&mut plan, "clonk-resources", &reason);
            add_engine_filter(
                &mut plan,
                "resource-scenario-loading",
                "legacy_scenario_loading::",
                CheckKind::Integration,
                &reason,
            );
            add_engine_filter(
                &mut plan,
                "resource-manifests",
                "manifest_definitions::",
                CheckKind::Integration,
                &reason,
            );
        } else if let Some(package) = crate_package(path) {
            add_package(&mut plan, &package, &reason);
        }
    }
    plan
}

fn gameplay_path(path: &str) -> bool {
    path.starts_with("crates/clonk-engine/src/")
        || path.starts_with("crates/clonk-script/src/")
        || path.starts_with("crates/clonk-resources/src/")
        || path.starts_with("crates/clonk-frontend/src/")
        || path.starts_with("crates/clonk-app/src/")
        || path == "content"
        || path.starts_with("content/")
        || path.starts_with("planet/System.c4g/")
        || path.starts_with("testdata/dev-replays/")
}

fn replay_test_name(recording_host: bool) -> &'static str {
    if recording_host {
        "dev_feedback_replay::committed_real_scenario_replays_are_deterministic"
    } else {
        "dev_feedback_replay::real_scenario_replays_repeat_with_native_group_order"
    }
}

fn add_replay_and_render(plan: &mut CheckPlan, reason: &str) {
    let replay_test = replay_test_name(cfg!(target_os = "macos"));
    plan.add(
        "deterministic-replay",
        CheckKind::Replay,
        CheckCwd::Workspace,
        "cargo",
        &[
            "nextest",
            "run",
            "-p",
            "clonk-engine-integration-tests",
            "--test",
            "engine_it",
            "--",
            replay_test,
            "--exact",
        ],
        reason,
    );
    plan.add(
        "frontend-render-probe",
        CheckKind::RenderProbe,
        CheckCwd::Workspace,
        "cargo",
        &[
            "nextest",
            "run",
            "-p",
            "clonk-frontend",
            "--features",
            "dev-feedback-render",
            "--test",
            "dev_feedback_render",
            "--",
            "dev_feedback_render",
            "--ignored",
            "--exact",
        ],
        "render the deterministic replay snapshot",
    );
}

fn plan_test_path(plan: &mut CheckPlan, path: &str, reason: &str) -> bool {
    let Some((prefix, tail)) = path.split_once("/tests/") else {
        return false;
    };
    let Some(package) = crate_package(&format!("{prefix}/src/lib.rs")) else {
        return false;
    };
    let parts: Vec<&str> = tail.split('/').collect();
    match parts.as_slice() {
        [file] if file.ends_with(".rs") => {
            let target = file.trim_end_matches(".rs");
            add_test_target(plan, &package, target, None, reason);
        }
        [target, "main.rs"] => add_test_target(plan, &package, target, None, reason),
        ["it", "support", "real_scenario.rs"] => {
            add_engine_filter(
                plan,
                "real-scenario-support",
                "real_",
                CheckKind::Headless,
                reason,
            );
        }
        ["it", "support", "virtual_player.rs"] => {
            add_engine_filter(
                plan,
                "tutorial-support",
                "real_tutorial",
                CheckKind::Headless,
                reason,
            );
            add_engine_filter(
                plan,
                "virtual-player-support",
                "virtual_player_harness::",
                CheckKind::Integration,
                reason,
            );
        }
        [target, "support", ..] => add_test_target(plan, &package, target, None, reason),
        [target, module] if module.ends_with(".rs") => {
            let module = module.trim_end_matches(".rs");
            add_test_target(plan, &package, target, Some(&format!("{module}::")), reason);
        }
        _ => add_package(plan, &package, reason),
    }
    true
}

fn add_test_target(
    plan: &mut CheckPlan,
    package: &str,
    target: &str,
    filter: Option<&str>,
    reason: &str,
) {
    let (package, target) = if package == "clonk-engine" && target == "it" {
        ("clonk-engine-integration-tests", "engine_it")
    } else {
        (package, target)
    };
    let mut args = vec!["nextest", "run", "-p", package, "--test", target];
    if let Some(filter) = filter {
        args.push(filter);
    }
    plan.add(
        format!("{package}-{target}-{}", filter.unwrap_or("all")),
        if filter.is_some_and(|filter| filter.starts_with("real_")) {
            CheckKind::Headless
        } else {
            CheckKind::Integration
        },
        CheckCwd::Workspace,
        "cargo",
        &args,
        reason,
    );
}

fn add_engine_checks(plan: &mut CheckPlan, path: &str, reason: &str) {
    plan.add(
        "clonk-engine-inline",
        CheckKind::Unit,
        CheckCwd::Workspace,
        "cargo",
        &[
            "nextest",
            "run",
            "-p",
            "clonk-engine-unit-tests",
            "--test",
            "engine_inline",
        ],
        reason,
    );
    plan.add(
        "clonk-engine-unit",
        CheckKind::Unit,
        CheckCwd::Workspace,
        "cargo",
        &[
            "nextest",
            "run",
            "-p",
            "clonk-engine-unit-tests",
            "--test",
            "unit",
        ],
        reason,
    );
    let file = path.rsplit('/').next().unwrap_or("");
    match file {
        "scenario.rs" | "definition.rs" | "init_placement.rs" | "player.rs" | "player_file.rs" => {
            add_engine_filter(
                plan,
                "scenario-loading",
                "legacy_scenario_loading::",
                CheckKind::Integration,
                reason,
            );
            add_engine_filter(
                plan,
                "manifest-definitions",
                "manifest_definitions::",
                CheckKind::Integration,
                reason,
            );
            add_engine_filter(
                plan,
                "real-scenario-smoke",
                "real_scenario_harness::",
                CheckKind::Headless,
                reason,
            );
        }
        "landscape.rs" | "map_creator.rs" | "map_creator_s2.rs" | "mass_mover.rs" | "pxs.rs" => {
            for (id, filter) in [
                ("walk-movement", "walk_movement::"),
                ("flight-movement", "flight_movement::"),
                ("hangle-movement", "hangle_movement::"),
                ("balloon-headless", "real_tutorial02_balloon_platform::"),
            ] {
                add_engine_filter(plan, id, filter, CheckKind::Integration, reason);
            }
        }
        "control.rs" | "input.rs" | "direct_com.rs" | "command.rs" => {
            add_engine_filter(
                plan,
                "action-procedure",
                "action_procedure::",
                CheckKind::Integration,
                reason,
            );
            add_engine_filter(
                plan,
                "virtual-player",
                "virtual_player_harness::",
                CheckKind::Headless,
                reason,
            );
        }
        "audio.rs" => {
            add_engine_filter(
                plan,
                "weather-audio",
                "weather_audio::",
                CheckKind::Integration,
                reason,
            );
            add_engine_filter(
                plan,
                "dragon-rock-audio",
                "dragon_rock_audio::",
                CheckKind::Headless,
                reason,
            );
        }
        "compat.rs" | "effect.rs" | "script_constants.rs" | "lib.rs" => {
            add_engine_filter(
                plan,
                "real-scenario-smoke",
                "real_scenario_harness::",
                CheckKind::Headless,
                reason,
            );
        }
        _ => {}
    }
}

fn add_script_checks(plan: &mut CheckPlan, reason: &str) {
    plan.add(
        "clonk-script-lib",
        CheckKind::Unit,
        CheckCwd::Workspace,
        "cargo",
        &["nextest", "run", "-p", "clonk-script", "--lib"],
        reason,
    );
    plan.add(
        "clonk-script-it",
        CheckKind::Integration,
        CheckCwd::Workspace,
        "cargo",
        &["nextest", "run", "-p", "clonk-script", "--test", "it"],
        reason,
    );
}

fn add_engine_filter(plan: &mut CheckPlan, id: &str, filter: &str, kind: CheckKind, reason: &str) {
    plan.add(
        id,
        kind,
        CheckCwd::Workspace,
        "cargo",
        &[
            "nextest",
            "run",
            "-p",
            "clonk-engine-integration-tests",
            "--test",
            "engine_it",
            filter,
        ],
        reason,
    );
}

fn add_package(plan: &mut CheckPlan, package: &str, reason: &str) {
    // Crates with `[lib] test = false` own no test binary, so
    // `cargo nextest run -p <them>` compiles and then exits non-zero with "no
    // tests to run". Route each to the companion crate that actually mounts its
    // inline `#[cfg(test)]` modules; `clonk-logging` has no companion at all, so
    // it gets no per-package unit command rather than a guaranteed failure.
    const INLINE_COMPANIONS: [(&str, &str, &str, &str); 2] = [
        (
            "clonk-frontend",
            "clonk-frontend-inline",
            "clonk-frontend-unit-tests",
            "frontend_inline",
        ),
        (
            "clonk-engine",
            "clonk-engine-inline",
            "clonk-engine-unit-tests",
            "engine_inline",
        ),
    ];
    if let Some((_, id, companion, target)) = INLINE_COMPANIONS
        .iter()
        .find(|(name, ..)| *name == package)
        .copied()
    {
        plan.add(
            id,
            CheckKind::Unit,
            CheckCwd::Workspace,
            "cargo",
            &["nextest", "run", "-p", companion, "--test", target],
            reason,
        );
        return;
    }
    if package == "clonk-logging" {
        return;
    }
    plan.add(
        format!("{package}-tests"),
        CheckKind::Unit,
        CheckCwd::Workspace,
        "cargo",
        &["nextest", "run", "-p", package],
        reason,
    );
}

fn add_workspace(plan: &mut CheckPlan, reason: &str) {
    plan.add(
        "workspace-tests",
        CheckKind::Workspace,
        CheckCwd::Workspace,
        "cargo",
        &["nextest", "run", "--workspace"],
        reason,
    );
}

fn add_snapshots(plan: &mut CheckPlan, reason: &str) {
    plan.add(
        "engine-snapshots",
        CheckKind::Snapshot,
        CheckCwd::Workspace,
        "cargo",
        &["xtask", "engine-snapshots", "verify"],
        reason,
    );
}

fn add_parity(plan: &mut CheckPlan, reason: &str) {
    plan.add(
        "parity",
        CheckKind::Parity,
        CheckCwd::Workspace,
        "cargo",
        &["xtask", "parity", "verify"],
        reason,
    );
}

fn plan_content(plan: &mut CheckPlan, path: &str, reason: &str) {
    if let Some(scenario) = scenario_ancestor(path) {
        add_sweep(plan, Some(&scenario), reason);
        add_known_scenario(plan, &scenario, reason);
    } else if is_shared_content(path) {
        add_engine_filter(
            plan,
            "shared-content-smoke",
            "real_scenario_harness::",
            CheckKind::Headless,
            reason,
        );
        if path == "planet/System.c4g/LanguageUS.txt" {
            add_test_target(plan, "clonk-core", "language_fixture", None, reason);
        }
    } else if let Some(pack) = content_pack(path) {
        add_sweep(plan, Some(&pack), reason);
    }
}

fn add_sweep(plan: &mut CheckPlan, filter: Option<&str>, reason: &str) {
    let mut args = vec!["xtask", "scenario-sweep"];
    if let Some(filter) = filter {
        args.push(filter);
    }
    plan.add(
        filter
            .map(|filter| format!("scenario-sweep-{}", sanitize_id(filter)))
            .unwrap_or_else(|| "scenario-sweep-all".to_string()),
        CheckKind::Headless,
        CheckCwd::Workspace,
        "cargo",
        &args,
        reason,
    );
}

fn add_known_scenario(plan: &mut CheckPlan, scenario: &str, reason: &str) {
    let lower = scenario.to_ascii_lowercase();
    if let Some(tutorial) = lower
        .split('/')
        .find(|part| part.starts_with("tutorial") && part.ends_with(".c4s"))
    {
        let stem = tutorial.trim_end_matches(".c4s");
        add_engine_filter(
            plan,
            &format!("headless-{stem}"),
            &format!("{stem}_"),
            CheckKind::Headless,
            reason,
        );
    } else {
        let known = [
            ("fantasy.c4f/drachenfels.c4s", "dragon_rock"),
            ("races.c4f/monsterrescue.c4s", "monster_rescue"),
            ("knights.c4f/regicide.c4s", "regicide"),
            ("fantasy.c4f/alchemy.c4s", "real_alchemy_revision::"),
        ];
        if let Some((_, filter)) = known.iter().find(|(suffix, _)| lower.ends_with(suffix)) {
            add_engine_filter(
                plan,
                &format!("headless-{}", sanitize_id(filter)),
                filter,
                CheckKind::Headless,
                reason,
            );
        }
    }
}

fn scenario_ancestor(path: &str) -> Option<String> {
    let mut prefix = Vec::new();
    for part in path.split('/') {
        prefix.push(part);
        if part.to_ascii_lowercase().ends_with(".c4s") {
            return prefix
                .join("/")
                .strip_prefix("content/")
                .map(str::to_string);
        }
    }
    None
}

fn content_pack(path: &str) -> Option<String> {
    if !path.starts_with("content/") || is_shared_content(path) {
        return None;
    }
    path.split('/')
        .skip(1)
        .find(|part| part.to_ascii_lowercase().ends_with(".c4f"))
        .map(str::to_string)
}

fn is_shared_content(path: &str) -> bool {
    path == "content"
        || path.starts_with("content/Objects.c4d/")
        || path.starts_with("content/Material.c4g/")
        || path == "content/Material.c4g"
        || path.starts_with("planet/System.c4g/")
}

fn crate_package(path: &str) -> Option<String> {
    let mut parts = path.split('/');
    (parts.next()? == "crates")
        .then(|| parts.next().map(str::to_string))
        .flatten()
}

fn sanitize_id(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    result.trim_matches('-').to_string()
}

#[derive(Debug, Clone)]
struct RawRun {
    success: bool,
    exit_code: Option<i32>,
    duration: Duration,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug, Clone)]
struct RunRecord {
    id: String,
    success: bool,
    exit_code: Option<i32>,
    duration_ms: u128,
    cargo_build_ms: Option<u128>,
    stdout_log: String,
    stderr_log: String,
}

#[derive(Debug, Clone, Default)]
struct ExecutionSummary {
    runs: Vec<RunRecord>,
    skipped: Vec<String>,
    failures: usize,
    budget_exhausted: bool,
}

fn execute_plan(
    plan: &CheckPlan,
    options: &Options,
    roots: &WorkspacePaths,
    artifact_dir: &Path,
    changes: &ChangeSet,
) -> Result<ExecutionSummary> {
    execute_plan_with_runner(plan, options, roots, artifact_dir, changes, run_command)
}

fn execute_plan_with_runner<F>(
    plan: &CheckPlan,
    options: &Options,
    roots: &WorkspacePaths,
    artifact_dir: &Path,
    changes: &ChangeSet,
    mut runner: F,
) -> Result<ExecutionSummary>
where
    F: FnMut(&PlannedCommand, &Path, &Path) -> RawRun,
{
    let command_logs = artifact_dir.join("commands");
    fs::create_dir_all(&command_logs).context("creating command log directory")?;
    let mut summary = ExecutionSummary::default();
    let mut elapsed = Duration::ZERO;
    let budget = options.budget_seconds.map(Duration::from_secs);
    let mut halt_after_failure = false;

    for (index, check) in plan.commands.iter().enumerate() {
        if halt_after_failure {
            let snapshot_exists = find_artifact_file(artifact_dir, "snapshot-final.json").is_some();
            if !(check.diagnostic_after_failure && snapshot_exists) {
                summary
                    .skipped
                    .extend(plan.commands[index..].iter().map(|check| check.id.clone()));
                break;
            }
        }

        println!(
            "[{}/{}] {}: {}",
            index + 1,
            plan.commands.len(),
            check.kind.label(),
            check.display()
        );
        let cwd = match check.cwd {
            CheckCwd::Repo => roots.repo_root.as_path(),
            CheckCwd::Workspace => roots.workspace_dir.as_path(),
        };
        let run = runner(check, cwd, artifact_dir);
        if check.kind == CheckKind::Replay {
            promote_replay_artifacts(artifact_dir)?;
        }
        elapsed += run.duration;
        let stem = format!("{:02}-{}", index + 1, sanitize_id(&check.id));
        let stdout_relative = format!("commands/{stem}.stdout.log");
        let stderr_relative = format!("commands/{stem}.stderr.log");
        fs::write(artifact_dir.join(&stdout_relative), &run.stdout)
            .with_context(|| format!("writing {stdout_relative}"))?;
        fs::write(artifact_dir.join(&stderr_relative), &run.stderr)
            .with_context(|| format!("writing {stderr_relative}"))?;
        summary.runs.push(RunRecord {
            id: check.id.clone(),
            success: run.success,
            exit_code: run.exit_code,
            duration_ms: run.duration.as_millis(),
            cargo_build_ms: cargo_reported_build_ms(&run.stderr),
            stdout_log: stdout_relative,
            stderr_log: stderr_relative,
        });
        if run.success {
            println!("  passed in {:.2}s", run.duration.as_secs_f64());
        } else {
            summary.failures += 1;
            eprintln!(
                "  FAILED in {:.2}s (see {}, {})",
                run.duration.as_secs_f64(),
                summary.runs.last().unwrap().stdout_log,
                summary.runs.last().unwrap().stderr_log
            );
            if !options.keep_going {
                halt_after_failure = true;
            }
        }
        write_runtime_reports(artifact_dir, options, changes, plan, &summary, "running")?;

        let has_remaining = index + 1 < plan.commands.len();
        if halt_after_failure && check.diagnostic_after_failure {
            summary.skipped.extend(
                plan.commands[index + 1..]
                    .iter()
                    .map(|check| check.id.clone()),
            );
            break;
        }
        if halt_after_failure {
            let diagnostic_can_run = plan.commands.get(index + 1).is_some_and(|next| {
                next.diagnostic_after_failure
                    && find_artifact_file(artifact_dir, "snapshot-final.json").is_some()
            });
            if !diagnostic_can_run {
                summary.skipped.extend(
                    plan.commands[index + 1..]
                        .iter()
                        .map(|check| check.id.clone()),
                );
                break;
            }
        }

        let next_is_render = plan
            .commands
            .get(index + 1)
            .is_some_and(|next| next.kind == CheckKind::RenderProbe);
        if has_remaining && !next_is_render && budget.is_some_and(|budget| elapsed >= budget) {
            summary.budget_exhausted = true;
            summary.skipped.extend(
                plan.commands[index + 1..]
                    .iter()
                    .map(|check| check.id.clone()),
            );
            break;
        }
    }
    Ok(summary)
}

fn cargo_reported_build_ms(stderr: &[u8]) -> Option<u128> {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .find_map(|line| {
            let (_, duration) = line.split_once(" target(s) in ")?;
            line.contains("Finished `")
                .then(|| parse_cargo_duration_ms(duration))?
        })
}

fn parse_cargo_duration_ms(raw: &str) -> Option<u128> {
    let mut milliseconds = 0u128;
    let mut found = false;
    for token in raw.split_whitespace() {
        let token = token.trim_matches(|character: char| {
            !character.is_ascii_digit() && character != '.' && character != 'm' && character != 's'
        });
        if let Some(minutes) = token.strip_suffix('m') {
            milliseconds = milliseconds.checked_add(minutes.parse::<u128>().ok()? * 60_000)?;
            found = true;
        } else if let Some(seconds) = token.strip_suffix('s') {
            let seconds = seconds.parse::<f64>().ok()?;
            milliseconds = milliseconds.checked_add((seconds * 1_000.0).round() as u128)?;
            found = true;
        }
    }
    found.then_some(milliseconds)
}

fn run_command(check: &PlannedCommand, cwd: &Path, artifact_dir: &Path) -> RawRun {
    let started = Instant::now();
    let mut command = Command::new(&check.program);
    command
        .args(&check.args)
        .current_dir(cwd)
        .env("LC_TEST_ARTIFACT_DIR", artifact_dir)
        .env("LC_DEV_CHECK_ARTIFACT_DIR", artifact_dir)
        .env("LC_KEEP_PASS_ARTIFACTS", "1")
        .env(
            "LC_DEV_CHECK_RENDER_METRICS",
            artifact_dir.join("render-metrics.json"),
        )
        .env(
            "LC_DEV_CHECK_FRAME_PNG",
            artifact_dir.join("frame-final.png"),
        );
    if check.kind == CheckKind::RenderProbe {
        if let Some(snapshot) = find_artifact_file(artifact_dir, "snapshot-final.json") {
            command.env("LC_DEV_CHECK_SNAPSHOT", snapshot);
        }
    }
    let output = command.output();
    match output {
        Ok(output) => RawRun {
            success: output.status.success(),
            exit_code: output.status.code(),
            duration: started.elapsed(),
            stdout: output.stdout,
            stderr: output.stderr,
        },
        Err(error) => RawRun {
            success: false,
            exit_code: None,
            duration: started.elapsed(),
            stdout: Vec::new(),
            stderr: error.to_string().into_bytes(),
        },
    }
}

fn promote_replay_artifacts(artifact_dir: &Path) -> Result<()> {
    for (source_name, stable_name) in [
        ("final.json", "snapshot-final.json"),
        ("replay-metrics.json", "replay-metrics.json"),
    ] {
        let stable = artifact_dir.join(stable_name);
        if stable.is_file() {
            continue;
        }
        if let Some(source) = find_latest_artifact_file(artifact_dir, source_name) {
            fs::copy(&source, &stable).with_context(|| {
                format!(
                    "promoting replay artifact {} to {}",
                    source.display(),
                    stable.display()
                )
            })?;
        }
    }
    Ok(())
}

fn find_latest_artifact_file(root: &Path, filename: &str) -> Option<PathBuf> {
    let mut directories = vec![root.to_path_buf()];
    let mut candidates = Vec::new();
    while let Some(directory) = directories.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if path.file_name().is_some_and(|name| name == filename) {
                let modified = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(UNIX_EPOCH);
                candidates.push((modified, path));
            }
        }
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

fn find_artifact_file(root: &Path, filename: &str) -> Option<PathBuf> {
    let direct = root.join(filename);
    if direct.is_file() {
        return Some(direct);
    }
    let mut directories = vec![root.to_path_buf()];
    let mut candidates = Vec::new();
    while let Some(directory) = directories.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if path.file_name().is_some_and(|name| name == filename) {
                candidates.push(path);
            }
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

fn print_plan(changes: &ChangeSet, plan: &CheckPlan, artifact_dir: &Path) {
    println!("dev-check changed paths ({}):", changes.paths.len());
    for path in &changes.paths {
        println!("  {path}");
    }
    println!("dev-check plan ({} commands):", plan.commands.len());
    for (index, check) in plan.commands.iter().enumerate() {
        println!(
            "  {:02}. [{}] {}",
            index + 1,
            check.kind.label(),
            check.display()
        );
        for reason in &check.reasons {
            println!("      because {reason}");
        }
    }
    println!("artifacts: {}", artifact_dir.display());
}

fn write_manifest(
    artifact_dir: &Path,
    options: &Options,
    changes: &ChangeSet,
    plan: &CheckPlan,
) -> Result<()> {
    let mut json = String::from("{\n  \"schema_version\": 1,");
    json.push_str(&format!(
        "\n  \"options\": {{\"plan_only\": {}, \"full\": {}, \"keep_going\": {}, \"budget_seconds\": {}}},",
        options.plan_only,
        options.full,
        options.keep_going,
        optional_number(options.budget_seconds)
    ));
    json.push_str(&format!(
        "\n  \"diff_base\": {},\n  \"resolved_base\": {},",
        json_string(&changes.diff_base),
        changes
            .resolved_base
            .as_deref()
            .map(json_string)
            .unwrap_or_else(|| "null".to_string())
    ));
    json.push_str("\n  \"changed_paths\": ");
    push_string_array(&mut json, changes.paths.iter().map(String::as_str));
    json.push_str(",\n  \"commands\": [");
    for (index, check) in plan.commands.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "\n    {{\"id\": {}, \"kind\": {}, \"cwd\": {}, \"program\": {}, \"args\": ",
            json_string(&check.id),
            json_string(check.kind.label()),
            json_string(match check.cwd {
                CheckCwd::Repo => "repo",
                CheckCwd::Workspace => "workspace",
            }),
            json_string(&check.program)
        ));
        push_string_array(&mut json, check.args.iter().map(String::as_str));
        json.push_str(", \"reasons\": ");
        push_string_array(&mut json, check.reasons.iter().map(String::as_str));
        json.push_str(&format!(
            ", \"diagnostic_after_failure\": {}}}",
            check.diagnostic_after_failure
        ));
    }
    json.push_str("\n  ]\n}\n");
    fs::write(artifact_dir.join("manifest.json"), json).context("writing manifest.json")
}

fn write_runtime_reports(
    artifact_dir: &Path,
    options: &Options,
    changes: &ChangeSet,
    plan: &CheckPlan,
    summary: &ExecutionSummary,
    state: &str,
) -> Result<()> {
    write_manifest(artifact_dir, options, changes, plan)?;

    let replay_metrics = artifact_json(artifact_dir, "replay-metrics.json");
    let render_metrics = artifact_json(artifact_dir, "render-metrics.json");
    let mut timings = String::from("{\n  \"schema_version\": 1,\n  \"commands\": [");
    for (index, run) in summary.runs.iter().enumerate() {
        if index != 0 {
            timings.push(',');
        }
        timings.push_str(&format!(
            "\n    {{\"id\": {}, \"success\": {}, \"exit_code\": {}, \"duration_ms\": {}, \"cargo_build_ms\": {}, \"execution_after_build_ms\": {}, \"stdout_log\": {}, \"stderr_log\": {}}}",
            json_string(&run.id),
            run.success,
            run.exit_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
            run.duration_ms,
            run.cargo_build_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
            run.cargo_build_ms
                .map(|build| run.duration_ms.saturating_sub(build))
                .map(|value| value.to_string())
                .unwrap_or_else(|| "null".to_string()),
            json_string(&run.stdout_log),
            json_string(&run.stderr_log)
        ));
    }
    timings.push_str(&format!(
        "\n  ],\n  \"phase_metrics\": {{\"replay\": {}, \"render\": {}}}\n}}\n",
        replay_metrics, render_metrics
    ));
    fs::write(artifact_dir.join("timings.json"), timings).context("writing timings.json")?;

    let total_duration_ms: u128 = summary.runs.iter().map(|run| run.duration_ms).sum();
    let cargo_build_total_ms: u128 = summary
        .runs
        .iter()
        .filter_map(|run| run.cargo_build_ms)
        .sum();
    let slowest = summary.runs.iter().max_by_key(|run| run.duration_ms);
    let mut summary_json = format!(
        "{{\n  \"schema_version\": 1,\n  \"state\": {},\n  \"passed\": {},\n  \"failed\": {},\n  \"budget_exhausted\": {},\n  \"total_duration_ms\": {},\n  \"cargo_build_total_ms\": {},\n  \"slowest_command\": {},\n  \"slowest_command_ms\": {},\n  \"skipped\": ",
        json_string(state),
        summary.runs.iter().filter(|run| run.success).count(),
        summary.failures,
        summary.budget_exhausted,
        total_duration_ms,
        cargo_build_total_ms,
        slowest
            .map(|run| json_string(&run.id))
            .unwrap_or_else(|| "null".to_string()),
        slowest
            .map(|run| run.duration_ms.to_string())
            .unwrap_or_else(|| "null".to_string())
    );
    push_string_array(
        &mut summary_json,
        summary.skipped.iter().map(String::as_str),
    );
    summary_json.push_str("\n}\n");
    fs::write(artifact_dir.join("summary.json"), summary_json).context("writing summary.json")
}

fn artifact_json(artifact_dir: &Path, filename: &str) -> String {
    find_artifact_file(artifact_dir, filename)
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|contents| contents.trim().to_string())
        .filter(|contents| {
            (contents.starts_with('{') && contents.ends_with('}'))
                || (contents.starts_with('[') && contents.ends_with(']'))
        })
        .unwrap_or_else(|| "null".to_string())
}

fn optional_number(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn push_string_array<'a>(json: &mut String, values: impl Iterator<Item = &'a str>) {
    json.push('[');
    for (index, value) in values.enumerate() {
        if index != 0 {
            json.push_str(", ");
        }
        json.push_str(&json_string(value));
    }
    json.push(']');
}

fn json_string(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('"');
    for character in value.chars() {
        match character {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            character if character.is_control() => {
                result.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => result.push(character),
        }
    }
    result.push('"');
    result
}

pub fn command(args: &[String]) -> Result<()> {
    let options = Options::parse(args)?;
    if options.help {
        print!("{HELP}");
        return Ok(());
    }
    let roots = WorkspacePaths::detect()?;
    let changes = collect_changes(&options, &roots)?;
    let plan = plan_checks(&changes, &options);
    let artifacts = roots.artifact_dir(options.artifacts.as_deref())?;
    env::set_var("LC_TEST_ARTIFACT_DIR", &artifacts);
    env::set_var("LC_DEV_CHECK_ARTIFACT_DIR", &artifacts);
    env::set_var(
        "LC_DEV_CHECK_RENDER_METRICS",
        artifacts.join("render-metrics.json"),
    );
    print_plan(&changes, &plan, &artifacts);

    let mut summary = ExecutionSummary::default();
    write_runtime_reports(
        &artifacts,
        &options,
        &changes,
        &plan,
        &summary,
        if options.plan_only {
            "plan_only"
        } else {
            "planned"
        },
    )?;
    if options.plan_only {
        return Ok(());
    }

    summary = execute_plan(&plan, &options, &roots, &artifacts, &changes)?;
    let state = if summary.failures > 0 {
        "failed"
    } else if summary.budget_exhausted {
        "budget_exhausted"
    } else {
        "passed"
    };
    write_runtime_reports(&artifacts, &options, &changes, &plan, &summary, state)?;
    println!(
        "dev-check: {} passed, {} failed, {} skipped ({})",
        summary.runs.iter().filter(|run| run.success).count(),
        summary.failures,
        summary.skipped.len(),
        artifacts.display()
    );
    if summary.failures > 0 {
        bail!("dev-check failed: {} command(s) failed", summary.failures);
    }
    if summary.budget_exhausted {
        bail!(
            "dev-check budget exhausted with {} command(s) unrun",
            summary.skipped.len()
        );
    }
    Ok(())
}

#[cfg(test)]
fn plan_for_paths(paths: &[&str], full: bool) -> CheckPlan {
    plan_checks(
        &ChangeSet {
            paths: paths.iter().map(|path| (*path).to_string()).collect(),
            diff_base: "HEAD".to_string(),
            resolved_base: None,
        },
        &Options {
            full,
            ..Options::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn detect_resolves_the_hoisted_repository_root() {
        // The Cargo workspace was hoisted to the repository root (c8f4153d0), so
        // xtask's manifest parent IS the repo root — there is no longer a `rust/`
        // level between them. `main.rs`'s WorkspacePaths::detect already models
        // this (repo_root = workspace_dir.clone()); this one must agree, or every
        // git invocation runs one directory above the repository.
        let paths = WorkspacePaths::detect().unwrap();
        assert_eq!(paths.repo_root, paths.workspace_dir);
        assert!(
            paths.repo_root.join("Cargo.toml").is_file(),
            "repo root {} has no Cargo.toml",
            paths.repo_root.display()
        );
        assert!(
            paths.repo_root.join("crates").is_dir(),
            "repo root {} has no crates/",
            paths.repo_root.display()
        );
    }

    #[test]
    fn cli_parses_all_required_options() {
        let options = Options::parse(&strings(&[
            "--base",
            "origin/main",
            "--changed",
            "crates/clonk-engine/src/compat.rs",
            "--changed=content/Foo.c4s/Script.c",
            "--plan",
            "--full",
            "--keep-going",
            "--budget-seconds",
            "45",
            "--artifacts=tmp/check",
        ]))
        .unwrap();
        assert_eq!(options.base.as_deref(), Some("origin/main"));
        assert_eq!(options.changed.len(), 2);
        assert!(options.plan_only && options.full && options.keep_going);
        assert_eq!(options.budget_seconds, Some(45));
        assert_eq!(options.artifacts, Some(PathBuf::from("tmp/check")));
    }

    #[test]
    fn cli_rejects_bad_or_duplicate_values() {
        assert!(Options::parse(&strings(&["--wat"])).is_err());
        assert!(Options::parse(&strings(&["--base", "a", "--base", "b"])).is_err());
        assert!(Options::parse(&strings(&["--budget-seconds=0"])).is_err());
        assert!(Options::parse(&strings(&["--changed="])).is_err());
    }

    #[test]
    fn name_status_parser_keeps_rename_delete_and_space_paths() {
        let parsed =
            parse_name_status_z(b"R100\0old path.rs\0new path.rs\0D\0deleted.rs\0M\0changed.rs\0")
                .unwrap();
        assert_eq!(
            parsed,
            vec!["changed.rs", "deleted.rs", "new path.rs", "old path.rs"]
        );
    }

    #[test]
    fn git_collection_unions_staged_unstaged_untracked_and_rename_ends() {
        let root = test_dir("git-union");
        fs::create_dir_all(&root).unwrap();
        init_test_git_repository(&root);
        fs::write(root.join("unstaged.txt"), "one\n").unwrap();
        fs::write(root.join("rename-me.txt"), "rename\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "base"]);
        fs::write(root.join("unstaged.txt"), "two\n").unwrap();
        fs::write(root.join("staged.txt"), "staged\n").unwrap();
        git(&root, &["add", "staged.txt"]);
        git(&root, &["mv", "rename-me.txt", "renamed.txt"]);
        fs::write(root.join("untracked path.txt"), "new\n").unwrap();

        let roots = WorkspacePaths {
            repo_root: root.clone(),
            workspace_dir: root.clone(),
        };
        let changes = collect_changes(&Options::default(), &roots).unwrap();
        for expected in [
            "unstaged.txt",
            "staged.txt",
            "rename-me.txt",
            "renamed.txt",
            "untracked path.txt",
        ] {
            assert!(
                changes.paths.iter().any(|path| path == expected),
                "missing {expected}: {:?}",
                changes.paths
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn base_collection_includes_committed_and_worktree_changes() {
        let root = test_dir("git-base");
        fs::create_dir_all(&root).unwrap();
        init_test_git_repository(&root);
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "base"]);
        let base = git_text(&root, &["rev-parse", "HEAD"]).unwrap();

        fs::write(root.join("committed.txt"), "committed\n").unwrap();
        git(&root, &["add", "committed.txt"]);
        git(&root, &["commit", "-m", "after base"]);
        fs::write(root.join("tracked.txt"), "dirty\n").unwrap();
        fs::write(root.join("untracked.txt"), "new\n").unwrap();

        let roots = WorkspacePaths {
            repo_root: root.clone(),
            workspace_dir: root.clone(),
        };
        let changes = collect_changes(
            &Options {
                base: Some(base.trim().to_string()),
                ..Options::default()
            },
            &roots,
        )
        .unwrap();
        assert_eq!(changes.diff_base, base.trim());
        assert_eq!(changes.resolved_base.as_deref(), Some(base.trim()));
        for expected in ["committed.txt", "tracked.txt", "untracked.txt"] {
            assert!(
                changes.paths.iter().any(|path| path == expected),
                "missing {expected}: {:?}",
                changes.paths
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn explicit_changes_are_normalized_sorted_and_deduped_without_git() {
        let root = test_dir("explicit");
        fs::create_dir_all(&root).unwrap();
        let roots = WorkspacePaths {
            repo_root: root.clone(),
            workspace_dir: root.clone(),
        };
        let changes = collect_changes(
            &Options {
                changed: vec![
                    PathBuf::from("./z.rs"),
                    PathBuf::from("a path.rs"),
                    PathBuf::from("z.rs"),
                ],
                ..Options::default()
            },
            &roots,
        )
        .unwrap();
        assert_eq!(changes.paths, vec!["a path.rs", "z.rs"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_test_false_crate_never_plans_its_own_bare_package_run() {
        // `clonk-engine`, `clonk-frontend` and `clonk-logging` all set
        // `[lib] test = false`, so `cargo nextest run -p <them>` builds fine and
        // then exits non-zero with "no tests to run". A `src/` change routes to
        // the companion crates, but any other file in those packages fell
        // through to the generic per-package command and reddened CI on a change
        // that broke nothing (an edit to a `clonk-engine` example did exactly
        // that). Their inline tests only ever run from the companion crates.
        for (package, path) in [
            (
                "clonk-engine",
                "crates/clonk-engine/examples/scenario_profile.rs",
            ),
            (
                "clonk-frontend",
                "crates/clonk-frontend/examples/anything.rs",
            ),
            ("clonk-logging", "crates/clonk-logging/examples/anything.rs"),
        ] {
            let plan = plan_for_paths(&[path], false);
            assert!(
                !plan.has_args(&["nextest", "run", "-p", package]),
                "{path} planned a bare `-p {package}` run, which always fails"
            );
        }

        // The engine case still has to be covered, not merely skipped.
        let plan = plan_for_paths(&["crates/clonk-engine/examples/scenario_profile.rs"], false);
        assert!(plan.has_args(&[
            "nextest",
            "run",
            "-p",
            "clonk-engine-unit-tests",
            "--test",
            "engine_inline",
        ]));
    }

    #[test]
    fn gameplay_plan_starts_with_replay_and_render_then_engine_checks() {
        let plan = plan_for_paths(&["crates/clonk-engine/src/compat.rs"], false);
        assert_eq!(plan.commands[0].kind, CheckKind::Replay);
        assert_eq!(plan.commands[1].kind, CheckKind::RenderProbe);
        assert_eq!(plan.commands[2].kind, CheckKind::Hygiene);
        assert!(plan.has_args(&[
            "nextest",
            "run",
            "-p",
            "clonk-engine-integration-tests",
            "--test",
            "engine_it",
            "--",
            replay_test_name(cfg!(target_os = "macos")),
            "--exact",
        ]));
        assert!(plan.has_args(&[
            "nextest",
            "run",
            "-p",
            "clonk-engine-unit-tests",
            "--test",
            "engine_inline",
        ]));
        assert!(plan.has_args(&[
            "nextest",
            "run",
            "-p",
            "clonk-engine-unit-tests",
            "--test",
            "unit",
        ]));
        assert!(plan.has_args(&[
            "nextest",
            "run",
            "-p",
            "clonk-engine-integration-tests",
            "--test",
            "engine_it",
            "real_scenario_harness::",
        ]));
    }

    #[test]
    fn recording_host_uses_committed_replay_oracles() {
        assert_eq!(
            replay_test_name(true),
            "dev_feedback_replay::committed_real_scenario_replays_are_deterministic"
        );
        assert_eq!(
            replay_test_name(false),
            "dev_feedback_replay::real_scenario_replays_repeat_with_native_group_order"
        );
    }

    #[test]
    fn frontend_and_app_sources_start_with_replay_and_render() {
        for path in [
            "crates/clonk-frontend/src/renderer.rs",
            "crates/clonk-app/src/input.rs",
        ] {
            let plan = plan_for_paths(&[path], false);
            assert_eq!(plan.commands[0].kind, CheckKind::Replay, "{path}");
            assert_eq!(plan.commands[1].kind, CheckKind::RenderProbe, "{path}");
            assert_eq!(plan.commands[2].kind, CheckKind::Hygiene, "{path}");
            if path.starts_with("crates/clonk-frontend/") {
                assert!(plan.has_args(&[
                    "nextest",
                    "run",
                    "-p",
                    "clonk-frontend-unit-tests",
                    "--test",
                    "frontend_inline",
                ]));
            }
        }
    }

    #[test]
    fn content_submodule_root_runs_replay_render_and_bounded_smoke() {
        let plan = plan_for_paths(&["content"], false);
        assert_eq!(plan.commands[0].kind, CheckKind::Replay);
        assert_eq!(plan.commands[1].kind, CheckKind::RenderProbe);
        assert!(plan.has_args(&[
            "nextest",
            "run",
            "-p",
            "clonk-engine-integration-tests",
            "--test",
            "engine_it",
            "real_scenario_harness::",
        ]));
        assert!(!plan.has_args(&["xtask", "scenario-sweep"]));
    }

    #[test]
    fn landscape_maps_to_movement_and_balloon_families() {
        let plan = plan_for_paths(&["crates/clonk-engine/src/landscape.rs"], false);
        for filter in [
            "walk_movement::",
            "flight_movement::",
            "hangle_movement::",
            "real_tutorial02_balloon_platform::",
        ] {
            assert!(plan.has_args(&[
                "nextest",
                "run",
                "-p",
                "clonk-engine-integration-tests",
                "--test",
                "engine_it",
                filter,
            ]));
        }
    }

    #[test]
    fn direct_module_maps_to_only_its_coalesced_test_target() {
        let plan = plan_for_paths(
            &["crates/clonk-engine/tests/it/real_alchemy_revision.rs"],
            false,
        );
        assert!(plan.has_args(&[
            "nextest",
            "run",
            "-p",
            "clonk-engine-integration-tests",
            "--test",
            "engine_it",
            "real_alchemy_revision::",
        ]));
        assert_eq!(plan.commands.len(), 2, "hygiene plus the focused module");
    }

    #[test]
    fn tutorial_content_maps_to_filtered_sweep_and_headless_tests() {
        let plan = plan_for_paths(&["content/Tutorial.c4f/Tutorial02.c4s/Script.c"], false);
        assert!(plan.has_args(&["xtask", "scenario-sweep", "Tutorial.c4f/Tutorial02.c4s",]));
        assert!(plan.has_args(&[
            "nextest",
            "run",
            "-p",
            "clonk-engine-integration-tests",
            "--test",
            "engine_it",
            "tutorial02_",
        ]));
    }

    #[test]
    fn snapshots_parity_and_workspace_files_escalate() {
        let plan = plan_for_paths(
            &[
                "Cargo.lock",
                "testdata/engine/v1/basic.json",
                "parity/golden/parity_golden.json",
            ],
            false,
        );
        assert!(plan.has_args(&["nextest", "run", "--workspace"]));
        assert!(plan.has_args(&["xtask", "engine-snapshots", "verify"]));
        assert!(plan.has_args(&["xtask", "parity", "verify"]));
    }

    #[test]
    fn plan_dedupes_commands_and_accumulates_stable_reasons() {
        let plan = plan_for_paths(
            &[
                "crates/clonk-engine/src/compat.rs",
                "crates/clonk-engine/src/effect.rs",
            ],
            false,
        );
        let unit: Vec<_> = plan
            .commands
            .iter()
            .filter(|check| check.id == "clonk-engine-unit")
            .collect();
        assert_eq!(unit.len(), 1);
        assert_eq!(unit[0].reasons.len(), 2);
    }

    #[test]
    fn full_plan_uses_broad_gates_and_content_sweep() {
        let plan = plan_for_paths(&["content/Objects.c4d/Foo.c4d/Script.c"], true);
        assert!(plan.has_args(&["nextest", "run", "--workspace"]));
        assert!(plan.has_args(&["xtask", "scenario-sweep"]));
        assert_eq!(
            plan.commands.len(),
            5,
            "replay, render, hygiene, workspace tests, and content sweep"
        );
    }

    #[test]
    fn budget_waits_for_render_probe_then_skips_ordinary_checks() {
        let changes = fake_changes();
        let plan = plan_for_paths(&["crates/clonk-engine/src/compat.rs"], false);
        let root = test_dir("budget");
        fs::create_dir_all(&root).unwrap();
        let roots = fake_roots(&root);
        let mut calls = Vec::new();
        let result = execute_plan_with_runner(
            &plan,
            &Options {
                keep_going: true,
                budget_seconds: Some(1),
                ..Options::default()
            },
            &roots,
            &root,
            &changes,
            |check, _, _| {
                calls.push(check.id.clone());
                RawRun {
                    success: true,
                    exit_code: Some(0),
                    duration: Duration::from_millis(600),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                }
            },
        )
        .unwrap();
        assert_eq!(calls, vec!["deterministic-replay", "frontend-render-probe"]);
        assert!(result.budget_exhausted);
        assert!(!result.skipped.is_empty());
        assert!(root
            .join("commands/01-deterministic-replay.stdout.log")
            .exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replay_failure_runs_render_diagnostic_when_snapshot_exists() {
        let changes = fake_changes();
        let plan = plan_for_paths(&["crates/clonk-engine/src/compat.rs"], false);
        let root = test_dir("replay-failure");
        fs::create_dir_all(&root).unwrap();
        let roots = fake_roots(&root);
        let mut calls = Vec::new();
        let result = execute_plan_with_runner(
            &plan,
            &Options::default(),
            &roots,
            &root,
            &changes,
            |check, _, artifacts| {
                calls.push(check.id.clone());
                if check.kind == CheckKind::Replay {
                    fs::write(artifacts.join("snapshot-final.json"), "{}").unwrap();
                }
                RawRun {
                    success: check.kind != CheckKind::Replay,
                    exit_code: Some(if check.kind == CheckKind::Replay {
                        1
                    } else {
                        0
                    }),
                    duration: Duration::from_millis(1),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                }
            },
        )
        .unwrap();
        assert_eq!(calls, vec!["deterministic-replay", "frontend-render-probe"]);
        assert_eq!(result.failures, 1);
        assert!(!result.skipped.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ordinary_failure_is_fail_fast_without_keep_going() {
        let mut plan = CheckPlan::default();
        for id in ["one", "two", "three"] {
            plan.add(id, CheckKind::Unit, CheckCwd::Repo, "fake", &[id], id);
        }
        let root = test_dir("fail-fast");
        fs::create_dir_all(&root).unwrap();
        let result = execute_plan_with_runner(
            &plan,
            &Options::default(),
            &fake_roots(&root),
            &root,
            &fake_changes(),
            |check, _, _| RawRun {
                success: check.id != "one",
                exit_code: Some(1),
                duration: Duration::from_millis(1),
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(result.runs.len(), 1);
        assert_eq!(result.skipped, vec!["two", "three"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stable_artifact_reports_include_command_and_phase_metrics() {
        let root = test_dir("reports");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("replay-metrics.json"),
            r#"{"load_ns": 11, "simulation_ns": 22}"#,
        )
        .unwrap();
        fs::write(
            root.join("render-metrics.json"),
            r#"{"cold_render_ns": 33}"#,
        )
        .unwrap();
        let plan = plan_for_paths(&["README.md"], false);
        write_runtime_reports(
            &root,
            &Options::default(),
            &fake_changes(),
            &plan,
            &ExecutionSummary::default(),
            "planned",
        )
        .unwrap();
        for name in ["manifest.json", "timings.json", "summary.json"] {
            assert!(root.join(name).is_file(), "missing {name}");
        }
        let timings = fs::read_to_string(root.join("timings.json")).unwrap();
        assert!(timings.contains("\"load_ns\": 11"));
        assert!(timings.contains("\"simulation_ns\": 22"));
        assert!(timings.contains("\"cold_render_ns\": 33"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cargo_build_time_is_separated_from_test_execution() {
        assert_eq!(
            cargo_reported_build_ms(
                b"   Compiling clonk-engine v0.1.0\n    Finished `test` profile [optimized] target(s) in 2.34s\n"
            ),
            Some(2_340)
        );
        assert_eq!(
            cargo_reported_build_ms(
                b"    Finished `test` profile [optimized] target(s) in 1m 03s\n"
            ),
            Some(63_000)
        );
        assert_eq!(cargo_reported_build_ms(b"ordinary test output\n"), None);
    }

    #[test]
    fn summary_report_names_total_time_and_slowest_bottleneck() {
        let root = test_dir("summary-bottleneck");
        fs::create_dir_all(&root).unwrap();
        let summary = ExecutionSummary {
            runs: vec![
                RunRecord {
                    id: "compile-heavy".to_string(),
                    success: true,
                    exit_code: Some(0),
                    duration_ms: 3_000,
                    cargo_build_ms: Some(2_500),
                    stdout_log: "commands/one.stdout.log".to_string(),
                    stderr_log: "commands/one.stderr.log".to_string(),
                },
                RunRecord {
                    id: "render".to_string(),
                    success: true,
                    exit_code: Some(0),
                    duration_ms: 700,
                    cargo_build_ms: Some(20),
                    stdout_log: "commands/two.stdout.log".to_string(),
                    stderr_log: "commands/two.stderr.log".to_string(),
                },
            ],
            ..ExecutionSummary::default()
        };
        let plan = plan_for_paths(&["README.md"], false);
        write_runtime_reports(
            &root,
            &Options::default(),
            &fake_changes(),
            &plan,
            &summary,
            "passed",
        )
        .unwrap();

        let timings = fs::read_to_string(root.join("timings.json")).unwrap();
        assert!(timings.contains("\"cargo_build_ms\": 2500"));
        assert!(timings.contains("\"execution_after_build_ms\": 500"));
        let report = fs::read_to_string(root.join("summary.json")).unwrap();
        assert!(report.contains("\"total_duration_ms\": 3700"));
        assert!(report.contains("\"slowest_command\": \"compile-heavy\""));
        assert!(report.contains("\"slowest_command_ms\": 3000"));
        assert!(report.contains("\"cargo_build_total_ms\": 2520"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replay_bundle_outputs_are_promoted_for_the_render_probe() {
        let root = test_dir("promote-replay");
        let bundle = root.join("tutorial01-idle-0");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("final.json"), r#"{"frame": 3}"#).unwrap();
        fs::write(
            bundle.join("replay-metrics.json"),
            r#"{"simulation_ns": 22}"#,
        )
        .unwrap();

        promote_replay_artifacts(&root).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("snapshot-final.json")).unwrap(),
            r#"{"frame": 3}"#
        );
        assert_eq!(
            fs::read_to_string(root.join("replay-metrics.json")).unwrap(),
            r#"{"simulation_ns": 22}"#
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn json_escaping_handles_report_control_characters() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn fake_changes() -> ChangeSet {
        ChangeSet {
            paths: vec!["crates/clonk-engine/src/compat.rs".to_string()],
            diff_base: "HEAD".to_string(),
            resolved_base: None,
        }
    }

    fn fake_roots(root: &Path) -> WorkspacePaths {
        WorkspacePaths {
            repo_root: root.to_path_buf(),
            workspace_dir: root.to_path_buf(),
        }
    }

    fn test_dir(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let next = NEXT.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "lc-dev-check-{label}-{}-{next}",
            std::process::id()
        ))
    }

    fn init_test_git_repository(root: &Path) {
        git(root, &["init"]);
        git(root, &["config", "user.email", "test@example.invalid"]);
        git(root, &["config", "user.name", "Dev Check"]);
        // Temporary fixture commits must not inherit a developer's global
        // signing policy: nextest has no interactive pinentry by design.
        git(root, &["config", "commit.gpgsign", "false"]);
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
