mod bundle;
mod localization;
mod log;
mod paths;
mod preferences;
mod provider;
mod report;
mod shell;
mod summary;
mod telemetry;
mod time;

pub use bundle::{append_support_bundle_report, create_support_bundle, regenerate_support_bundle};
pub use localization::{load_localization, Localization};
pub use log::LauncherLog;
pub use preferences::{
    load_launcher_preferences, save_launcher_preferences, LauncherPreferences,
    ReportSearchHighlightPreference, ReportSearchPreferences,
};
pub use provider::{
    ProviderAutomationState, ProviderDiagnostics, ProviderOverrideSource, ProviderPathProvenance,
    ProviderPathStatus, ProviderStatus,
};
pub use report::{
    build_support_bundle_report, render_support_bundle_report, report_search_triage_summary,
    ReportSearchTriageMatch, ReportSearchTriageSummary, SupportBundleReport,
};
pub use shell::{
    copy_support_artifacts, copy_support_bundle, ensure_support_bundle, load_shell_state,
    reveal_in_file_manager, support_artifacts, LauncherShellEnsureResult, LauncherShellState,
    LauncherTelemetryFailure, SupportArtifact,
};
pub use summary::{
    load_launcher_summary, write_launcher_summary, LauncherSummary, LauncherSummaryRecord,
    ProviderAutomationRecord, ProviderAutomationSnapshot, ProviderBulkRetargetRecord,
    ProviderBulkRetargetSummary, ProviderOverrideRecord, ProviderOverrideSourceRecord,
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
    use clonk_platform::AppPaths;
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

    struct LauncherPathsFixture {
        _install_dir: TempDir,
        _user_dir: TempDir,
        _guard: EnvGuard,
        paths: AppPaths,
    }

    impl std::ops::Deref for LauncherPathsFixture {
        type Target = AppPaths;

        fn deref(&self) -> &Self::Target {
            &self.paths
        }
    }

    fn launcher_paths_fixture() -> LauncherPathsFixture {
        let install_dir = TempDir::new().unwrap();
        prepare_install_root(install_dir.path());
        let user_dir = TempDir::new().unwrap();
        let guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.path())),
        ]);
        let paths = AppPaths::discover().unwrap();
        paths.ensure_user_dirs().unwrap();
        LauncherPathsFixture {
            _install_dir: install_dir,
            _user_dir: user_dir,
            _guard: guard,
            paths,
        }
    }

    #[test]
    fn load_shell_state_none_without_summary() {
        let paths = launcher_paths_fixture();

        assert!(
            load_shell_state(&paths).unwrap().is_none(),
            "state should be absent when no summary exists"
        );
    }

    #[test]
    fn load_shell_state_aggregates_paths() {
        let paths = launcher_paths_fixture();

        let logger = TestLogger::new(paths.logs_dir().join("clonk-launcher.log"));
        logger.log_line("launcher ready").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-session.log");
        fs::write(&runtime_log, "runtime").unwrap();
        let crash_log = paths.logs_dir().join("ClonkRust-crash.dmp");
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
            std::slice::from_ref(&runtime_log),
            std::slice::from_ref(&crash_log),
            &telemetry,
            Some(&bundle_path),
            None,
            None,
            None,
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
        assert_eq!(state.runtime_log_paths, vec![runtime_log.clone()]);
        assert_eq!(state.crash_report_paths, vec![crash_log.clone()]);
        assert_eq!(state.support_bundle_path, Some(bundle_path));
        assert_eq!(state.telemetry_success_logs, vec![runtime_log.clone()]);
        assert_eq!(state.telemetry_failures.len(), 1);
        assert_eq!(state.telemetry_failures[0].log_path, runtime_log);
        assert_eq!(
            state.telemetry_failures[0].message,
            "c4group returned status 1"
        );
    }

    #[test]
    fn ensure_support_bundle_regenerates_bundle() {
        let paths = launcher_paths_fixture();

        let logger = TestLogger::new(paths.logs_dir().join("clonk-launcher.log"));
        logger.log_line("regeneration start").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-regenerate.log");
        fs::write(&runtime_log, "Done.\n").unwrap();
        let crash_log = paths.logs_dir().join("ClonkRust-crash-regenerate.dmp");
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
            None,
            None,
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
    fn copy_support_artifacts_stages_files_with_unique_names() {
        let temp_dir = TempDir::new().unwrap();
        let bundle = temp_dir.path().join("support-bundle.zip");
        let summary = temp_dir.path().join("launcher-summary.json");
        fs::write(&bundle, b"bundle").unwrap();
        fs::write(&summary, b"summary").unwrap();

        let artifacts = vec![
            SupportArtifact {
                path: bundle.clone(),
                role: "support bundle",
            },
            SupportArtifact {
                path: summary.clone(),
                role: "launcher summary",
            },
        ];

        let destination = TempDir::new().unwrap();
        let copied = copy_support_artifacts(&artifacts, destination.path()).unwrap();
        assert_eq!(copied.len(), 2, "expected both artifacts to be copied");
        for path in &copied {
            assert!(
                path.exists(),
                "copied artifact {} should exist",
                path.display()
            );
        }

        // Copying again should produce suffixed names.
        let copied_again = copy_support_artifacts(&artifacts, destination.path()).unwrap();
        assert!(
            copied_again.iter().any(|path| path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.contains("-copy"))
                .unwrap_or(false)),
            "expected second copy to receive -copy suffix"
        );
    }

    #[test]
    fn support_artifacts_list_bundle_and_summary() {
        let paths = launcher_paths_fixture();

        let logger = TestLogger::new(paths.logs_dir().join("clonk-launcher.log"));
        logger.log_line("artifacts start").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-artifacts.log");
        fs::write(&runtime_log, "runtime").unwrap();
        let crash_log = paths.logs_dir().join("ClonkRust-crash-artifacts.dmp");
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
            None,
            None,
            None,
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

    #[test]
    fn write_launcher_summary_records_provider_automation() {
        let paths = launcher_paths_fixture();

        let logger = TestLogger::new(paths.logs_dir().join("clonk-launcher.log"));
        logger.log_line("provider automation test start").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-provider.log");
        fs::write(&runtime_log, "runtime").unwrap();
        let crash_log = paths.logs_dir().join("ClonkRust-crash-provider.dmp");
        fs::write(&crash_log, "crash").unwrap();

        let share_dir = paths.logs_dir().join("support-share");
        fs::create_dir_all(&share_dir).unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());

        let mut diagnostics = ProviderDiagnostics::default();
        diagnostics.share.push(ProviderStatus {
            name: "Support Share Drop".into(),
            path: share_dir.clone(),
            path_status: ProviderPathStatus::Ready,
            automation: ProviderAutomationState::Submitted {
                detail: "submission-request-share-test.json".into(),
            },
            path_provenance: ProviderPathProvenance::new(share_dir.clone()),
        });

        let snapshot = ProviderAutomationSnapshot::from_diagnostics(&diagnostics, paths.logs_dir());

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log],
            &[crash_log],
            &telemetry,
            None,
            Some(snapshot),
            None,
            None,
        )
        .unwrap();

        let summary_path = paths.logs_dir().join("launcher-summary.json");
        let summary_json = fs::read_to_string(summary_path).unwrap();
        let document: Value = serde_json::from_str(&summary_json).unwrap();

        let share_entries = document["provider_automation"]["share"].as_array().unwrap();
        assert_eq!(share_entries.len(), 1, "expected one share provider entry");
        let entry = &share_entries[0];
        assert_eq!(entry["name"].as_str(), Some("Support Share Drop"));
        assert_eq!(entry["path"].as_str(), Some("support-share"));
        assert_eq!(entry["path_status"].as_str(), Some("Ready"));
        assert_eq!(
            entry["automation"]["Submitted"]["detail"].as_str(),
            Some("submission-request-share-test.json")
        );
    }

    #[test]
    fn write_launcher_summary_includes_override_provenance() {
        let paths = launcher_paths_fixture();

        let logger = TestLogger::new(paths.logs_dir().join("clonk-launcher.log"));
        logger
            .log_line("override provenance test start")
            .expect("log line");

        let runtime_log = paths.logs_dir().join("Clonk-provider.log");
        fs::write(&runtime_log, "runtime").unwrap();

        let default_path = paths.logs_dir().join("support-share");
        let override_path = PathBuf::from("/tmp/custom-support-share");
        fs::create_dir_all(&default_path).unwrap();

        let mut provenance = ProviderPathProvenance::new(default_path.clone());
        provenance.apply_override(
            override_path.clone(),
            ProviderOverrideSource::Retargeted {
                applied_at: "2024-05-01T12:34:56Z".into(),
            },
        );

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());

        let mut diagnostics = ProviderDiagnostics::default();
        diagnostics.share.push(ProviderStatus {
            name: "Support Share Drop".into(),
            path: override_path.clone(),
            path_status: ProviderPathStatus::Ready,
            automation: ProviderAutomationState::Submitted {
                detail: "submission-request-share-test.json".into(),
            },
            path_provenance: provenance,
        });

        let snapshot = ProviderAutomationSnapshot::from_diagnostics(&diagnostics, paths.logs_dir());

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log],
            &[],
            &telemetry,
            None,
            Some(snapshot),
            None,
            None,
        )
        .unwrap();

        let summary_path = paths.logs_dir().join("launcher-summary.json");
        let summary_json = fs::read_to_string(summary_path).unwrap();
        let document: Value = serde_json::from_str(&summary_json).unwrap();

        let share_entries = document["provider_automation"]["share"]
            .as_array()
            .expect("share array");
        assert_eq!(share_entries.len(), 1);

        let entry = &share_entries[0];
        assert_eq!(entry["path"].as_str(), Some("/tmp/custom-support-share"));
        assert_eq!(entry["default_path"].as_str(), Some("support-share"));
        let overrides = entry["overrides"].as_array().expect("overrides array");
        assert_eq!(overrides.len(), 1);
        assert_eq!(
            overrides[0]["path"].as_str(),
            Some("/tmp/custom-support-share")
        );
        assert_eq!(overrides[0]["source"]["kind"].as_str(), Some("retargeted"));
        assert_eq!(
            overrides[0]["source"]["applied_at"].as_str(),
            Some("2024-05-01T12:34:56Z")
        );
    }

    #[test]
    fn write_launcher_summary_records_bulk_retarget_summary() {
        let paths = launcher_paths_fixture();

        let logger = TestLogger::new(paths.logs_dir().join("clonk-launcher.log"));
        logger
            .log_line("bulk retarget summary test start")
            .expect("log line");

        let runtime_log = paths.logs_dir().join("Clonk-bulk.log");
        fs::write(&runtime_log, "runtime").unwrap();

        let mut telemetry = UpdateTelemetrySummary::default();
        telemetry.record_success(runtime_log.clone());

        let mut summary = ProviderBulkRetargetSummary::default();
        summary.share.push(ProviderBulkRetargetRecord {
            base_path: "support-share".into(),
            retargeted_at: "2024-06-01T00:00:00Z".into(),
            total: 2,
            changed: 1,
        });

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log],
            &[],
            &telemetry,
            None,
            None,
            Some(summary),
            None,
        )
        .unwrap();

        let summary_path = paths.logs_dir().join("launcher-summary.json");
        let summary_json = fs::read_to_string(summary_path).unwrap();
        let document: Value = serde_json::from_str(&summary_json).unwrap();

        let share_summary = document["provider_bulk_retarget"]["share"]
            .as_array()
            .expect("share bulk array");
        assert_eq!(share_summary.len(), 1, "expected one share bulk record");
        let record = &share_summary[0];
        assert_eq!(
            record["base_path"].as_str(),
            Some("support-share"),
            "base path should be recorded"
        );
        assert_eq!(
            record["retargeted_at"].as_str(),
            Some("2024-06-01T00:00:00Z"),
            "bulk retarget timestamp should be recorded"
        );
        assert_eq!(
            record["total"].as_u64(),
            Some(2),
            "total bulk retarget count should match"
        );
        assert_eq!(
            record["changed"].as_u64(),
            Some(1),
            "changed bulk retarget count should match"
        );
    }

    #[test]
    fn write_launcher_summary_records_history_cleared_marker() {
        let paths = launcher_paths_fixture();

        let logger = TestLogger::new(paths.logs_dir().join("clonk-launcher.log"));
        let runtime_log = paths.logs_dir().join("Clonk-bulk.log");
        fs::write(&runtime_log, "runtime").unwrap();

        let telemetry = UpdateTelemetrySummary::default();

        let summary = ProviderBulkRetargetSummary {
            history_cleared_at: Some("2024-06-05T08:15:00Z".into()),
            ..Default::default()
        };

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log],
            &[],
            &telemetry,
            None,
            None,
            Some(summary),
            None,
        )
        .unwrap();

        let summary_path = paths.logs_dir().join("launcher-summary.json");
        let summary_json = fs::read_to_string(summary_path).unwrap();
        let document: Value = serde_json::from_str(&summary_json).unwrap();
        assert_eq!(
            document["provider_bulk_retarget"]["history_cleared_at"].as_str(),
            Some("2024-06-05T08:15:00Z"),
            "history cleared marker should be persisted"
        );
    }

    #[test]
    fn write_launcher_summary_records_report_search_preferences() {
        let paths = launcher_paths_fixture();

        let logger = TestLogger::new(paths.logs_dir().join("clonk-launcher.log"));
        logger.log_line("report search summary test").unwrap();

        let runtime_log = paths.logs_dir().join("Clonk-search.log");
        fs::write(&runtime_log, "runtime").unwrap();

        let telemetry = UpdateTelemetrySummary::default();
        let search = ReportSearchPreferences {
            query: "error".into(),
            highlight: ReportSearchHighlightPreference::Error,
            active_line: Some(41),
        };

        write_launcher_summary(
            &paths,
            &logger,
            logger.path(),
            &[runtime_log],
            &[],
            &telemetry,
            None,
            None,
            None,
            Some(search.clone()),
        )
        .unwrap();

        let summary_record = load_launcher_summary(&paths)
            .unwrap()
            .expect("launcher summary should exist");
        assert_eq!(summary_record.summary.report_search, Some(search));
    }
}
