use crate::provider::{ProviderAutomationState, ProviderPathStatus};
use crate::summary::{
    load_launcher_summary, LauncherSummaryRecord, ProviderAutomationRecord,
    ProviderBulkRetargetRecord, ProviderBulkRetargetSummary, ProviderOverrideSourceRecord,
};
use crate::telemetry::UpdateTelemetrySummary;
use lc_platform::AppPaths;
use std::path::{Path, PathBuf};

pub fn render_support_bundle_report(
    paths: &AppPaths,
    support_bundle: Option<&Path>,
    telemetry_summary: &UpdateTelemetrySummary,
) -> Vec<String> {
    let mut lines = Vec::new();

    let summary_path = paths.logs_dir().join("launcher-summary.json");
    lines.push(format!(
        "Launcher summary written to {}",
        summary_path.display()
    ));
    match support_bundle {
        Some(path) => lines.push(format!("Support bundle available at {}", path.display())),
        None => {
            lines.push("Support bundle was not created; check launcher logs for details.".into())
        }
    }

    lines.extend(telemetry_report_lines(telemetry_summary));

    lines.push(String::new());
    lines.extend(provider_section_lines(paths));

    lines.push(String::new());
    match support_bundle {
        Some(_) => lines.push(
            "Share the support bundle when filing bugs to include launcher, runtime, and telemetry logs."
                .into(),
        ),
        None => lines.push(
            "Share launcher-summary.json when filing bugs so support can collect the right logs."
                .into(),
        ),
    }

    lines
}

fn telemetry_report_lines(telemetry_summary: &UpdateTelemetrySummary) -> Vec<String> {
    if !telemetry_summary.failures().is_empty() {
        let mut lines = vec!["Updater issues detected:".into()];
        for failure in telemetry_summary.failures() {
            lines.push(format!(
                "  {} -> {}",
                failure.log_path.display(),
                failure.message
            ));
        }
        lines
    } else if !telemetry_summary.successes().is_empty() {
        let successes = telemetry_summary
            .successes()
            .iter()
            .map(|path| filename_or_display(path))
            .collect::<Vec<_>>()
            .join(", ");
        vec![format!(
            "Updater telemetry success recorded in: {successes}"
        )]
    } else {
        vec!["Updater telemetry: no signals captured in the collected runtime logs.".into()]
    }
}

fn provider_section_lines(paths: &AppPaths) -> Vec<String> {
    match load_launcher_summary(paths) {
        Ok(Some(record)) => provider_report_lines(&record),
        Ok(None) => vec!["First-party providers: launcher summary not available yet.".into()],
        Err(err) => vec![format!(
            "First-party providers: failed to load launcher summary ({err})."
        )],
    }
}

fn provider_report_lines(record: &LauncherSummaryRecord) -> Vec<String> {
    let snapshot = &record.summary.provider_automation;
    let has_share = !snapshot.share.is_empty();
    let has_upload = !snapshot.upload.is_empty();
    let has_automation = has_share || has_upload;

    let mut lines = Vec::new();
    if has_automation {
        lines.push(format!(
            "First-party providers (logs dir: {}):",
            record.logs_dir.display()
        ));
        if has_share {
            lines.push("  Share targets:".into());
            lines.extend(provider_category_lines(&snapshot.share, &record.logs_dir));
        }
        if has_upload {
            lines.push("  Upload targets:".into());
            lines.extend(provider_category_lines(&snapshot.upload, &record.logs_dir));
        }
    } else {
        lines.push("First-party providers: no automation targets recorded.".into());
    }

    if let Some(summary) = record.summary.provider_bulk_retarget.as_ref() {
        lines.extend(bulk_retarget_lines(summary, &record.logs_dir));
    }

    lines
}

fn provider_category_lines(providers: &[ProviderAutomationRecord], logs_dir: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    for provider in providers {
        lines.extend(provider_entry_lines("    ", provider, logs_dir));
    }
    lines
}

