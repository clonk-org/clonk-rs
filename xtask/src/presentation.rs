//! `cargo xtask presentation` — lightweight presentation-capture orchestration.
//!
//! The verifier owns the release-profile capture build and all presentation
//! evidence checks. This module only validates the command shape, resolves the
//! repository root, and starts the existing Python verifier, so invoking the
//! presentation gate does not first compile the engine-backed xtask binary.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::parity::workspace_dir;

fn accepted_verification_command(repo_root: &Path) -> Vec<OsString> {
    vec![
        repo_root
            .join("scripts/acquire_presentation_oracle.py")
            .into_os_string(),
        "verify-accepted-index".into(),
        "--repo-root".into(),
        repo_root.as_os_str().to_owned(),
    ]
}

fn current_verification_command(repo_root: &Path, arguments: &[String]) -> Vec<OsString> {
    let mut command = vec![
        repo_root
            .join("scripts/acquire_presentation_oracle.py")
            .into_os_string(),
        "verify-current".into(),
        "--repo-root".into(),
        repo_root.as_os_str().to_owned(),
    ];
    command.extend(arguments.iter().map(OsString::from));
    command
}

fn run_verification(repo_root: &Path, command: &[OsString]) -> Result<()> {
    let (script, arguments) = command
        .split_first()
        .context("presentation-verification command has no script")?;
    let status = Command::new("python3")
        .arg(script)
        .args(arguments)
        .current_dir(repo_root)
        .status()
        .with_context(|| {
            format!(
                "running presentation evidence verifier {}",
                script.to_string_lossy()
            )
        })?;
    if !status.success() {
        bail!("presentation evidence verification failed with {status}");
    }
    Ok(())
}

/// Verify the checked-in presentation evidence without running a capture.
///
/// `compat verify` uses this fail-closed structural/provenance check after it
/// validates the compatibility manifest. Keeping the implementation here
/// makes the accepted-evidence command and the current-capture command share
/// the same lightweight process boundary.
pub fn verify_accepted_evidence(repo_root: &Path) -> Result<()> {
    run_verification(repo_root, &accepted_verification_command(repo_root))
}

/// Run the current Rust presentation capture and compare it with accepted C++
/// evidence.
pub fn command(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        println!(
            "Usage:\n  cargo xtask presentation verify-current --profile <p> --output-dir <dir>   Capture current Rust twice and compare every case to accepted C++ evidence."
        );
        return Ok(());
    }
    if args[0] != "verify-current" {
        bail!(
            "unknown `presentation` subcommand `{}` (try `cargo xtask presentation --help`)",
            args[0]
        );
    }

    let repo_root = workspace_dir()?;
    run_verification(
        &repo_root,
        &current_verification_command(&repo_root, &args[1..]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn accepted_evidence_uses_the_index_only_verifier() {
        let repository = Path::new("/checkout");

        assert_eq!(
            accepted_verification_command(repository),
            vec![
                repository
                    .join("scripts/acquire_presentation_oracle.py")
                    .into_os_string(),
                "verify-accepted-index".into(),
                "--repo-root".into(),
                repository.as_os_str().to_owned(),
            ]
        );
    }

    #[test]
    fn current_capture_forwards_profile_and_output_directory() {
        let repository = Path::new("/checkout");
        let arguments = [
            "--profile".to_owned(),
            "test".to_owned(),
            "--output-dir".to_owned(),
            "/tmp/current-presentation".to_owned(),
        ];

        assert_eq!(
            current_verification_command(repository, &arguments),
            vec![
                repository
                    .join("scripts/acquire_presentation_oracle.py")
                    .into_os_string(),
                "verify-current".into(),
                "--repo-root".into(),
                repository.as_os_str().to_owned(),
                "--profile".into(),
                "test".into(),
                "--output-dir".into(),
                "/tmp/current-presentation".into(),
            ]
        );
    }

    #[test]
    fn verifier_failure_is_returned_to_the_caller() {
        let repository = tempfile::tempdir().expect("creating a verifier test directory");
        let script = repository.path().join("failing-verifier.py");
        fs::write(&script, "import sys\nsys.exit(7)\n").expect("writing the failing verifier");
        let command = [script.into_os_string()];

        let error = run_verification(repository.path(), &command)
            .expect_err("a failing verifier must be returned as an error");

        assert!(error
            .to_string()
            .starts_with("presentation evidence verification failed with "));
    }
}
