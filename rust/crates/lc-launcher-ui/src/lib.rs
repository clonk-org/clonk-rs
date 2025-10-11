use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lc_gui::{
    DrawCommand, Gui, GuiAction, GuiEvent, GuiEventResult, GuiResult, Rect, Size, WidgetId,
};
use lc_launcher::{
    support_artifacts, LauncherShellState, ProviderAutomationState, ProviderBulkRetargetRecord,
    ProviderBulkRetargetSummary, ProviderDiagnostics, ProviderOverrideSource, ProviderPathStatus,
    ProviderStatus, SupportArtifact,
};

const REPORT_PREVIEW_VISIBLE_LINES: usize = 28;
const REPORT_PREVIEW_SCROLL_STEP: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Share,
    Upload,
}

impl ProviderKind {
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Share => "share target",
            ProviderKind::Upload => "upload target",
        }
    }
}

#[derive(Debug, Clone)]
pub enum LauncherShellMessage {
    RegenerateSupportBundle,
    CopySupportBundle { bundle_path: PathBuf },
    RevealPath { path: PathBuf, label: String },
    UploadSupportArtifacts { artifacts: Vec<SupportArtifact> },
    RestageProvider { role: ProviderKind, index: usize },
    RetargetProvider { role: ProviderKind, index: usize },
    ClearProviderOverride { role: ProviderKind, index: usize },
    RetargetAllProviders,
    RestoreAllProviderDefaults,
    CopyReportPreview,
    ExportReportPreview,
    ScrollReportPreview { delta: isize },
}

#[derive(Debug)]
pub struct LauncherShellResponse {
    pub gui: GuiEventResult,
    pub messages: Vec<LauncherShellMessage>,
}

#[derive(Debug, Clone)]
pub struct ActionFeedback {
    pub message: String,
    pub kind: ActionFeedbackKind,
}

impl ActionFeedback {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ActionFeedbackKind::Info,
        }
    }

    pub fn success(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ActionFeedbackKind::Success,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ActionFeedbackKind::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionFeedbackKind {
    Info,
    Success,
    Error,
}

impl ActionFeedbackKind {
    fn label(self) -> &'static str {
        match self {
            ActionFeedbackKind::Info => "Info",
            ActionFeedbackKind::Success => "Success",
            ActionFeedbackKind::Error => "Error",
        }
    }
}

pub struct LauncherShellUi {
    gui: Gui,
    layout: LauncherShellLayout,
    state: Option<LauncherShellState>,
    feedback: Option<ActionFeedback>,
    providers: ProviderDiagnostics,
    report_scroll_offset: usize,
}

impl LauncherShellUi {
    pub fn new(state: Option<LauncherShellState>) -> GuiResult<Self> {
        let mut ui = Self {
            gui: Gui::new(),
            layout: LauncherShellLayout::default(),
            state: None,
            feedback: None,
            providers: ProviderDiagnostics::default(),
            report_scroll_offset: 0,
        };
        ui.set_state(state)?;
        Ok(ui)
    }

    pub fn set_state(&mut self, state: Option<LauncherShellState>) -> GuiResult<()> {
        self.state = state;
        self.report_scroll_offset = 0;
        self.rebuild()
    }

    pub fn set_action_feedback(&mut self, feedback: Option<ActionFeedback>) -> GuiResult<()> {
        self.feedback = feedback;
        self.rebuild()
    }

    pub fn set_providers(&mut self, providers: ProviderDiagnostics) -> GuiResult<()> {
        self.providers = providers;
        self.rebuild()
    }

    pub fn layout(&mut self, available: Size) -> Size {
        self.gui.layout(available)
    }

    pub fn render(&self) -> Vec<DrawCommand> {
        self.gui.render()
    }

    pub fn widget_rect(&self, id: WidgetId) -> Option<Rect> {
        self.gui.rect_of(id)
    }

    pub fn handle_event(&mut self, event: GuiEvent) -> LauncherShellResponse {
        let result = self.gui.handle_event(event);
        self.process_gui_result(result)
    }

    pub fn regenerate_button(&self) -> Option<WidgetId> {
        self.layout.regenerate_button
    }

    pub fn copy_button(&self) -> Option<WidgetId> {
        self.layout.copy_button
    }

    pub fn upload_button(&self) -> Option<WidgetId> {
        self.layout.upload_button
    }

    pub fn restore_defaults_button(&self) -> Option<WidgetId> {
        self.layout.restore_defaults_button
    }

    pub fn retarget_all_button(&self) -> Option<WidgetId> {
        self.layout.retarget_all_button
    }

    pub fn report_copy_button(&self) -> Option<WidgetId> {
        self.layout.report_copy_button
    }

    pub fn report_export_button(&self) -> Option<WidgetId> {
        self.layout.report_export_button
    }

    pub fn report_scroll_up_button(&self) -> Option<WidgetId> {
        self.layout.report_scroll_up_button
    }

    pub fn report_scroll_down_button(&self) -> Option<WidgetId> {
        self.layout.report_scroll_down_button
    }

    pub fn report_line_range_text(&self) -> Option<&str> {
        self.layout
            .report_line_range
            .as_ref()
            .map(|text| text.as_str())
    }

    pub fn state(&self) -> Option<&LauncherShellState> {
        self.state.as_ref()
    }

    pub fn action_feedback(&self) -> Option<&ActionFeedback> {
        self.feedback.as_ref()
    }

