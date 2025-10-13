use lc_core::std_config::Config;
use lc_platform::AppPaths;
use std::io::ErrorKind;

const DEFAULT_MAX_CHANNELS: usize = 32;
const MAX_CHANNELS_LIMIT: usize = 1024;

#[derive(Debug, Clone)]
pub struct AudioOptions {
    pub max_channels: usize,
    pub sound_enabled: bool,
    pub music_enabled: bool,
    pub menu_music_enabled: bool,
    pub menu_sound_enabled: bool,
    pub sound_volume: f32,
    pub music_volume: f32,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            max_channels: DEFAULT_MAX_CHANNELS,
            sound_enabled: true,
            music_enabled: true,
            menu_music_enabled: true,
            menu_sound_enabled: true,
            sound_volume: 1.0,
            music_volume: 1.0,
        }
    }
}

impl AudioOptions {
    pub fn load(paths: Option<&AppPaths>) -> Self {
        let mut options = Self::default();
        let Some(paths) = paths else {
            return options;
        };
        let config_path = paths.config_file();
        match Config::load(&config_path) {
            Ok(config) => options.apply_config(&config),
            Err(err) => {
                if err.kind() != ErrorKind::NotFound {
                    eprintln!(
                        "warning: failed to load config from {}: {err}",
                        config_path.display()
                    );
                }
            }
        }
        options
    }

    fn apply_config(&mut self, config: &Config) {
        if let Some(raw) = config.get_in(Some("Sound"), "Sound") {
            if let Some(parsed) = parse_bool(raw) {
                self.sound_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "Music") {
            if let Some(parsed) = parse_bool(raw) {
                self.music_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MenuMusic") {
            if let Some(parsed) = parse_bool(raw) {
                self.menu_music_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MenuSound") {
            if let Some(parsed) = parse_bool(raw) {
                self.menu_sound_enabled = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MusicVolume") {
            if let Ok(value) = raw.trim().parse::<i32>() {
                let clamped = value.clamp(0, 100);
                self.music_volume = clamped as f32 / 100.0;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "SoundVolume") {
            if let Ok(value) = raw.trim().parse::<i32>() {
                let clamped = value.clamp(0, 100);
                self.sound_volume = clamped as f32 / 100.0;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "MaxChannels") {
            if let Ok(value) = raw.trim().parse::<i32>() {
                let clamped = value.clamp(1, MAX_CHANNELS_LIMIT as i32);
                self.max_channels = clamped as usize;
            }
        }
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
