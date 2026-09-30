use super::*;
use clonk_frontend::settings_overlay::ApplyPolicy;
use clonk_frontend::startup_options_advanced::{AdvancedConfigChange, AdvancedConfigValue};

impl GameApp {
    pub(crate) fn change_unified_setting(
        &mut self,
        index: usize,
        value: AdvancedConfigValue,
    ) -> Result<(), EngineError> {
        let Some(setting) = self
            .unified_settings
            .as_ref()
            .and_then(|s| s.controller.settings.get(index))
            .cloned()
        else {
            return Ok(());
        };
        if setting.details.unavailable.is_some() || !valid_value(&setting.value, &value) {
            return Ok(());
        }
        if setting.id.section == "Session" {
            return self.change_unified_session_setting(&setting.id.key, &value);
        }
        if setting.details.policy == ApplyPolicy::DisplayPreview {
            self.begin_unified_display_preview(index, value);
            return Ok(());
        }
        let id = &setting.id;
        let text = value.serialized();
        let mut change = Config::new();
        crate::advanced_config::apply_changes(
            &mut change,
            &[AdvancedConfigChange {
                section: id.section.clone(),
                key: id.key.clone(),
                value: text.clone(),
            }],
        );
        let Some(text) = change.get_in(Some(&id.section), &id.key).map(str::to_owned) else {
            return Ok(());
        };
        if setting.details.policy == ApplyPolicy::Live {
            self.apply_unified_preference(id, &change)?;
        } else if setting.details.policy == ApplyPolicy::NextGame {
            // These are defaults read when the next engine is constructed.
            // Never change the active deterministic engine from this overlay.
            let enabled = text == "true" || text == "1";
            match (id.section.as_str(), id.key.as_str()) {
                ("Graphics", "SmokeLevel") => {
                    self.rendering.graphics_smoke_level = text.parse().unwrap_or(100)
                }
                ("Graphics", "FireParticles") => {
                    self.rendering.display_flags.fire_particles = enabled
                }
                ("General", "Record") => {
                    self.startup.view_flags.record = enabled;
                    if self.mode == AppMode::Menu {
                        self.records.enabled = enabled && self.records.directory.is_some();
                        self.scenario_game_options
                            .set_selector_preferences(None, Some(enabled));
                    }
                }
                ("General", "NoCrew") => {
                    self.startup.view_flags.fair_crew = enabled;
                    if self.mode == AppMode::Menu {
                        self.scenario_game_options
                            .set_selector_preferences(Some(enabled), None);
                    }
                }
                ("General", "AllowScriptingInReplays") => self.allow_scripting_in_replays = enabled,
                _ => {}
            }
        }
        self.config.deferred.set(&id.section, &id.key, &text);
        if let Some(settings) = self.unified_settings.as_mut() {
            settings.config.set_in(Some(&id.section), &id.key, &text);
            if let Some(row) = settings.controller.settings.get_mut(index) {
                if row.details.policy != ApplyPolicy::Live && row.details.active_value.is_none() {
                    row.details.active_value = Some(row.value.serialized());
                }
                row.value = match value {
                    AdvancedConfigValue::Text(_) => AdvancedConfigValue::Text(text),
                    other => other,
                };
            }
            settings.controller.view.message =
                format!("{} · {}", setting.label, setting.details.policy.label());
        }
        self.refresh_unified_binding_labels();
        Ok(())
    }