    pub fn scroll_report_preview(&mut self, delta: isize) -> GuiResult<()> {
        let total_lines = self
            .state
            .as_ref()
            .map(|state| state.support_bundle_report.len())
            .unwrap_or(0);
        if total_lines <= REPORT_PREVIEW_VISIBLE_LINES {
            self.report_scroll_offset = 0;
        } else {
            let visible = REPORT_PREVIEW_VISIBLE_LINES.min(total_lines);
            let max_offset = total_lines - visible;
            let capped_max = max_offset.min(isize::MAX as usize) as isize;
            let capped_current = self
                .report_scroll_offset
                .min(max_offset)
                .min(isize::MAX as usize) as isize;
            let mut next = capped_current + delta;
            if next < 0 {
                next = 0;
            } else if next > capped_max {
                next = capped_max;
            }
            self.report_scroll_offset = next as usize;
        }
        self.rebuild()
    }

    fn rebuild(&mut self) -> GuiResult<()> {
        self.clamp_report_scroll_offset();
        let borrowed_state = self.state.as_ref();
        let feedback = self.feedback.as_ref();
        let (gui, layout) = build_gui(
            borrowed_state,
            feedback,
            &self.providers,
            self.report_scroll_offset,
        )?;
        self.gui = gui;
        self.layout = layout;
        Ok(())
    }

    fn process_gui_result(&mut self, gui_result: GuiEventResult) -> LauncherShellResponse {
        let mut messages = Vec::new();

        for (widget, action) in gui_result.actions.iter().copied() {
            if action != GuiAction::Activate {
                continue;
            }
            if let Some(widget_action) = self.layout.action_map.get(&widget) {
                if let Some(message) = widget_action.to_message(&self.layout) {
                    messages.push(message);
                }
            }
        }

        LauncherShellResponse {
            gui: gui_result,
            messages,
        }
    }

    fn clamp_report_scroll_offset(&mut self) {
        let total_lines = self
            .state
            .as_ref()
            .map(|state| state.support_bundle_report.len())
            .unwrap_or(0);
        if total_lines == 0 {
            self.report_scroll_offset = 0;
            return;
        }
        let visible = REPORT_PREVIEW_VISIBLE_LINES.min(total_lines);
        let max_offset = total_lines.saturating_sub(visible);
        if self.report_scroll_offset > max_offset {
            self.report_scroll_offset = max_offset;
        }
    }
}

#[derive(Default)]
struct LauncherShellLayout {
    action_map: HashMap<WidgetId, WidgetAction>,
    support_artifacts: Vec<SupportArtifact>,
    feedback_message: Option<String>,
    regenerate_button: Option<WidgetId>,
    copy_button: Option<WidgetId>,
    upload_button: Option<WidgetId>,
    restore_defaults_button: Option<WidgetId>,
    retarget_all_button: Option<WidgetId>,
    report_copy_button: Option<WidgetId>,
    report_export_button: Option<WidgetId>,
    report_scroll_up_button: Option<WidgetId>,
    report_scroll_down_button: Option<WidgetId>,
    report_line_range: Option<String>,
}

enum WidgetAction {
    RegenerateBundle,
    CopyBundle { bundle_path: PathBuf },
    Reveal { path: PathBuf, label: String },
    UploadArtifacts,
    RestageProvider { role: ProviderKind, index: usize },
    RetargetProvider { role: ProviderKind, index: usize },
    ClearProviderOverride { role: ProviderKind, index: usize },
    RetargetAllProviders,
    RestoreAllProviderDefaults,
    CopyReportPreview,
    ExportReportPreview,
    ScrollReportPreview { delta: isize },
}

impl WidgetAction {
    fn to_message(&self, layout: &LauncherShellLayout) -> Option<LauncherShellMessage> {
        match self {
            WidgetAction::RegenerateBundle => Some(LauncherShellMessage::RegenerateSupportBundle),
            WidgetAction::CopyBundle { bundle_path } => {
                Some(LauncherShellMessage::CopySupportBundle {
                    bundle_path: bundle_path.clone(),
                })
            }
            WidgetAction::Reveal { path, label } => Some(LauncherShellMessage::RevealPath {
                path: path.clone(),
                label: label.clone(),
            }),
            WidgetAction::UploadArtifacts => {
                if layout.support_artifacts.is_empty() {
                    None
                } else {
                    Some(LauncherShellMessage::UploadSupportArtifacts {
                        artifacts: layout.support_artifacts.clone(),
                    })
                }
            }
            WidgetAction::RestageProvider { role, index } => {
                Some(LauncherShellMessage::RestageProvider {
                    role: *role,
                    index: *index,
                })
            }
            WidgetAction::RetargetProvider { role, index } => {
                Some(LauncherShellMessage::RetargetProvider {
                    role: *role,
                    index: *index,
                })
            }
            WidgetAction::ClearProviderOverride { role, index } => {
                Some(LauncherShellMessage::ClearProviderOverride {
                    role: *role,
                    index: *index,
                })
            }
            WidgetAction::RetargetAllProviders => Some(LauncherShellMessage::RetargetAllProviders),
            WidgetAction::RestoreAllProviderDefaults => {
                Some(LauncherShellMessage::RestoreAllProviderDefaults)
            }
            WidgetAction::CopyReportPreview => Some(LauncherShellMessage::CopyReportPreview),
            WidgetAction::ExportReportPreview => Some(LauncherShellMessage::ExportReportPreview),
            WidgetAction::ScrollReportPreview { delta } => {
                Some(LauncherShellMessage::ScrollReportPreview { delta: *delta })
            }
        }
    }
}

