use anyhow::{anyhow, bail, Context, Result};
use lc_engine::fixtures::SNAPSHOT_SCENARIOS;
use lc_engine::{Playback, Recording};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipWriter};

fn main() -> Result<()> {
    lc_core::logging::init();

    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        Some("package") => {
            if let Some(arg) = args.next() {
                bail!("unexpected argument `{}` for `package` command", arg);
            }
            package()
        }
        Some("engine-snapshots") => {
            let tail: Vec<String> = args.collect();
            engine_snapshots_command(&tail)
        }
        Some("ffi") => {
            let tail: Vec<String> = args.collect();
            ffi_command(&tail)
        }
        Some("parity") => {
            let tail: Vec<String> = args.collect();
            parity_command(&tail)
        }
        Some(cmd) => bail!("unknown command `{}` (try `cargo xtask --help`)", cmd),
    }
}

fn print_usage() {
    tracing::info!(
        "Usage:\n  cargo xtask package                 Build the Rust port and bundle a distributable archive.\n  cargo xtask engine-snapshots record Regenerate engine snapshot baselines.\n  cargo xtask engine-snapshots verify Check Rust engine output against recorded baselines.\n  cargo xtask ffi [options]           Build staticlib/cdylib artifacts for C++ integration.\n  cargo xtask parity record|verify    C++↔Rust differential parity harness (see parity/README.md)."
    );
}

fn engine_snapshots_command(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        print_engine_snapshots_usage();
        return Ok(());
    }

    match args[0].as_str() {
        "record" => {
            if args.len() > 1 {
                bail!("`engine-snapshots record` does not take additional arguments");
            }
            record_engine_snapshots()
        }
        "verify" => {
            if args.len() > 1 {
                bail!("`engine-snapshots verify` does not take additional arguments");
            }
            verify_engine_snapshots()
        }
        other => bail!(
            "unknown `engine-snapshots` subcommand `{}` (try `cargo xtask engine-snapshots --help`)",
            other
        ),
    }
}

fn print_engine_snapshots_usage() {
    tracing::info!(
        "Usage:\n  cargo xtask engine-snapshots record\n  cargo xtask engine-snapshots verify"
    );
}

/// `cargo xtask parity record|verify` — the C++↔Rust differential parity harness
/// (see `parity/README.md`). `record` regenerates the C++ golden oracle from the
/// real engine primitives; `verify` runs the Rust differential check.
fn parity_command(args: &[String]) -> Result<()> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h") {
        tracing::info!(
            "Usage:\n  cargo xtask parity record   Regenerate the C++ golden oracle (parity/golden).\n  cargo xtask parity verify   Run the Rust differential check against the golden."
        );
        return Ok(());
    }
    let paths = WorkspacePaths::detect()?;
    match args[0].as_str() {
        "record" => {
            if args.len() > 1 {
                bail!("`parity record` does not take additional arguments");
            }
            let script = paths.repo_root.join("parity/oracle/gen_golden.sh");
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
            let status = Command::new("cargo")
                .current_dir(&paths.workspace_dir)
                .args([
                    "test",
                    "-p",
                    "lc-engine",
                    "--lib",
                    "parity_differential_matches_cpp_golden",
                ])
                .status()
                .context("failed to run cargo test for parity verify")?;
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

fn ffi_command(args: &[String]) -> Result<()> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        print_ffi_usage();
        return Ok(());
    }

    let mut profile = BuildProfile::Debug;
    let mut requested = Vec::new();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--release" => {
                profile = BuildProfile::Release;
                idx += 1;
            }
            "--debug" => {
                profile = BuildProfile::Debug;
                idx += 1;
            }
            "--profile" => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| anyhow!("`--profile` expects a value"))?;
                profile = BuildProfile::from_str(value)
                    .ok_or_else(|| anyhow!("unknown profile `{}`", value))?;
                idx += 1;
            }
            "--package" | "-p" => {
                idx += 1;
                let value = args
                    .get(idx)
                    .ok_or_else(|| anyhow!("`{}` expects a crate name", args[idx - 1]))?;
                requested.push(value.clone());
                idx += 1;
            }
            other if other.starts_with('-') => {
                bail!(
                    "unknown argument `{}` (try `cargo xtask ffi --help`)",
                    other
                );
            }
            other => {
                requested.push(other.to_string());
                idx += 1;
            }
        }
    }

    let selected: Vec<&'static FfiCrate> = if requested.is_empty() {
        FFI_CRATES.iter().collect()
    } else {
        let mut seen = BTreeSet::new();
        let mut crates = Vec::new();
        for name in requested {
            let normalized = normalize_crate_name(&name);
            if !seen.insert(normalized.clone()) {
                continue;
            }
            let info =
                crate_by_name(&normalized).ok_or_else(|| anyhow!("unknown crate `{}`", name))?;
            crates.push(info);
        }
        crates
    };

    if selected.is_empty() {
        bail!("no crates selected for FFI build");
    }

    let paths = WorkspacePaths::detect()?;
    for krate in selected {
        build_ffi_crate(krate, profile, &paths)?;
    }
    Ok(())
}