fn provider_entry_lines(
    indent: &str,
    provider: &ProviderAutomationRecord,
    logs_dir: &Path,
) -> Vec<String> {
    let mut lines = Vec::new();
    let current_path = resolve_summary_entry(logs_dir, &provider.path);
    lines.push(format!(
        "{indent}- {} ({})",
        provider.name,
        describe_provider_path_status(&provider.path_status)
    ));
    lines.push(format!(
        "{indent}  Current path: {}",
        current_path.display()
    ));
    lines.push(format!(
        "{indent}  Automation: {}",
        describe_provider_automation(&provider.automation)
    ));

    let default_entry = provider.default_path.as_deref().unwrap_or(&provider.path);
    let default_path = resolve_summary_entry(logs_dir, default_entry);
    lines.push(format!(
        "{indent}  Default path: {}",
        default_path.display()
    ));

    if provider.overrides.is_empty() {
        lines.push(format!("{indent}  Overrides: none recorded."));
    } else {
        lines.push(format!("{indent}  Overrides:"));
        for override_entry in &provider.overrides {
            let path = resolve_summary_entry(logs_dir, &override_entry.path);
            let source = describe_override_source(&override_entry.source);
            lines.push(format!("{indent}    - {} -> {}", source, path.display()));
        }
    }
    lines
}

fn bulk_retarget_lines(summary: &ProviderBulkRetargetSummary, logs_dir: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    let has_records = !summary.share.is_empty() || !summary.upload.is_empty();
    if !has_records && summary.history_cleared_at.is_none() {
        return lines;
    }

    lines.push("  Bulk retarget history:".into());
    if has_records {
        if !summary.share.is_empty() {
            lines.push("    Share targets:".into());
            lines.extend(bulk_retarget_category_lines(
                "      ",
                &summary.share,
                logs_dir,
            ));
        }
        if !summary.upload.is_empty() {
            lines.push("    Upload targets:".into());
            lines.extend(bulk_retarget_category_lines(
                "      ",
                &summary.upload,
                logs_dir,
            ));
        }
    }

    if let Some(cleared_at) = &summary.history_cleared_at {
        let message = if has_records {
            format!("    Bulk retarget history last cleared at {cleared_at}.")
        } else {
            format!(
                "    Bulk retarget history was cleared at {cleared_at}. No retarget records remain while providers use default staging paths."
            )
        };
        lines.push(message);
    }

    lines
}

fn bulk_retarget_category_lines(
    indent: &str,
    records: &[ProviderBulkRetargetRecord],
    logs_dir: &Path,
) -> Vec<String> {
    let mut lines = Vec::new();
    for record in records {
        let base_path = resolve_summary_entry(logs_dir, &record.base_path);
        lines.push(format!(
            "{indent}- {} (last retargeted at {}, changed {} of {} targets)",
            base_path.display(),
            record.retargeted_at,
            record.changed,
            record.total
        ));
    }
    lines
}

fn resolve_summary_entry(logs_dir: &Path, entry: &str) -> PathBuf {
    let candidate = Path::new(entry);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        logs_dir.join(candidate)
    }
}

fn describe_provider_path_status(status: &ProviderPathStatus) -> String {
    match status {
        ProviderPathStatus::Ready => "ready".into(),
        ProviderPathStatus::Missing => "missing".into(),
        ProviderPathStatus::NotDirectory => "not a directory".into(),
        ProviderPathStatus::Inaccessible(err) => format!("inaccessible ({err})"),
    }
}

fn describe_provider_automation(state: &ProviderAutomationState) -> String {
    match state {
        ProviderAutomationState::Idle => "Idle".into(),
        ProviderAutomationState::Submitted { detail } => format!("Submitted ({detail})"),
        ProviderAutomationState::Stale { reason } => format!("Stale ({reason})"),
        ProviderAutomationState::Skipped { reason } => format!("Skipped ({reason})"),
        ProviderAutomationState::Failed { error } => format!("Failed ({error})"),
    }
}

fn describe_override_source(source: &ProviderOverrideSourceRecord) -> String {
    match source {
        ProviderOverrideSourceRecord::Preference => "Launcher preference".into(),
        ProviderOverrideSourceRecord::Retargeted { applied_at } => {
            format!("Retargeted at {applied_at}")
        }
    }
}

fn filename_or_display(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.display().to_string())
}
