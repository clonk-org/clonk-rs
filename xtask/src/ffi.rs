//! Build the C-ABI artifacts the pinned oracle's shadow-diff bridge links
//! against (clonk-org/clonk-rs#585).
//!
//! `CMakeLists.txt:138-143` at the oracle pin runs `cargo xtask ffi --profile
//! <profile>` and then imports `target/<profile>/libclonk_engine.a` for
//! `USE_RUST_ENGINE_VALIDATION` (`:97-107`, `:404-406`). That is the only
//! artifact engine validation links; the other `lc_*` libraries belong to the
//! config/group/GUI/platform bridges, which are off by default.
//!
//! The crate types are emitted **here** rather than declared in
//! `clonk-engine`'s manifest, exactly as the pinned tree did it: a
//! `crate-type` entry would make every ordinary build pay the staticlib
//! archive and the cdylib link, and the TDD loop runs far more often than this
//! command does.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

/// The crates carrying an `ffi` module. Only `clonk-engine` has one today; the
/// pinned tree also shipped `clonk-core`, `clonk-resources`, `clonk-gui`,
/// `clonk-platform`, `clonk-graphics`, `clonk-audio` and `clonk-script`
/// surfaces for the bridges this repository has not restored.
const FFI_CRATES: &[FfiCrate] = &[FfiCrate {
    name: "clonk-engine",
    feature: Some("ffi"),
}];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FfiCrate {
    name: &'static str,
    feature: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    /// CMake hands us `CMAKE_BUILD_TYPE` lowercased, so the multi-config
    /// release names have to map onto Cargo's single release profile.
    fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "debug" | "dev" => Some(Self::Debug),
            "release" | "relwithdebinfo" | "minsizerel" => Some(Self::Release),
            _ => None,
        }
    }

    fn dir_name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    profile: BuildProfile,
    crates: Vec<&'static FfiCrate>,
}

fn parse_options(args: &[String]) -> Result<Options> {
    let mut profile = BuildProfile::Debug;
    let mut selected: Vec<&'static FfiCrate> = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--release" => profile = BuildProfile::Release,
            "--profile" => {
                let raw = args
                    .get(index + 1)
                    .context("`--profile` requires a value (debug or release)")?;
                profile =
                    BuildProfile::parse(raw).with_context(|| format!("unknown profile `{raw}`"))?;
                index += 1;
            }
            "--package" | "-p" => {
                let raw = args
                    .get(index + 1)
                    .context("`--package` requires a crate name")?;
                let normalized = raw.replace('_', "-");
                let found = FFI_CRATES
                    .iter()
                    .find(|candidate| candidate.name == normalized)
                    .with_context(|| format!("`{raw}` has no FFI surface"))?;
                selected.push(found);
                index += 1;
            }
            other => bail!("unknown `ffi` argument `{other}`"),
        }
        index += 1;
    }
    if selected.is_empty() {
        selected = FFI_CRATES.iter().collect();
    }
    Ok(Options {
        profile,
        crates: selected,
    })
}

/// `cargo xtask ffi [--profile <debug|release>] [--package <crate>]`.
pub fn command(args: &[String]) -> Result<()> {
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "Usage:\n  cargo xtask ffi [--profile <debug|release>] [--release] [--package <crate>]\n\n  Emits staticlib/cdylib artifacts for the C++ shadow-diff bridge\n  (parity/bridge/README.md). Defaults to every FFI crate, debug profile."
        );
        return Ok(());
    }

    let options = parse_options(args)?;
    let workspace = crate::parity::workspace_dir()?;
    for krate in options.crates {
        build(krate, options.profile, &workspace)?;
    }
    Ok(())
}

