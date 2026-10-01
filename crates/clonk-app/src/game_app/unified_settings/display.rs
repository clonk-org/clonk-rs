use super::*;
use clonk_frontend::startup_options_advanced::AdvancedConfigValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SettingsDisplayState {
    pub(crate) mode: DisplayMode,
    pub(crate) percent: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(super) struct DisplayPreview {
    pub(super) deadline: Instant,
    index: usize,
    before: AdvancedConfigValue,
    candidate: SettingsDisplayState,
}

impl GameApp {
    pub(crate) fn begin_unified_display_preview(
        &mut self,
        index: usize,
        value: AdvancedConfigValue,
    ) {
        let Some(settings) = self.unified_settings.as_mut() else {
            return;
        };
        if settings.preview.is_some() {
            return;
        }
        let Some(setting) = settings.controller.settings.get(index) else {
            return;
        };
        let mut config = settings.config.clone();
        let integer = |config: &Config, key, default| {
            config
                .get_in(Some("Graphics"), key)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(default)
        };
        let old_percent = integer(&config, "Scale", 100).max(1);
        let mut width = integer(&config, "ResolutionX", 800) * old_percent / 100;
        let mut height = integer(&config, "ResolutionY", 600) * old_percent / 100;
        config.set_in(Some("Graphics"), &setting.id.key, value.serialized());
        let percent = integer(&config, "Scale", 100).max(1);
        if setting.id.key == "ResolutionX" {
            width = integer(&config, "ResolutionX", 800) * percent / 100;
        }
        if setting.id.key == "ResolutionY" {
            height = integer(&config, "ResolutionY", 600) * percent / 100;
        }
        let mode = if config.get_in(Some("Graphics"), "DisplayMode") == Some("Fullscreen") {
            DisplayMode::Fullscreen
        } else {
            DisplayMode::Window
        };
        if matches!(setting.id.key.as_str(), "ResolutionX" | "ResolutionY")
            && mode == DisplayMode::Fullscreen
        {
            settings.controller.view.message =
                "Switch to Windowed to adjust the window dimensions.".into();
            return;
        }
        if width * 100 / percent < 640 || height * 100 / percent < 480 {
            settings.controller.view.message =
                "This scale leaves too little room. Enlarge the window or choose a smaller scale."
                    .into();
            return;
        }
        let candidate = SettingsDisplayState {
            mode,
            percent: percent as i32,
            width,
            height,
        };
        settings.preview = Some(DisplayPreview {
            deadline: Instant::now() + Duration::from_secs(15),
            index,
            before: setting.value.clone(),
            candidate,
        });
        settings.controller.settings[index].value = value;
        settings.controller.view.display_confirmation = Some(15);
        self.queue_options_display_request(OptionsDisplayRequest::SettingsPreview(candidate));
    }

    pub(crate) fn finish_unified_display_preview(&mut self, keep: bool) {
        let Some(preview) = self
            .unified_settings
            .as_mut()
            .and_then(|settings| settings.preview.take())
        else {
            return;
        };
        if let Some(settings) = self.unified_settings.as_mut() {
            settings.controller.view.display_confirmation = None;
            if !keep {
                if let Some(setting) = settings.controller.settings.get_mut(preview.index) {
                    setting.value = preview.before;
                }
            }
            settings.controller.view.message = if keep {
                "Display settings accepted"
            } else {
                "Previous display settings restored"
            }
            .into();
        }
        if keep {
            self.record_unified_display_result(preview.candidate);
        }
        self.queue_options_display_request(OptionsDisplayRequest::SettingsConfirm(keep));
    }

    pub(crate) fn record_unified_display_result(&mut self, state: SettingsDisplayState) {
        let scale = state.percent.max(1) as u32;
        for (key, text) in [
            ("Scale", state.percent.to_string()),
            ("ResolutionX", (state.width * 100 / scale).to_string()),
            ("ResolutionY", (state.height * 100 / scale).to_string()),
            (
                "DisplayMode",
                if state.mode == DisplayMode::Fullscreen {
                    "Fullscreen"
                } else {
                    "Window"
                }
                .into(),
            ),
        ] {
            self.config.deferred.set("Graphics", key, &text);
            if let Some(settings) = self.unified_settings.as_mut() {
                settings.config.set_in(Some("Graphics"), key, &text);
                if let Some(setting) = settings
                    .controller
                    .settings
                    .iter_mut()
                    .find(|setting| setting.id.section == "Graphics" && setting.id.key == key)
                {
                    match &mut setting.value {
                        AdvancedConfigValue::Integer { value, .. } => {
                            if let Ok(new) = text.parse() {
                                *value = new;
                            }
                        }
                        AdvancedConfigValue::Choice { value, .. } => *value = text.clone(),
                        _ => {}
                    }
                }
            }
        }
    }
}
