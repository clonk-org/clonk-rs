//! One presentation-only settings overlay, independent of the underlying screen.
use super::*;
use clonk_frontend::settings_overlay::SettingsAction;
use clonk_frontend::settings_overlay::{SettingId, SettingsCategory, SettingsController};

#[path = "unified_settings/apply.rs"]
mod settings_apply;
#[path = "unified_settings/display.rs"]
mod settings_display;
#[path = "unified_settings/input.rs"]
mod settings_input;
#[path = "unified_settings/session.rs"]
mod settings_session;
#[path = "unified_settings/update.rs"]
mod settings_update;
pub(crate) use settings_display::SettingsDisplayState;

pub(crate) struct UnifiedSettings {
    pub(crate) controller: SettingsController,
    config: Config,
    owns_pause: bool,
    opened_in: AppMode,
    initial_pins: Vec<SettingId>,
    pub(crate) binding: Option<usize>,
    preview: Option<settings_display::DisplayPreview>,
    owner: i32,
}

impl GameApp {
    pub(crate) fn open_unified_voice_settings(&mut self) -> Result<(), EngineError> {
        self.open_unified_settings(SettingsCategory::Audio)?;
        if let Some(settings) = self.unified_settings.as_mut() {
            settings
                .controller
                .select_audio_page(clonk_frontend::settings_overlay::AudioPage::Voice);
        }
        self.update_unified_settings(Instant::now());
        Ok(())
    }

    pub(crate) fn open_unified_settings_for_player(
        &mut self,
        category: SettingsCategory,
        owner: i32,
    ) -> Result<(), EngineError> {
        self.open_unified_settings(category)?;
        if let Some(settings) = self.unified_settings.as_mut() {
            settings.owner = owner;
        }
        self.refresh_unified_session_settings();
        Ok(())
    }

    pub(crate) fn process_unified_settings_actions(
        &mut self,
        actions: Vec<SettingsAction>,
    ) -> Result<(), EngineError> {
        for action in actions {
            match action {
                SettingsAction::Close => self.close_unified_settings(),
                SettingsAction::CancelMicrophoneTest => self.cancel_voice_setup_test(),
                SettingsAction::Change(index, value) => {
                    self.change_unified_setting(index, value)?
                }
                SettingsAction::CaptureBinding(index) => {
                    self.cancel_voice_setup_test();
                    if let Some(settings) = self.unified_settings.as_mut() {
                        settings.binding = Some(index);
                        settings.controller.view.message =
                            "Press a key or controller input. Esc cancels.".into();
                    }
                }
                SettingsAction::TestMicrophone | SettingsAction::RefreshDevices => {
                    if self.voice_setup.is_none() {
                        self.open_voice_setup()?;
                    }
                    self.voice_setup_action(if action == SettingsAction::TestMicrophone {
                        clonk_frontend::voice_setup::VoiceSetupControl::Test
                    } else {
                        clonk_frontend::voice_setup::VoiceSetupControl::Retry
                    })?;
                }
                SettingsAction::ConfirmDisplay(keep) => self.finish_unified_display_preview(keep),
            }
        }
        Ok(())
    }

    pub(crate) fn settings_launcher(&self) -> Option<clonk_frontend::classic_gui::IntRect> {
        (self.config.compat_profile == crate::settings::CompatProfile::Normal
            && self.unified_settings.is_none()
            && self.voice_setup.is_none()
            && self.mode != AppMode::Running
            && (self.mode != AppMode::Menu || self.startup.view != StartupView::MainMenu)
            && self.startup.dialog_fade.is_none()
            && self.startup.view != StartupView::Options
            && self.dialogs.messages.is_empty()
            && self.context_menus.open.is_none())
        .then(|| {
            clonk_frontend::classic_gui::IntRect::new(
                self.rendering.graphics.surface().width() as i32 - 180,
                8,
                168,
                28,
            )
        })
    }

    pub(crate) fn render_unified_settings_to_surface(
        &mut self,
        surface: &mut Surface,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some(resources) = self.assets.message_dialog_resources() else {
            return false;
        };
        if let Some(settings) = self.unified_settings.as_mut() {
            let (Some(assets), Some(book)) = (
                self.assets.options_dlg_assets(self.config.compat_profile),
                self.assets.options_book_fonts.as_deref(),
            ) else {
                return false;
            };
            settings.controller.render(
                surface,
                &assets,
                resources.fonts,
                book,
                self.mode == AppMode::Menu,
                gamma,
            );
            true
        } else if let Some(bounds) = self.settings_launcher() {
            resources.skin.draw_button(
                surface,
                bounds,
                &format!("Settings (Ctrl+{})", format_key_label(self.settings_key)),
                resources.fonts,
                Default::default(),
                gamma,
            );
            true
        } else {
            false
        }
    }

    pub(crate) fn render_unified_settings(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let launcher = self.settings_launcher();
        let Some(resources) = self.assets.message_dialog_resources() else {
            return false;
        };
        let surface = self.rendering.graphics.surface_mut();
        if let Some(settings) = self.unified_settings.as_mut() {
            let (Some(assets), Some(book)) = (
                self.assets.options_dlg_assets(self.config.compat_profile),
                self.assets.options_book_fonts.as_deref(),
            ) else {
                return false;
            };
            settings.controller.render(
                surface,
                &assets,
                resources.fonts,
                book,
                self.mode == AppMode::Menu,
                gamma,
            );
            true
        } else if let Some(bounds) = launcher {
            resources.skin.draw_button(
                surface,
                bounds,
                &format!("Settings (Ctrl+{})", format_key_label(self.settings_key)),
                resources.fonts,
                Default::default(),
                gamma,
            );
            true
        } else {
            false
        }
    }

