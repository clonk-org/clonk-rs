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

    pub fn with_pxs_graphics(mut self, texture: Option<String>, rect: [i32; 6], size: i32) -> Self {
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
    pixel.red =
        lighten_material_channel(((u16::from(pixel.red) * u16::from(modifiers[0])) >> 8) as u8);
    pixel.green =
        lighten_material_channel(((u16::from(pixel.green) * u16::from(modifiers[1])) >> 8) as u8);
    pixel.blue =
        lighten_material_channel(((u16::from(pixel.blue) * u16::from(modifiers[2])) >> 8) as u8);
    let pattern_transparency = 255u8.saturating_sub(data[source + 3]);
    pixel.transparency = pixel.transparency.saturating_add(pattern_transparency);
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
    let source =
        (sample_y as u32 % height) as usize * width as usize + (sample_x as u32 % width) as usize;
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

#[cfg(test)]
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

/// The tools dialog's material preview swatch.
///
/// `C4ToolsDlg::UpdatePreview` (`C4ToolsDlg.cpp:601-708`) builds a surface and
/// draws one thing into it:
/// `DrawPatternedCircle(surface, w / 2, h / 2, Grade, bCol, Pattern1, Pattern2, pal)`
/// — a disc whose radius **is the grade**, which is what makes the swatch show
/// the brush about to be painted with rather than only its colour.
///
/// Two details are carried over deliberately:
///
/// - `DrawPatternedCircle` (`StdDDraw2.cpp:1191-1207`) runs `ycnt` over
///   `-r..r` and `xcnt` over `x - lwdt..x + lwdt`, both **exclusive** at the
///   top, so the disc is a pixel short on the right and bottom. Rounding that
///   out would make the preview disagree with the reference build.
/// - The preview applies **`Pattern1` then `Pattern2`** — the material overlay
///   *before* the texture — while `C4Landscape::GetClrByTex` applies them the
///   other way round (`C4Landscape.cpp:2629-2633`). The two call sites really
///   do differ, so this cannot reuse
///   [`compose_material_surface_pixel`]: swapping its arguments would swap the
///   per-role zooms with them.
pub fn material_preview_swatch(
    width: u32,
    height: u32,
    grade: i32,
    material: &MaterialRenderInfo,
    texture: &MaterialTextureSurface,
    overlay: Option<&MaterialTextureSurface>,
    background: Color,
) -> ImageData {
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for _ in 0..(width as usize) * (height as usize) {
        pixels.extend_from_slice(&[background.r, background.g, background.b, background.a]);
    }
    if grade <= 0 || width == 0 || height == 0 {
        return ImageData::new(width, height, pixels);
    }

    let center_x = (width / 2) as i32;
    let center_y = (height / 2) as i32;
    let texture = MaterialPatternRef::from(texture);
    let overlay = overlay.map(MaterialPatternRef::from);
    for row in -grade..grade {
        let half = ((grade * grade - row * row) as f32).sqrt() as i32;
        for column in (center_x - half)..(center_x + half) {
            let y = center_y + row;
            if column < 0 || y < 0 || column >= width as i32 || y >= height as i32 {
                continue;
            }
            let color = compose_material_preview_pixel(material, column, y, texture, overlay);
            let index = ((y as u32 * width + column as u32) * 4) as usize;
            pixels[index] = color.r;
            pixels[index + 1] = color.g;
            pixels[index + 2] = color.b;
            pixels[index + 3] = color.a;
        }
    }
    ImageData::new(width, height, pixels)
}

/// The preview swatch for a named material and texture.
///
/// Resolves the pair the way the landscape does, including the rule that a
/// liquid — density in `25..50` — asks for `Smooth` and is given `Liquid`
/// instead, so the swatch shows what would actually be painted rather than
/// what was asked for. `None` when either name is unknown, which is the
/// disabled-page case where C++ draws the grey box and nothing else.
pub fn material_preview_swatch_for(
    width: u32,
    height: u32,
    grade: i32,
    material_name: &str,
    texture_name: &str,
    material_render_info: &HashMap<String, MaterialRenderInfo>,
    material_textures: &HashMap<String, MaterialTextureSurface>,
    background: Color,
) -> Option<ImageData> {
    let material =
        material_render_info.get(&clonk_resources::material::c4_name_key(material_name))?;
    let resolve = |name: &str| {
        let key = if (25..50).contains(&material.density)
            && clonk_resources::material::c4_names_equal(name, "Smooth")
        {
            clonk_resources::material::c4_name_key("Liquid")
        } else {
            clonk_resources::material::c4_name_key(name)
        };
        material_textures.get(&key)
    };
    let texture = resolve(texture_name)?;
    let overlay = material.texture_overlay.as_deref().and_then(resolve);
    Some(material_preview_swatch(
        width, height, grade, material, texture, overlay, background,
    ))
}

/// One preview pixel: the material colour, the overlay, then the texture.
///
/// The order is `DrawPatternedCircle`'s, which is the reverse of the
/// landscape's. `landscape_pixel` is zero here because the preview has no
/// landscape behind it — C++ passes `bCol` straight from
/// `Mat2PixColDefault(...)` with no IFT bit set.
fn compose_material_preview_pixel(
    material: &MaterialRenderInfo,
    x: i32,
    y: i32,
    texture: MaterialPatternRef<'_>,
    overlay: Option<MaterialPatternRef<'_>>,
) -> Color {
    let mut pixel = MaterialPixel {
        red: material.color[0],
        green: material.color[1],
        blue: material.color[2],
        transparency: material.alpha[0],
    };
    let monochrome = material.overlay_type & MATERIAL_OVERLAY_MONOCHROME != 0;
    if let Some(overlay) = overlay {
        let overlay_zoom = if material.overlay_type & MATERIAL_OVERLAY_EXACT != 0 {
            1
        } else {
            2
        };
        apply_material_surface(
            &mut pixel,
            material,
            0,
            overlay,
            x,
            y,
            overlay_zoom,
            monochrome,
        );
    }
    let texture_zoom = if material.overlay_type & MATERIAL_OVERLAY_HUGE_ZOOM != 0 {
        4
    } else if material.overlay_type & MATERIAL_OVERLAY_EXACT != 0 {
        1
    } else {
        0
    };
    apply_material_surface(
        &mut pixel,
        material,
        0,
        texture,
        x,
        y,
        texture_zoom,
        monochrome,
    );
    Color::new(
        pixel.red,
        pixel.green,
        pixel.blue,
        255u8.saturating_sub(pixel.transparency),
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

// ---------------------------------------------------------------------------
// GPU-facing packing of the same composition
// ---------------------------------------------------------------------------
//
// `compose_material_surface_pixel` above walks INTEGER landscape-map
// coordinates, so the finest sampling rate the retained CPU composer can ever
// reach is one pattern texel per landscape pixel. Evaluating the identical
// arithmetic per fragment lifts that cap, but only if the shader receives the
// per-texmap-slot parameters in a form it can index. The types below are that
// form, plus a reference evaluator written in the same integer arithmetic the
// shader uses, so the packing can be proven equal to the CPU composer without a
// GPU present.

/// Slot is populated. Absent slots draw nothing, mirroring `Slot::Empty` in the
/// CPU composer.
pub(crate) const MATERIAL_GPU_PRESENT: u32 = 1;
/// `MATERIAL_OVERLAY_MONOCHROME`: take all three pattern modifiers from blue.
pub(crate) const MATERIAL_GPU_MONOCHROME: u32 = 2;
/// A secondary (overlay) pattern follows the primary one.
pub(crate) const MATERIAL_GPU_HAS_OVERLAY: u32 = 4;
/// The primary pattern is a `Surface8`; its atlas texels carry raw indices.
pub(crate) const MATERIAL_GPU_PRIMARY_INDEXED: u32 = 8;
/// The overlay pattern is a `Surface8`.
pub(crate) const MATERIAL_GPU_OVERLAY_INDEXED: u32 = 16;

/// Where one material pattern lives inside the shared pattern atlas. Shipped
/// patterns range 32x32..256x256, so every slot carries its own size — the
/// modulo tiling in `apply_material_pattern` is per pattern, not per atlas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MaterialAtlasRect {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// `Surface8` patterns store their palette index in the atlas red channel.
    pub(crate) indexed: bool,
}

/// One texmap slot in the layout the landscape fragment shader binds as a
/// uniform array. Four `vec4<u32>` keep the std140 stride at 64 bytes, so all
/// 128 slots fit inside the 16 KiB downlevel uniform-binding limit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub(crate) struct MaterialGpuSlot {
    /// `MaterialRenderInfo::color` triplets 0/1/2 packed as `r | g<<8 | b<<16`,
    /// then `alpha[0..3]` packed the same way.
    pub(crate) colors: [u32; 4],
    /// `alpha[3..6]` packed, the primary CPattern zoom, the overlay zoom, and
    /// the `MATERIAL_GPU_*` flag bits.
    pub(crate) params: [u32; 4],
    pub(crate) primary: [u32; 4],
    pub(crate) overlay: [u32; 4],
}

fn pack_triplet(values: &[u8], offset: usize) -> u32 {
    u32::from(values[offset])
        | (u32::from(values[offset + 1]) << 8)
        | (u32::from(values[offset + 2]) << 16)
}

#[cfg(test)]
fn unpack_triplet(packed: u32, channel: u32) -> u8 {
    ((packed >> (channel * 8)) & 0xff) as u8
}

/// Mirrors the zoom/monochrome selection in `compose_material_surface_pixel`
/// (materials.rs:359-382) once, at pack time, so the shader never has to know
/// the `MATERIAL_OVERLAY_*` bit meanings.
pub(crate) fn pack_material_gpu_slot(
    material: &MaterialRenderInfo,
    primary: MaterialAtlasRect,
    overlay: Option<MaterialAtlasRect>,
) -> MaterialGpuSlot {
    let primary_zoom = if material.overlay_type & MATERIAL_OVERLAY_HUGE_ZOOM != 0 {
        4
    } else if material.overlay_type & MATERIAL_OVERLAY_EXACT != 0 {
        1
    } else {
        0
    };
    let overlay_zoom = if material.overlay_type & MATERIAL_OVERLAY_EXACT != 0 {
        1
    } else {
        2
    };
    let mut flags = MATERIAL_GPU_PRESENT;
    if material.overlay_type & MATERIAL_OVERLAY_MONOCHROME != 0 {
        flags |= MATERIAL_GPU_MONOCHROME;
    }
    if primary.indexed {
        flags |= MATERIAL_GPU_PRIMARY_INDEXED;
    }
    if let Some(overlay) = overlay.filter(|rect| rect.width != 0 && rect.height != 0) {
        flags |= MATERIAL_GPU_HAS_OVERLAY;
        if overlay.indexed {
            flags |= MATERIAL_GPU_OVERLAY_INDEXED;
        }
    }
    let rect = |rect: MaterialAtlasRect| [rect.x, rect.y, rect.width, rect.height];
    MaterialGpuSlot {
        colors: [
            pack_triplet(&material.color, 0),
            pack_triplet(&material.color, 3),
            pack_triplet(&material.color, 6),
            pack_triplet(&material.alpha, 0),
        ],
        params: [
            pack_triplet(&material.alpha, 3),
            primary_zoom,
            overlay_zoom,
            flags,
        ],
        primary: rect(primary),
        overlay: rect(overlay.unwrap_or_default()),
    }
}

/// A single RGBA pattern atlas. `Surface8` patterns occupy the red channel.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct MaterialAtlasView<'a> {
    pub(crate) width: u32,
    pub(crate) pixels: &'a [u8],
}

