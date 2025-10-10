use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lc_gui::{
    DrawCommand, Gui, GuiAction, GuiEvent, GuiEventResult, GuiResult, Rect, Size, WidgetId,
};
use lc_launcher::{
    support_artifacts, LauncherShellState, ProviderAutomationState, ProviderDiagnostics,
    ProviderPathStatus, ProviderStatus, SupportArtifact,
};

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
}

impl LauncherShellUi {
    pub fn new(state: Option<LauncherShellState>) -> GuiResult<Self> {
        let mut ui = Self {
            gui: Gui::new(),
            layout: LauncherShellLayout::default(),
            state: None,
            feedback: None,
            providers: ProviderDiagnostics::default(),
        };
        ui.set_state(state)?;
        Ok(ui)
    }

    pub fn set_state(&mut self, state: Option<LauncherShellState>) -> GuiResult<()> {
        self.state = state;
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

    pub fn state(&self) -> Option<&LauncherShellState> {
        self.state.as_ref()
    }

    pub fn action_feedback(&self) -> Option<&ActionFeedback> {
        self.feedback.as_ref()
    }

    fn rebuild(&mut self) -> GuiResult<()> {
        let borrowed_state = self.state.as_ref();
        let feedback = self.feedback.as_ref();
        let (gui, layout) = build_gui(borrowed_state, feedback, &self.providers)?;
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
}

#[derive(Default)]
struct LauncherShellLayout {
    action_map: HashMap<WidgetId, WidgetAction>,
    support_artifacts: Vec<SupportArtifact>,
    feedback_message: Option<String>,
    regenerate_button: Option<WidgetId>,
    copy_button: Option<WidgetId>,
    upload_button: Option<WidgetId>,
}

enum WidgetAction {
    RegenerateBundle,
    CopyBundle { bundle_path: PathBuf },
    Reveal { path: PathBuf, label: String },
    UploadArtifacts,
    RestageProvider { role: ProviderKind, index: usize },
    RetargetProvider { role: ProviderKind, index: usize },
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
        }
    }
}

fn build_gui(
    state: Option<&LauncherShellState>,
    feedback: Option<&ActionFeedback>,
    providers: &ProviderDiagnostics,
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

    add_provider_section(&mut gui, &mut layout, root, providers);

    Ok((gui, layout))
}

fn add_provider_section(
    gui: &mut Gui,
    layout: &mut LauncherShellLayout,
    root: WidgetId,
    providers: &ProviderDiagnostics,
) {
    let section = gui.add_column(root, true);
    gui.add_label(section, "First-Party Providers");
    if providers.share.is_empty() && providers.upload.is_empty() {
        gui.add_label(
            section,
            "No first-party providers are configured. Configure LC_FIRST_PARTY_* variables to enable automated submissions.",
        );
        return;
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use lc_gui::{GuiEvent, Point};
    use lc_launcher::{
        LauncherSummary, LauncherTelemetryFailure, ProviderAutomationSnapshot,
        ProviderAutomationState, ProviderDiagnostics, ProviderPathStatus, ProviderStatus,
        SerializableTelemetryFailure, SerializableTelemetrySummary,
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
        ui.layout(Size::new(800.0, 600.0));

        let regenerate = ui.regenerate_button().expect("regenerate button");
        let regenerate_response = click_button(&mut ui, regenerate);
        assert!(matches!(
            regenerate_response.messages.as_slice(),
            [LauncherShellMessage::RegenerateSupportBundle]
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
}
