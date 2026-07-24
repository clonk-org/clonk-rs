//! Shared helpers for dialog render tests: real Graphics.c4g assets, the real
//! Endeavour fonts, and PPM dumps that ImageMagick can diff directly against
//! the C++ engine's F9 reference screenshots.

use crate::clonk_fonts::{build_font_set, ClonkFontSet};
use crate::ImageData;
use clonk_graphics::{GammaRamp, Surface};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// Repository root (two levels above the crate).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Decodes a PNG from `planet/Graphics.c4g` into raw RGBA `ImageData`.
pub fn load_graphics_png(name: &str) -> ImageData {
    let path = repo_root().join("planet/Graphics.c4g").join(name);
    let rgba = image::open(&path)
        .unwrap_or_else(|err| panic!("decode {}: {err}", path.display()))
        .into_rgba8();
    let (w, h) = rgba.dimensions();
    ImageData::new(w, h, rgba.into_raw())
}

/// The real GUI font set, rasterized once per test binary.
pub fn endeavour_font_set() -> Arc<ClonkFontSet> {
    static FONTS: OnceLock<Arc<ClonkFontSet>> = OnceLock::new();
    Arc::clone(FONTS.get_or_init(|| {
        let path = repo_root().join("planet/System.c4g/Endeavour.ttf");
        let bytes =
            std::fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        Arc::new(build_font_set(&bytes).expect("build Endeavour font set"))
    }))
}

/// The default gamma ramp shared by all startup rendering.
pub fn standard_gamma() -> &'static GammaRamp {
    static GAMMA: OnceLock<GammaRamp> = OnceLock::new();
    GAMMA.get_or_init(GammaRamp::standard)
}

/// Writes `surface` as a binary PPM (P6, RGB) — ImageMagick reads these
/// natively: `magick out.ppm out.png`, `compare -metric AE out.ppm ref.png ...`.
pub fn write_ppm(surface: &Surface, path: impl AsRef<Path>) {
    let (w, h) = (surface.width(), surface.height());
    let mut data = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in surface.pixels().chunks_exact(4) {
        data.extend_from_slice(&px[..3]);
    }
    std::fs::write(path.as_ref(), data)
        .unwrap_or_else(|err| panic!("write {}: {err}", path.as_ref().display()));
}