fn print_ffi_usage() {
    tracing::info!(
        "Usage:\n  cargo xtask ffi [--profile <debug|release>] [--package <crate> ...]\n\n  By default builds all FFI-enabled crates for the debug profile.\n  Pass --release or --profile release to emit optimized artifacts."
    );
}

#[derive(Clone, Copy)]
struct FfiCrate {
    name: &'static str,
    feature: Option<&'static str>,
}

const FFI_CRATES: &[FfiCrate] = &[
    FfiCrate {
        name: "lc-core",
        feature: Some("ffi"),
    },
    FfiCrate {
        name: "lc-resources",
        feature: Some("ffi"),
    },
    FfiCrate {
        name: "lc-engine",
        feature: Some("ffi"),
    },
    FfiCrate {
        name: "lc-gui",
        feature: None,
    },
    FfiCrate {
        name: "lc-platform",
        feature: Some("ffi"),
    },
    FfiCrate {
        name: "lc-graphics",
        feature: Some("ffi"),
    },
    FfiCrate {
        name: "lc-audio",
        feature: Some("ffi"),
    },
    FfiCrate {
        name: "lc-script",
        feature: Some("ffi"),
    },
];

fn crate_by_name(name: &str) -> Option<&'static FfiCrate> {
    FFI_CRATES.iter().find(|candidate| candidate.name == name)
}