fn build_gui(
    state: Option<&LauncherShellState>,
    feedback: Option<&ActionFeedback>,
    providers: &ProviderDiagnostics,
    report_scroll_offset: usize,
) -> GuiResult<(Gui, LauncherShellLayout)> {
    let mut gui = Gui::new();
    let mut layout = LauncherShellLayout::default();
    let root = gui.root();

    gui.add_label(root, "LegacyClonk Launcher Diagnostics");
    gui.add_label(
        root,
        "Inspect support bundles, updater telemetry, and captured logs before sharing them with support.",
    );

    if let Some(feedback) = feedback {
        let message = format_feedback_label(feedback);
        gui.add_label(root, message.clone());
        layout.feedback_message = Some(message);
    }

    match state {
        None => {
            gui.add_label(
                root,
                "Launch `lc-game` once to generate diagnostics and enable support bundle tooling.",
            );
            let regenerate_button = gui.add_button(root, "Regenerate");
            layout.regenerate_button = Some(regenerate_button);
            gui.set_button_enabled(regenerate_button, false)?;
        }
        Some(state) => {
            layout.support_artifacts = support_artifacts(state);

            let overview = gui.add_column(root, true);
            gui.add_label(
                overview,
                format!("Generated at {}", state.summary.generated_at),
            );
            gui.add_label(
                overview,
                format!("Logs directory: {}", display_path(&state.logs_dir)),
            );

            let summary_row = gui.add_row(overview, false);
            gui.add_label(
                summary_row,
                format!("Launcher summary: {}", display_path(&state.summary_path)),
            );
            let summary_button = gui.add_button(summary_row, "Reveal");
            layout.action_map.insert(
                summary_button,
                WidgetAction::Reveal {
                    path: state.summary_path.clone(),
                    label: "Launcher summary".into(),
                },
            );

            let launcher_row = gui.add_row(overview, false);
            gui.add_label(
                launcher_row,
                format!("Launcher log: {}", display_path(&state.launcher_log_path)),
            );
            let launcher_button = gui.add_button(launcher_row, "Reveal");
            layout.action_map.insert(
                launcher_button,
                WidgetAction::Reveal {
                    path: state.launcher_log_path.clone(),
                    label: "Launcher log".into(),
                },
            );

            let support_section = gui.add_column(root, true);
            gui.add_label(support_section, "Support Bundle");
            let bundle_status = match &state.support_bundle_path {
                Some(path) => format!("Bundle path: {}", display_path(path)),
                None => "Bundle path: Not generated yet.".to_string(),
            };
            gui.add_label(support_section, bundle_status);

            let button_row = gui.add_row(support_section, false);
            let regenerate_button = gui.add_button(button_row, "Regenerate");
            layout
                .action_map
                .insert(regenerate_button, WidgetAction::RegenerateBundle);
            layout.regenerate_button = Some(regenerate_button);

            let copy_button = gui.add_button(button_row, "Copy…");
            layout.copy_button = Some(copy_button);
            if let Some(bundle) = &state.support_bundle_path {
                layout.action_map.insert(
                    copy_button,
                    WidgetAction::CopyBundle {
                        bundle_path: bundle.clone(),
                    },
                );
            } else {
                gui.set_button_enabled(copy_button, false)?;
            }

            let reveal_bundle_button = gui.add_button(button_row, "Reveal");
            if let Some(bundle) = &state.support_bundle_path {
                layout.action_map.insert(
                    reveal_bundle_button,
                    WidgetAction::Reveal {
                        path: bundle.clone(),
                        label: "Support bundle".into(),
                    },
                );
            } else {
                gui.set_button_enabled(reveal_bundle_button, false)?;
            }

            let upload_button = gui.add_button(button_row, "Upload…");
            layout.upload_button = Some(upload_button);
            if layout.support_artifacts.is_empty() {
                gui.set_button_enabled(upload_button, false)?;
            } else {
                layout
                    .action_map
                    .insert(upload_button, WidgetAction::UploadArtifacts);
            }

            let report_section = gui.add_column(root, true);
            gui.add_label(report_section, "Support Bundle Report Preview");
            if state.support_bundle_report.is_empty() {
                gui.add_label(
                    report_section,
                    "Report preview will appear here after diagnostics are generated.",
                );
            } else {
                let controls = gui.add_row(report_section, false);
                let copy_button = gui.add_button(controls, "Copy report");
                layout.report_copy_button = Some(copy_button);
                layout
                    .action_map
                    .insert(copy_button, WidgetAction::CopyReportPreview);

                let export_button = gui.add_button(controls, "Save report…");
                layout.report_export_button = Some(export_button);
                layout
                    .action_map
                    .insert(export_button, WidgetAction::ExportReportPreview);

                let scroll_up = gui.add_button(controls, "Scroll up");
                layout.report_scroll_up_button = Some(scroll_up);
                layout.action_map.insert(
                    scroll_up,
                    WidgetAction::ScrollReportPreview {
                        delta: -(REPORT_PREVIEW_SCROLL_STEP as isize),
                    },
                );

                let scroll_down = gui.add_button(controls, "Scroll down");
                layout.report_scroll_down_button = Some(scroll_down);
                layout.action_map.insert(
                    scroll_down,
                    WidgetAction::ScrollReportPreview {
                        delta: REPORT_PREVIEW_SCROLL_STEP as isize,
                    },
                );

                let total_lines = state.support_bundle_report.len();
                let visible = REPORT_PREVIEW_VISIBLE_LINES.min(total_lines);
                let max_offset = total_lines.saturating_sub(visible);
                let offset = report_scroll_offset.min(max_offset);

                if offset == 0 {
                    gui.set_button_enabled(scroll_up, false)?;
                }
                if offset >= max_offset {
                    gui.set_button_enabled(scroll_down, false)?;
                }

                let range_label = format!(
                    "Showing lines {}-{} of {}",
                    offset + 1,
                    (offset + visible).min(total_lines),
                    total_lines
                );
                gui.add_label(controls, range_label.clone());
                layout.report_line_range = Some(range_label);

                let end = (offset + visible).min(total_lines);
                for line in &state.support_bundle_report[offset..end] {
                    gui.add_label(report_section, line.clone());
                }
                if end < total_lines {
                    let remaining = total_lines - end;
                    let suffix = if remaining == 1 { "" } else { "s" };
                    gui.add_label(
                        report_section,
                        format!(
                            "… {} additional line{} hidden. Use scroll controls to view remaining content.",
                            remaining, suffix
                        ),
                    );
                }
            }

            let artifacts_section = gui.add_column(root, true);
            gui.add_label(artifacts_section, "Artifacts");
            if layout.support_artifacts.is_empty() {
                gui.add_label(artifacts_section, "No artifacts are available yet.");
            } else {
                for artifact in &layout.support_artifacts {
                    let row = gui.add_row(artifacts_section, false);
                    gui.add_label(
                        row,
                        format!(
                            "{}: {}",
                            format_role(artifact.role),
                            display_path(&artifact.path)
                        ),
                    );
                    let button = gui.add_button(row, "Reveal");
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: artifact.path.clone(),
                            label: format!("{} path", format_role(artifact.role)),
                        },
                    );
                }
            }

            let runtime_section = gui.add_column(root, true);
            gui.add_label(runtime_section, "Runtime Logs");
            if state.runtime_log_paths.is_empty() {
                gui.add_label(runtime_section, "No runtime logs were captured.");
            } else {
                for path in &state.runtime_log_paths {
                    let row = gui.add_row(runtime_section, false);
                    gui.add_label(row, display_path(path));
                    let button = gui.add_button(row, "Reveal");
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: path.clone(),
                            label: "Runtime log".into(),
                        },
                    );
                }
            }

            let crash_section = gui.add_column(root, true);
            gui.add_label(crash_section, "Crash Reports");
            if state.crash_report_paths.is_empty() {
                gui.add_label(crash_section, "No crash reports were recorded.");
            } else {
                for path in &state.crash_report_paths {
                    let row = gui.add_row(crash_section, false);
                    gui.add_label(row, display_path(path));
                    let button = gui.add_button(row, "Reveal");
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: path.clone(),
                            label: "Crash report".into(),
                        },
                    );
                }
            }

            let telemetry_section = gui.add_column(root, true);
            gui.add_label(telemetry_section, "Updater Telemetry");
            if state.telemetry_success_logs.is_empty() && state.telemetry_failures.is_empty() {
                gui.add_label(
                    telemetry_section,
                    "No updater telemetry was detected in the captured logs.",
                );
            } else {
                for path in &state.telemetry_success_logs {
                    let row = gui.add_row(telemetry_section, false);
                    gui.add_label(row, format!("Success recorded in {}", display_path(path)));
                    let button = gui.add_button(row, "Reveal");
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: path.clone(),
                            label: "Telemetry success".into(),
                        },
                    );
                }
                for failure in &state.telemetry_failures {
                    let row = gui.add_row(telemetry_section, false);
                    gui.add_label(row, format!("Failure: {}", failure.message));
                    let button = gui.add_button(row, "Reveal log");
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: failure.log_path.clone(),
                            label: "Telemetry failure log".into(),
                        },
                    );
                }
            }
        }
    }

    add_provider_section(&mut gui, &mut layout, root, providers, state);

    Ok((gui, layout))
}

