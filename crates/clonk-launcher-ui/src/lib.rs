use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clonk_graphics::{BitmapFont, Color, TextFont};
use clonk_gui::{
    DrawCommand, Gui, GuiAction, GuiEvent, GuiEventResult, GuiResult, Rect, Size, WidgetId,
};
use clonk_launcher::{
    support_artifacts, LauncherShellState, Localization, ProviderAutomationState,
    ProviderBulkRetargetRecord, ProviderBulkRetargetSummary, ProviderDiagnostics,
    ProviderOverrideSource, ProviderPathStatus, ProviderStatus, SupportArtifact,
};
use std::sync::Arc;

const REPORT_PREVIEW_VISIBLE_LINES: usize = 28;
const REPORT_PREVIEW_SCROLL_STEP: usize = 12;

fn default_font() -> Arc<dyn TextFont> {
    Arc::new(BitmapFont::new())
}

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
    FocusReportSearch,
    ClearReportSearch,
    NextReportSearchMatch,
    PreviousReportSearchMatch,
    SetReportSearchPreset { preset: ReportSearchPreset },
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
    fn label(self, localization: &Localization) -> &str {
        match self {
            ActionFeedbackKind::Info => localization.text("IDS_LAUNCHER_UI_FEEDBACK_INFO"),
            ActionFeedbackKind::Success => localization.text("IDS_LAUNCHER_UI_FEEDBACK_SUCCESS"),
            ActionFeedbackKind::Error => localization.text("IDS_LAUNCHER_UI_FEEDBACK_ERROR"),
        }
    }

    pub fn english_label(self) -> &'static str {
        match self {
            ActionFeedbackKind::Info => "Info",
            ActionFeedbackKind::Success => "Success",
            ActionFeedbackKind::Error => "Error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportSearchHighlight {
    Generic,
    Error,
    Warning,
}

impl ReportSearchHighlight {
    pub fn active_color(self) -> Color {
        match self {
            ReportSearchHighlight::Generic => Color::opaque(200, 232, 255),
            ReportSearchHighlight::Error => Color::opaque(255, 152, 152),
            ReportSearchHighlight::Warning => Color::opaque(255, 220, 160),
        }
    }

    pub fn inactive_color(self) -> Color {
        match self {
            ReportSearchHighlight::Generic => Color::opaque(132, 188, 240),
            ReportSearchHighlight::Error => Color::opaque(220, 92, 92),
            ReportSearchHighlight::Warning => Color::opaque(236, 194, 104),
        }
    }

    pub fn label(self, localization: &Localization) -> &str {
        match self {
            ReportSearchHighlight::Generic => {
                localization.text("IDS_LAUNCHER_UI_SEARCH_HIGHLIGHT_GENERIC")
            }
            ReportSearchHighlight::Error => {
                localization.text("IDS_LAUNCHER_UI_SEARCH_HIGHLIGHT_ERRORS")
            }
            ReportSearchHighlight::Warning => {
                localization.text("IDS_LAUNCHER_UI_SEARCH_HIGHLIGHT_WARNINGS")
            }
        }
    }

