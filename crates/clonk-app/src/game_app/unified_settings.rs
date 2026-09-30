//! One presentation-only settings overlay, independent of the underlying screen.
use super::*;
use clonk_frontend::settings_overlay::SettingsAction;
use clonk_frontend::settings_overlay::{
    AudioPage, SettingId, SettingsCategory, SettingsController,
};

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
    preview: Option<settings_display::DisplayPreview>,
    owner: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingsPage {
    category: SettingsCategory,
    audio_page: AudioPage,
    group: Option<String>,
}

impl GameApp {
    /// Opens settings where they last closed; the first time, on the
    /// program page.
    pub(crate) fn open_unified_settings_where_left(&mut self) -> Result<(), EngineError> {
        self.open_unified_settings(SettingsCategory::Interface)?;
        self.resume_unified_settings_page();
        Ok(())
    }

    /// Returns an open overlay to the page and control set it last closed on.
    pub(crate) fn resume_unified_settings_page(&mut self) {
        let (Some(page), Some(settings)) = (
            self.settings_return_page.clone(),
            self.unified_settings.as_mut(),
        ) else {
            return;
        };
        let controller = &mut settings.controller;
        controller.group = page.group;
        if page.category == SettingsCategory::Audio {
            controller.select_audio_page(page.audio_page);
        } else {
            controller.select_category(page.category);
        }
    }

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
                        settings.controller.view.capturing = Some(index);
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

    /// The main menu hands the whole screen to the options book, as C++'s
    /// options dialog does. Every other screen stays in view, dimmed, behind
    /// settings opened over it.
    pub(crate) fn settings_cover_screen(&self) -> bool {
        self.mode == AppMode::Menu && self.startup.view == StartupView::MainMenu
    }

    pub(crate) fn render_unified_settings_to_surface(
        &mut self,
        surface: &mut Surface,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let cover_screen = self.settings_cover_screen();
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
                cover_screen,
                gamma,
            );
            if settings.controller.has_popup() {
                settings
                    .controller
                    .render_popup(surface, &assets, resources.fonts, book, gamma);
            }
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
        let cover_screen = self.settings_cover_screen();
        let assets = std::sync::Arc::clone(&self.assets);
        let Some(resources) = assets.message_dialog_resources() else {
            return false;
        };
        let Some(settings) = self.unified_settings.as_mut() else {
            let Some(bounds) = launcher else {
                return false;
            };
            resources.skin.draw_button(
                self.rendering.graphics.surface_mut(),
                bounds,
                &format!("Settings (Ctrl+{})", format_key_label(self.settings_key)),
                resources.fonts,
                Default::default(),
                gamma,
            );
            return true;
        };
        let (Some(dialog), Some(book)) = (
            assets.options_dlg_assets(self.config.compat_profile),
            assets.options_book_fonts.as_deref(),
        ) else {
            return false;
        };
        let surface = self.rendering.graphics.surface_mut();
        settings
            .controller
            .render(surface, &dialog, resources.fonts, book, cover_screen, gamma);
        if settings.controller.has_popup() {
            // An ordered presentation draws each layer's text above its boxes,
            // so there the popup takes a layer of its own and the page's text
            // stays under it. Drawing straight to the surface keeps its order.
            if self
                .rendering
                .graphics
                .surface()
                .is_clonk_text_capture_active()
            {
                self.next_pending_native_overlay();
            }
            if let Some(settings) = self.unified_settings.as_mut() {
                settings.controller.render_popup(
                    self.rendering.graphics.surface_mut(),
                    &dialog,
                    resources.fonts,
                    book,
                    gamma,
                );
            }
        }
        true
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
        let popup = !self.settings_cover_screen();
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
            controller.resize_book(
                surface.width() as i32,
                surface.height() as i32,
                fonts,
                book,
                popup,
            );
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
            controller,
            config,
            owns_pause,
            opened_in: self.mode,
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
        if let Err(error) = self.save_unified_settings() {
            if let Some(settings) = self.unified_settings.as_mut() {
                settings.controller.view.message =
                    format!("Could not save: {error}. Close again to retry.");
            }
            return;
        }
        if let Some(settings) = self.unified_settings.take() {
            self.settings_return_page = Some(SettingsPage {
                category: settings.controller.category,
                audio_page: settings.controller.audio_page,
                group: settings.controller.group.clone(),
            });
            if settings.owns_pause && settings.opened_in == self.mode {
                self.netplay.offline_halt_count =
                    self.netplay.offline_halt_count.saturating_sub(1).max(0);
            }
        }
        self.close_voice_setup();
    }
}