#[cfg(test)]
impl MaterialAtlasView<'_> {
    fn texel(&self, rect: [u32; 4], x: i32, y: i32, zoom: u32) -> Option<[u8; 4]> {
        if rect[2] == 0 || rect[3] == 0 {
            return None;
        }
        // C++ CPattern applies its own zoom to the LANDSCAPE coordinate before
        // the pattern modulo (materials.rs:184-186), which is why shipping a
        // larger pattern only changes the tiling period today.
        let sample_x = if zoom == 0 { x } else { x / zoom as i32 };
        let sample_y = if zoom == 0 { y } else { y / zoom as i32 };
        let texel_x = rect[0] + (sample_x as u32 % rect[2]);
        let texel_y = rect[1] + (sample_y as u32 % rect[3]);
        let offset = ((texel_y * self.width + texel_x) * 4) as usize;
        self.pixels
            .get(offset..offset + 4)
            .map(|slice| [slice[0], slice[1], slice[2], slice[3]])
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn apply_gpu_pattern(
    pixel: &mut MaterialPixel,
    slot: &MaterialGpuSlot,
    landscape_pixel: u8,
    rect: [u32; 4],
    zoom: u32,
    indexed: bool,
    monochrome: bool,
    x: i32,
    y: i32,
    atlas: MaterialAtlasView<'_>,
) {
    let Some(texel) = atlas.texel(rect, x, y, zoom) else {
        return;
    };
    if indexed {
        // Mirrors `apply_indexed_material_pattern` (materials.rs:252-260): the
        // index selects one of the three material colour triplets outright
        // rather than modulating the running pixel.
        let shift = u32::from(texel[0] % 3);
        let packed = slot.colors[shift as usize];
        pixel.red = unpack_triplet(packed, 0);
        pixel.green = unpack_triplet(packed, 1);
        pixel.blue = unpack_triplet(packed, 2);
        let alpha = if landscape_pixel & 0xf0 == 0 {
            slot.colors[3]
        } else {
            slot.params[0]
        };
        pixel.transparency = unpack_triplet(alpha, shift);
        return;
    }
    // Mirrors `apply_material_pattern` (materials.rs:192-204).
    let modifiers = if monochrome {
        [texel[2]; 3]
    } else {
        [texel[0], texel[1], texel[2]]
    };
    pixel.red =
        lighten_material_channel(((u16::from(pixel.red) * u16::from(modifiers[0])) >> 8) as u8);
    pixel.green =
        lighten_material_channel(((u16::from(pixel.green) * u16::from(modifiers[1])) >> 8) as u8);
    pixel.blue =
        lighten_material_channel(((u16::from(pixel.blue) * u16::from(modifiers[2])) >> 8) as u8);
    pixel.transparency = pixel
        .transparency
        .saturating_add(255u8.saturating_sub(texel[3]));
}

/// Reference evaluation of a packed slot, written in the integer arithmetic the
/// WGSL landscape shader uses. Proven equal to `compose_material_surface_pixel`
/// by `packed_material_slot_matches_the_cpu_composer`.
#[cfg(test)]
pub(crate) fn compose_material_gpu_slot(
    slot: &MaterialGpuSlot,
    landscape_pixel: u8,
    x: i32,
    y: i32,
    atlas: MaterialAtlasView<'_>,
) -> Color {
    let flags = slot.params[3];
    if flags & MATERIAL_GPU_PRESENT == 0 {
        return Color::transparent();
    }
    let base_alpha = if landscape_pixel & 0x80 == 0 {
        slot.colors[3]
    } else {
        slot.params[0]
    };
    let mut pixel = MaterialPixel {
        red: unpack_triplet(slot.colors[0], 0),
        green: unpack_triplet(slot.colors[0], 1),
        blue: unpack_triplet(slot.colors[0], 2),
        transparency: unpack_triplet(base_alpha, 0),
    };
    let monochrome = flags & MATERIAL_GPU_MONOCHROME != 0;
    apply_gpu_pattern(
        &mut pixel,
        slot,
        landscape_pixel,
        slot.primary,
        slot.params[1],
        flags & MATERIAL_GPU_PRIMARY_INDEXED != 0,
        monochrome,
        x,
        y,
        atlas,
    );
    if flags & MATERIAL_GPU_HAS_OVERLAY != 0 {
        apply_gpu_pattern(
            &mut pixel,
            slot,
            landscape_pixel,
            slot.overlay,
            slot.params[2],
            flags & MATERIAL_GPU_OVERLAY_INDEXED != 0,
            monochrome,
            x,
            y,
            atlas,
        );
    }
    Color::new(
        pixel.red,
        pixel.green,
        pixel.blue,
        255u8.saturating_sub(pixel.transparency),
    )
}

/// Lays every registered pattern out in one vertical strip. A strip needs no
/// packing heuristic and keeps each rect's origin independent of its
/// neighbours, which is all the per-slot modulo tiling requires.
/// Build the fragment-shader composer's inputs from resolved texmap slots.
///
/// Every pattern a slot references is packed into one atlas, then each slot is
/// packed with `pack_material_gpu_slot` — the same packer
/// `packed_material_slot_matches_the_cpu_composer` proves equals
/// `compose_material_surface_pixel`. Nothing here re-derives composition
/// arithmetic; it only marshals what the CPU composer already resolved.
/// The pattern atlas and packed slot table one material catalogue resolves to.
///
/// Both are shared rather than copied into each plan: they are the same bytes
/// until the catalogue is reloaded, and a landscape update must not pay for
/// them.
#[derive(Clone, Debug)]
pub(crate) struct PackedMaterialCatalogue {
    pub(crate) atlas: Arc<[u8]>,
    pub(crate) atlas_extent: [u32; 2],
    pub(crate) slots: Arc<[[u32; 16]]>,
}

/// Keeps the packed catalogue across landscape updates.
///
/// The atlas and slot table are resolved from two independent things: the
/// material catalogue the `GraphicsSystem` holds, which its setters revision,
/// and the texmap the *landscape grid* carries, which they do not see. Both
/// belong in the key — keying on the revision alone serves a stale atlas to a
/// landscape whose texmap has been replaced. The names are compared rather
/// than hashed, and cloned only when the packing is rebuilt.
#[derive(Debug, Default)]
pub(crate) struct MaterialAtlasCache {
    revision: u64,
    materials: Vec<Option<String>>,
    textures: Vec<Option<String>>,
    packed: Option<PackedMaterialCatalogue>,
}

impl MaterialAtlasCache {
    /// The packed catalogue for this revision and texmap, building it only
    /// when the one held belongs to another.
    pub(crate) fn resolve(
        &mut self,
        revision: u64,
        materials: &[Option<String>],
        textures: &[Option<String>],
        build: impl FnOnce() -> PackedMaterialCatalogue,
    ) -> PackedMaterialCatalogue {
        let current =
            self.revision == revision && self.materials == materials && self.textures == textures;
        let held = self.packed.as_ref().filter(|_| current).cloned();
        held.unwrap_or_else(|| {
            let packed = build();
            self.revision = revision;
            self.materials = materials.to_vec();
            self.textures = textures.to_vec();
            self.packed = Some(packed.clone());
            packed
        })
    }
}

pub(crate) fn build_shader_landscape_plan(
    extent: [u32; 2],
    index_plane: Vec<u8>,
    shading_plane: Option<Vec<u8>>,
    slots: &[MaterialSlot<'_>],
    catalogue: &mut MaterialAtlasCache,
    revision: u64,
    material_names: &[Option<String>],
    texture_names: &[Option<String>],
) -> clonk_graphics::ShaderLandscapePlan {
    let packed = catalogue.resolve(revision, material_names, texture_names, || {
        pack_material_catalogue(slots)
    });
    clonk_graphics::ShaderLandscapePlan {
        extent,
        index_plane,
        shading_plane,
        atlas: packed.atlas.to_vec(),
        atlas_extent: packed.atlas_extent,
        slots: packed.slots.to_vec(),
    }
}

/// Packs every pattern the texmap references into one atlas and packs each
/// slot against it. Catalogue work only: nothing here reads the landscape.
fn pack_material_catalogue(slots: &[MaterialSlot<'_>]) -> PackedMaterialCatalogue {
    // Collect every referenced pattern once, remembering where each slot's
    // primary and overlay landed so the rects can be looked up after packing.
    let mut patterns: Vec<MaterialPatternRef<'_>> = Vec::new();
    let mut indices: Vec<Option<(usize, Option<usize>)>> = Vec::with_capacity(slots.len());
    for slot in slots {
        match slot {
            MaterialSlot::Empty => indices.push(None),
            MaterialSlot::Patterns {
                texture, overlay, ..
            } => {
                let primary = patterns.len();
                patterns.push(MaterialPatternRef::from(*texture));
                let secondary = overlay.map(|overlay| {
                    patterns.push(MaterialPatternRef::from(overlay));
                    patterns.len() - 1
                });
                indices.push(Some((primary, secondary)));
            }
        }
    }
    let (atlas_width, atlas_height, atlas, rects) = build_material_atlas(&patterns);

    let packed: Vec<[u32; 16]> = slots
        .iter()
        .zip(&indices)
        .map(|(slot, index)| {
            let (MaterialSlot::Patterns { material, .. }, Some((primary, overlay))) = (slot, index)
            else {
                // An absent slot composes nothing; the shader reads the
                // PRESENT bit out of the zeroed params word.
                return [0_u32; 16];
            };
            let packed =
                pack_material_gpu_slot(material, rects[*primary], overlay.map(|i| rects[i]));
            let mut words = [0_u32; 16];
            words[0..4].copy_from_slice(&packed.colors);
            words[4..8].copy_from_slice(&packed.params);
            words[8..12].copy_from_slice(&packed.primary);
            words[12..16].copy_from_slice(&packed.overlay);
            words
        })
        .collect();

    PackedMaterialCatalogue {
        atlas: Arc::from(atlas),
        atlas_extent: [atlas_width, atlas_height],
        slots: Arc::from(packed),
    }
}

/// One texmap slot's composition inputs: `C4TexMapEntry`'s primary pattern
/// plus the material's secondary pattern.
pub(crate) enum MaterialSlot<'a> {
    Empty,
    Patterns {
        material: &'a MaterialRenderInfo,
        texture: &'a MaterialTextureSurface,
        overlay: Option<&'a MaterialTextureSurface>,
    },
}

/// Resolve all 128 texmap slots to their composition inputs.
///
/// Extracted from the retained CPU composer so the shader composer can build
/// its slot table from exactly the same resolution — the liquid-`Smooth`
/// substitution and the `Smooth` overlay fallback below are easy to restate
/// slightly differently, and a divergence there would silently compose a
/// different landscape on the two paths.
pub(crate) fn resolve_material_slots<'a>(
    materials: &[Option<String>],
    textures: &[Option<String>],
    material_render_info: &'a HashMap<String, MaterialRenderInfo>,
    material_textures: &'a HashMap<String, MaterialTextureSurface>,
) -> Vec<MaterialSlot<'a>> {
    (0..128usize)
        .map(|index| {
            let Some(material) = materials
                .get(index)
                .and_then(|name| name.as_deref())
                .and_then(|name| {
                    material_render_info.get(&clonk_resources::material::c4_name_key(name))
                })
            else {
                return MaterialSlot::Empty;
            };
            let resolve_texture = |name: &str| {
                let name = if (25..50).contains(&material.density)
                    && clonk_resources::material::c4_names_equal(name, "Smooth")
                {
                    clonk_resources::material::c4_name_key("Liquid")
                } else {
                    clonk_resources::material::c4_name_key(name)
                };
                material_textures.get(&name)
            };
            let Some(texture) = textures
                .get(index)
                .and_then(|name| name.as_deref())
                .and_then(resolve_texture)
            else {
                return MaterialSlot::Empty;
            };
            let overlay_name = material
                .texture_overlay
                .as_deref()
                .filter(|name| {
                    material_textures.contains_key(&clonk_resources::material::c4_name_key(name))
                })
                .unwrap_or("Smooth");
            MaterialSlot::Patterns {
                material,
                texture,
                overlay: resolve_texture(overlay_name),
            }
        })
        .collect()
}