    pub fn english_label(self) -> &'static str {
        match self {
            ReportSearchHighlight::Generic => "text",
            ReportSearchHighlight::Error => "errors",
            ReportSearchHighlight::Warning => "warnings",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportSearchPreset {
    Errors,
    Warnings,
}

#[derive(Debug, Clone)]
pub struct ReportSearchState {
    pub query: String,
    pub matches: Vec<usize>,
    pub active_index: Option<usize>,
    pub highlight: ReportSearchHighlight,
    pub editing: bool,
}

impl ReportSearchState {
    pub fn active_line(&self) -> Option<usize> {
        self.active_index
            .and_then(|index| self.matches.get(index).copied())
    }

    pub fn is_match(&self, line_index: usize) -> bool {
        self.matches.binary_search(&line_index).is_ok()
    }

    pub fn match_summary(&self) -> Option<(usize, usize)> {
        let total = self.matches.len();
        if total == 0 {
            None
        } else {
            let current = self.active_index.map(|index| index + 1).unwrap_or(1);
            Some((current, total))
        }
    }
}

pub struct LauncherShellUi {
    gui: Gui,
    layout: LauncherShellLayout,
    state: Option<LauncherShellState>,
    localization: Localization,
    feedback: Option<ActionFeedback>,
    providers: ProviderDiagnostics,
    report_scroll_offset: usize,
    report_search: Option<ReportSearchState>,
}

impl LauncherShellUi {
    pub fn new(state: Option<LauncherShellState>, localization: Localization) -> GuiResult<Self> {
        let mut ui = Self {
            gui: Gui::new(default_font()),
            layout: LauncherShellLayout::default(),
            state: None,
            localization,
            feedback: None,
            providers: ProviderDiagnostics::default(),
            report_scroll_offset: 0,
            report_search: None,
        };
        ui.set_state(state)?;
        Ok(ui)
    }

    pub fn set_state(&mut self, state: Option<LauncherShellState>) -> GuiResult<()> {
        self.state = state;
        self.report_scroll_offset = 0;
        if self.state.is_none() {
            self.report_search = None;
        }
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
        self.layout.report_line_range.as_deref()
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

    pub fn set_report_search(&mut self, report_search: Option<ReportSearchState>) -> GuiResult<()> {
        self.report_search = report_search;
        self.rebuild()
    }

    pub fn ensure_report_line_visible(&mut self, line_index: usize) -> GuiResult<()> {
        let total_lines = self
            .state
            .as_ref()
            .map(|state| state.support_bundle_report.len())
            .unwrap_or(0);
        if total_lines == 0 || line_index >= total_lines {
            return Ok(());
        }
        let visible = REPORT_PREVIEW_VISIBLE_LINES.min(total_lines);
        if total_lines <= visible {
            self.report_scroll_offset = 0;
            return self.rebuild();
        }
        let max_offset = total_lines.saturating_sub(visible);
        let current = self.report_scroll_offset.min(max_offset);
        if line_index < current {
            self.report_scroll_offset = line_index;
        } else if line_index >= current + visible {
            let new_offset = line_index + 1 - visible;
            self.report_scroll_offset = new_offset.min(max_offset);
        }
        self.rebuild()
    }

    pub fn report_search_focus_button(&self) -> Option<WidgetId> {
        self.layout.report_search_focus_button
    }

    pub fn report_search_clear_button(&self) -> Option<WidgetId> {
        self.layout.report_search_clear_button
    }

    pub fn report_search_next_button(&self) -> Option<WidgetId> {
        self.layout.report_search_next_button
    }

    pub fn report_search_previous_button(&self) -> Option<WidgetId> {
        self.layout.report_search_previous_button
    }

    pub fn report_search_error_button(&self) -> Option<WidgetId> {
        self.layout.report_search_error_button
    }

    pub fn report_search_warning_button(&self) -> Option<WidgetId> {
        self.layout.report_search_warning_button
    }

    pub fn report_search_status_text(&self) -> Option<&str> {
        self.layout.report_search_status.as_deref()
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
            self.report_search.as_ref(),
            &self.localization,
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
    report_search_focus_button: Option<WidgetId>,
    report_search_clear_button: Option<WidgetId>,
    report_search_next_button: Option<WidgetId>,
    report_search_previous_button: Option<WidgetId>,
    report_search_error_button: Option<WidgetId>,
    report_search_warning_button: Option<WidgetId>,
    report_search_status: Option<String>,
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
    FocusReportSearch,
    ClearReportSearch,
    NextReportSearchMatch,
    PreviousReportSearchMatch,
    SetReportSearchPreset { preset: ReportSearchPreset },
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
            WidgetAction::FocusReportSearch => Some(LauncherShellMessage::FocusReportSearch),
            WidgetAction::ClearReportSearch => Some(LauncherShellMessage::ClearReportSearch),
            WidgetAction::NextReportSearchMatch => {
                Some(LauncherShellMessage::NextReportSearchMatch)
            }
            WidgetAction::PreviousReportSearchMatch => {
                Some(LauncherShellMessage::PreviousReportSearchMatch)
            }
            WidgetAction::SetReportSearchPreset { preset } => {
                Some(LauncherShellMessage::SetReportSearchPreset { preset: *preset })
            }
        }
    }
}

fn build_gui(
    state: Option<&LauncherShellState>,
    feedback: Option<&ActionFeedback>,
    providers: &ProviderDiagnostics,
    report_scroll_offset: usize,
    report_search: Option<&ReportSearchState>,
    localization: &Localization,
) -> GuiResult<(Gui, LauncherShellLayout)> {
    let mut gui = Gui::new(default_font());
    let mut layout = LauncherShellLayout::default();
    let root = gui.root();

    gui.add_label(root, localization.text("IDS_LAUNCHER_UI_TITLE"));
    gui.add_label(root, localization.text("IDS_LAUNCHER_UI_DESCRIPTION"));

    if let Some(feedback) = feedback {
        let message = format_feedback_label(feedback, localization);
        gui.add_label(root, message.clone());
        layout.feedback_message = Some(message);
    }

    match state {
        None => {
            gui.add_label(
                root,
                localization.text("IDS_LAUNCHER_UI_PROMPT_LAUNCH_GAME"),
            );
            let regenerate_button =
                gui.add_button(root, localization.text("IDS_LAUNCHER_UI_BUTTON_REGENERATE"));
            layout.regenerate_button = Some(regenerate_button);
            gui.set_button_enabled(regenerate_button, false)?;
        }
        Some(state) => {
            let reveal_button_label = localization.text("IDS_LAUNCHER_UI_BUTTON_REVEAL");
            let regenerate_button_label = localization.text("IDS_LAUNCHER_UI_BUTTON_REGENERATE");
            let copy_button_label = localization.text("IDS_LAUNCHER_UI_BUTTON_COPY");
            let upload_button_label = localization.text("IDS_LAUNCHER_UI_BUTTON_UPLOAD");
            layout.support_artifacts = support_artifacts(state);

            let overview = gui.add_column(root, true);
            gui.add_label(
                overview,
                localization.format(
                    "IDS_LAUNCHER_UI_GENERATED_AT",
                    [("timestamp", state.summary.generated_at.as_str())],
                ),
            );
            let logs_dir_display = display_path(&state.logs_dir);
            gui.add_label(
                overview,
                localization.format(
                    "IDS_LAUNCHER_UI_LOGS_DIRECTORY",
                    [("path", logs_dir_display.as_str())],
                ),
            );

            let summary_row = gui.add_row(overview, false);
            let summary_path_display = display_path(&state.summary_path);
            gui.add_label(
                summary_row,
                localization.format(
                    "IDS_LAUNCHER_UI_LAUNCHER_SUMMARY_PATH",
                    [("path", summary_path_display.as_str())],
                ),
            );
            let summary_button = gui.add_button(summary_row, reveal_button_label);
            layout.action_map.insert(
                summary_button,
                WidgetAction::Reveal {
                    path: state.summary_path.clone(),
                    label: localization
                        .text("IDS_LAUNCHER_UI_LAUNCHER_SUMMARY_LABEL")
                        .into(),
                },
            );

            let launcher_row = gui.add_row(overview, false);
            let launcher_log_display = display_path(&state.launcher_log_path);
            gui.add_label(
                launcher_row,
                localization.format(
                    "IDS_LAUNCHER_UI_LAUNCHER_LOG_PATH",
                    [("path", launcher_log_display.as_str())],
                ),
            );
            let launcher_button = gui.add_button(launcher_row, reveal_button_label);
            layout.action_map.insert(
                launcher_button,
                WidgetAction::Reveal {
                    path: state.launcher_log_path.clone(),
                    label: localization
                        .text("IDS_LAUNCHER_UI_LAUNCHER_LOG_LABEL")
                        .into(),
                },
            );

            let support_section = gui.add_column(root, true);
            gui.add_label(
                support_section,
                localization.text("IDS_LAUNCHER_UI_SECTION_SUPPORT_BUNDLE"),
            );
            let bundle_status = match &state.support_bundle_path {
                Some(path) => {
                    let bundle_path = display_path(path);
                    localization.format(
                        "IDS_LAUNCHER_UI_BUNDLE_PATH",
                        [("path", bundle_path.as_str())],
                    )
                }
                None => localization
                    .text("IDS_LAUNCHER_UI_BUNDLE_NOT_GENERATED")
                    .to_string(),
            };
            gui.add_label(support_section, bundle_status);

            let button_row = gui.add_row(support_section, false);
            let regenerate_button = gui.add_button(button_row, regenerate_button_label);
            layout
                .action_map
                .insert(regenerate_button, WidgetAction::RegenerateBundle);
            layout.regenerate_button = Some(regenerate_button);

            let copy_button = gui.add_button(button_row, copy_button_label);
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

            let reveal_bundle_button = gui.add_button(button_row, reveal_button_label);
            if let Some(bundle) = &state.support_bundle_path {
                layout.action_map.insert(
                    reveal_bundle_button,
                    WidgetAction::Reveal {
                        path: bundle.clone(),
                        label: localization
                            .text("IDS_LAUNCHER_UI_SUPPORT_BUNDLE_LABEL")
                            .into(),
                    },
                );
            } else {
                gui.set_button_enabled(reveal_bundle_button, false)?;
            }

            let upload_button = gui.add_button(button_row, upload_button_label);
            layout.upload_button = Some(upload_button);
            if layout.support_artifacts.is_empty() {
                gui.set_button_enabled(upload_button, false)?;
            } else {
                layout
                    .action_map
                    .insert(upload_button, WidgetAction::UploadArtifacts);
            }

            let report_section = gui.add_column(root, true);
            gui.add_label(
                report_section,
                localization.text("IDS_LAUNCHER_UI_SECTION_REPORT_PREVIEW"),
            );
            if state.support_bundle_report.is_empty() {
                gui.add_label(
                    report_section,
                    localization.text("IDS_LAUNCHER_UI_REPORT_EMPTY"),
                );
            } else {
                let controls = gui.add_row(report_section, false);
                let copy_button = gui.add_button(
                    controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_COPY_REPORT"),
                );
                layout.report_copy_button = Some(copy_button);
                layout
                    .action_map
                    .insert(copy_button, WidgetAction::CopyReportPreview);

                let save_button = gui.add_button(
                    controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_SAVE_REPORT"),
                );
                layout.report_export_button = Some(save_button);
                layout
                    .action_map
                    .insert(save_button, WidgetAction::ExportReportPreview);

                let scroll_up = gui.add_button(
                    controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_SCROLL_UP"),
                );
                layout.report_scroll_up_button = Some(scroll_up);
                layout.action_map.insert(
                    scroll_up,
                    WidgetAction::ScrollReportPreview {
                        delta: -(REPORT_PREVIEW_SCROLL_STEP as isize),
                    },
                );

                let scroll_down = gui.add_button(
                    controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_SCROLL_DOWN"),
                );
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

                let range_start = (offset + 1).to_string();
                let range_end = ((offset + visible).min(total_lines)).to_string();
                let total_label = total_lines.to_string();
                let range_label = localization.format(
                    "IDS_LAUNCHER_UI_REPORT_RANGE",
                    [
                        ("start", range_start.as_str()),
                        ("end", range_end.as_str()),
                        ("total", total_label.as_str()),
                    ],
                );
                gui.add_label(controls, range_label.clone());
                layout.report_line_range = Some(range_label);

                let search_controls = gui.add_row(report_section, false);
                let search_active = report_search.map(|search| search.editing).unwrap_or(false);
                let focus_label_key = if search_active {
                    "IDS_LAUNCHER_UI_BUTTON_SEARCH_TYPING"
                } else {
                    "IDS_LAUNCHER_UI_BUTTON_SEARCH"
                };
                let focus_button =
                    gui.add_button(search_controls, localization.text(focus_label_key));
                layout.report_search_focus_button = Some(focus_button);
                layout
                    .action_map
                    .insert(focus_button, WidgetAction::FocusReportSearch);

                let clear_button = gui.add_button(
                    search_controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_CLEAR_SEARCH"),
                );
                layout.report_search_clear_button = Some(clear_button);
                layout
                    .action_map
                    .insert(clear_button, WidgetAction::ClearReportSearch);

                let error_button = gui.add_button(
                    search_controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_FIND_ERRORS"),
                );
                layout.report_search_error_button = Some(error_button);
                layout.action_map.insert(
                    error_button,
                    WidgetAction::SetReportSearchPreset {
                        preset: ReportSearchPreset::Errors,
                    },
                );

                let warning_button = gui.add_button(
                    search_controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_FIND_WARNINGS"),
                );
                layout.report_search_warning_button = Some(warning_button);
                layout.action_map.insert(
                    warning_button,
                    WidgetAction::SetReportSearchPreset {
                        preset: ReportSearchPreset::Warnings,
                    },
                );

                let previous_button = gui.add_button(
                    search_controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_PREVIOUS_MATCH"),
                );
                layout.report_search_previous_button = Some(previous_button);
                layout
                    .action_map
                    .insert(previous_button, WidgetAction::PreviousReportSearchMatch);

                let next_button = gui.add_button(
                    search_controls,
                    localization.text("IDS_LAUNCHER_UI_BUTTON_NEXT_MATCH"),
                );
                layout.report_search_next_button = Some(next_button);
                layout
                    .action_map
                    .insert(next_button, WidgetAction::NextReportSearchMatch);

                let mut search_status = localization
                    .text("IDS_LAUNCHER_UI_SEARCH_INACTIVE")
                    .to_string();
                let mut has_query = false;
                let mut match_count = 0usize;
                let mut editing = false;
                if let Some(search) = report_search {
                    has_query = !search.query.is_empty();
                    editing = search.editing;
                    match_count = search.matches.len();
                    if has_query {
                        let highlight_label = search.highlight.label(localization);
                        if match_count == 0 {
                            search_status = localization.format(
                                "IDS_LAUNCHER_UI_SEARCH_NO_MATCHES",
                                [
                                    ("highlight", highlight_label),
                                    ("query", search.query.as_str()),
                                ],
                            );
                        } else if let Some((current, total)) = search.match_summary() {
                            let current_str = current.to_string();
                            let total_str = total.to_string();
                            let replacements = [
                                ("highlight", highlight_label),
                                ("query", search.query.as_str()),
                                ("current", current_str.as_str()),
                                ("total", total_str.as_str()),
                            ];
                            search_status = if editing {
                                localization.format(
                                    "IDS_LAUNCHER_UI_SEARCH_MATCH_STATUS_EDITING",
                                    replacements,
                                )
                            } else {
                                localization
                                    .format("IDS_LAUNCHER_UI_SEARCH_MATCH_STATUS", replacements)
                            };
                        }
                    } else if editing {
                        search_status = localization
                            .text("IDS_LAUNCHER_UI_SEARCH_ACTIVE_EDITING")
                            .to_string();
                    }
                }

                if !has_query && !editing {
                    gui.set_button_enabled(clear_button, false)?;
                    gui.set_button_enabled(previous_button, false)?;
                    gui.set_button_enabled(next_button, false)?;
                } else if !has_query || match_count == 0 {
                    gui.set_button_enabled(previous_button, false)?;
                    gui.set_button_enabled(next_button, false)?;
                }

                gui.add_label(search_controls, search_status.clone());
                layout.report_search_status = Some(search_status);

                let end = (offset + visible).min(total_lines);
                for (relative_index, line) in
                    state.support_bundle_report[offset..end].iter().enumerate()
                {
                    let absolute_index = offset + relative_index;
                    let label = gui.add_label(report_section, line.clone());
                    if let Some(search) = report_search {
                        if let Some(active_line) = search.active_line() {
                            if active_line == absolute_index {
                                gui.set_label_color(label, search.highlight.active_color())?;
                                continue;
                            }
                        }
                        if search.is_match(absolute_index) {
                            gui.set_label_color(label, search.highlight.inactive_color())?;
                        }
                    }
                }
                if end < total_lines {
                    let remaining = total_lines - end;
                    let remaining_str = remaining.to_string();
                    let key = if remaining == 1 {
                        "IDS_LAUNCHER_UI_REPORT_TRUNCATED_ONE"
                    } else {
                        "IDS_LAUNCHER_UI_REPORT_TRUNCATED_MANY"
                    };
                    gui.add_label(
                        report_section,
                        localization.format(key, [("count", remaining_str.as_str())]),
                    );
                }
            }

            let artifacts_section = gui.add_column(root, true);
            gui.add_label(
                artifacts_section,
                localization.text("IDS_LAUNCHER_UI_SECTION_ARTIFACTS"),
            );
            if layout.support_artifacts.is_empty() {
                gui.add_label(
                    artifacts_section,
                    localization.text("IDS_LAUNCHER_UI_ARTIFACTS_EMPTY"),
                );
            } else {
                for artifact in &layout.support_artifacts {
                    let row = gui.add_row(artifacts_section, false);
                    let role_label = localized_artifact_role(localization, artifact.role);
                    let artifact_path = display_path(&artifact.path);
                    let role_text = role_label.as_str();
                    let path_text = artifact_path.as_str();
                    gui.add_label(
                        row,
                        localization.format(
                            "IDS_LAUNCHER_UI_ARTIFACT_ENTRY",
                            [("label", role_text), ("path", path_text)],
                        ),
                    );
                    let button = gui.add_button(row, reveal_button_label);
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: artifact.path.clone(),
                            label: localization.format(
                                "IDS_LAUNCHER_UI_ARTIFACT_PATH_LABEL",
                                [("label", role_text)],
                            ),
                        },
                    );
                }
            }

            let runtime_section = gui.add_column(root, true);
            gui.add_label(
                runtime_section,
                localization.text("IDS_LAUNCHER_UI_SECTION_RUNTIME_LOGS"),
            );
            if state.runtime_log_paths.is_empty() {
                gui.add_label(
                    runtime_section,
                    localization.text("IDS_LAUNCHER_UI_RUNTIME_LOGS_EMPTY"),
                );
            } else {
                for path in &state.runtime_log_paths {
                    let row = gui.add_row(runtime_section, false);
                    gui.add_label(row, display_path(path));
                    let button = gui.add_button(row, reveal_button_label);
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: path.clone(),
                            label: localization
                                .text("IDS_LAUNCHER_UI_RUNTIME_LOG_LABEL")
                                .into(),
                        },
                    );
                }
            }

