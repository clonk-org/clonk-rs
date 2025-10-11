use crate::preferences::{ReportSearchHighlightPreference, ReportSearchPreferences};
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
    let (provider_lines, summary_record) = match load_launcher_summary(paths) {
        Ok(Some(record)) => (provider_report_lines(&record), Some(record)),
        Ok(None) => (
            vec!["First-party providers: launcher summary not available yet.".into()],
            None,
        ),
        Err(err) => (
            vec![format!(
                "First-party providers: failed to load launcher summary ({err})."
            )],
            None,
        ),
    };
    lines.extend(provider_lines);

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

    let base_lines = lines.clone();
    if let Some(record) = summary_record.as_ref() {
        if let Some(annotations) = report_search_triage_lines(record, &base_lines) {
            lines.extend(annotations);
        }
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

fn report_search_triage_lines(
    record: &LauncherSummaryRecord,
    lines: &[String],
) -> Option<Vec<String>> {
    let preferences = record.summary.report_search.as_ref()?;
    triage_lines_for_search(preferences, lines)
}

fn triage_lines_for_search(
    preferences: &ReportSearchPreferences,
    lines: &[String],
) -> Option<Vec<String>> {
    let normalized_query = preferences.query.to_lowercase();
    if !search_has_term(preferences.highlight, &normalized_query) {
        return None;
    }

    let matches = collect_search_matches(preferences.highlight, &normalized_query, lines);
    let mut annotations = Vec::new();
    annotations.push(String::new());

    let label = highlight_label(preferences.highlight);
    let query_display = query_display(preferences);

    if matches.is_empty() {
        annotations.push(format!(
            "Search ({label}): {query_display} — no matches found."
        ));
        return Some(annotations);
    }

    annotations.push(format!(
        "Search ({label}): {query_display} — {} match(es).",
        matches.len()
    ));

    let active_match = preferences
        .active_line
        .and_then(|line| matches.iter().position(|candidate| *candidate == line));

    for (index, line_index) in matches.iter().enumerate() {
        let prefix = if Some(index) == active_match {
            "  *"
        } else {
            "   "
        };
        let line_number = line_index + 1;
        let line_text = lines
            .get(*line_index)
            .map(|line| line.as_str())
            .unwrap_or("<line unavailable>");
        annotations.push(format!("{prefix} Line {line_number}: {line_text}"));
    }

    Some(annotations)
}

fn collect_search_matches(
    highlight: ReportSearchHighlightPreference,
    normalized_query: &str,
    lines: &[String],
) -> Vec<usize> {
    let mut matches = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if matches_line(highlight, normalized_query, line) {
            matches.push(index);
        }
    }
    matches
}

fn matches_line(
    highlight: ReportSearchHighlightPreference,
    normalized_query: &str,
    line: &str,
) -> bool {
    let lower = line.to_lowercase();
    match highlight {
        ReportSearchHighlightPreference::Generic => lower.contains(normalized_query),
        ReportSearchHighlightPreference::Error => {
            lower.contains("error") || lower.contains("failed") || lower.contains("fatal")
        }
        ReportSearchHighlightPreference::Warning => {
            lower.contains("warning") || lower.contains("warn")
        }
    }
}

fn search_has_term(highlight: ReportSearchHighlightPreference, normalized_query: &str) -> bool {
    match highlight {
        ReportSearchHighlightPreference::Generic => !normalized_query.is_empty(),
        ReportSearchHighlightPreference::Error | ReportSearchHighlightPreference::Warning => true,
    }
}

fn query_display(preferences: &ReportSearchPreferences) -> String {
    match preferences.highlight {
        ReportSearchHighlightPreference::Generic => {
            if preferences.query.is_empty() {
                "<none>".into()
            } else {
                format!("\"{}\"", preferences.query)
            }
        }
        ReportSearchHighlightPreference::Error | ReportSearchHighlightPreference::Warning => {
            format!("\"{}\"", preferences.query)
        }
    }
}

fn highlight_label(highlight: ReportSearchHighlightPreference) -> &'static str {
    match highlight {
        ReportSearchHighlightPreference::Generic => "text",
        ReportSearchHighlightPreference::Error => "errors",
        ReportSearchHighlightPreference::Warning => "warnings",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_lines() -> Vec<String> {
        vec![
            "Launcher summary written to /tmp/launcher-summary.json".into(),
            "Support bundle available at /tmp/support-bundle.zip".into(),
            "Updater telemetry: no signals captured in the collected runtime logs.".into(),
            "First-party providers: no automation targets recorded.".into(),
            "Share the support bundle when filing bugs to include launcher, runtime, and telemetry logs."
                .into(),
        ]
    }

    #[test]
    fn triage_lines_skip_when_no_term() {
        let lines = sample_lines();
        let preferences = ReportSearchPreferences {
            query: String::new(),
            highlight: ReportSearchHighlightPreference::Generic,
            active_line: None,
        };

        assert!(
            triage_lines_for_search(&preferences, &lines).is_none(),
            "triage block should be absent without a search term"
        );
    }

    #[test]
    fn triage_lines_report_no_matches() {
        let lines = sample_lines();
        let preferences = ReportSearchPreferences {
            query: "missing".into(),
            highlight: ReportSearchHighlightPreference::Generic,
            active_line: None,
        };

        let block = triage_lines_for_search(&preferences, &lines).expect("triage block");
        assert_eq!(
            block.len(),
            2,
            "block should include blank line and summary"
        );
        assert_eq!(block[0], "", "first line should be blank for separation");
        assert_eq!(
            block[1], "Search (text): \"missing\" — no matches found.",
            "summary line should report no matches"
        );
    }

    #[test]
    fn triage_lines_report_matches_and_active_entry() {
        let lines = sample_lines();
        let preferences = ReportSearchPreferences {
            query: "bundle".into(),
            highlight: ReportSearchHighlightPreference::Generic,
            active_line: Some(1),
        };

        let block = triage_lines_for_search(&preferences, &lines).expect("triage block");
        assert_eq!(
            block.len(),
            4,
            "block should include summary and match entries"
        );
        assert_eq!(block[0], "", "first line should separate block");
        assert_eq!(
            block[1], "Search (text): \"bundle\" — 2 match(es).",
            "summary should capture match count"
        );
        assert_eq!(
            block[2], "  * Line 2: Support bundle available at /tmp/support-bundle.zip",
            "active match should be highlighted with an asterisk"
        );
        assert_eq!(
            block[3],
            "    Line 5: Share the support bundle when filing bugs to include launcher, runtime, and telemetry logs.",
            "non-active matches should be indented without an asterisk"
        );
    }

    #[test]
    fn triage_lines_follow_error_preset_keywords() {
        let mut lines = sample_lines();
        lines.push("Updater issues detected:".into());
        lines.push("  /tmp/runtime.log -> fatal: updater crashed".into());

        let preferences = ReportSearchPreferences {
            query: "error".into(),
            highlight: ReportSearchHighlightPreference::Error,
            active_line: Some(6),
        };

        let block = triage_lines_for_search(&preferences, &lines).expect("triage block");
        assert_eq!(
            block[1], "Search (errors): \"error\" — 1 match(es).",
            "error preset should aggregate fatal/error keywords"
        );
        assert_eq!(
            block[2], "  * Line 7:   /tmp/runtime.log -> fatal: updater crashed",
            "active error match should be reported with preserved text"
        );
    }
}