    pub(crate) fn open_unified_settings(
        &mut self,
        category: SettingsCategory,
    ) -> Result<(), EngineError> {
        if let Some(settings) = self.unified_settings.as_mut() {
            settings.controller.select_category(category);
            return Ok(());
        }
        self.guard_classic_global_gui_bootstrap()?;
        if self.mode == AppMode::Running {
            // Release held controls through the synchronized path, just as a
            // native modal does (C4PlayerList.cpp:588-595).
            self.dispatch_control_event(ControlEvent::ClearPressed)?;
            self.release_all_running_pointer_elements();
            self.suspend_ingame_pointer_for_gui();
        }
        self.close_voice_setup();
        self.close_context_menu_silently();
        // A release after returning must not activate a button pressed before
        // the overlay opened. Clear the underlying widgets' hover/capture too.
        self.pointer_left_unchecked();
        let mut config = self
            .app_paths
            .as_ref()
            .and_then(|paths| Config::load(paths.config_file()).ok())
            .unwrap_or_default();
        self.apply_display_flags_to_config(&mut config);
        let scale = self
            .loader
            .render_config
            .map(|config| config.application_scale())
            .unwrap_or_else(|| DisplayOptions::load(self.app_paths.as_ref()).scale);
        config.set_in(
            Some("Graphics"),
            "Scale",
            ((scale * 100.0).round() as i32).to_string(),
        );
        config.set_in(
            Some("Graphics"),
            "ResolutionX",
            self.rendering.graphics.surface().width().to_string(),
        );
        config.set_in(
            Some("Graphics"),
            "ResolutionY",
            self.rendering.graphics.surface().height().to_string(),
        );
        config.set_in(
            Some("Graphics"),
            "DisplayMode",
            if self.rendering.display_flags.is_fullscreen {
                "Fullscreen"
            } else {
                "Window"
            },
        );
        self.bindings.write_to_config(&mut config);
        self.input_routing
            .gamepad_bindings
            .write_to_config(&mut config);
        if let Some(audio) = self.sound.context.as_ref() {
            let audio = audio.borrow();
            audio.options.write_startup_sound_config(&mut config);
            audio.options.write_voice_setup_config(&mut config);
        }
        for (section, entries) in self.config.deferred.pending_by_section() {
            for (key, value) in entries {
                config.set_in(Some(&section), key, value);
            }
        }
        let mut controller = SettingsController::new(crate::settings_catalog::catalog(&config));
        controller.pinned = config
            .get_in(Some("Settings"), "Favorites")
            .map(|value| {
                value
                    .split(',')
                    .filter_map(|id| id.split_once(':'))
                    .map(|(section, key)| SettingId::new(section, key))
                    .collect()
            })
            .unwrap_or_else(|| {
                [
                    ("Sound", "MusicVolume"),
                    ("Sound", "SoundVolume"),
                    ("Voice", "Enabled"),
                    ("Voice", "InputDevice"),
                    ("Voice", "Volume"),
                    ("Graphics", "Scale"),
                    ("Graphics", "DisplayMode"),
                    ("Chat", "TextSize"),
                ]
                .into_iter()
                .map(|(section, key)| SettingId::new(section, key))
                .collect()
            });
        controller.select_category(category);
        let surface = self.rendering.graphics.surface();
        controller.resize(surface.width() as i32, surface.height() as i32);
        if let (Some(fonts), Some(book)) = (
            self.assets.clonk_fonts.as_deref(),
            self.assets.options_book_fonts.as_deref(),
        ) {
            controller.resize_book(surface.width() as i32, surface.height() as i32, fonts, book);
        }
        let owns_pause = self.mode == AppMode::Running
            && self.runtime_network_role() == RuntimeNetworkRole::Offline;
        if owns_pause {
            self.netplay.offline_halt_count = self.netplay.offline_halt_count.saturating_add(1);
        }
        controller.view.context = if owns_pause {
            "Game paused"
        } else if self.mode == AppMode::Running {
            "Online game continues while settings are open"
        } else {
            "Changes stay with you when you return to the game"
        }
        .into();
        self.unified_settings = Some(UnifiedSettings {
            initial_pins: controller.pinned.clone(),
            controller,
            config,
            owns_pause,
            opened_in: self.mode,
            binding: None,
            preview: None,
            owner: self.players.local_owner,
        });
        self.refresh_unified_binding_labels();
        if let Some(audio) = self.sound.context.as_ref() {
            audio.borrow().system.refresh_voice_input_devices();
        }
        self.update_unified_settings(Instant::now());
        Ok(())
    }
    pub(crate) fn close_unified_settings(&mut self) {
        self.finish_unified_display_preview(false);
        if let Some(settings) = self.unified_settings.as_ref() {
            if settings.initial_pins != settings.controller.pinned {
                self.config.deferred.set(
                    "Settings",
                    "Favorites",
                    settings
                        .controller
                        .pinned
                        .iter()
                        .map(|id| format!("{}:{}", id.section, id.key))
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
        }
        if let Err(error) = self.save_unified_settings() {
            if let Some(settings) = self.unified_settings.as_mut() {
                settings.controller.view.message =
                    format!("Could not save: {error}. Close again to retry.");
            }
            return;
        }
        if let Some(settings) = self.unified_settings.take() {
            if settings.owns_pause && settings.opened_in == self.mode {
                self.netplay.offline_halt_count =
                    self.netplay.offline_halt_count.saturating_sub(1).max(0);
            }
        }
        self.close_voice_setup();
    }
}
