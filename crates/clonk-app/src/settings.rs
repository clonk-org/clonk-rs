use clonk_core::std_config::Config;
use clonk_frontend::startup_options_graphics::{
    MAX_GRAPHICS_SCALE_PERCENT, MIN_GRAPHICS_SCALE_PERCENT,
};
use clonk_platform::AppPaths;
use std::io::ErrorKind;

const DEFAULT_MAX_CHANNELS: usize = 1024;
const MAX_CHANNELS_LIMIT: usize = 1024;
// C++ resolution defaults (C4Config.cpp:440-441).
const DEFAULT_RES_X: u32 = 800;
const DEFAULT_RES_Y: u32 = 600;
const MIN_RESOLUTION: u32 = 1;

#[derive(Debug, Clone)]
pub struct AudioOptions {
    pub max_channels: usize,
    pub prefer_linear_resampling: bool,
    pub sound_enabled: bool,
    pub music_enabled: bool,
    pub menu_music_enabled: bool,
    pub menu_sound_enabled: bool,
    /// Initial process-local mute state for every client's `/sound` command.
    pub mute_sound_command: bool,
    pub sound_volume: f32,
    pub music_volume: f32,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            max_channels: DEFAULT_MAX_CHANNELS,
            prefer_linear_resampling: false,
            sound_enabled: true,
            music_enabled: true,
            menu_music_enabled: true,
            menu_sound_enabled: true,
            mute_sound_command: false,
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
                    tracing::warn!(
                        error = %err,
                        path = %config_path.display(),
                        "failed to load audio config"
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

        if let Some(raw) = config.get_in(Some("Sound"), "MuteSoundCommand") {
            if let Some(parsed) = parse_bool(raw) {
                self.mute_sound_command = parsed;
            }
        }

        if let Some(raw) = config.get_in(Some("Sound"), "PreferLinearResampling") {
            if let Some(parsed) = parse_bool(raw) {
                self.prefer_linear_resampling = parsed;
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

    pub(crate) fn music_volume_percent(&self) -> i32 {
        normalized_volume_percent(self.music_volume)
    }

    pub(crate) fn sound_volume_percent(&self) -> i32 {
        normalized_volume_percent(self.sound_volume)
    }

    pub(crate) fn set_music_volume_percent(&mut self, value: i32) {
        self.music_volume = normalized_volume(value);
    }

    pub(crate) fn set_sound_volume_percent(&mut self, value: i32) {
        self.sound_volume = normalized_volume(value);
    }

    /// Writes exactly the six values owned by the classic startup Sound
    /// sheet. `MaxChannels` and all unknown/extension keys deliberately stay
    /// untouched when the surrounding config is saved.
    pub(crate) fn write_startup_sound_config(&self, config: &mut Config) {
        let section = Some("Sound");
        config.set_in(section, "Sound", bool_config_value(self.sound_enabled));
        config.set_in(section, "Music", bool_config_value(self.music_enabled));
        config.set_in(
            section,
            "MenuMusic",
            bool_config_value(self.menu_music_enabled),
        );
        config.set_in(
            section,
            "MenuSound",
            bool_config_value(self.menu_sound_enabled),
        );
        config.set_in(
            section,
            "MusicVolume",
            self.music_volume_percent().to_string(),
        );
        config.set_in(
            section,
            "SoundVolume",
            self.sound_volume_percent().to_string(),
        );
    }
}

fn normalized_volume(value: i32) -> f32 {
    value.clamp(0, 100) as f32 / 100.0
}

fn normalized_volume_percent(value: f32) -> i32 {
    if value.is_finite() {
        (value.clamp(0.0, 1.0) * 100.0).round() as i32
    } else {
        0
    }
}

fn bool_config_value(value: bool) -> &'static str {
    // StdCompilerINIWrite::Boolean emits these exact spellings
    // (StdCompiler.cpp:345-349).
    if value {
        "true"
    } else {
        "false"
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Window,
    Fullscreen,
}

impl DisplayMode {
    fn from_config(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "1" | "window" | "windowed" => Some(DisplayMode::Window),
            "0" | "fullscreen" => Some(DisplayMode::Fullscreen),
            value => value.parse::<i32>().ok().and_then(|parsed| match parsed {
                0 => Some(DisplayMode::Fullscreen),
                1 => Some(DisplayMode::Window),
                _ => None,
            }),
        }
    }

    fn to_config_value(self) -> &'static str {
        match self {
            DisplayMode::Window => "1",
            DisplayMode::Fullscreen => "0",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_options_default_channel_count_matches_cpp() {
        // C4AudioSystem::MaxChannels is 1024, and C4ConfigSound::CompileFunc
        // uses it as the default (C4AudioSystem.h:103; C4Config.cpp:516).
        assert_eq!(AudioOptions::default().max_channels, 1024);
    }

    #[test]
    fn l040_audio_options_read_prefer_linear_resampling() {
        let mut config = Config::new();
        config.set_in(Some("Sound"), "PreferLinearResampling", "true");
        let mut options = AudioOptions::default();

        options.apply_config(&config);

        assert!(options.prefer_linear_resampling);
    }

    #[test]
    fn audio_options_load_sound_command_mute_without_owning_its_persistence() {
        let mut config = Config::new();
        config.set_in(Some("Sound"), "MuteSoundCommand", "true");
        let mut options = AudioOptions::default();
        options.apply_config(&config);
        assert!(options.mute_sound_command);
    }

    #[test]
    fn audio_options_sound_sheet_percent_accessors_clamp_and_round() {
        let mut options = AudioOptions::default();
        options.set_music_volume_percent(-1);
        options.set_sound_volume_percent(101);
        assert_eq!(options.music_volume_percent(), 0);
        assert_eq!(options.sound_volume_percent(), 100);

        options.music_volume = 0.555;
        options.sound_volume = f32::NAN;
        assert_eq!(options.music_volume_percent(), 56);
        assert_eq!(options.sound_volume_percent(), 0);
    }

    #[test]
    fn audio_options_write_only_the_six_classic_sound_sheet_keys() {
        let mut config = Config::new();
        config.set_in(Some("Sound"), "MaxChannels", "37");
        config.set_in(Some("Sound"), "VendorExtension", "keep-me");
        config.set_in(Some("General"), "Unrelated", "also-keep-me");
        let options = AudioOptions {
            max_channels: 999,
            prefer_linear_resampling: true,
            sound_enabled: false,
            music_enabled: true,
            menu_music_enabled: false,
            menu_sound_enabled: true,
            mute_sound_command: true,
            sound_volume: 0.27,
            music_volume: 0.83,
        };

        options.write_startup_sound_config(&mut config);

        assert_eq!(config.get_in(Some("Sound"), "Sound"), Some("false"));
        assert_eq!(config.get_in(Some("Sound"), "Music"), Some("true"));
        assert_eq!(config.get_in(Some("Sound"), "MenuMusic"), Some("false"));
        assert_eq!(config.get_in(Some("Sound"), "MenuSound"), Some("true"));
        assert_eq!(config.get_in(Some("Sound"), "MusicVolume"), Some("83"));
        assert_eq!(config.get_in(Some("Sound"), "SoundVolume"), Some("27"));
        assert_eq!(config.get_in(Some("Sound"), "MaxChannels"), Some("37"));
        assert_eq!(
            config.get_in(Some("Sound"), "MuteSoundCommand"),
            None,
            "the startup sound sheet does not own the /sound mute setting"
        );
        assert_eq!(
            config.get_in(Some("Sound"), "VendorExtension"),
            Some("keep-me")
        );
        assert_eq!(
            config.get_in(Some("General"), "Unrelated"),
            Some("also-keep-me")
        );
    }

    #[test]
    fn display_options_apply_config_parses_values() {
        // The config file is shared with the C++ engine, which names the
        // keys ResolutionX/ResolutionY (C4Config.cpp:440-441).
        let mut cfg = Config::new();
        cfg.set_in(Some("Graphics"), "ResolutionX", "1280");
        cfg.set_in(Some("Graphics"), "ResolutionY", "720");
        cfg.set_in(Some("Graphics"), "Scale", "150");
        cfg.set_in(Some("Graphics"), "PointFiltering", "true");
        cfg.set_in(Some("Graphics"), "DisplayMode", "0");
        cfg.set_in(Some("Graphics"), "Maximized", "true");
        cfg.set_in(Some("Graphics"), "PositionX", "42");
        cfg.set_in(Some("Graphics"), "PositionY", "84");

        let mut options = DisplayOptions::default();
        options.apply_config(&cfg);

        assert_eq!(options.base_width, 1280);
        assert_eq!(options.base_height, 720);
        assert!((options.scale - 1.5).abs() < f32::EPSILON);
        assert!(options.point_filtering);
        assert_eq!(options.mode, DisplayMode::Fullscreen);
        assert!(options.maximized);
        assert_eq!(options.position, Some((42, 84)));
    }

    #[test]
    fn display_options_default_resolution_matches_cpp() {
        // C4Config.cpp:440-441 defaults ResolutionX/Y to 800x600.
        let options = DisplayOptions::default();
        assert_eq!(options.base_width, 800);
        assert_eq!(options.base_height, 600);
    }

    #[test]
    fn first_run_display_scale_follows_the_monitor_density_only_without_a_config() {
        // Deliberate divergence: C++ always starts at Scale=100, so a 2x
        // panel gets an 800x600 *device pixel* window with a 14px font
        // (src/C4Config.cpp:440-441, :480). Seeding the application scale
        // from the monitor keeps the classic 800x600 logical layout while
        // giving it the panel's real pixel density — the window covers the
        // same physical area it did on a 1x display.
        let mut fresh = DisplayOptions::default();
        fresh.mark_first_run();
        assert!(fresh.apply_first_run_display_scale(2.0));
        assert_eq!(fresh.scale_percent(), 200);
        assert_eq!(fresh.base_width, 800, "the logical layout is unchanged");
        assert_eq!(fresh.base_height, 600);
        assert_eq!(fresh.actual_size(), (1600, 1200));

        // A configuration that exists on disk is the player's choice.
        let mut configured = DisplayOptions::default();
        assert!(!configured.apply_first_run_display_scale(2.0));
        assert_eq!(configured.scale_percent(), 100);

        // Fractional densities round to an integer scale: a non-integer
        // application scale routes every glyph through a bilinear resample
        // of the atlas (`requires_resampling`, clonk_fonts.rs:102-113).
        for (factor, percent) in [(1.0, 100), (1.25, 100), (1.5, 200), (2.0, 200), (3.0, 300)] {
            let mut options = DisplayOptions::default();
            options.mark_first_run();
            options.apply_first_run_display_scale(factor);
            assert_eq!(options.scale_percent(), percent, "scale factor {factor}");
        }

        // Beyond the supported range the scale clamps rather than producing
        // an unreachable Options-dialog value.
        let mut huge = DisplayOptions::default();
        huge.mark_first_run();
        huge.apply_first_run_display_scale(9.0);
        assert_eq!(huge.scale_percent(), MAX_GRAPHICS_SCALE_PERCENT);

        // Degenerate factors never disturb the default.
        for factor in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            let mut options = DisplayOptions::default();
            options.mark_first_run();
            assert!(!options.apply_first_run_display_scale(factor));
            assert_eq!(options.scale_percent(), 100);
        }
    }

    #[test]
    fn l007_display_mode_default_override_and_numeric_persistence_match_cpp() {
        let mut missing_mode = DisplayOptions::default();
        assert_eq!(missing_mode.mode, DisplayMode::Fullscreen);
        missing_mode.apply_config(&Config::new());
        assert!(
            missing_mode.dirty,
            "an absent DisplayMode is materialized by the shutdown save"
        );
        let mut persisted = Config::new();
        missing_mode.write_config(&mut persisted);
        assert_eq!(persisted.get_in(Some("Graphics"), "DisplayMode"), Some("0"));

        for raw in ["1", "Window"] {
            let mut config = Config::new();
            config.set_in(Some("Graphics"), "DisplayMode", raw);
            let mut options = DisplayOptions::default();
            options.apply_config(&config);
            assert_eq!(options.mode, DisplayMode::Window, "raw value {raw}");
            assert!(!options.dirty, "an explicit valid mode stays clean");

            let mut persisted = Config::new();
            options.write_config(&mut persisted);
            assert_eq!(persisted.get_in(Some("Graphics"), "DisplayMode"), Some("1"));
        }
    }

    #[test]
    fn display_options_preserve_unclamped_cpp_scale_percent() {
        let mut cfg = Config::new();
        cfg.set_in(Some("Graphics"), "Scale", "500");
        let mut options = DisplayOptions::default();
        options.apply_config(&cfg);
        assert!((options.scale - 5.0).abs() < f32::EPSILON);
        assert_eq!(options.checked_loader_actual_size(), Ok((4000, 3000)));
    }

    #[test]
    fn l008_scale_fifty_uses_half_size_and_survives_resize_round_trip() {
        let mut config = Config::new();
        config.set_in(Some("Graphics"), "ResolutionX", "800");
        config.set_in(Some("Graphics"), "ResolutionY", "600");
        config.set_in(Some("Graphics"), "Scale", "50");

        let mut options = DisplayOptions::default();
        options.apply_config(&config);
        assert_eq!(options.scale_percent(), 50);
        assert_eq!(options.scale, 0.5);
        assert_eq!(options.actual_size(), (400, 300));
        assert_eq!(options.checked_loader_actual_size(), Ok((400, 300)));

        options.record_actual_size(401, 301);
        assert_eq!((options.base_width, options.base_height), (802, 602));
        assert_eq!(options.scale_percent(), 50);

        let mut persisted = Config::new();
        options.write_config(&mut persisted);
        assert_eq!(persisted.get_in(Some("Graphics"), "Scale"), Some("50"));

        let mut reloaded = DisplayOptions::default();
        reloaded.apply_config(&persisted);
        assert_eq!(reloaded.scale_percent(), 50);
        assert_eq!(reloaded.actual_size(), (401, 301));
    }

    #[test]
    fn loader_scale_validation_accepts_fractional_and_rejects_nonpositive_or_overflow() {
        for percent in [0, -100] {
            let mut cfg = Config::new();
            cfg.set_in(Some("Graphics"), "Scale", percent.to_string());
            let mut options = DisplayOptions::default();
            options.apply_config(&cfg);
            assert!(options.checked_loader_actual_size().is_err());
        }

        let mut fractional = Config::new();
        fractional.set_in(Some("Graphics"), "ResolutionX", "320");
        fractional.set_in(Some("Graphics"), "ResolutionY", "200");
        fractional.set_in(Some("Graphics"), "Scale", "150");
        let mut options = DisplayOptions::default();
        options.apply_config(&fractional);
        assert_eq!(options.checked_loader_actual_size(), Ok((480, 300)));

        let mut tiny = Config::new();
        tiny.set_in(Some("Graphics"), "ResolutionX", "1");
        tiny.set_in(Some("Graphics"), "ResolutionY", "1");
        tiny.set_in(Some("Graphics"), "Scale", "1");
        let mut options = DisplayOptions::default();
        options.apply_config(&tiny);
        assert_eq!(options.checked_loader_actual_size(), Ok((1, 1)));

        let mut cfg = Config::new();
        cfg.set_in(Some("Graphics"), "ResolutionX", i32::MAX.to_string());
        cfg.set_in(Some("Graphics"), "Scale", "300");
        let mut options = DisplayOptions::default();
        options.apply_config(&cfg);
        assert!(options
            .checked_loader_actual_size()
            .expect_err("output width must be checked")
            .contains("overflows"));
    }

    #[test]
    fn display_options_persist_writes_cpp_key_names() {
        let mut cfg = Config::new();
        let options = DisplayOptions {
            base_width: 1371,
            base_height: 858,
            scale: 3.0,
            scale_percent: 300,
            point_filtering: true,
            mode: DisplayMode::Window,
            maximized: false,
            position: None,
            dirty: true,
            first_run: false,
        };
        options.write_config(&mut cfg);
        assert_eq!(cfg.get_in(Some("Graphics"), "ResolutionX"), Some("1371"));
        assert_eq!(cfg.get_in(Some("Graphics"), "ResolutionY"), Some("858"));
        assert_eq!(cfg.get_in(Some("Graphics"), "Scale"), Some("300"));
        assert_eq!(cfg.get_in(Some("Graphics"), "PointFiltering"), Some("true"));
    }

    #[test]
    fn display_options_record_actual_size_updates_base_resolution() {
        let mut options = DisplayOptions {
            base_width: 800,
            base_height: 600,
            mode: DisplayMode::Window,
            ..Default::default()
        };
        options.record_actual_size(1024, 768);
        assert_eq!(options.base_width, 1024);
        assert_eq!(options.base_height, 768);
        assert!(options.dirty);
        options.dirty = false;
        // Should not update when values identical
        options.record_actual_size(1024, 768);
        assert!(!options.dirty);
    }

    #[test]
    fn display_options_record_actual_size_divides_by_scale_with_ceil() {
        // C4Application::SetResolution stores ceil(pixels / scale)
        // (C4Application.cpp:536-538).
        let mut options = DisplayOptions {
            base_width: 800,
            base_height: 600,
            scale: 3.0,
            scale_percent: 300,
            mode: DisplayMode::Window,
            ..Default::default()
        };
        options.record_actual_size(2743, 1717);
        assert_eq!(options.base_width, 915);
        assert_eq!(options.base_height, 573);
    }

    #[test]
    fn accepted_scale_retains_physical_size_and_recomputes_base_resolution() {
        let mut options = DisplayOptions::default();
        options.record_scale_percent(200, 1_280, 720);
        assert_eq!(options.scale_percent(), 200);
        assert!((options.scale - 2.0).abs() < f32::EPSILON);
        assert_eq!((options.base_width, options.base_height), (640, 360));
        assert_eq!(options.actual_size(), (1_280, 720));
        assert!(options.dirty);
    }
}

#[derive(Debug, Clone)]
pub struct DisplayOptions {
    base_width: u32,
    base_height: u32,
    pub scale: f32,
    scale_percent: i32,
    pub point_filtering: bool,
    pub mode: DisplayMode,
    pub maximized: bool,
    pub position: Option<(i32, i32)>,
    dirty: bool,
    first_run: bool,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            base_width: DEFAULT_RES_X,
            base_height: DEFAULT_RES_Y,
            scale: 1.0,
            scale_percent: 100,
            point_filtering: false,
            mode: DisplayMode::Fullscreen,
            maximized: false,
            position: None,
            dirty: false,
            first_run: false,
        }
    }
}

