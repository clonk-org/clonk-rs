mod bundle;
mod log;
mod paths;
mod shell;
mod summary;
mod telemetry;
mod time;

pub use bundle::{create_support_bundle, regenerate_support_bundle};
pub use log::LauncherLog;
pub use shell::{
    copy_support_bundle, ensure_support_bundle, load_shell_state, reveal_in_file_manager,
    support_artifacts, LauncherShellEnsureResult, LauncherShellState, SupportArtifact,
};
pub use summary::{
    load_launcher_summary, write_launcher_summary, LauncherSummary, LauncherSummaryRecord,
};
pub use telemetry::{
    digest_update_telemetry, SerializableTelemetryFailure, SerializableTelemetrySummary,
    UpdateTelemetryFailure, UpdateTelemetrySummary,
};
pub use time::{timestamp_for_filename, timestamp_for_log};

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use lc_platform::AppPaths;
    use serde_json::Value;
    use std::env;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    struct EnvGuard {
        saved: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, Option<&Path>)]) -> Self {
            let mut saved = Vec::with_capacity(vars.len());
            for (key, value) in vars {
                saved.push((key.to_string(), env::var_os(key)));
                match value {
                    Some(path) => env::set_var(key, path),
                    None => env::remove_var(key),
                }
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(v) => env::set_var(key, v),
                    None => env::remove_var(key),
                }
            }
        }
    }

    struct TestLogger {
        path: PathBuf,
    }

    impl TestLogger {
        fn new(path: PathBuf) -> Self {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::File::create(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl LauncherLog for TestLogger {
        fn log_line(&self, message: &str) -> Result<()> {
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            writeln!(file, "{message}")?;
            Ok(())
        }
    }

    fn prepare_install_root(dir: &Path) {
        let planet_dir = dir.join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();
    }

    #[test]
    fn load_shell_state_none_without_summary() {
        let install_dir = TempDir::new().unwrap();
        prepare_install_root(install_dir.path());
        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();

        assert!(
            load_shell_state(&paths).unwrap().is_none(),
            "state should be absent when no summary exists"
        );
    }

    #[test]
    fn load_shell_state_aggregates_paths() {
        let install_dir = TempDir::new().unwrap();
        prepare_install_root(install_dir.path());
        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();

        let logger = TestLogger::new(paths.logs_dir().join("lc-launcher.log"));
        logger.log_line("launcher ready").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-session.log");
        fs::write(&runtime_log, "runtime").unwrap();
        let crash_log = paths.logs_dir().join("LegacyClonk-crash.dmp");
        fs::write(&crash_log, "crash").unwrap();
        let bundle_path = paths.logs_dir().join("support-bundle.zip");
        fs::write(&bundle_path, b"bundle").unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());
        telemetry.record_failure(runtime_log.clone(), "c4group returned status 1".into());

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log.clone()],
            &[crash_log.clone()],
            &telemetry,
            Some(&bundle_path),
        )
        .unwrap();

        let state = load_shell_state(&paths)
            .unwrap()
            .expect("launcher state should be available");

        assert_eq!(
            state.summary_path,
            paths.logs_dir().join("launcher-summary.json")
        );
        assert_eq!(state.launcher_log_path, logger.path());
        assert_eq!(state.runtime_log_paths, vec![runtime_log]);
        assert_eq!(state.crash_report_paths, vec![crash_log]);
        assert_eq!(state.support_bundle_path, Some(bundle_path));
    }

    #[test]
    fn ensure_support_bundle_regenerates_bundle() {
        let install_dir = TempDir::new().unwrap();
        prepare_install_root(install_dir.path());
        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();

        let logger = TestLogger::new(paths.logs_dir().join("lc-launcher.log"));
        logger.log_line("regeneration start").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-regenerate.log");
        fs::write(&runtime_log, "Done.\n").unwrap();
        let crash_log = paths.logs_dir().join("LegacyClonk-crash-regenerate.dmp");
        fs::write(&crash_log, "crash dump").unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log],
            &[crash_log],
            &telemetry,
            None,
        )
        .unwrap();

        let result = ensure_support_bundle(&paths, &logger, logger.path()).unwrap();
        assert!(
            result.bundle_path.exists(),
            "expected regenerated bundle to exist"
        );
        assert_eq!(
            result.telemetry.successes().len(),
            1,
            "telemetry should preserve success entries"
        );

        let summary_contents =
            fs::read_to_string(paths.logs_dir().join("launcher-summary.json")).unwrap();
        let document: Value = serde_json::from_str(&summary_contents).unwrap();
        let recorded = document["support_bundle"]
            .as_str()
            .expect("summary support bundle entry");
        let relative = result
            .bundle_path
            .strip_prefix(paths.logs_dir())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| result.bundle_path.display().to_string());
        assert_eq!(
            recorded, relative,
            "summary should point at regenerated bundle"
        );
    }

    #[test]
    fn copy_support_bundle_creates_unique_name() {
        let temp_dir = TempDir::new().unwrap();
        let source = temp_dir.path().join("support-bundle.zip");
        fs::write(&source, b"bundle").unwrap();
        let dest_dir = TempDir::new().unwrap();

        let first = copy_support_bundle(&source, dest_dir.path()).unwrap();
        assert!(first.exists(), "copied bundle should exist");

        // Second copy should receive a suffix because the target name already exists.
        let second = copy_support_bundle(&source, dest_dir.path()).unwrap();
        assert!(
            second
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.contains("-copy"))
                .unwrap_or(false),
            "second copy should receive -copy suffix"
        );
    }

    #[test]
    fn support_artifacts_list_bundle_and_summary() {
        let install_dir = TempDir::new().unwrap();
        prepare_install_root(install_dir.path());
        let user_dir = TempDir::new().unwrap();
        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);

        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();

        let logger = TestLogger::new(paths.logs_dir().join("lc-launcher.log"));
        logger.log_line("artifacts start").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-artifacts.log");
        fs::write(&runtime_log, "runtime").unwrap();
        let crash_log = paths.logs_dir().join("LegacyClonk-crash-artifacts.dmp");
        fs::write(&crash_log, "crash").unwrap();
        let bundle_path = paths.logs_dir().join("support-bundle.zip");
        fs::write(&bundle_path, b"bundle").unwrap();

        let telemetry = UpdateTelemetrySummary::default();
        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log],
            &[crash_log],
            &telemetry,
            Some(&bundle_path),
        )
        .unwrap();

        let state = load_shell_state(&paths)
            .unwrap()
            .expect("state should exist for artifact test");
        let artifacts = support_artifacts(&state);
        assert_eq!(artifacts.len(), 2, "expected bundle and summary artifacts");
        assert_eq!(artifacts[0].path, bundle_path);
        assert_eq!(artifacts[0].role, "support bundle");
        assert_eq!(
            artifacts[1].path,
            paths.logs_dir().join("launcher-summary.json")
        );
        assert_eq!(artifacts[1].role, "launcher summary");
    }

    #[test]
    fn reveal_in_file_manager_rejects_missing_path() {
        let missing = PathBuf::from("does-not-exist/support-bundle.zip");
        let err = reveal_in_file_manager(&missing).unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "expected missing path error, got {err:?}"
        );
    }
}