            let crash_section = gui.add_column(root, true);
            gui.add_label(
                crash_section,
                localization.text("IDS_LAUNCHER_UI_SECTION_CRASH_REPORTS"),
            );
            if state.crash_report_paths.is_empty() {
                gui.add_label(
                    crash_section,
                    localization.text("IDS_LAUNCHER_UI_CRASH_REPORTS_EMPTY"),
                );
            } else {
                for path in &state.crash_report_paths {
                    let row = gui.add_row(crash_section, false);
                    gui.add_label(row, display_path(path));
                    let button = gui.add_button(row, reveal_button_label);
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: path.clone(),
                            label: localization
                                .text("IDS_LAUNCHER_UI_CRASH_REPORT_LABEL")
                                .into(),
                        },
                    );
                }
            }

            let telemetry_section = gui.add_column(root, true);
            gui.add_label(
                telemetry_section,
                localization.text("IDS_LAUNCHER_UI_SECTION_TELEMETRY"),
            );
            if state.telemetry_success_logs.is_empty() && state.telemetry_failures.is_empty() {
                gui.add_label(
                    telemetry_section,
                    localization.text("IDS_LAUNCHER_UI_TELEMETRY_EMPTY"),
                );
            } else {
                for path in &state.telemetry_success_logs {
                    let row = gui.add_row(telemetry_section, false);
                    let success_path = display_path(path);
                    gui.add_label(
                        row,
                        localization.format(
                            "IDS_LAUNCHER_UI_TELEMETRY_SUCCESS_ENTRY",
                            [("path", success_path.as_str())],
                        ),
                    );
                    let button = gui.add_button(row, reveal_button_label);
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: path.clone(),
                            label: localization
                                .text("IDS_LAUNCHER_UI_TELEMETRY_SUCCESS_LABEL")
                                .into(),
                        },
                    );
                }
                for failure in &state.telemetry_failures {
                    let row = gui.add_row(telemetry_section, false);
                    gui.add_label(
                        row,
                        localization.format(
                            "IDS_LAUNCHER_UI_TELEMETRY_FAILURE_ENTRY",
                            [("message", failure.message.as_str())],
                        ),
                    );
                    let button =
                        gui.add_button(row, localization.text("IDS_LAUNCHER_UI_BUTTON_REVEAL_LOG"));
                    layout.action_map.insert(
                        button,
                        WidgetAction::Reveal {
                            path: failure.log_path.clone(),
                            label: localization
                                .text("IDS_LAUNCHER_UI_TELEMETRY_FAILURE_LABEL")
                                .into(),
                        },
                    );
                }
            }
        }
    }

    add_provider_section(&mut gui, &mut layout, root, providers, state, localization);

    Ok((gui, layout))
}