pub(crate) fn build_material_atlas(
    patterns: &[MaterialPatternRef<'_>],
) -> (u32, u32, Vec<u8>, Vec<MaterialAtlasRect>) {
    let width = patterns
        .iter()
        .map(|pattern| match pattern {
            MaterialPatternRef::Surface32(image) => image.width(),
            MaterialPatternRef::Surface8 { width, .. } => *width,
        })
        .max()
        .unwrap_or(1)
        .max(1);
    let mut rects = Vec::with_capacity(patterns.len());
    let mut pixels: Vec<u8> = Vec::new();
    let mut origin_y = 0;
    for pattern in patterns {
        let (pattern_width, pattern_height, indexed) = match pattern {
            MaterialPatternRef::Surface32(image) => (image.width(), image.height(), false),
            MaterialPatternRef::Surface8 { width, height, .. } => (*width, *height, true),
        };
        rects.push(MaterialAtlasRect {
            x: 0,
            y: origin_y,
            width: pattern_width,
            height: pattern_height,
            indexed,
        });
        for row in 0..pattern_height {
            let start = pixels.len();
            pixels.resize(start + width as usize * 4, 0);
            for column in 0..pattern_width {
                let destination = start + column as usize * 4;
                match pattern {
                    MaterialPatternRef::Surface32(image) => {
                        let source = ((row * image.width() + column) * 4) as usize;
                        if let Some(texel) = image.pixels().get(source..source + 4) {
                            pixels[destination..destination + 4].copy_from_slice(texel);
                        }
                    }
                    MaterialPatternRef::Surface8 { indices, .. } => {
                        let source = (row * pattern_width + column) as usize;
                        pixels[destination] = indices.get(source).copied().unwrap_or(0);
                        pixels[destination + 3] = 255;
                    }
                }
            }
        }
        origin_y += pattern_height;
    }
    if pixels.is_empty() {
        pixels.resize(width as usize * 4, 0);
        origin_y = 1;
    }
    (width, origin_y, pixels, rects)
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

#[cfg(test)]
mod gpu_slot_tests {
    use super::*;

    fn surface32_pattern() -> ImageData {
        // Deliberately non-square so a wrong modulo shows up as a mismatch.
        let (width, height) = (4_u32, 3_u32);
        let pixels = (0..width * height)
            .flat_map(|index| {
                let index = index as u8;
                [
                    index.wrapping_mul(37).wrapping_add(3),
                    index.wrapping_mul(91).wrapping_add(17),
                    index.wrapping_mul(53).wrapping_add(200),
                    index.wrapping_mul(29).wrapping_add(11),
                ]
            })
            .collect();
        ImageData::new(width, height, pixels)
    }

    fn surface8_pattern() -> MaterialTextureSurface {
        MaterialTextureSurface::surface8(3, 5, (0..15u8).map(|index| index * 7).collect())
    }

    /// A plan built from RESOLVED slots must compose the same pixels the CPU
    /// composer does for those slots. `packed_material_slot_matches_the_cpu_composer`
    /// already proves the packer's arithmetic; this proves the marshalling
    /// around it — atlas placement, per-slot rect lookup and the flat word
    /// layout the renderer reads — does not scramble which pattern a slot gets.
    #[test]
    fn a_landscape_plan_composes_the_same_pixels_as_the_cpu_composer() {
        let image = surface32_pattern();
        let indexed = surface8_pattern();
        let mut render_info = HashMap::new();
        let mut textures = HashMap::new();
        // Two materials with different patterns and overlay policies, so a
        // swapped atlas rect cannot pass by coincidence.
        render_info.insert(
            clonk_resources::material::c4_name_key("Earth"),
            MaterialRenderInfo::new(
                [127, 95, 63, 147, 111, 75, 171, 127, 91],
                [0, 30, 60, 90, 120, 200],
                Some("Rough".to_string()),
                MATERIAL_OVERLAY_EXACT,
                50,
            ),
        );
        render_info.insert(
            clonk_resources::material::c4_name_key("Rock"),
            MaterialRenderInfo::new(
                [86, 86, 86, 106, 106, 106, 126, 126, 126],
                [10, 40, 70, 100, 130, 210],
                None,
                MATERIAL_OVERLAY_MONOCHROME,
                60,
            ),
        );
        textures.insert(
            clonk_resources::material::c4_name_key("Smooth"),
            MaterialTextureSurface::surface32(image.clone()),
        );
        textures.insert(clonk_resources::material::c4_name_key("Rough"), indexed);

        let mut material_names = vec![None; 128];
        let mut texture_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        texture_names[1] = Some("Smooth".to_string());
        material_names[2] = Some("Rock".to_string());
        texture_names[2] = Some("Smooth".to_string());

        let slots =
            resolve_material_slots(&material_names, &texture_names, &render_info, &textures);
        let extent = [7_u32, 5_u32];
        let index_plane: Vec<u8> = (0..extent[0] * extent[1])
            .map(|i| if i % 3 == 0 { 1 } else { 2 })
            .collect();
        let mut catalogue = MaterialAtlasCache::default();
        let plan = build_shader_landscape_plan(
            extent,
            index_plane.clone(),
            None,
            &slots,
            &mut catalogue,
            1,
            &material_names,
            &texture_names,
        );

        assert_eq!(plan.slots.len(), 128);
        assert_eq!(plan.extent, extent);
        // Sky and every unnamed slot stay absent, so the shader composes nothing.
        assert_eq!(plan.slots[0], [0_u32; 16]);
        assert_eq!(plan.slots[3], [0_u32; 16]);

        let atlas = MaterialAtlasView {
            width: plan.atlas_extent[0],
            pixels: &plan.atlas,
        };
        let mut compared = 0_usize;
        for (offset, byte) in index_plane.iter().enumerate() {
            let MaterialSlot::Patterns {
                material,
                texture,
                overlay,
            } = &slots[usize::from(byte & 0x7f)]
            else {
                panic!("fixture slot {byte} must carry patterns");
            };
            let x = (offset as u32 % extent[0]) as i32;
            let y = (offset as u32 / extent[0]) as i32;
            let expected = compose_material_surface_pixel(
                material,
                *byte,
                x,
                y,
                MaterialPatternRef::from(*texture),
                overlay.map(MaterialPatternRef::from),
            );
            let words = plan.slots[usize::from(byte & 0x7f)];
            let packed = MaterialGpuSlot {
                colors: words[0..4].try_into().expect("colors"),
                params: words[4..8].try_into().expect("params"),
                primary: words[8..12].try_into().expect("primary"),
                overlay: words[12..16].try_into().expect("overlay"),
            };
            assert_eq!(
                compose_material_gpu_slot(&packed, *byte, x, y, atlas),
                expected,
                "plan slot {byte} disagrees at ({x},{y})"
            );
            compared += 1;
        }
        assert_eq!(compared, index_plane.len());
    }

    /// The packed slot plus the shared atlas must reproduce
    /// `compose_material_surface_pixel` exactly; anything less makes a shader
    /// composer a divergence rather than a lift of the detail cap.
    #[test]
    fn packed_material_slot_matches_the_cpu_composer() {
        let image = surface32_pattern();
        let indexed = surface8_pattern();
        let primary_refs = [
            MaterialPatternRef::Surface32(&image),
            MaterialPatternRef::from(&indexed),
        ];
        let (atlas_width, _, atlas_pixels, rects) = build_material_atlas(&primary_refs);
        let atlas = MaterialAtlasView {
            width: atlas_width,
            pixels: &atlas_pixels,
        };

        let color = [10, 90, 200, 40, 130, 250, 70, 20, 160];
        let alpha = [0, 30, 60, 90, 120, 200];
        let overlay_types = [
            0,
            MATERIAL_OVERLAY_EXACT,
            MATERIAL_OVERLAY_HUGE_ZOOM,
            MATERIAL_OVERLAY_MONOCHROME,
            MATERIAL_OVERLAY_HUGE_ZOOM | MATERIAL_OVERLAY_MONOCHROME,
        ];
        let mut compared = 0_usize;
        for overlay_type in overlay_types {
            let material = MaterialRenderInfo::new(color, alpha, None, overlay_type, 50);
            for (primary_index, primary) in primary_refs.iter().enumerate() {
                for overlay_index in [None, Some(0), Some(1)] {
                    let overlay = overlay_index.map(|index| primary_refs[index]);
                    let slot = pack_material_gpu_slot(
                        &material,
                        rects[primary_index],
                        overlay_index.map(|index| rects[index]),
                    );
                    for landscape_pixel in [1_u8, 0x21, 0x81, 0xff] {
                        for y in 0..11 {
                            for x in 0..11 {
                                let expected = compose_material_surface_pixel(
                                    &material,
                                    landscape_pixel,
                                    x,
                                    y,
                                    *primary,
                                    overlay,
                                );
                                let actual =
                                    compose_material_gpu_slot(&slot, landscape_pixel, x, y, atlas);
                                assert_eq!(
                                    actual, expected,
                                    "overlay_type {overlay_type}, primary {primary_index}, \
                                     overlay {overlay_index:?}, pixel {landscape_pixel:#04x} \
                                     at ({x}, {y})"
                                );
                                compared += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(compared > 3000, "the sweep must cover the tiling period");
    }

    /// An unpopulated slot draws nothing, mirroring `Slot::Empty`.
    #[test]
    fn absent_material_slot_composes_nothing() {
        let atlas_pixels = [0_u8; 4];
        let atlas = MaterialAtlasView {
            width: 1,
            pixels: &atlas_pixels,
        };
        let composed = compose_material_gpu_slot(&MaterialGpuSlot::default(), 1, 0, 0, atlas);
        assert_eq!(composed, Color::transparent());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `C4ToolsDlg::UpdatePreview` draws the swatch as
    /// `DrawPatternedCircle(surface, w / 2, h / 2, Grade, ...)` — a disc of the
    /// **grade** radius, not a filled box (`C4ToolsDlg.cpp:673-677`,
    /// `StdDDraw2.cpp:1191-1207`). That is why the grade slider sits beside it.
    #[test]
    fn the_material_preview_is_a_disc_of_the_grade_radius() {
        let material =
            MaterialRenderInfo::new([64, 96, 128, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 50);
        // A neutral 128 pattern survives `ModulateClrA` and `LightenClr`
        // unchanged — `(c * 128) >> 8` then `<< 1` — so the disc carries the
        // material's own colour and this test is about the shape.
        let texture =
            MaterialTextureSurface::surface32(ImageData::new(1, 1, vec![128, 128, 128, 255]));
        let background = Color::opaque(9, 9, 9);
        let swatch = material_preview_swatch(16, 16, 4, &material, &texture, None, background);
        let at = |x: usize, y: usize| {
            let index = (y * 16 + x) * 4;
            let pixels = swatch.pixels();
            Color::new(
                pixels[index],
                pixels[index + 1],
                pixels[index + 2],
                pixels[index + 3],
            )
        };

        assert_eq!(
            at(8, 8),
            Color::opaque(64, 96, 128),
            "the centre is inside the disc and carries the material colour"
        );
        assert_eq!(at(0, 0), background, "a corner is outside it");

        // The loop bounds are exclusive at the top on both axes, so the disc is
        // one pixel short on the right. That asymmetry is C++'s, not a slip.
        assert_eq!(
            at(4, 8),
            Color::opaque(64, 96, 128),
            "the left edge is drawn"
        );
        assert_eq!(at(3, 8), background, "and one past it is not");
        assert_eq!(
            at(11, 8),
            Color::opaque(64, 96, 128),
            "the last drawn column"
        );
        assert_eq!(at(12, 8), background, "x + lwdt itself is not drawn");
    }

    /// A liquid asking for `Smooth` is given `Liquid`, the same substitution
    /// the landscape makes, so the swatch shows what would be painted.
    #[test]
    fn a_liquid_preview_resolves_smooth_to_the_liquid_texture() {
        // Density 25 puts this in the liquid band.
        let material = MaterialRenderInfo::new([0, 190, 0, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25);
        let materials = HashMap::from([("acid".to_owned(), material)]);
        let textures = HashMap::from([
            // Neutral, so the disc keeps the material colour.
            (
                "liquid".to_owned(),
                MaterialTextureSurface::surface32(ImageData::new(1, 1, vec![128, 128, 128, 255])),
            ),
            // Black, so picking this one instead would be obvious.
            (
                "smooth".to_owned(),
                MaterialTextureSurface::surface32(ImageData::new(1, 1, vec![0, 0, 0, 255])),
            ),
        ]);
        let background = Color::opaque(9, 9, 9);

        let swatch = material_preview_swatch_for(
            8, 8, 3, "Acid", "Smooth", &materials, &textures, background,
        )
        .expect("a known pair produces a swatch");
        let index = ((4 * 8 + 4) * 4) as usize;
        let pixels = swatch.pixels();
        assert_eq!(
            [pixels[index], pixels[index + 1], pixels[index + 2]],
            [0, 190, 0],
            "the neutral Liquid texture was used, not the black Smooth one"
        );

        assert!(
            material_preview_swatch_for(
                8,
                8,
                3,
                "Nonexistent",
                "Smooth",
                &materials,
                &textures,
                background
            )
            .is_none(),
            "an unknown material has no swatch rather than a black one"
        );
    }

    /// A grade of zero has no disc at all, which is what a disabled or
    /// zero-width brush shows.
    #[test]
    fn a_zero_grade_preview_is_all_background() {
        let material =
            MaterialRenderInfo::new([64, 96, 128, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 50);
        let texture =
            MaterialTextureSurface::surface32(ImageData::new(1, 1, vec![128, 128, 128, 255]));
        let background = Color::opaque(9, 9, 9);
        let swatch = material_preview_swatch(8, 8, 0, &material, &texture, None, background);
        assert!(
            swatch
                .pixels()
                .chunks_exact(4)
                .all(|pixel| pixel == [9, 9, 9, 255]),
            "no radius means nothing is drawn over the background"
        );
    }

    /// The pattern atlas and packed slot table are built from the material
    /// catalogue, not from the landscape, so a landscape update must reuse
    /// them. They were rebuilt — atlas allocation, pattern copies and all —
    /// on every update.
    #[test]
    fn the_packed_catalogue_is_rebuilt_only_when_the_catalogue_changes() {
        let mut cache = MaterialAtlasCache::default();
        let builds = std::cell::Cell::new(0_usize);
        let build = || {
            builds.set(builds.get() + 1);
            PackedMaterialCatalogue {
                atlas: Arc::from(vec![7_u8; 4]),
                atlas_extent: [1, 1],
                slots: Arc::from(vec![[0_u32; 16]]),
            }
        };

        let earth = [Some("Earth".to_string()), None];
        let smooth = [Some("Smooth".to_string()), None];
        let rough = [Some("Rough".to_string()), None];

        let first = cache.resolve(1, &earth, &smooth, build);
        assert_eq!(builds.get(), 1);

        let again = cache.resolve(1, &earth, &smooth, build);
        assert_eq!(builds.get(), 1, "an unchanged catalogue is not rebuilt");
        assert!(
            Arc::ptr_eq(&first.atlas, &again.atlas),
            "the retained atlas is shared, not copied"
        );
        assert!(Arc::ptr_eq(&first.slots, &again.slots));

        let reloaded = cache.resolve(2, &earth, &smooth, build);
        assert_eq!(builds.get(), 2, "a reloaded catalogue is rebuilt");
        assert!(!Arc::ptr_eq(&first.atlas, &reloaded.atlas));

        // The texmap belongs to the landscape grid, not to the catalogue, so
        // it moves without a revision bump.
        let _ = cache.resolve(2, &earth, &rough, build);
        assert_eq!(
            builds.get(),
            3,
            "a texmap the catalogue's revision cannot see is still rebuilt"
        );
    }
}
