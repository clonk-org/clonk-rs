use super::*;

/// C4GraphicsResource loads this 6-bit RGB palette and expands every channel
/// by `<< 2` (src/C4GraphicsResource.cpp:176-193). Typed line definitions
/// retain palette indices all the way to `C4FacetEx::DrawLine`.
const C4_GAME_PALETTE: &[u8; 256 * 3] = include_bytes!("../../../planet/Graphics.c4g/C4.PAL");

/// The per-game palette installed by `C4GraphicsResource::Init`. Source
/// bytes use C4.PAL's six-bit RGB channels; indices 0 and 191 receive the
/// engine's fixed transparency/force-field overrides after expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamePalette {
    colors: [Color; 256],
}

impl GamePalette {
    pub const BYTE_LEN: usize = 256 * 3;

    pub fn from_c4_pal(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::BYTE_LEN {
            return None;
        }
        let mut colors = [Color::opaque(0, 0, 0); 256];
        for (index, color) in colors.iter_mut().enumerate() {
            let offset = index * 3;
            *color = Color::opaque(
                bytes[offset] << 2,
                bytes[offset + 1] << 2,
                bytes[offset + 2] << 2,
            );
        }
        colors[0] = Color::transparent();
        colors[191] = Color::new(0, 0, 255, 128);
        Some(Self { colors })
    }

    pub fn color(&self, index: u8) -> Color {
        self.colors[usize::from(index)]
    }
}

impl Default for GamePalette {
    fn default() -> Self {
        Self::from_c4_pal(C4_GAME_PALETTE).expect("embedded C4.PAL has 256 RGB entries")
    }
}

/// One native material texture surface. PNG entries carry `Surface32`, while
/// legacy indexed BMP entries carry `Surface8`; C++ uses both for landscape
/// patterns but permits only `Surface32` as graphical PXS artwork.
#[derive(Clone, Debug, PartialEq)]
pub enum MaterialTextureSurface {
    Surface32(ImageData),
    Surface8 {
        width: u32,
        height: u32,
        indices: Arc<[u8]>,
    },
}

impl MaterialTextureSurface {
    pub fn surface32(image: ImageData) -> Self {
        Self::Surface32(image)
    }

    pub fn surface8(width: u32, height: u32, indices: Vec<u8>) -> Self {
        Self::Surface8 {
            width,
            height,
            indices: Arc::from(indices.into_boxed_slice()),
        }
    }

    pub fn surface32_image(&self) -> Option<&ImageData> {
        match self {
            Self::Surface32(image) => Some(image),
            Self::Surface8 { .. } => None,
        }
    }

    pub fn indexed_pixels(&self) -> Option<(u32, u32, &[u8])> {
        match self {
            Self::Surface32(_) => None,
            Self::Surface8 {
                width,
                height,
                indices,
            } => Some((*width, *height, indices)),
        }
    }
}

/// Presentation fields from one C4MaterialCore. Colors and alpha retain the
/// C++ arrays verbatim: three RGB triplets and two sets of three transparency
/// values (`0` opaque, `255` transparent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialRenderInfo {
    pub(crate) color: [u8; 9],
    pub(crate) alpha: [u8; 6],
    pub(crate) texture_overlay: Option<String>,
    overlay_type: i32,
    pub(crate) density: i32,
    /// C4MaterialCore::Placement after its load-time defaulting. ApplyLighting
    /// compares this value around each Surface8 pixel.
    pub(crate) placement: i32,
    /// Loose-material sprite sheet (`C4MaterialCore::sPXSGfx`). The sheet is
    /// resolved through the same texture map as landscape patterns.
    pub(crate) pxs_gfx: Option<String>,
    /// `C4TargetRect` [x, y, width, height, target-x, target-y].
    pub(crate) pxs_gfx_rect: [i32; 6],
    pub(crate) pxs_gfx_size: i32,
}

impl MaterialRenderInfo {
    pub fn new(
        color: [u8; 9],
        alpha: [u8; 6],
        texture_overlay: Option<String>,
        overlay_type: i32,
        density: i32,
    ) -> Self {
        let placement = if density >= 50 {
            70
        } else if density >= 25 {
            10
        } else {
            5
        };
        Self {
            color,
            alpha,
            texture_overlay,
            overlay_type,
            density,
            placement,
            pxs_gfx: None,
            pxs_gfx_rect: [0; 6],
            pxs_gfx_size: 0,
        }
    }