fn add_provider_section(
    gui: &mut Gui,
    layout: &mut LauncherShellLayout,
    root: WidgetId,
    providers: &ProviderDiagnostics,
    state: Option<&LauncherShellState>,
    localization: &Localization,
) {
    let section = gui.add_column(root, true);
    gui.add_label(
        section,
        localization.text("IDS_LAUNCHER_UI_SECTION_PROVIDERS"),
    );
    let (logs_dir, bulk_summary) = match state {
        Some(state) => (
            Some(state.logs_dir.as_path()),
            state.summary.provider_bulk_retarget.as_ref(),
        ),
        None => (None, None),
    };
    if providers.share.is_empty() && providers.upload.is_empty() {
        gui.add_label(section, localization.text("IDS_LAUNCHER_UI_PROVIDERS_NONE"));
        if let Some(summary) = bulk_summary {
            add_bulk_retarget_summary(gui, section, summary, logs_dir, localization);
        }
        return;
    }

    let bulk_row = gui.add_row(section, false);
    gui.add_label(
        bulk_row,
        localization.text("IDS_LAUNCHER_UI_PROVIDERS_BULK_ACTIONS"),
    );
    let restore_button = gui.add_button(
        bulk_row,
        localization.text("IDS_LAUNCHER_UI_BUTTON_RESTORE_DEFAULTS"),
    );
    layout
        .action_map
        .insert(restore_button, WidgetAction::RestoreAllProviderDefaults);
    layout.restore_defaults_button = Some(restore_button);
    let retarget_button = gui.add_button(
        bulk_row,
        localization.text("IDS_LAUNCHER_UI_BUTTON_RETARGET_ALL"),
    );
    layout
        .action_map
        .insert(retarget_button, WidgetAction::RetargetAllProviders);
    layout.retarget_all_button = Some(retarget_button);

    if !providers.share.is_empty() {
        let share_section = gui.add_column(section, true);
        gui.add_label(
            share_section,
            localization.text("IDS_LAUNCHER_UI_SECTION_SHARE_TARGETS"),
        );
        render_provider_list(
            gui,
            layout,
            share_section,
            &providers.share,
            ProviderKind::Share,
            localization,
        );
    }

    if !providers.upload.is_empty() {
        let upload_section = gui.add_column(section, true);
        gui.add_label(
            upload_section,
            localization.text("IDS_LAUNCHER_UI_SECTION_UPLOAD_TARGETS"),
        );
        render_provider_list(
            gui,
            layout,
            upload_section,
            &providers.upload,
            ProviderKind::Upload,
            localization,
        );
    }

    if let Some(summary) = bulk_summary {
        add_bulk_retarget_summary(gui, section, summary, logs_dir, localization);
    }
}