fn add_provider_section(
    gui: &mut Gui,
    layout: &mut LauncherShellLayout,
    root: WidgetId,
    providers: &ProviderDiagnostics,
    state: Option<&LauncherShellState>,
) {
    let section = gui.add_column(root, true);
    gui.add_label(section, "First-Party Providers");
    let (logs_dir, bulk_summary) = match state {
        Some(state) => (
            Some(state.logs_dir.as_path()),
            state.summary.provider_bulk_retarget.as_ref(),
        ),
        None => (None, None),
    };
    if providers.share.is_empty() && providers.upload.is_empty() {
        gui.add_label(
            section,
            "No first-party providers are configured. Configure LC_FIRST_PARTY_* variables to enable automated submissions.",
        );
        if let Some(summary) = bulk_summary {
            add_bulk_retarget_summary(gui, section, summary, logs_dir);
        }
        return;
    }

    let bulk_row = gui.add_row(section, false);
    gui.add_label(bulk_row, "Bulk actions:");
    let restore_button = gui.add_button(bulk_row, "Restore defaults");
    layout
        .action_map
        .insert(restore_button, WidgetAction::RestoreAllProviderDefaults);
    layout.restore_defaults_button = Some(restore_button);
    let retarget_button = gui.add_button(bulk_row, "Retarget all…");
    layout
        .action_map
        .insert(retarget_button, WidgetAction::RetargetAllProviders);
    layout.retarget_all_button = Some(retarget_button);

    if !providers.share.is_empty() {
        let share_section = gui.add_column(section, true);
        gui.add_label(share_section, "Share Targets");
        render_provider_list(
            gui,
            layout,
            share_section,
            &providers.share,
            ProviderKind::Share,
        );
    }

    if !providers.upload.is_empty() {
        let upload_section = gui.add_column(section, true);
        gui.add_label(upload_section, "Upload Targets");
        render_provider_list(
            gui,
            layout,
            upload_section,
            &providers.upload,
            ProviderKind::Upload,
        );
    }

    if let Some(summary) = bulk_summary {
        add_bulk_retarget_summary(gui, section, summary, logs_dir);
    }
}

