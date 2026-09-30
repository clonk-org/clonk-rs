use super::*;
use clonk_audio::VoiceInputDeviceInventory;
use clonk_frontend::startup_options_advanced::{AdvancedConfigChoice, AdvancedConfigValue};

impl GameApp {
    pub(crate) fn update_unified_settings(&mut self, now: Instant) {
        if self.unified_settings.is_none() {
            return;
        }
        if self
            .unified_settings
            .as_ref()
            .and_then(|settings| settings.preview.as_ref())
            .is_some_and(|preview| now >= preview.deadline)
        {
            self.finish_unified_display_preview(false);
        }
        let owns_pause = self.mode == AppMode::Running
            && self.runtime_network_role() == RuntimeNetworkRole::Offline;
        if let Some(settings) = self.unified_settings.as_mut() {
            settings.controller.view.display_confirmation = settings
                .preview
                .as_ref()
                .map(|preview| preview.deadline.saturating_duration_since(now).as_secs() + 1);
            if settings.opened_in != self.mode {
                settings.owns_pause = false;
                settings.binding = None;
                settings.controller.cancel_interaction();
                settings.opened_in = self.mode;
            }
            if owns_pause && !settings.owns_pause {
                self.netplay.offline_halt_count = self.netplay.offline_halt_count.saturating_add(1);
                settings.owns_pause = true;
            } else if settings.owns_pause && !owns_pause {
                self.netplay.offline_halt_count =
                    self.netplay.offline_halt_count.saturating_sub(1).max(0);
                settings.owns_pause = false;
            }
            settings.controller.view.context = match self.mode {
                AppMode::Running if owns_pause => "Game paused · Esc returns to your game",
                AppMode::Running => {
                    "Online game continues · Your controls are held while settings are open"
                }
                AppMode::Loading => "Loading continues · Settings stay open when the game begins",
                AppMode::Menu => {
                    "Settings stay with you · Ctrl+F searches · Tab navigates · Esc returns"
                }
            }
            .into();
        }
        if self.voice_setup.is_none()
            && self
                .unified_settings
                .as_ref()
                .is_some_and(|settings| settings.controller.category == SettingsCategory::Audio)
        {
            if let Err(error) = self.open_voice_setup() {
                tracing::warn!(%error, "could not initialize settings microphone test");
            }
        }
        let state = self.voice_options_state();
        self.refresh_unified_session_settings();
        let audio = self.sound.context.as_ref().map(|audio| audio.borrow());
        let input = audio
            .as_ref()
            .map(|audio| audio.system.voice_input_inventory());
        let output = audio
            .as_ref()
            .map(|audio| audio.system.output_devices())
            .unwrap_or_default();
        if let Some(settings) = self.unified_settings.as_mut() {
            settings.controller.view.microphone_status = if audio.is_some() {
                state.status
            } else {
                "Audio unavailable. Connect a device and choose Refresh devices.".into()
            };
            settings.controller.view.microphone_level = state.level;
            settings.controller.view.microphone_testing = state.testing;
            settings.controller.view.audio_device_status = state.device_status;
            for key in ["InputDevice", "OutputDevice"] {
                let Some(setting) = settings
                    .controller
                    .settings
                    .iter_mut()
                    .find(|s| s.id.section == "Voice" && s.id.key == key)
                else {
                    continue;
                };
                let current = setting.value.serialized();
                let mut choices = vec![AdvancedConfigChoice {
                    value: String::new(),
                    label: "System default".into(),
                }];
                if key == "InputDevice" {
                    if let Some(VoiceInputDeviceInventory::Ready(devices)) = &input {
                        choices.extend(devices.iter().map(|device| {
                            AdvancedConfigChoice {
                                value: device.id.as_str().into(),
                                label: if devices
                                    .iter()
                                    .filter(|other| other.name == device.name)
                                    .count()
                                    > 1
                                {
                                    format!("{} ({})", device.name, device.id)
                                } else {
                                    device.name.clone()
                                },
                            }
                        }));
                    }
                } else {
                    choices.extend(output.iter().map(|device| AdvancedConfigChoice {
                        value: device.id.clone(),
                        label: device.name.clone(),
                    }));
                }
                if !choices.iter().any(|choice| choice.value == current) {
                    choices.push(AdvancedConfigChoice {
                        value: current.clone(),
                        label: format!("Unavailable ({current})"),
                    });
                }
                setting.default = AdvancedConfigValue::Choice {
                    value: String::new(),
                    choices: choices.clone(),
                };
                setting.value = AdvancedConfigValue::Choice {
                    value: current,
                    choices,
                };
            }
        }
    }
}
