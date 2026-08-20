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
                .args([
                    "nextest",
                    "run",
                    "-p",
                    "clonk-engine-unit-tests",
                    "--test",
                    "engine_inline",
                    "-E",
                    "test(parity_differential_matches_cpp_golden)",
                ])
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

    #[test]
    fn workspace_is_the_parent_of_the_xtask_manifest() {
        let root = workspace_dir().expect("workspace root");

        assert!(root.join("Cargo.toml").is_file());
        assert!(root.join("parity").is_dir());
    }
}