fn build(krate: &FfiCrate, profile: BuildProfile, workspace: &Path) -> Result<()> {
    println!(
        "building {} FFI artifacts ({})",
        krate.name,
        profile.dir_name()
    );

    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.arg("rustc");
    if matches!(profile, BuildProfile::Release) {
        command.arg("--release");
    }
    command.arg("-p").arg(krate.name);
    if let Some(feature) = krate.feature {
        command.arg("--features").arg(feature);
    }
    // Emitted here, not in the manifest — see the module comment.
    command.arg("--crate-type").arg("staticlib,cdylib");

    let status = command
        .current_dir(workspace)
        .status()
        .with_context(|| format!("failed to invoke cargo for `{}`", krate.name))?;
    if !status.success() {
        bail!("cargo rustc failed for `{}` ({status})", krate.name);
    }

    let directory = workspace.join("target").join(profile.dir_name());
    let stem = krate.name.replace('-', "_");
    let missing: Vec<String> = artifact_names(&stem)
        .into_iter()
        .filter(|name| !directory.join(name).exists())
        .collect();
    if !missing.is_empty() {
        bail!(
            "`{}` emitted no {} in {}",
            krate.name,
            missing.join(" or "),
            directory.display()
        );
    }
    println!(
        "{} FFI artifacts ready in {}",
        krate.name,
        directory.display()
    );
    Ok(())
}

/// The file names `cargo rustc --crate-type staticlib,cdylib` produces, which
/// are the ones CMake imports by absolute path.
fn artifact_names(stem: &str) -> Vec<String> {
    let dynamic = if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(target_os = "windows") {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
    };
    let static_lib = if cfg!(target_os = "windows") {
        format!("{stem}.lib")
    } else {
        format!("lib{stem}.a")
    };
    vec![static_lib, dynamic]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_selection_is_every_ffi_crate_in_debug() {
        let options = parse_options(&[]).expect("no arguments parse");
        assert_eq!(options.profile, BuildProfile::Debug);
        assert_eq!(options.crates.len(), FFI_CRATES.len());
    }

    /// CMake passes `CMAKE_BUILD_TYPE` lowercased, and its multi-config release
    /// names have no Cargo profile of their own (CMakeLists.txt:49-54,138-143).
    #[test]
    fn cmake_release_build_types_map_onto_the_cargo_release_profile() {
        for raw in ["release", "RelWithDebInfo", "MinSizeRel", "RELEASE"] {
            assert_eq!(
                BuildProfile::parse(raw),
                Some(BuildProfile::Release),
                "{raw} is a release build type"
            );
        }
        for raw in ["debug", "Dev"] {
            assert_eq!(BuildProfile::parse(raw), Some(BuildProfile::Debug));
        }
        assert_eq!(BuildProfile::parse("coverage"), None);
    }

    #[test]
    fn a_crate_without_an_ffi_surface_is_rejected_before_spawning_cargo() {
        let error = parse_options(&["--package".into(), "clonk-network".into()])
            .expect_err("clonk-network has no FFI surface");
        assert_eq!(error.to_string(), "`clonk-network` has no FFI surface");
    }

    /// Underscores are what a C++ or CMake caller reaches for, since that is
    /// how the artifact is spelled.
    #[test]
    fn a_package_name_may_be_spelled_with_underscores() {
        let options = parse_options(&["-p".into(), "clonk_engine".into()])
            .expect("underscored name resolves");
        assert_eq!(options.crates, vec![&FFI_CRATES[0]]);
    }

    #[test]
    fn unknown_arguments_and_missing_values_are_rejected() {
        assert!(parse_options(&["--wat".into()]).is_err());
        assert!(parse_options(&["--profile".into()]).is_err());
        assert!(parse_options(&["--package".into()]).is_err());
        assert!(parse_options(&["--profile".into(), "coverage".into()]).is_err());
    }

    /// The names CMake imports by absolute path (CMakeLists.txt:97-107).
    #[test]
    fn artifact_names_match_what_cmake_imports() {
        let names = artifact_names("clonk_engine");
        if cfg!(target_os = "macos") {
            assert_eq!(names, vec!["libclonk_engine.a", "libclonk_engine.dylib"]);
        } else if cfg!(target_os = "windows") {
            assert_eq!(names, vec!["clonk_engine.lib", "clonk_engine.dll"]);
        } else {
            assert_eq!(names, vec!["libclonk_engine.a", "libclonk_engine.so"]);
        }
    }
}
