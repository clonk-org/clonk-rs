use crate::log::LauncherLog;
use crate::paths::{ensure_logs_dir, launcher_summary_path, resolve_logs_entry};
use crate::report::render_support_bundle_report;
use crate::summary::{load_launcher_summary, write_launcher_summary};
use crate::telemetry::UpdateTelemetrySummary;
use crate::time::timestamp_for_filename;
use anyhow::{anyhow, Context, Result};
use clonk_platform::AppPaths;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use zip::write::FileOptions;
use zip::CompressionMethod;

pub fn create_support_bundle(
    paths: &AppPaths,
    logger: &dyn LauncherLog,
    launcher_log_path: &Path,
    runtime_logs: &[PathBuf],
    crash_reports: &[PathBuf],
    telemetry_summary: &UpdateTelemetrySummary,
) -> Result<Option<PathBuf>> {
    let logs_dir = ensure_logs_dir(paths)?;
    let bundle_path = logs_dir.join(format!("support-bundle-{}.zip", timestamp_for_filename()));
    let file = File::create(&bundle_path).with_context(|| {
        format!(
            "failed to create support bundle archive at {}",
            bundle_path.display()
        )
    })?;

    let mut writer = zip::ZipWriter::new(file);
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut entries_written = 0usize;

    match add_file_to_bundle(
        &mut writer,
        &format!(
            "launcher/{}",
            launcher_log_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("clonk-game.log")
        ),
        launcher_log_path,
        options,
        "launcher log",
    ) {
        Ok(()) => entries_written += 1,
        Err(err) => {
            let _ = logger.log_line(&format!(
                "failed to include launcher log in support bundle: {err}"
            ));
        }
    }

    for (index, path) in runtime_logs.iter().enumerate() {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("runtime.log");
        let entry_name = format!("runtime/{:02}_{}", index + 1, file_name);
        match add_file_to_bundle(&mut writer, &entry_name, path, options, "runtime log") {
            Ok(()) => entries_written += 1,
            Err(err) => {
                let _ = logger.log_line(&format!(
                    "failed to include runtime log {}: {err}",
                    path.display()
                ));
            }
        }
    }

    for (index, path) in crash_reports.iter().enumerate() {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("crash.dmp");
        let entry_name = format!("crash/{:02}_{}", index + 1, file_name);
        match add_file_to_bundle(&mut writer, &entry_name, path, options, "crash artifact") {
            Ok(()) => entries_written += 1,
            Err(err) => {
                let _ = logger.log_line(&format!(
                    "failed to include crash artifact {}: {err}",
                    path.display()
                ));
            }
        }
    }

    let telemetry_payload =
        serde_json::to_vec_pretty(&telemetry_summary.to_serializable(&logs_dir))
            .context("failed to serialize telemetry summary for support bundle")?;
    writer
        .start_file("telemetry-summary.json", options)
        .context("failed to create telemetry summary entry in support bundle")?;
    writer
        .write_all(&telemetry_payload)
        .context("failed to write telemetry summary into support bundle")?;
    entries_written += 1;

    writer
        .finish()
        .context("failed to finish writing support bundle")?;

    if entries_written > 0 {
        logger
            .log_line(&format!(
                "created support bundle at {}",
                bundle_path.display()
            ))
            .context("failed to log support bundle path")?;
        Ok(Some(bundle_path))
    } else {
        logger
            .log_line("support bundle skipped because no entries could be written")
            .context("failed to log support bundle skip")?;
        Ok(None)
    }
}

pub fn regenerate_support_bundle(
    paths: &AppPaths,
    logger: &dyn LauncherLog,
    launcher_log_path: &Path,
) -> Result<(PathBuf, UpdateTelemetrySummary)> {
    let record = load_launcher_summary(paths)?.ok_or_else(|| {
        anyhow!(
            "no launcher summary found at {}; launch clonk-game normally first",
            launcher_summary_path(paths.logs_dir()).display()
        )
    })?;

    logger
        .log_line("manual support bundle regeneration requested")
        .context("failed to record manual support bundle request")?;

    let runtime_logs = record
        .summary
        .runtime_logs
        .iter()
        .map(|entry| resolve_logs_entry(&record.logs_dir, entry))
        .collect::<Vec<_>>();
    let crash_reports = record
        .summary
        .crash_reports
        .iter()
        .map(|entry| resolve_logs_entry(&record.logs_dir, entry))
        .collect::<Vec<_>>();

    let telemetry = UpdateTelemetrySummary::from_serializable(
        &record.summary.update_telemetry,
        &record.logs_dir,
    );

    let bundle = create_support_bundle(
        paths,
        logger,
        launcher_log_path,
        &runtime_logs,
        &crash_reports,
        &telemetry,
    )?
    .ok_or_else(|| anyhow!("support bundle regeneration produced no entries"))?;

    write_launcher_summary(
        paths,
        logger,
        launcher_log_path,
        &runtime_logs,
        &crash_reports,
        &telemetry,
        Some(&bundle),
        Some(record.summary.provider_automation.clone()),
        record.summary.provider_bulk_retarget.clone(),
        record.summary.report_search.clone(),
    )?;

    append_support_bundle_report(paths, &bundle, &telemetry)?;

    Ok((bundle, telemetry))
}

pub fn append_support_bundle_report(
    paths: &AppPaths,
    bundle_path: &Path,
    telemetry_summary: &UpdateTelemetrySummary,
) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(bundle_path)
        .with_context(|| {
            format!(
                "failed to open support bundle {} for report append",
                bundle_path.display()
            )
        })?;

    let mut writer = zip::ZipWriter::new_append(file)
        .context("failed to prepare support bundle for report append")?;
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut report =
        render_support_bundle_report(paths, Some(bundle_path), telemetry_summary).join("\n");
    report.push('\n');

    writer
        .start_file("support-bundle-report.txt", options)
        .context("failed to create report entry in support bundle")?;
    writer
        .write_all(report.as_bytes())
        .context("failed to write support bundle report")?;
    writer
        .finish()
        .context("failed to finalise support bundle report append")?;

    Ok(())
}

fn add_file_to_bundle(
    writer: &mut zip::ZipWriter<File>,
    entry_name: &str,
    source: &Path,
    options: FileOptions,
    role: &str,
) -> Result<()> {
    let mut file = File::open(source)
        .with_context(|| format!("failed to open {role} {} for bundling", source.display()))?;
    writer
        .start_file(entry_name, options)
        .with_context(|| format!("failed to create archive entry {entry_name}"))?;
    io::copy(&mut file, writer)
        .with_context(|| format!("failed to copy {role} {} into bundle", source.display()))?;
    Ok(())
}
