use crate::preferences::{ReportSearchHighlightPreference, ReportSearchPreferences};
use crate::provider::{ProviderAutomationState, ProviderPathStatus};
use crate::summary::{
    load_launcher_summary, LauncherSummaryRecord, ProviderAutomationRecord,
    ProviderBulkRetargetRecord, ProviderBulkRetargetSummary, ProviderOverrideSourceRecord,
};
use crate::telemetry::UpdateTelemetrySummary;
use clonk_platform::AppPaths;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SupportBundleReport {
    pub lines: Vec<String>,
    pub triage: Option<ReportSearchTriageSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportSearchTriageSummary {
    pub label: String,
    pub query: String,
    pub query_display: String,
    pub highlight: ReportSearchHighlightPreference,
    pub match_count: usize,
    pub active_match_index: Option<usize>,
    pub matches: Vec<ReportSearchTriageMatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportSearchTriageMatch {
    pub line_index: usize,
    pub line_number: usize,
    pub text: String,
    pub active: bool,
}

pub fn build_support_bundle_report(
    paths: &AppPaths,
    support_bundle: Option<&Path>,
    telemetry_summary: &UpdateTelemetrySummary,
) -> SupportBundleReport {
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
        let triage = report_search_triage_summary(record, &base_lines);
        if let Some(summary) = triage.as_ref() {
            lines.push(String::new());
            lines.extend(render_triage_summary_lines(summary));
        }
        return SupportBundleReport { lines, triage };
    }

    SupportBundleReport {
        lines,
        triage: None,
    }
}

pub fn render_support_bundle_report(
    paths: &AppPaths,
    support_bundle: Option<&Path>,
    telemetry_summary: &UpdateTelemetrySummary,
) -> Vec<String> {
    build_support_bundle_report(paths, support_bundle, telemetry_summary).lines
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

pub fn report_search_triage_summary(
    record: &LauncherSummaryRecord,
    lines: &[String],
) -> Option<ReportSearchTriageSummary> {
    let preferences = record.summary.report_search.as_ref()?;
    triage_summary_for_search(preferences, lines)
}

fn triage_summary_for_search(
    preferences: &ReportSearchPreferences,
    lines: &[String],
) -> Option<ReportSearchTriageSummary> {
    let normalized_query = preferences.query.to_lowercase();
    if !search_has_term(preferences.highlight, &normalized_query) {
        return None;
    }

    let matches = collect_search_matches(preferences.highlight, &normalized_query, lines);
    let label = highlight_label(preferences.highlight);
    let query_display = query_display(preferences);

    let active_match = preferences
        .active_line
        .and_then(|line| matches.iter().position(|candidate| *candidate == line));

    let mut match_entries = Vec::with_capacity(matches.len());
    for (index, line_index) in matches.iter().enumerate() {
        let line_number = *line_index + 1;
        let line_text = lines
            .get(*line_index)
            .cloned()
            .unwrap_or_else(|| "<line unavailable>".into());
        match_entries.push(ReportSearchTriageMatch {
            line_index: *line_index,
            line_number,
            text: line_text,
            active: Some(index) == active_match,
        });
    }

    Some(ReportSearchTriageSummary {
        label: label.to_string(),
        query: preferences.query.clone(),
        query_display,
        highlight: preferences.highlight,
        match_count: match_entries.len(),
        active_match_index: active_match,
        matches: match_entries,
    })
}

fn render_triage_summary_lines(summary: &ReportSearchTriageSummary) -> Vec<String> {
    if summary.match_count == 0 {
        return vec![format!(
            "Search ({}): {} — no matches found.",
            summary.label, summary.query_display
        )];
    }

    let mut lines = Vec::with_capacity(1 + summary.matches.len());
    lines.push(format!(
        "Search ({}): {} — {} match(es).",
        summary.label, summary.query_display, summary.match_count
    ));

    for (index, entry) in summary.matches.iter().enumerate() {
        let prefix = if Some(index) == summary.active_match_index {
            "  *"
        } else {
            "   "
        };
        lines.push(format!(
            "{prefix} Line {}: {}",
            entry.line_number, entry.text
        ));
    }

    lines
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
    fn triage_summary_skip_when_no_term() {
        let lines = sample_lines();
        let preferences = ReportSearchPreferences {
            query: String::new(),
            highlight: ReportSearchHighlightPreference::Generic,
            active_line: None,
        };

        assert!(
            triage_summary_for_search(&preferences, &lines).is_none(),
            "triage summary should be absent without a search term"
        );
    }

    #[test]
    fn triage_summary_reports_no_matches() {
        let lines = sample_lines();
        let preferences = ReportSearchPreferences {
            query: "missing".into(),
            highlight: ReportSearchHighlightPreference::Generic,
            active_line: None,
        };

        let summary =
            triage_summary_for_search(&preferences, &lines).expect("triage summary to exist");
        assert_eq!(summary.match_count, 0, "should report zero matches");
        assert!(
            summary.matches.is_empty(),
            "no match entries should be generated"
        );

        let rendered = render_triage_summary_lines(&summary);
        assert_eq!(
            rendered,
            vec!["Search (text): \"missing\" — no matches found.".to_string()],
            "rendered summary should match legacy formatting"
        );
    }

    #[test]
    fn triage_summary_reports_matches_and_active_entry() {
        let lines = sample_lines();
        let preferences = ReportSearchPreferences {
            query: "bundle".into(),
            highlight: ReportSearchHighlightPreference::Generic,
            active_line: Some(1),
        };

        let summary =
            triage_summary_for_search(&preferences, &lines).expect("triage summary to exist");
        assert_eq!(summary.match_count, 2, "two matches should be detected");
        assert_eq!(
            summary.active_match_index,
            Some(0),
            "first match should be marked as active"
        );
        assert_eq!(
            summary.matches[0].line_number, 2,
            "line numbering should be 1-based"
        );
        assert!(
            summary.matches[0].active,
            "first match should be flagged active"
        );
        assert!(
            !summary.matches[1].active,
            "second match should be inactive"
        );

        let rendered = render_triage_summary_lines(&summary);
        assert_eq!(
            rendered[0], "Search (text): \"bundle\" — 2 match(es).",
            "summary header should report match count"
        );
        assert_eq!(
            rendered[1], "  * Line 2: Support bundle available at /tmp/support-bundle.zip",
            "active match should use asterisk prefix"
        );
        assert_eq!(
            rendered[2],
            "    Line 5: Share the support bundle when filing bugs to include launcher, runtime, and telemetry logs.",
            "inactive match should be indented without asterisk"
        );
    }

    #[test]
    fn triage_summary_follows_error_preset_keywords() {
        let mut lines = sample_lines();
        lines.push("Updater issues detected:".into());
        lines.push("  /tmp/runtime.log -> fatal: updater crashed".into());

        let preferences = ReportSearchPreferences {
            query: "error".into(),
            highlight: ReportSearchHighlightPreference::Error,
            active_line: Some(6),
        };

        let summary =
            triage_summary_for_search(&preferences, &lines).expect("triage summary to exist");
        assert_eq!(
            summary.match_count, 1,
            "fatal keyword should count as error"
        );
        assert_eq!(
            summary.matches[0].line_number, 7,
            "reported line number should match match position"
        );
        assert!(
            summary.matches[0].active,
            "fatal entry should be the active match"
        );

        let rendered = render_triage_summary_lines(&summary);
        assert_eq!(
            rendered[0], "Search (errors): \"error\" — 1 match(es).",
            "error preset should surface summary with match count"
        );
        assert_eq!(
            rendered[1], "  * Line 7:   /tmp/runtime.log -> fatal: updater crashed",
            "fatal keyword should appear as the highlighted entry"
        );
    }
}