fn render_provider_list(
    gui: &mut Gui,
    layout: &mut LauncherShellLayout,
    parent: WidgetId,
    providers: &[ProviderStatus],
    role: ProviderKind,
) {
    for (index, provider) in providers.iter().enumerate() {
        render_provider_entry(gui, layout, parent, provider, role, index);
    }
}

fn render_provider_entry(
    gui: &mut Gui,
    layout: &mut LauncherShellLayout,
    parent: WidgetId,
    provider: &ProviderStatus,
    role: ProviderKind,
    index: usize,
) {
    let entry = gui.add_column(parent, true);
    gui.add_label(
        entry,
        format!("{}: {}", provider.name, display_path(&provider.path)),
    );
    gui.add_label(
        entry,
        format!(
            "Path status: {}; Automation: {}",
            format_path_status(&provider.path_status),
            format_automation_state(&provider.automation)
        ),
    );
    gui.add_label(
        entry,
        format!(
            "Default path: {}",
            display_path(provider.path_provenance.default_path())
        ),
    );

    let overrides = provider.path_provenance.overrides();
    if overrides.is_empty() {
        gui.add_label(entry, "Override history: none recorded.");
    } else {
        gui.add_label(entry, "Override history:");
        for override_entry in overrides {
            gui.add_label(
                entry,
                format!(
                    "  - {} -> {}",
                    format_override_source(override_entry.source()),
                    display_path(override_entry.path())
                ),
            );
        }
    }

    let has_preference_override = provider.path_provenance.has_preference_override();
    let path_is_default = provider.path == provider.path_provenance.default_path();
    if has_preference_override || !path_is_default {
        let controls = gui.add_row(entry, false);
        gui.add_label(controls, "Override controls:");
        let label = if path_is_default {
            "Clear saved overrides"
        } else {
            "Restore default path"
        };
        let button = gui.add_button(controls, label);
        layout
            .action_map
            .insert(button, WidgetAction::ClearProviderOverride { role, index });
    }

    if matches!(provider.automation, ProviderAutomationState::Stale { .. }) {
        let actions = gui.add_row(entry, false);
        gui.add_label(actions, "Actions:");
        let restage = gui.add_button(actions, "Restage");
        layout
            .action_map
            .insert(restage, WidgetAction::RestageProvider { role, index });
        let retarget = gui.add_button(actions, "Retarget…");
        layout
            .action_map
            .insert(retarget, WidgetAction::RetargetProvider { role, index });
    }
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn format_feedback_label(feedback: &ActionFeedback) -> String {
    format!("[{}] {}", feedback.kind.label(), feedback.message)
}

fn format_role(role: &str) -> String {
    role.split_whitespace()
        .map(capitalize_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_path_status(status: &ProviderPathStatus) -> String {
    match status {
        ProviderPathStatus::Ready => "Ready".into(),
        ProviderPathStatus::Missing => "Missing".into(),
        ProviderPathStatus::NotDirectory => "Not a directory".into(),
        ProviderPathStatus::Inaccessible(message) => {
            format!("Inaccessible ({message})")
        }
    }
}

fn format_automation_state(state: &ProviderAutomationState) -> String {
    match state {
        ProviderAutomationState::Idle => "Idle".into(),
        ProviderAutomationState::Submitted { detail } => format!("Submitted ({detail})"),
        ProviderAutomationState::Stale { reason } => format!("Stale ({reason})"),
        ProviderAutomationState::Skipped { reason } => format!("Skipped ({reason})"),
        ProviderAutomationState::Failed { error } => format!("Failed ({error})"),
    }
}

fn format_override_source(source: &ProviderOverrideSource) -> String {
    match source {
        ProviderOverrideSource::Preference => "Launcher preference".into(),
        ProviderOverrideSource::Retargeted { applied_at } => {
            format!("Retargeted at {applied_at}")
        }
    }
}

fn capitalize_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => {
            let mut result = String::with_capacity(word.len());
            result.push(first.to_ascii_uppercase());
            result.push_str(chars.as_str());
            result
        }
        None => String::new(),
    }
}

fn add_bulk_retarget_summary(
    gui: &mut Gui,
    section: WidgetId,
    summary: &ProviderBulkRetargetSummary,
    logs_dir: Option<&Path>,
) {
    let has_records = !summary.share.is_empty() || !summary.upload.is_empty();
    if !has_records && summary.history_cleared_at.is_none() {
        return;
    }
    if has_records {
        gui.add_label(section, "Bulk retarget history:");
        add_bulk_retarget_records(gui, section, "  Share targets:", &summary.share, logs_dir);
        add_bulk_retarget_records(gui, section, "  Upload targets:", &summary.upload, logs_dir);
    }
    if let Some(cleared_at) = &summary.history_cleared_at {
        let message = if has_records {
            format!("Bulk retarget history last cleared at {cleared_at}.")
        } else {
            format!(
                "Bulk retarget history was cleared at {cleared_at}. No retarget records remain while providers use default staging paths."
            )
        };
        gui.add_label(section, message);
    }
}

fn add_bulk_retarget_records(
    gui: &mut Gui,
    section: WidgetId,
    label: &str,
    records: &[ProviderBulkRetargetRecord],
    logs_dir: Option<&Path>,
) {
    if records.is_empty() {
        return;
    }
    gui.add_label(section, label.to_string());
    for record in records {
        let base_path = resolve_logs_entry(logs_dir, &record.base_path);
        gui.add_label(
            section,
            format!(
                "    - {} (last retargeted at {}, changed {} of {} targets)",
                display_path(&base_path),
                record.retargeted_at,
                record.changed,
                record.total
            ),
        );
    }
}

fn resolve_logs_entry(logs_dir: Option<&Path>, entry: &str) -> PathBuf {
    let candidate = Path::new(entry);
    if candidate.is_absolute() || logs_dir.is_none() {
        candidate.to_path_buf()
    } else {
        logs_dir.unwrap().join(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_gui::{GuiEvent, Point};
    use lc_launcher::{
        LauncherSummary, LauncherTelemetryFailure, ProviderAutomationSnapshot,
        ProviderAutomationState, ProviderBulkRetargetRecord, ProviderBulkRetargetSummary,
        ProviderDiagnostics, ProviderOverrideSource, ProviderPathProvenance, ProviderPathStatus,
        ProviderStatus, SerializableTelemetryFailure, SerializableTelemetrySummary,
    };
    use tempfile::TempDir;

    #[test]
    fn buttons_are_disabled_before_summary_exists() {
        let mut ui = LauncherShellUi::new(None).expect("ui");
        ui.layout(Size::new(640.0, 480.0));

        let regenerate = ui.regenerate_button().expect("regenerate button");
        let rect = ui.widget_rect(regenerate).expect("regenerate rect");
        let pos = center(rect);

        let down = ui.handle_event(GuiEvent::PointerDown { position: pos });
        assert!(down.messages.is_empty());
        assert!(!down.gui.captured);

        let up = ui.handle_event(GuiEvent::PointerUp { position: pos });
        assert!(up.messages.is_empty());
        assert!(!up.gui.captured);
    }

    #[test]
    fn clicking_buttons_emits_expected_messages() {
        let temp = TempDir::new().unwrap();
        let state = sample_state(temp.path());
        let bundle_path = state.support_bundle_path.clone().expect("bundle");
        let summary_path = state.summary_path.clone();

        let mut ui = LauncherShellUi::new(Some(state.clone())).expect("ui");
        let share_path = temp.path().join("support-share");
        let mut diagnostics = ProviderDiagnostics::default();
        diagnostics.share.push(ProviderStatus {
            name: "Support Share Drop".into(),
            path: share_path.clone(),
            path_status: ProviderPathStatus::Ready,
            automation: ProviderAutomationState::Idle,
            path_provenance: ProviderPathProvenance::new(share_path.clone()),
        });
        ui.set_providers(diagnostics).expect("set providers");
        ui.layout(Size::new(960.0, 1280.0));

        let regenerate = ui.regenerate_button().expect("regenerate button");
        let regenerate_response = click_button(&mut ui, regenerate);
        assert!(matches!(
            regenerate_response.messages.as_slice(),
            [LauncherShellMessage::RegenerateSupportBundle]
        ));

        let restore_defaults = ui
            .restore_defaults_button()
            .expect("restore defaults button");
        let restore_response = click_button(&mut ui, restore_defaults);
        assert!(matches!(
            restore_response.messages.as_slice(),
            [LauncherShellMessage::RestoreAllProviderDefaults]
        ));

        let retarget_all = ui.retarget_all_button().expect("retarget all button");
        let retarget_response = click_button(&mut ui, retarget_all);
        assert!(matches!(
            retarget_response.messages.as_slice(),
            [LauncherShellMessage::RetargetAllProviders]
        ));

        let copy = ui.copy_button().expect("copy button");
        let copy_response = click_button(&mut ui, copy);
        assert!(matches!(
            copy_response.messages.as_slice(),
            [LauncherShellMessage::CopySupportBundle { bundle_path: path }]
                if path == &bundle_path
        ));

        let upload = ui.upload_button().expect("upload button");
        let upload_response = click_button(&mut ui, upload);
        assert!(matches!(
            upload_response.messages.as_slice(),
            [LauncherShellMessage::UploadSupportArtifacts { artifacts }]
                if artifacts.iter().any(|artifact| artifact.path == bundle_path)
        ));

        // Capturing telemetry and runtime logs through the support artifact upload flow keeps
        // the implementation honest. The upload payload should include both bundle and summary.
        assert!(upload_response.messages.iter().any(
            |msg| matches!(msg, LauncherShellMessage::UploadSupportArtifacts { artifacts }
                if artifacts.iter().any(|artifact| artifact.path == summary_path))
        ));

        let report_copy = ui.report_copy_button().expect("report copy button");
        let report_copy_response = click_button(&mut ui, report_copy);
        assert!(matches!(
            report_copy_response.messages.as_slice(),
            [LauncherShellMessage::CopyReportPreview]
        ));

        let report_export = ui.report_export_button().expect("report export button");
        let report_export_response = click_button(&mut ui, report_export);
        assert!(matches!(
            report_export_response.messages.as_slice(),
            [LauncherShellMessage::ExportReportPreview]
        ));

        if let Some(scroll_up) = ui.report_scroll_up_button() {
            let scroll_up_response = click_button(&mut ui, scroll_up);
            assert!(
                scroll_up_response.messages.is_empty(),
                "scroll up should be disabled when the report fits without scrolling"
            );
        }
    }

    #[test]
    fn report_preview_scrolling_updates_layout() {
        let temp = TempDir::new().unwrap();
        let mut state = sample_state(temp.path());
        state.support_bundle_report = (1..=50).map(|idx| format!("Line {idx:02}")).collect();
        let mut ui = LauncherShellUi::new(Some(state)).expect("ui");
        ui.layout(Size::new(960.0, 1280.0));

        let initial_range = ui.report_line_range_text().expect("initial range label");
        assert_eq!(
            initial_range,
            format!(
                "Showing lines 1-{} of 50",
                REPORT_PREVIEW_VISIBLE_LINES.min(50)
            )
        );

        let scroll_down = ui.report_scroll_down_button().expect("scroll down button");
        let scroll_down_response = click_button(&mut ui, scroll_down);
        let delta = match scroll_down_response.messages.as_slice() {
            [LauncherShellMessage::ScrollReportPreview { delta }] => *delta,
            other => panic!("unexpected scroll messages: {other:?}"),
        };
        ui.scroll_report_preview(delta).expect("scroll preview");
        ui.layout(Size::new(960.0, 1280.0));

        let after_range = ui
            .report_line_range_text()
            .expect("range label after scroll")
            .to_string();
        let expected_offset = delta.max(0) as usize;
        let expected_start = expected_offset + 1;
        let expected_end = (expected_offset + REPORT_PREVIEW_VISIBLE_LINES).min(50);
        assert_eq!(
            after_range,
            format!("Showing lines {}-{} of 50", expected_start, expected_end)
        );

        let commands = ui.render();
        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Line 13")
            )),
            "expected Line 13 to be visible after scrolling"
        );
        assert!(
            commands.iter().all(|command| !matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Line 01")
            )),
            "expected the first line to be hidden after scrolling"
        );

        let scroll_up = ui
            .report_scroll_up_button()
            .expect("scroll up button after scrolling");
        let scroll_up_response = click_button(&mut ui, scroll_up);
        assert!(matches!(
            scroll_up_response.messages.as_slice(),
            [LauncherShellMessage::ScrollReportPreview { delta }] if *delta < 0
        ));
    }

    fn click_button(ui: &mut LauncherShellUi, id: WidgetId) -> LauncherShellResponse {
        let rect = ui.widget_rect(id).expect("widget rect");
        let pos = center(rect);
        ui.handle_event(GuiEvent::PointerDown { position: pos });
        ui.handle_event(GuiEvent::PointerUp { position: pos })
    }

    fn center(rect: Rect) -> Point {
        Point::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        )
    }

    fn sample_state(base: &Path) -> LauncherShellState {
        let logs_dir = base.join("Logs");
        let summary_path = logs_dir.join("launcher-summary.json");
        let launcher_log_path = logs_dir.join("launcher/lc-launcher.log");
        let runtime_log_path = logs_dir.join("Clonk-session.log");
        let crash_report_path = logs_dir.join("LegacyClonk-crash.dmp");
        let bundle_path = logs_dir.join("support-bundle.zip");

        let summary = LauncherSummary {
            schema_version: 1,
            generated_at: "2024-05-01T12:00:00Z".into(),
            launcher_log: "launcher/lc-launcher.log".into(),
            runtime_logs: vec!["Clonk-session.log".into()],
            crash_reports: vec!["LegacyClonk-crash.dmp".into()],
            support_bundle: Some("support-bundle.zip".into()),
            update_telemetry: SerializableTelemetrySummary {
                successes: vec!["Clonk-session.log".into()],
                failures: vec![SerializableTelemetryFailure {
                    log: "Clonk-session.log".into(),
                    message: "c4group returned status 1".into(),
                }],
            },
            provider_automation: ProviderAutomationSnapshot::default(),
            provider_bulk_retarget: None,
        };

        LauncherShellState {
            summary_path,
            logs_dir,
            summary,
            launcher_log_path,
            runtime_log_paths: vec![runtime_log_path.clone()],
            crash_report_paths: vec![crash_report_path],
            support_bundle_path: Some(bundle_path),
            telemetry_success_logs: vec![runtime_log_path.clone()],
            telemetry_failures: vec![LauncherTelemetryFailure {
                log_path: runtime_log_path,
                message: "c4group returned status 1".into(),
            }],
            support_bundle_report: vec![
                "Launcher summary written to Logs/launcher-summary.json".into(),
                "Support bundle available at Logs/support-bundle.zip".into(),
                "Share the support bundle when filing bugs to include launcher, runtime, and telemetry logs."
                    .into(),
            ],
        }
    }

    #[test]
    fn action_feedback_survives_state_refresh() {
        let temp = TempDir::new().unwrap();
        let state = sample_state(temp.path());
        let mut ui = LauncherShellUi::new(Some(state.clone())).expect("ui");

        ui.set_action_feedback(Some(ActionFeedback::success("Copied bundle")))
            .expect("set feedback");
        assert!(
            ui.layout
                .feedback_message
                .as_deref()
                .map(|text| text.contains("Copied bundle"))
                .unwrap_or(false),
            "expected feedback message to include update text"
        );

        ui.set_state(Some(state)).expect("state refresh");
        assert!(
            ui.layout
                .feedback_message
                .as_deref()
                .map(|text| text.contains("Copied bundle"))
                .unwrap_or(false),
            "feedback message should survive state rebuild"
        );
    }

    #[test]
    fn provider_diagnostics_are_rendered() {
        let temp = TempDir::new().unwrap();
        let mut diagnostics = ProviderDiagnostics::default();
        diagnostics.share.push(ProviderStatus {
            name: "Support Share Drop".into(),
            path: temp.path().join("support-share"),
            path_status: ProviderPathStatus::Ready,
            automation: ProviderAutomationState::Submitted {
                detail: "submission-request-1.json".into(),
            },
            path_provenance: ProviderPathProvenance::new(temp.path().join("support-share")),
        });

        let mut ui = LauncherShellUi::new(None).expect("ui");
        ui.set_providers(diagnostics).expect("set providers");
        ui.layout(Size::new(640.0, 480.0));
        let commands = ui.render();

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Support Share Drop")
            )),
            "expected provider name to be rendered"
        );

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Submitted (submission-request-1.json)")
            )),
            "expected automation status to be rendered"
        );

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Default path:")
            )),
            "default path should be rendered"
        );

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Override history: none recorded.")
            )),
            "override history placeholder should be rendered"
        );
    }

    #[test]
    fn override_entries_are_listed() {
        let temp = TempDir::new().unwrap();
        let mut provenance = ProviderPathProvenance::new(temp.path().join("support-share-default"));
        provenance.apply_override(
            temp.path().join("support-share-retarget"),
            ProviderOverrideSource::Retargeted {
                applied_at: "2024-06-01T12:00:00Z".into(),
            },
        );

        let mut diagnostics = ProviderDiagnostics::default();
        diagnostics.share.push(ProviderStatus {
            name: "Support Share Drop".into(),
            path: temp.path().join("support-share-retarget"),
            path_status: ProviderPathStatus::Ready,
            automation: ProviderAutomationState::Idle,
            path_provenance: provenance,
        });

        let mut ui = LauncherShellUi::new(None).expect("ui");
        ui.set_providers(diagnostics).expect("set providers");
        ui.layout(Size::new(640.0, 480.0));
        let commands = ui.render();

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Retargeted at 2024-06-01T12:00:00Z")
            )),
            "override timestamp should be rendered"
        );

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("support-share-retarget")
            )),
            "override path should be rendered"
        );
    }

    #[test]
    fn bulk_retarget_history_is_rendered() {
        let temp = TempDir::new().unwrap();
        let mut state = sample_state(temp.path());
        let mut summary = ProviderBulkRetargetSummary::default();
        summary.share.push(ProviderBulkRetargetRecord {
            base_path: "support-share".into(),
            retargeted_at: "2024-06-02T15:00:00Z".into(),
            total: 3,
            changed: 2,
        });
        summary.upload.push(ProviderBulkRetargetRecord {
            base_path: "support-upload".into(),
            retargeted_at: "2024-06-02T16:00:00Z".into(),
            total: 2,
            changed: 1,
        });
        state.summary.provider_bulk_retarget = Some(summary);

        let mut ui = LauncherShellUi::new(Some(state)).expect("ui");
        ui.layout(Size::new(640.0, 480.0));
        let commands = ui.render();

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Bulk retarget history:")
            )),
            "expected bulk retarget heading to be rendered"
        );

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("support-share")
            )),
            "expected share base directory to be rendered"
        );

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("support-upload")
            )),
            "expected upload base directory to be rendered"
        );
    }

    #[test]
    fn bulk_retarget_history_cleared_annotation_is_rendered() {
        let temp = TempDir::new().unwrap();
        let mut state = sample_state(temp.path());
        let mut summary = ProviderBulkRetargetSummary::default();
        summary.history_cleared_at = Some("2024-06-05T18:30:00Z".into());
        state.summary.provider_bulk_retarget = Some(summary);

        let mut ui = LauncherShellUi::new(Some(state)).expect("ui");
        ui.layout(Size::new(640.0, 480.0));
        let commands = ui.render();

        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, .. } if text.contains("Bulk retarget history was cleared at 2024-06-05T18:30:00Z")
            )),
            "history cleared annotation should be rendered"
        );
    }

    #[test]
    fn stale_providers_offer_actions() {
        let mut diagnostics = ProviderDiagnostics::default();
        diagnostics.share.push(ProviderStatus {
            name: "Support Share Drop".into(),
            path: PathBuf::from("/tmp/support-share"),
            path_status: ProviderPathStatus::Missing,
            automation: ProviderAutomationState::Stale {
                reason: "missing directory".into(),
            },
            path_provenance: ProviderPathProvenance::new(PathBuf::from("/tmp/support-share")),
        });

        let mut ui = LauncherShellUi::new(None).expect("ui");
        ui.set_providers(diagnostics).expect("set providers");
        ui.layout(Size::new(640.0, 480.0));

        let restage_present = ui.layout.action_map.values().any(|action| {
            matches!(
                action,
                WidgetAction::RestageProvider {
                    role: ProviderKind::Share,
                    index: 0
                }
            )
        });
        assert!(restage_present, "expected restage action to be registered");

        let retarget_present = ui.layout.action_map.values().any(|action| {
            matches!(
                action,
                WidgetAction::RetargetProvider {
                    role: ProviderKind::Share,
                    index: 0
                }
            )
        });
        assert!(
            retarget_present,
            "expected retarget action to be registered"
        );
    }

    #[test]
    fn overridden_providers_offer_restore_controls() {
        let temp = TempDir::new().unwrap();
        let default_path = temp.path().join("support-share-default");
        let override_path = temp.path().join("support-share-override");
        let mut provenance = ProviderPathProvenance::new(default_path.clone());
        provenance.apply_override(override_path.clone(), ProviderOverrideSource::Preference);

        let mut diagnostics = ProviderDiagnostics::default();
        diagnostics.share.push(ProviderStatus {
            name: "Support Share Drop".into(),
            path: override_path,
            path_status: ProviderPathStatus::Ready,
            automation: ProviderAutomationState::Idle,
            path_provenance: provenance,
        });

        let mut ui = LauncherShellUi::new(None).expect("ui");
        ui.set_providers(diagnostics).expect("set providers");
        ui.layout(Size::new(640.0, 480.0));

        let button = {
            let (button, _) = ui
                .layout
                .action_map
                .iter()
                .find(|(_, action)| {
                    matches!(
                        action,
                        WidgetAction::ClearProviderOverride {
                            role: ProviderKind::Share,
                            index: 0
                        }
                    )
                })
                .expect("restore override control");
            *button
        };

        let response = click_button(&mut ui, button);
        assert!(matches!(
            response.messages.as_slice(),
            [LauncherShellMessage::ClearProviderOverride {
                role: ProviderKind::Share,
                index: 0
            }]
        ));
    }
}
