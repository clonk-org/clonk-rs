use crate::math::{self, C4Fixed};
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
    /// Raw `C4Sky::BackClr`, retained even while `back_color` is disabled.
    /// C++ persists this independently from `BackClrEnabled`
    /// (C4Sky.cpp:246-258).
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub back_color_raw: u32,
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
            back_color_raw: 0,
        }
    }
}

fn u32_is_zero(value: &u32) -> bool {
    *value == 0
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
    /// Float projections of the fixed scroll position (renderer
    /// convenience; C4Sky::Draw consumes `fixtoi(x)`, C4Sky.cpp:215-216).
    pub offset_x: f32,
    pub offset_y: f32,
    /// Raw C4Fixed `[x, y, xdir, ydir]` — the exact 16.16 bits
    /// C4Sky::CompileFunc persists via `mkCastIntAdapt` (C4Sky.cpp:248-251).
    /// `None` only in pre-fixed-point recordings.
    #[serde(default)]
    pub fixed: Option<[i32; 4]>,
}

/// `SkyPar_KEEP` (C4Script.cpp:4955): the magic int scripts pass to
/// SetSkyParallax to preserve a parameter slot.
pub const SKY_PAR_KEEP: i32 = -163764;

/// The two raw values exposed by `GetSkyAdjust`. `BackClrEnabled` remains
/// represented by `SkySettings::back_color`; it does not affect this pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SkyAdjustment {
    pub modulation: u32,
    pub back_color: u32,
}

impl SkyAdjustment {
    pub(crate) fn from_settings(settings: &SkySettings) -> Self {
        Self {
            // C4Sky::Default initializes Modulation to RGB(255,255,255)
            // and BackClr to zero (C4Sky.cpp:154-164).
            modulation: settings.modulation.unwrap_or(0x00ff_ffff),
            // `back_color` is the compatibility representation used by old
            // snapshots. Prefer the new raw slot, falling back only when it
            // is absent/defaulted.
            back_color: if settings.back_color_raw == 0 {
                settings.back_color.unwrap_or(0)
            } else {
                settings.back_color_raw
            },
        }
    }

    /// `GetClrModulation` (StdColors.h:245-278): derive the packed sky
    /// modulation/back-mask pair that transforms `source` into `target`.
    pub(crate) fn from_color_modulation(source: RgbColor, target: RgbColor) -> Self {
        let diff_r = i32::from(target.r) - i32::from(source.r);
        let diff_g = i32::from(target.g) - i32::from(source.g);
        let diff_b = i32::from(target.b) - i32::from(source.b);
        let alpha = diff_r.max(diff_g).max(diff_b).max(0);

        let combine = |source: u8, target: u8| -> u32 {
            (u32::from(target) * 256 / u32::from(source).max(1)).min(0xff)
        };
        let modulation = ((alpha as u32) << 24)
            | (combine(source.r, target.r) << 16)
            | (combine(source.g, target.g) << 8)
            | combine(source.b, target.b);

        let back_color = if alpha > 0 {
            let back = |base: u8, component_diff: i32| -> u32 {
                (i32::from(base) + component_diff * 0xff / alpha) as u8 as u32
            };
            // Current C++ starts green from dstG rather than srcG. Preserve
            // that observable legacy quirk exactly (StdColors.h:262).
            (back(source.r, diff_r) << 16) | (back(target.g, diff_g) << 8) | back(source.b, diff_b)
        } else {
            // Native `back` is uninitialized in this branch, but modulation
            // alpha zero disables the back fill. Use a deterministic raw zero.
            0
        };

        Self {
            modulation,
            back_color,
        }
    }
}

impl Default for SkyAdjustment {
    fn default() -> Self {
        Self {
            modulation: 0x00ff_ffff,
            back_color: 0,
        }
    }
}

/// The C4Sky scroll state (C4Sky.h): position and per-frame speed as
/// C4Fixed, advanced by `C4Sky::Execute` (C4Sky.cpp:193-204).
#[derive(Debug, Clone)]
pub struct SkyState {
    settings: SkySettings,
    x: C4Fixed,
    y: C4Fixed,
    xdir: C4Fixed,
    ydir: C4Fixed,
}