    pub fn with_placement(mut self, placement: i32) -> Self {
        self.placement = placement;
        self
    }

    pub fn with_pxs_graphics(
        mut self,
        texture: Option<String>,
        rect: [i32; 6],
        size: i32,
    ) -> Self {
        self.pxs_gfx = texture;
        self.pxs_gfx_rect = rect;
        self.pxs_gfx_size = size;
        self
    }
}

#[derive(Clone, Copy)]
pub(crate) struct MaterialPixel {
    pub(crate) red: u8,
    pub(crate) green: u8,
    pub(crate) blue: u8,
    pub(crate) transparency: u8,
}

fn lighten_material_channel(channel: u8) -> u8 {
    if channel & 0x80 != 0 {
        255
    } else {
        channel << 1
    }
}

pub(crate) fn apply_material_pattern(
    pixel: &mut MaterialPixel,
    pattern: &ImageData,
    x: i32,
    y: i32,
    zoom: i32,
    monochrome: bool,
) {
    let width = pattern.width();
    let height = pattern.height();
    if width == 0 || height == 0 {
        return;
    }
    let sample_x = if zoom == 0 { x } else { x / zoom };
    let sample_y = if zoom == 0 { y } else { y / zoom };
    let source = (((sample_y as u32 % height) * width + sample_x as u32 % width) * 4) as usize;
    let data = pattern.pixels();
    if source + 4 > data.len() {
        return;
    }

    let modifiers = if monochrome {
        [data[source + 2]; 3]
    } else {
        [data[source], data[source + 1], data[source + 2]]
    };
    pixel.red = lighten_material_channel(
        ((u16::from(pixel.red) * u16::from(modifiers[0])) >> 8) as u8,
    );
    pixel.green = lighten_material_channel(
        ((u16::from(pixel.green) * u16::from(modifiers[1])) >> 8) as u8,
    );
    pixel.blue = lighten_material_channel(
        ((u16::from(pixel.blue) * u16::from(modifiers[2])) >> 8) as u8,
    );
    let pattern_transparency = 255u8.saturating_sub(data[source + 3]);
    pixel.transparency = pixel
        .transparency
        .saturating_add(pattern_transparency);
}

#[derive(Clone, Copy)]
pub(crate) enum MaterialPatternRef<'a> {
    Surface32(&'a ImageData),
    Surface8 {
        width: u32,
        height: u32,
        indices: &'a [u8],
    },
}

impl<'a> From<&'a MaterialTextureSurface> for MaterialPatternRef<'a> {
    fn from(surface: &'a MaterialTextureSurface) -> Self {
        match surface {
            MaterialTextureSurface::Surface32(image) => Self::Surface32(image),
            MaterialTextureSurface::Surface8 {
                width,
                height,
                indices,
            } => Self::Surface8 {
                width: *width,
                height: *height,
                indices,
            },
        }
    }
}

fn apply_indexed_material_pattern(
    pixel: &mut MaterialPixel,
    material: &MaterialRenderInfo,
    landscape_pixel: u8,
    width: u32,
    height: u32,
    indices: &[u8],
    x: i32,
    y: i32,
    zoom: i32,
) {
    if width == 0 || height == 0 {
        return;
    }
    let sample_x = if zoom == 0 { x } else { x / zoom };
    let sample_y = if zoom == 0 { y } else { y / zoom };
    let source = (sample_y as u32 % height) as usize * width as usize
        + (sample_x as u32 % width) as usize;
    let Some(&shift) = indices.get(source) else {
        return;
    };
    let shift = usize::from(shift % 3);
    let color = shift * 3;
    pixel.red = material.color[color];
    pixel.green = material.color[color + 1];
    pixel.blue = material.color[color + 2];
    pixel.transparency = material.alpha[shift + if landscape_pixel & 0xf0 == 0 { 0 } else { 3 }];
}