    fn apply_unified_preference(
        &mut self,
        id: &SettingId,
        change: &Config,
    ) -> Result<(), EngineError> {
        let text = change.get_in(Some(&id.section), &id.key).unwrap_or("");
        let enabled = matches!(text, "true" | "1");
        if matches!(id.section.as_str(), "Sound" | "Voice") {
            self.cancel_voice_setup_test();
            if self.sound.context.is_none() {
                let mut options = AudioOptions::load(self.app_paths.as_ref());
                options.update_preferences(change);
                match AudioContext::try_new_with_paths(options, self.app_paths.as_ref()) {
                    Ok(audio) => {
                        self.sound.context = Some(connect_audio_context(&mut self.engine, audio))
                    }
                    Err(error) => tracing::warn!(%error, "settings audio unavailable"),
                }
            }
            if let Some(audio) = self.sound.context.as_ref() {
                let mut audio = audio.borrow_mut();
                audio.options.update_preferences(change);
                let music = audio.options.music_volume_percent();
                let sound = audio.options.sound_volume_percent();
                audio.set_music_volume_percent(music);
                audio.set_sound_volume_percent(sound);
                if id.key == "OutputDevice" {
                    audio
                        .system
                        .select_output_device(audio.options.voice_output_device.clone());
                }
            }
            if id.section == "Sound" && self.sound.context.is_some() {
                if id.key == "Music" && self.mode == AppMode::Running {
                    self.sound.set_runtime_music_playback(
                        enabled,
                        self.scenario_lifecycle
                            .active
                            .as_ref()
                            .and_then(|scenario| scenario.path.as_deref()),
                    );
                } else if id.key == "MenuMusic" && self.mode != AppMode::Running {
                    self.sound.set_frontend_music_option(enabled)?;
                }
            }
        }
        if let Some((set, binding)) = crate::settings_catalog::keyboard_binding(id) {
            if let Some(key) = text
                .parse()
                .ok()
                .and_then(crate::input::decode_platform_key_code)
            {
                self.bindings.rebind_for_set(set, binding, key);
                self.engine
                    .set_control_key_names(configured_control_key_names(&self.bindings));
            }
        }
        if let Some((set, binding)) = crate::settings_catalog::gamepad_binding(id) {
            if let Ok(raw) = text.parse() {
                self.input_routing
                    .gamepad_bindings
                    .rebind_raw(set, binding, raw);
            }
        }
        let flags = &mut self.rendering.display_flags;
        match (id.section.as_str(), id.key.as_str()) {
            ("Settings", "OpenKey") => {
                if let Some(key) = text
                    .parse()
                    .ok()
                    .and_then(crate::input::decode_platform_key_code)
                    .filter(|key| *key != VirtualKeyCode::Escape)
                {
                    self.settings_key = key;
                }
            }
            ("Graphics", "ShowPortraits") => flags.portraits = enabled,
            ("Graphics", "ShowCrewNames") => flags.player_names = enabled,
            ("Graphics", "ShowCrewCNames") => flags.clonk_names = enabled,
            ("Graphics", "ShowCommands") => flags.show_commands = enabled,
            ("Graphics", "ShowCommandKeys") => flags.show_command_keys = enabled,
            ("Graphics", "ShowPlayerHUDAlways") => flags.show_player_hud_always = enabled,
            ("Graphics", "ShowClock") => flags.clock = enabled,
            ("Graphics", "ShowStats") => flags.show_stats = enabled,
            ("Graphics", "SplitscreenDividers") => flags.splitscreen_dividers = enabled,
            ("Graphics", "PXSGfx") => {
                flags.pxs_gfx = enabled;
                self.rendering.graphics.set_pxs_graphics(enabled);
            }
            ("General", "FPS") => flags.fps = enabled,
            ("General", "ScrollSmooth") => flags.scroll_smooth = text.parse().unwrap_or(0),
            ("General", "UseWhiteIngameChat") => flags.white_chat = enabled,
            ("General", "UseWhiteLobbyChat") => self.lobby.white_chat = enabled,
            ("General", "ShowLogTimestamps") => self.chat.show_log_timestamps = enabled,
            ("Controls", "GamepadGuiControl") => self.config.gamepad_gui_control = enabled,
            ("Toasts", "ReadyCheck") => self.lobby.ready_check_toasts_enabled = enabled,
            ("Graphics", "ShowFolderMaps") => self.config.show_folder_maps = enabled,
            ("Graphics", "MsgBoard") => self.set_message_board_line_count(i32::from(enabled)),
            ("Graphics", "PointFiltering") => self.rendering.graphics.set_point_filtering(enabled),
            ("Graphics", "UpperBoard") => {
                flags.upper_board = match text {
                    "Small" => UpperBoardMode::Small,
                    "Mini" => UpperBoardMode::Mini,
                    "Hide" => UpperBoardMode::Hide,
                    _ => UpperBoardMode::Full,
                };
                let mode = frontend_upper_board_mode(flags.upper_board);
                let time = self.game_time_seconds();
                self.rendering.graphics.set_upper_board_mode(mode, time);
            }
            ("Chat", _) => {
                if let Some(settings) = self.unified_settings.as_ref() {
                    let mut config = settings.config.clone();
                    config.set_in(Some(&id.section), &id.key, text);
                    self.chat.enhanced_preferences =
                        crate::settings::enhanced_chat_preferences(&config);
                }
            }
            ("Graphics", "NoAlphaAdd" | "AllowedBlitModes" | "TexIndent" | "BlitOffset") => {
                if let Some(settings) = self.unified_settings.as_ref() {
                    let mut config = settings.config.clone();
                    config.set_in(Some(&id.section), &id.key, text);
                    if let Ok(config) = config.to_string() {
                        self.rendering.graphics.set_advanced_renderer_config(
                            load_advanced_renderer_config(config.as_bytes()),
                        );
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn refresh_unified_binding_labels(&mut self) {
        if let Some(settings) = self.unified_settings.as_mut() {
            for setting in &mut settings.controller.settings {
                if !setting.details.binding {
                    continue;
                }
                setting.details.display_value =
                    setting.value.serialized().parse::<i32>().ok().map(|raw| {
                        if crate::settings_catalog::gamepad_binding(&setting.id).is_some() {
                            crate::input::legacy_gamepad_key_label(Some(raw))
                        } else {
                            crate::input::decode_platform_key_code(raw)
                                .map(format_key_label)
                                .unwrap_or_else(|| "Unassigned".into())
                        }
                    });
            }
        }
    }

    pub(crate) fn save_unified_settings(&mut self) -> io::Result<()> {
        let Some(paths) = self.app_paths.as_ref() else {
            return Ok(());
        };
        let pending = self.config.deferred.pending_by_section();
        if pending.is_empty() {
            return Ok(());
        }
        let path = paths.config_file();
        let mut config = match Config::load(&path) {
            Ok(config) => config,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
            Err(error) => return Err(error),
        };
        for (section, entries) in pending {
            for (key, text) in entries {
                config.set_in(Some(&section), key, text);
            }
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("config has no directory"))?;
        fs::create_dir_all(parent)?;
        let temp = tempfile::NamedTempFile::new_in(parent)?;
        if let Ok(metadata) = fs::metadata(&path) {
            temp.as_file().set_permissions(metadata.permissions())?;
        }
        let boolean = |key, default| {
            config
                .get_in(Some("General"), key)
                .map(|value| matches!(value.trim(), "true" | "1"))
                .unwrap_or(default)
        };
        save_config_preserving_native_general_booleans(
            &config,
            temp.path(),
            Some(boolean("GamepadEnabled", true)),
            Some(boolean("DebugMode", false)),
        )?;
        temp.as_file().sync_all()?;
        temp.persist(&path).map_err(|error| error.error)?;
        self.config.deferred.take_by_section();
        Ok(())
    }
}

fn valid_value(current: &AdvancedConfigValue, proposed: &AdvancedConfigValue) -> bool {
    match (current, proposed) {
        (AdvancedConfigValue::Bool(_), AdvancedConfigValue::Bool(_)) => true,
        (
            AdvancedConfigValue::Integer { min, max, .. },
            AdvancedConfigValue::Integer { value, .. },
        ) => (min..=max).contains(&value),
        (
            AdvancedConfigValue::Choice { choices, .. },
            AdvancedConfigValue::Choice { value, .. },
        ) => choices.iter().any(|choice| &choice.value == value),
        (AdvancedConfigValue::Text(_), AdvancedConfigValue::Text(value)) => {
            value.len() <= 254 && !value.contains(['\r', '\n', '\0'])
        }
        _ => false,
    }
}
