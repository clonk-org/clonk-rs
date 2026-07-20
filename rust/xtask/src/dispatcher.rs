use std::env;
use std::ffi::OsString;
use std::process::Command;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "dev-check") {
        return xtask::dev_check::command(&args[1..]);
    }

    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let status = Command::new(cargo)
        .args([
            "run",
            "--profile",
            "test",
            "--package",
            "xtask",
            "--features",
            "engine-tools",
            "--bin",
            "xtask-engine-tools",
            "--",
        ])
        .args(&args)
        .status()
        .context("running the engine-backed xtask command")?;
    if !status.success() {
        bail!("engine-backed xtask command failed with {status}");
    }
    Ok(())
}