fn normalize_crate_name(raw: &str) -> String {
    raw.replace('_', "-")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    fn from_str(raw: &str) -> Option<Self> {
        let lower = raw.to_ascii_lowercase();
        match lower.as_str() {
            "debug" | "dev" => Some(Self::Debug),
            "release" | "relwithdebinfo" | "minsizerel" => Some(Self::Release),
            _ => None,
        }
    }

    fn apply(self, command: &mut Command) {
        if matches!(self, Self::Release) {
            command.arg("--release");
        }
    }

    fn dir_name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    fn display(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

#[derive(Clone, Copy)]
enum ArtifactKind {
    Static,
    Dynamic,
}

fn build_ffi_crate(krate: &FfiCrate, profile: BuildProfile, paths: &WorkspacePaths) -> Result<()> {
    tracing::info!(
        crate = krate.name,
        profile = profile.display(),
        "building FFI artifacts"
    );

    let mut cmd = Command::new("cargo");
    cmd.arg("build");
    profile.apply(&mut cmd);
    cmd.arg("-p").arg(krate.name);
    if let Some(feature) = krate.feature {
        cmd.arg("--features").arg(feature);
    }
    let status = cmd
        .current_dir(&paths.workspace_dir)
        .status()
        .with_context(|| format!("failed to invoke cargo for crate `{}`", krate.name))?;
    if !status.success() {
        bail!("cargo build failed for crate `{}`", krate.name);
    }

    let profile_dir = paths.profile_dir(profile);
    let static_lib = find_artifact(&profile_dir, krate.name, ArtifactKind::Static)
        .with_context(|| format!("crate `{}` did not emit a static library", krate.name))?;
    let dynamic_lib = find_artifact(&profile_dir, krate.name, ArtifactKind::Dynamic)
        .with_context(|| format!("crate `{}` did not emit a dynamic library", krate.name))?;

    tracing::info!(
        crate = krate.name,
        static = %display_relative(&static_lib, &paths.repo_root),
        dynamic = %display_relative(&dynamic_lib, &paths.repo_root),
        "finished building FFI artifacts"
    );
    Ok(())
}

fn find_artifact(dir: &Path, crate_name: &str, kind: ArtifactKind) -> Option<PathBuf> {
    let base = crate_name.replace('-', "_");
    let mut candidates = Vec::new();
    match kind {
        ArtifactKind::Static => {
            let static_exts: &[&str] = if cfg!(target_os = "windows") {
                &["lib", "a"]
            } else {
                &["a", "lib"]
            };
            for ext in static_exts {
                candidates.push(format!("lib{}.{}", base, ext));
                candidates.push(format!("{}.{}", base, ext));
            }
        }
        ArtifactKind::Dynamic => {
            let dynamic_exts: &[&str] = if cfg!(target_os = "windows") {
                &["dll"]
            } else if cfg!(target_os = "macos") {
                &["dylib"]
            } else {
                &["so"]
            };
            for ext in dynamic_exts {
                candidates.push(format!("lib{}.{}", base, ext));
                candidates.push(format!("{}.{}", base, ext));
            }
            if cfg!(target_os = "windows") {
                candidates.push(format!("lib{}.dll.a", base));
                candidates.push(format!("{}.dll.lib", base));
            }
        }
    }

    for candidate in candidates {
        let path = dir.join(&candidate);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn display_relative(path: &Path, base: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn record_engine_snapshots() -> Result<()> {
    let paths = WorkspacePaths::detect()?;
    let snapshot_dir = engine_snapshot_dir(&paths);
    fs::create_dir_all(&snapshot_dir)
        .with_context(|| format!("failed to create {}", snapshot_dir.display()))?;

    for scenario in SNAPSHOT_SCENARIOS {
        let recording = (scenario.generator)(scenario.default_frames)
            .with_context(|| format!("failed to record scenario `{}`", scenario.name))?;
        let path = snapshot_dir.join(format!("{}.json", scenario.name));
        let file =
            File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
        recording
            .to_writer(file)
            .map_err(|error| anyhow!(error))
            .with_context(|| format!("failed to serialize recording for `{}`", scenario.name))?;
        let display_path = match path.strip_prefix(&paths.repo_root) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => path.clone(),
        };
        tracing::info!(
            path = %display_path.display(),
            frames = scenario.default_frames,
            "wrote engine snapshot"
        );
    }

    Ok(())
}

fn verify_engine_snapshots() -> Result<()> {
    let paths = WorkspacePaths::detect()?;
    let snapshot_dir = engine_snapshot_dir(&paths);

    for scenario in SNAPSHOT_SCENARIOS {
        let path = snapshot_dir.join(format!("{}.json", scenario.name));
        let baseline = load_recording(&path)
            .with_context(|| format!("failed to load baseline {}", path.display()))?;
        let frames = baseline.frames().len();
        if frames != scenario.default_frames {
            bail!(
                "baseline {} contains {} frames but scenario expects {}",
                path.display(),
                frames,
                scenario.default_frames
            );
        }
        let playback = Playback::from_recording(baseline);
        let actual = (scenario.generator)(scenario.default_frames)
            .with_context(|| format!("failed to run scenario `{}`", scenario.name))?;
        playback
            .validate_sequence(actual.into_frames())
            .map_err(|error| anyhow!(error))
            .with_context(|| format!("snapshot mismatch for `{}`", scenario.name))?;
        tracing::info!(
            scenario = scenario.name,
            frames = scenario.default_frames,
            "validated engine snapshot"
        );
    }

    Ok(())
}

fn engine_snapshot_dir(paths: &WorkspacePaths) -> PathBuf {
    paths
        .workspace_dir
        .join("snapshots")
        .join("engine")
        .join("v1")
}

fn load_recording(path: &Path) -> Result<Recording> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    Recording::from_reader(BufReader::new(file)).map_err(|error| anyhow!(error))
}

fn package() -> Result<()> {
    let paths = WorkspacePaths::detect()?;
    build_lc_game(&paths)?;
    let package_dir = assemble_package_layout(&paths)?;
    let archive = create_archive(&paths, &package_dir)?;
    tracing::info!(path = %archive.display(), "packaged Rust port");
    Ok(())
}

fn build_lc_game(paths: &WorkspacePaths) -> Result<()> {
    tracing::info!("building lc-game (release)");
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "lc-game"])
        .current_dir(&paths.workspace_dir)
        .status()
        .context("failed to invoke cargo build")?;
    if !status.success() {
        bail!("cargo build failed with status {:?}", status.code());
    }
    Ok(())
}

fn assemble_package_layout(paths: &WorkspacePaths) -> Result<PathBuf> {
    let dist_dir = paths.workspace_dir.join("target").join("dist");
    let package_dir = dist_dir.join("legacyclonk-rs");

    if package_dir.exists() {
        fs::remove_dir_all(&package_dir)
            .with_context(|| format!("failed to remove {}", package_dir.display()))?;
    }
    fs::create_dir_all(&package_dir)
        .with_context(|| format!("failed to create {}", package_dir.display()))?;

    let bin_dir = package_dir.join("bin");
    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;

    let exe_name = format!("lc-game{}", env::consts::EXE_SUFFIX);
    let built_binary = paths
        .workspace_dir
        .join("target")
        .join("release")
        .join(&exe_name);
    if !built_binary.exists() {
        bail!("expected lc-game binary at {}", built_binary.display());
    }
    let packaged_binary = bin_dir.join(&exe_name);
    fs::copy(&built_binary, &packaged_binary).with_context(|| {
        format!(
            "failed to copy {} to {}",
            built_binary.display(),
            packaged_binary.display()
        )
    })?;
    set_executable(&packaged_binary)?;

    copy_file(
        &paths.repo_root.join("COPYING"),
        &package_dir.join("COPYING"),
    )?;
    copy_file(
        &paths.repo_root.join("README.md"),
        &package_dir.join("README.md"),
    )?;
    copy_file(
        &paths.repo_root.join("credits.txt"),
        &package_dir.join("credits.txt"),
    )?;

    let planet_src = paths.repo_root.join("planet");
    let planet_dst = package_dir.join("planet");
    copy_directory(&planet_src, &planet_dst)?;

    Ok(package_dir)
}

fn create_archive(paths: &WorkspacePaths, package_dir: &Path) -> Result<PathBuf> {
    let dist_dir = paths.workspace_dir.join("target").join("dist");
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;
    let archive_path = dist_dir.join("legacyclonk-rs.zip");
    if archive_path.exists() {
        fs::remove_file(&archive_path)
            .with_context(|| format!("failed to remove {}", archive_path.display()))?;
    }

    let file = File::create(&archive_path)
        .with_context(|| format!("unable to create archive {}", archive_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let base_name = package_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "package".to_string());
    let dir_options = FileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o755);
    zip.add_directory(format!("{}/", base_name), dir_options)?;

    let file_options = FileOptions::default().compression_method(CompressionMethod::Deflated);

    for entry in WalkDir::new(package_dir) {
        let entry = entry?;
        let rel_path = entry.path().strip_prefix(package_dir).unwrap();
        if rel_path.as_os_str().is_empty() {
            continue;
        }
        let mut zip_path = PathBuf::from(&base_name);
        zip_path.push(rel_path);
        let zip_path_str = path_to_zip_string(&zip_path);

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            let options = dir_options.clone();
            zip.add_directory(format!("{}/", zip_path_str), options)?;
            continue;
        }

        if metadata.is_file() {
            let mut options = file_options.clone();
            if zip_path.components().nth(1).map(|c| c.as_os_str())
                == Some(std::ffi::OsStr::new("bin"))
            {
                options = options.unix_permissions(0o755);
            } else {
                options = options.unix_permissions(0o644);
            }
            zip.start_file(&zip_path_str, options)?;
            let mut src = File::open(entry.path())?;
            io::copy(&mut src, &mut zip)?;
        }
    }

    zip.finish()?;
    Ok(archive_path)
}

fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        bail!("required file {} was not found", src.display());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} to {}", src.display(), dst.display()))?;
    Ok(())
}

fn copy_directory(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        bail!("directory {} does not exist", src.display());
    }
    fs::create_dir_all(dst).with_context(|| format!("failed to create {}", dst.display()))?;

    for entry in WalkDir::new(src) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src).unwrap();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(rel);
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create {}", target.display()))?;
        } else if metadata.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(entry.path(), &target).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    entry.path().display(),
                    target.display()
                )
            })?;
        }
    }

    Ok(())
}

fn path_to_zip_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

struct WorkspacePaths {
    workspace_dir: PathBuf,
    repo_root: PathBuf,
}

impl WorkspacePaths {
    fn detect() -> Result<Self> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_dir = manifest_dir
            .parent()
            .context("xtask manifest is missing parent directory")?
            .to_path_buf();
        let repo_root = workspace_dir
            .parent()
            .context("workspace directory is missing parent directory")?
            .to_path_buf();
        Ok(Self {
            workspace_dir,
            repo_root,
        })
    }

    fn profile_dir(&self, profile: BuildProfile) -> PathBuf {
        self.workspace_dir.join("target").join(profile.dir_name())
    }
}