fn render_provider_list(
    gui: &mut Gui,
    layout: &mut LauncherShellLayout,
    parent: WidgetId,
    providers: &[ProviderStatus],
    role: ProviderKind,
    localization: &Localization,
) {
    for (index, provider) in providers.iter().enumerate() {
        render_provider_entry(gui, layout, parent, provider, role, index, localization);
    }
}

fn render_provider_entry(
    gui: &mut Gui,
    layout: &mut LauncherShellLayout,
    parent: WidgetId,
    provider: &ProviderStatus,
    role: ProviderKind,
    index: usize,
    localization: &Localization,
) {
    let entry = gui.add_column(parent, true);
    let provider_path = display_path(&provider.path);
    gui.add_label(
        entry,
        localization.format(
            "IDS_LAUNCHER_UI_PROVIDER_ENTRY",
            [
                ("name", provider.name.as_str()),
                ("path", provider_path.as_str()),
            ],
        ),
    );
    let path_status = format_path_status(&provider.path_status, localization);
    let automation_status = format_automation_state(&provider.automation, localization);
    gui.add_label(
        entry,
        localization.format(
            "IDS_LAUNCHER_UI_PROVIDER_STATUS",
            [
                ("path_status", path_status.as_str()),
                ("automation", automation_status.as_str()),
            ],
        ),
    );
    let default_path = display_path(provider.path_provenance.default_path());
    gui.add_label(
        entry,
        localization.format(
            "IDS_LAUNCHER_UI_PROVIDER_DEFAULT_PATH",
            [("path", default_path.as_str())],
        ),
    );

    let overrides = provider.path_provenance.overrides();
    if overrides.is_empty() {
        gui.add_label(
            entry,
            localization.text("IDS_LAUNCHER_UI_PROVIDER_OVERRIDE_NONE"),
        );
    } else {
        gui.add_label(
            entry,
            localization.text("IDS_LAUNCHER_UI_PROVIDER_OVERRIDE_HEADER"),
        );
        for override_entry in overrides {
            let path = display_path(override_entry.path());
            gui.add_label(
                entry,
                localization.format(
                    "IDS_LAUNCHER_UI_PROVIDER_OVERRIDE_ENTRY",
                    [
                        (
                            "source",
                            format_override_source(override_entry.source(), localization).as_str(),
                        ),
                        ("path", path.as_str()),
                    ],
                ),
            );
        }
    }

    let has_preference_override = provider.path_provenance.has_preference_override();
    let path_is_default = provider.path == provider.path_provenance.default_path();
    if has_preference_override || !path_is_default {
        let controls = gui.add_row(entry, false);
        gui.add_label(
            controls,
            localization.text("IDS_LAUNCHER_UI_PROVIDER_OVERRIDE_CONTROLS"),
        );
        let label_key = if path_is_default {
            "IDS_LAUNCHER_UI_BUTTON_CLEAR_OVERRIDES"
        } else {
            "IDS_LAUNCHER_UI_BUTTON_RESTORE_DEFAULT_PATH"
        };
        let button = gui.add_button(controls, localization.text(label_key));
        layout
            .action_map
            .insert(button, WidgetAction::ClearProviderOverride { role, index });
    }

    if matches!(provider.automation, ProviderAutomationState::Stale { .. }) {
        let actions = gui.add_row(entry, false);
        gui.add_label(
            actions,
            localization.text("IDS_LAUNCHER_UI_PROVIDER_ACTIONS"),
        );
        let restage = gui.add_button(actions, localization.text("IDS_LAUNCHER_UI_BUTTON_RESTAGE"));
        layout
            .action_map
            .insert(restage, WidgetAction::RestageProvider { role, index });
        let retarget = gui.add_button(
            actions,
            localization.text("IDS_LAUNCHER_UI_BUTTON_RETARGET"),
        );
        layout
            .action_map
            .insert(retarget, WidgetAction::RetargetProvider { role, index });
    }
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn format_feedback_label(feedback: &ActionFeedback, localization: &Localization) -> String {
    let label = feedback.kind.label(localization);
    localization.format(
        "IDS_LAUNCHER_UI_FEEDBACK_FORMAT",
        [("label", label), ("message", feedback.message.as_str())],
    )
}

fn localized_artifact_role(localization: &Localization, role: &str) -> String {
    match role {
        "support bundle" => localization
            .text("IDS_LAUNCHER_UI_ARTIFACT_ROLE_SUPPORT_BUNDLE")
            .to_string(),
        "launcher summary" => localization
            .text("IDS_LAUNCHER_UI_ARTIFACT_ROLE_LAUNCHER_SUMMARY")
            .to_string(),
        other => other
            .split_whitespace()
            .map(capitalize_word)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn format_path_status(status: &ProviderPathStatus, localization: &Localization) -> String {
    match status {
        ProviderPathStatus::Ready => localization
            .text("IDS_LAUNCHER_UI_PATH_STATUS_READY")
            .to_string(),
        ProviderPathStatus::Missing => localization
            .text("IDS_LAUNCHER_UI_PATH_STATUS_MISSING")
            .to_string(),
        ProviderPathStatus::NotDirectory => localization
            .text("IDS_LAUNCHER_UI_PATH_STATUS_NOT_DIRECTORY")
            .to_string(),
        ProviderPathStatus::Inaccessible(message) => localization.format(
            "IDS_LAUNCHER_UI_PATH_STATUS_INACCESSIBLE",
            [("message", message.as_str())],
        ),
    }
}

fn format_automation_state(state: &ProviderAutomationState, localization: &Localization) -> String {
    match state {
        ProviderAutomationState::Idle => localization
            .text("IDS_LAUNCHER_UI_AUTOMATION_IDLE")
            .to_string(),
        ProviderAutomationState::Submitted { detail } => localization.format(
            "IDS_LAUNCHER_UI_AUTOMATION_SUBMITTED",
            [("detail", detail.as_str())],
        ),
        ProviderAutomationState::Stale { reason } => localization.format(
            "IDS_LAUNCHER_UI_AUTOMATION_STALE",
            [("reason", reason.as_str())],
        ),
        ProviderAutomationState::Skipped { reason } => localization.format(
            "IDS_LAUNCHER_UI_AUTOMATION_SKIPPED",
            [("reason", reason.as_str())],
        ),
        ProviderAutomationState::Failed { error } => localization.format(
            "IDS_LAUNCHER_UI_AUTOMATION_FAILED",
            [("error", error.as_str())],
        ),
    }
}

fn format_override_source(source: &ProviderOverrideSource, localization: &Localization) -> String {
    match source {
        ProviderOverrideSource::Preference => localization
            .text("IDS_LAUNCHER_UI_OVERRIDE_SOURCE_PREFERENCE")
            .to_string(),
        ProviderOverrideSource::Retargeted { applied_at } => localization.format(
            "IDS_LAUNCHER_UI_OVERRIDE_SOURCE_RETARGETED",
            [("timestamp", applied_at.as_str())],
        ),
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
    localization: &Localization,
) {
    let has_records = !summary.share.is_empty() || !summary.upload.is_empty();
    if !has_records && summary.history_cleared_at.is_none() {
        return;
    }
    if has_records {
        gui.add_label(
            section,
            localization.text("IDS_LAUNCHER_UI_BULK_HISTORY_HEADER"),
        );
        add_bulk_retarget_records(
            gui,
            section,
            localization.text("IDS_LAUNCHER_UI_BULK_SHARE_HEADER"),
            &summary.share,
            logs_dir,
            localization,
        );
        add_bulk_retarget_records(
            gui,
            section,
            localization.text("IDS_LAUNCHER_UI_BULK_UPLOAD_HEADER"),
            &summary.upload,
            logs_dir,
            localization,
        );
    }
    if let Some(cleared_at) = &summary.history_cleared_at {
        let message = if has_records {
            localization.format(
                "IDS_LAUNCHER_UI_BULK_HISTORY_CLEARED_WITH_RECORDS",
                [("timestamp", cleared_at.as_str())],
            )
        } else {
            localization.format(
                "IDS_LAUNCHER_UI_BULK_HISTORY_CLEARED_NO_RECORDS",
                [("timestamp", cleared_at.as_str())],
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
    localization: &Localization,
) {
    if records.is_empty() {
        return;
    }
    gui.add_label(section, label.to_string());
    for record in records {
        let base_path = resolve_logs_entry(logs_dir, &record.base_path);
        let path_str = display_path(&base_path);
        let changed_str = record.changed.to_string();
        let total_str = record.total.to_string();
        gui.add_label(
            section,
            localization.format(
                "IDS_LAUNCHER_UI_BULK_RECORD_ENTRY",
                [
                    ("path", path_str.as_str()),
                    ("timestamp", record.retargeted_at.as_str()),
                    ("changed", changed_str.as_str()),
                    ("total", total_str.as_str()),
                ],
            ),
        );
    }
}

fn resolve_logs_entry(logs_dir: Option<&Path>, entry: &str) -> PathBuf {
    let candidate = Path::new(entry);
    match logs_dir {
        Some(logs_dir) if !candidate.is_absolute() => logs_dir.join(candidate),
        _ => candidate.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_gui::{GuiEvent, Point};
    use clonk_launcher::{
        load_localization, LauncherSummary, LauncherTelemetryFailure, ProviderAutomationSnapshot,
        ProviderAutomationState, ProviderBulkRetargetRecord, ProviderBulkRetargetSummary,
        ProviderDiagnostics, ProviderOverrideSource, ProviderPathProvenance, ProviderPathStatus,
        ProviderStatus, SerializableTelemetryFailure, SerializableTelemetrySummary,
    };
    use clonk_platform::AppPaths;
    use tempfile::TempDir;

    fn test_localization() -> Localization {
        let paths = AppPaths::discover().expect("app paths");
        load_localization(&paths).expect("localization")
    }

    fn build_ui(state: Option<LauncherShellState>) -> LauncherShellUi {
        LauncherShellUi::new(state, test_localization()).expect("ui")
    }

    #[test]
    fn buttons_are_disabled_before_summary_exists() {
        let mut ui = build_ui(None);
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

        let mut ui = build_ui(Some(state.clone()));
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

        let search_focus = ui
            .report_search_focus_button()
            .expect("report search focus button");
        let search_focus_response = click_button(&mut ui, search_focus);
        assert!(matches!(
            search_focus_response.messages.as_slice(),
            [LauncherShellMessage::FocusReportSearch]
        ));

        let search_error = ui
            .report_search_error_button()
            .expect("report search error button");
        let search_error_response = click_button(&mut ui, search_error);
        assert!(matches!(
            search_error_response.messages.as_slice(),
            [LauncherShellMessage::SetReportSearchPreset { preset }]
                if *preset == ReportSearchPreset::Errors
        ));

        let search_warning = ui
            .report_search_warning_button()
            .expect("report search warning button");
        let search_warning_response = click_button(&mut ui, search_warning);
        assert!(matches!(
            search_warning_response.messages.as_slice(),
            [LauncherShellMessage::SetReportSearchPreset { preset }]
                if *preset == ReportSearchPreset::Warnings
        ));

        if let Some(search_clear) = ui.report_search_clear_button() {
            let search_clear_response = click_button(&mut ui, search_clear);
            assert!(
                search_clear_response.messages.is_empty(),
                "clear search should be disabled before a query is active"
            );
        }

        if let Some(search_next) = ui.report_search_next_button() {
            let search_next_response = click_button(&mut ui, search_next);
            assert!(
                search_next_response.messages.is_empty(),
                "next match should be disabled before a query is active"
            );
        }

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
        let mut ui = build_ui(Some(state));
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

    #[test]
    fn report_search_highlights_matches() {
        let temp = TempDir::new().unwrap();
        let mut state = sample_state(temp.path());
        state.support_bundle_report = vec![
            "all systems nominal".into(),
            "ERROR: subsystem offline".into(),
            "Warning: connection unstable".into(),
        ];
        let mut ui = build_ui(Some(state));
        let search_state = ReportSearchState {
            query: "error".into(),
            matches: vec![1],
            active_index: Some(0),
            highlight: ReportSearchHighlight::Error,
            editing: false,
        };
        ui.set_report_search(Some(search_state))
            .expect("set report search state");
        ui.layout(Size::new(960.0, 1280.0));

        let commands = ui.render();
        let active_color = ReportSearchHighlight::Error.active_color();
        assert!(
            commands.iter().any(|command| matches!(
                command,
                DrawCommand::Text { text, color, .. }
                    if text.contains("ERROR: subsystem offline") && *color == active_color
            )),
            "expected the active error line to be rendered with highlight colour"
        );

        let status = ui
            .report_search_status_text()
            .expect("report search status label");
        assert!(
            status.contains("match 1 of 1"),
            "search status should summarise current match position"
        );
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
        let launcher_log_path = logs_dir.join("launcher/clonk-launcher.log");
        let runtime_log_path = logs_dir.join("Clonk-session.log");
        let crash_report_path = logs_dir.join("LegacyClonk-crash.dmp");
        let bundle_path = logs_dir.join("support-bundle.zip");

        let summary = LauncherSummary {
            schema_version: 1,
            generated_at: "2024-05-01T12:00:00Z".into(),
            launcher_log: "launcher/clonk-launcher.log".into(),
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
            report_search: None,
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
        let mut ui = build_ui(Some(state.clone()));

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

        let mut ui = build_ui(None);
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

        let mut ui = build_ui(None);
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

        let mut ui = build_ui(Some(state));
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
        let summary = ProviderBulkRetargetSummary {
            history_cleared_at: Some("2024-06-05T18:30:00Z".into()),
            ..Default::default()
        };
        state.summary.provider_bulk_retarget = Some(summary);

        let mut ui = build_ui(Some(state));
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

        let mut ui = build_ui(None);
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

        let mut ui = build_ui(None);
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