impl SkyState {
    pub fn new(mut settings: SkySettings) -> Self {
        // C4Sky::Init resets x = y = xdir = ydir = 0 (C4Sky.cpp:79); the
        // fixture-world `base_xdir`/`base_ydir` extension seeds the
        // initial speed (C++ only sets xdir/ydir via script
        // SetSkyParallax, itofix ints — ftofix keeps integral seeds exact).
        settings.back_color_raw = SkyAdjustment::from_settings(&settings).back_color;
        let xdir = math::ftofix(settings.base_xdir);
        let ydir = math::ftofix(settings.base_ydir);
        Self {
            settings,
            x: C4Fixed::ZERO,
            y: C4Fixed::ZERO,
            xdir,
            ydir,
        }
    }

    /// Rebuild the exact scroll state from a snapshot frame —
    /// C4Sky::CompileFunc's load half (C4Sky.cpp:248-251); float-only
    /// legacy frames fall back to the ftofix projections.
    pub fn from_frame(frame: &SkyFrame) -> Self {
        let [x, y, xdir, ydir] = frame.fixed.unwrap_or([
            math::ftofix(frame.offset_x).val(),
            math::ftofix(frame.offset_y).val(),
            math::ftofix(frame.settings.base_xdir).val(),
            math::ftofix(frame.settings.base_ydir).val(),
        ]);
        let mut settings = frame.settings.clone();
        settings.back_color_raw = SkyAdjustment::from_settings(&settings).back_color;
        Self {
            settings,
            x: C4Fixed::from_raw(x),
            y: C4Fixed::from_raw(y),
            xdir: C4Fixed::from_raw(xdir),
            ydir: C4Fixed::from_raw(ydir),
        }
    }

    pub fn settings(&self) -> &SkySettings {
        &self.settings
    }

    pub(crate) fn adjustment(&self) -> SkyAdjustment {
        SkyAdjustment::from_settings(&self.settings)
    }

    /// `C4Sky::Execute` (C4Sky.cpp:193-204): no advance without a surface;
    /// the position moves by the PREVIOUS frame's speed; each axis wraps
    /// by a single subtraction only at `>= itofix(size)` (never upward);
    /// wind mode refreshes xdir to `FIXED100(Wind)` AFTER the move.
    pub fn advance(&mut self, environment: &EnvironmentSettings) {
        if !self.settings.has_surface {
            return;
        }
        self.x += self.xdir;
        self.y += self.ydir;
        let width = math::itofix(self.settings.width as i32);
        if self.x >= width {
            self.x -= width;
        }
        let height = math::itofix(self.settings.height as i32);
        if self.y >= height {
            self.y -= height;
        }
        if matches!(self.settings.parallax_mode, SkyParallaxMode::Wind) {
            self.xdir = math::fixed100(environment.wind);
        }
    }