fn apply_material_surface(
    pixel: &mut MaterialPixel,
    material: &MaterialRenderInfo,
    landscape_pixel: u8,
    pattern: MaterialPatternRef<'_>,
    x: i32,
    y: i32,
    zoom: i32,
    monochrome: bool,
) {
    match pattern {
        MaterialPatternRef::Surface32(pattern) => {
            apply_material_pattern(pixel, pattern, x, y, zoom, monochrome)
        }
        MaterialPatternRef::Surface8 {
            width,
            height,
            indices,
        } => apply_indexed_material_pattern(
            pixel,
            material,
            landscape_pixel,
            width,
            height,
            indices,
            x,
            y,
            zoom,
        ),
    }
}

#[cfg(test)]
std::thread_local! {
    static MATERIAL_COMPOSITION_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    // Each test thread owns its counter while scoped Rayon workers receive an
    // Arc clone from that originating thread. This retains test isolation and
    // makes increments from parallel landscape rows race-free.
    pub(crate) static LANDSCAPE_DESTINATION_SAMPLES: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
}

#[cfg(test)]
pub(crate) fn reset_material_composition_calls() {
    MATERIAL_COMPOSITION_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn material_composition_calls() -> usize {
    MATERIAL_COMPOSITION_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_landscape_destination_samples() {
    LANDSCAPE_DESTINATION_SAMPLES.with(|samples| samples.store(0, Ordering::Relaxed));
}

#[cfg(test)]
pub(crate) fn landscape_destination_samples() -> usize {
    LANDSCAPE_DESTINATION_SAMPLES.with(|samples| samples.load(Ordering::Relaxed))
}

pub(crate) fn compose_material_pixel(
    material: &MaterialRenderInfo,
    landscape_pixel: u8,
    x: i32,
    y: i32,
    texture: &ImageData,
    overlay: Option<&ImageData>,
) -> Color {
    compose_material_surface_pixel(
        material,
        landscape_pixel,
        x,
        y,
        MaterialPatternRef::Surface32(texture),
        overlay.map(MaterialPatternRef::Surface32),
    )
}

pub(crate) fn compose_material_surface_pixel(
    material: &MaterialRenderInfo,
    landscape_pixel: u8,
    x: i32,
    y: i32,
    texture: MaterialPatternRef<'_>,
    overlay: Option<MaterialPatternRef<'_>>,
) -> Color {
    #[cfg(test)]
    MATERIAL_COMPOSITION_CALLS.with(|calls| calls.set(calls.get() + 1));

    let mut pixel = MaterialPixel {
        red: material.color[0],
        green: material.color[1],
        blue: material.color[2],
        transparency: material.alpha[if landscape_pixel & 0x80 == 0 { 0 } else { 3 }],
    };
    let primary_zoom = if material.overlay_type & MATERIAL_OVERLAY_HUGE_ZOOM != 0 {
        4
    } else if material.overlay_type & MATERIAL_OVERLAY_EXACT != 0 {
        1
    } else {
        0
    };
    let monochrome = material.overlay_type & MATERIAL_OVERLAY_MONOCHROME != 0;
    apply_material_surface(
        &mut pixel,
        material,
        landscape_pixel,
        texture,
        x,
        y,
        primary_zoom,
        monochrome,
    );
    if let Some(overlay) = overlay {
        let overlay_zoom = if material.overlay_type & MATERIAL_OVERLAY_EXACT != 0 {
            1
        } else {
            2
        };
        apply_material_surface(
            &mut pixel,
            material,
            landscape_pixel,
            overlay,
            x,
            y,
            overlay_zoom,
            monochrome,
        );
    }
    Color::new(
        pixel.red,
        pixel.green,
        pixel.blue,
        255u8.saturating_sub(pixel.transparency),
    )
}

pub(crate) fn lighten_material_color(color: &mut Color, amount: i32) {
    let amount = amount.clamp(0, 255) as u8;
    color.r = color.r.saturating_add(amount);
    color.g = color.g.saturating_add(amount);
    color.b = color.b.saturating_add(amount);
}

pub(crate) fn darken_material_color(color: &mut Color, amount: i32) {
    let amount = amount.clamp(0, 255) as u8;
    color.r = color.r.saturating_sub(amount);
    color.g = color.g.saturating_sub(amount);
    color.b = color.b.saturating_sub(amount);
}
