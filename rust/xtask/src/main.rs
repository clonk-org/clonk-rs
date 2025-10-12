use anyhow::{anyhow, bail, Context, Result};
use lc_engine::fixtures::SNAPSHOT_SCENARIOS;
use lc_engine::{Playback, Recording};
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
        Some(cmd) => bail!("unknown command `{}` (try `cargo xtask --help`)", cmd),
    }
}

fn print_usage() {
    eprintln!(
        "Usage:\n  cargo xtask package                 Build the Rust port and bundle a distributable archive.\n  cargo xtask engine-snapshots record Regenerate engine snapshot baselines.\n  cargo xtask engine-snapshots verify Check Rust engine output against recorded baselines."
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
    eprintln!(
        "Usage:\n  cargo xtask engine-snapshots record\n  cargo xtask engine-snapshots verify"
    );
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
        println!(
            "wrote {} ({} frames)",
            display_path.display(),
            scenario.default_frames
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
        println!(
            "validated {} ({} frames)",
            scenario.name, scenario.default_frames
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
    println!("Packaged Rust port to {}", archive.display());
    Ok(())
}

fn build_lc_game(paths: &WorkspacePaths) -> Result<()> {
    println!("Building lc-game (release)...");
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
}