    /// FnSetSkyParallax (C4Script.cpp:4955-4970): each slot applies unless
    /// it holds `SkyPar_KEEP`; the mode assigns only inside 0..1 (script
    /// can never reach the settings-level "both axes" preset); a ZERO
    /// ParX/ParY is ignored (they divide in Draw); xdir/ydir/x/y assign
    /// `itofix(int)`.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_parallax(
        &mut self,
        mode: i32,
        par_x: i32,
        par_y: i32,
        xdir: i32,
        ydir: i32,
        x: i32,
        y: i32,
    ) {
        if mode != SKY_PAR_KEEP {
            match mode {
                0 => self.settings.parallax_mode = SkyParallaxMode::Fixed,
                1 => self.settings.parallax_mode = SkyParallaxMode::Wind,
                _ => {}
            }
        }
        if par_x != SKY_PAR_KEEP && par_x != 0 {
            self.settings.parallax_x = par_x;
        }
        if par_y != SKY_PAR_KEEP && par_y != 0 {
            self.settings.parallax_y = par_y;
        }
        if xdir != SKY_PAR_KEEP {
            self.xdir = math::itofix(xdir);
        }
        if ydir != SKY_PAR_KEEP {
            self.ydir = math::itofix(ydir);
        }
        if x != SKY_PAR_KEEP {
            self.x = math::itofix(x);
        }
        if y != SKY_PAR_KEEP {
            self.y = math::itofix(y);
        }
    }

    /// FnSetSkyAdjust -> C4Sky::SetModulation (C4Sky.cpp:238-244):
    /// `Modulation = dwWithClr; BackClr = dwBackClr; BackClrEnabled =
    /// (Modulation >> 24) != 0` — the `Option` on back_color models the
    /// enable flag while `back_color_raw` retains the independently readable
    /// and persisted value.
    pub fn apply_modulation(&mut self, modulation: u32, back_color: u32) {
        self.settings.modulation = Some(modulation);
        self.settings.back_color_raw = back_color;
        self.settings.back_color = (modulation >> 24 != 0).then_some(back_color);
    }

    pub fn snapshot(&self) -> SkyFrame {
        SkyFrame {
            settings: self.settings.clone(),
            offset_x: math::fixtof(self.x),
            offset_y: math::fixtof(self.y),
            fixed: Some([self.x.val(), self.y.val(), self.xdir.val(), self.ydir.val()]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{fixed100, itofix};

    fn surface_settings(mode: SkyParallaxMode) -> SkySettings {
        let mut settings = SkySettings::default().with_surface(128, 128);
        settings.parallax_mode = mode;
        settings
    }

    // C4Sky::Execute (C4Sky.cpp:193-204):
    //   if (!Surface) return;
    //   x += xdir; y += ydir;
    //   if (x >= itofix(Width))  x -= itofix(Width);
    //   if (y >= itofix(Height)) y -= itofix(Height);
    //   if (ParallaxMode == C4SkyPM_Wind) xdir = FIXED100(Game.Weather.Wind);

    #[test]
    fn sky_does_not_scroll_without_a_surface() {
        // `if (!Surface) return;` (C4Sky.cpp:196) — a fade-only sky never
        // advances, not even the wind-mode xdir refresh.
        let mut settings = SkySettings::default();
        settings.parallax_mode = SkyParallaxMode::Wind;
        settings.base_xdir = 1.0;
        let mut sky = SkyState::new(settings);
        let environment = EnvironmentSettings::new(100);
        sky.advance(&environment);
        let frame = sky.snapshot();
        assert_eq!(frame.fixed, Some([0, 0, itofix(1).val(), 0]));
    }

    #[test]
    fn wind_mode_updates_xdir_after_the_position_advance() {
        // The position moves with the PREVIOUS frame's xdir (C4Sky.cpp:198)
        // — the wind refresh happens at the END of Execute (:203), so the
        // first frame after init moves by the initial xdir of 0.
        let mut sky = SkyState::new(surface_settings(SkyParallaxMode::Wind));
        let environment = EnvironmentSettings::new(50);
        sky.advance(&environment);
        let frame = sky.snapshot();
        assert_eq!(frame.fixed, Some([0, 0, fixed100(50).val(), 0]));
        sky.advance(&environment);
        let frame = sky.snapshot();
        assert_eq!(
            frame.fixed,
            Some([fixed100(50).val(), 0, fixed100(50).val(), 0])
        );
    }

    #[test]
    fn wind_xdir_is_the_truncated_fixed100_quotient() {
        // FIXED100(33) (Fixed.h:231 -> :74-78): 33*655 + (33*36)/100 =
        // 21626 raw — NOT the f32 wind/100 projection.
        let mut sky = SkyState::new(surface_settings(SkyParallaxMode::Wind));
        let environment = EnvironmentSettings::new(33);
        sky.advance(&environment);
        let frame = sky.snapshot();
        assert_eq!(frame.fixed.map(|fixed| fixed[2]), Some(21626));
    }

    #[test]
    fn wrap_subtracts_width_once_at_the_boundary() {
        // `if (x >= itofix(Width)) x -= itofix(Width)` (C4Sky.cpp:200): a
        // single subtraction exactly at the bound.
        let mut settings = surface_settings(SkyParallaxMode::Fixed);
        settings.base_xdir = 64.0;
        let mut sky = SkyState::new(settings);
        let environment = EnvironmentSettings::new(0);
        sky.advance(&environment);
        assert_eq!(
            sky.snapshot().fixed.map(|fixed| fixed[0]),
            Some(itofix(64).val())
        );
        sky.advance(&environment);
        assert_eq!(sky.snapshot().fixed.map(|fixed| fixed[0]), Some(0));
        sky.advance(&environment);
        assert_eq!(
            sky.snapshot().fixed.map(|fixed| fixed[0]),
            Some(itofix(64).val())
        );
    }

    #[test]
    fn negative_scroll_never_wraps_upward() {
        // C++ only wraps on `x >= itofix(Width)` — a negative xdir walks x
        // below zero forever (C4Sky.cpp:200 has no lower bound).
        let mut settings = surface_settings(SkyParallaxMode::Fixed);
        settings.base_xdir = -1.0;
        let mut sky = SkyState::new(settings);
        let environment = EnvironmentSettings::new(0);
        sky.advance(&environment);
        sky.advance(&environment);
        assert_eq!(
            sky.snapshot().fixed.map(|fixed| fixed[0]),
            Some(itofix(-2).val())
        );
    }

    #[test]
    fn set_parallax_follows_fnsetskyparallax_keep_and_zero_rules() {
        // FnSetSkyParallax (C4Script.cpp:4955-4970): SkyPar_KEEP (-163764)
        // preserves a slot; the mode assigns only inside 0..1; a ZERO
        // ParX/ParY is ignored (divisor protection); xdir/ydir/x/y assign
        // itofix(int) — C++ nil args arrive as 0 and thus ZERO them.
        let mut sky = SkyState::new(surface_settings(SkyParallaxMode::Wind));
        sky.apply_parallax(0, 0, 15, 3, -2, 7, 9);
        let frame = sky.snapshot();
        assert_eq!(frame.settings.parallax_mode, SkyParallaxMode::Fixed);
        assert_eq!(
            frame.settings.parallax_x, 10,
            "zero ParX keeps the previous divisor"
        );
        assert_eq!(frame.settings.parallax_y, 15);
        assert_eq!(
            frame.fixed,
            Some([
                itofix(7).val(),
                itofix(9).val(),
                itofix(3).val(),
                itofix(-2).val(),
            ])
        );

        sky.apply_parallax(
            2,
            SKY_PAR_KEEP,
            SKY_PAR_KEEP,
            SKY_PAR_KEEP,
            SKY_PAR_KEEP,
            SKY_PAR_KEEP,
            SKY_PAR_KEEP,
        );
        let frame = sky.snapshot();
        assert_eq!(
            frame.settings.parallax_mode,
            SkyParallaxMode::Fixed,
            "mode outside 0..1 is ignored (Inside gate)"
        );
        assert_eq!(
            frame.fixed,
            Some([
                itofix(7).val(),
                itofix(9).val(),
                itofix(3).val(),
                itofix(-2).val(),
            ]),
            "SkyPar_KEEP preserves every scroll slot"
        );
    }

    #[test]
    fn set_sky_adjust_models_c4sky_set_modulation() {
        // C4Sky::SetModulation (C4Sky.cpp:238-244): Modulation and BackClr
        // assign; BackClrEnabled = (Modulation >> 24) != 0 — a modulation
        // without an alpha byte disables the background fill.
        let mut sky = SkyState::new(surface_settings(SkyParallaxMode::Fixed));
        sky.apply_modulation(0x80ffffff, 0x40c4ff);
        let frame = sky.snapshot();
        assert_eq!(frame.settings.modulation, Some(0x80ffffff));
        assert_eq!(frame.settings.back_color, Some(0x40c4ff));

        sky.apply_modulation(0x00ffffff, 0x123456);
        let frame = sky.snapshot();
        assert_eq!(frame.settings.modulation, Some(0x00ffffff));
        assert_eq!(
            frame.settings.back_color, None,
            "alpha-less modulation disables the back fill"
        );
        assert_eq!(
            frame.settings.back_color_raw, 0x123456,
            "disabled BackClr remains readable and persisted"
        );
    }

    #[test]
    fn snapshot_round_trips_the_fixed_scroll_state() {
        // C4Sky::CompileFunc persists x/y/xdir/ydir as the raw 16.16 bits
        // (mkCastIntAdapt, C4Sky.cpp:248-251); a restored sky must resume
        // from the exact fixed state.
        let mut sky = SkyState::new(surface_settings(SkyParallaxMode::Wind));
        let environment = EnvironmentSettings::new(33);
        sky.advance(&environment);
        sky.advance(&environment);
        let frame = sky.snapshot();
        let restored = SkyState::from_frame(&frame);
        assert_eq!(restored.snapshot(), frame);
    }
}
