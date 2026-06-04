use crate::{EnvironmentSettings, RgbColor};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SkyParallaxMode {
    #[default]
    Fixed,
    Wind,
    Parallax,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkySettings {
    pub has_surface: bool,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub parallax_mode: SkyParallaxMode,
    #[serde(default = "default_parallax")]
    pub parallax_x: i32,
    #[serde(default = "default_parallax")]
    pub parallax_y: i32,
    #[serde(default)]
    pub base_xdir: f32,
    #[serde(default)]
    pub base_ydir: f32,
    #[serde(default = "default_fade_top")]
    pub fade_top: RgbColor,
    #[serde(default = "default_fade_bottom")]
    pub fade_bottom: RgbColor,
    #[serde(default)]
    pub modulation: Option<u32>,
    #[serde(default)]
    pub back_color: Option<u32>,
}

impl SkySettings {
    pub fn with_surface(mut self, width: u32, height: u32) -> Self {
        self.has_surface = true;
        self.width = width;
        self.height = height;
        self
    }
}

impl Default for SkySettings {
    fn default() -> Self {
        Self {
            has_surface: false,
            width: 0,
            height: 0,
            parallax_mode: SkyParallaxMode::Fixed,
            parallax_x: default_parallax(),
            parallax_y: default_parallax(),
            base_xdir: 0.0,
            base_ydir: 0.0,
            fade_top: default_fade_top(),
            fade_bottom: default_fade_bottom(),
            modulation: None,
            back_color: None,
        }
    }
}

fn default_parallax() -> i32 {
    10
}

fn default_fade_top() -> RgbColor {
    RgbColor::new(16, 24, 48)
}

fn default_fade_bottom() -> RgbColor {
    RgbColor::new(96, 128, 192)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkyFrame {
    pub settings: SkySettings,
    pub offset_x: f32,
    pub offset_y: f32,
}

#[derive(Debug, Clone)]
pub struct SkyState {
    settings: SkySettings,
    offset_x: f32,
    offset_y: f32,
}

impl SkyState {
    pub fn new(settings: SkySettings) -> Self {
        Self {
            settings,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    pub fn settings(&self) -> &SkySettings {
        &self.settings
    }

    pub fn advance(&mut self, environment: &EnvironmentSettings) {
        let mut x_velocity = self.settings.base_xdir;
        if matches!(self.settings.parallax_mode, SkyParallaxMode::Wind) {
            x_velocity = environment.wind as f32 / 100.0;
        }
        self.offset_x += x_velocity;
        self.offset_y += self.settings.base_ydir;

        if self.settings.has_surface {
            self.offset_x = wrap_offset(self.offset_x, self.settings.width);
            self.offset_y = wrap_offset(self.offset_y, self.settings.height);
        }
    }

    pub fn snapshot(&self) -> SkyFrame {
        SkyFrame {
            settings: self.settings.clone(),
            offset_x: self.offset_x,
            offset_y: self.offset_y,
        }
    }
}

fn wrap_offset(value: f32, dimension: u32) -> f32 {
    if dimension == 0 {
        return 0.0;
    }
    let span = dimension as f32;
    let mut wrapped = value % span;
    if wrapped < 0.0 {
        wrapped += span;
    }
    wrapped
}