impl DisplayOptions {
    pub fn load(paths: Option<&AppPaths>) -> Self {
        let mut options = Self::default();
        let Some(paths) = paths else {
            return options;
        };
        let config_path = paths.config_file();
        match Config::load(&config_path) {
            Ok(config) => options.apply_config(&config),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                // C4Application saves the freshly defaulted configuration
                // during startup. Keep the missing-file case dirty so the
                // normal shutdown path persists the native fullscreen mode.
                options.dirty = true;
                options.first_run = true;
            }
            Err(err) => tracing::warn!(
                error = %err,
                path = %config_path.display(),
                "failed to load display config"
            ),
        }
        options
    }

    /// Marks a configuration as never having been written to disk.
    #[cfg(test)]
    pub fn mark_first_run(&mut self) {
        self.first_run = true;
    }

    /// Seeds the application scale from the display's own pixel density, but
    /// only when no configuration file exists yet.
    ///
    /// Deliberate divergence: C++ starts every install at `Scale=100`
    /// (src/C4Config.cpp:480), which on a 2x panel means an 800x600 *device
    /// pixel* window and a 14px font. Because `Scale` divides the physical
    /// extent into the logical layout
    /// (`logical_size_for`, crates/clonk-scaling/src/lib.rs:12-17), seeding it
    /// from the monitor keeps the classic 800x600 logical layout and simply
    /// gives it the panel's real pixel density. The scale is rounded to an
    /// integer: a fractional application scale sends every glyph through a
    /// bilinear resample of the native atlas
    /// (`requires_resampling`, crates/clonk-frontend/src/clonk_fonts.rs:102-113).
    ///
    /// Returns whether the scale was changed.
    pub fn apply_first_run_display_scale(&mut self, scale_factor: f64) -> bool {
        if !self.first_run || !scale_factor.is_finite() || scale_factor < 1.0 {
            return false;
        }
        let percent = ((scale_factor.round() as i32).saturating_mul(100))
            .clamp(MIN_GRAPHICS_SCALE_PERCENT, MAX_GRAPHICS_SCALE_PERCENT);
        if percent == self.scale_percent {
            return false;
        }
        self.scale_percent = percent;
        self.scale = percent as f32 / 100.0;
        self.dirty = true;
        true
    }

    /// Window size in output pixels: ResX*Scale truncated like the C++
    /// window setup (C4Application.cpp:183).
    pub fn actual_size(&self) -> (u32, u32) {
        let min = MIN_RESOLUTION as f32;
        let width = ((self.base_width as f32) * self.scale).max(min);
        let height = ((self.base_height as f32) * self.scale).max(min);
        (width as u32, height as u32)
    }

    pub const fn scale_percent(&self) -> i32 {
        self.scale_percent
    }

    /// Commits an accepted application scale while retaining the current
    /// physical window size. C++ recomputes ResolutionX/Y from the physical
    /// extent at the new scale before saving the confirmed value.
    pub fn record_scale_percent(
        &mut self,
        percent: i32,
        physical_width: u32,
        physical_height: u32,
    ) {
        let percent = percent.clamp(1, 10_000);
        let scale = percent as f32 / 100.0;
        if self.scale_percent != percent {
            self.scale_percent = percent;
            self.scale = scale;
            self.dirty = true;
        }
        self.record_actual_size(physical_width, physical_height);
    }

    pub fn checked_loader_actual_size(&self) -> Result<(u32, u32), String> {
        if self.scale_percent <= 0 {
            return Err(format!(
                "classic loader application scale must be positive, got {}%",
                self.scale_percent
            ));
        }
        let percent = u64::try_from(self.scale_percent)
            .map_err(|_| "classic loader application scale is out of range".to_string())?;
        let scaled = |extent: u32, axis: &str| {
            u64::from(extent)
                .checked_mul(percent)
                .map(|value| (value / 100).max(u64::from(MIN_RESOLUTION)))
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    format!(
                        "classic loader output {axis} overflows: {extent} * {}%",
                        self.scale_percent
                    )
                })
        };
        let width = scaled(self.base_width, "width").map_err(|_| {
            format!(
                "classic loader output width overflows: {} * {}%",
                self.base_width, self.scale_percent
            )
        })?;
        let height = scaled(self.base_height, "height")?;
        Ok((width, height))
    }

    /// Stores ceil(pixels / scale) like C4Application::SetResolution
    /// (C4Application.cpp:536-538).
    pub fn record_actual_size(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let scale = self.scale.max(f32::EPSILON);
        let min = MIN_RESOLUTION as f32;
        let max = u32::MAX as f32;
        let new_width = ((width as f32) / scale).ceil().clamp(min, max);
        let new_height = ((height as f32) / scale).ceil().clamp(min, max);
        let new_width = new_width as u32;
        let new_height = new_height as u32;
        if new_width != self.base_width || new_height != self.base_height {
            self.base_width = new_width;
            self.base_height = new_height;
            self.dirty = true;
        }
    }

    pub fn record_position(&mut self, x: i32, y: i32) {
        let new_pos = Some((x, y));
        if self.position != new_pos {
            self.position = new_pos;
            self.dirty = true;
        }
    }

    pub fn record_maximized(&mut self, maximized: bool) {
        if self.maximized != maximized {
            self.maximized = maximized;
            self.dirty = true;
        }
    }

    pub fn record_mode(&mut self, mode: DisplayMode) {
        if self.mode != mode {
            self.mode = mode;
            self.dirty = true;
        }
    }

    pub fn persist_if_dirty(&mut self, paths: &AppPaths) {
        if !self.dirty {
            return;
        }
        let config_path = paths.config_file();
        let mut config = match Config::load(&config_path) {
            Ok(config) => config,
            Err(err) if err.kind() == ErrorKind::NotFound => Config::new(),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to load config for saving display options"
                );
                return;
            }
        };
        self.write_config(&mut config);
        if let Err(err) =
            crate::save_config_preserving_native_general_booleans(&config, &config_path, None, None)
        {
            tracing::warn!(
                error = %err,
                path = %config_path.display(),
                "failed to persist display settings"
            );
            return;
        }
        self.dirty = false;
    }

    /// Writes the C++ engine's key names (C4Config.cpp:440-442); the config
    /// file is shared with the C++ install.
    fn write_config(&self, config: &mut Config) {
        config.set_in(Some("Graphics"), "ResolutionX", self.base_width.to_string());
        config.set_in(
            Some("Graphics"),
            "ResolutionY",
            self.base_height.to_string(),
        );
        config.set_in(Some("Graphics"), "Scale", self.scale_percent.to_string());
        config.set_in(
            Some("Graphics"),
            "PointFiltering",
            if self.point_filtering {
                "true"
            } else {
                "false"
            },
        );
        config.set_in(Some("Graphics"), "DisplayMode", self.mode.to_config_value());
        config.set_in(
            Some("Graphics"),
            "Maximized",
            if self.maximized { "true" } else { "false" },
        );
        if let Some((x, y)) = self.position {
            config.set_in(Some("Graphics"), "PositionX", x.to_string());
            config.set_in(Some("Graphics"), "PositionY", y.to_string());
        }
    }

    fn apply_config(&mut self, config: &Config) {
        if let Some(raw) = config.get_in(Some("Graphics"), "ResolutionX") {
            if let Ok(parsed) = raw.trim().parse::<i32>() {
                if parsed > 0 {
                    self.base_width = parsed as u32;
                }
            }
        }
        if let Some(raw) = config.get_in(Some("Graphics"), "ResolutionY") {
            if let Ok(parsed) = raw.trim().parse::<i32>() {
                if parsed > 0 {
                    self.base_height = parsed as u32;
                }
            }
        }
        if let Some(raw) = config.get_in(Some("Graphics"), "Scale") {
            if let Ok(percent) = raw.trim().parse::<i32>() {
                self.scale_percent = percent;
                self.scale = (percent as f32) / 100.0;
            }
        }
        if let Some(raw) = config.get_in(Some("Graphics"), "PointFiltering") {
            if let Some(parsed) = parse_bool(raw) {
                self.point_filtering = parsed;
            }
        }
        match config.get_in(Some("Graphics"), "DisplayMode") {
            Some(raw) => {
                if let Some(mode) = DisplayMode::from_config(raw) {
                    self.mode = mode;
                }
            }
            // The classic startup save materializes an absent enum default.
            None => self.dirty = true,
        }
        if let Some(raw) = config.get_in(Some("Graphics"), "Maximized") {
            if let Some(parsed) = parse_bool(raw) {
                self.maximized = parsed;
            }
        }
        let pos_x = config
            .get_in(Some("Graphics"), "PositionX")
            .and_then(|raw| raw.trim().parse::<i32>().ok());
        let pos_y = config
            .get_in(Some("Graphics"), "PositionY")
            .and_then(|raw| raw.trim().parse::<i32>().ok());
        if let (Some(x), Some(y)) = (pos_x, pos_y) {
            self.position = Some((x, y));
        }
    }
}
