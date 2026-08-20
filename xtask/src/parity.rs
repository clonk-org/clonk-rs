//! Lightweight entry point for the C++↔Rust differential harness.
//!
//! The verification itself is an engine test, but selecting that one test does
//! not require linking the engine-backed release-tool binary first.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// `cargo xtask parity record|verify` — the C++↔Rust differential parity
/// harness documented in `parity/README.md`.
pub fn command(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        println!(
            "Usage:\n  cargo xtask parity record   Regenerate the C++ golden oracle (parity/golden).\n  cargo xtask parity verify   Run the Rust differential check against the golden."
        );
        return Ok(());
    }

    let workspace_dir = workspace_dir()?;
    match args[0].as_str() {
        "record" => {
            if args.len() > 1 {
                bail!("`parity record` does not take additional arguments");
            }
            let script = workspace_dir.join("parity/oracle/gen_golden.sh");
            let status = Command::new("bash")
                .arg(&script)
                .status()
                .with_context(|| format!("failed to run {}", script.display()))?;
            if !status.success() {
                bail!("parity golden generation failed ({status})");
            }
            Ok(())
        }
        "verify" => {
            if args.len() > 1 {
                bail!("`parity verify` does not take additional arguments");
            }
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let status = Command::new(cargo)
                .current_dir(&workspace_dir)
                .args(verify_nextest_args())
                .status()
                .context("failed to run cargo nextest for parity verify")?;
            if !status.success() {
                bail!("parity differential check failed ({status})");
            }
            Ok(())
        }
        other => bail!(
            "unknown `parity` subcommand `{}` (try `cargo xtask parity --help`)",
            other
        ),
    }
}

/// Test crates that may host a golden comparator.
///
/// `parity verify` used to run one hardcoded package *and* one hardcoded test
/// target, which meant every comparator had to live in `clonk-engine` and could
/// only reach crates the engine depends on. Anything in `clonk-app` or
/// `clonk-frontend` is downstream of the engine, so it could not be compared
/// against the golden at all — golden sections were written for
/// clonk-org/clonk-rs#521 and clonk-org/clonk-rs#523 and then reverted for want
/// of a reachable Rust side (clonk-org/clonk-rs#856).
///
/// Adding an app-layer comparator is now a one-line change here. The test
/// target is deliberately no longer pinned: a package is free to host its
/// comparator in whichever target suits it, and the name filter is what selects
/// the test.
const PARITY_PACKAGES: &[&str] = &["clonk-engine-unit-tests"];

/// The comparator's test name, matched across every [`PARITY_PACKAGES`] entry.
const PARITY_TEST_FILTER: &str = "test(parity_differential_matches_cpp_golden)";

fn verify_nextest_args() -> Vec<String> {
    let mut args = vec!["nextest".to_string(), "run".to_string()];
    for package in PARITY_PACKAGES {
        args.push("-p".to_string());
        args.push((*package).to_string());
    }
    args.push("-E".to_string());
    args.push(PARITY_TEST_FILTER.to_string());
    args
}

pub fn workspace_dir() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .context("xtask manifest has no workspace parent")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_verify_arguments_are_rejected_before_spawning_cargo() {
        let error = command(&["verify".into(), "extra".into()])
            .expect_err("extra parity verify argument must fail");

        assert_eq!(
            error.to_string(),
            "`parity verify` does not take additional arguments"
        );
    }

    /// The package list is the whole point of clonk-org/clonk-rs#856: a
    /// comparator outside `clonk-engine` becomes reachable by adding an entry
    /// here, without touching the command construction.
    #[test]
    fn verify_runs_the_comparator_across_every_listed_package() {
        let args = verify_nextest_args();

        assert_eq!(&args[..2], &["nextest".to_string(), "run".to_string()]);
        for package in PARITY_PACKAGES {
            let position = args
                .iter()
                .position(|arg| arg == package)
                .unwrap_or_else(|| panic!("{package} is passed to nextest"));
            assert_eq!(args[position - 1], "-p", "each package gets its own -p");
        }
        assert_eq!(
            args.iter().filter(|arg| *arg == "-p").count(),
            PARITY_PACKAGES.len(),
            "one -p per listed package and no more"
        );
        assert_eq!(args.last().map(String::as_str), Some(PARITY_TEST_FILTER));

        // The test target is deliberately unpinned so a package can host its
        // comparator wherever it likes; only the name selects it.
        assert!(
            !args.iter().any(|arg| arg == "--test"),
            "pinning a test target would re-impose the single-crate limit"
        );
    }

    #[test]
    fn workspace_is_the_parent_of_the_xtask_manifest() {
        let root = workspace_dir().expect("workspace root");

        assert!(root.join("Cargo.toml").is_file());
        assert!(root.join("parity").is_dir());
    }
}
