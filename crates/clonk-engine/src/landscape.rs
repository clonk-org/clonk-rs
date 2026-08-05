use std::collections::{HashMap, VecDeque};
use std::convert::TryFrom;
use std::mem;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

#[cfg(test)]
use std::cell::Cell;

use crate::material::TemperatureDirection;
#[cfg(test)]
use crate::EnvironmentSettings;
use crate::{math, C4Fixed, FixedVec2, MaterialId, MaterialSet, Vector2};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

const C4M_VEHICLE: i32 = 100;
const C4M_BACKGROUND: i32 = 0;
/// DensitySolid threshold (C4M_Solid, C4Material.h:200).
const C4M_SOLID: i32 = 50;
/// DensityLiquid lower bound (C4M_Liquid, C4Material.h:203).
const C4M_LIQUID: i32 = 25;
/// C4M_MaxTexIndex: slot 127 is reserved for landscape diffs, so runtime
/// texture-map lookup/allocation only visits 1..126 (C4Constants.h:63;
/// C4Texture.cpp:319-340).
const C4M_MAX_TEX_INDEX: usize = 127;
/// C++ retains at most fifty pending relight rectangles
/// (`C4LS_MaxRelights`, C4Landscape.h:43). Rust keeps the same spatial cap in
/// each completed render-dirty generation and reapplies it when joining
/// skipped generations. Generation history is also bounded; an older cache
/// safely falls back to a full rebuild when its lineage has expired.
const MAX_RENDER_DIRTY_GENERATIONS: usize = 50;
const MAX_RENDER_DIRTY_RECTS: usize = 50;

const RENDER_TOKEN_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const RENDER_TOKEN_PRIME: u64 = 0x0000_0100_0000_01b3;

#[cfg(test)]
std::thread_local! {
    static MATERIAL_COUNT_FULL_REBUILDS: Cell<usize> = const { Cell::new(0) };
}

#[cfg(windows)]
const C_RAND_MAX: i32 = 32_767;
#[cfg(not(windows))]
const C_RAND_MAX: i32 = i32::MAX;

unsafe extern "C" {
    fn rand() -> std::ffi::c_int;
}

fn render_token_bytes(mut token: u64, bytes: impl IntoIterator<Item = u8>) -> u64 {
    for byte in bytes {
        token ^= u64::from(byte);
        token = token.wrapping_mul(RENDER_TOKEN_PRIME);
    }
    token
}

fn initial_render_token(width: u32, height: u32, bytes: &[u8]) -> u64 {
    let token = render_token_bytes(RENDER_TOKEN_OFFSET, width.to_le_bytes());
    let token = render_token_bytes(token, height.to_le_bytes());
    render_token_bytes(token, bytes.iter().copied())
}

/// Which C4Landscape pixel write this is. `SetPix` notes the pixel for
/// relighting (C4Landscape.cpp:755-761); the raw `_SetPix` does not, and
/// leaves that to the `PrepareChange`/`FinishChange` caller that relights the
/// whole changed rectangle (C4Landscape.cpp:2851-2880).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PixelWrite {
    /// `C4Landscape::SetPix`.
    SetPix,
    /// `C4Landscape::_SetPix`.
    Raw,
}

/// A clipped half-open rectangle whose current texmap bytes must be
/// recomposed in the frontend's persistent landscape cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelGridDirtyRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl PixelGridDirtyRect {
    fn single(x: i32, y: i32) -> Self {
        Self {
            x: x as u32,
            y: y as u32,
            width: 1,
            height: 1,
        }
    }

    fn from_vertices(vertices: &[(i32, i32)], width: u32, height: u32) -> Option<Self> {
        let min_x = vertices.iter().map(|&(x, _)| x).min()?;
        let min_y = vertices.iter().map(|&(_, y)| y).min()?;
        let max_x = vertices.iter().map(|&(x, _)| x).max()?;
        let max_y = vertices.iter().map(|&(_, y)| y).max()?;
        RasterChangeRect::new(
            min_x,
            min_y,
            max_x.saturating_sub(min_x).saturating_add(1),
            max_y.saturating_sub(min_y).saturating_add(1),
        )
        .clipped_to(width as i32, height as i32)
        .map(Self::from)
    }

    fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let bottom = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    fn overlaps(self, other: Self) -> bool {
        self.x < other.x.saturating_add(other.width)
            && other.x < self.x.saturating_add(self.width)
            && self.y < other.y.saturating_add(other.height)
            && other.y < self.y.saturating_add(self.height)
    }

    fn set_pix_overlap_area(self) -> Self {
        let x = self.x.saturating_sub(2);
        let y = self.y.saturating_sub(16);
        let right = self.x.saturating_add(self.width).saturating_add(2);
        let bottom = self.y.saturating_add(self.height).saturating_add(16);
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    pub fn x(self) -> u32 {
        self.x
    }

    pub fn y(self) -> u32 {
        self.y
    }

    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }

    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x as i32
            && y >= self.y as i32
            && x < self.x.saturating_add(self.width) as i32
            && y < self.y.saturating_add(self.height) as i32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PixelGridDirtyGeneration {
    base_revision: u64,
    revision: u64,
    base_token: u64,
    token: u64,
    /// Bounding union retained under the original field name so an older
    /// reader safely over-renders a sparse generation it cannot understand.
    rect: PixelGridDirtyRect,
    /// Runtime-only precision for the frontend cache. Serialized engine state
    /// retains the legacy bounding rectangle so replay hashes remain stable.
    #[serde(skip)]
    rects: Vec<PixelGridDirtyRect>,
}

impl PixelGridDirtyGeneration {
    fn rects(&self) -> impl Iterator<Item = PixelGridDirtyRect> + '_ {
        let legacy = self.rects.is_empty().then_some(self.rect);
        legacy.into_iter().chain(self.rects.iter().copied())
    }

    fn add_rect(&mut self, rect: PixelGridDirtyRect) {
        let overlap_area = rect.set_pix_overlap_area();
        if self.rects.is_empty() {
            if self.rect.overlaps(overlap_area) {
                self.rect = self.rect.union(rect);
                return;
            }
            self.rects.push(self.rect);
        }
        Self::add_capped_rect(&mut self.rects, rect);
        self.rect = self.rect.union(rect);
    }

    /// `C4Landscape::SetPix`: merge lighting-nearby changes, retain fifty
    /// regions, then union all overflow into the final slot.
    fn add_capped_rect(rects: &mut Vec<PixelGridDirtyRect>, rect: PixelGridDirtyRect) {
        let overlap_area = rect.set_pix_overlap_area();
        if let Some(existing) = rects
            .iter_mut()
            .find(|existing| existing.overlaps(overlap_area))
        {
            *existing = existing.union(rect);
        } else if rects.len() < MAX_RENDER_DIRTY_RECTS {
            rects.push(rect);
        } else if let Some(last) = rects.last_mut() {
            *last = last.union(rect);
        }
    }
}

/// Hex-string serde for the pixel byte plane (a JSON number array would be
/// ~10MB for a real map; hex keeps state exports tractable).
mod hex_bytes {
    use std::sync::Arc;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        bytes: &Arc<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes.iter() {
            text.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble"));
            text.push(char::from_digit((byte & 0xf) as u32, 16).expect("nibble"));
        }
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<Vec<u8>>, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() % 2 != 0 {
            return Err(D::Error::custom("odd hex length"));
        }
        let bytes = text
            .as_bytes()
            .chunks(2)
            .map(|pair| {
                let high = (pair[0] as char).to_digit(16);
                let low = (pair[1] as char).to_digit(16);
                match (high, low) {
                    (Some(high), Some(low)) => Ok(((high << 4) | low) as u8),
                    _ => Err(D::Error::custom("invalid hex digit")),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Arc::new(bytes))
    }
}

mod surface32_pixels_serde {
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        pixels: &Arc<HashMap<usize, u32>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        pixels.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Arc<HashMap<usize, u32>>, D::Error> {
        HashMap::deserialize(deserializer).map(Arc::new)
    }
}

fn surface32_pixels_are_empty(pixels: &Arc<HashMap<usize, u32>>) -> bool {
    pixels.is_empty()
}

/// The per-pixel landscape plane (C4Landscape `Surface8`): each byte is a
/// texmap index in the low 7 bits with the IFT bit 0x80
/// (C4Landscape.h:29-32). Densities and materials resolve through the
/// `Pix2Dens`/`Pix2Mat`-style tables (UpdatePixMaps,
/// C4Landscape.cpp:2832-2839). Classified static/exact maps build one;
/// column-only landscapes (fixtures) carry none and keep the surface
/// approximation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelGrid {
    width: u32,
    height: u32,
    /// Row-major texmap-index bytes. Landscape clones made for script host
    /// contexts and snapshots share this large plane until a terrain write.
    #[serde(with = "hex_bytes")]
    bytes: Arc<Vec<u8>>,
    /// Sparse direct writes into C4Landscape's presentation-only Surface32.
    /// C++ saves Surface32 as Landscape.png alongside Surface8, so preserve
    /// these replacements across EngineState and snapshot serialization.
    #[serde(
        default,
        with = "surface32_pixels_serde",
        skip_serializing_if = "surface32_pixels_are_empty"
    )]
    surface32_pixels: Arc<HashMap<usize, u32>>,
    /// TEXTURE name per texmap index (presentation only: the frontend
    /// samples the texture png per pixel).
    #[serde(default)]
    texture_names: Vec<Option<String>>,
    /// Bumped on every pixel change — the frontend's render cache key.
    #[serde(default)]
    revision: u64,
    /// Stable content-lineage token. Revision alone cannot distinguish two
    /// cloned landscapes that both make a different first edit.
    #[serde(default)]
    render_token: u64,
    /// Bounded COW generations joining a rendered/snapshotted byte plane to
    /// the dirty rectangle of its successor. A cache outside this ancestry
    /// performs a safe full rebuild.
    #[serde(default)]
    dirty_generations: VecDeque<PixelGridDirtyGeneration>,
    /// Surface32 writes share the frontend cache but not Surface8's revision:
    /// cosmetic writes must not look like material-plane mutations.
    #[serde(skip)]
    surface32_revision: u64,
    #[serde(skip)]
    surface32_render_token: u64,
    #[serde(skip)]
    surface32_dirty_generations: VecDeque<PixelGridDirtyGeneration>,
    /// C4Landscape::SetPix queues relights until Draw::DoRelights. Direct
    /// Surface32 writes inside one of these regions cannot survive that draw.
    #[serde(skip)]
    pending_surface32_relights: Vec<PixelGridDirtyRect>,
    /// Pix2Dens: density per texmap index (IFT stripped); index 0 and
    /// unmapped entries are sky (density 0).
    densities: Vec<i32>,
    /// Material NAME per texmap index — resolved into [`Self::materials`]
    /// once the engine's `MaterialSet` exists (`Engine::set_landscape`).
    material_names: Vec<Option<String>>,
    /// Pix2Mat: the engine `MaterialId` per texmap index.
    #[serde(default)]
    materials: Vec<Option<MaterialId>>,
    /// C4Landscape::MatCount, rebuilt from the byte plane after material
    /// resolution and updated incrementally on every pixel write.
    #[serde(skip)]
    material_counts: Vec<u32>,
    /// Content-derived identity of [`Self::material_names`] and
    /// [`Self::texture_names`], recomputed only where those tables are
    /// assigned. `render_dirty_rects_since` runs on every presented frame and
    /// used to compare both ~128-entry `Vec<Option<String>>` element by
    /// element to answer one yes/no question; this answers it with a `u64`.
    ///
    /// Derived rather than a counter on purpose: two grids that never synced a
    /// texmap would share any naive generation, and the frontend may be handed
    /// grids from unrelated landscapes.
    #[serde(skip)]
    texmap_identity: RuntimeTexmapIdentity,
}

/// Runtime-only identity of the texmap name tables.
///
/// Not serialized and deliberately equal to every other value: it is a cache
/// of what the tables already say, so a grid that carries it must stay equal
/// to the identical grid a save round-trip produces without it. Zero means
/// "not computed" — a deserialized grid — and
/// [`PixelGrid::texmap_tables_match`] then falls back to comparing the tables,
/// so a save can never take a wrong fast path.
#[derive(Debug, Clone, Copy, Default)]
struct RuntimeTexmapIdentity(u64);

impl PartialEq for RuntimeTexmapIdentity {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for RuntimeTexmapIdentity {}

/// Identity of the two texmap name tables: FNV-1a over their bytes, with each
/// table's length and each entry's length folded in so that `["ab", "c"]` and
/// `["a", "bc"]`, and `[None]` and `[Some("")]`, all differ. Never zero, which
/// [`RuntimeTexmapIdentity`] reserves for "not computed".
fn texmap_identity(
    material_names: &[Option<String>],
    texture_names: &[Option<String>],
) -> RuntimeTexmapIdentity {
    let token =
        [material_names, texture_names]
            .into_iter()
            .fold(RENDER_TOKEN_OFFSET, |token, names| {
                names.iter().fold(
                    render_token_bytes(token, (names.len() as u64).to_le_bytes()),
                    |token, name| {
                        let length = name.as_ref().map_or(0, |name| name.len() as u64 + 1);
                        let token = render_token_bytes(token, length.to_le_bytes());
                        render_token_bytes(token, name.iter().flat_map(|name| name.bytes()))
                    },
                )
            });
    RuntimeTexmapIdentity(if token == 0 { 1 } else { token })
}

impl PixelGrid {
    pub fn new(
        width: u32,
        height: u32,
        bytes: Vec<u8>,
        densities: Vec<i32>,
        material_names: Vec<Option<String>>,
        texture_names: Vec<Option<String>>,
    ) -> Self {
        debug_assert_eq!(bytes.len(), width as usize * height as usize);
        let materials = vec![None; material_names.len()];
        let render_token = initial_render_token(width, height, &bytes);
        let identity = texmap_identity(&material_names, &texture_names);
        Self {
            texmap_identity: identity,
            width,
            height,
            bytes: Arc::new(bytes),
            surface32_pixels: Arc::new(HashMap::new()),
            densities,
            material_names,
            materials,
            material_counts: Vec::new(),
            texture_names,
            revision: 0,
            render_token,
            dirty_generations: VecDeque::new(),
            surface32_revision: 0,
            surface32_render_token: 0,
            surface32_dirty_generations: VecDeque::new(),
            pending_surface32_relights: Vec::new(),
        }
    }

    /// Install the complete presentation-only Surface32 created while an
    /// exact landscape is loading. This is an initial-state operation rather
    /// than a sequence of runtime SetPixDw calls: no dirty generations have
    /// been presented yet, and constructing the map in one pass avoids a
    /// revision (and allocation bookkeeping) for every PNG pixel.
    pub(crate) fn install_initial_surface32_pixels(&mut self, pixels: Vec<u32>) {
        debug_assert_eq!(pixels.len(), self.width as usize * self.height as usize);
        let mut surface32_pixels = HashMap::with_capacity(pixels.len());
        for (slot, color) in pixels.into_iter().enumerate() {
            // C4Surface::SetPixDw discards stale RGB for a fully transparent
            // source pixel (C4Surface.cpp:726-728).
            let color = if color >> 24 == 0xff {
                0xff00_0000
            } else {
                color
            };
            surface32_pixels.insert(slot, color);
        }
        self.surface32_pixels = Arc::new(surface32_pixels);
        self.surface32_revision = 0;
        self.surface32_render_token = 0;
        self.surface32_dirty_generations.clear();
        self.pending_surface32_relights.clear();
    }

    fn slot(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        Some(y as usize * self.width as usize + x as usize)
    }

    /// The raw pixel byte (in bounds only; callers apply GetPix border
    /// rules themselves).
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    pub fn texture_names(&self) -> &[Option<String>] {
        &self.texture_names
    }

    /// `Mat2PixColDefault(MVehic)` — the first texmap slot carrying the
    /// Vehicle material (the byte C4SolidMask bakes, `MCVehic`).
    pub fn vehicle_byte(&self) -> Option<u8> {
        self.material_names
            .iter()
            .position(|name| {
                name.as_deref()
                    .is_some_and(|name| clonk_resources::material::c4_names_equal(name, "Vehicle"))
            })
            .map(|index| index as u8)
    }

    /// Raw plane write (C4SolidMask's _SBackPix): bumps the revision on change.
    pub fn write_byte(&mut self, x: i32, y: i32, byte: u8) {
        self.set_byte_impl(x, y, byte, PixelWrite::Raw);
    }

    /// `CSurface8::Circle` (`src/StdSurface8.cpp:231-239`). Its bottom and
    /// right edges are deliberately exclusive: radius one writes exactly
    /// `(x-1,y)` and `(x,y)`.
    fn draw_editor_circle(&mut self, x: i32, y: i32, radius: i32, byte: u8) {
        if radius <= 0 {
            return;
        }
        for y_offset in -radius..radius {
            let radius_squared = i64::from(radius) * i64::from(radius);
            let y_squared = i64::from(y_offset) * i64::from(y_offset);
            let half_width = ((radius_squared - y_squared) as f32).sqrt() as i32;
            for x_counter in (0..half_width.saturating_mul(2)).rev() {
                self.write_byte(
                    x.wrapping_sub(half_width).wrapping_add(x_counter),
                    y.wrapping_add(y_offset),
                    byte,
                );
            }
        }
    }

    /// The exact `ForLine` major-axis walk used by C4Landscape::DrawLine;
    /// every visited center receives the asymmetric Surface8 circle above.
    fn draw_editor_line(
        &mut self,
        mut x1: i32,
        mut y1: i32,
        mut x2: i32,
        mut y2: i32,
        radius: i32,
        byte: u8,
    ) {
        if x2.wrapping_sub(x1).abs() < y2.wrapping_sub(y1).abs() {
            if y1 > y2 {
                std::mem::swap(&mut x1, &mut x2);
                std::mem::swap(&mut y1, &mut y2);
            }
            let x_increment = if x2 > x1 { 1 } else { -1 };
            let dy = y2.wrapping_sub(y1);
            let dx = x2.wrapping_sub(x1).abs();
            let mut decision = dx.wrapping_mul(2).wrapping_sub(dy);
            let advance_both = dx.wrapping_sub(dy).wrapping_mul(2);
            let advance_major = dx.wrapping_mul(2);
            let mut x = x1;
            self.draw_editor_circle(x, y1, radius, byte);
            let mut y = y1.wrapping_add(1);
            while y <= y2 {
                if decision >= 0 {
                    x = x.wrapping_add(x_increment);
                    decision = decision.wrapping_add(advance_both);
                } else {
                    decision = decision.wrapping_add(advance_major);
                }
                self.draw_editor_circle(x, y, radius, byte);
                y = y.wrapping_add(1);
            }
        } else {
            if x1 > x2 {
                std::mem::swap(&mut x1, &mut x2);
                std::mem::swap(&mut y1, &mut y2);
            }
            let y_increment = if y2 > y1 { 1 } else { -1 };
            let dx = x2.wrapping_sub(x1);
            let dy = y2.wrapping_sub(y1).abs();
            let mut decision = dy.wrapping_mul(2).wrapping_sub(dx);
            let advance_both = dy.wrapping_sub(dx).wrapping_mul(2);
            let advance_major = dy.wrapping_mul(2);
            let mut y = y1;
            self.draw_editor_circle(x1, y, radius, byte);
            let mut x = x1.wrapping_add(1);
            while x <= x2 {
                if decision >= 0 {
                    y = y.wrapping_add(y_increment);
                    decision = decision.wrapping_add(advance_both);
                } else {
                    decision = decision.wrapping_add(advance_major);
                }
                self.draw_editor_circle(x, y, radius, byte);
                x = x.wrapping_add(1);
            }
        }
    }

    fn draw_editor_box(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, byte: u8) {
        let (left, right) = (x1.min(x2), x1.max(x2));
        let (top, bottom) = (y1.min(y2), y1.max(y2));
        for y in top..=bottom {
            for x in left..=right {
                self.write_byte(x, y, byte);
            }
        }
    }

    pub fn material_names(&self) -> &[Option<String>] {
        &self.material_names
    }

    /// Identity of the texmap name tables, or zero for a grid that came from a
    /// save and has not recomputed one. Two grids with nonzero identities
    /// share them exactly when their `material_names` and `texture_names` are
    /// equal.
    pub fn texmap_identity(&self) -> u64 {
        self.texmap_identity.0
    }

    /// Whether two grids describe the same texmap. Uses the cached identity
    /// when both sides have one and falls back to the tables themselves when
    /// either came from a save.
    fn texmap_tables_match(&self, other: &Self) -> bool {
        match (self.texmap_identity.0, other.texmap_identity.0) {
            (0, _) | (_, 0) => {
                self.material_names == other.material_names
                    && self.texture_names == other.texture_names
            }
            (left, right) => left == right,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the exact bounded cache rectangles connecting `previous` to
    /// this grid, or `None` when the grids are unrelated/incompatible and the
    /// frontend must rebuild its persistent RGBA surface. Holding `previous`
    /// also keeps its byte `Arc` shared, so the next write starts a distinct
    /// COW generation that cloned snapshots can identify safely.
    pub fn render_dirty_rects_since(&self, previous: &Self) -> Option<Vec<PixelGridDirtyRect>> {
        if (self.width, self.height) != (previous.width, previous.height)
            || !self.texmap_tables_match(previous)
        {
            return None;
        }
        let same_surface8 =
            (self.revision, self.render_token) == (previous.revision, previous.render_token);
        if same_surface8
            && !Arc::ptr_eq(&self.bytes, &previous.bytes)
            && self.bytes.as_slice() != previous.bytes.as_slice()
        {
            return None;
        }
        let mut rects = Self::render_lineage_dirty_rects(
            self.revision,
            self.render_token,
            &self.dirty_generations,
            previous.revision,
            previous.render_token,
            true,
        )?;
        let same_surface32 = (self.surface32_revision, self.surface32_render_token)
            == (previous.surface32_revision, previous.surface32_render_token);
        if same_surface32
            && !Arc::ptr_eq(&self.surface32_pixels, &previous.surface32_pixels)
            && self.surface32_pixels.as_ref() != previous.surface32_pixels.as_ref()
        {
            return None;
        }
        rects.extend(Self::render_lineage_dirty_rects(
            self.surface32_revision,
            self.surface32_render_token,
            &self.surface32_dirty_generations,
            previous.surface32_revision,
            previous.surface32_render_token,
            false,
        )?);
        Some(rects)
    }

    fn render_lineage_dirty_rects(
        current_revision: u64,
        current_token: u64,
        generations: &VecDeque<PixelGridDirtyGeneration>,
        previous_revision: u64,
        previous_token: u64,
        cap_set_pix_rects: bool,
    ) -> Option<Vec<PixelGridDirtyRect>> {
        if (current_revision, current_token) == (previous_revision, previous_token) {
            return Some(Vec::new());
        }

        let mut revision = previous_revision;
        let mut token = previous_token;
        let mut rects = Vec::new();
        for generation in generations {
            if (generation.base_revision, generation.base_token) != (revision, token) {
                continue;
            }
            if cap_set_pix_rects {
                for rect in generation.rects() {
                    PixelGridDirtyGeneration::add_capped_rect(&mut rects, rect);
                }
            } else {
                rects.extend(generation.rects());
            }
            revision = generation.revision;
            token = generation.token;
            if (revision, token) == (current_revision, current_token) {
                return Some(rects);
            }
        }
        None
    }

    pub fn byte_at(&self, x: i32, y: i32) -> Option<u8> {
        self.slot(x, y).map(|slot| self.bytes[slot])
    }

    /// Raw C4 packed color written directly to the presentation-only
    /// Surface32 at this coordinate, if one has not since been relit.
    pub fn surface32_pixel_at(&self, x: i32, y: i32) -> Option<u32> {
        let slot = self.slot(x, y)?;
        self.surface32_pixels.get(&slot).copied()
    }

    pub fn has_surface32_pixels(&self) -> bool {
        !self.surface32_pixels.is_empty()
    }

    fn density_of(&self, byte: u8) -> i32 {
        self.densities
            .get((byte & 0x7f) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// `Pix2Dens[byte & 0x7f]` for presentation consumers that already hold
    /// a Surface8 byte. Keeping this lookup separate from [`Self::density_at`]
    /// lets retained render caches classify a changed plane without repeating
    /// coordinate validation for every output they derive from the same byte.
    pub fn density_of_byte(&self, byte: u8) -> i32 {
        self.density_of(byte)
    }

    pub fn density_at(&self, x: i32, y: i32) -> Option<i32> {
        self.byte_at(x, y).map(|byte| self.density_of(byte))
    }

    pub fn material_id_at(&self, x: i32, y: i32) -> Option<MaterialId> {
        let byte = self.byte_at(x, y)?;
        self.materials
            .get((byte & 0x7f) as usize)
            .copied()
            .flatten()
    }

    /// Resolve `Pix2Mat` once the engine material ids exist.
    pub fn resolve_materials(&mut self, mut lookup: impl FnMut(&str) -> Option<MaterialId>) {
        self.materials = self
            .material_names
            .iter()
            .map(|name| name.as_deref().and_then(&mut lookup))
            .collect();
        self.rebuild_material_counts();
    }

    fn material_for_byte(&self, byte: u8) -> Option<MaterialId> {
        self.materials
            .get((byte & 0x7f) as usize)
            .copied()
            .flatten()
    }

    fn rebuild_material_counts(&mut self) {
        #[cfg(test)]
        MATERIAL_COUNT_FULL_REBUILDS.with(|rebuilds| rebuilds.set(rebuilds.get() + 1));
        let count = self
            .materials
            .iter()
            .flatten()
            .map(|material| material.index() + 1)
            .max()
            .unwrap_or(0);
        self.material_counts.clear();
        self.material_counts.resize(count, 0);
        for &byte in self.bytes.iter() {
            if let Some(material) = self.material_for_byte(byte) {
                self.material_counts[material.index()] =
                    self.material_counts[material.index()].wrapping_add(1);
            }
        }
    }

    fn material_count(&self, material: MaterialId) -> u32 {
        self.material_counts
            .get(material.index())
            .copied()
            .unwrap_or(0)
    }

    /// C4Landscape::UpdateMatCnt subtracts the old contents of a change
    /// rectangle in PrepareChange and adds the new contents in FinishChange.
    /// Keep the same bounded work for bulk Surface8 writers instead of
    /// recounting the complete landscape after every small polygon/chunk.
    fn adjust_material_counts_in_rect(&mut self, rect: PixelGridDirtyRect, add: bool) {
        if self.material_counts.is_empty() && self.materials.iter().any(Option::is_some) {
            self.rebuild_material_counts();
        }
        for y in rect.y..rect.y.saturating_add(rect.height) {
            let row = y as usize * self.width as usize;
            for x in rect.x..rect.x.saturating_add(rect.width) {
                let byte = self.bytes[row + x as usize];
                let Some(material) = self.material_for_byte(byte) else {
                    continue;
                };
                let count = &mut self.material_counts[material.index()];
                *count = if add {
                    count.wrapping_add(1)
                } else {
                    count.wrapping_sub(1)
                };
            }
        }
    }

    /// Keep the pixel lookup tables aligned with the mutable runtime texmap.
    /// Existing resolved material ids follow their material NAME to newly
    /// allocated texture slots; no pixel byte or render revision changes.
    fn sync_runtime_texmap(&mut self, texmap: &RuntimeTexMapState) {
        if self.densities == texmap.densities
            && self.material_names == texmap.material_names
            && self.texture_names == texmap.texture_names
        {
            return;
        }
        let old_materials = self
            .material_names
            .iter()
            .zip(&self.materials)
            .filter_map(|(name, material)| Some((name.as_deref()?, (*material)?)))
            .collect::<Vec<_>>();
        let materials = texmap
            .material_names
            .iter()
            .map(|name| {
                name.as_deref().and_then(|name| {
                    old_materials
                        .iter()
                        .find(|(old_name, _)| {
                            clonk_resources::material::c4_names_equal(old_name, name)
                        })
                        .map(|(_, material)| *material)
                })
            })
            .collect::<Vec<_>>();
        let material_mapping_changed = self.materials != materials;
        self.materials = materials;
        self.densities.clone_from(&texmap.densities);
        self.material_names.clone_from(&texmap.material_names);
        self.texture_names.clone_from(&texmap.texture_names);
        // The only site that moves either name table after construction.
        self.texmap_identity = texmap_identity(&self.material_names, &self.texture_names);
        if material_mapping_changed {
            self.rebuild_material_counts();
        }
    }

    fn record_render_change(
        &mut self,
        base_revision: u64,
        base_token: u64,
        rect: PixelGridDirtyRect,
        storage_was_shared: bool,
    ) {
        Self::record_lineage_change(
            &mut self.dirty_generations,
            self.revision,
            self.render_token,
            base_revision,
            base_token,
            rect,
            storage_was_shared,
        );
    }

    fn record_surface32_change(
        &mut self,
        base_revision: u64,
        base_token: u64,
        rect: PixelGridDirtyRect,
        storage_was_shared: bool,
    ) {
        Self::record_lineage_change(
            &mut self.surface32_dirty_generations,
            self.surface32_revision,
            self.surface32_render_token,
            base_revision,
            base_token,
            rect,
            storage_was_shared,
        );
    }

    fn record_lineage_change(
        generations: &mut VecDeque<PixelGridDirtyGeneration>,
        revision: u64,
        token: u64,
        base_revision: u64,
        base_token: u64,
        rect: PixelGridDirtyRect,
        storage_was_shared: bool,
    ) {
        let can_extend = !storage_was_shared
            && generations.back().is_some_and(|generation| {
                (generation.revision, generation.token) == (base_revision, base_token)
            });
        if can_extend {
            let generation = generations
                .back_mut()
                .expect("checked dirty generation exists");
            generation.revision = revision;
            generation.token = token;
            generation.add_rect(rect);
        } else {
            generations.push_back(PixelGridDirtyGeneration {
                base_revision,
                revision,
                base_token,
                token,
                rect,
                rects: Vec::new(),
            });
        }
        while generations.len() > MAX_RENDER_DIRTY_GENERATIONS {
            generations.pop_front();
        }
    }

    fn advance_pixel_render_token(
        token: u64,
        revision: u64,
        x: i32,
        y: i32,
        old: u8,
        new: u8,
    ) -> u64 {
        let token = render_token_bytes(token, revision.to_le_bytes());
        let token = render_token_bytes(token, x.to_le_bytes());
        let token = render_token_bytes(token, y.to_le_bytes());
        render_token_bytes(token, [old, new])
    }

    fn advance_surface32_render_token(
        token: u64,
        revision: u64,
        x: i32,
        y: i32,
        old: Option<u32>,
        new: Option<u32>,
    ) -> u64 {
        let token = render_token_bytes(token, [0x32]);
        let token = render_token_bytes(token, revision.to_le_bytes());
        let token = render_token_bytes(token, x.to_le_bytes());
        let token = render_token_bytes(token, y.to_le_bytes());
        let token = render_token_bytes(token, [u8::from(old.is_some())]);
        let token = render_token_bytes(token, old.unwrap_or_default().to_le_bytes());
        let token = render_token_bytes(token, [u8::from(new.is_some())]);
        render_token_bytes(token, new.unwrap_or_default().to_le_bytes())
    }

    /// C4Surface::SetPixDw on the landscape's 32-bit presentation surface.
    /// The packed high byte is legacy transparency (0 opaque, 255 clear).
    pub fn set_surface32_pixel(&mut self, x: i32, y: i32, color: u32) -> bool {
        let Some(slot) = self.slot(x, y) else {
            return false;
        };
        if self
            .pending_surface32_relights
            .iter()
            .any(|rect| rect.contains(x, y))
        {
            // C++ performs this write immediately, but Draw::DoRelights
            // rebuilds the queued region before it can be presented.
            return true;
        }
        // C4Surface::SetPixDw canonicalizes fully transparent pixels so stale
        // RGB data cannot leak through later filtering.
        let color = if color >> 24 == 0xff {
            0xff00_0000
        } else {
            color
        };
        let old = self.surface32_pixels.get(&slot).copied();
        if old == Some(color) {
            return true;
        }

        let storage_was_shared = Arc::strong_count(&self.surface32_pixels) > 1;
        let base_revision = self.surface32_revision;
        let base_token = self.surface32_render_token;
        Arc::make_mut(&mut self.surface32_pixels).insert(slot, color);
        self.surface32_revision = self.surface32_revision.wrapping_add(1);
        self.surface32_render_token = Self::advance_surface32_render_token(
            base_token,
            self.surface32_revision,
            x,
            y,
            old,
            Some(color),
        );
        self.record_surface32_change(
            base_revision,
            base_token,
            PixelGridDirtyRect::single(x, y),
            storage_was_shared,
        );
        true
    }

    /// A later material relight rebuilds Surface32 from Surface8 in this
    /// expanded region, replacing any cosmetic SetLandscapePixel writes.
    fn clear_surface32_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        let Some(bounds) = RasterChangeRect::new(x, y, width, height)
            .clipped_to(self.width as i32, self.height as i32)
        else {
            return;
        };
        if self.surface32_pixels.is_empty() {
            return;
        }
        let area = i64::from(bounds.width) * i64::from(bounds.height);
        let mut removed = if area <= self.surface32_pixels.len() as i64 {
            let mut removed = Vec::new();
            for pixel_y in bounds.y..bounds.y + bounds.height {
                for pixel_x in bounds.x..bounds.x + bounds.width {
                    let slot = pixel_y as usize * self.width as usize + pixel_x as usize;
                    if let Some(&color) = self.surface32_pixels.get(&slot) {
                        removed.push((slot, pixel_x, pixel_y, color));
                    }
                }
            }
            removed
        } else {
            self.surface32_pixels
                .iter()
                .filter_map(|(&slot, &color)| {
                    let pixel_x = (slot % self.width as usize) as i32;
                    let pixel_y = (slot / self.width as usize) as i32;
                    (pixel_x >= bounds.x
                        && pixel_x < bounds.x + bounds.width
                        && pixel_y >= bounds.y
                        && pixel_y < bounds.y + bounds.height)
                        .then_some((slot, pixel_x, pixel_y, color))
                })
                .collect::<Vec<_>>()
        };
        if removed.is_empty() {
            return;
        }
        removed.sort_unstable_by_key(|&(slot, _, _, _)| slot);

        let storage_was_shared = Arc::strong_count(&self.surface32_pixels) > 1;
        let base_revision = self.surface32_revision;
        let base_token = self.surface32_render_token;
        let pixels = Arc::make_mut(&mut self.surface32_pixels);
        for &(slot, _, _, _) in &removed {
            pixels.remove(&slot);
        }
        self.surface32_revision = self.surface32_revision.wrapping_add(1);
        let mut token = base_token;
        for &(_, pixel_x, pixel_y, color) in &removed {
            token = Self::advance_surface32_render_token(
                token,
                self.surface32_revision,
                pixel_x,
                pixel_y,
                Some(color),
                None,
            );
        }
        self.surface32_render_token = token;
        self.record_surface32_change(base_revision, base_token, bounds.into(), storage_was_shared);
    }

    fn schedule_surface32_relight_around(&mut self, x: i32, y: i32) {
        // C4Landscape::Relight expands a changed Surface8 pixel by
        // C4LS_MaxLightDistX/Y = 1/8 before rebuilding Surface32.
        let x = x.saturating_sub(1);
        let y = y.saturating_sub(8);
        self.clear_surface32_rect(x, y, 3, 17);
        let Some(rect) = RasterChangeRect::new(x, y, 3, 17)
            .clipped_to(self.width as i32, self.height as i32)
            .map(PixelGridDirtyRect::from)
        else {
            return;
        };
        if let Some(existing) = self
            .pending_surface32_relights
            .iter_mut()
            .find(|existing| existing.overlaps(rect))
        {
            *existing = existing.union(rect);
        } else if self.pending_surface32_relights.len() < MAX_RENDER_DIRTY_GENERATIONS {
            self.pending_surface32_relights.push(rect);
        } else if let Some(last) = self.pending_surface32_relights.last_mut() {
            *last = last.union(rect);
        }
    }

    fn relight_surface32_rect(&mut self, bounds: RasterChangeRect) {
        self.clear_surface32_rect(
            bounds.x.saturating_sub(1),
            bounds.y.saturating_sub(8),
            bounds.width.saturating_add(2),
            bounds.height.saturating_add(16),
        );
    }

    fn finish_pending_surface32_relights(&mut self) {
        self.pending_surface32_relights.clear();
    }

    fn advance_rect_render_token(
        &self,
        token: u64,
        revision: u64,
        rect: PixelGridDirtyRect,
    ) -> u64 {
        let token = render_token_bytes(token, revision.to_le_bytes());
        let token = render_token_bytes(token, rect.x.to_le_bytes());
        let token = render_token_bytes(token, rect.y.to_le_bytes());
        let token = render_token_bytes(token, rect.width.to_le_bytes());
        let mut token = render_token_bytes(token, rect.height.to_le_bytes());
        for y in rect.y..rect.y.saturating_add(rect.height) {
            let start = (y * self.width + rect.x) as usize;
            let end = start.saturating_add(rect.width as usize);
            token = render_token_bytes(token, self.bytes[start..end].iter().copied());
        }
        token
    }

    /// CSurface8::Polygon on the authoritative plane. The shared chunky-map
    /// rasterizer is already a line-faithful port of StdSurface8.cpp:306-404;
    /// move the byte plane through it without cloning a full landscape.
    fn draw_polygon(&mut self, vertices: &[(i32, i32)], byte: u8) {
        let Some(rect) = PixelGridDirtyRect::from_vertices(vertices, self.width, self.height)
        else {
            return;
        };
        self.clear_surface32_rect(
            (rect.x as i32).saturating_sub(1),
            (rect.y as i32).saturating_sub(8),
            (rect.width as i32).saturating_add(2),
            (rect.height as i32).saturating_add(16),
        );
        let storage_was_shared = Arc::strong_count(&self.bytes) > 1;
        let base_revision = self.revision;
        let base_token = self.render_token;
        self.adjust_material_counts_in_rect(rect, false);
        let bytes = mem::take(Arc::make_mut(&mut self.bytes));
        let mut surface =
            crate::chunky::Surface8::from_bytes(self.width as i32, self.height as i32, bytes);
        crate::chunky::polygon(&mut surface, vertices, byte);
        self.bytes = Arc::new(surface.into_bytes());
        self.adjust_material_counts_in_rect(rect, true);
        self.revision = self.revision.wrapping_add(1);
        self.render_token = self.advance_rect_render_token(base_token, self.revision, rect);
        self.record_render_change(base_revision, base_token, rect, storage_was_shared);
    }

    /// C4Landscape::DrawChunks' single clipped Surface8 batch
    /// (C4Landscape.cpp:2419-2442). The synchronized Random(1000) results
    /// were sampled by the script host; DrawChunk's remaining jitter is the
    /// deterministic ChunkyRandom chain derived from each result and MapSeed.
    #[allow(clippy::too_many_arguments)]
    fn draw_material_chunks(
        &mut self,
        clip: RasterChangeRect,
        origin: Vector2,
        width: i32,
        height: i32,
        count_x: i32,
        count_y: i32,
        byte: u8,
        shape: crate::chunky::ChunkShape,
        random_offsets: &[i32],
        map_seed: i32,
    ) {
        let Some(rect) = clip
            .clipped_to(self.width as i32, self.height as i32)
            .map(PixelGridDirtyRect::from)
        else {
            return;
        };

        let storage_was_shared = Arc::strong_count(&self.bytes) > 1;
        let base_revision = self.revision;
        let base_token = self.render_token;
        self.adjust_material_counts_in_rect(rect, false);
        let bytes = mem::take(Arc::make_mut(&mut self.bytes));
        let mut surface =
            crate::chunky::Surface8::from_bytes(self.width as i32, self.height as i32, bytes);
        surface.clip(
            clip.x,
            clip.y,
            clip.x.saturating_add(clip.width).saturating_sub(1),
            clip.y.saturating_add(clip.height).saturating_sub(1),
        );

        let chunk_width = width / count_x;
        let chunk_height = height / count_y;
        let mut offsets = random_offsets.iter().copied();
        for x in 0..count_x {
            for y in 0..count_y {
                let random_offset = offsets
                    .next()
                    .expect("validated DrawMatChunks random offset count");
                crate::chunky::draw_chunk(
                    &mut surface,
                    origin.x.wrapping_add(width.wrapping_mul(x) / count_x),
                    origin.y.wrapping_add(height.wrapping_mul(y) / count_y),
                    chunk_width,
                    chunk_height,
                    byte,
                    shape,
                    random_offset,
                    map_seed,
                );
            }
        }
        debug_assert!(offsets.next().is_none());

        self.bytes = Arc::new(surface.into_bytes());
        self.adjust_material_counts_in_rect(rect, true);
        self.revision = self.revision.wrapping_add(1);
        self.render_token = self.advance_rect_render_token(base_token, self.revision, rect);
        self.record_render_change(base_revision, base_token, rect, storage_was_shared);
    }

    /// The first texmap index carrying the given material (the
    /// Mat2PixColDefault stand-in for grid writes).
    fn byte_for_material(&self, material: MaterialId) -> Option<u8> {
        self.materials
            .iter()
            .position(|slot| *slot == Some(material))
            .map(|index| index as u8)
    }

    fn name_for_material(&self, material: MaterialId) -> Option<&str> {
        self.material_names
            .iter()
            .zip(&self.materials)
            .find_map(|(name, slot)| {
                (*slot == Some(material))
                    .then_some(name.as_deref())
                    .flatten()
            })
    }

    /// Any solid texmap byte (fallback fill when no material is known).
    fn any_solid_byte(&self) -> Option<u8> {
        self.densities
            .iter()
            .position(|&density| density >= C4M_SOLID)
            .map(|index| index as u8)
    }

    fn set_byte(&mut self, x: i32, y: i32, byte: u8) {
        self.set_byte_impl(x, y, byte, PixelWrite::SetPix);
    }

    /// FnDrawVolcanoBranch's raw C4Landscape::SetPix loop
    /// (C4Script.cpp:2500-2509). This deliberately does not use the
    /// PrepareChange/FinishChange transaction: each changed pixel queues its
    /// own deferred relight, preserves the pixel's current IFT bit, and may
    /// overwrite an active solid-mask marker exactly like SetPix.
    fn draw_volcano_branch(
        &mut self,
        from: Vector2,
        to: Vector2,
        size: i32,
        material_byte: u8,
    ) -> Option<Range<usize>> {
        let half_size = size / 2;
        if to.y >= from.y || half_size <= 0 {
            return None;
        }

        let denominator = to.y.wrapping_sub(from.y);
        let mut first_column = self.width as i32;
        let mut last_column = -1;
        for y in to.y..from.y {
            let center_x = from.x.wrapping_add(
                to.x.wrapping_sub(from.x)
                    .wrapping_mul(y.wrapping_sub(from.y))
                    .wrapping_div(denominator),
            );
            let start_x = center_x.wrapping_sub(half_size);
            let end_x = center_x.wrapping_add(half_size);
            for x in start_x..end_x {
                let Some(current) = self.byte_at(x, y) else {
                    continue;
                };
                self.set_byte(x, y, material_byte | (current & 0x80));
                first_column = first_column.min(x);
                last_column = last_column.max(x);
            }
        }

        (last_column >= first_column)
            .then_some(first_column as usize..last_column.saturating_add(1) as usize)
    }

    fn set_byte_impl(&mut self, x: i32, y: i32, byte: u8, write: PixelWrite) {
        if let Some(slot) = self.slot(x, y) {
            let old = self.bytes[slot];
            if old == byte {
                return;
            }
            if write == PixelWrite::SetPix {
                self.schedule_surface32_relight_around(x, y);
            }
            if self.material_counts.is_empty() && self.materials.iter().any(Option::is_some) {
                self.rebuild_material_counts();
            }
            let old_material = self.material_for_byte(old);
            let new_material = self.material_for_byte(byte);
            if old_material != new_material {
                if let Some(old_material) = old_material {
                    if let Some(count) = self.material_counts.get_mut(old_material.index()) {
                        *count = count.wrapping_sub(1);
                    }
                }
                if let Some(new_material) = new_material {
                    if self.material_counts.len() <= new_material.index() {
                        self.material_counts.resize(new_material.index() + 1, 0);
                    }
                    self.material_counts[new_material.index()] =
                        self.material_counts[new_material.index()].wrapping_add(1);
                }
            }
            let storage_was_shared = Arc::strong_count(&self.bytes) > 1;
            let base_revision = self.revision;
            let base_token = self.render_token;
            Arc::make_mut(&mut self.bytes)[slot] = byte;
            self.revision = self.revision.wrapping_add(1);
            self.render_token =
                Self::advance_pixel_render_token(base_token, self.revision, x, y, old, byte);
            self.record_render_change(
                base_revision,
                base_token,
                PixelGridDirtyRect::single(x, y),
                storage_was_shared,
            );
        }
    }

    fn clear_pixel(&mut self, x: i32, y: i32) {
        self.set_byte(x, y, 0);
    }

    /// Fixture fallback for the default tunnel byte when no retained runtime
    /// texture map exists.
    fn tunnel_byte(&self) -> u8 {
        self.material_names
            .iter()
            .position(|name| {
                name.as_deref()
                    .is_some_and(|name| clonk_resources::material::c4_names_equal(name, "Tunnel"))
            })
            .map(|index| index as u8)
            .unwrap_or(0)
    }

    /// C4Landscape::ClearPix (C4Landscape.cpp:880-888): an IFT pixel
    /// clears to the tunnel background (+IFT); a surface pixel to sky.
    pub fn clear_pix(&mut self, x: i32, y: i32) {
        self.clear_pix_with_tunnel(x, y, None);
    }

    fn clear_pix_with_tunnel(&mut self, x: i32, y: i32, tunnel_byte: Option<u8>) {
        let Some(byte) = self.byte_at(x, y) else {
            return;
        };
        if byte & 0x80 != 0 {
            self.set_byte(
                x,
                y,
                tunnel_byte.unwrap_or_else(|| self.tunnel_byte()) | 0x80,
            );
        } else {
            self.set_byte(x, y, 0);
        }
    }

    fn fill_band(&mut self, x: i32, from_y: i32, to_y: i32, byte: u8) {
        for y in from_y.max(0)..to_y.min(self.height as i32) {
            self.set_byte(x, y, byte);
        }
    }

    fn clear_band(&mut self, x: i32, from_y: i32, to_y: i32) {
        self.fill_band(x, from_y, to_y, 0);
    }

    fn derived_column(&self, x: usize) -> Option<RasterColumnSummary> {
        if x >= self.width as usize {
            return None;
        }
        let height = self.height as i32;
        let width = self.width as usize;
        let byte_at = |y: i32| self.bytes[y as usize * width + x];
        let surface = (0..height)
            .find(|&y| self.density_of(byte_at(y)) >= C4M_SOLID)
            .unwrap_or(height);
        let mut liquid_segments = Vec::new();
        let mut tunnel_ranges = Vec::new();
        let mut liquid_run = None;
        let mut tunnel_start = None;
        for y in 0..=height {
            let pixel = (y < height).then(|| byte_at(y));
            let liquid_material = pixel.and_then(|byte| {
                (C4M_LIQUID..C4M_SOLID)
                    .contains(&self.density_of(byte))
                    .then(|| {
                        self.materials
                            .get((byte & 0x7f) as usize)
                            .copied()
                            .flatten()
                    })
            });
            match (liquid_material, liquid_run) {
                (Some(material), None) => liquid_run = Some((y, material)),
                (Some(material), Some((start, previous))) if material != previous => {
                    liquid_segments.push(LiquidSegment::with_material(start, y - 1, previous));
                    liquid_run = Some((y, material));
                }
                (None, Some((start, material))) => {
                    liquid_segments.push(LiquidSegment::with_material(start, y - 1, material));
                    liquid_run = None;
                }
                _ => {}
            }
            let tunnel = pixel.map(|byte| byte & 0x80 != 0).unwrap_or(false);
            match (tunnel, tunnel_start) {
                (true, None) => tunnel_start = Some(y),
                (false, Some(start)) => {
                    tunnel_ranges.push((start, y - 1));
                    tunnel_start = None;
                }
                _ => {}
            }
        }
        Some(RasterColumnSummary {
            surface,
            liquid_segments,
            tunnel_ranges,
        })
    }
}

struct RasterColumnSummary {
    surface: i32,
    liquid_segments: Vec<LiquidSegment>,
    tunnel_ranges: Vec<(i32, i32)>,
}

/// The material properties needed when a runtime landscape operation adds a
/// texture-map entry. This is deliberately narrower than the complete
/// resource definition: `C4TextureMap::GetIndex` only needs the material's
/// density and `MapChunkType` after validating its name (C4Texture.cpp:319-345).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RuntimeTexMapMaterial {
    pub(crate) name: String,
    pub(crate) density: i32,
    pub(crate) shape: crate::chunky::ChunkShape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeTexMapLookup {
    pub(crate) material_texture: String,
    pub(crate) default_texture: Option<String>,
    pub(crate) eager_index: u8,
}

/// The live C4 texture-map state needed by post-initialization raster writes.
/// Unlike [`PixelGrid`], this retains the raw texture names used for pair
/// matching, the available texture/material inventories, and the exact
/// `DefaultMatTex` result for every material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct RuntimeTexMapState {
    pub(crate) densities: Vec<i32>,
    pub(crate) material_names: Vec<Option<String>>,
    pub(crate) texture_names: Vec<Option<String>>,
    pub(crate) match_texture_names: Vec<Option<String>>,
    pub(crate) shapes: Vec<Option<crate::chunky::ChunkShape>>,
    pub(crate) materials: Vec<RuntimeTexMapMaterial>,
    pub(crate) texture_inventory: Vec<String>,
    /// `(material name, texmap slot)` in C++ material-map order.
    #[serde(default)]
    pub(crate) default_material_entries: Vec<(String, u8)>,
    /// Numeric `BlastShiftTo`, `BelowTempConvertTo`, and
    /// `AboveTempConvertTo` slots captured by CrossMapMaterials. These remain
    /// fixed when later texture-map edits create duplicate entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) material_crossmap_entries: Vec<u8>,
    /// `C4TextureMap::fEntriesAdded`: runtime GetIndex allocations,
    /// MoveIndex and RemoveUnusedTexMapEntries make the local TexMap dirty.
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub(crate) entries_added: bool,
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub(crate) overload_materials: bool,
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub(crate) overload_textures: bool,
}

impl Default for RuntimeTexMapState {
    fn default() -> Self {
        let slots = C4M_MAX_TEX_INDEX + 1;
        Self {
            densities: vec![0; slots],
            material_names: vec![None; slots],
            texture_names: vec![None; slots],
            match_texture_names: vec![None; slots],
            shapes: vec![None; slots],
            materials: Vec::new(),
            texture_inventory: Vec::new(),
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            entries_added: false,
            overload_materials: false,
            overload_textures: false,
        }
    }
}

impl RuntimeTexMapState {
    fn replay_section_lookups(
        mut live: Self,
        lookups: &[RuntimeTexMapLookup],
    ) -> (Self, [u8; 128]) {
        let mut remap = std::array::from_fn(|slot| slot as u8);
        for lookup in lookups {
            let destination =
                live.get_index_mat_tex(&lookup.material_texture, lookup.default_texture.as_deref());
            let source = usize::from(lookup.eager_index & 0x7f);
            if source != 0 {
                remap[source] = destination & 0x7f;
            }
        }
        (live, remap)
    }

    pub(crate) fn texture_exists(&self, name: &str) -> bool {
        self.texture_inventory
            .iter()
            .any(|texture| clonk_resources::material::c4_names_equal(texture, name))
    }

    pub(crate) fn material(&self, name: &str) -> Option<&RuntimeTexMapMaterial> {
        self.materials
            .iter()
            .find(|material| clonk_resources::material::c4_names_equal(&material.name, name))
    }

    /// Return the numeric slot captured by `CrossMapMaterials`, if this is a
    /// current-format state with a matching material ledger. `Some(0)` is a
    /// real failed cross-map and must not fall back to a later name lookup.
    fn frozen_crossmap_entry(
        &self,
        materials: &MaterialSet,
        source: MaterialId,
        target_spec: &str,
    ) -> Option<u8> {
        let runtime_material = self.materials.get(source.index())?;
        let material = materials.get_by_id(source)?;
        if !clonk_resources::material::c4_names_equal(&runtime_material.name, material.name()) {
            return None;
        }
        let ordinal = materials.crossmap_entry_ordinal(source, target_spec)?;
        self.material_crossmap_entries.get(ordinal).copied()
    }

    /// The already-cross-mapped result of `C4TextureMap::GetIndexMatTex`
    /// (C4Texture.cpp:346-369). Scenario activation has allocated every
    /// `BlastShiftTo` pair before simulation starts, so runtime mutation must
    /// recover that exact slot rather than the first slot with the material.
    pub(crate) fn resolved_index_mat_tex(&self, material_texture: &str) -> u8 {
        let material_texture = clonk_resources::material::c4_c_string(material_texture);
        if let Some((material, texture)) = material_texture.split_once('-') {
            return self
                .material_names
                .iter()
                .zip(&self.match_texture_names)
                .enumerate()
                .skip(1)
                .take(C4M_MAX_TEX_INDEX - 1)
                .find(|(_, (slot_material, slot_texture))| {
                    slot_material.as_deref().is_some_and(|name| {
                        clonk_resources::material::c4_names_equal(name, material)
                    }) && slot_texture.as_deref().is_some_and(|name| {
                        clonk_resources::material::c4_names_equal(name, texture)
                    })
                })
                .map_or(0, |(slot, _)| slot as u8);
        }
        self.default_material_entry(&material_texture).unwrap_or(0)
    }

    /// C4TextureMap::GetIndex (C4Texture.cpp:319-345), relocated from the
    /// activation-only classifier so runtime landscape writes can share the
    /// same retained texmap state while preserving its lookup/allocation
    /// behavior.
    pub(crate) fn get_index(
        &mut self,
        material_name: &str,
        texture_name: Option<&str>,
        add_if_missing: bool,
    ) -> u8 {
        let material_name = clonk_resources::material::c4_c_string(material_name);
        let texture_name = texture_name.map(clonk_resources::material::c4_c_string);
        for slot in 1..C4M_MAX_TEX_INDEX {
            if let Some(existing) = &self.material_names[slot] {
                if clonk_resources::material::c4_names_equal(existing, &material_name)
                    && texture_name
                        .as_deref()
                        .map(|texture| {
                            self.match_texture_names[slot]
                                .as_deref()
                                .is_some_and(|existing| {
                                    clonk_resources::material::c4_names_equal(existing, texture)
                                })
                        })
                        .unwrap_or(true)
                {
                    return slot as u8;
                }
            }
        }
        if !add_if_missing {
            return 0;
        }
        // AddEntry rejects a null texture before mutating the slot, while
        // GetIndex's scan above still treats it as a wildcard for existing
        // material entries (src/C4Texture.cpp:119-121,319-340).
        if texture_name.is_none() {
            return 0;
        }
        let Some((density, shape)) = self
            .material(&material_name)
            .map(|material| (material.density, material.shape))
        else {
            return 0;
        };
        if let Some(texture) = texture_name.as_deref() {
            let validation_texture = if (C4M_LIQUID..C4M_SOLID).contains(&density)
                && clonk_resources::material::c4_names_equal(texture, "Smooth")
            {
                "Liquid"
            } else {
                texture
            };
            if !self.texture_exists(validation_texture) {
                return 0;
            }
        }
        let Some(slot) = (1..C4M_MAX_TEX_INDEX).find(|&slot| self.material_names[slot].is_none())
        else {
            return 0;
        };
        self.material_names[slot] = Some(material_name);
        self.match_texture_names[slot] = texture_name.clone();
        self.texture_names[slot] = texture_name;
        self.shapes[slot] = Some(shape);
        self.densities[slot] = density;
        self.entries_added = true;
        slot as u8
    }

    /// `C4Landscape::SetTextureIndex`'s defined texture-map mutation
    /// (C4Landscape.cpp:2733-2808). `MoveIndex` is a copy despite its name:
    /// the source slot remains occupied and material defaults are unchanged.
    ///
    /// The native insertion scan tests the non-null pointer returned by
    /// `GetEntry`, rather than whether that entry is null. It consequently
    /// rejects every safely representable insertion index before mutation.
    /// Its remaining 126/127 paths write beyond `Entry[126]`; keep those
    /// undefined cases contained instead of reproducing memory corruption.
    /// The optional index pair describes the defined retained-map rewrite.
    pub(crate) fn set_texture_index(
        &mut self,
        material_texture: &str,
        new_index: u8,
        insert: bool,
    ) -> (bool, Option<(u8, u8)>) {
        let material_texture = clonk_resources::material::c4_c_string(material_texture);
        if insert {
            // At 127, the empty-input native path returns true before its
            // out-of-bounds MoveIndex arms. Preserve that defined no-op.
            return (
                new_index == C4M_MAX_TEX_INDEX as u8 && material_texture.is_empty(),
                None,
            );
        }

        let new_slot = usize::from(new_index);
        if material_texture.is_empty()
            || !(1..C4M_MAX_TEX_INDEX).contains(&new_slot)
            || self
                .material_names
                .get(new_slot)
                .and_then(Option::as_ref)
                .is_some()
        {
            return (false, None);
        }

        let (material, texture) = match material_texture.split_once('-') {
            Some((material, texture)) => (material, Some(texture)),
            None => (material_texture.as_str(), None),
        };
        let old_slot = usize::from(self.get_index(material, texture, false));
        if old_slot == 0 {
            return (false, None);
        }

        self.densities[new_slot] = self.densities[old_slot];
        self.material_names[new_slot] = self.material_names[old_slot].clone();
        self.texture_names[new_slot] = self.texture_names[old_slot].clone();
        self.match_texture_names[new_slot] = self.match_texture_names[old_slot].clone();
        self.shapes[new_slot] = self.shapes[old_slot];
        self.entries_added = true;
        (true, Some((old_slot as u8, new_index)))
    }

    /// C4Landscape::RemoveUnusedTexMapEntries clears every texture-map entry
    /// that is neither present in Surface8 nor held by one of the material
    /// map's numeric texmap references (C4Landscape.cpp:2983-3007).
    /// Returns the ascending slot list so callback-local worlds can preview
    /// the synchronous removal without replacing unrelated state. The
    /// authoritative fold recomputes this list at the operation's ordered
    /// position because earlier deferred pixel writes may change usage.
    pub(crate) fn remove_unused_entries(&mut self, mut texture_usage: [bool; 128]) -> Vec<u8> {
        // Native code sets fEntriesAdded after the removal scan even when no
        // slot was cleared.
        self.entries_added = true;
        for &(_, slot) in &self.default_material_entries {
            texture_usage[usize::from(slot & 0x7f)] = true;
        }
        for &slot in &self.material_crossmap_entries {
            texture_usage[usize::from(slot & 0x7f)] = true;
        }

        let cleared_slots = (1..C4M_MAX_TEX_INDEX)
            .filter(|&slot| !texture_usage[slot])
            .map(|slot| slot as u8)
            .collect::<Vec<_>>();
        self.clear_entries(&cleared_slots);
        cleared_slots
    }

    pub(crate) fn clear_entries(&mut self, slots: &[u8]) {
        for &slot in slots {
            let slot = usize::from(slot);
            if !(1..C4M_MAX_TEX_INDEX).contains(&slot) {
                continue;
            }
            self.densities[slot] = 0;
            self.material_names[slot] = None;
            self.texture_names[slot] = None;
            self.match_texture_names[slot] = None;
            self.shapes[slot] = None;
        }
    }

    /// C4TextureMap::GetIndexMatTex (C4Texture.cpp:346-369). An explicit
    /// `Material-Texture` first tries/allocates that pair. If it fails and
    /// there is no caller-supplied default texture, C++ looks up the ORIGINAL
    /// full string as a material name; therefore `Water-Missing` does not
    /// silently fall back to Water's default entry.
    pub(crate) fn get_index_mat_tex(
        &mut self,
        material_texture: &str,
        default_texture: Option<&str>,
    ) -> u8 {
        let material_texture = clonk_resources::material::c4_c_string(material_texture);
        let default_texture = default_texture.map(clonk_resources::material::c4_c_string);
        let (material, texture) = match material_texture.split_once('-') {
            Some((material, texture)) => (material, Some(texture)),
            None => (material_texture.as_str(), None),
        };
        if let Some(texture) = texture {
            let index = self.get_index(material, Some(texture), true);
            if index != 0 {
                return index;
            }
        }
        if let Some(default_texture) = default_texture.as_deref() {
            let index = self.get_index(material, Some(default_texture), true);
            if index != 0 {
                return index;
            }
        }
        self.default_material_entry(&material_texture).unwrap_or(0)
    }

    pub(crate) fn entries_added(&self) -> bool {
        self.entries_added
    }

    fn serialize_added_entries(&self) -> Vec<u8> {
        let mut output = b"# Automatically generated texture map\r\n\
# Contains material-texture-combinations added at runtime\r\n"
            .to_vec();
        if self.overload_materials {
            output.extend_from_slice(
                b"# Import materials from global file as well\r\nOverloadMaterials\r\n",
            );
        }
        if self.overload_textures {
            output.extend_from_slice(
                b"# Import textures from global file as well\r\nOverloadTextures\r\n",
            );
        }
        output.extend_from_slice(b"\r\n");
        for slot in 0..C4M_MAX_TEX_INDEX {
            let Some(material) = self.material_names[slot].as_deref() else {
                continue;
            };
            let texture = self.match_texture_names[slot].as_deref().unwrap_or("");
            output.extend_from_slice(slot.to_string().as_bytes());
            output.push(b'=');
            let material = clonk_resources::material::c4_c_string(material);
            output.extend_from_slice(&clonk_script::c4_string_bytes(&material));
            output.push(b'-');
            let texture = clonk_resources::material::c4_c_string(texture);
            output.extend_from_slice(&clonk_script::c4_string_bytes(&texture));
            output.extend_from_slice(b"\r\n");
        }
        output
    }

    /// Append one C4Material::DefaultMatTex row. Same-load duplicate names
    /// retain distinct material slots; name lookup below deliberately finds
    /// the first/lower material index.
    pub(crate) fn set_default_material_entry(&mut self, name: &str, slot: u8) {
        self.default_material_entries
            .push((clonk_resources::material::c4_c_string(name), slot));
    }

    pub(crate) fn default_material_entry(&self, name: &str) -> Option<u8> {
        self.default_material_entries
            .iter()
            .find(|(material, _)| clonk_resources::material::c4_names_equal(material, name))
            .map(|(_, slot)| *slot)
    }

    /// `Game.Material.Map[index].DefaultMatTex` for the retained material-map
    /// order. Callers validate the index even though the original helper's
    /// direct array access is undefined for invalid script input.
    pub(crate) fn default_material_entry_by_index(&self, index: i32) -> Option<u8> {
        let index = usize::try_from(index).ok()?;
        self.materials.get(index)?;
        self.default_material_entries
            .get(index)
            .map(|(_, slot)| *slot)
    }
}

fn store_map_palette(
    texmap: &RuntimeTexMapState,
    materials: &MaterialSet,
) -> clonk_resources::bitmap::RgbPalette {
    let mut palette = [[0_u8; 3]; 256];
    palette[0] = [192, 196, 252];
    let mut set = [false; 256];
    for slot in 0..C4M_MAX_TEX_INDEX {
        let Some(material) = texmap.material_names[slot]
            .as_deref()
            .and_then(|name| materials.get(name))
        else {
            continue;
        };
        let color = material.color();
        palette[slot] = [
            color.get(6).copied().unwrap_or(0) as u8,
            color.get(7).copied().unwrap_or(0) as u8,
            color.get(8).copied().unwrap_or(0) as u8,
        ];
        palette[slot + 128] = [
            color.get(3).copied().unwrap_or(0) as u8,
            color.get(4).copied().unwrap_or(0) as u8,
            color.get(5).copied().unwrap_or(0) as u8,
        ];
        set[slot] = true;
        set[slot + 128] = true;
    }

    for slot in 0..palette.len() {
        if !set[slot] {
            continue;
        }
        while (0..slot).any(|previous| set[previous] && palette[previous] == palette[slot]) {
            for channel in &mut palette[slot] {
                // StoreMapPalette intentionally consumes the process-global
                // C rand() stream; the chosen color is presentation-only.
                let increase = unsafe { rand() } < C_RAND_MAX / 2;
                *channel = if increase {
                    channel.wrapping_add(3)
                } else {
                    channel.wrapping_sub(3)
                };
            }
        }
    }
    palette
}

fn store_surface_palette(
    source_palette: &clonk_resources::bitmap::RgbPalette,
    painted_materials: &[Option<String>],
    materials: &MaterialSet,
) -> clonk_resources::bitmap::RgbPalette {
    let mut palette = *source_palette;
    for slot in 0..C4M_MAX_TEX_INDEX {
        let Some(material) = painted_materials
            .get(slot)
            .and_then(Option::as_deref)
            .and_then(|name| materials.get(name))
        else {
            continue;
        };
        let color = material.color();
        let rgb = [
            color.first().copied().unwrap_or(0) as u8,
            color.get(1).copied().unwrap_or(0) as u8,
            color.get(2).copied().unwrap_or(0) as u8,
        ];
        palette[slot] = rgb;
        palette[slot + 128] = rgb;
    }
    palette
}

/// State C++ keeps alongside `Surface8` for deterministic map-to-landscape
/// rasterization after scenario activation (`C4Landscape.h:57-71`). It is
/// absent for fixture/column-only landscapes that have no texture map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LandscapeRasterState {
    map_zoom: i32,
    map_seed: i32,
    texmap: RuntimeTexMapState,
    #[serde(
        default = "default_surface_palette_vec",
        skip_serializing_if = "surface_palette_is_default"
    )]
    surface_palette: Vec<[u8; 3]>,
    /// Material colors most recently painted into the live Surface8 palette
    /// by Mat2Pal. Slots are intentionally sticky: RemoveEntry/MoveIndex do
    /// not call HandleTexMapUpdate, so cleared/moved entries retain their old
    /// RGB bytes until a later successful AddEntry repaints all mapped slots.
    #[serde(default, skip_serializing_if = "surface_palette_materials_are_empty")]
    surface_palette_materials: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "bool_is_false")]
    map_changed: bool,
    /// The authoritative indexed `C4Landscape::Map`. Static editor tools
    /// mutate this plane before MapToLandscape; exact-mode edits intentionally
    /// leave it untouched so switching back to Static restores the map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    map_indices: Vec<u8>,
    #[serde(default, skip_serializing_if = "crate::u32_is_zero")]
    map_width: u32,
    #[serde(default, skip_serializing_if = "crate::u32_is_zero")]
    map_height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    map_creator: Option<crate::map_creator_s2::MapCreatorS2State>,
}

impl LandscapeRasterState {
    pub(crate) fn new(map_zoom: i32, map_seed: i32, texmap: RuntimeTexMapState) -> Self {
        let mut state = Self {
            map_zoom,
            map_seed,
            texmap,
            surface_palette: default_surface_palette().to_vec(),
            surface_palette_materials: Vec::new(),
            map_changed: false,
            map_indices: Vec::new(),
            map_width: 0,
            map_height: 0,
            map_creator: None,
        };
        state.repaint_surface_palette_materials();
        state
    }

    pub(crate) fn map_zoom(&self) -> i32 {
        self.map_zoom
    }

    pub(crate) fn set_map_zoom(&mut self, map_zoom: i32) {
        self.map_zoom = map_zoom;
    }

    pub(crate) fn map_seed(&self) -> i32 {
        self.map_seed
    }

    pub(crate) fn set_map_seed(&mut self, map_seed: i32) {
        self.map_seed = map_seed;
    }

    pub(crate) fn map_changed(&self) -> bool {
        self.map_changed
    }

    pub(crate) fn set_map_changed(&mut self) {
        self.map_changed = true;
    }

    pub(crate) fn texmap(&self) -> &RuntimeTexMapState {
        &self.texmap
    }

    pub(crate) fn texmap_mut(&mut self) -> &mut RuntimeTexMapState {
        &mut self.texmap
    }

    pub(crate) fn set_surface_palette(&mut self, palette: clonk_resources::bitmap::RgbPalette) {
        self.surface_palette = palette.to_vec();
    }

    fn surface_palette(&self) -> clonk_resources::bitmap::RgbPalette {
        let mut palette = default_surface_palette();
        for (target, source) in palette.iter_mut().zip(&self.surface_palette) {
            *target = *source;
        }
        palette
    }

    fn repaint_surface_palette_materials(&mut self) {
        if self.surface_palette_materials.is_empty()
            && !self
                .texmap
                .material_names
                .iter()
                .take(C4M_MAX_TEX_INDEX)
                .any(Option::is_some)
        {
            return;
        }
        self.surface_palette_materials
            .resize(C4M_MAX_TEX_INDEX, None);
        for slot in 0..C4M_MAX_TEX_INDEX {
            if let Some(material) = self
                .texmap
                .material_names
                .get(slot)
                .and_then(Option::as_ref)
            {
                self.surface_palette_materials[slot] = Some(material.clone());
            }
        }
    }

    fn texmap_adds_slot(&self, texmap: &RuntimeTexMapState) -> bool {
        (1..C4M_MAX_TEX_INDEX).any(|slot| {
            self.texmap
                .material_names
                .get(slot)
                .and_then(Option::as_ref)
                .is_none()
                && texmap
                    .material_names
                    .get(slot)
                    .and_then(Option::as_ref)
                    .is_some()
        })
    }

    fn replace_texmap(&mut self, texmap: RuntimeTexMapState, force_repaint: bool) {
        let repaint = force_repaint || self.texmap_adds_slot(&texmap);
        self.texmap = texmap;
        if repaint {
            if force_repaint {
                self.surface_palette_materials.clear();
            }
            self.repaint_surface_palette_materials();
        }
    }

    pub(crate) fn set_map(&mut self, bitmap: &clonk_resources::bitmap::IndexedBitmap) {
        self.map_width = bitmap.width;
        self.map_height = bitmap.height;
        self.map_indices.clone_from(&bitmap.indices);
    }

    pub(crate) fn map(&self) -> Option<clonk_resources::bitmap::IndexedBitmap> {
        let expected = (self.map_width as usize).checked_mul(self.map_height as usize)?;
        (self.map_width > 0 && self.map_height > 0 && self.map_indices.len() == expected).then(
            || clonk_resources::bitmap::IndexedBitmap {
                width: self.map_width,
                height: self.map_height,
                indices: self.map_indices.clone(),
            },
        )
    }

    fn has_map(&self) -> bool {
        let Some(expected) = (self.map_width as usize).checked_mul(self.map_height as usize) else {
            return false;
        };
        self.map_width > 0 && self.map_height > 0 && self.map_indices.len() == expected
    }

    pub(crate) fn clear_map(&mut self) {
        self.map_indices.clear();
        self.map_width = 0;
        self.map_height = 0;
    }

    fn map_mut(&mut self) -> Option<(u32, u32, &mut [u8])> {
        let expected = (self.map_width as usize).checked_mul(self.map_height as usize)?;
        if self.map_width == 0 || self.map_height == 0 || self.map_indices.len() != expected {
            return None;
        }
        Some((self.map_width, self.map_height, &mut self.map_indices))
    }

    /// C4Landscape::ReplaceMapColor rewrites the retained editor map only;
    /// Surface8 and its Pix2* caches stay unchanged until MapToLandscape
    /// performs a later static redraw.
    fn replace_map_color(&mut self, old_index: u8, new_index: u8) {
        let Some((_, _, indices)) = self.map_mut() else {
            return;
        };
        for byte in indices {
            if *byte & 0x7f == old_index {
                *byte = (*byte & 0x80) + new_index;
            }
        }
    }

    pub(crate) fn map_creator(&self) -> Option<&crate::map_creator_s2::MapCreatorS2State> {
        self.map_creator.as_ref()
    }

    pub(crate) fn set_map_creator(
        &mut self,
        creator: Option<crate::map_creator_s2::MapCreatorS2State>,
    ) {
        self.map_creator = creator;
    }
}

/// A half-open landscape change rectangle, matching `C4Rect`'s x/y +
/// width/height convention used by PrepareChange/FinishChange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RasterChangeRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// Fully synthesized, deterministic payload for one `MapToLandscape` call.
/// DrawMap/DrawDefMap build this while the script callback owns the synced
/// RNG, then both the callback COW preview and the authoritative fold apply
/// these exact bytes without parsing, rendering, or drawing RNG again.
struct IndexedMapRasterDraw {
    bounds: RasterChangeRect,
    origin: Vector2,
    target_width: i32,
    target_height: i32,
    synthesized_width: i32,
    bytes: Vec<u8>,
    texmap: RuntimeTexMapState,
}

enum PreparedIndexedMapDraw {
    Noop,
    Draw(IndexedMapRasterDraw),
}

fn prepare_indexed_map_draw(
    landscape: &Landscape,
    origin: Vector2,
    bitmap: &clonk_resources::bitmap::IndexedBitmap,
    requested_map_width: i32,
    requested_map_height: i32,
    texmap: RuntimeTexMapState,
) -> Option<PreparedIndexedMapDraw> {
    let (map_zoom, map_seed) = landscape
        .raster_state()
        .map(|state| (state.map_zoom(), state.map_seed()))?;
    let (map_width, map_height) = (
        i32::try_from(bitmap.width).ok()?,
        i32::try_from(bitmap.height).ok()?,
    );
    let expected_len = (map_width as usize).checked_mul(map_height as usize)?;
    if map_zoom <= 0 || bitmap.indices.len() != expected_len {
        return None;
    }
    let map_segment_width = requested_map_width.min(map_width);
    let map_segment_height = requested_map_height.min(map_height);
    if map_segment_width <= 0 || map_segment_height <= 0 {
        return Some(PreparedIndexedMapDraw::Noop);
    }
    let target_width = map_segment_width.checked_mul(map_zoom)?;
    let target_height = map_segment_height.checked_mul(map_zoom)?;
    let synthesized_width = map_width.checked_mul(map_zoom)?;
    let surface = crate::chunky::synthesize_landscape(
        &bitmap.indices,
        map_width,
        map_height,
        map_zoom,
        map_seed,
        &texmap.shapes,
    );
    Some(PreparedIndexedMapDraw::Draw(IndexedMapRasterDraw {
        bounds: RasterChangeRect::new(origin.x, origin.y, target_width, target_height),
        origin,
        target_width,
        target_height,
        synthesized_width,
        bytes: surface.into_bytes(),
        texmap,
    }))
}

impl IndexedMapRasterDraw {
    fn apply(self, grid: &mut PixelGrid, state: &mut LandscapeRasterState) {
        state.replace_texmap(self.texmap, false);
        for local_y in 0..self.target_height {
            for local_x in 0..self.target_width {
                let index = (local_y * self.synthesized_width + local_x) as usize;
                grid.write_byte(
                    self.origin.x + local_x,
                    self.origin.y + local_y,
                    self.bytes[index],
                );
            }
        }
    }
}

impl RasterChangeRect {
    pub(crate) const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn clipped_to(self, width: i32, height: i32) -> Option<Self> {
        if self.width <= 0 || self.height <= 0 || width <= 0 || height <= 0 {
            return None;
        }
        let x0 = i64::from(self.x).clamp(0, i64::from(width));
        let y0 = i64::from(self.y).clamp(0, i64::from(height));
        let x1 = (i64::from(self.x) + i64::from(self.width)).clamp(0, i64::from(width));
        let y1 = (i64::from(self.y) + i64::from(self.height)).clamp(0, i64::from(height));
        (x1 > x0 && y1 > y0)
            .then(|| Self::new(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32))
    }

    fn columns(self) -> Range<usize> {
        self.x as usize..(self.x + self.width) as usize
    }

    fn intersection(self, x: i32, y: i32, width: i32, height: i32) -> Option<Self> {
        let x0 = i64::from(self.x).max(i64::from(x));
        let y0 = i64::from(self.y).max(i64::from(y));
        let x1 = (i64::from(self.x) + i64::from(self.width)).min(i64::from(x) + i64::from(width));
        let y1 = (i64::from(self.y) + i64::from(self.height)).min(i64::from(y) + i64::from(height));
        (x1 > x0 && y1 > y0)
            .then(|| Self::new(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32))
    }
}

fn material_chunk_raster_bounds(
    origin: Vector2,
    width: i32,
    height: i32,
    landscape_width: i32,
    landscape_height: i32,
) -> (RasterChangeRect, Option<RasterChangeRect>) {
    let prepare_bounds = RasterChangeRect::new(
        origin.x.saturating_sub(5),
        origin.y.saturating_sub(5),
        width.saturating_add(10),
        height.saturating_add(10),
    );
    if landscape_width <= 0 || landscape_height <= 0 {
        return (prepare_bounds, None);
    }

    // CSurface8::Clip clamps each inclusive endpoint independently instead
    // of intersecting a rectangle with the surface.
    let clip_x = origin.x.saturating_sub(5).clamp(0, landscape_width - 1);
    let clip_y = origin.y.saturating_sub(5).clamp(0, landscape_height - 1);
    let clip_x2 = origin
        .x
        .saturating_add(width)
        .saturating_add(5)
        .clamp(0, landscape_width - 1);
    let clip_y2 = origin
        .y
        .saturating_add(height)
        .saturating_add(5)
        .clamp(0, landscape_height - 1);
    let changed_bounds = (clip_x2 >= clip_x && clip_y2 >= clip_y)
        .then(|| RasterChangeRect::new(clip_x, clip_y, clip_x2 - clip_x + 1, clip_y2 - clip_y + 1));
    (prepare_bounds, changed_bounds)
}

impl From<RasterChangeRect> for PixelGridDirtyRect {
    fn from(rect: RasterChangeRect) -> Self {
        Self {
            x: rect.x as u32,
            y: rect.y as u32,
            width: rect.width as u32,
            height: rect.height as u32,
        }
    }
}

#[derive(Debug, Error)]
pub enum LandscapeError {
    #[error("height map length {found} does not match width {width}")]
    InvalidHeightMap { width: u32, found: usize },
    #[error("landscape has no Surface8 pixel grid")]
    MissingPixelGrid,
    #[error("initial landscape snapshot has not been captured")]
    MissingInitialPixels,
    #[error(
        "initial landscape snapshot {initial_width}x{initial_height} does not match Surface8 {width}x{height}"
    )]
    InitialPixelDimensions {
        initial_width: u32,
        initial_height: u32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Error)]
pub enum LandscapePersistenceError {
    #[error("engine has no landscape")]
    MissingLandscape,
    #[error("landscape has no retained map")]
    MissingMap,
    #[error("static scenario save requested for landscape mode {0}")]
    NotStatic(i32),
    #[error("Material.c4g exists but is not a child group")]
    MaterialGroupIsFile,
    #[error("landscape surface persistence failed: {0}")]
    Landscape(#[from] LandscapeError),
    #[error("failed to encode retained map: {0}")]
    Bitmap(#[from] clonk_resources::bitmap::BitmapError),
    #[error("failed to update C4Group: {0}")]
    Group(#[from] clonk_resources::MutableGroupError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LiquidColumn {
    #[serde(default)]
    segments: Vec<LiquidSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidSegment {
    pub top: i32,
    pub bottom: i32,
    #[serde(default)]
    pub material: Option<MaterialId>,
}

pub const LANDSCAPE_MODE_UNDEFINED: i32 = 0;
pub const LANDSCAPE_MODE_DYNAMIC: i32 = 1;
pub const LANDSCAPE_MODE_STATIC: i32 = 2;
pub const LANDSCAPE_MODE_EXACT: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertMaterialDestination {
    Column,
    Grid { x: i32, y: i32 },
}

/// C4Landscape::pInitial — the raw Surface8 plane captured after base
/// landscape creation and before DiffLandscape.bmp is applied. Arc preserves
/// the native copy semantics cheaply because later PixelGrid writes are COW.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LandscapeInitialPixels {
    width: u32,
    height: u32,
    bytes: Arc<Vec<u8>>,
}

/// Runtime-only C4Landscape::pInitial storage. It is deliberately ignored by
/// Landscape equality just like other non-serialized load/save helpers.
#[derive(Debug, Clone, Default)]
struct RuntimeInitialPixels(Option<LandscapeInitialPixels>);

impl PartialEq for RuntimeInitialPixels {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for RuntimeInitialPixels {}

/// Runtime-only memoization for synthetic/old landscapes which do not carry
/// C4Landscape's exact `GBackHgt`. Height is queried in the innermost terrain
/// probe loops, while deriving it requires walking every surface and liquid
/// column. Mutators clear this cache at the same seams that maintain those
/// derived columns.
#[derive(Debug, Clone, Default)]
struct RuntimeEstimatedHeight(OnceLock<i32>);

impl PartialEq for RuntimeEstimatedHeight {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for RuntimeEstimatedHeight {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Landscape {
    width: u32,
    surface: Vec<i32>,
    /// C4Landscape::Mode. Synthetic/old snapshots default to the native
    /// pre-initialization value; scenario loading assigns the actual mode.
    #[serde(default, skip_serializing_if = "landscape_mode_is_undefined")]
    mode: i32,
    /// C4Landscape::Modulation: raw landscape blit modulation. Zero keeps
    /// normal drawing; any other value is applied during presentation.
    #[serde(default, skip_serializing_if = "crate::u32_is_zero")]
    modulation: u32,
    /// C4SLandscape::ShadeMaterials: enable Placement-based relief shading
    /// while the frontend rebuilds C4Landscape's presentation Surface32.
    #[serde(
        default = "default_shade_materials",
        skip_serializing_if = "bool_is_true"
    )]
    shade_materials: bool,
    #[serde(default)]
    liquids: Vec<LiquidColumn>,
    #[serde(default)]
    solid_materials: Vec<Option<MaterialId>>,
    #[serde(default)]
    default_solid_material: Option<MaterialId>,
    #[serde(default)]
    default_liquid_material: Option<MaterialId>,
    /// Tunnel-background (IFT) overlay: per-column inclusive y ranges where
    /// the landscape pixel carries the IFT bit (C4Landscape `PixColIFT`).
    /// Wind is dead inside (`GBackWind`, C4Wrappers.h:189-192). The map
    /// renderer that paints IFT from Landscape.txt still needs the pixel
    /// landscape; scenarios populate this via `set_tunnel_column`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    tunnels: HashMap<u32, Vec<(i32, i32)>>,
    /// The real landscape height (`GBackHgt`, C4Landscape.h): the column
    /// model can only estimate it from surface depths, which undershoots
    /// when no column is all-sky. Search loops and border rules bound on
    /// this when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    world_height: Option<i32>,
    /// Runtime-only cached fallback for [`Self::estimated_height`].
    #[serde(skip)]
    estimated_height_cache: RuntimeEstimatedHeight,
    /// The per-pixel plane (C4Landscape Surface8). When present it is the
    /// TRUTH for solidity/material queries; the column model above stays
    /// maintained as the approximation legacy helpers consume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pixels: Option<PixelGrid>,
    /// C4Landscape::ScanX: the next Surface8 column visited by ExecuteScan.
    /// SyncClearance-NoSave in C++; restore/synchronize starts at column zero.
    #[serde(skip)]
    scan_x: u32,
    /// C4SLandscape::NoScan disables ExecuteScan entirely.
    #[serde(default, skip_serializing_if = "bool_is_false")]
    no_scan: bool,
    /// Border-open state (C4Landscape.h:64-65): LeftOpen/RightOpen are y
    /// thresholds (`y < LeftOpen` reads sky beyond the side), Top/BottomOpen
    /// are flags. Defaults mirror C4SLandscape::Default
    /// (C4Scenario.cpp:295-296): sides/bottom closed, top open.
    #[serde(default)]
    left_open: i32,
    #[serde(default)]
    right_open: i32,
    #[serde(default = "default_top_open")]
    top_open: bool,
    #[serde(default)]
    bottom_open: bool,
    /// `MVehic` (C4Game.cpp:1669): the material a closed border's MCVehic
    /// pixel maps to through Pix2Mat. Resolved when the engine materials
    /// exist (Engine::set_landscape).
    #[serde(default)]
    vehicle_material: Option<MaterialId>,
    /// Runtime texmap/map-creator inputs required by DrawMap and direct
    /// material raster writes. Old saves and synthetic landscapes omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raster_state: Option<LandscapeRasterState>,
    /// Raw base Surface8 used by SaveDiff. This is C++'s runtime-only
    /// `pInitial`; legacy scenario loading recreates it before ApplyDiff.
    #[serde(skip)]
    initial_pixels: RuntimeInitialPixels,
}

fn default_top_open() -> bool {
    true
}

fn default_shade_materials() -> bool {
    true
}

fn landscape_mode_is_undefined(mode: &i32) -> bool {
    *mode == LANDSCAPE_MODE_UNDEFINED
}

fn bool_is_false(value: &bool) -> bool {
    !*value
}

fn bool_is_true(value: &bool) -> bool {
    *value
}

fn default_surface_palette() -> clonk_resources::bitmap::RgbPalette {
    let source = include_bytes!("../../../planet/Graphics.c4g/C4.PAL");
    let mut palette = [[0_u8; 3]; 256];
    for (index, color) in palette.iter_mut().enumerate() {
        let offset = index * 3;
        *color = [
            source[offset] << 2,
            source[offset + 1] << 2,
            source[offset + 2] << 2,
        ];
    }
    palette[0] = [0, 0, 0];
    palette[191] = [0, 0, 255];
    palette
}

fn default_surface_palette_vec() -> Vec<[u8; 3]> {
    default_surface_palette().to_vec()
}

fn surface_palette_is_default(palette: &Vec<[u8; 3]>) -> bool {
    palette.as_slice() == default_surface_palette()
}

fn surface_palette_materials_are_empty(materials: &[Option<String>]) -> bool {
    materials.iter().all(Option::is_none)
}

/// What `C4Landscape::GetPix` (C4Landscape.h:144-161) reads beyond the
/// landscape bounds: pix 0 (sky) past an open border, MCVehic past a
/// closed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderPixel {
    Sky,
    Vehicle,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlastResult {
    pub removed_by_material: HashMap<MaterialId, i32>,
    pub affected_columns: Vec<(i32, i32)>,
    /// `BlastMatCount` (C4Landscape::BlastFree, C4Landscape.cpp:1044-1055):
    /// in-circle pixels per material counted BEFORE removal, including
    /// materials that neither BlastFree nor BlastShiftTo. The cast
    /// amounts (:1066-1079) and the BlastShiftTo probability
    /// (BlastFreePix, :964-970) derive from this count.
    pub pixel_count_by_material: HashMap<MaterialId, i32>,
    pub shift_candidates: Vec<BlastShiftCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlastShiftCandidate {
    pub column: i32,
    pub material: MaterialId,
    pub target: MaterialId,
    pub pixel_count: i32,
    /// Whether a successful per-pixel shift can be represented by changing
    /// the column's solid material. BlastFree pixels are cleared immediately
    /// after their shift in C++, so their draw must not recolor what remains.
    pub apply_column_shift: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LandscapeCommand {
    LowerRange {
        start: i32,
        end: i32,
        height: i32,
    },
    SetLiquidColumn {
        column: i32,
        segments: Vec<LiquidSegment>,
    },
    ClearLiquidColumn {
        column: i32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemperatureConversionAction {
    ChangeMaterial { target: MaterialId, strength: i32 },
    RemoveToSky { strength: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TemperatureScanAction {
    target: MaterialId,
    target_byte: u8,
    strength: i32,
}

impl TemperatureConversionAction {
    fn strength(self) -> i32 {
        match self {
            TemperatureConversionAction::ChangeMaterial { strength, .. } => strength,
            TemperatureConversionAction::RemoveToSky { strength } => strength,
        }
    }
}

fn find_temperature_conversion_action(
    materials: &MaterialSet,
    material_id: MaterialId,
    direction: TemperatureDirection,
    ambient_temperature: i32,
) -> Option<TemperatureConversionAction> {
    let outcome =
        materials.evaluate_temperature_conversion(material_id, direction, ambient_temperature)?;
    if outcome.strength <= 0 {
        return None;
    }
    if let Some(target) = outcome.target.as_material_id() {
        if target == material_id {
            return None;
        }
        return Some(TemperatureConversionAction::ChangeMaterial {
            target,
            strength: outcome.strength,
        });
    }
    if outcome.target.is_sky() {
        return Some(TemperatureConversionAction::RemoveToSky {
            strength: outcome.strength,
        });
    }
    None
}

fn find_column_conversion(
    materials: &MaterialSet,
    material_id: MaterialId,
    downward_temperature: i32,
    upward_temperature: i32,
) -> Option<TemperatureConversionAction> {
    if let Some(action) = find_temperature_conversion_action(
        materials,
        material_id,
        TemperatureDirection::Downwards,
        downward_temperature,
    ) {
        return Some(action);
    }
    if let Some(action) = find_temperature_conversion_action(
        materials,
        material_id,
        TemperatureDirection::Upwards,
        upward_temperature,
    ) {
        return Some(action);
    }
    None
}

fn segment_material_override(
    target: MaterialId,
    default: Option<MaterialId>,
) -> Option<MaterialId> {
    if Some(target) == default {
        None
    } else {
        Some(target)
    }
}

impl Landscape {
    pub fn new(width: u32, surface: Vec<i32>) -> Result<Self, LandscapeError> {
        Self::with_default_material(width, surface, None)
    }

    pub fn new_with_material(
        width: u32,
        surface: Vec<i32>,
        default_material: Option<MaterialId>,
    ) -> Result<Self, LandscapeError> {
        Self::with_default_material(width, surface, default_material)
    }

    pub fn flat(width: u32, height: i32) -> Self {
        Self::flat_with_material(width, height, None)
    }

    pub fn flat_with_material(
        width: u32,
        height: i32,
        default_material: Option<MaterialId>,
    ) -> Self {
        Self::with_default_material(width, vec![height; width as usize], default_material)
            .expect("flat landscape constructs")
    }

    #[doc(hidden)]
    pub fn with_default_material(
        width: u32,
        surface: Vec<i32>,
        default_material: Option<MaterialId>,
    ) -> Result<Self, LandscapeError> {
        if width as usize != surface.len() {
            return Err(LandscapeError::InvalidHeightMap {
                width,
                found: surface.len(),
            });
        }
        let size = width as usize;
        Ok(Self {
            width,
            surface,
            mode: LANDSCAPE_MODE_UNDEFINED,
            modulation: 0,
            shade_materials: true,
            liquids: vec![LiquidColumn::default(); size],
            solid_materials: vec![default_material; size],
            default_solid_material: default_material,
            default_liquid_material: None,
            tunnels: HashMap::new(),
            world_height: None,
            estimated_height_cache: RuntimeEstimatedHeight::default(),
            pixels: None,
            scan_x: 0,
            no_scan: false,
            left_open: 0,
            right_open: 0,
            top_open: true,
            bottom_open: false,
            vehicle_material: None,
            raster_state: None,
            initial_pixels: RuntimeInitialPixels::default(),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn mode(&self) -> i32 {
        self.mode
    }

    pub fn set_mode(&mut self, mode: i32) -> bool {
        if !(LANDSCAPE_MODE_DYNAMIC..=LANDSCAPE_MODE_EXACT).contains(&mode) {
            return false;
        }
        self.mode = mode;
        true
    }

    pub(crate) fn set_runtime_mode(&mut self, mode: i32) {
        // CompileFunc assigns the integer directly; malformed/future values
        // are retained even though the editor-facing SetMode API rejects
        // values outside Dynamic..Exact.
        self.mode = mode;
    }

    pub fn modulation(&self) -> u32 {
        self.modulation
    }

    pub(crate) fn set_modulation(&mut self, modulation: u32) {
        self.modulation = modulation;
    }

    pub fn shade_materials(&self) -> bool {
        self.shade_materials
    }

    pub fn set_shade_materials(&mut self, shade_materials: bool) {
        self.shade_materials = shade_materials;
    }

    pub fn map_seed(&self) -> i32 {
        self.raster_state
            .as_ref()
            .map_or(0, LandscapeRasterState::map_seed)
    }

    pub(crate) fn set_map_seed(&mut self, map_seed: i32) {
        if let Some(state) = self.raster_state.as_mut() {
            state.set_map_seed(map_seed);
        }
    }

    pub fn map_changed(&self) -> bool {
        self.raster_state
            .as_ref()
            .is_some_and(LandscapeRasterState::map_changed)
    }

    pub(crate) fn set_map_changed(&mut self) {
        if let Some(state) = self.raster_state.as_mut() {
            state.set_map_changed();
        }
    }

    pub fn texture_map_entries_added(&self) -> bool {
        self.raster_state
            .as_ref()
            .is_some_and(|state| state.texmap().entries_added())
    }

    /// C4Landscape::SaveMap: write the retained indexed editor map as
    /// `Map.bmp` using C4TextureMap::StoreMapPalette colors.
    pub(crate) fn save_c4_map(
        &self,
        materials: &MaterialSet,
        group: &mut clonk_resources::MutableGroup,
    ) -> Result<(), LandscapePersistenceError> {
        let state = self
            .raster_state
            .as_ref()
            .ok_or(LandscapePersistenceError::MissingMap)?;
        let map = state.map().ok_or(LandscapePersistenceError::MissingMap)?;
        let palette = store_map_palette(state.texmap(), materials);
        group.add_file("Map.bmp", map.encode_with_palette(&palette)?)?;
        Ok(())
    }

    /// SaveMap gate used by exact/forced-exact Save and SaveDiff. A normal
    /// non-exact Static scenario save calls [`Self::save_c4_map`] directly.
    pub(crate) fn save_changed_c4_map(
        &self,
        materials: &MaterialSet,
        group: &mut clonk_resources::MutableGroup,
    ) -> Result<bool, LandscapePersistenceError> {
        if !self.map_changed()
            || self
                .raster_state
                .as_ref()
                .is_none_or(|state| !state.has_map())
        {
            return Ok(false);
        }
        self.save_c4_map(materials, group)?;
        Ok(true)
    }

    /// C4GameSave::SaveLandscape's non-exact Static branch: discard a stale
    /// full-resolution Landscape.bmp, always write Map.bmp, then persist any
    /// runtime texture-map additions.
    pub(crate) fn save_c4_static_scenario(
        &self,
        materials: &MaterialSet,
        group: &mut clonk_resources::MutableGroup,
    ) -> Result<(), LandscapePersistenceError> {
        if self.mode != LANDSCAPE_MODE_STATIC {
            return Err(LandscapePersistenceError::NotStatic(self.mode));
        }
        group.remove_entry("Landscape.bmp");
        self.save_c4_map(materials, group)?;
        self.save_c4_textures(group)?;
        Ok(())
    }

    /// C4Landscape::SaveTextures. The false dirty flag is a successful no-op;
    /// a missing Material.c4g creates a child, while an existing child is
    /// updated in place and retains every unrelated entry.
    pub(crate) fn save_c4_textures(
        &self,
        group: &mut clonk_resources::MutableGroup,
    ) -> Result<bool, LandscapePersistenceError> {
        let Some(texmap) = self
            .raster_state
            .as_ref()
            .map(LandscapeRasterState::texmap)
            .filter(|texmap| texmap.entries_added())
        else {
            return Ok(false);
        };
        let bytes = texmap.serialize_added_entries();
        match group.child_mut("Material.c4g")? {
            clonk_resources::MutableGroupChildMut::Missing => {}
            clonk_resources::MutableGroupChildMut::File => {
                return Err(LandscapePersistenceError::MaterialGroupIsFile);
            }
            clonk_resources::MutableGroupChildMut::Child(child) => {
                child.add_file("TexMap.txt", bytes)?;
                return Ok(true);
            }
        }
        let mut child = clonk_resources::MutableGroup::new("Material.c4g");
        child.add_file("TexMap.txt", bytes)?;
        group.add_child("Material.c4g", child)?;
        Ok(true)
    }

    /// C4Landscape::SaveDiff: write the full or 0xff-masked Surface8 diff
    /// with the live Mat2Pal colors, then persist changed Map/TexMap state.
    pub(crate) fn save_c4_diff(
        &self,
        materials: &MaterialSet,
        group: &mut clonk_resources::MutableGroup,
        sync_save: bool,
    ) -> Result<bool, LandscapePersistenceError> {
        let diff = self.save_diff(sync_save)?;
        let wrote_diff = if let Some(diff) = diff {
            let raster = self
                .raster_state
                .as_ref()
                .ok_or(LandscapePersistenceError::MissingMap)?;
            let source_palette = raster.surface_palette();
            let painted_materials = if raster.surface_palette_materials.is_empty() {
                &raster.texmap().material_names
            } else {
                &raster.surface_palette_materials
            };
            let palette = store_surface_palette(&source_palette, painted_materials, materials);
            group.add_file("DiffLandscape.bmp", diff.encode_with_palette(&palette)?)?;
            true
        } else {
            false
        };
        self.save_changed_c4_map(materials, group)?;
        self.save_c4_textures(group)?;
        Ok(wrote_diff)
    }

    pub fn scan_x(&self) -> u32 {
        self.scan_x
    }

    pub fn set_no_scan(&mut self, no_scan: bool) {
        self.no_scan = no_scan;
    }

    pub(crate) fn synchronize_temperature_scan(&mut self) {
        self.scan_x = 0;
    }

    pub fn surface(&self) -> &[i32] {
        &self.surface
    }

    pub fn shape_sum(&self) -> i64 {
        self.surface.iter().map(|&value| i64::from(value)).sum()
    }

    pub fn liquids(&self) -> &[LiquidColumn] {
        &self.liquids
    }

    pub fn set_pixel_grid(&mut self, grid: PixelGrid) {
        self.initial_pixels = RuntimeInitialPixels::default();
        self.pixels = Some(grid);
    }

    /// Capture C4Landscape::pInitial after the base Surface8 has been built
    /// and before DiffLandscape.bmp is applied.
    pub fn save_initial(&mut self) -> Result<(), LandscapeError> {
        let grid = self
            .pixels
            .as_ref()
            .ok_or(LandscapeError::MissingPixelGrid)?;
        self.initial_pixels.0 = Some(LandscapeInitialPixels {
            width: grid.width,
            height: grid.height,
            bytes: Arc::clone(&grid.bytes),
        });
        Ok(())
    }

    /// Build the legacy DiffLandscape.bmp index plane. This mirrors C++'s
    /// `fSyncSave` flag exactly: false masks unchanged pixels with 0xff,
    /// while true retains the complete current Surface8 plane. The save
    /// caller passes `!IsSynced()`.
    pub fn save_diff(
        &self,
        sync_save: bool,
    ) -> Result<Option<clonk_resources::bitmap::IndexedBitmap>, LandscapeError> {
        let grid = self
            .pixels
            .as_ref()
            .ok_or(LandscapeError::MissingPixelGrid)?;
        let initial = self
            .initial_pixels
            .0
            .as_ref()
            .ok_or(LandscapeError::MissingInitialPixels)?;
        if (initial.width, initial.height) != (grid.width, grid.height)
            || initial.bytes.len() != grid.bytes.len()
        {
            return Err(LandscapeError::InitialPixelDimensions {
                initial_width: initial.width,
                initial_height: initial.height,
                width: grid.width,
                height: grid.height,
            });
        }

        if sync_save {
            return Ok(Some(clonk_resources::bitmap::IndexedBitmap {
                width: grid.width,
                height: grid.height,
                indices: grid.bytes.as_ref().clone(),
            }));
        }

        let mut changed = false;
        let indices = grid
            .bytes
            .iter()
            .zip(initial.bytes.iter())
            .map(|(&current, &original)| {
                if current == original {
                    0xff
                } else {
                    changed = true;
                    current
                }
            })
            .collect();
        Ok(changed.then_some(clonk_resources::bitmap::IndexedBitmap {
            width: grid.width,
            height: grid.height,
            indices,
        }))
    }

    /// Overlay a legacy DiffLandscape.bmp through C4Landscape::SetPix
    /// semantics. The reserved 0xff byte preserves the current pixel; bytes
    /// outside an undersized diff read as zero like CSurface8::GetPix.
    pub fn apply_diff(
        &mut self,
        diff: &clonk_resources::bitmap::IndexedBitmap,
    ) -> Result<(), LandscapeError> {
        let grid = self
            .pixels
            .as_mut()
            .ok_or(LandscapeError::MissingPixelGrid)?;
        let width = grid.width;
        let height = grid.height;
        let diff_width = diff.width as usize;
        let mut first_changed = width;
        let mut last_changed = None;

        for y in 0..height {
            for x in 0..width {
                let byte = if x < diff.width && y < diff.height {
                    diff.indices
                        .get(y as usize * diff_width + x as usize)
                        .copied()
                        .unwrap_or(0)
                } else {
                    0
                };
                if byte == 0xff || grid.byte_at(x as i32, y as i32) == Some(byte) {
                    continue;
                }
                grid.set_byte(x as i32, y as i32, byte);
                first_changed = first_changed.min(x);
                last_changed = Some(last_changed.map_or(x, |last: u32| last.max(x)));
            }
        }

        if let Some(last_changed) = last_changed {
            self.refresh_raster_columns(first_changed as usize..last_changed as usize + 1);
        }
        Ok(())
    }

    pub fn pixel_grid(&self) -> Option<&PixelGrid> {
        self.pixels.as_ref()
    }

    /// One entry from C4Landscape's live Surface8 palette plus the material
    /// whose Mat2Pal color currently overrides that slot. Material mappings
    /// are intentionally the sticky raster-state copy, not the current
    /// texture map, matching C4Landscape::HandleTexMapUpdate.
    pub fn surface8_palette_entry(&self, index: u8) -> ([u8; 3], u8, Option<&str>) {
        let index = usize::from(index);
        let source_transparency = if index == 0 {
            255
        } else if index == 191 && self.mode != LANDSCAPE_MODE_EXACT {
            127
        } else {
            0
        };
        let slot = index & 0x7f;
        let Some(raster) = self.raster_state.as_ref() else {
            let material = (slot < C4M_MAX_TEX_INDEX)
                .then(|| {
                    self.pixels
                        .as_ref()
                        .and_then(|grid| grid.material_names().get(slot))
                        .and_then(Option::as_deref)
                })
                .flatten();
            return (
                default_surface_palette()[index],
                source_transparency,
                material,
            );
        };
        let color = raster
            .surface_palette
            .get(index)
            .copied()
            .unwrap_or_else(|| default_surface_palette()[index]);
        let painted_materials = if raster.surface_palette_materials.is_empty() {
            &raster.texmap.material_names
        } else {
            &raster.surface_palette_materials
        };
        let material = (slot < C4M_MAX_TEX_INDEX)
            .then(|| painted_materials.get(slot).and_then(Option::as_deref))
            .flatten();
        (color, source_transparency, material)
    }

    pub(crate) fn set_raster_state(&mut self, state: LandscapeRasterState) {
        self.raster_state = Some(state);
    }

    pub(crate) fn raster_state(&self) -> Option<&LandscapeRasterState> {
        self.raster_state.as_ref()
    }

    pub(crate) fn raster_state_mut(&mut self) -> Option<&mut LandscapeRasterState> {
        self.raster_state.as_mut()
    }

    pub(crate) fn clear_retained_map(&mut self) {
        if let Some(state) = self.raster_state.as_mut() {
            state.clear_map();
        }
    }

    pub(crate) fn replace_runtime_texmap_state(&mut self, texmap: RuntimeTexMapState) -> bool {
        self.replace_runtime_texmap_state_with_repaint(texmap, false)
    }

    pub(crate) fn replace_runtime_texmap_state_and_repaint(
        &mut self,
        texmap: RuntimeTexMapState,
    ) -> bool {
        self.replace_runtime_texmap_state_with_repaint(texmap, true)
    }

    fn replace_runtime_texmap_state_with_repaint(
        &mut self,
        texmap: RuntimeTexMapState,
        force_repaint: bool,
    ) -> bool {
        let Landscape {
            pixels,
            raster_state,
            ..
        } = self;
        let Some(state) = raster_state.as_mut() else {
            return false;
        };
        state.replace_texmap(texmap, force_repaint);
        if let Some(pixels) = pixels {
            pixels.sync_runtime_texmap(state.texmap());
        }
        true
    }

    /// C4TextureMap is game-global and survives `C4Landscape::Clear`. Eager
    /// Rust section maps carry the entries they allocated while preparing;
    /// merge those into the current live map and remap target Surface8/Map
    /// bytes when a runtime entry already occupied the same numeric slot.
    pub(crate) fn merge_runtime_texmap_for_section(
        &mut self,
        live: RuntimeTexMapState,
        lookups: &[RuntimeTexMapLookup],
    ) -> bool {
        let raw_diff = self.save_diff(false).ok().flatten();
        {
            let Landscape {
                pixels,
                raster_state,
                initial_pixels,
                ..
            } = self;
            let Some(state) = raster_state.as_mut() else {
                return false;
            };
            let (merged, remap) = RuntimeTexMapState::replay_section_lookups(live, lookups);
            let remap_byte = |byte: &mut u8| {
                *byte = (*byte & 0x80) | remap[usize::from(*byte & 0x7f)];
            };
            for byte in &mut state.map_indices {
                remap_byte(byte);
            }
            if let Some(creator) = state.map_creator.as_mut() {
                creator.remap_material_colors(&remap);
            }
            if let Some(initial) = initial_pixels.0.as_mut() {
                for byte in Arc::make_mut(&mut initial.bytes) {
                    remap_byte(byte);
                }
                if let Some(pixels) = pixels.as_mut() {
                    pixels.bytes = Arc::clone(&initial.bytes);
                }
            } else if let Some(pixels) = pixels.as_mut() {
                for byte in Arc::make_mut(&mut pixels.bytes) {
                    remap_byte(byte);
                }
            }
            state.replace_texmap(merged, true);
            if let Some(pixels) = pixels.as_mut() {
                pixels.sync_runtime_texmap(state.texmap());
            }
        }
        self.refresh_all_raster_columns();
        if let Some(diff) = raw_diff {
            if self.apply_diff(&diff).is_err() {
                return false;
            }
        }
        true
    }

    pub(crate) fn replay_empty_section_texmap_lookups(
        &mut self,
        lookups: &[RuntimeTexMapLookup],
    ) -> Option<[u8; 128]> {
        let Landscape {
            pixels,
            raster_state,
            ..
        } = self;
        let Some(state) = raster_state.as_mut() else {
            return None;
        };
        let (replayed, remap) =
            RuntimeTexMapState::replay_section_lookups(state.texmap().clone(), lookups);
        state.replace_texmap(replayed, false);
        if let Some(pixels) = pixels {
            pixels.sync_runtime_texmap(state.texmap());
        }
        Some(remap)
    }

    /// A raw section Map.bmp is interpreted only when the section is
    /// activated, against the then-live global texture map. Rebuild the
    /// eagerly prepared Surface8 with those live slot shapes, while keeping
    /// the section's sparse DiffLandscape overlay relative to the new base.
    pub(crate) fn resynthesize_retained_map_for_section(&mut self) -> bool {
        let diff = self.save_diff(false).ok().flatten();
        let Some((map, map_zoom, map_seed, shapes)) =
            self.raster_state.as_ref().and_then(|state| {
                Some((
                    state.map()?,
                    state.map_zoom(),
                    state.map_seed(),
                    state.texmap().shapes.clone(),
                ))
            })
        else {
            return false;
        };
        let (Ok(map_width), Ok(map_height)) = (i32::try_from(map.width), i32::try_from(map.height))
        else {
            return false;
        };
        if map_zoom <= 0 {
            return false;
        }
        let synthesized = crate::chunky::synthesize_landscape(
            &map.indices,
            map_width,
            map_height,
            map_zoom,
            map_seed,
            &shapes,
        )
        .into_bytes();
        let Some(grid) = self.pixels.as_mut() else {
            return false;
        };
        let target_width = map_width.saturating_mul(map_zoom).max(0) as usize;
        let target_height = map_height.saturating_mul(map_zoom).max(0) as usize;
        let grid_width = grid.width as usize;
        let grid_height = grid.height as usize;
        let bytes = Arc::make_mut(&mut grid.bytes);
        bytes.fill(0);
        let copy_width = target_width.min(grid_width);
        let copy_height = target_height.min(grid_height);
        for row in 0..copy_height {
            let source = row * target_width;
            let destination = row * grid_width;
            bytes[destination..destination + copy_width]
                .copy_from_slice(&synthesized[source..source + copy_width]);
        }
        self.refresh_all_raster_columns();
        if self.save_initial().is_err() {
            return false;
        }
        if let Some(diff) = diff {
            if self.apply_diff(&diff).is_err() {
                return false;
            }
        }
        true
    }

    /// Apply C4Landscape::SetTextureIndex's ReplaceMapColor + MoveIndex pair.
    /// Neither arm calls `HandleTexMapUpdate`, so Surface8 and its cached
    /// Pix2Mat/Pix2Dens tables remain untouched.
    pub(crate) fn apply_runtime_texture_index_move(
        &mut self,
        texmap: RuntimeTexMapState,
        old_index: u8,
        new_index: u8,
    ) -> bool {
        let Some(state) = self.raster_state.as_mut() else {
            return false;
        };
        state.replace_map_color(old_index, new_index);
        *state.texmap_mut() = texmap;
        true
    }

    /// Replay C4TextureMap::RemoveEntry without HandleTexMapUpdate: retained
    /// entries change, but Surface8 and its Pix2* cache tables remain intact.
    pub(crate) fn clear_runtime_texmap_entries(&mut self, slots: &[u8]) -> bool {
        let Some(state) = self.raster_state.as_mut() else {
            return false;
        };
        let texmap = state.texmap_mut();
        texmap.clear_entries(slots);
        // RemoveUnusedTexMapEntries sets fEntriesAdded even when its scan
        // clears nothing. Preserve that sticky bit in callback-local replay
        // so a later operation carrying the full texmap cannot erase it.
        texmap.entries_added = true;
        true
    }

    /// Apply RemoveUnusedTexMapEntries against the authoritative Surface8 at
    /// this exact fold position. Callback previews may not contain pixels
    /// from earlier deferred writers, so their captured slot list cannot be
    /// authoritative here.
    pub(crate) fn remove_unused_runtime_texmap_entries(&mut self) -> bool {
        let Some(texture_usage) = self.texture_index_usage() else {
            return false;
        };
        let Some(state) = self.raster_state.as_mut() else {
            return false;
        };
        state.texmap_mut().remove_unused_entries(texture_usage);
        true
    }

    pub(crate) fn replace_runtime_map_creator_state(
        &mut self,
        creator: crate::map_creator_s2::MapCreatorS2State,
    ) -> bool {
        let Some(state) = self.raster_state.as_mut() else {
            return false;
        };
        state.set_map_creator(Some(creator));
        true
    }

    /// Apply DrawMatChunks to a callback's COW landscape so later native
    /// reads observe C++'s synchronous Surface8 mutation. The authoritative
    /// Engine fold repeats the same captured geometry with solid-mask
    /// handling after the VM returns; no global RNG is read here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preview_draw_material_chunks(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
        count_x: i32,
        count_y: i32,
        material: &str,
        byte: u8,
        map_seed: i32,
        random_offsets: &[i32],
        texmap: RuntimeTexMapState,
    ) -> bool {
        let Some(shape) = texmap.material(material).map(|material| material.shape) else {
            return false;
        };
        if !self.replace_runtime_texmap_state(texmap) {
            return false;
        }
        let Some((landscape_width, landscape_height)) = self.grid_dimensions() else {
            return true;
        };
        let (prepare_bounds, changed_bounds) =
            material_chunk_raster_bounds(origin, width, height, landscape_width, landscape_height);
        if count_x <= 0 || count_y <= 0 {
            let _ = self.raster_transaction(prepare_bounds, |_, _| {});
            return true;
        }
        let expected_offsets = (count_x as usize).saturating_mul(count_y as usize);
        if random_offsets.len() != expected_offsets {
            return false;
        }
        let Some(changed_bounds) = changed_bounds else {
            let _ = self.raster_transaction(prepare_bounds, |_, _| {});
            return true;
        };
        let _ = self.raster_transaction_with_bounds(
            Some(prepare_bounds),
            Some(changed_bounds),
            |grid, _state| {
                grid.draw_material_chunks(
                    changed_bounds,
                    origin,
                    width,
                    height,
                    count_x,
                    count_y,
                    byte,
                    shape,
                    random_offsets,
                    map_seed,
                );
            },
        );
        true
    }

    /// Callback-COW counterpart of Engine's solid-mask transaction. The
    /// change runs once with intersecting masks removed newest-to-oldest;
    /// repair then refreshes their saved backgrounds oldest-to-newest.
    pub(crate) fn preview_raster_transaction_with_masks<R>(
        &mut self,
        bakes: &mut [(crate::ObjectId, crate::SolidMaskBake)],
        prepare_bounds: RasterChangeRect,
        change: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let mask_bounds = self.grid_dimensions().and_then(|(width, height)| {
            RasterChangeRect::new(
                prepare_bounds.x.saturating_sub(2),
                prepare_bounds.y.saturating_sub(16),
                prepare_bounds.width.saturating_add(4),
                prepare_bounds.height.saturating_add(32),
            )
            .clipped_to(width, height)
        });
        let vehicle = self.grid_vehicle_byte();
        let mask_indices = vehicle
            .map(|_| {
                bakes
                    .iter()
                    .enumerate()
                    .filter_map(|(index, (_, bake))| {
                        mask_bounds?
                            .intersection(bake.x, bake.y, bake.width, bake.height)
                            .map(|_| index)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if let Some(vehicle) = vehicle {
            for &index in mask_indices.iter().rev() {
                let bake = bakes[index].1.clone();
                let overlap = mask_bounds
                    .and_then(|bounds| bounds.intersection(bake.x, bake.y, bake.width, bake.height))
                    .expect("selected preview mask still overlaps");
                for y in overlap.y..overlap.y + overlap.height {
                    for x in overlap.x..overlap.x + overlap.width {
                        let buffer_index = ((y - bake.y) * bake.width + (x - bake.x)) as usize;
                        let saved = bake.buffer[buffer_index];
                        if saved != vehicle {
                            self.grid_write_byte(x, y, saved);
                        }
                    }
                }
            }
        }

        let result = change(self);

        if let Some(vehicle) = vehicle {
            for &index in &mask_indices {
                let bake = &mut bakes[index].1;
                let overlap = mask_bounds
                    .and_then(|bounds| bounds.intersection(bake.x, bake.y, bake.width, bake.height))
                    .expect("selected preview mask still overlaps");
                for y in overlap.y..overlap.y + overlap.height {
                    for x in overlap.x..overlap.x + overlap.width {
                        let buffer_index = ((y - bake.y) * bake.width + (x - bake.x)) as usize;
                        if bake.buffer[buffer_index] == vehicle {
                            continue;
                        }
                        bake.buffer[buffer_index] = self.grid_byte_at(x, y).unwrap_or(0);
                        self.grid_write_byte(x, y, vehicle);
                    }
                }
            }
        }
        result
    }

    /// Keep active MCVehic pixels put while updating each bake's saved
    /// background around callback-time DrawMatChunks.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preview_draw_material_chunks_with_masks(
        &mut self,
        bakes: &mut [(crate::ObjectId, crate::SolidMaskBake)],
        origin: Vector2,
        width: i32,
        height: i32,
        count_x: i32,
        count_y: i32,
        material: &str,
        byte: u8,
        map_seed: i32,
        random_offsets: &[i32],
        texmap: RuntimeTexMapState,
    ) -> bool {
        let prepare_bounds = self.grid_dimensions().map(|(grid_width, grid_height)| {
            material_chunk_raster_bounds(origin, width, height, grid_width, grid_height).0
        });
        let draw = |landscape: &mut Self| {
            landscape.preview_draw_material_chunks(
                origin,
                width,
                height,
                count_x,
                count_y,
                material,
                byte,
                map_seed,
                random_offsets,
                texmap,
            )
        };
        match prepare_bounds {
            Some(bounds) => self.preview_raster_transaction_with_masks(bakes, bounds, draw),
            None => draw(self),
        }
    }

    /// Callback-COW counterpart of DrawMaterialQuad's
    /// PrepareChange/FinishChange transaction. Later callbacks in the same
    /// native batch must observe both the drawn bytes and repaired masks.
    pub(crate) fn preview_draw_material_quad_with_masks(
        &mut self,
        bakes: &mut [(crate::ObjectId, crate::SolidMaskBake)],
        material_texture: &str,
        vertices: [Vector2; 4],
        ift: bool,
    ) -> bool {
        let slot = self.resolve_runtime_material_texture(material_texture);
        if slot == 0 {
            return false;
        }
        let min_x = vertices.iter().map(|point| point.x).min().unwrap_or(0);
        let max_x = vertices.iter().map(|point| point.x).max().unwrap_or(0);
        let min_y = vertices.iter().map(|point| point.y).min().unwrap_or(0);
        let max_y = vertices.iter().map(|point| point.y).max().unwrap_or(0);
        let bounds = RasterChangeRect::new(
            min_x,
            min_y,
            max_x.saturating_sub(min_x).saturating_add(1),
            max_y.saturating_sub(min_y).saturating_add(1),
        );
        let polygon = vertices.map(|point| (point.x, point.y));
        let byte = slot | if ift { 0x80 } else { 0 };
        self.preview_raster_transaction_with_masks(bakes, bounds, |landscape| {
            let _ = landscape.raster_transaction(bounds, |grid, _| {
                grid.draw_polygon(&polygon, byte);
            });
            true
        })
    }

    /// Callback-COW counterpart of Engine::draw_indexed_map, including the
    /// rounded PrepareChange/FinishChange bounds and live mask repair.
    pub(crate) fn preview_draw_indexed_map_with_masks(
        &mut self,
        bakes: &mut [(crate::ObjectId, crate::SolidMaskBake)],
        origin: Vector2,
        bitmap: &clonk_resources::bitmap::IndexedBitmap,
        requested_map_width: i32,
        requested_map_height: i32,
        texmap: RuntimeTexMapState,
    ) -> bool {
        let Some(prepared) = prepare_indexed_map_draw(
            self,
            origin,
            bitmap,
            requested_map_width,
            requested_map_height,
            texmap,
        ) else {
            return false;
        };
        let PreparedIndexedMapDraw::Draw(draw) = prepared else {
            return true;
        };
        let bounds = draw.bounds;
        self.preview_raster_transaction_with_masks(bakes, bounds, move |landscape| {
            landscape
                .raster_transaction(bounds, move |grid, state| draw.apply(grid, state))
                .is_some()
        })
    }

    /// Rebuild the Rust column approximation from the authoritative Surface8
    /// plane. Scenario activation uses this once; runtime transactions use the
    /// affected columns only.
    pub(crate) fn refresh_all_raster_columns(&mut self) {
        let width = self
            .pixels
            .as_ref()
            .map(|grid| grid.width as usize)
            .unwrap_or(0);
        self.refresh_raster_columns(0..width);
    }

    pub(crate) fn refresh_raster_columns(&mut self, columns: Range<usize>) {
        self.invalidate_estimated_height();
        let end = columns.end.min(self.surface.len());
        let start = columns.start.min(end);
        self.ensure_liquid_capacity();
        for x in start..end {
            let Some(summary) = self.pixels.as_ref().and_then(|grid| grid.derived_column(x)) else {
                continue;
            };
            self.surface[x] = summary.surface;
            self.liquids[x] = LiquidColumn::from_segments(summary.liquid_segments);
            if summary.tunnel_ranges.is_empty() {
                self.tunnels.remove(&(x as u32));
            } else {
                self.tunnels.insert(x as u32, summary.tunnel_ranges);
            }
        }
    }

    /// Group an authoritative landscape raster edit like C++
    /// PrepareChange/FinishChange: expose the pixel plane and retained raster
    /// state together, synchronize any newly allocated texmap slots, then
    /// refresh only the affected derived columns. Raw solid-mask writes keep
    /// using [`Self::grid_write_byte`] and intentionally bypass this seam.
    pub(crate) fn raster_transaction<R>(
        &mut self,
        bounds: RasterChangeRect,
        change: impl FnOnce(&mut PixelGrid, &mut LandscapeRasterState) -> R,
    ) -> Option<R> {
        let (width, height) = self.grid_dimensions()?;
        let changed_bounds = bounds.clipped_to(width, height);
        self.raster_transaction_with_bounds(Some(bounds), changed_bounds, change)
    }

    /// DrawChunks is the one legacy writer whose Surface8 clip extends one
    /// inclusive pixel beyond its PrepareChange/FinishChange C4Rect. Keep
    /// relight bounds separate from the actual changed-byte bounds while
    /// retaining the shared texture-map/derived-column transaction seam.
    fn raster_transaction_with_bounds<R>(
        &mut self,
        relight_bounds: Option<RasterChangeRect>,
        changed_bounds: Option<RasterChangeRect>,
        change: impl FnOnce(&mut PixelGrid, &mut LandscapeRasterState) -> R,
    ) -> Option<R> {
        let result = {
            let pixels = self.pixels.as_mut()?;
            let state = self.raster_state.as_mut()?;
            let result = change(pixels, state);
            pixels.sync_runtime_texmap(state.texmap());
            // FinishChange relights synchronously, so a later direct
            // Surface32 write in the same script call remains visible.
            if let Some(bounds) = relight_bounds {
                pixels.relight_surface32_rect(bounds);
            }
            result
        };
        if let Some(bounds) = changed_bounds {
            self.refresh_raster_columns(bounds.columns());
        }
        Some(result)
    }

    /// Resolve (and, for an explicit valid texture pair, allocate) the live
    /// texmap slot before DrawQuad enters PrepareChange. A newly allocated
    /// slot immediately updates the pixel lookup tables just like
    /// C4TextureMap::AddEntry -> HandleTexMapUpdate
    /// (C4Texture.cpp:116-135).
    fn resolve_runtime_material_texture(&mut self, material_texture: &str) -> u8 {
        let Self {
            pixels,
            raster_state,
            ..
        } = self;
        let Some(state) = raster_state.as_mut() else {
            return 0;
        };
        let occupied_before = state
            .texmap()
            .material_names
            .iter()
            .map(Option::is_some)
            .collect::<Vec<_>>();
        let slot = state.texmap_mut().get_index_mat_tex(material_texture, None);
        if slot != 0 {
            if !occupied_before
                .get(usize::from(slot))
                .copied()
                .unwrap_or(false)
                && state
                    .texmap()
                    .material_names
                    .get(usize::from(slot))
                    .is_some_and(Option::is_some)
            {
                state.repaint_surface_palette_materials();
            }
            if let Some(pixels) = pixels {
                pixels.sync_runtime_texmap(state.texmap());
            }
        }
        slot
    }

    /// C4Landscape::GetMapColorIndex for editor Brush/Line/Rect. `Sky` is a
    /// case-sensitive special material that always paints byte zero; every
    /// other pair goes through TextureMap.GetIndex and may allocate a slot.
    fn resolve_editor_color(&mut self, material: &str, texture: &str, ift: bool) -> Option<u8> {
        if material == "Sky" {
            return Some(0);
        }
        let Self {
            pixels,
            raster_state,
            ..
        } = self;
        let state = raster_state.as_mut()?;
        let occupied_before = state
            .texmap()
            .material_names
            .iter()
            .map(Option::is_some)
            .collect::<Vec<_>>();
        let slot = state.texmap_mut().get_index(material, Some(texture), true);
        if slot == 0 {
            return None;
        }
        if !occupied_before
            .get(usize::from(slot))
            .copied()
            .unwrap_or(false)
            && state
                .texmap()
                .material_names
                .get(usize::from(slot))
                .is_some_and(Option::is_some)
        {
            state.repaint_surface_palette_materials();
        }
        if let Some(pixels) = pixels {
            pixels.sync_runtime_texmap(state.texmap());
        }
        Some(slot | if ift { 0x80 } else { 0 })
    }

    /// Resolve the grid's Pix2Mat table once the engine materials exist
    /// (UpdatePixMaps, C4Landscape.cpp:2832-2839).
    /// C4Landscape::DigFreePix (C4Landscape.cpp:936-944) on the pixel
    /// grid: resolves the pixel through GetPix's open/closed-border rules,
    /// returns its material (C++ returns it even when nothing clears), and
    /// clears only DigFree materials. `None` when no grid exists — callers
    /// keep the column-model fallback.
    pub fn dig_free_pix(&mut self, x: i32, y: i32, materials: &MaterialSet) -> Option<MaterialId> {
        self.pixels.as_ref()?;
        let material_id = match self.border_pixel(x, y) {
            Some(BorderPixel::Sky) => None,
            Some(BorderPixel::Vehicle) => self.vehicle_material,
            None => self
                .pixels
                .as_ref()
                .and_then(|grid| grid.material_id_at(x, y)),
        };
        if let Some(id) = material_id {
            if materials
                .get_by_id(id)
                .map(|material| material.dig_free())
                .unwrap_or(false)
            {
                let tunnel_byte = self.default_material_byte("Tunnel");
                if let Some(grid) = self.pixels.as_mut() {
                    grid.clear_pix_with_tunnel(x, y, tunnel_byte);
                }
            }
        }
        material_id
    }

    /// ClearPix on the grid (no diggable gate) — C4Landscape::ClearRect's
    /// per-pixel body (C4Landscape.cpp:2184-2194).
    pub fn clear_pix(&mut self, x: i32, y: i32) -> bool {
        let tunnel_byte = self.default_material_byte("Tunnel");
        match self.pixels.as_mut() {
            Some(grid) if grid.byte_at(x, y).is_some() => {
                grid.clear_pix_with_tunnel(x, y, tunnel_byte);
                true
            }
            Some(_) | None => false,
        }
    }

    /// ClearRect's complete raw Surface8 row loop. Solid-mask removal and
    /// repair are owned by the caller so this can run inside one whole-rect
    /// PrepareChange/FinishChange bracket.
    pub(crate) fn clear_rect_pixels(&mut self, bounds: RasterChangeRect) {
        let tunnel_byte = self.default_material_byte("Tunnel");
        if let Some(grid) = self.pixels.as_mut() {
            for y in bounds.y..bounds.y.saturating_add(bounds.height) {
                for x in bounds.x..bounds.x.saturating_add(bounds.width) {
                    grid.clear_pix_with_tunnel(x, y, tunnel_byte);
                }
            }
        }
        self.finish_clear_rect_change(bounds);
    }

    /// FinishChange's relight/derived-column half after a callback preview
    /// has interleaved ClearRect rows with its already-consumed Rnd3 ledger.
    pub(crate) fn finish_clear_rect_change(&mut self, bounds: RasterChangeRect) {
        let Some((width, height)) = self.grid_dimensions() else {
            return;
        };
        if let Some(grid) = self.pixels.as_mut() {
            grid.relight_surface32_rect(bounds);
        }
        if let Some(changed) = bounds.clipped_to(width, height) {
            self.refresh_raster_columns(changed.columns());
        }
    }

    fn default_material_byte(&self, material: &str) -> Option<u8> {
        self.raster_state
            .as_ref()
            .and_then(|state| state.texmap().default_material_entry(material))
    }

    fn default_material_byte_for_id(&self, material: MaterialId) -> Option<u8> {
        match self.raster_state.as_ref() {
            Some(state) => self
                .pixels
                .as_ref()
                .and_then(|grid| grid.name_for_material(material))
                .and_then(|name| state.texmap().default_material_entry(name)),
            None => self
                .pixels
                .as_ref()
                .and_then(|grid| grid.byte_for_material(material)),
        }
    }

    /// Resolve a material's cross-mapped texmap byte. With retained scenario
    /// state, an explicit `Material-Texture` must match that exact pair; the
    /// fallback exists only for synthetic pixel-grid fixtures without a live
    /// texture map.
    pub fn material_texture_byte(
        &self,
        material_texture: &str,
        fallback_material: MaterialId,
    ) -> Option<u8> {
        match self.raster_state.as_ref() {
            Some(state) => {
                let byte = state.texmap().resolved_index_mat_tex(material_texture);
                (byte != 0).then_some(byte)
            }
            None => self
                .pixels
                .as_ref()
                .and_then(|grid| grid.byte_for_material(fallback_material)),
        }
    }

    /// Use the source material's numeric CrossMapMaterials result. Missing
    /// ledgers belong to old saves and synthetic fixtures, which retain the
    /// legacy name-resolution fallback; a stored zero remains disabled.
    pub(crate) fn crossmapped_material_texture_byte(
        &self,
        material_texture: &str,
        source_material: MaterialId,
        materials: &MaterialSet,
        fallback_material: MaterialId,
    ) -> Option<u8> {
        match self.raster_state.as_ref() {
            Some(state) => {
                let byte = state
                    .texmap()
                    .frozen_crossmap_entry(materials, source_material, material_texture)
                    .unwrap_or_else(|| state.texmap().resolved_index_mat_tex(material_texture));
                (byte != 0).then_some(byte)
            }
            None => self
                .pixels
                .as_ref()
                .and_then(|grid| grid.byte_for_material(fallback_material)),
        }
    }

    /// `SetPix(..., MatTex2PixCol(...)+GBackIFT(...))` for an already-resolved
    /// material-texture byte (C4Landscape.cpp:951).
    pub fn insert_material_texture_pix(&mut self, x: i32, y: i32, byte: u8) -> bool {
        let Some(grid) = self.pixels.as_mut() else {
            return false;
        };
        let Some(current) = grid.byte_at(x, y) else {
            return false;
        };
        grid.set_byte(x, y, byte | (current & 0x80));
        true
    }

    /// The InsertMaterial dead-material write (C4Landscape.cpp:1218):
    /// `SetPix(tx, ty, Mat2PixColDefault(mat) + GBackIFT(tx, ty))` — the
    /// material byte keeps the CURRENT pixel's IFT bit.
    pub fn insert_material_pix(&mut self, x: i32, y: i32, material: MaterialId) -> bool {
        let byte = self.default_material_byte_for_id(material);
        let Some(grid) = self.pixels.as_mut() else {
            return false;
        };
        let Some(byte) = byte else {
            return false;
        };
        let Some(current) = grid.byte_at(x, y) else {
            return false;
        };
        grid.set_byte(x, y, byte | (current & 0x80));
        true
    }

    /// Apply FnDrawVolcanoBranch to the callback or authoritative Surface8.
    /// The byte was captured from `DefaultMatTex` at script-call time so a
    /// deferred engine fold cannot observe later texture-map mutations.
    pub(crate) fn draw_volcano_branch(
        &mut self,
        from: Vector2,
        to: Vector2,
        size: i32,
        material_byte: u8,
    ) -> bool {
        let Some(grid) = self.pixels.as_mut() else {
            return false;
        };
        let changed_columns = grid.draw_volcano_branch(from, to, size, material_byte);
        if let Some(columns) = changed_columns {
            self.refresh_raster_columns(columns);
        }
        true
    }

    /// C4SolidMask plumbing: the plane's Vehicle byte, raw reads and
    /// writes. None/no-op without a pixel grid (fixture worlds keep the
    /// mask-rect overlay).
    pub fn grid_vehicle_byte(&self) -> Option<u8> {
        self.default_material_byte("Vehicle")
            .or_else(|| self.pixels.as_ref().and_then(|grid| grid.vehicle_byte()))
    }

    pub fn grid_byte_at(&self, x: i32, y: i32) -> Option<u8> {
        self.pixels.as_ref().and_then(|grid| grid.byte_at(x, y))
    }

    /// C4Landscape::GetPix for Surface8 presentation consumers, including
    /// open-sky and closed-MCVehic border bytes.
    pub fn grid_byte_with_border(&self, x: i32, y: i32) -> Option<u8> {
        let grid = self.pixels.as_ref()?;
        let open = |is_open: bool| {
            if is_open {
                Some(0)
            } else {
                self.grid_vehicle_byte()
            }
        };
        if x < 0 {
            return open(y < self.left_open);
        }
        if x as u32 >= grid.width {
            return open(y < self.right_open);
        }
        if y < 0 {
            return open(self.top_open);
        }
        if y as u32 >= grid.height {
            return open(self.bottom_open);
        }
        grid.byte_at(x, y)
    }

    /// Texture-map indices present in C4Landscape::Surface8. The IFT bit is
    /// presentation state and does not distinguish texture-map entries.
    pub(crate) fn texture_index_usage(&self) -> Option<[bool; 128]> {
        let pixels = self.pixels.as_ref()?;
        let mut usage = [false; 128];
        for &byte in pixels.bytes() {
            usage[usize::from(byte & 0x7f)] = true;
        }
        Some(usage)
    }

    /// Raw packed C4 color explicitly written into the presentation-only
    /// Surface32. Absence means the frontend composes this cell from Surface8.
    pub fn surface32_pixel_at(&self, x: i32, y: i32) -> Option<u32> {
        self.pixels
            .as_ref()
            .and_then(|grid| grid.surface32_pixel_at(x, y))
    }

    /// Direct Surface32 write used by SetLandscapePixel. Returns false for a
    /// missing pixel surface or out-of-bounds coordinates; the script native
    /// intentionally discards that result like C++.
    pub fn set_surface32_pixel(&mut self, x: i32, y: i32, color: u32) -> bool {
        self.pixels
            .as_mut()
            .is_some_and(|grid| grid.set_surface32_pixel(x, y, color))
    }

    pub(crate) fn finish_surface32_draw(&mut self) {
        if let Some(grid) = self.pixels.as_mut() {
            grid.finish_pending_surface32_relights();
        }
    }

    pub fn grid_write_byte(&mut self, x: i32, y: i32, byte: u8) {
        if let Some(grid) = self.pixels.as_mut() {
            grid.write_byte(x, y, byte);
        }
    }

    pub(crate) fn grid_set_byte(&mut self, x: i32, y: i32, byte: u8) {
        if let Some(grid) = self.pixels.as_mut() {
            grid.set_byte(x, y, byte);
        }
    }

    pub fn grid_dimensions(&self) -> Option<(i32, i32)> {
        self.pixels
            .as_ref()
            .map(|grid| (grid.width() as i32, grid.height() as i32))
    }

    pub fn resolve_grid_materials(&mut self, lookup: impl FnMut(&str) -> Option<MaterialId>) {
        if let Some(grid) = self.pixels.as_mut() {
            grid.resolve_materials(lookup);
        }
    }

    /// Grid upkeep for a column-surface change: the pixel plane follows
    /// the scalar where the scalar IS the terrain top (exposed columns);
    /// cave interiors are untouched because surface ops act at the top.
    /// `byte` overrides the fill material for terrain ADDED below `old`.
    fn grid_track_surface(&mut self, x: i32, old: i32, new: i32, byte: Option<u8>) {
        let Some(grid) = self.pixels.as_mut() else {
            return;
        };
        if new > old {
            grid.clear_band(x, old.max(0), new);
        } else if new < old {
            let fill = byte
                .or_else(|| {
                    grid.byte_at(x, old)
                        .filter(|&byte| grid.density_of(byte) >= C4M_SOLID)
                })
                .or_else(|| grid.any_solid_byte());
            if let Some(fill) = fill {
                grid.fill_band(x, new.max(0), old, fill);
            }
        }
    }

    pub fn set_height(&mut self, x: u32, height: i32) {
        self.invalidate_estimated_height();
        let old = self.surface.get(x as usize).copied();
        if let Some(slot) = self.surface.get_mut(x as usize) {
            if *slot != height {
                *slot = height;
                if let Some(old) = old {
                    self.grid_track_surface(x as i32, old, height, None);
                }
            }
        }
    }

    /// Mark inclusive y ranges of a column as tunnel background (IFT).
    pub fn set_tunnel_column(&mut self, x: u32, ranges: Vec<(i32, i32)>) {
        if ranges.is_empty() {
            self.tunnels.remove(&x);
        } else {
            self.tunnels.insert(x, ranges);
        }
    }

    /// `GBackIFT` (C4Wrappers.h:159-162): true where the landscape pixel
    /// carries the tunnel-background bit.
    pub fn is_tunnel_at(&self, x: i32, y: i32) -> bool {
        u32::try_from(x)
            .ok()
            .and_then(|column| self.tunnels.get(&column))
            .map(|ranges| ranges.iter().any(|&(top, bottom)| y >= top && y <= bottom))
            .unwrap_or(false)
    }

    pub fn set_liquid_column(&mut self, x: u32, segments: Vec<LiquidSegment>) {
        self.invalidate_estimated_height();
        self.ensure_liquid_capacity();
        if let Some(column) = self.liquids.get_mut(x as usize) {
            *column = LiquidColumn::from_segments(segments);
        }
    }

    pub fn clear_liquid_column(&mut self, x: u32) {
        self.invalidate_estimated_height();
        self.ensure_liquid_capacity();
        if let Some(column) = self.liquids.get_mut(x as usize) {
            column.clear();
        }
    }

    pub fn set_default_liquid_material(&mut self, material: Option<MaterialId>) {
        self.default_liquid_material = material;
    }

    pub fn default_liquid_material(&self) -> Option<MaterialId> {
        self.default_liquid_material
    }

    pub fn set_solid_material(&mut self, column: u32, material: Option<MaterialId>) {
        self.ensure_material_capacity();
        if let Some(slot) = self.solid_materials.get_mut(column as usize) {
            if *slot != material {
                *slot = material;
            }
        }
    }

    pub fn fill_solid_material(&mut self, material: Option<MaterialId>) {
        self.default_solid_material = material;
        let desired_len = self.surface.len();
        if self.solid_materials.len() != desired_len {
            self.solid_materials = vec![material; desired_len];
        } else {
            for slot in &mut self.solid_materials {
                *slot = material;
            }
        }
    }

    pub fn set_default_solid_material(&mut self, material: Option<MaterialId>) {
        self.default_solid_material = material;
        self.ensure_material_capacity();
        if let Some(material) = material {
            for slot in &mut self.solid_materials {
                if slot.is_none() {
                    *slot = Some(material);
                }
            }
        }
    }

    pub fn default_solid_material(&self) -> Option<MaterialId> {
        self.default_solid_material
    }

    pub fn solid_material_at(&self, x: i32) -> Option<MaterialId> {
        if self.surface.is_empty() {
            return None;
        }
        if x < 0 {
            return self.default_solid_material;
        }
        let index = usize::try_from(x).ok()?;
        if index >= self.surface.len() {
            return None;
        }
        self.column_material(index)
    }

    fn ensure_material_capacity(&mut self) {
        if self.solid_materials.len() != self.surface.len() {
            self.solid_materials
                .resize(self.surface.len(), self.default_solid_material);
        }
    }

    fn estimate_world_height(&self) -> i32 {
        let mut height = self.surface.iter().copied().max().unwrap_or(0);
        for column in &self.liquids {
            for segment in column.segments() {
                height = height.max(segment.top);
                height = height.max(segment.bottom);
            }
        }
        height.max(0)
    }

    fn invalidate_estimated_height(&mut self) {
        self.estimated_height_cache.0.take();
    }

    pub fn estimated_height(&self) -> i32 {
        self.world_height.unwrap_or_else(|| {
            *self
                .estimated_height_cache
                .0
                .get_or_init(|| self.estimate_world_height())
        })
    }

    /// Pin the real landscape height (`GBackHgt`). The legacy loader knows
    /// it exactly (map height × zoom); without it the estimate from surface
    /// depths is used.
    pub fn set_world_height(&mut self, height: i32) {
        self.invalidate_estimated_height();
        self.world_height = Some(height.max(0));
    }

    pub fn apply_temperature_conversions(
        &mut self,
        materials: &MaterialSet,
        temperature: i32,
    ) -> Vec<(i32, i32)> {
        let mut probes = Vec::new();
        self.apply_temperature_conversions_with(materials, temperature, &mut |_landscape, x, y| {
            probes.push((x, y))
        });
        probes
    }

    pub(crate) fn apply_temperature_conversions_with(
        &mut self,
        materials: &MaterialSet,
        temperature: i32,
        on_instability: &mut dyn FnMut(&Landscape, i32, i32),
    ) {
        if self.no_scan || self.surface.is_empty() || materials.is_empty() {
            return;
        }
        if self.pixels.is_some() {
            self.execute_temperature_scan(materials, temperature, on_instability);
            return;
        }
        self.apply_column_temperature_conversions(materials, temperature);
    }

    fn apply_column_temperature_conversions(&mut self, materials: &MaterialSet, temperature: i32) {
        if self.surface.is_empty() || materials.is_empty() {
            return;
        }
        self.invalidate_estimated_height();
        self.ensure_material_capacity();
        self.ensure_liquid_capacity();
        let original_default = self.default_solid_material;

        let mut default_change_indices = Vec::new();
        let mut default_change_target: Option<MaterialId> = None;
        let mut default_change_uniform = true;

        let mut column_actions: Vec<Option<TemperatureConversionAction>> =
            vec![None; self.surface.len()];
        let mut uses_default = vec![false; self.surface.len()];

        for (index, slot) in self.solid_materials.iter().enumerate() {
            let (material_id, is_default) = match slot {
                Some(id) => (Some(*id), false),
                None => (original_default, true),
            };
            uses_default[index] = is_default;
            let Some(material_id) = material_id else {
                continue;
            };
            column_actions[index] =
                find_column_conversion(materials, material_id, temperature, temperature);
            if is_default {
                if let Some(TemperatureConversionAction::ChangeMaterial { target, .. }) =
                    column_actions[index]
                {
                    default_change_indices.push(index);
                    if let Some(prev) = default_change_target {
                        if prev != target {
                            default_change_uniform = false;
                        }
                    } else {
                        default_change_target = Some(target);
                    }
                }
            }
        }

        for (index, action) in column_actions.iter().enumerate() {
            let Some(action) = action else {
                continue;
            };
            if uses_default[index] {
                continue;
            }
            match action {
                TemperatureConversionAction::ChangeMaterial { target, .. } => {
                    if let Some(current) = self.solid_materials[index] {
                        if current != *target {
                            self.solid_materials[index] = Some(*target);
                        }
                    }
                }
                TemperatureConversionAction::RemoveToSky { strength } => {
                    if *strength > 0 {
                        let new_height = self.surface[index].saturating_add(*strength);
                        if new_height != self.surface[index] {
                            let old = self.surface[index];
                            self.surface[index] = new_height;
                            self.grid_track_surface(index as i32, old, new_height, None);
                        }
                    }
                }
            }
        }

        let default_column_count = uses_default.iter().filter(|&&flag| flag).count();
        if default_column_count > 0 {
            let mut removal_applied = false;
            for (index, action) in column_actions.iter().enumerate() {
                if let Some(TemperatureConversionAction::RemoveToSky { strength }) = action {
                    if *strength > 0 {
                        let new_height = self.surface[index].saturating_add(*strength);
                        if new_height != self.surface[index] {
                            let old = self.surface[index];
                            self.surface[index] = new_height;
                            self.grid_track_surface(index as i32, old, new_height, None);
                            removal_applied = true;
                        }
                    }
                }
            }

            let can_update_default = !removal_applied
                && default_column_count == default_change_indices.len()
                && default_change_uniform
                && default_change_target != original_default;

            if can_update_default {
                self.default_solid_material = default_change_target;
            } else {
                for index in default_change_indices {
                    if let Some(TemperatureConversionAction::ChangeMaterial { target, .. }) =
                        column_actions[index]
                    {
                        self.solid_materials[index] = Some(target);
                    }
                }
            }
        } else if !self.solid_materials.is_empty() {
            let mut target: Option<MaterialId> = None;
            let mut uniform = true;
            for entry in &self.solid_materials {
                match entry {
                    Some(material_id) => {
                        if let Some(existing) = target {
                            if existing != *material_id {
                                uniform = false;
                                break;
                            }
                        } else {
                            target = Some(*material_id);
                        }
                    }
                    None => {
                        uniform = false;
                        break;
                    }
                }
            }
            if uniform && target != self.default_solid_material {
                self.default_solid_material = target;
            }
        }

        let temperature_lookup = |_y: i32| temperature;
        for column in &mut self.liquids {
            let _ = column.apply_temperature_conversions(
                materials,
                self.default_liquid_material,
                &temperature_lookup,
            );
        }
    }

    fn execute_temperature_scan(
        &mut self,
        materials: &MaterialSet,
        temperature: i32,
        on_instability: &mut dyn FnMut(&Landscape, i32, i32),
    ) {
        let Some((width, height)) = self.grid_dimensions() else {
            return;
        };
        if width <= 0 || height <= 0 || !self.temperature_scan_needed(materials, temperature) {
            return;
        }

        let scan_speed = (width / 500).clamp(2, 15);
        let mut scanned_columns = Vec::with_capacity(scan_speed as usize);
        for _ in 0..scan_speed {
            let x = (self.scan_x % width as u32) as i32;
            self.execute_temperature_scan_column(x, height, materials, temperature, on_instability);
            scanned_columns.push(x as usize);
            self.scan_x = (self.scan_x + 1) % width as u32;
        }

        scanned_columns.sort_unstable();
        scanned_columns.dedup();
        for x in scanned_columns {
            self.refresh_raster_columns(x..x + 1);
        }
    }

    fn temperature_scan_needed(&self, materials: &MaterialSet, temperature: i32) -> bool {
        let Some(grid) = self.pixels.as_ref() else {
            return false;
        };
        materials.iter().any(|material| {
            let material = material.id();
            if grid.material_count(material) == 0 {
                return false;
            }
            [
                TemperatureDirection::Downwards,
                TemperatureDirection::Upwards,
            ]
            .into_iter()
            .any(|direction| {
                self.temperature_scan_action(materials, material, direction, temperature)
                    .is_some()
            })
        })
    }

    fn temperature_scan_action(
        &self,
        materials: &MaterialSet,
        material: MaterialId,
        direction: TemperatureDirection,
        temperature: i32,
    ) -> Option<TemperatureScanAction> {
        let outcome =
            materials.evaluate_temperature_conversion(material, direction, temperature)?;
        let frozen_byte = self.raster_state.as_ref().and_then(|state| {
            state
                .texmap()
                .frozen_crossmap_entry(materials, material, &outcome.target_spec)
        });
        let (target, target_byte) = match frozen_byte {
            Some(0) => return None,
            Some(target_byte) => {
                let target = self
                    .raster_state
                    .as_ref()
                    .and_then(|state| {
                        state
                            .texmap()
                            .material_names
                            .get(usize::from(target_byte & 0x7f))
                    })
                    .and_then(Option::as_deref)
                    .and_then(|name| materials.id_of(name))?;
                (target, target_byte)
            }
            None => {
                let target = outcome.target.as_material_id()?;
                let target_byte = self.crossmapped_material_texture_byte(
                    &outcome.target_spec,
                    material,
                    materials,
                    target,
                )?;
                (target, target_byte)
            }
        };
        Some(TemperatureScanAction {
            target,
            target_byte,
            strength: outcome.strength,
        })
    }

    fn temperature_scan_material_at(&self, x: i32, y: i32) -> Option<MaterialId> {
        self.pixels
            .as_ref()
            .and_then(|grid| grid.material_id_at(x, y))
    }

    fn execute_temperature_scan_column(
        &mut self,
        x: i32,
        height: i32,
        materials: &MaterialSet,
        temperature: i32,
        on_instability: &mut dyn FnMut(&Landscape, i32, i32),
    ) {
        let mut y = 0;
        let mut last_material = None;
        while y < height {
            let material = self.temperature_scan_material_at(x, y);
            if material != last_material {
                if let Some(previous) = last_material {
                    self.do_temperature_scan(
                        x,
                        y - 1,
                        previous,
                        TemperatureDirection::Upwards,
                        materials,
                        temperature,
                        on_instability,
                    );
                }
                if let Some(material) = material {
                    y += self.do_temperature_scan(
                        x,
                        y,
                        material,
                        TemperatureDirection::Downwards,
                        materials,
                        temperature,
                        on_instability,
                    );
                }
            }
            last_material = material;
            y += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn do_temperature_scan(
        &mut self,
        x: i32,
        y: i32,
        material: MaterialId,
        direction: TemperatureDirection,
        materials: &MaterialSet,
        temperature: i32,
        on_instability: &mut dyn FnMut(&Landscape, i32, i32),
    ) -> i32 {
        let Some(action) =
            self.temperature_scan_action(materials, material, direction, temperature)
        else {
            return 0;
        };
        let Some((width, height)) = self.grid_dimensions() else {
            return 0;
        };
        let y_direction = match direction {
            TemperatureDirection::Downwards => 1,
            TemperatureDirection::Upwards => -1,
        };
        let left_material = if x > 0 {
            self.temperature_scan_material_at(x - 1, y)
        } else {
            None
        };
        if left_material == Some(material) {
            return 0;
        }

        let mut remaining = action.strength;
        if left_material != Some(action.target) {
            let search_range = 5.max(remaining);
            let mut search_x = x;
            let mut search_y = y;
            while search_x < width - 1 {
                search_x += 1;
                if self.temperature_scan_material_at(search_x, search_y) == Some(material) {
                    search_y -= y_direction;
                    while (0..height).contains(&search_y)
                        && self.temperature_scan_material_at(search_x, search_y) == Some(material)
                    {
                        search_y -= y_direction;
                        remaining = remaining.min(action.strength - (search_y - y).abs());
                        if remaining < 0 {
                            return 0;
                        }
                    }
                    if !(0..height).contains(&search_y) {
                        break;
                    }
                    search_y += y_direction;
                } else {
                    search_y += y_direction;
                    while (0..height).contains(&search_y)
                        && self.temperature_scan_material_at(search_x, search_y) != Some(material)
                    {
                        search_y += y_direction;
                        if (search_y - y).abs() > search_range {
                            break;
                        }
                    }
                    if !(0..height).contains(&search_y) || (search_y - y).abs() > search_range {
                        break;
                    }
                }
            }
        }

        let mut converted_y = y;
        while remaining >= 0 && (0..height).contains(&converted_y) {
            if self.temperature_scan_material_at(x, converted_y) != Some(material) {
                break;
            }
            let left_material = if x > 0 {
                self.temperature_scan_material_at(x - 1, converted_y)
            } else {
                None
            };
            if left_material == Some(material) {
                break;
            }
            let Some(old_byte) = self.grid_byte_at(x, converted_y) else {
                break;
            };
            self.grid_set_byte(x, converted_y, action.target_byte | (old_byte & 0x80));
            on_instability(self, x, converted_y);
            converted_y += y_direction;
            remaining -= 1;
        }
        (converted_y - y).abs()
    }

    fn column_material(&self, index: usize) -> Option<MaterialId> {
        self.solid_materials
            .get(index)
            .copied()
            .flatten()
            .or(self.default_solid_material)
    }

    fn ensure_liquid_capacity(&mut self) {
        if self.liquids.len() != self.surface.len() {
            self.invalidate_estimated_height();
            self.liquids
                .resize(self.surface.len(), LiquidColumn::default());
        }
    }

    #[allow(dead_code)]
    fn is_within_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && (x as u32) < self.width && y >= 0
    }

    pub fn liquid_material_at(&self, x: i32, y: i32) -> Option<MaterialId> {
        if x < 0 {
            return None;
        }
        let index = usize::try_from(x).ok()?;
        self.liquids
            .get(index)
            .and_then(|column| column.material_at(y, self.default_liquid_material))
    }

    pub fn material_at(&self, x: i32, y: i32) -> Option<MaterialId> {
        // Pix2Mat (C4Wrappers.h:120-129) once the grid's material table is
        // resolved; falls back to the column model otherwise.
        if let Some(grid) = &self.pixels {
            if let Some(material) = grid.material_id_at(x, y) {
                return Some(material);
            }
            if grid.density_at(x, y).map(|d| d > 0).unwrap_or(false) {
                // Classified non-sky pixel whose material name did not
                // resolve: keep the column approximation as a stand-in.
            } else {
                return None;
            }
        }
        if self.is_solid_at(x, y) {
            self.solid_material_at(x)
        } else {
            self.liquid_material_at(x, y)
        }
    }

    /// Selected `C4Landscape::MatCount`/`EffectiveMatCount` value for one
    /// material. `minimum_height=None` is the raw per-pixel count; `Some`
    /// counts only complete vertical runs reaching `MinHeightCount`
    /// (C4Landscape.cpp:2904-2967). The value wraps as the C++ `uint32_t`
    /// counters do (C4Landscape.h:59-60).
    pub fn material_pixel_count(&self, material: MaterialId, minimum_height: Option<i32>) -> u32 {
        let (width, height) = self
            .pixels
            .as_ref()
            .map(|grid| (grid.width() as i32, grid.height() as i32))
            .unwrap_or((self.width as i32, self.estimated_height()));
        let mut count = 0_u32;
        for x in 0..width {
            let mut run = 0_u32;
            for y in 0..height {
                if self.material_at(x, y) == Some(material) {
                    run = run.wrapping_add(1);
                    if minimum_height.is_none() {
                        count = count.wrapping_add(1);
                    }
                } else if let Some(minimum_height) = minimum_height {
                    if i64::from(run) >= i64::from(minimum_height) {
                        count = count.wrapping_add(run);
                    }
                    run = 0;
                }
            }
            if let Some(minimum_height) = minimum_height {
                if i64::from(run) >= i64::from(minimum_height) {
                    count = count.wrapping_add(run);
                }
            }
        }
        count
    }

    /// The GetPix border rules (C4Landscape.h:144-161), checked in the C++
    /// branch order (x before y). `None` = in bounds, read the landscape.
    fn border_pixel(&self, x: i32, y: i32) -> Option<BorderPixel> {
        let open = |open: bool| {
            if open {
                BorderPixel::Sky
            } else {
                BorderPixel::Vehicle
            }
        };
        if x < 0 {
            return Some(open(y < self.left_open));
        }
        if x as u32 >= self.width {
            return Some(open(y < self.right_open));
        }
        if y < 0 {
            return Some(open(self.top_open));
        }
        if y >= self.estimated_height() {
            return Some(open(self.bottom_open));
        }
        None
    }

    /// `GBackMat` (C4Wrappers.h:164-167 → GetMat → GetPix): material lookup
    /// with the closed-border MCVehic mapping — past a closed side/bottom
    /// the landscape answers the Vehicle material instead of sky, so PXS
    /// and material reactions interact with the border like C++.
    pub fn border_material_at(&self, x: i32, y: i32) -> Option<MaterialId> {
        match self.border_pixel(x, y) {
            Some(BorderPixel::Sky) => None,
            Some(BorderPixel::Vehicle) => self.vehicle_material,
            None => self.material_at(x, y),
        }
    }

    pub fn density_at(&self, x: i32, y: i32, materials: &MaterialSet) -> i32 {
        // GBackDensity = Pix2Dens[GetPix] (C4Wrappers.h:169-172) with the
        // GetPix border rules (C4Landscape.h:144-161).
        match self.border_pixel(x, y) {
            Some(BorderPixel::Sky) => return C4M_BACKGROUND,
            Some(BorderPixel::Vehicle) => return C4M_VEHICLE,
            None => {}
        }
        if let Some(grid) = &self.pixels {
            return grid.density_at(x, y).unwrap_or(C4M_BACKGROUND);
        }
        match self.material_at(x, y) {
            Some(material_id) => materials
                .get_by_id(material_id)
                .map(|material| material.density())
                .unwrap_or(C4M_BACKGROUND),
            None => C4M_BACKGROUND,
        }
    }

    /// `GBackIFT` (C4Wrappers.h:174-177): the pixel's IFT bit with the
    /// GetPix border rules (border sky/MCVehic bytes carry no IFT).
    pub fn is_ift_at(&self, x: i32, y: i32) -> bool {
        if self.border_pixel(x, y).is_some() {
            return false;
        }
        if let Some(grid) = &self.pixels {
            return grid
                .byte_at(x, y)
                .map(|byte| byte & 0x80 != 0)
                .unwrap_or(false);
        }
        self.tunnels
            .get(&(x.max(0) as u32))
            .map(|ranges| ranges.iter().any(|&(top, bottom)| y >= top && y <= bottom))
            .unwrap_or(false)
    }

    /// C4Landscape::ScenarioInit border-open assignment
    /// (C4Landscape.cpp:67-71) from the Scenario.txt keys.
    pub fn set_border_open(
        &mut self,
        left_open: i32,
        right_open: i32,
        top_open: bool,
        bottom_open: bool,
    ) {
        self.left_open = left_open;
        self.right_open = right_open;
        self.top_open = top_open;
        self.bottom_open = bottom_open;
    }

    pub fn left_open(&self) -> i32 {
        self.left_open
    }

    pub fn right_open(&self) -> i32 {
        self.right_open
    }

    pub fn top_open(&self) -> bool {
        self.top_open
    }

    pub fn bottom_open(&self) -> bool {
        self.bottom_open
    }

    /// `MVehic` resolution (C4Game::InitMaterialTexture, C4Game.cpp:1669).
    pub fn set_vehicle_material(&mut self, material: Option<MaterialId>) {
        self.vehicle_material = material;
    }

    /// C4Landscape::ScanSideOpen (C4Landscape.cpp:231-238): LeftOpen /
    /// RightOpen become the first y whose border-column pixel is non-zero
    /// (non-sky). Runs when the scenario sets AutoScanSideOpen
    /// (C4Landscape::ScenarioInit, C4Landscape.cpp:72-73).
    pub fn scan_side_open(&mut self) {
        if self.width == 0 {
            self.left_open = 0;
            self.right_open = 0;
            return;
        }
        let height = self.estimated_height();
        let first_non_sky = |x: i32| {
            (0..height)
                .find(|&y| match &self.pixels {
                    Some(grid) => grid.byte_at(x, y).map(|byte| byte != 0).unwrap_or(false),
                    None => self.material_at(x, y).is_some(),
                })
                .unwrap_or(height)
        };
        let left = first_non_sky(0);
        let right = first_non_sky(self.width as i32 - 1);
        self.left_open = left;
        self.right_open = right;
    }

    pub fn find_liquid_surface(
        &self,
        material: MaterialId,
        mut x: i32,
        mut y: i32,
        materials: &MaterialSet,
    ) -> (i32, i32) {
        let max_slide = materials
            .get_by_id(material)
            .map(|mat| mat.max_slide())
            .unwrap_or(0);
        loop {
            let mut slide_direction: Option<(i32, i32)> = None;
            let mut left_active = true;
            let mut right_active = true;
            let mut cslide = 0;
            while cslide <= max_slide && (left_active || right_active) {
                if left_active {
                    if self.material_at(x - cslide, y) != Some(material) {
                        left_active = false;
                    } else if self.material_at(x - cslide, y - 1) == Some(material) {
                        slide_direction = Some((-cslide, -1));
                        break;
                    }
                }
                if right_active {
                    if self.material_at(x + cslide, y) != Some(material) {
                        right_active = false;
                    } else if self.material_at(x + cslide, y - 1) == Some(material) {
                        slide_direction = Some((cslide, -1));
                        break;
                    }
                }
                cslide += 1;
            }

            match slide_direction {
                Some((dx, dy)) => {
                    x += dx;
                    y += dy;
                }
                None => break,
            }
        }
        (x, y)
    }

    pub fn remove_liquid_at(&mut self, x: i32, y: i32) -> Option<MaterialId> {
        if x < 0 {
            return None;
        }
        let Ok(index) = usize::try_from(x) else {
            return None;
        };
        self.invalidate_estimated_height();
        self.ensure_liquid_capacity();
        let column = self.liquids.get_mut(index)?;
        if let Some(material) = column.remove_pixel(y) {
            if let Some(grid) = self.pixels.as_mut() {
                grid.clear_pixel(x, y);
            }
            material.or(self.default_liquid_material)
        } else {
            None
        }
    }

    pub fn insert_liquid_at(&mut self, x: i32, y: i32, material: Option<MaterialId>) -> bool {
        if x < 0 {
            return false;
        }
        let Ok(index) = usize::try_from(x) else {
            return false;
        };
        let Some(surface_y) = self.surface_height(x) else {
            return false;
        };
        if y >= surface_y {
            return false;
        }
        self.invalidate_estimated_height();
        self.ensure_liquid_capacity();
        let column = self.liquids.get_mut(index).expect("column must exist");
        let desired_material = material.or(self.default_liquid_material);
        let inserted = column.insert_pixel(y, desired_material);
        if inserted {
            if let Some(grid) = self.pixels.as_mut() {
                if let Some(byte) = desired_material.and_then(|id| grid.byte_for_material(id)) {
                    grid.set_byte(x, y, byte);
                }
            }
        }
        inserted
    }

    pub fn lower_range(&mut self, start: i32, end: i32, height: i32) {
        if start >= end {
            return;
        }
        let width = self.width as i32;
        let clamped_start = start.clamp(0, width);
        let clamped_end = end.clamp(0, width);
        if clamped_start >= clamped_end {
            return;
        }
        self.invalidate_estimated_height();
        let target_height = height.max(0);
        for x in clamped_start..clamped_end {
            let old = self.surface.get(x as usize).copied();
            if let Some(slot) = self.surface.get_mut(x as usize) {
                if target_height > *slot {
                    *slot = target_height;
                    if let Some(old) = old {
                        self.grid_track_surface(x, old, target_height, None);
                    }
                }
            }
        }
    }

    pub fn ensure_surface_at_least(&mut self, column: i32, height: i32) {
        if column < 0 {
            return;
        }
        let index = match usize::try_from(column) {
            Ok(index) => index,
            Err(_) => return,
        };
        if index >= self.surface.len() {
            return;
        }
        self.invalidate_estimated_height();
        let target_height = height.max(0);
        let old = self.surface.get(index).copied();
        if let Some(slot) = self.surface.get_mut(index) {
            if *slot < target_height {
                *slot = target_height;
                if let Some(old) = old {
                    self.grid_track_surface(column, old, target_height, None);
                }
            }
        }
    }

    pub fn blast_circle(
        &mut self,
        center: Vector2,
        radius: i32,
        materials: &MaterialSet,
    ) -> BlastResult {
        let mut result = BlastResult::default();
        if radius < 0 || self.surface.is_empty() || materials.is_empty() {
            return result;
        }

        self.invalidate_estimated_height();
        self.ensure_material_capacity();

        // BlastMatCount (C4Landscape::BlastFree, C4Landscape.cpp:1044-1055):
        // count every valid in-circle material pixel BEFORE removal — the
        // C++ loop shape (ycnt -rad..=rad, lwdt = sqrt(rad²-ycnt²), xcnt
        // -lwdt..lwdt + the lwdt==0 extension). BlastShiftTo evaluation and
        // the Blast2Object/Blast2PXS cast amounts derive from this count,
        // so inert materials count too.
        for y_offset in -radius..=radius {
            let remaining =
                i64::from(radius) * i64::from(radius) - i64::from(y_offset) * i64::from(y_offset);
            let line_width = (remaining.max(0) as f64).sqrt() as i32;
            let y = center.y.saturating_add(y_offset);
            let extend = i32::from(line_width == 0);
            for x_offset in -line_width..line_width + extend {
                let x = center.x.saturating_add(x_offset);
                if let Some(material_id) = self.material_at(x, y) {
                    *result
                        .pixel_count_by_material
                        .entry(material_id)
                        .or_insert(0) += 1;
                    if let Some(material) = materials.get_by_id(material_id) {
                        if let Some(target) = material.blast_shift_to_target() {
                            // Preserve BlastFreePix's y/x scan order. One
                            // candidate represents one unconditional Random
                            // call, even when the write is immediately cleared
                            // or resolves to the source material.
                            result.shift_candidates.push(BlastShiftCandidate {
                                column: x,
                                material: material_id,
                                target,
                                pixel_count: 1,
                                apply_column_shift: !material.blast_free()
                                    && target != material_id
                                    && self.surface_height(x).is_some_and(|surface| y >= surface)
                                    && self.solid_material_at(x) == Some(material_id),
                            });
                        }
                    }
                }
            }
        }

        let width = self.width as i32;
        let radius_sq = i64::from(radius) * i64::from(radius);
        let mut column_targets: Vec<Option<i32>> = vec![None; self.surface.len()];

        for y_offset in -radius..=radius {
            let y_offset_i64 = i64::from(y_offset);
            let remaining = radius_sq - y_offset_i64 * y_offset_i64;
            if remaining < 0 {
                continue;
            }
            let horizontal = (remaining as f64).sqrt().floor() as i32;
            let start = -horizontal;
            let end = if horizontal == 0 { 1 } else { horizontal };
            let y = match i64::from(center.y).checked_add(y_offset_i64) {
                Some(value) => value,
                None => continue,
            };
            if y > i64::from(i32::MAX) {
                continue;
            }
            let y_i32 = y as i32;

            for x_offset in start..end {
                let Some(column) = center.x.checked_add(x_offset) else {
                    continue;
                };
                if column < 0 || column >= width {
                    continue;
                }
                let index = column as usize;
                let current_height = match self.surface.get(index) {
                    Some(&height) => height,
                    None => continue,
                };
                if y_i32 < current_height {
                    continue;
                }
                let candidate = y_i32.saturating_add(1).max(0);
                let slot = &mut column_targets[index];
                match slot {
                    Some(existing) => {
                        if candidate > *existing {
                            *existing = candidate;
                        }
                    }
                    None => *slot = Some(candidate),
                }
            }
        }

        for (index, maybe_target) in column_targets.into_iter().enumerate() {
            let Some(target_height) = maybe_target else {
                continue;
            };
            let current_height = self.surface[index];
            if target_height <= current_height {
                continue;
            }
            let Some(material_id) = self.column_material(index) else {
                continue;
            };
            let Some(material) = materials.get_by_id(material_id) else {
                continue;
            };

            let removed_height = target_height - current_height;
            if removed_height <= 0 {
                continue;
            }

            let column = index as i32;
            if !material.blast_free() {
                continue;
            }

            let old = self.surface[index];
            self.surface[index] = target_height;
            self.grid_track_surface(index as i32, old, target_height, None);
            result
                .removed_by_material
                .entry(material_id)
                .and_modify(|count| *count += removed_height)
                .or_insert(removed_height);
            result.affected_columns.push((column, target_height));
        }

        result
    }
    pub fn can_incinerate(&self, x: i32, y: i32, materials: &MaterialSet) -> bool {
        // C4Landscape::Incinerate reads GetMat(x, y), so the authoritative
        // raster pixel (and the normal GetPix border rules) select the
        // material; the derived column surface is irrelevant here
        // (C4Landscape.cpp:1417-1426).
        self.border_material_at(x, y)
            .and_then(|material| materials.get_by_id(material))
            .is_some_and(|material| material.inflammable() != 0)
    }

    pub fn surface_height(&self, x: i32) -> Option<i32> {
        if self.surface.is_empty() {
            return None;
        }
        if x < 0 {
            return None;
        }
        let max_index = (self.width.saturating_sub(1)) as i32;
        if x > max_index {
            return None;
        }
        self.surface.get(x as usize).copied()
    }

    pub fn is_liquid_at(&self, x: i32, y: i32) -> bool {
        // GBackLiquid = DensityLiquid(GBackDensity(x, y))
        // (C4Wrappers.h:179-182): density in [C4M_Liquid, C4M_Solid).
        // The per-pixel grid is authoritative when present (static maps
        // paint their water as material pixels, never as liquid
        // columns); the segment columns remain the fixture model.
        if let Some(grid) = &self.pixels {
            if x >= 0 && (x as u32) < self.width && y >= 0 && y < self.estimated_height() {
                return grid
                    .density_at(x, y)
                    .map(|density| (C4M_LIQUID..C4M_SOLID).contains(&density))
                    .unwrap_or(false);
            }
            return false;
        }
        if x < 0 {
            return false;
        }
        let index = match usize::try_from(x) {
            Ok(index) => index,
            Err(_) => return false,
        };
        match self.liquids.get(index) {
            Some(column) => column.contains(y),
            None => false,
        }
    }

    pub fn is_solid_at(&self, x: i32, y: i32) -> bool {
        // GBackSolid reads DensitySolid(GetDensity), so configured GetPix
        // borders apply before the in-bounds landscape: open borders are
        // sky and closed borders are MCVehic (C4Landscape.h:144-180;
        // C4Wrappers.h:169-177). Strict bounds remain a separate concern of
        // LandscapeFree/C4PathFinder::PointFree (C4Game.cpp:2288-2292).
        match self.border_pixel(x, y) {
            Some(BorderPixel::Sky) => return false,
            Some(BorderPixel::Vehicle) => return true,
            None => {}
        }
        // GBackSolid = DensitySolid(Pix2Dens[pix]) (C4Wrappers.h:174-177):
        // the pixel plane is the truth — caves are AIR below the column
        // surface, water is never solid.
        if let Some(grid) = &self.pixels {
            return grid.density_at(x, y).unwrap_or(0) >= C4M_SOLID;
        }
        match self.surface_height(x) {
            Some(surface_y) => y >= surface_y,
            None => false,
        }
    }

    /// `PathFreeIgnoreVehiclePix` (C4Landscape.cpp:2044-2052) passes the
    /// *material index* from `GetPixMat` to `DensitySolid`, rather than the
    /// material's density. Consequently every material below index 50 is
    /// passable, irrespective of density; MVehic is passable at any index.
    pub(crate) fn is_solid_ignoring_vehicle_at(&self, x: i32, y: i32) -> bool {
        match self.border_pixel(x, y) {
            Some(BorderPixel::Sky | BorderPixel::Vehicle) => return false,
            None => {}
        }
        if let Some(grid) = &self.pixels {
            let Some(byte) = grid.byte_at(x, y) else {
                return false;
            };
            if byte == 0 {
                return false;
            }
            return grid.material_for_byte(byte).is_some_and(|material| {
                material.index() >= C4M_SOLID as usize && Some(material) != self.vehicle_material
            });
        }

        self.material_at(x, y).is_some_and(|material| {
            material.index() >= C4M_SOLID as usize && Some(material) != self.vehicle_material
        })
    }

    /// `GBackSemiSolid` (C4Material.h:202): density >= C4M_SemiSolid(25),
    /// which both solid ground and liquids satisfy.
    pub fn is_semi_solid_at(&self, x: i32, y: i32) -> bool {
        self.is_solid_at(x, y) || self.is_liquid_at(x, y)
    }

    /// `AboveSemiSolid` (C4Landscape.cpp:1736-1755): nearest free directly
    /// above semi-solid, scanning up and down simultaneously from `y`. The
    /// UP scan returns the first FREE pixel once it has passed semi-solid;
    /// the DOWN scan returns the first SEMI-SOLID pixel once it has passed
    /// free space — the asymmetry is load-bearing for placement results.
    pub fn above_semi_solid(&self, x: i32, y: i32) -> Option<i32> {
        let height = self.estimated_height();
        let (mut cy1, mut cy2) = (y, y);
        let mut use_upwards_next_free = false;
        let mut use_downwards_next_solid = false;
        while cy1 >= 0 || cy2 < height {
            if cy1 >= 0 {
                if self.is_semi_solid_at(x, cy1) {
                    use_upwards_next_free = true;
                } else if use_upwards_next_free {
                    return Some(cy1);
                }
            }
            if cy2 < height {
                if !self.is_semi_solid_at(x, cy2) {
                    use_downwards_next_solid = true;
                } else if use_downwards_next_solid {
                    return Some(cy2);
                }
            }
            cy1 -= 1;
            cy2 += 1;
        }
        None
    }

    /// `AboveSolid` (C4Landscape.cpp:1757-1782): nearest pixel free of
    /// SEMI-solid that rests directly on solid, scanning up and down. The
    /// down scan is guarded by `cy2 + 1 < GBackHgt`.
    pub fn above_solid(&self, x: i32, y: i32) -> Option<i32> {
        let height = self.estimated_height();
        let (mut cy1, mut cy2) = (y, y);
        while cy1 >= 0 || cy2 < height {
            if cy1 >= 0 && !self.is_semi_solid_at(x, cy1) && self.is_solid_at(x, cy1 + 1) {
                return Some(cy1);
            }
            if cy2 + 1 < height && !self.is_semi_solid_at(x, cy2) && self.is_solid_at(x, cy2 + 1) {
                return Some(cy2);
            }
            cy1 -= 1;
            cy2 += 1;
        }
        None
    }

    /// `SemiAboveSolid` (C4Landscape.cpp:1784-1809): like [`above_solid`]
    /// but only requires freedom from SOLID — a liquid surface row resting
    /// on rock qualifies.
    pub fn semi_above_solid(&self, x: i32, y: i32) -> Option<i32> {
        let height = self.estimated_height();
        let (mut cy1, mut cy2) = (y, y);
        while cy1 >= 0 || cy2 < height {
            if cy1 >= 0 && !self.is_solid_at(x, cy1) && self.is_solid_at(x, cy1 + 1) {
                return Some(cy1);
            }
            if cy2 + 1 < height && !self.is_solid_at(x, cy2) && self.is_solid_at(x, cy2 + 1) {
                return Some(cy2);
            }
            cy1 -= 1;
            cy2 += 1;
        }
        None
    }

    /// Follow one fixed-point ballistic path until it leaves the landscape
    /// or strikes solid terrain, retaining the closest whole-pixel distance
    /// to `target` (`TrajectoryDistance`, C4Landscape.cpp:2055-2068).
    fn throwing_trajectory_distance(
        &self,
        start: Vector2,
        mut velocity: FixedVec2,
        target: Vector2,
        gravity: C4Fixed,
    ) -> i32 {
        let mut closest = math::integer_distance(start.x, start.y, target.x, target.y);
        let mut position = FixedVec2::from_ints(start.x, start.y);
        let width = i32::try_from(self.width()).unwrap_or(i32::MAX);
        let height = self.estimated_height();
        loop {
            let pixel = Vector2::new(math::fixtoi(position.x), math::fixtoi(position.y));
            if !(0..width).contains(&pixel.x)
                || !(0..height).contains(&pixel.y)
                || self.is_solid_at(pixel.x, pixel.y)
            {
                return closest;
            }
            closest = closest.min(math::integer_distance(pixel.x, pixel.y, target.x, target.y));
            position.x += velocity.x;
            position.y += velocity.y;
            velocity.y += gravity;
        }
    }

    /// Find the first surface point from which `launch` passes within two
    /// pixels of `target`. The search starts beneath the target and walks at
    /// most sixty pixels opposite the horizontal launch direction
    /// (`FindThrowingPosition`, C4Landscape.cpp:2073-2103).
    pub(crate) fn find_throwing_position(
        &self,
        target: Vector2,
        launch: FixedVec2,
        object_height: i32,
        gravity: C4Fixed,
    ) -> Option<Vector2> {
        let width = i32::try_from(self.width()).unwrap_or(i32::MAX);
        let mut y = self.semi_above_solid(target.x, target.y)?;
        if !(-50..=50).contains(&(y - target.y)) {
            return None;
        }

        let direction = if launch.x > C4Fixed::ZERO { -1 } else { 1 };
        let mut x = target.x;
        for _ in 0..=60 {
            if !(0..width).contains(&x) {
                return None;
            }
            y = self.semi_above_solid(x, y)?;
            if self.throwing_trajectory_distance(
                Vector2::new(x, y - object_height),
                launch,
                target,
                gravity,
            ) <= 2
            {
                return Some(Vector2::new(x, y));
            }
            x += direction;
        }
        None
    }

    /// Search expanding ten-pixel rings for the first in-bounds pixel that
    /// is not semi-solid (`FindClosestFree`, C4Landscape.cpp:2102-2123).
    /// Angles are visited in ascending order with an exclusive upper bound;
    /// the exclusion interval is inclusive like C++ `Inside`.
    pub(crate) fn find_closest_free(
        &self,
        origin: Vector2,
        angle_start: i32,
        angle_end: i32,
        exclude_start: i32,
        exclude_end: i32,
    ) -> Option<Vector2> {
        let width = i32::try_from(self.width()).unwrap_or(i32::MAX);
        let height = self.estimated_height();
        for radius in (10..200).step_by(10) {
            for angle in (angle_start..angle_end).step_by(10) {
                if (exclude_start..=exclude_end).contains(&angle) {
                    continue;
                }
                let x = origin.x + math::fixtoi(math::itofix(angle).sin_deg() * radius);
                let y = origin.y - math::fixtoi(math::itofix(angle).cos_deg() * radius);
                if (0..width).contains(&x)
                    && (0..height).contains(&y)
                    && !self.is_semi_solid_at(x, y)
                {
                    return Some(Vector2::new(x, y));
                }
            }
        }
        None
    }

    /// `FindSolidGround` (C4Landscape.cpp:1843-1869): starting from (x, y),
    /// search left and right for a `width`-long run of columns with solid
    /// ground; returns the bottom center of the surface found (run-center
    /// x, with the y settled by a final [`above_semi_solid`]). The left run
    /// is checked first, and the per-side y trackers persist across columns
    /// exactly like the C++ by-ref `cy1`/`cy2`.
    pub fn find_solid_ground(&self, x: i32, y: i32, width: i32) -> Option<(i32, i32)> {
        let back_width = self.width as i32;
        let (mut cx1, mut cx2) = (x, x);
        let (mut cy1, mut cy2) = (y, y);
        let (mut rl1, mut rl2) = (0i32, 0i32);
        while cx1 > 0 || cx2 < back_width {
            if cx1 >= 0 {
                match self.above_solid(cx1, cy1) {
                    Some(adjusted) => {
                        cy1 = adjusted;
                        rl1 += 1;
                    }
                    None => rl1 = 0,
                }
            }
            if cx2 < back_width {
                match self.above_solid(cx2, cy2) {
                    Some(adjusted) => {
                        cy2 = adjusted;
                        rl2 += 1;
                    }
                    None => rl2 = 0,
                }
            }
            if rl1 >= width {
                let rx = cx1 + rl1 / 2;
                return Some((rx, self.above_semi_solid(rx, cy1).unwrap_or(cy1)));
            }
            if rl2 >= width {
                let rx = cx2 - rl2 / 2;
                return Some((rx, self.above_semi_solid(rx, cy2).unwrap_or(cy2)));
            }
            cx1 -= 1;
            cx2 += 1;
        }
        None
    }

    /// `FindLevelGround` (C4Landscape.cpp:1942-1976): like
    /// [`find_solid_ground`] but follows the surface with
    /// [`above_semi_solid`], resets the run when the surface height jumps
    /// by `hrange` or more, and aborts a side whose column has no surface.
    /// The first checked columns are `x - 1` and `x + 1` (the C++ for-init
    /// decrement/increment).
    pub fn find_level_ground(&self, x: i32, y: i32, width: i32, hrange: i32) -> Option<(i32, i32)> {
        let back_width = self.width as i32;
        let (mut cx1, mut cx2) = (x, x);
        let (mut cy1, mut cy2) = (y, y);
        let (mut rh1, mut rh2) = (cy1, cy2);
        let (mut rl1, mut rl2) = (0i32, 0i32);
        cx1 -= 1;
        cx2 += 1;
        while cx1 > 0 || cx2 < back_width {
            if cx1 > 0 {
                match self.above_semi_solid(cx1, cy1) {
                    None => cx1 = -1, // abort left
                    Some(adjusted) => {
                        cy1 = adjusted;
                        if self.is_solid_at(cx1, cy1 + 1) && (cy1 - rh1).abs() < hrange {
                            rl1 += 1;
                        } else {
                            rl1 = 0;
                            rh1 = cy1;
                        }
                    }
                }
            }
            if cx2 < back_width {
                match self.above_semi_solid(cx2, cy2) {
                    None => cx2 = back_width, // abort right
                    Some(adjusted) => {
                        cy2 = adjusted;
                        if self.is_solid_at(cx2, cy2 + 1) && (cy2 - rh2).abs() < hrange {
                            rl2 += 1;
                        } else {
                            rl2 = 0;
                            rh2 = cy2;
                        }
                    }
                }
            }
            if rl1 >= width {
                let rx = cx1 + rl1 / 2;
                return Some((rx, self.above_semi_solid(rx, cy1).unwrap_or(cy1)));
            }
            if rl2 >= width {
                let rx = cx2 - rl2 / 2;
                return Some((rx, self.above_semi_solid(rx, cy2).unwrap_or(cy2)));
            }
            cx1 -= 1;
            cx2 += 1;
        }
        None
    }

    /// `FindConSiteSpot` (C4Landscape.cpp:1982-2043): level-ground search
    /// for a construction site, with offset starting positions and an
    /// object-overlap veto. `overlaps(x, y, wdt, hgt)` must answer
    /// `Game.OverlapObject` for the site rect `(x, y - hgt - 10, wdt,
    /// hgt + 40)` — the caller supplies it because object knowledge lives
    /// outside the landscape. `hrange == -1` selects the standard
    /// smooth-surface limit `max(wdt / 4, 5)`.
    pub fn find_con_site_spot(
        &self,
        x: i32,
        y: i32,
        wdt: i32,
        hgt: i32,
        hrange: i32,
        overlaps: impl Fn(i32, i32, i32, i32) -> bool,
    ) -> Option<(i32, i32)> {
        let hrange = if hrange == -1 {
            (wdt / 4).max(5)
        } else {
            hrange
        };
        let back_width = self.width as i32;

        // Left offset starting position; fall back to centered.
        let mut cx1 = (x + wdt / 2).min(back_width - 1);
        let mut cy1 = y;
        match self.above_semi_solid(cx1, cy1) {
            Some(adjusted) => cy1 = adjusted,
            None => {
                cx1 = x.min(back_width - 1);
                cy1 = y;
            }
        }
        // Right offset starting position; fall back to centered.
        let mut cx2 = (x - wdt / 2).max(0);
        let mut cy2 = y;
        match self.above_semi_solid(cx2, cy2) {
            Some(adjusted) => cy2 = adjusted,
            None => {
                cx2 = x.min(back_width - 1);
                cy2 = y;
            }
        }

        let (mut rh1, mut rh2) = (cy1, cy2);
        let (mut rl1, mut rl2) = (0i32, 0i32);
        cx1 -= 1;
        cx2 += 1;
        while cx1 > 0 || cx2 < back_width {
            if cx1 > 0 {
                match self.above_semi_solid(cx1, cy1) {
                    None => cx1 = -1, // abort left
                    Some(adjusted) => {
                        cy1 = adjusted;
                        if self.is_solid_at(cx1, cy1 + 1) && (cy1 - rh1).abs() < hrange {
                            rl1 += 1;
                        } else {
                            rl1 = 0;
                            rh1 = cy1;
                        }
                    }
                }
            }
            if cx2 < back_width {
                match self.above_semi_solid(cx2, cy2) {
                    None => cx2 = back_width, // abort right
                    Some(adjusted) => {
                        cy2 = adjusted;
                        if self.is_solid_at(cx2, cy2 + 1) && (cy2 - rh2).abs() < hrange {
                            rl2 += 1;
                        } else {
                            rl2 = 0;
                            rh2 = cy2;
                        }
                    }
                }
            }
            if rl1 >= wdt && cx1 > 0 && !overlaps(cx1, cy1 - hgt - 10, wdt, hgt + 40) {
                let rx = cx1 + wdt / 2;
                return Some((rx, self.above_semi_solid(rx, cy1).unwrap_or(cy1)));
            }
            if rl2 >= wdt && cx2 < back_width && !overlaps(cx2 - wdt, cy2 - hgt - 10, wdt, hgt + 40)
            {
                let rx = cx2 - wdt / 2;
                return Some((rx, self.above_semi_solid(rx, cy2).unwrap_or(cy2)));
            }
            cx1 -= 1;
            cx2 += 1;
        }
        None
    }

    /// One 17×15 `PixCnt` cell's occupancy: whether any pixel in the cell has
    /// nonzero density (`UpdatePixCnt` counts `_GetDensity(x, y) != 0`,
    /// C4Landscape.cpp:2894-2908). Computed from the authoritative pixel grid
    /// when present, with the column model retained for fixture landscapes.
    fn pix_cnt_cell_occupied(&self, cell_x: i32, cell_y: i32, materials: &MaterialSet) -> bool {
        let top = cell_y * 15;
        let bottom = top + 15;
        let left = (cell_x * 17).max(0);
        let right = (cell_x * 17 + 17).min(self.width as i32);
        if let Some(grid) = self.pixels.as_ref() {
            let top = top.max(0);
            let bottom = bottom.min(grid.height() as i32);
            return (left..right).any(|x| {
                (top..bottom).any(|y| grid.density_at(x, y).is_some_and(|density| density != 0))
            });
        }
        for x in left..right {
            // solid part of the column inside the cell rows
            if let Some(surface) = self.surface_height(x) {
                if surface < bottom {
                    let solid_density = self
                        .solid_material_at(x)
                        .and_then(|id| materials.get_by_id(id))
                        .map(|material| material.density())
                        .unwrap_or(0);
                    if solid_density != 0 && surface.max(top) < bottom {
                        return true;
                    }
                }
            }
            // liquid segments overlapping the cell rows
            if let Some(column) = self.liquids.get(x as usize) {
                // segments are inclusive of both top and bottom rows
                for segment in column.segments() {
                    if segment.top < bottom && segment.bottom >= top {
                        let density = segment
                            .material
                            .or(self.default_liquid_material)
                            .and_then(|id| materials.get_by_id(id))
                            .map(|material| material.density())
                            .unwrap_or(0);
                        if density != 0 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// `C4Landscape::_PathFree` (C4Landscape.cpp:890-915): coarse-grid path
    /// check on 17×15 cells — diagonal steps while both axes differ, then a
    /// straight run, then the destination cell itself.
    pub fn path_free(&self, x: i32, y: i32, x2: i32, y2: i32, materials: &MaterialSet) -> bool {
        let (mut x, mut y, x2, y2) = (x / 17, y / 15, x2 / 17, y2 / 15);
        while x != x2 && y != y2 {
            if self.pix_cnt_cell_occupied(x, y, materials) {
                return false;
            }
            if x > x2 {
                x -= 1;
            } else {
                x += 1;
            }
            if y > y2 {
                y -= 1;
            } else {
                y += 1;
            }
        }
        if x != x2 {
            loop {
                if self.pix_cnt_cell_occupied(x, y, materials) {
                    return false;
                }
                if x > x2 {
                    x -= 1;
                } else {
                    x += 1;
                }
                if x == x2 {
                    break;
                }
            }
        } else {
            while y != y2 {
                if self.pix_cnt_cell_occupied(x, y, materials) {
                    return false;
                }
                if y > y2 {
                    y -= 1;
                } else {
                    y += 1;
                }
            }
        }
        !self.pix_cnt_cell_occupied(x, y2, materials)
    }

    /// `C4Landscape::FindMatSlide` (C4Landscape.cpp:1260-1290): find the
    /// closest immediate slide position for a material of density `mdens`.
    /// Straight down first, then per `cslide` ring check left before right; a
    /// side is clogged only when both its cells are >= `mdens`; a slide fires
    /// when the side's down-cell is free. Mutates `fx`/`fy` on success.
    /// C4Landscape::FindMatPath (C4Landscape.cpp:1226-1256): the next pixel
    /// position toward the desired slide. Unlike FindMatSlide the sideways
    /// step advances `fx` by exactly ONE pixel and leaves `fy` unchanged.
    pub fn find_mat_path(
        &self,
        fx: &mut i32,
        fy: &mut i32,
        ydir: i32,
        mdens: i32,
        mslide: i32,
        materials: &MaterialSet,
    ) -> bool {
        // One downwards
        if self.density_at(*fx, *fy + ydir, materials) < mdens {
            *fy += ydir;
            return true;
        }
        // Find downwards slide path
        let (mut left, mut right) = (true, true);
        let mut cslide = 1;
        while cslide <= mslide && (left || right) {
            // Check left
            if left {
                if self.density_at(*fx - cslide, *fy, materials) >= mdens {
                    // Left clogged
                    left = false;
                } else if self.density_at(*fx - cslide, *fy + ydir, materials) < mdens {
                    // Left slide okay
                    *fx -= 1;
                    return true;
                }
            }
            // Check right
            if right {
                if self.density_at(*fx + cslide, *fy, materials) >= mdens {
                    // Right clogged
                    right = false;
                } else if self.density_at(*fx + cslide, *fy + ydir, materials) < mdens {
                    // Right slide okay
                    *fx += 1;
                    return true;
                }
            }
            cslide += 1;
        }
        false
    }

    pub fn find_mat_slide(
        &self,
        fx: &mut i32,
        fy: &mut i32,
        ydir: i32,
        mdens: i32,
        mslide: i32,
        materials: &MaterialSet,
    ) -> bool {
        // One downwards
        if self.density_at(*fx, *fy + ydir, materials) < mdens {
            *fy += ydir;
            return true;
        }
        // Find downwards slide path
        let (mut left, mut right) = (true, true);
        let mut cslide = 1;
        while cslide <= mslide && (left || right) {
            // Check left
            if left {
                if self.density_at(*fx - cslide, *fy, materials) >= mdens
                    && self.density_at(*fx - cslide, *fy + ydir, materials) >= mdens
                {
                    left = false;
                } else if self.density_at(*fx - cslide, *fy + ydir, materials) < mdens {
                    *fx -= cslide;
                    *fy += ydir;
                    return true;
                }
            }
            // Check right
            if right {
                if self.density_at(*fx + cslide, *fy, materials) >= mdens
                    && self.density_at(*fx + cslide, *fy + ydir, materials) >= mdens
                {
                    right = false;
                } else if self.density_at(*fx + cslide, *fy + ydir, materials) < mdens {
                    *fx += cslide;
                    *fy += ydir;
                    return true;
                }
            }
            cslide += 1;
        }
        false
    }

    /// `C4Landscape::FindMatPathPush` (C4Landscape.cpp:1282-1415): walk the
    /// equal-density border and select the closest lower-density point.
    /// `liquid` is the material's `Instable` flag, which switches the C++
    /// distance calculation to its signed vertical term. Like C++, the
    /// border-pruning calculation assumes a positive `mslide`.
    pub(crate) fn find_mat_path_push(
        &self,
        fx: &mut i32,
        fy: &mut i32,
        mdens: i32,
        mslide: i32,
        liquid: bool,
        materials: &MaterialSet,
    ) -> bool {
        let Some((width, height)) = self.grid_dimensions() else {
            return false;
        };
        *fx = (*fx).clamp(0, width - 1);
        *fy = (*fy).clamp(0, height - 1);

        const PUSH_RANGE: i32 = 500;
        const RIGHT: i8 = 0;
        const DOWN: i8 = 1;
        const LEFT: i8 = 2;
        const UP: i8 = 3;

        let mut left = 0.max(*fx - PUSH_RANGE);
        let mut right = (width - 1).min(*fx + PUSH_RANGE);
        let mut top = 0.max(*fy - PUSH_RANGE);
        let mut bottom = (height - 1).min(*fy + PUSH_RANGE);
        let mut direction = RIGHT;
        let (mut x, mut y) = (*fx, *fy);
        let density = self.density_at(*fx, *fy, materials);
        if density < mdens {
            return true;
        }
        if density == mdens {
            let mut radius = 0;
            loop {
                if x - radius - 1 < left || self.density_at(x - radius - 1, y, materials) != mdens {
                    x -= radius;
                    direction = LEFT;
                    break;
                }
                if y - radius - 1 < top || self.density_at(x, y - radius - 1, materials) != mdens {
                    y -= radius;
                    direction = UP;
                    break;
                }
                if x + radius + 1 > right || self.density_at(x + radius + 1, y, materials) != mdens
                {
                    x += radius;
                    direction = RIGHT;
                    break;
                }
                if y + radius + 1 > bottom || self.density_at(x, y + radius + 1, materials) != mdens
                {
                    y += radius;
                    direction = DOWN;
                    break;
                }
                radius += 1;
            }
        } else {
            let mut radius = 1;
            while radius < PUSH_RANGE {
                if self.density_at(x - radius, y, materials) <= mdens {
                    x -= radius;
                    direction = RIGHT;
                    break;
                }
                if self.density_at(x, y - radius, materials) <= mdens {
                    y -= radius;
                    direction = DOWN;
                    break;
                }
                if self.density_at(x + radius, y, materials) <= mdens {
                    x += radius;
                    direction = LEFT;
                    break;
                }
                if self.density_at(x, y + radius, materials) <= mdens {
                    y += radius;
                    direction = UP;
                    break;
                }
                radius += 1;
            }
            if radius >= PUSH_RANGE {
                return false;
            }
            if self.density_at(x, y, materials) < mdens {
                *fx = x;
                *fy = y;
                return true;
            }
        }

        let (mut start_x, mut start_y, mut start_direction) = (x, y, direction);
        let mut best: Option<(i32, i32, i32)> = None;
        loop {
            debug_assert!(
                x >= left
                    && y >= top
                    && x <= right
                    && y <= bottom
                    && self.density_at(x, y, materials) == mdens
            );
            let (mut next_x, mut next_y) = (x, y);
            match direction {
                RIGHT => next_x += 1,
                DOWN => next_y += 1,
                LEFT => next_x -= 1,
                UP => next_y -= 1,
                _ => unreachable!("FindMatPathPush direction"),
            }
            let in_bounds = next_x >= left && next_y >= top && next_x <= right && next_y <= bottom;
            let density = self.density_at(next_x, next_y, materials);
            if density < mdens {
                let vertical = if liquid {
                    *fy - next_y
                } else {
                    (*fy - next_y).abs()
                };
                let distance = (next_x - *fx).abs() + mslide * vertical;
                if best.is_none_or(|(_, _, best_distance)| distance < best_distance) {
                    best = Some((next_x, next_y, distance));
                    top = top.max(*fy - distance / mslide - 1);
                    if !liquid {
                        bottom = bottom.min(*fy + distance / mslide + 1);
                        left = left.max(*fx - distance - 1);
                        right = right.min(*fx + distance + 1);
                    }
                    (start_x, start_y, start_direction) = (x, y, direction);
                }
            }
            if in_bounds && density == mdens {
                (x, y) = (next_x, next_y);
                direction = (direction + 3) % 4;
            } else {
                direction = (direction + 1) % 4;
            }
            if (x, y, direction) == (start_x, start_y, start_direction) {
                break;
            }
        }
        let Some((best_x, best_y, _)) = best else {
            return false;
        };
        *fx = best_x;
        *fy = best_y;
        true
    }

    /// The failure-producing prefix of `C4Landscape::InsertMaterial`. Once
    /// this succeeds, slide, reaction, and dead-pixel insertion paths all
    /// return true.
    pub(crate) fn insert_material_destination(
        &self,
        mut x: i32,
        mut y: i32,
        density: i32,
        landscape_push_pull: bool,
        max_slide: i32,
        liquid: bool,
        materials: &MaterialSet,
    ) -> Option<InsertMaterialDestination> {
        let Some((width, height)) = self.grid_dimensions() else {
            let index = usize::try_from(x).ok()?;
            return (index < self.surface.len()).then_some(InsertMaterialDestination::Column);
        };
        // C++ deliberately accepts y == Height (C4Landscape.cpp:1166).
        if !(0..width).contains(&x) || !(0..=height).contains(&y) {
            return None;
        }
        if landscape_push_pull {
            if self.density_at(x, y, materials) >= density
                && !self.find_mat_path_push(&mut x, &mut y, density, max_slide, liquid, materials)
            {
                return None;
            }
        } else {
            while density == self.density_at(x, y, materials) {
                y -= 1;
                if y < 0 {
                    return None;
                }
                if self.density_at(x - 1, y, materials) < density {
                    x -= 1;
                }
                if self.density_at(x + 1, y, materials) < density {
                    x += 1;
                }
            }
            if self.density_at(x, y, materials) > density {
                return None;
            }
        }
        Some(InsertMaterialDestination::Grid { x, y })
    }

    pub fn insert_material_at(&mut self, x: i32, y: i32, material: MaterialId) -> bool {
        if x < 0 {
            return false;
        }
        let index = match usize::try_from(x) {
            Ok(index) => index,
            Err(_) => return false,
        };
        if index >= self.surface.len() {
            return false;
        }
        self.invalidate_estimated_height();
        self.ensure_material_capacity();
        let current_height = self.surface[index].max(0);
        let mut target = y.saturating_add(1);
        if target < 0 {
            target = 0;
        }
        if target == current_height {
            self.solid_materials[index] = Some(material);
            return true;
        }
        let old = self.surface[index];
        self.surface[index] = target;
        let fill = self.default_material_byte_for_id(material);
        self.grid_track_surface(index as i32, old, target, fill);
        self.solid_materials[index] = Some(material);
        true
    }

    pub fn insert_material_pixel_at(
        &mut self,
        x: i32,
        y: i32,
        material: MaterialId,
        materials: &MaterialSet,
    ) -> bool {
        let Some(definition) = materials.get_by_id(material) else {
            return false;
        };
        if definition.is_liquid() {
            self.insert_liquid_at(x, y, Some(material))
        } else {
            self.insert_material_at(x, y, material)
        }
    }

    /// C4Landscape::FindMatTop (C4Landscape.cpp:1118-1146): slide the
    /// point up along same-material pixels (within the material's
    /// MaxSlide) to the column top — extraction always takes the
    /// SURFACE pixel, leaving the probed spot wet.
    /// Simulates FnExtractMaterialAmount's loop (C4Script.cpp:2264-2273:
    /// extract while `GBackMat(x,y) == mat`, each via ExtractMaterial's
    /// FindMatTop-then-clear) WITHOUT mutating, so the host fn can return
    /// the exact count and stage the real extraction as an operation.
    /// Valid while no earlier same-batch op touches these pixels.
    pub fn simulate_extract_material_amount(
        &self,
        materials: &MaterialSet,
        x: i32,
        y: i32,
        material: MaterialId,
        amount: i32,
    ) -> i32 {
        let max_slide = materials
            .get_by_id(material)
            .map(|entry| entry.max_slide())
            .unwrap_or(0);
        let mut cleared: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
        let mat_at = |cleared: &std::collections::HashSet<(i32, i32)>, px: i32, py: i32| {
            !cleared.contains(&(px, py)) && self.material_at(px, py) == Some(material)
        };
        let mut extracted = 0;
        while extracted < amount {
            if !mat_at(&cleared, x, y) {
                break;
            }
            // FindMatTop (C4Landscape.cpp:1118-1156) against the overlay
            let (mut tx, mut ty) = (x, y);
            loop {
                let mut left = true;
                let mut right = true;
                let mut slide = 0;
                let mut cslide = 0;
                while cslide <= max_slide && (left || right) {
                    if left {
                        if !mat_at(&cleared, tx - cslide, ty) {
                            left = false;
                        } else if mat_at(&cleared, tx - cslide, ty - 1) {
                            slide = 1;
                            break;
                        }
                    }
                    if right {
                        if !mat_at(&cleared, tx + cslide, ty) {
                            right = false;
                        } else if mat_at(&cleared, tx + cslide, ty - 1) {
                            slide = 2;
                            break;
                        }
                    }
                    cslide += 1;
                }
                match slide {
                    1 => {
                        tx -= cslide;
                        ty -= 1;
                    }
                    2 => {
                        tx += cslide;
                        ty -= 1;
                    }
                    _ => break,
                }
            }
            cleared.insert((tx, ty));
            extracted += 1;
        }
        extracted
    }

    fn find_mat_top(
        &self,
        material: MaterialId,
        mut x: i32,
        mut y: i32,
        materials: &MaterialSet,
    ) -> (i32, i32) {
        let max_slide = materials
            .get_by_id(material)
            .map(|entry| entry.max_slide())
            .unwrap_or(0);
        let mat_at = |x: i32, y: i32| self.material_at(x, y) == Some(material);
        loop {
            let mut left = true;
            let mut right = true;
            let mut slide = 0;
            let mut cslide = 0;
            while cslide <= max_slide && (left || right) {
                if left {
                    if !mat_at(x - cslide, y) {
                        left = false;
                    } else if mat_at(x - cslide, y - 1) {
                        slide = 1;
                        break;
                    }
                }
                if right {
                    if !mat_at(x + cslide, y) {
                        right = false;
                    } else if mat_at(x + cslide, y - 1) {
                        slide = 2;
                        break;
                    }
                }
                cslide += 1;
            }
            match slide {
                1 => {
                    x -= cslide;
                    y -= 1;
                }
                2 => {
                    x += cslide;
                    y -= 1;
                }
                _ => break,
            }
        }
        (x, y)
    }

    /// C4Landscape::ExtractMaterial's landscape half (C4Landscape.cpp:
    /// 1148-1153): FindMatTop + ClearPix, returning the extracted material
    /// AND the cleared coordinates — the by-ref `fx`/`fy` the C++ passes on
    /// to CheckInstabilityRange (:1154). The engine wrapper
    /// (`Engine::extract_material`) fires the instability probe there.
    pub fn extract_material_probe(
        &mut self,
        x: i32,
        y: i32,
        materials: &MaterialSet,
    ) -> Option<(MaterialId, i32, i32)> {
        let material = self.material_at(x, y)?;
        let (top_x, top_y) = self.find_mat_top(material, x, y, materials);
        if self.pixels.is_some() {
            self.clear_pix(top_x, top_y);
        } else {
            self.extract_material_at(top_x, top_y)?;
        }
        Some((material, top_x, top_y))
    }

    pub fn extract_material_at(&mut self, x: i32, y: i32) -> Option<MaterialId> {
        if self.is_liquid_at(x, y) {
            self.remove_liquid_at(x, y)
        } else {
            self.material_at(x, y)
                .filter(|_| self.is_solid_at(x, y))
                .filter(|&material| self.remove_material_at(x, y))
        }
    }

    pub fn remove_material_at(&mut self, x: i32, _y: i32) -> bool {
        if x < 0 {
            return false;
        }
        let index = match usize::try_from(x) {
            Ok(index) => index,
            Err(_) => return false,
        };
        self.invalidate_estimated_height();
        if let Some(height) = self.surface.get_mut(index) {
            if *height <= 0 {
                return false;
            }
            let target = (*height - 1).max(0);
            *height = target;
            if let Some(grid) = self.pixels.as_mut() {
                // The column op removes the TOP pixel; mirror it on the plane.
                grid.clear_pixel(index as i32, target);
            }
            true
        } else {
            false
        }
    }

    pub fn path_is_clear(&self, start: Vector2, end: Vector2) -> bool {
        self.first_collision_on_line(start, end).is_none()
    }

    pub fn first_collision_on_line(&self, start: Vector2, end: Vector2) -> Option<Vector2> {
        const MIN_COORD: i64 = i32::MIN as i64;
        const MAX_COORD: i64 = i32::MAX as i64;

        let mut x0 = i64::from(start.x);
        let mut y0 = i64::from(start.y);
        let x1 = i64::from(end.x);
        let y1 = i64::from(end.y);

        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 {
            1
        } else if x0 > x1 {
            -1
        } else {
            0
        };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 {
            1
        } else if y0 > y1 {
            -1
        } else {
            0
        };
        let mut err = dx + dy;

        loop {
            let clamped_x = x0.clamp(MIN_COORD, MAX_COORD) as i32;
            let clamped_y = y0.clamp(MIN_COORD, MAX_COORD) as i32;
            if self.is_solid_at(clamped_x, clamped_y) {
                return Some(Vector2::new(clamped_x, clamped_y));
            }
            if x0 == x1 && y0 == y1 {
                break;
            }
            let double_error = err * 2;
            if double_error >= dy {
                err += dy;
                if sx != 0 {
                    x0 += sx;
                }
            }
            if double_error <= dx {
                err += dx;
                if sy != 0 {
                    y0 += sy;
                }
            }
            if sx == 0 && sy == 0 {
                break;
            }
        }

        None
    }
}

struct EditorMapSurface<'a> {
    width: i32,
    height: i32,
    bytes: &'a mut [u8],
}

impl EditorMapSurface<'_> {
    fn write(&mut self, x: i32, y: i32, byte: u8) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        self.bytes[(y * self.width + x) as usize] = byte;
    }

    fn circle(&mut self, x: i32, y: i32, radius: i32, byte: u8) {
        if radius <= 0 {
            return;
        }
        for y_offset in -radius..radius {
            let remaining =
                i64::from(radius) * i64::from(radius) - i64::from(y_offset) * i64::from(y_offset);
            let half_width = (remaining as f32).sqrt() as i32;
            for x_counter in (0..half_width.saturating_mul(2)).rev() {
                self.write(
                    x.wrapping_sub(half_width).wrapping_add(x_counter),
                    y.wrapping_add(y_offset),
                    byte,
                );
            }
        }
    }

    fn stamp(&mut self, x: i32, y: i32, radius: i32, byte: u8) {
        // DrawLineMap/DrawBrush bypass CSurface8::Circle at radius one and
        // write exactly the center map pixel.
        if radius == 1 {
            self.write(x, y, byte);
        } else {
            self.circle(x, y, radius, byte);
        }
    }

    fn line(&mut self, mut x1: i32, mut y1: i32, mut x2: i32, mut y2: i32, radius: i32, byte: u8) {
        if x2.wrapping_sub(x1).abs() < y2.wrapping_sub(y1).abs() {
            if y1 > y2 {
                std::mem::swap(&mut x1, &mut x2);
                std::mem::swap(&mut y1, &mut y2);
            }
            let x_increment = if x2 > x1 { 1 } else { -1 };
            let dy = y2.wrapping_sub(y1);
            let dx = x2.wrapping_sub(x1).abs();
            let mut decision = dx.wrapping_mul(2).wrapping_sub(dy);
            let advance_both = dx.wrapping_sub(dy).wrapping_mul(2);
            let advance_major = dx.wrapping_mul(2);
            let mut x = x1;
            self.stamp(x, y1, radius, byte);
            let mut y = y1.wrapping_add(1);
            while y <= y2 {
                if decision >= 0 {
                    x = x.wrapping_add(x_increment);
                    decision = decision.wrapping_add(advance_both);
                } else {
                    decision = decision.wrapping_add(advance_major);
                }
                self.stamp(x, y, radius, byte);
                y = y.wrapping_add(1);
            }
        } else {
            if x1 > x2 {
                std::mem::swap(&mut x1, &mut x2);
                std::mem::swap(&mut y1, &mut y2);
            }
            let y_increment = if y2 > y1 { 1 } else { -1 };
            let dx = x2.wrapping_sub(x1);
            let dy = y2.wrapping_sub(y1).abs();
            let mut decision = dy.wrapping_mul(2).wrapping_sub(dx);
            let advance_both = dy.wrapping_sub(dx).wrapping_mul(2);
            let advance_major = dy.wrapping_mul(2);
            let mut y = y1;
            self.stamp(x1, y, radius, byte);
            let mut x = x1.wrapping_add(1);
            while x <= x2 {
                if decision >= 0 {
                    y = y.wrapping_add(y_increment);
                    decision = decision.wrapping_add(advance_both);
                } else {
                    decision = decision.wrapping_add(advance_major);
                }
                self.stamp(x, y, radius, byte);
                x = x.wrapping_add(1);
            }
        }
    }

    fn box_fill(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, byte: u8) {
        let (left, right) = (x1.min(x2), x1.max(x2));
        let (top, bottom) = (y1.min(y2), y1.max(y2));
        for y in top..=bottom {
            for x in left..=right {
                self.write(x, y, byte);
            }
        }
    }
}

impl crate::Engine {
    /// Persist a non-exact static C4 landscape using this engine's material
    /// palette and texture map.
    pub fn save_c4_static_landscape(
        &self,
        group: &mut clonk_resources::MutableGroup,
    ) -> Result<(), LandscapePersistenceError> {
        let landscape = self
            .landscape()
            .ok_or(LandscapePersistenceError::MissingLandscape)?;
        landscape.save_c4_static_scenario(self.materials(), group)
    }

    /// Persist `Map.bmp` only when C4's `fMapChanged && Map` gate is true.
    pub fn save_changed_c4_landscape_map(
        &self,
        group: &mut clonk_resources::MutableGroup,
    ) -> Result<bool, LandscapePersistenceError> {
        let landscape = self
            .landscape()
            .ok_or(LandscapePersistenceError::MissingLandscape)?;
        landscape.save_changed_c4_map(self.materials(), group)
    }

    /// Persist runtime texture-map additions into `Material.c4g/TexMap.txt`.
    pub fn save_c4_landscape_textures(
        &self,
        group: &mut clonk_resources::MutableGroup,
    ) -> Result<bool, LandscapePersistenceError> {
        let landscape = self
            .landscape()
            .ok_or(LandscapePersistenceError::MissingLandscape)?;
        landscape.save_c4_textures(group)
    }

    /// Persist C4's DiffLandscape.bmp plus its gated Map.bmp and TexMap.txt
    /// companions. `sync_save` selects a full index plane instead of 0xff
    /// masking unchanged pixels.
    pub fn save_c4_landscape_diff(
        &self,
        group: &mut clonk_resources::MutableGroup,
        sync_save: bool,
    ) -> Result<bool, LandscapePersistenceError> {
        let landscape = self
            .landscape_without_solid_masks()
            .ok_or(LandscapePersistenceError::MissingLandscape)?;
        landscape.save_c4_diff(self.materials(), group, sync_save)
    }

    /// Engine-level half of a raster change: temporarily expose the saved
    /// background under intersecting solid masks, run the landscape change,
    /// then record the changed background and put the masks back. This is the
    /// C4SolidMask RemoveTemporary/Repair ordering used by
    /// C4Landscape::PrepareChange/FinishChange (C4Landscape.cpp:2851-2880).
    pub(crate) fn landscape_raster_transaction<R>(
        &mut self,
        bounds: RasterChangeRect,
        change: impl FnOnce(&mut PixelGrid, &mut LandscapeRasterState) -> R,
    ) -> Option<R> {
        self.landscape_raster_transaction_with_draw_bounds(bounds, bounds, change)
    }

    /// Mask-only core of PrepareChange/FinishChange. Unlike the retained
    /// raster transaction, this also works for synthetic PixelGrid worlds
    /// without a LandscapeRasterState.
    pub(crate) fn landscape_solid_mask_transaction<R>(
        &mut self,
        prepare_bounds: RasterChangeRect,
        change: impl FnOnce(&mut Landscape) -> R,
    ) -> Option<R> {
        let (width, height) = self.landscape.as_ref()?.grid_dimensions()?;
        let mask_bounds = RasterChangeRect::new(
            prepare_bounds.x.saturating_sub(2),
            prepare_bounds.y.saturating_sub(16),
            prepare_bounds.width.saturating_add(4),
            prepare_bounds.height.saturating_add(32),
        )
        .clipped_to(width, height);
        let vehicle = self.landscape.as_ref()?.grid_vehicle_byte();
        let mask_indices = vehicle
            .map(|_| {
                self.objects
                    .iter()
                    .enumerate()
                    .filter_map(|(index, object)| {
                        let bake = object.solid_mask_bake.as_ref()?;
                        mask_bounds?
                            .intersection(bake.x, bake.y, bake.width, bake.height)
                            .map(|_| index)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Native RemoveTemporary walks Last -> Prev. The result here is
        // order-independent: for every pixel, only its owning bake stores a
        // non-MCVehic background byte; all overlapping bakes skip it.
        if let Some(vehicle) = vehicle {
            for &index in mask_indices.iter().rev() {
                let bake = self.objects[index].solid_mask_bake.clone()?;
                let overlap = mask_bounds?.intersection(bake.x, bake.y, bake.width, bake.height)?;
                let landscape = self.landscape.as_mut()?;
                for y in overlap.y..overlap.y + overlap.height {
                    for x in overlap.x..overlap.x + overlap.width {
                        let buffer_index = ((y - bake.y) * bake.width + (x - bake.x)) as usize;
                        let saved = bake.buffer[buffer_index];
                        if saved == vehicle {
                            continue;
                        }
                        debug_assert_eq!(landscape.grid_byte_at(x, y), Some(vehicle));
                        landscape.grid_write_byte(x, y, saved);
                    }
                }
            }
        }

        let result = change(self.landscape.as_mut()?);

        // Native Repair walks First -> Next. This fold is likewise
        // order-independent because the same one owning bake refreshes the
        // saved background before restoring MCVehic.
        if let Some(vehicle) = vehicle {
            for &index in &mask_indices {
                let (objects, landscape) = (&mut self.objects, &mut self.landscape);
                let bake = objects[index].solid_mask_bake.as_mut()?;
                let overlap = mask_bounds?.intersection(bake.x, bake.y, bake.width, bake.height)?;
                let landscape = landscape.as_mut()?;
                for y in overlap.y..overlap.y + overlap.height {
                    for x in overlap.x..overlap.x + overlap.width {
                        let buffer_index = ((y - bake.y) * bake.width + (x - bake.x)) as usize;
                        if bake.buffer[buffer_index] == vehicle {
                            continue;
                        }
                        bake.buffer[buffer_index] = landscape.grid_byte_at(x, y).unwrap_or(0);
                        landscape.grid_write_byte(x, y, vehicle);
                    }
                }
            }
        }
        Some(result)
    }

    /// Variant for C4Landscape::DrawChunks, whose actual inclusive
    /// Surface8 clip differs from its PrepareChange/FinishChange bounds.
    fn landscape_raster_transaction_with_draw_bounds<R>(
        &mut self,
        prepare_bounds: RasterChangeRect,
        changed_bounds: RasterChangeRect,
        change: impl FnOnce(&mut PixelGrid, &mut LandscapeRasterState) -> R,
    ) -> Option<R> {
        let (width, height) = self.landscape.as_ref()?.grid_dimensions()?;
        let changed_bounds = changed_bounds.clipped_to(width, height);
        self.landscape_solid_mask_transaction(prepare_bounds, |landscape| {
            landscape.raster_transaction_with_bounds(Some(prepare_bounds), changed_bounds, change)
        })
        .flatten()
    }

    pub(crate) fn set_editor_landscape_mode(&mut self, mode: i32) {
        let previous = self
            .landscape
            .as_ref()
            .map(Landscape::mode)
            .unwrap_or(LANDSCAPE_MODE_UNDEFINED);
        let changed = self
            .landscape
            .as_mut()
            .is_some_and(|landscape| landscape.set_mode(mode));
        if !changed || previous != LANDSCAPE_MODE_EXACT || mode != LANDSCAPE_MODE_STATIC {
            return;
        }
        let redraw = self.landscape.as_ref().and_then(|landscape| {
            let state = landscape.raster_state()?;
            Some((state.map()?, state.texmap().clone()))
        });
        if let Some((map, texmap)) = redraw {
            let _ = self.redraw_retained_map_segment(
                &map,
                0,
                0,
                map.width as i32,
                map.height as i32,
                texmap,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_editor_landscape(
        &mut self,
        action: u8,
        x: i32,
        y: i32,
        x2: i32,
        y2: i32,
        grade: i32,
        material: &str,
        texture: &str,
        ift: bool,
    ) -> bool {
        let mode = self
            .landscape
            .as_ref()
            .map(Landscape::mode)
            .unwrap_or(LANDSCAPE_MODE_UNDEFINED);
        if mode != LANDSCAPE_MODE_STATIC && mode != LANDSCAPE_MODE_EXACT {
            // The native primitive switches on mode before texture lookup;
            // Dynamic, Undefined, and corrupted unknown modes all fall
            // through as successful no-ops without allocating a texmap slot.
            return true;
        }
        let Some(byte) = self
            .landscape
            .as_mut()
            .and_then(|landscape| landscape.resolve_editor_color(material, texture, ift))
        else {
            return false;
        };

        if mode == LANDSCAPE_MODE_STATIC {
            let redraw = {
                let Some(landscape) = self.landscape.as_mut() else {
                    return false;
                };
                let Some(state) = landscape.raster_state_mut() else {
                    return false;
                };
                let zoom = state.map_zoom();
                if zoom <= 0 {
                    return false;
                }
                let Some((width, height, indices)) = state.map_mut() else {
                    return false;
                };
                let segment = {
                    let mut map = EditorMapSurface {
                        width: width as i32,
                        height: height as i32,
                        bytes: indices,
                    };
                    match action {
                        crate::control::EMDT_BRUSH => {
                            let radius = grade.wrapping_mul(2).wrapping_div(zoom).max(1);
                            let map_x = x / zoom;
                            let map_y = y / zoom;
                            map.stamp(map_x, map_y, radius, byte);
                            (
                                map_x.wrapping_sub(radius).wrapping_sub(1),
                                map_y.wrapping_sub(radius).wrapping_sub(1),
                                radius.wrapping_mul(2).wrapping_add(2),
                                radius.wrapping_mul(2).wrapping_add(2),
                            )
                        }
                        crate::control::EMDT_LINE => {
                            let radius = grade.wrapping_mul(2).wrapping_div(zoom).max(1);
                            let (map_x, map_y, map_x2, map_y2) =
                                (x / zoom, y / zoom, x2 / zoom, y2 / zoom);
                            map.line(map_x, map_y, map_x2, map_y2, radius, byte);
                            (
                                map_x.min(map_x2).wrapping_sub(radius).wrapping_sub(1),
                                map_y.min(map_y2).wrapping_sub(radius).wrapping_sub(1),
                                map_x
                                    .wrapping_sub(map_x2)
                                    .abs()
                                    .wrapping_add(radius.wrapping_mul(2))
                                    .wrapping_add(2),
                                map_y
                                    .wrapping_sub(map_y2)
                                    .abs()
                                    .wrapping_add(radius.wrapping_mul(2))
                                    .wrapping_add(2),
                            )
                        }
                        crate::control::EMDT_RECT => {
                            let (map_x, map_y, map_x2, map_y2) =
                                (x / zoom, y / zoom, x2 / zoom, y2 / zoom);
                            map.box_fill(map_x, map_y, map_x2, map_y2, byte);
                            let left = map_x.min(map_x2);
                            let top = map_y.min(map_y2);
                            (
                                left.wrapping_sub(1),
                                top.wrapping_sub(1),
                                map_x.wrapping_sub(map_x2).abs().wrapping_add(3),
                                map_y.wrapping_sub(map_y2).abs().wrapping_add(3),
                            )
                        }
                        _ => return true,
                    }
                };
                state.set_map_changed();
                state
                    .map()
                    .map(|map| (map, state.texmap().clone(), segment))
            };
            let Some((map, texmap, (map_x, map_y, map_width, map_height))) = redraw else {
                return false;
            };
            return self
                .redraw_retained_map_segment(&map, map_x, map_y, map_width, map_height, texmap);
        }

        if mode != LANDSCAPE_MODE_EXACT {
            return false;
        }
        match action {
            crate::control::EMDT_BRUSH => {
                let bounds = RasterChangeRect::new(
                    x.saturating_sub(grade).saturating_sub(1),
                    y.saturating_sub(grade).saturating_sub(1),
                    grade.saturating_mul(2).saturating_add(2),
                    grade.saturating_mul(2).saturating_add(2),
                );
                self.landscape_raster_transaction(bounds, |grid, _| {
                    grid.draw_editor_circle(x, y, grade, byte);
                })
                .is_some()
            }
            crate::control::EMDT_LINE => {
                let left = x.min(x2).saturating_sub(grade);
                let top = y.min(y2).saturating_sub(grade);
                let right = x.max(x2).saturating_add(grade);
                let bottom = y.max(y2).saturating_add(grade);
                let bounds = RasterChangeRect::new(
                    left,
                    top,
                    right.saturating_sub(left).saturating_add(1),
                    bottom.saturating_sub(top).saturating_add(1),
                );
                self.landscape_raster_transaction(bounds, |grid, _| {
                    grid.draw_editor_line(x, y, x2, y2, grade, byte);
                })
                .is_some()
            }
            crate::control::EMDT_RECT => {
                let left = x.min(x2);
                let top = y.min(y2);
                let right = x.max(x2);
                let bottom = y.max(y2);
                let bounds = RasterChangeRect::new(
                    left,
                    top,
                    right.saturating_sub(left).saturating_add(1),
                    bottom.saturating_sub(top).saturating_add(1),
                );
                self.landscape_raster_transaction(bounds, |grid, _| {
                    grid.draw_editor_box(x, y, x2, y2, byte);
                })
                .is_some()
            }
            _ => true,
        }
    }

    /// FnDrawMaterialQuad/C4Landscape::DrawQuad
    /// (C4Script.cpp:5111-5115; C4Landscape.cpp:2448-2468): resolve the live
    /// material-texture slot, compute the inclusive vertex bounds, and run
    /// the exact Surface8 polygon through PrepareChange/FinishChange's Rust
    /// transaction seam.
    pub(crate) fn draw_material_quad(
        &mut self,
        material_texture: &str,
        vertices: [Vector2; 4],
        ift: bool,
    ) -> bool {
        let slot = self
            .landscape
            .as_mut()
            .map(|landscape| landscape.resolve_runtime_material_texture(material_texture))
            .unwrap_or(0);
        if slot == 0 {
            return false;
        }

        let min_x = vertices.iter().map(|point| point.x).min().unwrap_or(0);
        let max_x = vertices.iter().map(|point| point.x).max().unwrap_or(0);
        let min_y = vertices.iter().map(|point| point.y).min().unwrap_or(0);
        let max_y = vertices.iter().map(|point| point.y).max().unwrap_or(0);
        let bounds = RasterChangeRect::new(
            min_x,
            min_y,
            max_x.saturating_sub(min_x).saturating_add(1),
            max_y.saturating_sub(min_y).saturating_add(1),
        );
        let polygon = vertices.map(|point| (point.x, point.y));
        let byte = slot | if ift { 0x80 } else { 0 };
        let _ = self.landscape_raster_transaction(bounds, |grid, _state| {
            grid.draw_polygon(&polygon, byte);
        });
        // Once GetIndexMatTex resolved, C++ returns true even when Surface8
        // clipping leaves the polygon wholly outside the landscape.
        true
    }

    /// FnDrawMatChunks/C4Landscape::DrawChunks
    /// (C4Script.cpp:4802-4805; C4Landscape.cpp:2419-2445). Texture-map
    /// resolution and every synced Random(1000) draw already happened in
    /// the host callback; this fold only applies the retained deterministic
    /// ChunkyRandom geometry to the authoritative Surface8 plane.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_material_chunks(
        &mut self,
        origin: Vector2,
        width: i32,
        height: i32,
        count_x: i32,
        count_y: i32,
        material: &str,
        byte: u8,
        map_seed: i32,
        random_offsets: &[i32],
        texmap: RuntimeTexMapState,
    ) -> bool {
        let Some(shape) = texmap.material(material).map(|material| material.shape) else {
            return false;
        };
        if !self.replace_runtime_texmap(texmap) {
            return false;
        }

        let (landscape_width, landscape_height) = self
            .landscape
            .as_ref()
            .and_then(Landscape::grid_dimensions)
            .unwrap_or_default();
        let (prepare_bounds, changed_bounds) =
            material_chunk_raster_bounds(origin, width, height, landscape_width, landscape_height);

        // Non-positive counts enter neither C++ loop, but resolution and a
        // possible texmap allocation still succeed without any RNG draw.
        // PrepareChange/FinishChange nevertheless bracket the empty batch.
        if count_x <= 0 || count_y <= 0 {
            let _ = self.landscape_raster_transaction(prepare_bounds, |_, _| {});
            return true;
        }
        let expected_offsets = (count_x as usize).saturating_mul(count_y as usize);
        if random_offsets.len() != expected_offsets {
            return false;
        }

        let Some(changed_bounds) = changed_bounds else {
            let _ = self.landscape_raster_transaction(prepare_bounds, |_, _| {});
            return true;
        };
        let _ = self.landscape_raster_transaction_with_draw_bounds(
            prepare_bounds,
            changed_bounds,
            |grid, _state| {
                grid.draw_material_chunks(
                    changed_bounds,
                    origin,
                    width,
                    height,
                    count_x,
                    count_y,
                    byte,
                    shape,
                    random_offsets,
                    map_seed,
                );
            },
        );
        // Once material/texture resolution succeeds, C++ returns true even
        // when clipping or degenerate geometry changes no pixel.
        true
    }

    /// C4Landscape::DrawMap -> MapToLandscape/MapToSurface
    /// (C4Landscape.cpp:2636-2668,337-510): expand the callback-rendered
    /// indexed map by MapZoom, clear the complete rounded destination to sky,
    /// draw material chunks with their IFT bits, and bracket the authoritative
    /// write with the shared PrepareChange/FinishChange transaction seam.
    pub(crate) fn draw_indexed_map(
        &mut self,
        origin: Vector2,
        bitmap: &clonk_resources::bitmap::IndexedBitmap,
        requested_map_width: i32,
        requested_map_height: i32,
        texmap: RuntimeTexMapState,
    ) -> bool {
        let Some(prepared) = self.landscape.as_ref().and_then(|landscape| {
            prepare_indexed_map_draw(
                landscape,
                origin,
                bitmap,
                requested_map_width,
                requested_map_height,
                texmap,
            )
        }) else {
            return false;
        };
        let PreparedIndexedMapDraw::Draw(draw) = prepared else {
            return true;
        };
        let bounds = draw.bounds;
        self.landscape_raster_transaction(bounds, move |grid, state| draw.apply(grid, state))
            .is_some()
    }

    /// C4Landscape::MapToLandscape for a segment of the retained editor map.
    /// Chunk synthesis uses the full map so two-cell rim/smoother reads see
    /// their real neighbors, while PrepareChange/SkyToLandscape and the
    /// actual copy remain clipped to the native affected rectangle.
    fn redraw_retained_map_segment(
        &mut self,
        bitmap: &clonk_resources::bitmap::IndexedBitmap,
        map_x: i32,
        map_y: i32,
        map_width: i32,
        map_height: i32,
        texmap: RuntimeTexMapState,
    ) -> bool {
        let Some((map_zoom, map_seed)) = self
            .landscape
            .as_ref()
            .and_then(Landscape::raster_state)
            .map(|state| (state.map_zoom(), state.map_seed()))
        else {
            return false;
        };
        let (Ok(full_width), Ok(full_height)) =
            (i32::try_from(bitmap.width), i32::try_from(bitmap.height))
        else {
            return false;
        };
        let Some(expected_len) = (full_width as usize).checked_mul(full_height as usize) else {
            return false;
        };
        if map_zoom <= 0
            || full_width <= 0
            || full_height <= 0
            || bitmap.indices.len() != expected_len
        {
            return false;
        }

        // BoundBy parameters from MapToLandscape: start coordinates clamp to
        // the final map cell, then size clamps against the remaining extent.
        let map_x = map_x.clamp(0, full_width - 1);
        let map_y = map_y.clamp(0, full_height - 1);
        let map_width = map_width.clamp(0, full_width - map_x);
        let map_height = map_height.clamp(0, full_height - map_y);
        if map_width == 0 || map_height == 0 {
            return true;
        }
        let (Some(target_x), Some(target_y), Some(target_width), Some(target_height)) = (
            map_x.checked_mul(map_zoom),
            map_y.checked_mul(map_zoom),
            map_width.checked_mul(map_zoom),
            map_height.checked_mul(map_zoom),
        ) else {
            return false;
        };
        let Some(synthesized_width) = full_width.checked_mul(map_zoom) else {
            return false;
        };
        let surface = crate::chunky::synthesize_landscape(
            &bitmap.indices,
            full_width,
            full_height,
            map_zoom,
            map_seed,
            &texmap.shapes,
        );
        let bytes = surface.into_bytes();
        let bounds = RasterChangeRect::new(target_x, target_y, target_width, target_height);
        self.landscape_raster_transaction(bounds, move |grid, state| {
            state.replace_texmap(texmap, false);
            for target_local_y in 0..target_height {
                for target_local_x in 0..target_width {
                    let source_x = target_x + target_local_x;
                    let source_y = target_y + target_local_y;
                    let index = (source_y * synthesized_width + source_x) as usize;
                    grid.write_byte(source_x, source_y, bytes[index]);
                }
            }
        })
        .is_some()
    }

    /// Apply parser-side C4TextureMap allocations when DrawMap has no
    /// renderable map. AddEntry immediately runs HandleTexMapUpdate in C++,
    /// so keep retained state and pixel lookup tables synchronized without a
    /// landscape pixel transaction (C4Texture.cpp:116-135,319-369).
    pub(crate) fn replace_runtime_texmap(&mut self, texmap: RuntimeTexMapState) -> bool {
        let Some(landscape) = self.landscape.as_mut() else {
            return false;
        };
        landscape.replace_runtime_texmap_state(texmap)
    }

    /// Fold SetTextureIndex's retained-map rewrite and MoveIndex copy without
    /// the HandleTexMapUpdate work that C++ deliberately omits.
    pub(crate) fn apply_runtime_texture_index_move(
        &mut self,
        texmap: RuntimeTexMapState,
        old_index: u8,
        new_index: u8,
    ) -> bool {
        let Some(landscape) = self.landscape.as_mut() else {
            return false;
        };
        landscape.apply_runtime_texture_index_move(texmap, old_index, new_index)
    }

    pub(crate) fn remove_unused_runtime_texmap_entries(&mut self) -> bool {
        let Some(landscape) = self.landscape.as_mut() else {
            return false;
        };
        landscape.remove_unused_runtime_texmap_entries()
    }

    /// Persist DrawDefMap's in-place C4MapCreatorS2 mutation after the
    /// callback-rendered bytes have been folded into the real landscape.
    pub(crate) fn replace_runtime_map_creator(
        &mut self,
        creator: crate::map_creator_s2::MapCreatorS2State,
    ) -> bool {
        let Some(landscape) = self.landscape.as_mut() else {
            return false;
        };
        landscape.replace_runtime_map_creator_state(creator)
    }

    pub(crate) fn clear_runtime_map_creator(&mut self) -> bool {
        let Some(state) = self
            .landscape
            .as_mut()
            .and_then(Landscape::raster_state_mut)
        else {
            return false;
        };
        state.set_map_creator(None);
        true
    }
}

impl LandscapeCommand {
    pub fn apply(&self, landscape: &mut Landscape) {
        match *self {
            LandscapeCommand::LowerRange { start, end, height } => {
                landscape.lower_range(start, end, height);
            }
            LandscapeCommand::SetLiquidColumn {
                column,
                ref segments,
            } => {
                if column >= 0 {
                    landscape.set_liquid_column(column as u32, segments.clone());
                }
            }
            LandscapeCommand::ClearLiquidColumn { column } => {
                if column >= 0 {
                    landscape.clear_liquid_column(column as u32);
                }
            }
        }
    }
}

impl LiquidColumn {
    pub fn from_segments(segments: Vec<LiquidSegment>) -> Self {
        let mut column = Self { segments };
        column.normalize();
        column
    }

    pub fn contains(&self, y: i32) -> bool {
        self.segments.iter().any(|segment| segment.contains(y))
    }

    pub fn clear(&mut self) {
        self.segments.clear();
    }

    pub fn segments(&self) -> &[LiquidSegment] {
        &self.segments
    }

    pub fn material_at(&self, y: i32, default: Option<MaterialId>) -> Option<MaterialId> {
        self.segments
            .iter()
            .find(|segment| segment.contains(y))
            .and_then(|segment| segment.material.or(default))
    }

    fn remove_pixel(&mut self, y: i32) -> Option<Option<MaterialId>> {
        let mut removed_material = None;
        let mut new_segments = Vec::with_capacity(self.segments.len());
        let mut encountered = false;
        for segment in &self.segments {
            if !segment.contains(y) {
                new_segments.push(*segment);
                continue;
            }

            if encountered {
                // Should not happen due to normalization but guard anyway.
                continue;
            }
            encountered = true;
            removed_material = Some(segment.material);
            if segment.top < y {
                new_segments.push(LiquidSegment {
                    top: segment.top,
                    bottom: y - 1,
                    material: segment.material,
                });
            }
            if y < segment.bottom {
                new_segments.push(LiquidSegment {
                    top: y + 1,
                    bottom: segment.bottom,
                    material: segment.material,
                });
            }
        }

        if !encountered {
            return None;
        }

        self.segments = new_segments;
        self.normalize();
        removed_material
    }

    fn insert_pixel(&mut self, y: i32, material: Option<MaterialId>) -> bool {
        if self.contains(y) {
            return false;
        }
        self.segments.push(LiquidSegment {
            top: y,
            bottom: y,
            material,
        });
        self.normalize();
        true
    }

    fn apply_temperature_conversions(
        &mut self,
        materials: &MaterialSet,
        default_material: Option<MaterialId>,
        temperature_at: &impl Fn(i32) -> i32,
    ) -> bool {
        if self.segments.is_empty() {
            return false;
        }
        let original_segments = self.segments.clone();
        let mut updated_segments: Vec<LiquidSegment> = Vec::with_capacity(original_segments.len());

        for segment in original_segments.iter() {
            let Some(material_id) = segment.material.or(default_material) else {
                updated_segments.push(*segment);
                continue;
            };

            let mut top_segments: Vec<LiquidSegment> = Vec::new();
            let mut bottom_segments: Vec<LiquidSegment> = Vec::new();
            let mut remaining_top = segment.top;
            let mut remaining_bottom = segment.bottom;

            if let Some(action) = find_temperature_conversion_action(
                materials,
                material_id,
                TemperatureDirection::Downwards,
                temperature_at(remaining_top),
            ) {
                let available = (remaining_bottom - remaining_top).saturating_add(1);
                let strength = action.strength().max(0);
                if available > 0 && strength > 0 {
                    let convert_height = available.min(strength);
                    if convert_height > 0 {
                        match action {
                            TemperatureConversionAction::ChangeMaterial { target, .. } => {
                                let bottom = remaining_top.saturating_add(convert_height - 1);
                                top_segments.push(LiquidSegment {
                                    top: remaining_top,
                                    bottom,
                                    material: segment_material_override(target, default_material),
                                });
                                remaining_top = remaining_top.saturating_add(convert_height);
                            }
                            TemperatureConversionAction::RemoveToSky { .. } => {
                                remaining_top = remaining_top.saturating_add(convert_height);
                            }
                        }
                    }
                }
            }

            if remaining_top > remaining_bottom {
                updated_segments.extend(top_segments);
                continue;
            }

            if let Some(action) = find_temperature_conversion_action(
                materials,
                material_id,
                TemperatureDirection::Upwards,
                temperature_at(remaining_bottom),
            ) {
                let available = (remaining_bottom - remaining_top).saturating_add(1);
                let strength = action.strength().max(0);
                if available > 0 && strength > 0 {
                    let convert_height = available.min(strength);
                    if convert_height > 0 {
                        match action {
                            TemperatureConversionAction::ChangeMaterial { target, .. } => {
                                let top = remaining_bottom.saturating_sub(convert_height - 1);
                                bottom_segments.push(LiquidSegment {
                                    top,
                                    bottom: remaining_bottom,
                                    material: segment_material_override(target, default_material),
                                });
                                remaining_bottom = remaining_bottom.saturating_sub(convert_height);
                            }
                            TemperatureConversionAction::RemoveToSky { .. } => {
                                remaining_bottom = remaining_bottom.saturating_sub(convert_height);
                            }
                        }
                    }
                }
            }

            updated_segments.extend(top_segments);
            if remaining_top <= remaining_bottom {
                updated_segments.push(LiquidSegment {
                    top: remaining_top,
                    bottom: remaining_bottom,
                    material: segment.material,
                });
            }
            updated_segments.extend(bottom_segments);
        }

        self.segments = updated_segments;
        self.normalize();
        self.segments != original_segments
    }

    fn normalize(&mut self) {
        if self.segments.is_empty() {
            return;
        }
        let mut segments = mem::take(&mut self.segments);
        for segment in &mut segments {
            if segment.bottom < segment.top {
                mem::swap(&mut segment.top, &mut segment.bottom);
            }
        }
        segments.retain(|segment| segment.top <= segment.bottom);
        segments.sort_by(|a, b| a.top.cmp(&b.top).then_with(|| a.bottom.cmp(&b.bottom)));
        let mut merged: Vec<LiquidSegment> = Vec::with_capacity(segments.len());
        for segment in segments {
            if let Some(last) = merged.last_mut() {
                if segment.top <= last.bottom + 1 && segment.material == last.material {
                    if segment.bottom > last.bottom {
                        last.bottom = segment.bottom;
                    }
                    continue;
                }
            }
            merged.push(segment);
        }
        self.segments = merged;
    }
}

impl LiquidSegment {
    pub fn new(top: i32, bottom: i32) -> Self {
        let (top, bottom) = if bottom < top {
            (bottom, top)
        } else {
            (top, bottom)
        };
        Self {
            top,
            bottom,
            material: None,
        }
    }

    pub fn with_material(top: i32, bottom: i32, material: Option<MaterialId>) -> Self {
        let mut segment = Self::new(top, bottom);
        segment.material = material;
        segment
    }

    pub fn contains(&self, y: i32) -> bool {
        self.top <= y && y <= self.bottom
    }
}

impl<'de> Deserialize<'de> for Landscape {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct LandscapeData {
            width: u32,
            surface: Vec<i32>,
            #[serde(default)]
            mode: i32,
            #[serde(default)]
            modulation: u32,
            #[serde(default = "default_shade_materials")]
            shade_materials: bool,
            #[serde(default)]
            liquids: Vec<LiquidColumn>,
            #[serde(default)]
            solid_materials: Vec<Option<MaterialId>>,
            #[serde(default)]
            default_solid_material: Option<MaterialId>,
            #[serde(default)]
            default_liquid_material: Option<MaterialId>,
            #[serde(default)]
            tunnels: HashMap<u32, Vec<(i32, i32)>>,
            #[serde(default)]
            world_height: Option<i32>,
            #[serde(default)]
            pixels: Option<PixelGrid>,
            #[serde(default)]
            no_scan: bool,
            #[serde(default)]
            left_open: i32,
            #[serde(default)]
            right_open: i32,
            #[serde(default = "default_top_open")]
            top_open: bool,
            #[serde(default)]
            bottom_open: bool,
            #[serde(default)]
            vehicle_material: Option<MaterialId>,
            #[serde(default)]
            raster_state: Option<LandscapeRasterState>,
        }

        let mut data = LandscapeData::deserialize(deserializer)?;
        let expected = data.width as usize;
        if data.surface.len() != expected {
            return Err(D::Error::custom(format!(
                "height map length {} does not match width {}",
                data.surface.len(),
                data.width
            )));
        }

        if data.liquids.len() < expected {
            data.liquids.resize(expected, LiquidColumn::default());
        } else if data.liquids.len() > expected {
            data.liquids.truncate(expected);
        }
        for column in &mut data.liquids {
            column.normalize();
        }

        if data.solid_materials.len() < expected {
            data.solid_materials
                .resize(expected, data.default_solid_material);
        } else if data.solid_materials.len() > expected {
            data.solid_materials.truncate(expected);
        }

        let mut landscape =
            Landscape::with_default_material(data.width, data.surface, data.default_solid_material)
                .map_err(|error| D::Error::custom(error.to_string()))?;
        landscape.modulation = data.modulation;
        landscape.shade_materials = data.shade_materials;
        landscape.mode = data.mode;
        landscape.liquids = data.liquids;
        landscape.solid_materials = data.solid_materials;
        landscape.default_liquid_material = data.default_liquid_material;
        landscape.tunnels = data.tunnels;
        landscape.world_height = data.world_height;
        landscape.pixels = data.pixels;
        if let Some(grid) = landscape.pixels.as_mut() {
            grid.rebuild_material_counts();
        }
        landscape.no_scan = data.no_scan;
        landscape.left_open = data.left_open;
        landscape.right_open = data.right_open;
        landscape.top_open = data.top_open;
        landscape.bottom_open = data.bottom_open;
        landscape.vehicle_material = data.vehicle_material;
        landscape.raster_state = data.raster_state;
        Ok(landscape)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_resources::MaterialLibrary;

    /// A 100-wide world with ground from y=50 down and an explicit world
    /// height (GBackHgt) of 400, mirroring a real zoomed map landscape.
    fn flat_world() -> Landscape {
        let mut landscape = Landscape::flat(100, 50);
        landscape.set_world_height(400);
        landscape
    }

    #[test]
    fn landscape_without_raster_state_remains_backward_compatible() {
        let landscape: Landscape =
            serde_json::from_str(r#"{"width":2,"surface":[3,4],"top_open":true}"#)
                .expect("pre-raster-state landscape deserializes");
        assert!(landscape.raster_state().is_none());

        let serialized = serde_json::to_value(&landscape).expect("landscape serializes");
        assert!(
            serialized.get("raster_state").is_none(),
            "absent state stays omitted from old/save fixture shapes"
        );
    }

    #[test]
    fn surface_palette_retains_source_and_stale_slots_until_add_entry_repaint() {
        let library = MaterialLibrary::parse(
            "[Material]\nName=Earth\nColor=10,20,30\n\
             [Material]\nName=Rock\nColor=40,50,60\n",
        )
        .expect("palette materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let mut texmap = RuntimeTexMapState::default();
        texmap.material_names[1] = Some("Earth".to_string());
        let mut raster = LandscapeRasterState::new(1, 7, texmap);
        let mut source = default_surface_palette();
        source[255] = [71, 72, 73];
        raster.set_surface_palette(source);

        raster.texmap_mut().clear_entries(&[1]);
        let palette = store_surface_palette(
            &raster.surface_palette(),
            &raster.surface_palette_materials,
            &materials,
        );
        assert_eq!(palette[1], [10, 20, 30]);
        assert_eq!(palette[129], [10, 20, 30]);
        assert_eq!(palette[255], [71, 72, 73]);

        let mut added = raster.texmap().clone();
        added.material_names[2] = Some("Rock".to_string());
        raster.replace_texmap(added, false);
        let palette = store_surface_palette(
            &raster.surface_palette(),
            &raster.surface_palette_materials,
            &materials,
        );
        assert_eq!(palette[1], [10, 20, 30], "cleared slot stays stale");
        assert_eq!(palette[2], [40, 50, 60], "AddEntry repaints mappings");
        assert_eq!(palette[130], [40, 50, 60]);
        assert_eq!(palette[255], [71, 72, 73]);

        let current = raster.texmap().clone();
        raster.replace_texmap(current, true);
        let palette = store_surface_palette(
            &raster.surface_palette(),
            &raster.surface_palette_materials,
            &materials,
        );
        assert_eq!(
            palette[1], source[1],
            "a new Surface8 drops stale eager-target palette slots"
        );
        assert_eq!(palette[2], [40, 50, 60]);

        let mut landscape = Landscape::flat(4, 4);
        landscape.set_raster_state(raster);
        assert_eq!(
            landscape.surface8_palette_entry(2),
            (source[2], 0, Some("Rock")),
            "the renderer sees the live Mat2Pal material mapping separately from its source palette"
        );
    }

    #[test]
    fn legacy_landscape_diff_round_trip_uses_set_pix_path() {
        let original = vec![
            0, 0, 0, //
            0, 1, 0, //
            1, 1, 1,
        ];
        let mut changed = raster_grid_landscape(3, 3, original.clone());
        changed.save_initial().expect("initial Surface8 captures");
        assert_eq!(
            changed.save_diff(false).expect("unchanged diff builds"),
            None,
            "a masked diff is omitted until Surface8 changes"
        );
        assert_eq!(
            changed
                .save_diff(true)
                .expect("unchanged full diff builds")
                .expect("a full diff is always emitted")
                .indices,
            original,
            "the full variant is not gated on a changed pixel"
        );

        changed.grid_set_byte(0, 0, 1);
        changed.grid_set_byte(1, 1, 3);
        changed.refresh_all_raster_columns();
        let current = changed
            .pixel_grid()
            .expect("changed Surface8")
            .bytes()
            .to_vec();
        let masked = changed
            .save_diff(false)
            .expect("masked diff builds")
            .expect("changed diff is emitted");
        assert_eq!(
            masked.indices,
            vec![
                1, 0xff, 0xff, //
                0xff, 3, 0xff, //
                0xff, 0xff, 0xff,
            ]
        );
        assert_eq!(
            changed
                .save_diff(true)
                .expect("full diff builds")
                .expect("full diff is always emitted")
                .indices,
            current
        );

        let encoded = masked.encode().expect("masked diff encodes as BMP");
        let decoded = clonk_resources::bitmap::IndexedBitmap::decode(&encoded)
            .expect("encoded masked diff decodes");
        assert_eq!(decoded, masked);

        changed.finish_surface32_draw();
        let serialized = serde_json::to_string(&changed).expect("landscape serializes");
        let restored: Landscape = serde_json::from_str(&serialized).expect("landscape restores");
        assert_eq!(
            restored, changed,
            "runtime-only pInitial does not break native state equality"
        );
        assert!(matches!(
            restored.save_diff(false),
            Err(LandscapeError::MissingInitialPixels)
        ));

        let mut reloaded = raster_grid_landscape(3, 3, original);
        reloaded.save_initial().expect("reload initial captures");
        assert!(reloaded.set_surface32_pixel(0, 0, 0x0011_2233));
        let revision = reloaded.pixel_grid().expect("reload Surface8").revision();
        reloaded.apply_diff(&decoded).expect("diff applies");

        assert_eq!(
            reloaded.pixel_grid().expect("reload Surface8").bytes(),
            current,
            "base plus masked diff reproduces the changed Surface8 byte-for-byte"
        );
        assert_eq!(
            reloaded.pixel_grid().expect("reload Surface8").revision(),
            revision + 2,
            "each differing byte passes through SetPix bookkeeping"
        );
        assert_eq!(
            reloaded.surface32_pixel_at(0, 0),
            None,
            "SetPix schedules the stale presentation pixel for relighting"
        );
        assert_eq!(reloaded.surface(), changed.surface());
        assert_eq!(reloaded.liquids(), changed.liquids());
        assert_eq!(
            reloaded.save_diff(false).expect("round-trip diff builds"),
            Some(masked)
        );
    }

    #[test]
    fn shade_materials_surface8_reads_use_getpix_border_rules() {
        let mut landscape = raster_grid_landscape(2, 3, vec![1; 6]);
        landscape.set_border_open(2, 1, true, false);

        assert_eq!(landscape.grid_byte_with_border(0, 2), Some(1));
        assert_eq!(landscape.grid_byte_with_border(-1, 1), Some(0));
        assert_eq!(landscape.grid_byte_with_border(-1, 2), Some(2));
        assert_eq!(landscape.grid_byte_with_border(2, 0), Some(0));
        assert_eq!(landscape.grid_byte_with_border(2, 1), Some(2));
        assert_eq!(landscape.grid_byte_with_border(0, -1), Some(0));
        assert_eq!(landscape.grid_byte_with_border(0, 3), Some(2));
        assert_eq!(
            landscape.grid_byte_with_border(-1, -1),
            Some(0),
            "the C++ branch order checks side openness before top openness"
        );
    }

    #[test]
    fn shade_materials_default_and_disabled_override_round_trip() {
        let mut landscape = Landscape::flat(1, 1);
        assert!(landscape.shade_materials());
        let default_json = serde_json::to_value(&landscape).expect("landscape serializes");
        assert!(default_json.get("shade_materials").is_none());
        let restored: Landscape =
            serde_json::from_value(default_json).expect("default landscape restores");
        assert!(restored.shade_materials());

        landscape.set_shade_materials(false);
        let disabled_json = serde_json::to_value(&landscape).expect("landscape serializes");
        assert_eq!(
            disabled_json.get("shade_materials"),
            Some(&serde_json::json!(false))
        );
        let restored: Landscape =
            serde_json::from_value(disabled_json).expect("disabled landscape restores");
        assert!(!restored.shade_materials());
    }

    #[test]
    fn runtime_texmap_and_raster_inputs_round_trip_with_landscape() {
        let mut texture_names = vec![None; 128];
        texture_names[7] = Some("Smooth".to_string());
        let mut material_names = vec![None; 128];
        material_names[7] = Some("Earth".to_string());
        let mut densities = vec![0; 128];
        densities[7] = 100;
        let mut shapes = vec![None; 128];
        shapes[7] = Some(crate::chunky::ChunkShape::Smooth);
        let mut texmap = RuntimeTexMapState {
            densities,
            material_names,
            texture_names: texture_names.clone(),
            match_texture_names: texture_names,
            shapes,
            materials: vec![RuntimeTexMapMaterial {
                name: "Earth".to_string(),
                density: 100,
                shape: crate::chunky::ChunkShape::Smooth,
            }],
            texture_inventory: vec!["smooth".to_string()],
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            ..Default::default()
        };
        texmap.set_default_material_entry("Earth", 7);

        let mut landscape = Landscape::flat(2, 4);
        let map = clonk_resources::bitmap::IndexedBitmap {
            width: 2,
            height: 2,
            indices: vec![0, 7, 7 | 0x80, 0],
        };
        let mut raster_state = LandscapeRasterState::new(10, 31337, texmap);
        raster_state.set_map(&map);
        landscape.set_raster_state(raster_state);
        let serialized = serde_json::to_string(&landscape).expect("landscape serializes");
        let restored: Landscape = serde_json::from_str(&serialized).expect("state restores");

        assert_eq!(restored, landscape);
        let state = restored.raster_state().expect("raster state survives");
        assert_eq!(state.map_zoom(), 10);
        assert_eq!(state.map_seed(), 31337);
        assert_eq!(state.map(), Some(map));
        assert_eq!(state.texmap().default_material_entry("earth"), Some(7));
        assert!(state.map_creator().is_none());
    }

    #[test]
    fn identical_pixel_write_does_not_bump_render_revision() {
        let mut grid = PixelGrid::new(2, 1, vec![7, 0], Vec::new(), Vec::new(), Vec::new());
        let shared = grid.clone();
        let revision = grid.revision();

        grid.write_byte(0, 0, 7);

        assert_eq!(
            grid.bytes().as_ptr(),
            shared.bytes().as_ptr(),
            "an identical write must not detach shared pixel storage"
        );
        assert_eq!(
            grid.revision(),
            revision,
            "an identical byte cannot invalidate the full terrain render cache"
        );
        grid.write_byte(0, 0, 8);
        assert_ne!(grid.bytes().as_ptr(), shared.bytes().as_ptr());
        assert_eq!(grid.revision(), revision + 1);
        assert_eq!(grid.byte_at(0, 0), Some(8));
        assert_eq!(shared.byte_at(0, 0), Some(7));
    }

    #[test]
    fn pixel_grid_clone_shares_alchemy_sized_plane_until_mutation() {
        // Alchemy's generated landscape is 1488x1536. HostWorldContext and
        // SimulationSnapshot both clone Landscape, so this byte plane must
        // stay shared until a real terrain write needs an independent copy.
        const WIDTH: u32 = 1_488;
        const HEIGHT: u32 = 1_536;
        let original = PixelGrid::new(
            WIDTH,
            HEIGHT,
            vec![7; WIDTH as usize * HEIGHT as usize],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let original_revision = original.revision();
        let mut clone = original.clone();

        assert_eq!(
            original.bytes().as_ptr(),
            clone.bytes().as_ptr(),
            "cloning an Alchemy-sized grid must not copy its 2.2 MiB byte plane"
        );
        assert_eq!(clone, original, "shared storage preserves value equality");

        clone.write_byte(0, 0, 8);

        assert_ne!(
            original.bytes().as_ptr(),
            clone.bytes().as_ptr(),
            "the first changed pixel detaches the clone"
        );
        assert_eq!(original.byte_at(0, 0), Some(7));
        assert_eq!(clone.byte_at(0, 0), Some(8));
        assert_eq!(original.revision(), original_revision);
        assert_eq!(clone.revision(), original_revision + 1);
    }

    #[test]
    fn simulation_snapshot_shares_landscape_plane_with_live_engine() {
        let grid = PixelGrid::new(4, 2, vec![1; 8], Vec::new(), Vec::new(), Vec::new());
        let mut landscape = Landscape::flat(4, 2);
        landscape.set_pixel_grid(grid);
        let mut engine = crate::Engine::new();
        engine.set_landscape(landscape);

        let snapshot = engine.snapshot();
        let live = engine
            .landscape()
            .and_then(Landscape::pixel_grid)
            .expect("live landscape has a pixel grid");
        let captured = snapshot
            .landscape
            .as_ref()
            .and_then(Landscape::pixel_grid)
            .expect("snapshot carries the pixel grid");

        assert_eq!(
            live.bytes().as_ptr(),
            captured.bytes().as_ptr(),
            "capturing a simulation snapshot must not copy the byte plane"
        );
        assert_eq!(captured, live);
    }

    /// `render_dirty_rects_since` ran on every presented frame and compared
    /// two ~128-entry `Vec<Option<String>>` element by element to decide
    /// whether the texmap had changed. The tables only move when
    /// `sync_runtime_texmap` installs a new texmap, so a content-derived
    /// identity answers the same question with one `u64` compare. This pins
    /// the new predicate against the compare it replaces, including the case
    /// the identity must NOT get wrong: unrelated grids that have never
    /// synced a texmap and would share any naive generation counter.
    #[test]
    fn texmap_identity_agrees_with_the_name_table_compare() {
        fn grid(materials: &[&str], textures: &[&str]) -> PixelGrid {
            PixelGrid::new(
                2,
                2,
                vec![0; 4],
                vec![0; materials.len()],
                materials
                    .iter()
                    .map(|name| Some((*name).to_owned()))
                    .collect(),
                textures
                    .iter()
                    .map(|name| Some((*name).to_owned()))
                    .collect(),
            )
        }
        let cases = [
            (grid(&["Earth"], &["earth"]), grid(&["Earth"], &["earth"])),
            (grid(&["Earth"], &["earth"]), grid(&["Water"], &["earth"])),
            (grid(&["Earth"], &["earth"]), grid(&["Earth"], &["rock"])),
            (
                grid(&["Earth"], &["earth"]),
                grid(&["Earth", "Water"], &["earth", "rock"]),
            ),
            (grid(&[], &[]), grid(&["Earth"], &["earth"])),
            (grid(&[], &[]), grid(&[], &[])),
        ];
        for (left, right) in &cases {
            let compared = left.material_names() == right.material_names()
                && left.texture_names() == right.texture_names();
            assert_eq!(
                left.texmap_identity() == right.texmap_identity(),
                compared,
                "texmap identity must answer exactly what the name-table compare answered"
            );
        }

        // A texmap sync that really changes the tables must move the identity,
        // and one that changes nothing must leave it alone.
        let mut synced = grid(&["Earth"], &["earth"]);
        let before = synced.texmap_identity();
        let mut texmap = RuntimeTexMapState::default();
        texmap.densities = synced.densities.clone();
        texmap.material_names = synced.material_names.clone();
        texmap.texture_names = synced.texture_names.clone();
        synced.sync_runtime_texmap(&texmap);
        assert_eq!(
            synced.texmap_identity(),
            before,
            "a no-op texmap sync must not invalidate the frontend's cache"
        );

        texmap.material_names = vec![Some("Water".to_owned())];
        synced.sync_runtime_texmap(&texmap);
        assert_ne!(
            synced.texmap_identity(),
            before,
            "a texmap sync that renames a slot must force the full rebuild"
        );
        assert!(
            grid(&["Earth"], &["earth"])
                .render_dirty_rects_since(&synced)
                .is_none(),
            "a changed texmap still rejects the incremental path"
        );

        // The identity is runtime-only, so a grid restored from a save carries
        // none. That must degrade to the old table compare, never to a wrong
        // fast path, and must not make an otherwise-identical grid unequal.
        let live = grid(&["Earth"], &["earth"]);
        let restored: PixelGrid =
            serde_json::from_str(&serde_json::to_string(&live).expect("grid serializes"))
                .expect("grid deserializes");
        assert_eq!(restored.texmap_identity(), 0);
        assert_eq!(
            restored, live,
            "a runtime-only identity must not make a save round-trip unequal"
        );
        assert!(
            live.render_dirty_rects_since(&restored).is_some(),
            "a restored grid with the same tables still takes the incremental path"
        );
        assert!(
            grid(&["Water"], &["earth"])
                .render_dirty_rects_since(&restored)
                .is_none(),
            "a restored grid with different tables still forces the rebuild"
        );
    }

    /// The frontend calls `render_dirty_rects_since` once per presented frame
    /// on a fully populated texmap. Times the identity check against the
    /// element-by-element name compare it replaced, on the shipped table size.
    #[test]
    fn texmap_identity_costs_less_than_the_name_table_compare() {
        const SLOTS: usize = C4M_MAX_TEX_INDEX + 1;
        const ROUNDS: u32 = 10_000;

        let names = |prefix: &str| {
            (0..SLOTS)
                .map(|slot| Some(format!("{prefix}Material{slot:03}")))
                .collect::<Vec<_>>()
        };
        let materials = names("Mat");
        let textures = names("Tex");
        let grid = PixelGrid::new(
            2,
            2,
            vec![0; 4],
            vec![0; SLOTS],
            materials.clone(),
            textures.clone(),
        );
        let previous = grid.clone();

        // Worst case for the compare it replaces: equal tables, so every one
        // of the 2 x 128 entries is examined before it can answer "same".
        let started = std::time::Instant::now();
        let mut compared = 0_u32;
        for _ in 0..ROUNDS {
            compared += u32::from(
                grid.material_names == previous.material_names
                    && grid.texture_names == previous.texture_names,
            );
        }
        let compare = started.elapsed();

        let started = std::time::Instant::now();
        let mut matched = 0_u32;
        for _ in 0..ROUNDS {
            matched += u32::from(grid.texmap_tables_match(&previous));
        }
        let identity = started.elapsed();

        assert_eq!(compared, ROUNDS);
        assert_eq!(matched, ROUNDS);
        println!(
            "{SLOTS}-slot texmap, {ROUNDS} checks: name compare {:?} ({:.3} us/frame), \
             identity {:?} ({:.3} us/frame)",
            compare,
            compare.as_secs_f64() * 1e6 / f64::from(ROUNDS),
            identity,
            identity.as_secs_f64() * 1e6 / f64::from(ROUNDS),
        );
        assert!(
            identity < compare,
            "the identity check must beat the compare it replaced: {identity:?} vs {compare:?}"
        );
    }

    #[test]
    fn render_dirty_rects_follow_snapshot_cow_ancestry_and_reject_siblings() {
        let mut live = PixelGrid::new(8, 8, vec![1; 64], Vec::new(), Vec::new(), Vec::new());
        let first_snapshot = live.clone();

        live.write_byte(2, 3, 7);
        live.write_byte(4, 5, 8);
        assert_eq!(
            live.render_dirty_rects_since(&first_snapshot),
            Some(vec![PixelGridDirtyRect {
                x: 2,
                y: 3,
                width: 3,
                height: 3,
            }]),
            "writes after one shared snapshot coalesce into one bounded generation"
        );

        let second_snapshot = live.clone();
        live.write_byte(6, 1, 9);
        assert_eq!(
            live.render_dirty_rects_since(&second_snapshot),
            Some(vec![PixelGridDirtyRect {
                x: 6,
                y: 1,
                width: 1,
                height: 1,
            }]),
            "the next COW snapshot begins a fresh dirty generation"
        );
        assert_eq!(
            live.render_dirty_rects_since(&first_snapshot),
            Some(vec![PixelGridDirtyRect {
                x: 2,
                y: 1,
                width: 5,
                height: 5,
            }]),
            "a skipped snapshot merges nearby generations like one C++ pending relight list"
        );

        let mut sibling = second_snapshot.clone();
        sibling.write_byte(0, 0, 4);
        assert_eq!(
            live.revision(),
            sibling.revision(),
            "sibling clones can reach the same numeric revision"
        );
        assert!(
            live.render_dirty_rects_since(&sibling).is_none(),
            "same-revision sibling content is not a valid cache ancestor"
        );

        sibling.render_token = live.render_token;
        assert!(
            live.render_dirty_rects_since(&sibling).is_none(),
            "equal legacy/default tokens still compare content before reusing a cache"
        );
    }

    #[test]
    fn distant_set_pix_writes_keep_separate_render_dirty_rectangles() {
        // C4Landscape::SetPix retains up to C4LS_MaxRelights spatially
        // separate rectangles and merges only nearby changes
        // (C4Landscape.cpp:741-763). Far Worlds maps are several million
        // pixels, so joining distant edits would relight the strip between
        // them even though C++ never touches it.
        let mut live = PixelGrid::new(
            512,
            64,
            vec![1; 512 * 64],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let snapshot = live.clone();

        live.set_byte(2, 32, 7);
        live.set_byte(509, 32, 8);

        assert_eq!(
            live.render_dirty_rects_since(&snapshot),
            Some(vec![
                PixelGridDirtyRect::single(2, 32),
                PixelGridDirtyRect::single(509, 32),
            ])
        );
    }

    #[test]
    fn set_pix_dirty_rects_use_cpp_lighting_overlap_distances() {
        let mut live = PixelGrid::new(20, 40, vec![1; 20 * 40], Vec::new(), Vec::new(), Vec::new());
        let snapshot = live.clone();

        for (x, y) in [(2, 2), (4, 2), (7, 2), (15, 2), (15, 18), (15, 35)] {
            live.set_byte(x, y, 2);
        }

        assert_eq!(
            live.render_dirty_rects_since(&snapshot),
            Some(vec![
                PixelGridDirtyRect {
                    x: 2,
                    y: 2,
                    width: 3,
                    height: 1,
                },
                PixelGridDirtyRect::single(7, 2),
                PixelGridDirtyRect {
                    x: 15,
                    y: 2,
                    width: 1,
                    height: 17,
                },
                PixelGridDirtyRect::single(15, 35),
            ]),
            "dx=2 and dy=16 merge, while dx=3 and dy=17 stay separate"
        );
    }

    #[test]
    fn skipped_render_snapshots_keep_cpp_relight_thresholds_and_global_cap() {
        let mut live = PixelGrid::new(
            180,
            40,
            vec![1; 180 * 40],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let rendered = live.clone();

        for index in 0..30 {
            live.set_byte(index * 3, 20, 2);
        }
        let _skipped_snapshot = live.clone();
        for index in 30..60 {
            live.set_byte(index * 3, 20, 2);
        }

        let rects = live
            .render_dirty_rects_since(&rendered)
            .expect("both COW generations descend from the rendered grid");
        assert_eq!(rects.len(), MAX_RENDER_DIRTY_RECTS);
        assert_eq!(rects[48], PixelGridDirtyRect::single(144, 20));
        assert_eq!(
            rects[49],
            PixelGridDirtyRect {
                x: 147,
                y: 20,
                width: 31,
                height: 1,
            },
            "C++ unions the 51st and later distant changes into its final relight slot"
        );
    }

    #[test]
    fn sparse_render_dirty_generation_keeps_legacy_bounding_rect() {
        let mut live = PixelGrid::new(
            512,
            64,
            vec![1; 512 * 64],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let snapshot = live.clone();
        live.set_byte(2, 32, 7);
        live.set_byte(509, 32, 8);

        let generation = live
            .dirty_generations
            .back()
            .expect("two writes create one COW generation");
        let encoded = serde_json::to_value(generation).expect("generation serializes");
        assert_eq!(
            encoded["rect"],
            serde_json::json!({"x": 2, "y": 32, "width": 508, "height": 1}),
            "old readers safely over-render the bounding union"
        );
        assert!(
            encoded.get("rects").is_none(),
            "runtime cache precision must not change serialized replay state"
        );
        assert_eq!(
            generation.rects().collect::<Vec<_>>(),
            live.render_dirty_rects_since(&snapshot).unwrap()
        );

        let legacy: PixelGridDirtyGeneration = serde_json::from_value(serde_json::json!({
            "base_revision": 0,
            "revision": 2,
            "base_token": 11,
            "token": 22,
            "rect": {"x": 2, "y": 32, "width": 508, "height": 1}
        }))
        .expect("legacy one-rect generation still decodes");
        assert_eq!(
            legacy.rects().collect::<Vec<_>>(),
            vec![PixelGridDirtyRect {
                x: 2,
                y: 32,
                width: 508,
                height: 1,
            }]
        );
    }

    #[test]
    fn surface32_writes_have_independent_lineage_and_relight_back_to_surface8() {
        let mut grid = PixelGrid::new(4, 3, vec![0; 12], Vec::new(), Vec::new(), Vec::new());
        let material_revision = grid.revision();
        let before = grid.clone();

        assert!(grid.set_surface32_pixel(2, 1, 0x0011_2233));
        assert_eq!(grid.surface32_pixel_at(2, 1), Some(0x0011_2233));
        assert_eq!(grid.bytes(), before.bytes());
        assert_eq!(
            grid.revision(),
            material_revision,
            "a cosmetic Surface32 write cannot mutate Surface8's revision"
        );
        assert_eq!(
            grid.render_dirty_rects_since(&before),
            Some(vec![PixelGridDirtyRect::single(2, 1)]),
            "the frontend can patch only the changed Surface32 cache cell"
        );

        let opaque = grid.clone();
        assert!(grid.set_surface32_pixel(3, 2, 0xffab_cdef));
        assert_eq!(
            grid.surface32_pixel_at(3, 2),
            Some(0xff00_0000),
            "C4Surface canonicalizes fully transparent writes to transparent black"
        );
        assert_eq!(
            grid.render_dirty_rects_since(&opaque),
            Some(vec![PixelGridDirtyRect::single(3, 2)])
        );
        let restored: PixelGrid = serde_json::from_str(
            &serde_json::to_string(&grid).expect("Surface32 pixels serialize"),
        )
        .expect("Surface32 pixels restore");
        assert_eq!(restored.surface32_pixel_at(2, 1), Some(0x0011_2233));
        assert_eq!(restored.surface32_pixel_at(3, 2), Some(0xff00_0000));

        // SetPix schedules a relight expanded by x=1/y=8. Changing this
        // neighboring Surface8 byte therefore rebuilds both cosmetic cells
        // from the material plane without changing their Surface8 bytes.
        grid.set_byte(2, 0, 1);
        assert_eq!(grid.surface32_pixel_at(2, 1), None);
        assert_eq!(grid.surface32_pixel_at(3, 2), None);
        assert_eq!(grid.byte_at(2, 1), Some(0));
        assert_eq!(grid.byte_at(3, 2), Some(0));
        assert!(grid.set_surface32_pixel(2, 1, 0x0044_5566));
        assert_eq!(
            grid.surface32_pixel_at(2, 1),
            None,
            "Draw::DoRelights overwrites even a direct write after SetPix"
        );
        grid.finish_pending_surface32_relights();
        assert!(grid.set_surface32_pixel(2, 1, 0x0044_5566));
        assert_eq!(grid.surface32_pixel_at(2, 1), Some(0x0044_5566));
    }

    #[test]
    fn insert_material_pix_uses_default_mattex_instead_of_first_material_slot() {
        // InsertMaterial writes Mat2PixColDefault(mat), not the first texmap
        // entry carrying that material (C4Landscape.cpp:1214-1219).
        let rock = MaterialId::new(0).expect("first material id exists");
        let mut densities = vec![0; 128];
        densities[3] = 100;
        densities[7] = 100;
        let mut material_names = vec![None; 128];
        material_names[3] = Some("Rock".to_string());
        material_names[7] = Some("Rock".to_string());
        let mut texture_names = vec![None; 128];
        texture_names[3] = Some("Rough".to_string());
        texture_names[7] = Some("Smooth".to_string());
        let grid = PixelGrid::new(
            1,
            1,
            vec![0x80],
            densities.clone(),
            material_names.clone(),
            texture_names.clone(),
        );
        let mut texmap = RuntimeTexMapState {
            densities,
            material_names,
            texture_names: texture_names.clone(),
            match_texture_names: texture_names,
            shapes: vec![None; 128],
            materials: vec![RuntimeTexMapMaterial {
                name: "Rock".to_string(),
                density: 100,
                shape: crate::chunky::ChunkShape::Flat,
            }],
            texture_inventory: vec!["Rough".to_string(), "Smooth".to_string()],
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            ..Default::default()
        };
        texmap.set_default_material_entry("Rock", 7);

        let mut landscape = Landscape::flat(1, 1);
        landscape.set_pixel_grid(grid);
        landscape.resolve_grid_materials(|name| name.eq_ignore_ascii_case("Rock").then_some(rock));
        landscape.set_raster_state(LandscapeRasterState::new(1, 0, texmap));

        assert!(landscape.insert_material_pix(0, 0, rock));
        assert_eq!(
            landscape.grid_byte_at(0, 0),
            Some(7 | 0x80),
            "DefaultMatTex byte 7 and the current IFT bit must survive"
        );
    }

    #[test]
    fn runtime_texmap_only_fold_updates_pixel_lookup_tables() {
        // DrawMap's ReadScript can call GetIndexMatTex/AddEntry before a null
        // Render result. AddEntry invokes HandleTexMapUpdate immediately, so
        // the retained map and Pix2* tables change without any landscape
        // pixels changing (C4Texture.cpp:116-135,319-369).
        let landscape = raster_grid_landscape(2, 2, vec![0; 4]);
        let revision = landscape.pixel_grid().expect("pixel grid").revision();
        let mut texmap = landscape
            .raster_state()
            .expect("raster state")
            .texmap()
            .clone();
        texmap.texture_inventory.push("Ridge".to_string());
        let slot = texmap.get_index("Earth", Some("Ridge"), true);
        assert_eq!(slot, 4);

        let mut engine = crate::Engine::with_seed(3);
        engine.set_landscape(landscape);
        assert!(engine.replace_runtime_texmap(texmap));

        let landscape = engine.landscape().expect("landscape");
        assert_eq!(
            landscape
                .raster_state()
                .expect("raster state")
                .texmap()
                .match_texture_names[slot as usize]
                .as_deref(),
            Some("Ridge")
        );
        let grid = landscape.pixel_grid().expect("pixel grid");
        assert_eq!(
            grid.material_names()[slot as usize].as_deref(),
            Some("Earth")
        );
        assert_eq!(
            grid.texture_names()[slot as usize].as_deref(),
            Some("Ridge")
        );
        assert_eq!(grid.revision(), revision, "texmap sync writes no pixels");
    }

    #[test]
    fn runtime_texmap_null_texture_matches_existing_but_never_allocates() {
        // GetIndex checks existing material entries before its add path, but
        // AddEntry rejects a null texture (src/C4Texture.cpp:119-121,
        // 319-340). A wildcard lookup may therefore match but never create.
        let base = raster_grid_landscape(1, 1, vec![0])
            .raster_state()
            .expect("raster state")
            .texmap()
            .clone();
        let without_earth = || {
            let mut texmap = base.clone();
            texmap.material_names[1] = None;
            texmap.texture_names[1] = None;
            texmap.match_texture_names[1] = None;
            texmap.shapes[1] = None;
            texmap.densities[1] = 0;
            texmap
        };

        let mut add_miss = without_earth();
        let before_add_miss = add_miss.clone();
        assert_eq!(add_miss.get_index("Earth", None, true), 0);
        assert_eq!(add_miss, before_add_miss, "failed add changes no slots");

        let mut existing = without_earth();
        existing.material_names[12] = Some("Earth".to_string());
        existing.texture_names[12] = Some("Rough".to_string());
        existing.match_texture_names[12] = Some("Rough".to_string());
        existing.shapes[12] = Some(crate::chunky::ChunkShape::Flat);
        existing.densities[12] = 100;
        let before_existing = existing.clone();
        assert_eq!(existing.get_index("Earth", None, true), 12);
        assert_eq!(existing.get_index("Earth", None, false), 12);
        assert_eq!(existing, before_existing, "existing lookup is read-only");

        let mut no_add = without_earth();
        let before_no_add = no_add.clone();
        assert_eq!(no_add.get_index("Earth", None, false), 0);
        assert_eq!(no_add, before_no_add, "add=false remains read-only");
    }

    #[test]
    fn runtime_texmap_liquid_smooth_validates_liquid_but_stores_raw_name() {
        let make_texmap = || {
            let landscape = raster_grid_landscape(1, 1, vec![0]);
            landscape
                .raster_state()
                .expect("raster state")
                .texmap()
                .clone()
        };

        let mut liquid_only = make_texmap();
        liquid_only.texture_inventory = vec!["Liquid".to_string()];
        let slot = liquid_only.get_index("Water", Some("Smooth"), true);
        assert_eq!(slot, 4, "liquid Smooth validates the Liquid texture");
        assert_eq!(
            liquid_only.match_texture_names[slot as usize].as_deref(),
            Some("Smooth")
        );
        assert_eq!(
            liquid_only.texture_names[slot as usize].as_deref(),
            Some("Smooth"),
            "the entry retains the raw name used by GetIndex matching"
        );
        assert_eq!(liquid_only.get_index("water", Some("smooth"), false), slot);
        liquid_only.texture_inventory.clear();
        assert_eq!(
            liquid_only.get_index("WATER", Some("SMOOTH"), true),
            slot,
            "an existing raw-name pair bypasses add-path validation"
        );

        let mut smooth_only = make_texmap();
        smooth_only.texture_inventory = vec!["Smooth".to_string()];
        assert_eq!(smooth_only.get_index("Water", Some("Smooth"), true), 0);
        assert!(smooth_only.material_names[4].is_none());

        let mut non_liquid = make_texmap();
        non_liquid.texture_inventory = vec!["Smooth".to_string()];
        assert_eq!(non_liquid.get_index("Earth", Some("Smooth"), true), 4);
    }

    #[test]
    fn runtime_texmap_copies_native_names_through_the_first_nul() {
        let landscape = raster_grid_landscape(1, 1, vec![0]);
        let mut texmap = landscape
            .raster_state()
            .expect("raster state")
            .texmap()
            .clone();
        texmap.texture_inventory = vec!["Ridge".to_string()];
        let material = clonk_script::c4_string_from_bytes(b"Earth\0ignored\x80");
        let texture = clonk_script::c4_string_from_bytes(b"Ridge\0ignored\x81");

        let slot = texmap.get_index(&material, Some(&texture), true);
        assert_eq!(slot, 4);
        assert_eq!(texmap.material_names[4].as_deref(), Some("Earth"));
        assert_eq!(texmap.match_texture_names[4].as_deref(), Some("Ridge"));
        assert_eq!(texmap.texture_names[4].as_deref(), Some("Ridge"));

        let serialized = texmap.serialize_added_entries();
        assert!(serialized
            .windows(b"4=Earth-Ridge\r\n".len())
            .any(|line| { line == b"4=Earth-Ridge\r\n" }));
        assert!(!serialized
            .windows(b"ignored".len())
            .any(|part| part == b"ignored"));

        let material_texture = clonk_script::c4_string_from_bytes(b"Earth\0-Missing");
        assert_eq!(
            texmap.get_index_mat_tex(&material_texture, Some(&texture)),
            slot,
            "pair splitting and the default texture see only the native C string"
        );
    }

    fn raster_grid_landscape(width: u32, height: u32, bytes: Vec<u8>) -> Landscape {
        let mut densities = vec![0; 128];
        densities[1] = 100;
        densities[2] = 100;
        densities[3] = 25;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        material_names[2] = Some("Vehicle".to_string());
        material_names[3] = Some("Water".to_string());
        let texture_names = vec![None; 128];
        let grid = PixelGrid::new(
            width,
            height,
            bytes,
            densities.clone(),
            material_names.clone(),
            texture_names.clone(),
        );
        let mut texmap = RuntimeTexMapState {
            densities,
            material_names,
            texture_names: texture_names.clone(),
            match_texture_names: texture_names,
            shapes: vec![None; 128],
            materials: vec![
                RuntimeTexMapMaterial {
                    name: "Earth".to_string(),
                    density: 100,
                    shape: crate::chunky::ChunkShape::Flat,
                },
                RuntimeTexMapMaterial {
                    name: "Vehicle".to_string(),
                    density: 100,
                    shape: crate::chunky::ChunkShape::Flat,
                },
                RuntimeTexMapMaterial {
                    name: "Water".to_string(),
                    density: 25,
                    shape: crate::chunky::ChunkShape::Flat,
                },
            ],
            texture_inventory: Vec::new(),
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            ..Default::default()
        };
        texmap.set_default_material_entry("Earth", 1);
        texmap.set_default_material_entry("Vehicle", 2);
        texmap.set_default_material_entry("Water", 3);
        let mut landscape =
            Landscape::new(width, vec![height as i32; width as usize]).expect("landscape builds");
        landscape.set_world_height(height as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_raster_state(LandscapeRasterState::new(1, 0, texmap));
        landscape.refresh_all_raster_columns();
        landscape
    }

    fn overlapping_solid_mask_engine() -> (crate::Engine, [crate::ObjectId; 3]) {
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
func RotateMaskCycle()
{
    SetR(1);
    SetR(0);
    return true;
}

func ContainerMaskCycle(object container)
{
    if (!Enter(container)) return 1;
    if (!Exit()) return 2;
    return 3;
}


func MoveMask(int x, int y)
{
    return SetPosition(x, y);
}
"#,
        )
        .expect("definition compiles");
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(8);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let mut ids = [crate::ObjectId::new(0); 3];
        for id in &mut ids {
            *id = engine
                .spawn_object(crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1)))
                .expect("overlapping mask spawns");
        }
        (engine, ids)
    }

    #[test]
    fn raster_refresh_preserves_adjacent_liquid_materials() {
        // UpdatePixMaps keeps a separate Pix2Mat entry for every texture-map
        // byte (C4Landscape.cpp:2819-2826). Derived liquid runs must retain
        // that identity: reaction/extraction code may distinguish adjacent
        // liquids even though both satisfy the same density classification.
        let water = MaterialId::new(3).expect("water id");
        let acid = MaterialId::new(4).expect("acid id");
        let mut grid = PixelGrid::new(
            1,
            4,
            vec![0, 3, 4, 1],
            vec![0, 100, 100, 25, 25],
            vec![
                None,
                Some("Earth".to_string()),
                Some("Vehicle".to_string()),
                Some("Water".to_string()),
                Some("Acid".to_string()),
            ],
            vec![None; 5],
        );
        grid.resolve_materials(|name| match name {
            "Water" => Some(water),
            "Acid" => Some(acid),
            _ => None,
        });
        let mut landscape = Landscape::new(1, vec![4]).expect("landscape builds");
        landscape.set_world_height(4);
        landscape.set_pixel_grid(grid);

        landscape.refresh_all_raster_columns();

        assert_eq!(
            landscape.liquids()[0].segments(),
            &[
                LiquidSegment::with_material(1, 1, Some(water)),
                LiquidSegment::with_material(2, 2, Some(acid)),
            ]
        );
    }

    #[test]
    fn raster_transaction_refreshes_derived_columns_without_rewriting_pixels() {
        // DrawQuad encloses its Surface8 polygon write in PrepareChange /
        // FinishChange (C4Landscape.cpp:2448-2467). FinishChange refreshes
        // derived bookkeeping but never rewrites Surface8 (:2864-2880), so
        // Rust's column approximation must follow the changed pixels while
        // the pixel bytes and revision remain exactly those written.
        let mut landscape = raster_grid_landscape(
            3,
            5,
            vec![
                0,
                0,
                0, // y=0
                1,
                0,
                1, // y=1
                1,
                3,
                1, // y=2
                1,
                3 | 0x80,
                1, // y=3
                1,
                1,
                1, // y=4
            ],
        );
        assert_eq!(landscape.surface(), &[1, 4, 1]);
        assert_eq!(
            landscape.liquids()[1].segments(),
            &[LiquidSegment::new(2, 3)]
        );
        assert!(landscape.is_tunnel_at(1, 3));
        let revision = landscape.pixel_grid().expect("grid").revision();

        landscape
            .raster_transaction(RasterChangeRect::new(1, 0, 1, 5), |grid, _state| {
                grid.write_byte(1, 0, 1);
                grid.write_byte(1, 2, 0);
                grid.write_byte(1, 3, 3);
                grid.write_byte(1, 4, 0);
            })
            .expect("raster transaction runs");

        assert_eq!(landscape.surface(), &[1, 0, 1]);
        assert_eq!(
            landscape.liquids()[1].segments(),
            &[LiquidSegment::new(3, 3)]
        );
        assert!(!landscape.is_tunnel_at(1, 3));
        let grid = landscape.pixel_grid().expect("grid");
        assert_eq!(grid.byte_at(1, 0), Some(1));
        assert_eq!(grid.byte_at(1, 2), Some(0));
        assert_eq!(grid.byte_at(1, 3), Some(3));
        assert_eq!(grid.byte_at(1, 4), Some(0));
        assert_eq!(grid.revision(), revision + 4);
    }

    #[test]
    fn draw_material_quad_uses_cpp_polygon_and_inclusive_change_bounds() {
        // FnDrawMaterialQuad forwards the four GLOBAL vertices and IFT flag
        // verbatim (C4Script.cpp:5111-5115). C4Landscape::DrawQuad builds
        // its change box from four 1x1 vertex rects, then calls Surface8's
        // polygon with MatTex2PixCol+IFT (C4Landscape.cpp:2448-2468).
        // The Allegro fill includes the right edge and excludes the bottom
        // scanline (StdSurface8.cpp:306-404), so this quad paints x=1..4,
        // y=1..3; x=4 also proves the derived-column refresh used the
        // INCLUSIVE vertex bounding box.
        let mut engine = crate::Engine::with_seed(9);
        engine.set_landscape(raster_grid_landscape(6, 6, vec![0; 36]));

        assert!(engine.draw_material_quad(
            "Water",
            [
                Vector2::new(1, 1),
                Vector2::new(4, 1),
                Vector2::new(4, 4),
                Vector2::new(1, 4),
            ],
            true,
        ));

        let landscape = engine.landscape().expect("landscape remains");
        for y in 0..6 {
            for x in 0..6 {
                let expected = if (1..=4).contains(&x) && (1..=3).contains(&y) {
                    3 | 0x80
                } else {
                    0
                };
                assert_eq!(landscape.grid_byte_at(x, y), Some(expected), "({x},{y})");
            }
        }
        for x in 1..=4 {
            assert_eq!(
                landscape.liquids()[x as usize].segments(),
                &[LiquidSegment::new(1, 3)],
                "liquid column {x} follows Surface8"
            );
            assert!(landscape.is_tunnel_at(x, 1));
            assert!(landscape.is_tunnel_at(x, 3));
            assert!(!landscape.is_tunnel_at(x, 4));
        }
        assert_eq!(landscape.surface(), &[6; 6]);
    }

    #[test]
    fn draw_material_quad_updates_counts_without_full_landscape_rescan() {
        // PrepareChange/FinishChange call UpdateMatCnt only for the quad's
        // bounding rectangle (C4Landscape.cpp:2448-2468,2851-2967). A tiny
        // runtime quad must therefore not scan the full Surface8 merely to
        // maintain MatCount.
        let mut engine = crate::Engine::with_seed(9);
        let materials = MaterialSet::from_resource_library(
            &MaterialLibrary::parse(
                "[Material Earth]\nName=Earth\nDensity=100\n\n\
                 [Material Vehicle]\nName=Vehicle\nDensity=100\n\n\
                 [Material Water]\nName=Water\nDensity=25\n",
            )
            .expect("materials parse"),
        );
        engine.set_materials(materials);
        engine.set_landscape(raster_grid_landscape(64, 48, vec![0; 64 * 48]));
        MATERIAL_COUNT_FULL_REBUILDS.with(|rebuilds| rebuilds.set(0));

        assert!(engine.draw_material_quad(
            "Water",
            [
                Vector2::new(2, 3),
                Vector2::new(5, 3),
                Vector2::new(5, 7),
                Vector2::new(2, 7),
            ],
            false,
        ));

        let grid = engine
            .landscape()
            .and_then(Landscape::pixel_grid)
            .expect("quad keeps the pixel grid");
        let water = grid.material_for_byte(3).expect("Water resolves");
        assert_eq!(grid.material_count(water), 16);
        MATERIAL_COUNT_FULL_REBUILDS.with(|rebuilds| {
            assert_eq!(
                rebuilds.get(),
                0,
                "bounded DrawQuad count maintenance cannot rebuild the full plane"
            );
        });
    }

    #[test]
    fn draw_material_quad_rejects_unresolved_material_without_writes() {
        // GetIndexMatTex returns zero for an invalid material and DrawQuad
        // returns false before PrepareChange or Surface8::Polygon
        // (C4Texture.cpp:346-369; C4Landscape.cpp:2450-2452).
        let mut engine = crate::Engine::with_seed(10);
        engine.set_landscape(raster_grid_landscape(3, 3, vec![0; 9]));
        let revision = engine
            .landscape()
            .and_then(Landscape::pixel_grid)
            .expect("pixel grid")
            .revision();

        assert!(!engine.draw_material_quad(
            "Water-Missing",
            [
                Vector2::new(0, 0),
                Vector2::new(2, 0),
                Vector2::new(2, 2),
                Vector2::new(0, 2),
            ],
            false,
        ));

        let landscape = engine.landscape().expect("landscape remains");
        for y in 0..3 {
            for x in 0..3 {
                assert_eq!(landscape.grid_byte_at(x, y), Some(0));
            }
        }
        assert_eq!(landscape.surface(), &[3; 3]);
        assert_eq!(
            landscape.pixel_grid().expect("pixel grid").revision(),
            revision,
            "failed resolution never enters PrepareChange"
        );
    }

    #[test]
    fn raw_solid_mask_grid_write_does_not_enter_derived_columns() {
        // C4SolidMask puts MCVehic with raw _SBackPix and later restores the
        // saved background (C4SolidMask.cpp:323-363). Those temporary mask
        // bytes are collision truth while put, but are not a landscape
        // change: the Rust-only surface/liquid/IFT summaries stay untouched.
        let mut landscape = raster_grid_landscape(1, 4, vec![0, 0, 3 | 0x80, 1]);
        let surface = landscape.surface().to_vec();
        let liquids = landscape.liquids().to_vec();
        let was_tunnel = landscape.is_tunnel_at(0, 2);
        let revision = landscape.pixel_grid().expect("grid").revision();

        landscape.grid_write_byte(0, 0, 2);

        assert_eq!(landscape.grid_byte_at(0, 0), Some(2));
        assert_eq!(landscape.surface(), surface);
        assert_eq!(landscape.liquids(), liquids);
        assert_eq!(landscape.is_tunnel_at(0, 2), was_tunnel);
        assert_eq!(
            landscape.pixel_grid().expect("grid").revision(),
            revision + 1
        );
    }

    #[test]
    fn solid_mask_remove_reputs_newest_instance_first() {
        // A, B, C construct and put over the same background pixel. A owns
        // the saved sky byte; B/C see only MCVehic. Removing A walks
        // C4SolidMask::Last->Prev, so C claims the freed bytes before B
        // (C4SolidMask.cpp:263-274,387-400).
        let (mut engine, ids) = overlapping_solid_mask_engine();
        let indices = ids.map(|id| engine.find_object_index(id).expect("mask exists"));
        let sequences = indices.map(|index| {
            engine.objects[index]
                .solid_mask_bake
                .as_ref()
                .expect("mask is put")
                .instance_sequence
        });
        assert!(sequences[0] < sequences[1] && sequences[1] < sequences[2]);
        assert_eq!(
            engine.objects[indices[0]]
                .solid_mask_bake
                .as_ref()
                .expect("A bake")
                .buffer,
            vec![0]
        );

        engine.remove_solid_mask(indices[0]);

        assert_eq!(
            engine.objects[indices[2]]
                .solid_mask_bake
                .as_ref()
                .expect("C bake")
                .buffer,
            vec![0],
            "newest survivor C owns A's freed background"
        );
        assert_eq!(
            engine.objects[indices[1]]
                .solid_mask_bake
                .as_ref()
                .expect("B bake")
                .buffer,
            vec![2],
            "older survivor B sees C's already-restored MCVehic"
        );
    }

    #[test]
    fn solid_mask_first_instance_follows_construction_callback() {
        // NewObject starts at Con=0, so Init cannot create a mask.
        // Construction runs first; only the following initial DoCon makes
        // the new object eligible (C4Game.cpp:1117-1127). A mask recreated
        // by Construction is therefore older than the new object's mask.
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
protected func Construction(object creator)
{
    if (creator) SetSolidMask(0, 0, 1, 1, 0, 0, creator);
}
"#,
        )
        .expect("definition compiles");
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(11);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let spawn = || crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        let oldest = engine.spawn_object(spawn()).expect("oldest mask spawns");
        let recreated = engine.spawn_object(spawn()).expect("second mask spawns");
        let recreated_index = engine
            .find_object_index(recreated)
            .expect("second mask exists");
        let original_sequence = engine.objects[recreated_index]
            .solid_mask_bake
            .as_ref()
            .expect("second mask is put")
            .instance_sequence;

        let newest = engine
            .spawn_object_with_initial_lifecycle(spawn(), Some(recreated))
            .expect("native lifecycle completes")
            .expect("new mask survives Construction");
        let recreated_sequence = engine.objects[recreated_index]
            .solid_mask_bake
            .as_ref()
            .expect("Construction recreated target mask")
            .instance_sequence;
        let newest_index = engine
            .find_object_index(newest)
            .expect("newest mask exists");
        let newest_sequence = engine.objects[newest_index]
            .solid_mask_bake
            .as_ref()
            .expect("initial DoCon put the new mask")
            .instance_sequence;
        assert!(original_sequence < recreated_sequence);
        assert!(recreated_sequence < newest_sequence);

        let oldest_index = engine
            .find_object_index(oldest)
            .expect("oldest mask exists");
        engine.remove_solid_mask(oldest_index);
        assert_eq!(
            engine.objects[newest_index]
                .solid_mask_bake
                .as_ref()
                .expect("newest bake")
                .buffer,
            vec![0],
            "the post-Construction instance claims the freed background"
        );
        assert_eq!(
            engine.objects[recreated_index]
                .solid_mask_bake
                .as_ref()
                .expect("recreated bake")
                .buffer,
            vec![2]
        );
    }

    #[test]
    fn solid_mask_direct_spawn_finishes_construction_replay_scope() {
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
protected func Construction()
{
    SetSolidMask(0, 0, 1, 1, 0, 0);
    return true;
}
"#,
        )
        .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(16);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let mut spawn = crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        spawn.position_adjusted = true;
        let first = engine
            .spawn_object(spawn.clone())
            .expect("first mask spawns");
        let second = engine.spawn_object(spawn).expect("second mask spawns");

        for id in [first, second] {
            let index = engine.find_object_index(id).expect("spawn remains live");
            assert!(engine.objects[index].solid_mask_bake.is_some());
        }
        assert!(
            !engine.solid_mask_staging.defer_solid_mask_updates,
            "direct spawn must close and replay its deferred construction scope"
        );
    }

    #[test]
    fn solid_mask_foreign_only_construction_reserves_spawn_instance_after_stream() {
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
local Victim;

protected func Construction()
{
    if (Victim) SetSolidMask(0, 0, 1, 1, 0, 0, Victim);
    return true;
}
"#,
        )
        .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(19);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let spawn = || crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        let foreign = engine.spawn_object(spawn()).expect("foreign mask spawns");
        let mut config = spawn().with_local_vars(std::collections::HashMap::from([(
            "Victim".to_string(),
            clonk_script::Value::Object(foreign.as_u64()),
        )]));
        config.position_adjusted = true;
        let spawned = engine.spawn_object(config).expect("new mask spawns");
        let foreign_index = engine.find_object_index(foreign).expect("foreign exists");
        let spawned_index = engine.find_object_index(spawned).expect("spawned exists");
        let foreign_sequence = engine.objects[foreign_index]
            .solid_mask_instance_sequence
            .expect("Construction recreated foreign mask");
        let spawned_sequence = engine.objects[spawned_index]
            .solid_mask_instance_sequence
            .expect("spawn retained its default mask");
        assert!(foreign_sequence < spawned_sequence);
        assert!(engine.objects[spawned_index].solid_mask_bake.is_some());
    }

    #[test]
    fn solid_mask_direct_spawn_replays_after_same_call_children_materialize() {
        let mut mask =
            crate::Definition::from_script("MASK", "Mask", "").expect("mask definition compiles");
        mask.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        mask.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        let mut parent = crate::Definition::from_script(
            "PARN",
            "Parent",
            r#"
#strict 2
local Victim;

protected func Construction()
{
    CreateObject(MASK, 1, 2, -1);
    SetSolidMask(0, 0, 0, 0, 0, 0, Victim);
    return true;
}
"#,
        )
        .expect("parent definition compiles");
        parent.set_c4_callback_convention(true);

        let mut engine = crate::Engine::with_seed(17);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine.register_definition(mask).expect("mask registers");
        engine
            .register_definition(parent)
            .expect("parent registers");
        let mut mask_spawn = crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        mask_spawn.position_adjusted = true;
        let oldest = engine
            .spawn_object(mask_spawn.clone())
            .expect("oldest mask spawns");
        let survivor = engine
            .spawn_object(mask_spawn)
            .expect("survivor mask spawns");

        let mut parent_spawn = crate::SpawnConfig::new("PARN")
            .with_position(Vector2::ZERO)
            .with_local_vars(std::collections::HashMap::from([(
                "Victim".to_string(),
                clonk_script::Value::Object(oldest.as_u64()),
            )]));
        parent_spawn.position_adjusted = true;
        let parent_id = engine
            .spawn_object(parent_spawn)
            .expect("parent and child spawn");
        let child = engine
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "MASK" && object.id != oldest && object.id != survivor
            })
            .map(|object| object.id)
            .expect("Construction child materialized");
        let child_index = engine.find_object_index(child).expect("child exists");
        let survivor_index = engine.find_object_index(survivor).expect("survivor exists");
        assert!(engine.find_object_index(parent_id).is_some());
        assert_eq!(
            engine.objects[child_index]
                .solid_mask_bake
                .as_ref()
                .expect("child bake")
                .buffer,
            vec![0],
            "same-call child claims the victim's freed background"
        );
        assert_eq!(
            engine.objects[survivor_index]
                .solid_mask_bake
                .as_ref()
                .expect("older survivor bake")
                .buffer,
            vec![2]
        );
    }

    #[test]
    fn solid_mask_deferred_effect_keeps_native_creation_order() {
        // The effect creates A to completion and only then recreates B's
        // mask. C++ therefore links A before the new B instance even though
        // Rust folds the outer B update before materializing deferred A.
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
func Arm()
{
    AddEffect("Recreate", this(), 1, 1, this());
    return true;
}

func FxRecreateTimer(object target, int number, int time)
{
    CreateObject(MASK, 0, 1, -1);
    SetSolidMask(0, 0, 1, 1, 0, 0);
    return 0;
}
"#,
        )
        .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(12);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let spawn = || crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        let oldest = engine.spawn_object(spawn()).expect("oldest mask spawns");
        let recreated = engine.spawn_object(spawn()).expect("effect owner spawns");
        let recreated_index = engine
            .find_object_index(recreated)
            .expect("overlapping B exists");
        let original_sequence = engine.objects[recreated_index]
            .solid_mask_bake
            .as_ref()
            .expect("effect owner mask is put")
            .instance_sequence;

        engine
            .call_object_function(recreated_index, "Arm", Vec::new())
            .expect("effect arms");
        let effect = engine.objects[recreated_index]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Recreate")
            .cloned()
            .expect("recreation effect exists");
        let definition_id = engine.objects[recreated_index].definition_id.clone();
        engine
            .dispatch_object_effect_events(
                recreated_index,
                &definition_id,
                vec![crate::effect::EffectEvent::timer(effect)],
            )
            .expect("effect timer executes");

        let spawned = engine
            .objects
            .iter()
            .find(|object| object.id != oldest && object.id != recreated)
            .map(|object| object.id)
            .expect("effect spawned overlapping mask");
        let spawned_index = engine
            .find_object_index(spawned)
            .expect("spawned mask exists");
        let spawned_sequence = engine.objects[spawned_index]
            .solid_mask_bake
            .as_ref()
            .expect("spawned mask is put")
            .instance_sequence;
        let recreated_sequence = engine.objects[recreated_index]
            .solid_mask_bake
            .as_ref()
            .expect("effect owner mask was recreated")
            .instance_sequence;
        let spawned_position = engine.objects[spawned_index]
            .solid_mask_bake
            .as_ref()
            .map(|bake| (bake.x, bake.y));
        let recreated_position = engine.objects[recreated_index]
            .solid_mask_bake
            .as_ref()
            .map(|bake| (bake.x, bake.y));
        assert_eq!(spawned_position, recreated_position);
        assert!(original_sequence < spawned_sequence);
        assert!(
            spawned_sequence < recreated_sequence,
            "CreateObject's initial DoCon precedes the following SetSolidMask"
        );

        let oldest_index = engine
            .find_object_index(oldest)
            .expect("oldest mask exists");
        engine.remove_solid_mask(oldest_index);
        assert_eq!(
            engine.objects[recreated_index]
                .solid_mask_bake
                .as_ref()
                .expect("recreated bake")
                .buffer,
            vec![0],
            "newest recreated B claims the freed background"
        );
        assert_eq!(
            engine.objects[spawned_index]
                .solid_mask_bake
                .as_ref()
                .expect("spawned bake")
                .buffer,
            vec![2]
        );
    }

    #[test]
    fn solid_mask_effect_batch_threads_instance_age() {
        // Each effect callback gets its own copy-in/copy-out host context.
        // The first makes A eligible and then recreates B; the second does
        // an ordinary A re-put, which must retain A's older live instance.
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
func ArmThreaded(object b)
{
    AddEffect("First", this(), 1, 1, this(), 0, b);
    AddEffect("Second", this(), 1, 1, this());
    return true;
}

func FxFirstStart(object target, int number, int temp, object b)
{
    if (!temp) EffectVar(0, target, number) = b;
    return 0;
}

func FxFirstTimer(object target, int number, int time)
{
    DoCon(100);
    SetSolidMask(0, 0, 1, 1, 0, 0, EffectVar(0, target, number));
    return 0;
}

func FxSecondTimer(object target, int number, int time)
{
    DoCon(0);
    return 0;
}
"#,
        )
        .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(13);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let full = || crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        let _oldest = engine.spawn_object(full()).expect("oldest mask spawns");
        let recreated = engine.spawn_object(full()).expect("effect owner spawns");
        let mut pending_config = full().with_construction(0);
        pending_config.position_adjusted = true;
        let pending = engine
            .spawn_object(pending_config)
            .expect("incomplete mask object spawns");
        let recreated_index = engine
            .find_object_index(recreated)
            .expect("effect owner exists");
        let original_recreated_sequence = engine.objects[recreated_index]
            .solid_mask_bake
            .as_ref()
            .expect("effect owner mask is put")
            .instance_sequence;

        let pending_index = engine.find_object_index(pending).expect("A exists");
        engine
            .call_object_function(
                pending_index,
                "ArmThreaded",
                vec![clonk_script::Value::Object(recreated.as_u64())],
            )
            .expect("effects arm");
        let first = engine.objects[pending_index]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "First")
            .cloned()
            .expect("first effect exists");
        let second = engine.objects[pending_index]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Second")
            .cloned()
            .expect("second effect exists");
        assert_eq!(
            first.vars,
            vec![crate::effect::EffectVarValue::Object(recreated.as_u64())]
        );
        assert!(second.vars.is_empty());
        let definition_id = engine.objects[pending_index].definition_id.clone();
        engine
            .dispatch_object_effect_events(
                pending_index,
                &definition_id,
                vec![
                    crate::effect::EffectEvent::timer(first),
                    crate::effect::EffectEvent::timer(second),
                ],
            )
            .expect("effect batch executes");

        let pending_sequence = engine.objects[pending_index]
            .solid_mask_instance_sequence
            .expect("first effect created A's mask instance");
        let recreated_sequence = engine.objects[recreated_index]
            .solid_mask_instance_sequence
            .expect("first effect recreated B's mask instance");
        assert!(original_recreated_sequence < pending_sequence);
        assert!(
            pending_sequence < recreated_sequence,
            "the second effect's ordinary A re-put retains its first-effect age"
        );
    }

    #[test]
    fn solid_mask_effect_batch_threads_foreign_mask_state() {
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
func ArmForeignState(object other)
{
    AddEffect("DisableForeign", this(), 1, 1, this(), 0, other);
    AddEffect("MoveForeign", this(), 1, 1, this(), 0, other);
    return true;
}

func FxDisableForeignStart(object target, int number, int temp, object other)
{
    if (!temp) EffectVar(0, target, number) = other;
    return 0;
}

func FxDisableForeignTimer(object target, int number, int time)
{
    SetSolidMask(0, 0, 0, 0, 0, 0, EffectVar(0, target, number));
    return 0;
}

func FxMoveForeignStart(object target, int number, int temp, object other)
{
    if (!temp) EffectVar(0, target, number) = other;
    return 0;
}

func FxMoveForeignTimer(object target, int number, int time)
{
    SetPosition(2, 1, EffectVar(0, target, number));
    return 0;
}
"#,
        )
        .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(18);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let spawn = || crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        let outer = engine.spawn_object(spawn()).expect("outer mask spawns");
        let foreign = engine.spawn_object(spawn()).expect("foreign mask spawns");
        let outer_index = engine.find_object_index(outer).expect("outer exists");
        let foreign_index = engine.find_object_index(foreign).expect("foreign exists");

        engine
            .call_object_function(
                outer_index,
                "ArmForeignState",
                vec![clonk_script::Value::Object(foreign.as_u64())],
            )
            .expect("effects arm");
        let events = ["DisableForeign", "MoveForeign"].map(|name| {
            let effect = engine.objects[outer_index]
                .state
                .effects
                .iter()
                .find(|effect| effect.name == name)
                .cloned()
                .expect("effect exists");
            crate::effect::EffectEvent::timer(effect)
        });
        let definition_id = engine.objects[outer_index].definition_id.clone();
        engine
            .dispatch_object_effect_events(
                outer_index,
                &definition_id,
                events.into_iter().collect(),
            )
            .expect("effect batch executes");

        assert_eq!(
            engine.objects[foreign_index].state.position,
            Vector2::new(2, 1)
        );
        assert!(engine.objects[foreign_index].solid_mask_bake.is_none());
        assert!(engine.objects[foreign_index]
            .solid_mask_instance_sequence
            .is_none());
    }

    #[test]
    fn solid_mask_effect_batch_replays_foreign_and_outer_updates_in_call_order() {
        // A owns the shared background. Effect 1 recreates foreign B, then
        // effect 2 disables outer A. Native C++ therefore re-puts B before
        // removing A, and B (now newer than C) claims A's freed byte. The
        // copy-out channels must not remove A before applying foreign B.
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
func ArmOwnership(object b)
{
    AddEffect("RecreateForeign", this(), 1, 1, this(), 0, b);
    AddEffect("DisableOuter", this(), 1, 1, this());
    return true;
}

func FxRecreateForeignStart(object target, int number, int temp, object b)
{
    if (!temp) EffectVar(0, target, number) = b;
    return 0;
}

func FxRecreateForeignTimer(object target, int number, int time)
{
    SetSolidMask(0, 0, 1, 1, 0, 0, EffectVar(0, target, number));
    return 0;
}

func FxDisableOuterTimer(object target, int number, int time)
{
    SetSolidMask(0, 0, 0, 0);
    return 0;
}
"#,
        )
        .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(14);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let spawn = || crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        let a = engine.spawn_object(spawn()).expect("A spawns");
        let b = engine.spawn_object(spawn()).expect("B spawns");
        let c = engine.spawn_object(spawn()).expect("C spawns");
        let a_index = engine.find_object_index(a).expect("A exists");
        let b_index = engine.find_object_index(b).expect("B exists");
        let c_index = engine.find_object_index(c).expect("C exists");

        engine
            .call_object_function(
                a_index,
                "ArmOwnership",
                vec![clonk_script::Value::Object(b.as_u64())],
            )
            .expect("effects arm");
        let recreate = engine.objects[a_index]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "RecreateForeign")
            .cloned()
            .expect("foreign recreation effect exists");
        let disable = engine.objects[a_index]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "DisableOuter")
            .cloned()
            .expect("outer disable effect exists");
        let definition_id = engine.objects[a_index].definition_id.clone();
        engine
            .dispatch_object_effect_events(
                a_index,
                &definition_id,
                vec![
                    crate::effect::EffectEvent::timer(recreate),
                    crate::effect::EffectEvent::timer(disable),
                ],
            )
            .expect("effect batch executes");

        assert!(engine.objects[a_index].solid_mask_bake.is_none());
        assert!(engine.objects[a_index]
            .solid_mask_instance_sequence
            .is_none());
        assert!(
            engine.objects[b_index]
                .solid_mask_instance_sequence
                .expect("B has recreated instance")
                > engine.objects[c_index]
                    .solid_mask_instance_sequence
                    .expect("C keeps original instance")
        );
        assert_eq!(
            engine.objects[b_index]
                .solid_mask_bake
                .as_ref()
                .expect("B bake")
                .buffer,
            vec![0],
            "B is first to claim A's freed background"
        );
        assert_eq!(
            engine.objects[c_index]
                .solid_mask_bake
                .as_ref()
                .expect("C bake")
                .buffer,
            vec![2]
        );
    }

    #[test]
    fn solid_mask_replay_preserves_raw_landscape_write_order() {
        // DrawVolcanoBranch uses raw SetPix rather than the landscape mask
        // transaction. Its call must therefore remain interleaved with
        // SetSolidMask: Put->raw leaves Earth visible with the old saved
        // background, while raw->Put leaves MCVehic with Earth saved.
        let mut definition = crate::Definition::from_script(
            "MASK",
            "Mask",
            r#"
#strict 2
func PutThenRaw()
{
    SetSolidMask(0, 0, 1, 1, 0, 0);
    DrawVolcanoBranch(0, 1, 2, 1, 1, 2);
    return true;
}

func RawThenPut()
{
    DrawVolcanoBranch(0, 1, 2, 1, 1, 2);
    SetSolidMask(0, 0, 1, 1, 0, 0);
    return true;
}

func RawThenTransaction()
{
    SetSolidMask(0, 0, 0, 0);
    DrawVolcanoBranch(0, 1, 2, 1, 1, 2);
    DrawMaterialQuad("Water", 1,1, 2,1, 2,2, 1,2, false);
    return true;
}

func TransactionThenRaw()
{
    DrawMaterialQuad("Water", 1,1, 2,1, 2,2, 1,2, false);
    DrawVolcanoBranch(0, 1, 2, 1, 1, 2);
    return true;
}
"#,
        )
        .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(15);
        engine.set_landscape(raster_grid_landscape(4, 5, vec![0; 20]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let mut spawn = crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1));
        spawn.position_adjusted = true;
        let id = engine.spawn_object(spawn).expect("mask spawns");
        let index = engine.find_object_index(id).expect("mask exists");

        engine
            .call_object_function(index, "PutThenRaw", Vec::new())
            .expect("put then raw executes");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.grid_byte_at(1, 1)),
            Some(1),
            "raw Earth remains visible after the preceding Put"
        );
        assert_eq!(
            engine.objects[index]
                .solid_mask_bake
                .as_ref()
                .expect("mask bake remains linked")
                .buffer,
            vec![0]
        );

        engine
            .call_object_function(index, "RawThenPut", Vec::new())
            .expect("raw then put executes");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.grid_byte_at(1, 1)),
            Some(2),
            "the following Put restores MCVehic"
        );
        assert_eq!(
            engine.objects[index]
                .solid_mask_bake
                .as_ref()
                .expect("mask bake remains linked")
                .buffer,
            vec![1],
            "Put saves the raw Earth byte"
        );

        engine
            .call_object_function(index, "RawThenTransaction", Vec::new())
            .expect("raw then transaction executes");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.grid_byte_at(1, 1)),
            Some(3),
            "the later transactional Water draw wins"
        );

        engine
            .call_object_function(index, "TransactionThenRaw", Vec::new())
            .expect("transaction then raw executes");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.grid_byte_at(1, 1)),
            Some(1),
            "the later raw Earth write wins"
        );
    }

    #[test]
    fn solid_mask_rotation_eligibility_cycle_recreates_instance() {
        let (mut engine, ids) = overlapping_solid_mask_engine();
        let indices = ids.map(|id| engine.find_object_index(id).expect("mask exists"));
        let previous_newest = engine.objects[indices[2]]
            .solid_mask_bake
            .as_ref()
            .expect("C bake")
            .instance_sequence;

        engine
            .call_object_function(indices[1], "RotateMaskCycle", Vec::new())
            .expect("rotation cycle completes");
        let recreated_b = engine.objects[indices[1]]
            .solid_mask_bake
            .as_ref()
            .expect("B mask was recreated")
            .instance_sequence;
        assert!(recreated_b > previous_newest);

        engine.remove_solid_mask(indices[0]);
        assert_eq!(
            engine.objects[indices[1]]
                .solid_mask_bake
                .as_ref()
                .expect("B bake")
                .buffer,
            vec![0],
            "rotation eligibility loss moves B to the native list tail"
        );
        assert_eq!(
            engine.objects[indices[2]]
                .solid_mask_bake
                .as_ref()
                .expect("C bake")
                .buffer,
            vec![2]
        );
    }

    #[test]
    fn solid_mask_set_position_reputs_only_after_integer_motion() {
        let (mut engine, ids) = overlapping_solid_mask_engine();
        let indices = ids.map(|id| engine.find_object_index(id).expect("mask exists"));
        let original_sequence = engine.objects[indices[1]]
            .solid_mask_bake
            .as_ref()
            .expect("B bake")
            .instance_sequence;

        engine
            .call_object_function(
                indices[1],
                "MoveMask",
                vec![clonk_script::Value::Int(2), clonk_script::Value::Int(1)],
            )
            .expect("changed SetPosition succeeds");
        let moved = engine.objects[indices[1]]
            .solid_mask_bake
            .as_ref()
            .expect("B was re-put at its new position");
        assert_eq!((moved.x, moved.y), (2, 1));
        assert_eq!(moved.instance_sequence, original_sequence);
        assert_eq!(moved.buffer, vec![0]);

        let before_same_position = moved.clone();
        engine
            .call_object_function(
                indices[1],
                "MoveMask",
                vec![clonk_script::Value::Int(2), clonk_script::Value::Int(1)],
            )
            .expect("same-position SetPosition succeeds");
        let after_same_position = engine.objects[indices[1]]
            .solid_mask_bake
            .as_ref()
            .expect("same-position call leaves the bake put");
        assert_eq!(
            after_same_position.instance_sequence,
            before_same_position.instance_sequence
        );
        assert_eq!(after_same_position.buffer, before_same_position.buffer);

        engine.remove_solid_mask(indices[0]);
        assert_eq!(
            engine.objects[indices[2]]
                .solid_mask_bake
                .as_ref()
                .expect("C bake")
                .buffer,
            vec![0],
            "moving B away leaves C to claim A's freed old pixel"
        );
    }

    #[test]
    fn solid_mask_containment_eligibility_cycle_recreates_instance() {
        let (mut engine, ids) = overlapping_solid_mask_engine();
        let target_index = engine.find_object_index(ids[1]).expect("target exists");
        let container = engine
            .spawn_object(crate::SpawnConfig::new("MASK").with_position(Vector2::new(-20, -20)))
            .expect("offscreen container spawns");
        let container_index = engine
            .find_object_index(container)
            .expect("container exists");
        let container_sequence = engine.objects[container_index]
            .solid_mask_instance_sequence
            .expect("eligible offscreen container has an instance");

        let result = engine
            .call_object_function(
                target_index,
                "ContainerMaskCycle",
                vec![clonk_script::Value::Object(container.as_u64())],
            )
            .expect("containment cycle completes");
        assert_eq!(result, clonk_script::Value::Int(3));
        assert!(engine.objects[target_index].state.container.is_none());
        assert!(engine.objects[target_index].solid_mask_bake.is_none());
        let target_sequence = engine.objects[target_index]
            .solid_mask_instance_sequence
            .expect("Exit recreated the eligible offscreen instance");
        assert!(
            target_sequence > container_sequence,
            "target sequence {target_sequence}, container sequence {container_sequence}"
        );
    }

    #[test]
    fn solid_mask_instance_age_survives_reput_but_setsolidmask_recreates() {
        let (mut engine, ids) = overlapping_solid_mask_engine();
        let indices = ids.map(|id| engine.find_object_index(id).expect("mask exists"));
        let original_b = engine.objects[indices[1]]
            .solid_mask_bake
            .as_ref()
            .expect("B bake")
            .instance_sequence;
        let original_c = engine.objects[indices[2]]
            .solid_mask_bake
            .as_ref()
            .expect("C bake")
            .instance_sequence;

        engine.update_solid_mask(indices[1]);
        assert_eq!(
            engine.objects[indices[1]]
                .solid_mask_bake
                .as_ref()
                .expect("B re-put")
                .instance_sequence,
            original_b,
            "ordinary Remove/Put retains the live C4SolidMask instance"
        );

        engine
            .apply_object_update(
                ids[1],
                crate::ObjectUpdate {
                    solid_mask_override: Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)),
                    ..crate::ObjectUpdate::default()
                },
            )
            .expect("same-rect SetSolidMask update applies");
        let recreated_b = engine.objects[indices[1]]
            .solid_mask_bake
            .as_ref()
            .expect("B recreated")
            .instance_sequence;
        assert!(
            recreated_b > original_c,
            "SetSolidMask deletes and reconstructs B at the list tail"
        );

        engine
            .apply_object_update(
                ids[1],
                crate::ObjectUpdate {
                    change_def: Some("MASK".to_string()),
                    ..crate::ObjectUpdate::default()
                },
            )
            .expect("same-definition ChangeDef update applies");
        let changed_def_b = engine.objects[indices[1]]
            .solid_mask_bake
            .as_ref()
            .expect("B recreated after ChangeDef")
            .instance_sequence;
        assert!(
            changed_def_b > recreated_b,
            "ChangeDef deletes and reconstructs even for the same definition"
        );

        engine.remove_solid_mask(indices[0]);
        assert_eq!(
            engine.objects[indices[1]]
                .solid_mask_bake
                .as_ref()
                .expect("B bake")
                .buffer,
            vec![0],
            "recreated newest B now claims A's freed background"
        );
        assert_eq!(
            engine.objects[indices[2]]
                .solid_mask_bake
                .as_ref()
                .expect("C bake")
                .buffer,
            vec![2]
        );
    }

    #[test]
    fn solid_mask_offscreen_instance_age_survives_until_eligibility_loss() {
        // C4SolidMask construction links the instance before Put clips it.
        // A fully off-landscape eligible mask therefore has no raster bake,
        // but remains logically put and keeps its list age when it later
        // moves on-screen.
        let mut definition =
            crate::Definition::from_script("MASK", "Mask", "").expect("definition compiles");
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        let mut engine = crate::Engine::with_seed(9);
        engine.set_landscape(raster_grid_landscape(4, 4, vec![0; 16]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(crate::SpawnConfig::new("MASK").with_position(Vector2::new(-20, -20)))
            .expect("offscreen mask spawns");
        let index = engine.find_object_index(id).expect("mask exists");
        let offscreen_sequence = engine.objects[index]
            .solid_mask_instance_sequence
            .expect("eligible offscreen instance was constructed");
        assert!(engine.objects[index].solid_mask_bake.is_none());
        assert!(engine.objects[index].solid_mask_empty_put);

        engine.objects[index].set_position(Vector2::new(1, 1));
        engine.update_solid_mask(index);
        assert_eq!(
            engine.objects[index]
                .solid_mask_bake
                .as_ref()
                .expect("on-screen mask puts")
                .instance_sequence,
            offscreen_sequence
        );
        assert!(!engine.objects[index].solid_mask_empty_put);

        engine.objects[index].state.container = Some(crate::ObjectId::new(999));
        engine.update_solid_mask(index);
        assert!(engine.objects[index].solid_mask_instance_sequence.is_none());
        engine.objects[index].state.container = None;
        engine.update_solid_mask(index);
        assert!(
            engine.objects[index]
                .solid_mask_bake
                .as_ref()
                .expect("re-eligible mask puts")
                .instance_sequence
                > offscreen_sequence,
            "regaining eligibility constructs a new tail instance"
        );
    }

    #[test]
    fn draw_material_quad_repairs_solid_mask_over_changed_background() {
        // PrepareChange removes intersecting masks temporarily and
        // FinishChange repairs them (C4Landscape.cpp:2851-2880).
        // C4SolidMask::Repair records the changed background before putting
        // MCVehic back (C4SolidMask.cpp:365-383), so removing the mask later
        // must reveal the new water+IFT byte, not the old sky.
        let mut definition =
            crate::Definition::from_script("MASK", "Mask", "").expect("definition compiles");
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = crate::Engine::with_seed(7);
        engine.set_landscape(raster_grid_landscape(4, 4, vec![0; 16]));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1)))
            .expect("mask spawns");
        let index = engine.find_object_index(id).expect("mask exists");
        assert!(engine.solid_mask_grid_mode());
        assert!(engine.solid_mask_spec(index).is_some());
        engine.update_solid_mask(index);
        let (mask_x, mask_y) = engine.objects[index]
            .solid_mask_bake
            .as_ref()
            .map(|bake| (bake.x, bake.y))
            .expect("mask is put");
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(mask_x, mask_y)),
            Some(2)
        );

        assert!(engine.draw_material_quad(
            "Water",
            [
                Vector2::new(mask_x, mask_y),
                Vector2::new(mask_x + 1, mask_y),
                Vector2::new(mask_x + 1, mask_y + 1),
                Vector2::new(mask_x, mask_y + 1),
            ],
            true,
        ));

        let landscape = engine.landscape().expect("landscape");
        assert_eq!(
            landscape.grid_byte_at(mask_x, mask_y),
            Some(2),
            "mask is repaired"
        );
        assert_eq!(landscape.surface_height(mask_x), Some(4));
        assert!(landscape.liquids()[mask_x as usize].contains(mask_y));
        assert!(landscape.is_tunnel_at(mask_x, mask_y));

        engine.remove_solid_mask(index);
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(mask_x, mask_y)),
            Some(3 | 0x80),
            "removing the mask reveals the changed background"
        );
    }

    #[test]
    fn draw_map_writes_zoomed_ift_pixels_and_repairs_solid_mask() {
        // DrawMap hands its full indexed map to MapToLandscape, whose target
        // is map-size * MapZoom and whose MapToSurface preserves each source
        // cell's IFT bit (C4Landscape.cpp:337-376,453-510,2636-2668).
        // PrepareChange/FinishChange must also repair an intersecting solid
        // mask over the newly written background (C4Landscape.cpp:2851-2880;
        // C4SolidMask.cpp:323-342,365-385).
        let mut definition =
            crate::Definition::from_script("MASK", "Mask", "").expect("definition compiles");
        definition.set_shape_rect(Some(crate::DefinitionRect::new(0, 0, 1, 1)));
        definition.set_solid_mask(Some(crate::DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut landscape = raster_grid_landscape(7, 5, vec![1; 35]);
        let mut texmap = landscape
            .raster_state()
            .expect("raster state")
            .texmap()
            .clone();
        texmap.shapes[3] = Some(crate::chunky::ChunkShape::Flat);
        landscape.set_raster_state(LandscapeRasterState::new(2, 0, texmap.clone()));
        let mut engine = crate::Engine::with_seed(11);
        engine.set_landscape(landscape);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(crate::SpawnConfig::new("MASK").with_position(Vector2::new(1, 1)))
            .expect("mask spawns");
        let index = engine.find_object_index(id).expect("mask exists");
        engine.update_solid_mask(index);
        let (mask_x, mask_y) = engine.objects[index]
            .solid_mask_bake
            .as_ref()
            .map(|bake| (bake.x, bake.y))
            .expect("mask is put");

        let bitmap = clonk_resources::bitmap::IndexedBitmap {
            width: 3,
            height: 1,
            indices: vec![3 | 0x80, 0, 3 | 0x80],
        };
        assert!(engine.draw_indexed_map(Vector2::new(mask_x, mask_y), &bitmap, 2, 1, texmap));

        let landscape = engine.landscape().expect("landscape");
        assert_eq!(landscape.grid_byte_at(mask_x, mask_y), Some(2));
        assert_eq!(landscape.grid_byte_at(mask_x + 1, mask_y), Some(3 | 0x80));
        assert_eq!(landscape.grid_byte_at(mask_x, mask_y + 1), Some(3 | 0x80));
        assert_eq!(
            landscape.grid_byte_at(mask_x + 1, mask_y + 1),
            Some(3 | 0x80)
        );
        assert_eq!(
            landscape.grid_byte_at(mask_x + 3, mask_y),
            Some(0),
            "SkyToLandscape clears old material under zero map cells"
        );
        assert_eq!(
            landscape.grid_byte_at(mask_x + 4, mask_y),
            Some(1),
            "MapToLandscape does not copy rendered cells past the requested segment"
        );
        for x in mask_x..=mask_x + 1 {
            assert!(landscape.liquids()[x as usize].contains(mask_y));
            assert!(landscape.is_tunnel_at(x, mask_y));
        }

        engine.remove_solid_mask(index);
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(mask_x, mask_y)),
            Some(3 | 0x80),
            "removing the mask reveals DrawMap's changed background"
        );
    }

    #[test]
    fn explicit_world_height_overrides_the_surface_estimate() {
        // GBackHgt is the real landscape height (C4Landscape.h `Height`),
        // not the deepest surface column: search loops and border rules
        // (C4Landscape.h:144-161) bound on it.
        let mut landscape = Landscape::flat(10, 50);
        assert_eq!(landscape.estimated_height(), 50);
        landscape.set_world_height(400);
        assert_eq!(landscape.estimated_height(), 400);
        // Below the real world bottom stays border-solid.
        assert!(landscape.is_solid_at(5, 400));
        // Inside the ground body is solid as before.
        assert!(landscape.is_solid_at(5, 200));
    }

    #[test]
    fn estimated_height_cache_tracks_surface_and_liquid_mutations() {
        let material = MaterialId::new(0).expect("material id");
        let mut landscape =
            Landscape::new(3, vec![10, 20, 30]).expect("synthetic landscape builds");

        assert!(landscape.estimated_height_cache.0.get().is_none());
        assert_eq!(landscape.estimated_height(), 30);
        assert_eq!(landscape.estimated_height_cache.0.get(), Some(&30));

        landscape.set_height(2, 5);
        assert!(landscape.estimated_height_cache.0.get().is_none());
        assert_eq!(landscape.estimated_height(), 20);

        landscape.set_liquid_column(0, vec![LiquidSegment::new(40, 50)]);
        assert_eq!(landscape.estimated_height(), 50);
        assert_eq!(landscape.remove_liquid_at(0, 50), None);
        assert_eq!(landscape.estimated_height(), 49);
        landscape.clear_liquid_column(0);
        assert_eq!(landscape.estimated_height(), 20);

        landscape.lower_range(0, 1, 60);
        assert_eq!(landscape.estimated_height(), 60);
        landscape.ensure_surface_at_least(1, 70);
        assert_eq!(landscape.estimated_height(), 70);
        assert!(landscape.insert_material_at(2, 79, material));
        assert_eq!(landscape.estimated_height(), 80);
        assert!(landscape.remove_material_at(2, 79));
        assert_eq!(landscape.estimated_height(), 79);
    }

    #[test]
    fn estimated_height_cache_is_runtime_only_state() {
        let uncached = Landscape::flat(8, 40);
        let cached = uncached.clone();
        assert_eq!(cached.estimated_height(), 40);

        assert_eq!(cached, uncached);
        assert_eq!(
            serde_json::to_value(&cached).expect("cached landscape serializes"),
            serde_json::to_value(&uncached).expect("uncached landscape serializes")
        );
    }

    #[test]
    fn semi_solid_includes_liquids() {
        // GBackSemiSolid = density >= C4M_SemiSolid(25) (C4Material.h:202),
        // which liquids satisfy; GBackSolid = density >= C4M_Solid(50).
        let mut landscape = flat_world();
        landscape.set_liquid_column(
            25,
            vec![LiquidSegment {
                top: 40,
                bottom: 49,
                material: None,
            }],
        );
        assert!(landscape.is_semi_solid_at(25, 45), "water is semi-solid");
        assert!(!landscape.is_solid_at(25, 45), "water is not solid");
        assert!(landscape.is_semi_solid_at(25, 60), "ground is semi-solid");
        assert!(!landscape.is_semi_solid_at(25, 10), "sky is neither");
    }

    #[test]
    fn above_semi_solid_is_asymmetric_like_cpp() {
        // AboveSemiSolid (C4Landscape.cpp:1736-1755): scanning UP from a
        // buried point returns the first FREE pixel; scanning DOWN from a
        // free point returns the first SEMI-SOLID pixel.
        let landscape = flat_world();
        // Buried at y=70: first free pixel above ground is 49.
        assert_eq!(landscape.above_semi_solid(10, 70), Some(49));
        // In the air at y=30: lands ON the first solid row, 50.
        assert_eq!(landscape.above_semi_solid(10, 30), Some(50));

        // Above a lake the DOWN scan stops on the water surface row.
        let mut with_lake = flat_world();
        with_lake.set_liquid_column(
            25,
            vec![LiquidSegment {
                top: 40,
                bottom: 49,
                material: None,
            }],
        );
        assert_eq!(with_lake.above_semi_solid(25, 35), Some(40));
        // Buried under the lake: first free pixel above the WATER.
        assert_eq!(with_lake.above_semi_solid(25, 70), Some(39));
    }

    #[test]
    fn above_solid_skips_liquids_but_semi_above_solid_accepts_them() {
        // AboveSolid wants free-of-SEMI-solid above solid
        // (C4Landscape.cpp:1757-1782); SemiAboveSolid only wants
        // free-of-SOLID above solid (:1784-1809), so a water column
        // resting on rock satisfies the latter but not the former.
        let mut landscape = flat_world();
        landscape.set_liquid_column(
            25,
            vec![LiquidSegment {
                top: 40,
                bottom: 49,
                material: None,
            }],
        );
        assert_eq!(landscape.above_solid(10, 30), Some(49));
        assert_eq!(landscape.above_solid(25, 30), None);
        assert_eq!(landscape.semi_above_solid(25, 30), Some(49));
    }

    #[test]
    fn find_solid_ground_returns_bottom_center_of_the_left_run() {
        // FindSolidGround (C4Landscape.cpp:1843-1869): two-sided column
        // scan; the LEFT run is checked first, the result is the run's
        // center x, and the final AboveSemiSolid lands the y ON the first
        // solid row.
        let landscape = flat_world();
        assert_eq!(landscape.find_solid_ground(50, 30, 10), Some((46, 50)));
    }

    #[test]
    fn find_level_ground_rejects_steps_larger_than_hrange() {
        // FindLevelGround (C4Landscape.cpp:1942-1976): the run resets when
        // the surface height jumps by >= hrange, so the search walks past a
        // terrace step and settles on the flat side. The final
        // AboveSemiSolid starts ON the solid row and so returns the free
        // row above it (the up-scan branch of C4Landscape.cpp:1740-1745).
        let mut surfaces = vec![50; 30];
        surfaces.extend(vec![45; 30]);
        let mut landscape = Landscape::new(60, surfaces).expect("landscape builds");
        landscape.set_world_height(400);
        assert_eq!(landscape.find_level_ground(35, 20, 10, 3), Some((41, 44)));
    }

    #[test]
    fn find_con_site_spot_consults_the_overlap_check() {
        // FindConSiteSpot (C4Landscape.cpp:1982-2043): a run only counts as
        // found when the construction-site rect (x, y-hgt-10, wdt, hgt+40)
        // is free of overlapping objects; otherwise the scan keeps walking.
        let landscape = flat_world();
        // No obstructions: spot right of center (left offset start walks
        // left; the left run finds its spot first).
        assert_eq!(
            landscape.find_con_site_spot(50, 20, 10, 8, -1, |_, _, _, _| false),
            Some((50, 49))
        );
        // Everything left of x=60 blocked: the right-hand scan walks until
        // its rect clears the blocked region.
        // Everything left of x=60 blocked: the right-hand scan walks until
        // its rect clears the blocked region. The surface follower
        // oscillates between the free row (49) and the solid row (50) each
        // column (AboveSemiSolid asymmetry), so the y at the exit column —
        // and hence the final adjustment — depends on that parity: here the
        // tracker sits on the free row and the final AboveSemiSolid
        // descends onto the solid row.
        let blocked = landscape.find_con_site_spot(50, 20, 10, 8, -1, |x, _, _, _| x < 60);
        let (x, y) = blocked.expect("spot found right of the blocked region");
        assert!(x >= 60, "blocked region avoided, got x={x}");
        assert_eq!(y, 50);
    }

    fn legacy_expected_surface(initial: &[i32], center: Vector2, radius: i32) -> Vec<i32> {
        let mut surface = initial.to_vec();
        if radius <= 0 || surface.is_empty() {
            return surface;
        }
        let width = surface.len() as i32;
        let radius_sq = i64::from(radius) * i64::from(radius);

        for y_offset in -radius..=radius {
            let y_offset_i64 = i64::from(y_offset);
            let remaining = radius_sq - y_offset_i64 * y_offset_i64;
            if remaining < 0 {
                continue;
            }
            let horizontal = (remaining as f64).sqrt().floor() as i32;
            let start = -horizontal;
            let end = if horizontal == 0 { 1 } else { horizontal };
            let y = center.y + y_offset;
            for x_offset in start..end {
                let column = center.x + x_offset;
                if column < 0 || column >= width {
                    continue;
                }
                let index = column as usize;
                if y < surface[index] {
                    continue;
                }
                let candidate = y.saturating_add(1);
                if candidate > surface[index] {
                    surface[index] = candidate;
                }
            }
        }

        surface
    }

    #[test]
    fn solid_material_accessors() {
        let material_a = MaterialId::new(0).unwrap();
        let material_b = MaterialId::new(1).unwrap();
        let mut landscape = Landscape::flat_with_material(4, 6, Some(material_a));
        assert_eq!(landscape.solid_material_at(0), Some(material_a));
        landscape.set_solid_material(1, Some(material_b));
        assert_eq!(landscape.solid_material_at(1), Some(material_b));
        landscape.fill_solid_material(None);
        assert_eq!(landscape.solid_material_at(2), None);
        landscape.set_default_solid_material(Some(material_a));
        assert_eq!(landscape.default_solid_material(), Some(material_a));
        assert_eq!(landscape.solid_material_at(3), Some(material_a));
    }

    #[test]
    fn insert_material_backfills_surface_depth() {
        let material = MaterialId::new(0).unwrap();
        let mut landscape = Landscape::flat_with_material(5, 40, Some(material));
        landscape.set_height(2, 60);
        assert_eq!(landscape.surface()[2], 60);

        assert!(landscape.insert_material_at(2, 39, material));
        assert_eq!(landscape.surface()[2], 40);
    }

    #[test]
    fn lower_range_expands_surface_depth() {
        let mut landscape = Landscape::flat(8, 10);
        landscape.lower_range(2, 6, 14);
        assert_eq!(landscape.surface()[1], 10);
        assert_eq!(landscape.surface()[2], 14);
        assert_eq!(landscape.surface()[5], 14);
        assert_eq!(landscape.surface()[6], 10);
    }

    #[test]
    fn lower_range_clamps_bounds_and_ignores_raises() {
        let mut landscape = Landscape::flat(5, 12);
        landscape.lower_range(-3, 3, 18);
        assert_eq!(landscape.surface()[0], 18);
        assert_eq!(landscape.surface()[2], 18);
        landscape.lower_range(0, 5, 6);
        assert_eq!(landscape.surface()[2], 18);
        assert_eq!(landscape.surface()[4], 12);
    }

    #[test]
    fn path_free_reports_clear_segment_above_surface() {
        let landscape = Landscape::flat(16, 8);
        let start = Vector2::new(0, 0);
        let end = Vector2::new(15, 1);
        assert!(landscape.path_is_clear(start, end));
        assert!(landscape.first_collision_on_line(start, end).is_none());
    }

    #[test]
    fn path_free_reports_collision_when_line_enters_surface() {
        let landscape = Landscape::flat(16, 8);
        let start = Vector2::new(0, 0);
        let end = Vector2::new(15, 16);
        let collision = landscape.first_collision_on_line(start, end);
        assert!(collision.is_some());
        let hit = collision.unwrap();
        assert!(hit.y >= 8);
        assert!(!landscape.path_is_clear(start, end));
    }

    #[test]
    fn ignore_vehicle_solid_probe_treats_every_low_index_material_as_free() {
        // C++ accidentally passes Pix2Mat(pixel), a material index, to
        // DensitySolid. With fewer than 50 materials even dense Granite is
        // free, as are sky, Vehicle pixels (including IFT), and closed borders.
        let granite = MaterialId::new(17).expect("low Granite index");
        let vehicle = MaterialId::new(18).expect("low Vehicle index");
        let mut grid = PixelGrid::new(
            4,
            1,
            vec![1, 0, 2, 0x81],
            vec![0, 100, 100],
            vec![None, Some("Granite".to_owned()), Some("Vehicle".to_owned())],
            vec![None; 3],
        );
        grid.resolve_materials(|name| match name {
            "Granite" => Some(granite),
            "Vehicle" => Some(vehicle),
            _ => None,
        });
        let mut landscape = Landscape::new(4, vec![1; 4]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(1);
        landscape.set_vehicle_material(Some(vehicle));

        assert!(!landscape.is_solid_ignoring_vehicle_at(0, 0));
        assert!(!landscape.is_solid_ignoring_vehicle_at(1, 0));
        assert!(!landscape.is_solid_ignoring_vehicle_at(2, 0));
        assert!(!landscape.is_solid_ignoring_vehicle_at(3, 0));
        assert!(!landscape.is_solid_ignoring_vehicle_at(-1, 0));
        assert!(!landscape.is_solid_ignoring_vehicle_at(4, 0));
    }

    #[test]
    fn ignore_vehicle_solid_probe_blocks_only_non_vehicle_index_at_least_50() {
        let dense_low = MaterialId::new(49).expect("last passable material index");
        let sparse_high = MaterialId::new(50).expect("first blocking material index");
        let vehicle = MaterialId::new(51).expect("high Vehicle index");
        let mut grid = PixelGrid::new(
            3,
            1,
            vec![1, 2, 3],
            vec![0, 100, 1, 100],
            vec![
                None,
                Some("DenseLow".to_owned()),
                Some("SparseHigh".to_owned()),
                Some("Vehicle".to_owned()),
            ],
            vec![None; 4],
        );
        grid.resolve_materials(|name| match name {
            "DenseLow" => Some(dense_low),
            "SparseHigh" => Some(sparse_high),
            "Vehicle" => Some(vehicle),
            _ => None,
        });
        let mut landscape = Landscape::new(3, vec![1; 3]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(1);
        landscape.set_vehicle_material(Some(vehicle));

        assert!(!landscape.is_solid_ignoring_vehicle_at(0, 0));
        assert!(landscape.is_solid_ignoring_vehicle_at(1, 0));
        assert!(!landscape.is_solid_ignoring_vehicle_at(2, 0));
    }

    #[test]
    fn liquid_column_contains_points_in_segment() {
        let mut landscape = Landscape::flat(6, 10);
        landscape.set_liquid_column(2, vec![LiquidSegment::new(4, 7)]);
        assert!(landscape.is_liquid_at(2, 4));
        assert!(landscape.is_liquid_at(2, 6));
        assert!(landscape.is_liquid_at(2, 7));
        assert!(!landscape.is_liquid_at(2, 3));
        assert!(!landscape.is_liquid_at(2, 8));
        assert!(!landscape.is_liquid_at(5, 5));
    }

    #[test]
    fn liquid_column_merges_overlapping_segments() {
        let mut landscape = Landscape::flat(4, 10);
        landscape.set_liquid_column(
            1,
            vec![
                LiquidSegment::new(2, 4),
                LiquidSegment::new(5, 6),
                LiquidSegment::new(8, 9),
                LiquidSegment::new(3, 7),
            ],
        );
        let column = &landscape.liquids()[1];
        assert_eq!(column.segments(), &[LiquidSegment::new(2, 9)]);
    }

    #[test]
    fn liquid_column_retains_disjoint_segments() {
        let mut landscape = Landscape::flat(4, 10);
        landscape.set_liquid_column(1, vec![LiquidSegment::new(2, 4), LiquidSegment::new(7, 8)]);
        let column = &landscape.liquids()[1];
        assert_eq!(
            column.segments(),
            &[LiquidSegment::new(2, 4), LiquidSegment::new(7, 8)]
        );
    }

    #[test]
    fn landscape_command_set_liquid_column_applies_segments() {
        let mut landscape = Landscape::flat(5, 12);
        let command = LandscapeCommand::SetLiquidColumn {
            column: 2,
            segments: vec![LiquidSegment::new(3, 5), LiquidSegment::new(7, 9)],
        };
        command.apply(&mut landscape);
        let column = &landscape.liquids()[2];
        assert_eq!(
            column.segments(),
            &[LiquidSegment::new(3, 5), LiquidSegment::new(7, 9)]
        );
    }

    #[test]
    fn landscape_command_clear_liquid_column_removes_segments() {
        let mut landscape = Landscape::flat(5, 12);
        landscape.set_liquid_column(3, vec![LiquidSegment::new(4, 6)]);
        LandscapeCommand::ClearLiquidColumn { column: 3 }.apply(&mut landscape);
        assert!(landscape.liquids()[3].segments().is_empty());
    }

    #[test]
    fn path_free_walks_coarse_cells_like_cpp() {
        // C4Landscape::_PathFree (C4Landscape.cpp:890-915): coordinates are
        // divided into 17×15 cells; the walk steps diagonally while both
        // axes differ, then straight; any cell containing a nonzero-density
        // pixel (UpdatePixCnt, C4Landscape.cpp:2894-2908) blocks the path.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25

            [Material Water]
            Name=Water
            Density=25
            Friction=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let water = materials.id_of("Water").expect("water exists");

        // Flat earth surface at y = 30: cell row 2 (rows 30-44) is occupied,
        // rows 0-29 (cell rows 0-1) are open sky.
        let mut landscape = Landscape::flat_with_material(64, 30, Some(earth));

        // Sky-only horizontal path.
        assert!(landscape.path_free(5, 5, 40, 10, &materials));
        // Vertical path crossing the surface cell row.
        assert!(!landscape.path_free(5, 5, 5, 40, &materials));
        // Diagonal phase: (0,0)→(50,40) walks cells (0,0),(1,1),(2,2);
        // cell (2,2) covers rows 30-44 → blocked.
        assert!(!landscape.path_free(0, 0, 50, 40, &materials));
        // Endpoint cell itself occupied → blocked even if the walk is clear.
        assert!(!landscape.path_free(5, 5, 5, 31, &materials));

        // A liquid segment in an otherwise open cell blocks too (PixCnt
        // counts any nonzero density, liquids included).
        landscape.set_liquid_column(40, vec![LiquidSegment::with_material(5, 8, Some(water))]);
        assert!(!landscape.path_free(5, 5, 60, 10, &materials));
    }

    #[test]
    fn negative_max_slide_skips_cslide_zero_find_mat_top_search() {
        // C4Landscape::FindMatTop (C4Landscape.cpp:1100-1127) starts its
        // upward search at cslide=0. A negative MaxSlide therefore skips the
        // loop entirely and extracts the probed pixel, while zero still
        // climbs through the same-material pixel directly above it.
        let library = MaterialLibrary::parse(
            r#"
            [Material Negative]
            Name=Negative
            Density=25
            MaxSlide=-1

            [Material Zero]
            Name=Zero
            Density=25
            MaxSlide=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let negative = materials
            .id_of("Negative")
            .expect("negative material exists");
        let zero = materials.id_of("Zero").expect("zero material exists");
        assert_eq!(
            materials
                .get_by_id(negative)
                .map(|material| material.max_slide()),
            Some(-1),
            "the signed field must survive material compilation"
        );

        let mut bytes = vec![0; 5 * 5];
        for y in [2, 3] {
            bytes[y * 5 + 1] = 1;
            bytes[y * 5 + 3] = 2;
        }
        let grid = PixelGrid::new(
            5,
            5,
            bytes,
            vec![0, 25, 25],
            vec![None, Some("Negative".into()), Some("Zero".into())],
            vec![None; 3],
        );
        let mut landscape = Landscape::new(5, vec![5; 5]).expect("landscape builds");
        landscape.set_world_height(5);
        landscape.set_pixel_grid(grid);
        landscape.resolve_grid_materials(|name| materials.id_of(name));

        assert_eq!(
            landscape.extract_material_probe(1, 3, &materials),
            Some((negative, 1, 3)),
            "negative MaxSlide skips even the cslide=0 upward probe"
        );
        assert_eq!(landscape.material_at(1, 2), Some(negative));
        assert_eq!(
            landscape.extract_material_probe(3, 3, &materials),
            Some((zero, 3, 2)),
            "zero MaxSlide still climbs vertically before extraction"
        );
        assert_eq!(landscape.material_at(3, 3), Some(zero));
    }

    #[test]
    fn find_mat_slide_matches_cpp_search_order() {
        // C4Landscape::FindMatSlide (C4Landscape.cpp:1260-1290): straight
        // down first; then per cslide = 1..=mslide check LEFT before RIGHT;
        // a side is clogged only when both (±cslide, fy) and (±cslide, fy+1)
        // are >= mdens; a slide fires when (±cslide, fy+1) is free.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=50
            Friction=20
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mdens = 25;

        // Straight down free → move one down.
        let landscape = Landscape::with_default_material(3, vec![10, 10, 10], Some(earth))
            .expect("landscape builds");
        let (mut fx, mut fy) = (1, 8);
        assert!(landscape.find_mat_slide(&mut fx, &mut fy, 1, mdens, 1, &materials));
        assert_eq!((fx, fy), (1, 9));

        // Down blocked, both sides open → LEFT wins (checked first).
        let landscape = Landscape::with_default_material(3, vec![11, 10, 11], Some(earth))
            .expect("landscape builds");
        let (mut fx, mut fy) = (1, 9);
        assert!(landscape.find_mat_slide(&mut fx, &mut fy, 1, mdens, 1, &materials));
        assert_eq!((fx, fy), (0, 10));

        // Left clogged (both side cells solid) → slides right.
        let landscape = Landscape::with_default_material(3, vec![9, 10, 11], Some(earth))
            .expect("landscape builds");
        let (mut fx, mut fy) = (1, 9);
        assert!(landscape.find_mat_slide(&mut fx, &mut fy, 1, mdens, 1, &materials));
        assert_eq!((fx, fy), (2, 10));

        // Slide target two columns out: found with mslide = 2, not with 1.
        let landscape = Landscape::with_default_material(5, vec![11, 10, 10, 10, 11], Some(earth))
            .expect("landscape builds");
        let (mut fx, mut fy) = (2, 9);
        assert!(!landscape.find_mat_slide(&mut fx, &mut fy, 1, mdens, 1, &materials));
        assert_eq!((fx, fy), (2, 9), "no move on failure");
        assert!(landscape.find_mat_slide(&mut fx, &mut fy, 1, mdens, 2, &materials));
        assert_eq!((fx, fy), (0, 10));

        // Fully enclosed → false.
        let landscape = Landscape::with_default_material(3, vec![9, 10, 9], Some(earth))
            .expect("landscape builds");
        let (mut fx, mut fy) = (1, 9);
        assert!(!landscape.find_mat_slide(&mut fx, &mut fy, 1, mdens, 3, &materials));
    }

    fn find_mat_path_push_landscape(width: u32, height: u32, bytes: Vec<u8>) -> Landscape {
        let grid = PixelGrid::new(
            width,
            height,
            bytes,
            vec![0, 100, 25],
            vec![None, Some("Solid".into()), Some("Flow".into())],
            vec![None; 3],
        );
        let mut landscape = Landscape::new(width, vec![height as i32; width as usize])
            .expect("push-path landscape builds");
        landscape.set_world_height(height as i32);
        landscape.set_pixel_grid(grid);
        landscape.set_border_open(0, 0, false, false);
        landscape
    }

    #[test]
    fn find_mat_path_push_matches_cpp_start_and_ray_order() {
        // Equal-density startpoint selection expands in L/U/R/D order at
        // every radius. Each case keeps the preceding directions equal and
        // makes only the selected neighbor lower-density.
        let origin = (3_i32, 3_i32);
        let neighbors = [(2, 3), (3, 2), (4, 3), (3, 4)];
        for selected in 0..neighbors.len() {
            let mut bytes = vec![1; 7 * 7];
            bytes[origin.1 as usize * 7 + origin.0 as usize] = 2;
            for &(x, y) in &neighbors[..selected] {
                bytes[y as usize * 7 + x as usize] = 2;
            }
            let (exit_x, exit_y) = neighbors[selected];
            bytes[exit_y as usize * 7 + exit_x as usize] = 0;
            let landscape = find_mat_path_push_landscape(7, 7, bytes);
            let (mut x, mut y) = origin;
            assert!(landscape.find_mat_path_push(
                &mut x,
                &mut y,
                25,
                1,
                false,
                &MaterialSet::new(),
            ));
            assert_eq!((x, y), neighbors[selected], "direction case {selected}");
        }

        // The greater-density ray uses the same L/U/R/D priority. Four
        // equal-distance exits therefore return the left one immediately.
        let mut bytes = vec![1; 7 * 7];
        for (x, y) in neighbors {
            bytes[y as usize * 7 + x as usize] = 0;
        }
        let landscape = find_mat_path_push_landscape(7, 7, bytes);
        let (mut x, mut y) = origin;
        assert!(landscape.find_mat_path_push(&mut x, &mut y, 25, 1, false, &MaterialSet::new(),));
        assert_eq!((x, y), (2, 3));
    }

    #[test]
    fn find_mat_path_push_matches_cpp_border_metric_and_strict_ties() {
        // A 3x3 equal-density island has one upper and one lower exit. The
        // stable metric chooses the shorter weighted upper exit; Instable's
        // signed (origin_y-next_y) term instead chooses the lower exit.
        let mut bytes = vec![1; 7 * 7];
        for y in 2..=4 {
            for x in 2..=4 {
                bytes[y * 7 + x] = 2;
            }
        }
        let upper_exit_y = 1;
        bytes[upper_exit_y * 7 + 3] = 0;
        bytes[5 * 7 + 4] = 0;
        let landscape = find_mat_path_push_landscape(7, 7, bytes);
        let mut stable = (3, 3);
        assert!(landscape.find_mat_path_push(
            &mut stable.0,
            &mut stable.1,
            25,
            2,
            false,
            &MaterialSet::new(),
        ));
        assert_eq!(stable, (3, 1));
        let mut instable = (3, 3);
        assert!(landscape.find_mat_path_push(
            &mut instable.0,
            &mut instable.1,
            25,
            2,
            true,
            &MaterialSet::new(),
        ));
        assert_eq!(instable, (4, 5));

        // Best replacement is strict: four equal exits around one material
        // pixel retain the traversal-first left candidate.
        let mut bytes = vec![1; 7 * 7];
        bytes[3 * 7 + 3] = 2;
        for (x, y) in [(2, 3), (3, 2), (4, 3), (3, 4)] {
            bytes[y * 7 + x] = 0;
        }
        let landscape = find_mat_path_push_landscape(7, 7, bytes);
        let mut tied = (3, 3);
        assert!(landscape.find_mat_path_push(
            &mut tied.0,
            &mut tied.1,
            25,
            1,
            false,
            &MaterialSet::new(),
        ));
        assert_eq!(tied, (2, 3));
    }

    #[test]
    fn find_mat_path_push_matches_cpp_range_and_sealed_failure() {
        let sealed = find_mat_path_push_landscape(7, 7, vec![1; 7 * 7]);
        let mut point = (3, 3);
        assert!(!sealed.find_mat_path_push(
            &mut point.0,
            &mut point.1,
            25,
            1,
            false,
            &MaterialSet::new(),
        ));

        // The greater-density ray tests radii 1..499; radius 500 is excluded.
        let mut bytes = vec![1; 1001 * 3];
        let exit_y = 1;
        bytes[exit_y * 1001 + 1] = 0;
        let landscape = find_mat_path_push_landscape(1001, 3, bytes);
        let mut within = (500, 1);
        assert!(landscape.find_mat_path_push(
            &mut within.0,
            &mut within.1,
            25,
            1,
            false,
            &MaterialSet::new(),
        ));
        assert_eq!(within, (1, 1));

        let mut bytes = vec![1; 1001 * 3];
        bytes[exit_y * 1001] = 0;
        let landscape = find_mat_path_push_landscape(1001, 3, bytes);
        let mut at_limit = (500, 1);
        assert!(!landscape.find_mat_path_push(
            &mut at_limit.0,
            &mut at_limit.1,
            25,
            1,
            false,
            &MaterialSet::new(),
        ));
    }

    fn temperature_pixel_landscape(
        width: u32,
        height: u32,
        bytes: Vec<u8>,
        materials: &MaterialSet,
    ) -> Landscape {
        let mut densities = vec![0; 128];
        let mut material_names = vec![None; 128];
        for material in materials.iter() {
            let slot = material.id().index() + 1;
            densities[slot] = material.density();
            material_names[slot] = Some(material.name().to_string());
        }
        let grid = PixelGrid::new(
            width,
            height,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        );
        let mut landscape = Landscape::new(width, vec![height as i32; width as usize])
            .expect("temperature landscape builds");
        landscape.set_world_height(height as i32);
        landscape.set_pixel_grid(grid);
        landscape.resolve_grid_materials(|name| materials.id_of(name));
        landscape.refresh_all_raster_columns();
        landscape
    }

    fn frozen_temperature_pixel_landscape(materials: &MaterialSet) -> Landscape {
        let mut densities = vec![0; 128];
        densities[10] = 80;
        densities[11] = 80;
        densities[40] = 30;
        let mut material_names = vec![None; 128];
        material_names[10] = Some("HotAbove".to_string());
        material_names[11] = Some("ColdBelow".to_string());
        material_names[40] = Some("Target".to_string());
        let mut texture_names = vec![None; 128];
        texture_names[10] = Some("Rough".to_string());
        texture_names[11] = Some("Rough".to_string());
        texture_names[40] = Some("Smooth".to_string());
        let grid = PixelGrid::new(
            2,
            2,
            vec![0, 0, 10 | 0x80, 11],
            densities.clone(),
            material_names.clone(),
            texture_names.clone(),
        );
        let runtime_materials = materials
            .iter()
            .map(|material| RuntimeTexMapMaterial {
                name: material.name().to_string(),
                density: material.density(),
                shape: crate::chunky::ChunkShape::Smooth,
            })
            .collect();
        let texmap = RuntimeTexMapState {
            densities,
            material_names,
            texture_names: texture_names.clone(),
            match_texture_names: texture_names,
            shapes: vec![None; 128],
            materials: runtime_materials,
            texture_inventory: vec!["Rough".to_string(), "Smooth".to_string()],
            default_material_entries: vec![
                ("HotAbove".to_string(), 10),
                ("ColdBelow".to_string(), 11),
                ("Target".to_string(), 40),
            ],
            material_crossmap_entries: vec![40, 40],
            ..Default::default()
        };
        let mut landscape = Landscape::new(2, vec![2; 2]).expect("temperature landscape builds");
        landscape.set_world_height(2);
        landscape.set_pixel_grid(grid);
        landscape.resolve_grid_materials(|name| materials.id_of(name));
        landscape.refresh_all_raster_columns();
        landscape.set_raster_state(LandscapeRasterState::new(1, 0, texmap));
        landscape
    }

    #[test]
    fn temperature_scan_uses_frozen_crossmap_slots_after_lower_index_copy() {
        let library = MaterialLibrary::parse(
            r#"
            [Material HotAbove]
            Name=HotAbove
            Density=80
            AboveTempConvert=-1
            AboveTempConvertDir=0
            AboveTempConvertTo=Target-Smooth
            TempConvStrength=0

            [Material ColdBelow]
            Name=ColdBelow
            Density=80
            BelowTempConvert=1
            BelowTempConvertDir=0
            BelowTempConvertTo=Target-Smooth
            TempConvStrength=0

            [Material Target]
            Name=Target
            Density=30

            [Material Stale]
            Name=Stale
            Density=50
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let hot = materials.id_of("HotAbove").expect("hot source exists");
        let target = materials.id_of("Target").expect("target exists");
        let stale = materials.id_of("Stale").expect("stale material exists");
        let mut baseline = frozen_temperature_pixel_landscape(&materials);
        let mut moved = baseline.clone();
        let mut moved_texmap = moved
            .raster_state()
            .expect("raster state exists")
            .texmap()
            .clone();
        let (success, indices) = moved_texmap.set_texture_index("Target-Smooth", 5, false);
        assert!(success);
        assert_eq!(indices, Some((40, 5)));
        assert!(moved.apply_runtime_texture_index_move(moved_texmap, 40, 5));

        baseline.apply_temperature_conversions(&materials, 0);
        moved.apply_temperature_conversions(&materials, 0);

        assert_eq!(baseline.grid_byte_at(0, 1), Some(40 | 0x80));
        assert_eq!(baseline.grid_byte_at(1, 1), Some(40));
        assert_eq!(moved.grid_byte_at(0, 1), Some(40 | 0x80));
        assert_eq!(moved.grid_byte_at(1, 1), Some(40));
        assert_eq!(moved.material_pixel_count(target, None), 2);
        let texmap = moved.raster_state().expect("raster state exists").texmap();
        assert_eq!(texmap.material_names[5].as_deref(), Some("Target"));
        assert_eq!(texmap.material_names[40].as_deref(), Some("Target"));
        assert_eq!(texmap.material_crossmap_entries, vec![40, 40]);

        let mut stale_cache = frozen_temperature_pixel_landscape(&materials);
        let grid = stale_cache.pixels.as_mut().expect("pixel grid exists");
        grid.material_names[40] = Some("Stale".to_string());
        grid.resolve_materials(|name| materials.id_of(name));
        assert_eq!(grid.material_for_byte(40), Some(stale));
        let action = stale_cache
            .temperature_scan_action(&materials, hot, TemperatureDirection::Downwards, 0)
            .expect("above conversion applies");
        assert_eq!(action.target, target);
        assert_eq!(action.target_byte, 40);
    }

    #[test]
    fn temperature_scan_uses_global_integer_temperature_without_climate_or_height() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=30
            AboveTempConvert=4
            AboveTempConvertDir=1
            AboveTempConvertTo=Ice
            TempConvStrength=3

            [Material Ice]
            Name=Ice
            Density=80
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let ice = materials.id_of("Ice").expect("ice exists");
        let water_byte = (water.index() + 1) as u8;
        let mut bytes = vec![0; 2 * 8];
        for y in 2..=5 {
            for x in 0..2 {
                bytes[y * 2 + x] = water_byte;
            }
        }
        let mut landscape = temperature_pixel_landscape(2, 8, bytes, &materials);
        let environment = EnvironmentSettings::new(0)
            .with_temperature(1)
            .with_climate(20)
            .with_temperature_range(30);

        landscape.apply_temperature_conversions(&materials, environment.temperature);

        assert_eq!(landscape.material_pixel_count(water, None), 8);
        assert_eq!(landscape.material_pixel_count(ice, None), 0);
        assert_eq!(
            landscape.liquid_material_at(0, 5),
            Some(water),
            "the derived column view must not observe the invented warm bottom gradient"
        );
    }

    #[test]
    fn temperature_scan_converts_only_scan_speed_columns_per_frame() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Ice]
            Name=Ice
            Density=80
            AboveTempConvert=0
            AboveTempConvertDir=0
            AboveTempConvertTo=Water
            TempConvStrength=3

            [Material Water]
            Name=Water
            Density=30
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let ice = materials.id_of("Ice").expect("ice exists");
        let water = materials.id_of("Water").expect("water exists");
        let ice_byte = (ice.index() + 1) as u8;
        let water_byte = (water.index() + 1) as u8;
        let mut bytes = vec![0; 6 * 8];
        for y in 2..8 {
            for x in 0..6 {
                bytes[y * 6 + x] = ice_byte;
            }
        }
        let mut landscape = temperature_pixel_landscape(6, 8, bytes, &materials);
        let environment = EnvironmentSettings::new(0)
            .with_temperature(10)
            .with_climate(50)
            .with_temperature_range(100);

        landscape.apply_temperature_conversions(&materials, environment.temperature);

        assert_eq!(landscape.material_pixel_count(water, None), 8);
        for x in 0..2 {
            for y in 2..6 {
                assert_eq!(landscape.grid_byte_at(x, y), Some(water_byte));
            }
        }
        for x in 2..6 {
            assert_eq!(landscape.grid_byte_at(x, 2), Some(ice_byte));
        }
        assert_eq!(landscape.scan_x(), 2);
    }

    #[test]
    fn temperature_scan_preserves_ift_and_strength_zero_converts_one_pixel() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Ice]
            Name=Ice
            Density=80
            AboveTempConvert=0
            AboveTempConvertTo=Water
            TempConvStrength=0

            [Material Water]
            Name=Water
            Density=30
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let ice = materials.id_of("Ice").expect("ice exists");
        let water = materials.id_of("Water").expect("water exists");
        let ice_byte = (ice.index() + 1) as u8;
        let water_byte = (water.index() + 1) as u8;
        let mut bytes = vec![0; 2 * 4];
        for y in 1..4 {
            bytes[y * 2] = ice_byte | 0x80;
            bytes[y * 2 + 1] = ice_byte;
        }
        let mut landscape = temperature_pixel_landscape(2, 4, bytes, &materials);

        landscape.apply_temperature_conversions(&materials, 1);

        assert_eq!(landscape.grid_byte_at(0, 1), Some(water_byte | 0x80));
        assert_eq!(landscape.grid_byte_at(1, 1), Some(water_byte));
        assert_eq!(landscape.grid_byte_at(0, 2), Some(ice_byte | 0x80));
        assert_eq!(landscape.grid_byte_at(1, 2), Some(ice_byte));
    }

    #[test]
    fn temperature_scan_pretty_surface_waits_for_the_left_column() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Ice]
            Name=Ice
            Density=80
            AboveTempConvert=0
            AboveTempConvertTo=Water
            TempConvStrength=0

            [Material Water]
            Name=Water
            Density=30
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let ice = materials.id_of("Ice").expect("ice exists");
        let water = materials.id_of("Water").expect("water exists");
        let ice_byte = (ice.index() + 1) as u8;
        let mut bytes = vec![0; 2 * 4];
        bytes[2] = ice_byte;
        for y in 2..4 {
            bytes[y * 2] = ice_byte;
            bytes[y * 2 + 1] = ice_byte;
        }
        let mut landscape = temperature_pixel_landscape(2, 4, bytes, &materials);

        landscape.apply_temperature_conversions(&materials, 1);

        assert_eq!(landscape.material_pixel_count(water, None), 1);
        assert_eq!(
            landscape.grid_byte_at(1, 2),
            Some(ice_byte),
            "the deeper right-hand surface waits while its left pixel is still Ice"
        );
    }

    #[test]
    fn temperature_scan_cursor_pauses_without_an_eligible_conversion_or_when_disabled() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Ice]
            Name=Ice
            Density=80
            AboveTempConvert=10
            AboveTempConvertTo=Water
            TempConvStrength=3

            [Material Water]
            Name=Water
            Density=30
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let ice = materials.id_of("Ice").expect("ice exists");
        let ice_byte = (ice.index() + 1) as u8;
        let mut bytes = vec![0; 4 * 4];
        for y in 1..4 {
            for x in 0..4 {
                bytes[y * 4 + x] = ice_byte;
            }
        }
        let mut landscape = temperature_pixel_landscape(4, 4, bytes, &materials);

        landscape.apply_temperature_conversions(&materials, 10);
        assert_eq!(landscape.scan_x(), 0, "strict threshold does not scan");

        landscape.set_no_scan(true);
        landscape.apply_temperature_conversions(&materials, 11);
        assert_eq!(landscape.scan_x(), 0, "NoScan freezes the cursor");
        assert_eq!(landscape.material_pixel_count(ice, None), 12);
    }

    #[test]
    fn apply_temperature_conversions_updates_materials() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Ice]
            Name=Ice
            Density=80
            Friction=15
            AboveTempConvert=0
            AboveTempConvertDir=0
            AboveTempConvertTo=Water
            TempConvStrength=4

            [Material Water]
            Name=Water
            Density=60
            Friction=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let ice = materials.id_of("Ice").expect("ice exists");
        let water = materials.id_of("Water").expect("water exists");

        let mut landscape = Landscape::flat_with_material(3, 12, Some(ice));
        let environment = EnvironmentSettings::new(0)
            .with_temperature(10)
            .with_climate(0)
            .with_temperature_range(0);
        landscape.apply_temperature_conversions(&materials, environment.temperature);

        assert_eq!(landscape.solid_material_at(0), Some(water));
        assert_eq!(landscape.solid_material_at(1), Some(water));
        assert_eq!(landscape.default_solid_material(), Some(water));
    }

    #[test]
    fn temperature_conversion_to_sky_removes_surface_pixels() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Frost]
            Name=Frost
            Density=80
            Friction=10
            AboveTempConvert=0
            AboveTempConvertDir=0
            AboveTempConvertTo=Sky
            TempConvStrength=3
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let frost = materials.id_of("Frost").expect("frost exists");

        let mut landscape = Landscape::flat_with_material(2, 8, Some(frost));
        let environment = EnvironmentSettings::new(0)
            .with_temperature(5)
            .with_climate(0)
            .with_temperature_range(0);
        landscape.apply_temperature_conversions(&materials, environment.temperature);

        assert_eq!(landscape.surface(), &[11, 11]);
        assert_eq!(landscape.solid_material_at(0), Some(frost));
    }

    #[test]
    fn temperature_conversion_converts_liquid_segments() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=30
            Friction=0
            AboveTempConvert=0
            AboveTempConvertDir=0
            AboveTempConvertTo=Steam
            TempConvStrength=2

            [Material Steam]
            Name=Steam
            Density=5
            Friction=1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let steam = materials.id_of("Steam").expect("steam exists");

        let mut landscape = Landscape::flat(1, 20);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(0, vec![LiquidSegment::new(4, 7)]);

        let environment = EnvironmentSettings::new(0)
            .with_temperature(5)
            .with_climate(0)
            .with_temperature_range(0);
        landscape.apply_temperature_conversions(&materials, environment.temperature);

        let column = &landscape.liquids()[0];
        assert_eq!(
            column.segments(),
            &[
                LiquidSegment::with_material(4, 5, Some(steam)),
                LiquidSegment::new(6, 7)
            ]
        );
    }

    #[test]
    fn temperature_conversion_evaporates_liquid_segments() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Steam]
            Name=Steam
            Density=5
            Friction=1
            BelowTempConvert=10
            BelowTempConvertDir=1
            BelowTempConvertTo=Sky
            TempConvStrength=1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let steam = materials.id_of("Steam").expect("steam exists");

        let mut landscape = Landscape::flat(1, 20);
        landscape.set_default_liquid_material(Some(steam));
        landscape.set_liquid_column(0, vec![LiquidSegment::new(3, 5)]);

        let environment = EnvironmentSettings::new(0)
            .with_temperature(0)
            .with_climate(0)
            .with_temperature_range(0);
        landscape.apply_temperature_conversions(&materials, environment.temperature);

        let column = &landscape.liquids()[0];
        assert_eq!(column.segments(), &[LiquidSegment::new(3, 4)]);
    }

    #[test]
    fn climate_zone_gradient_does_not_affect_material_conversion() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Ice]
            Name=Ice
            Density=80
            Friction=15
            AboveTempConvert=0
            AboveTempConvertDir=0
            AboveTempConvertTo=Water
            TempConvStrength=4

            [Material Water]
            Name=Water
            Density=60
            Friction=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let ice = materials.id_of("Ice").expect("ice exists");

        let mut landscape = Landscape::flat_with_material(2, 20, Some(ice));
        landscape.set_height(0, 5);
        landscape.set_height(1, 100);

        let environment = EnvironmentSettings::new(0)
            .with_temperature(0)
            .with_climate(20)
            .with_temperature_range(40);

        landscape.apply_temperature_conversions(&materials, environment.temperature);

        assert_eq!(landscape.solid_material_at(0), Some(ice));
        assert_eq!(landscape.solid_material_at(1), Some(ice));
    }

    #[test]
    fn blast_circle_raises_surface_for_blastable_materials() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            BlastFree=1
            Inflammable=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut landscape = Landscape::flat_with_material(11, 40, Some(earth));
        let center = Vector2::new(5, 40);
        let radius = 3;
        let before = landscape.surface().to_vec();

        let result = landscape.blast_circle(center, radius, &materials);

        let expected_surface = legacy_expected_surface(&before, center, radius);
        assert_eq!(landscape.surface(), expected_surface.as_slice());
        let mut expected_removed = 0;
        for (after, prior) in expected_surface.iter().zip(&before) {
            expected_removed += (*after - *prior).max(0);
        }

        assert_eq!(
            result
                .removed_by_material
                .get(&earth)
                .copied()
                .unwrap_or_default(),
            expected_removed
        );
        assert!(result
            .affected_columns
            .iter()
            .all(|(column, height)| *height == landscape.surface()[*column as usize]));
    }

    #[test]
    fn blast_circle_does_not_change_non_blastable_materials() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Steel]
            Name=Steel
            Density=120
            Friction=60
            BlastFree=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let steel = materials.id_of("Steel").expect("steel exists");

        let mut landscape = Landscape::flat_with_material(7, 50, Some(steel));
        let before = landscape.surface().to_vec();
        let result = landscape.blast_circle(Vector2::new(3, 50), 4, &materials);

        assert_eq!(landscape.surface(), before.as_slice());
        assert!(result.removed_by_material.is_empty());
        assert!(result.affected_columns.is_empty());
    }

    #[test]
    fn can_incinerate_respects_inflammable_materials() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Wood]
            Name=Wood
            Density=90
            Friction=15
            BlastFree=1
            Inflammable=-50

            [Material Stone]
            Name=Stone
            Density=120
            Friction=50
            BlastFree=0
            Inflammable=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let wood = materials.id_of("Wood").expect("wood exists");
        let stone = materials.id_of("Stone").expect("stone exists");

        let mut landscape_flammable = Landscape::flat_with_material(5, 60, Some(wood));
        landscape_flammable.set_world_height(100);
        assert!(landscape_flammable.can_incinerate(2, 65, &materials));
        assert!(!landscape_flammable.can_incinerate(2, 55, &materials));

        let mut landscape_non_flammable = Landscape::flat_with_material(5, 60, Some(stone));
        landscape_non_flammable.set_world_height(100);
        assert!(!landscape_non_flammable.can_incinerate(2, 65, &materials));
    }

    #[test]
    fn can_incinerate_reads_the_exact_raster_material_like_cpp() {
        // C++ oracle: C4Landscape::Incinerate calls GetMat(x, y) and tests
        // that exact material's Inflammable field (src/C4Landscape.cpp:
        // 1417-1426). It does not substitute the column's surface material.
        let library = MaterialLibrary::parse(
            r#"
            [Material Stone]
            Name=Stone
            Density=120
            Inflammable=0

            [Material Oil]
            Name=Oil
            Density=100
            Inflammable=50
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let stone = materials.id_of("Stone").expect("stone exists");

        let mut densities = vec![0; 128];
        densities[1] = 120;
        densities[2] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Stone".to_string());
        material_names[2] = Some("Oil".to_string());
        let grid = PixelGrid::new(
            3,
            3,
            vec![1, 1, 1, 1, 2, 1, 1, 1, 1],
            densities,
            material_names,
            vec![None; 128],
        );
        let mut landscape = Landscape::flat_with_material(3, 3, Some(stone));
        landscape.set_pixel_grid(grid);
        landscape.resolve_grid_materials(|name| materials.id_of(name));

        assert!(
            landscape.can_incinerate(1, 1, &materials),
            "the exact Oil pixel is inflammable even above the column surface"
        );
        assert!(
            !landscape.can_incinerate(0, 1, &materials),
            "the adjacent exact Stone pixel is not inflammable"
        );
    }

    #[test]
    fn blast_circle_precounts_in_circle_pixels_per_material_like_cpp() {
        // C4Landscape::BlastFree counts every valid in-circle material
        // pixel BEFORE removal (C4Landscape.cpp:1048-1055) — including
        // materials that neither BlastFree nor BlastShiftTo — and the
        // cast amounts derive from that count (:1066-1079). Circle loop:
        // ycnt -rad..=rad, lwdt = sqrt(rad²-ycnt²), xcnt -lwdt..lwdt with
        // the lwdt==0 single-pixel extension.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            BlastFree=1

            [Material Granite]
            Name=Granite
            Density=150
            Friction=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let granite = materials.id_of("Granite").expect("granite exists");

        let mut landscape =
            Landscape::with_default_material(17, vec![40; 17], Some(earth)).expect("builds");
        landscape.set_world_height(80);
        for column in 9..17 {
            landscape.set_solid_material(column, Some(granite));
        }

        let result = landscape.blast_circle(Vector2::new(8, 40), 4, &materials);
        // Solid half of the r=4 circle at the surface row 40: earth
        // pixels x∈4..=8 (rows 40..44), granite x∈9..=11.
        assert_eq!(result.pixel_count_by_material.get(&earth), Some(&17));
        assert_eq!(result.pixel_count_by_material.get(&granite), Some(&8));
        // Granite neither BlastFrees nor shifts: nothing removed.
        assert_eq!(result.removed_by_material.get(&granite), None);
    }

    fn vehicle_earth_materials() -> (MaterialSet, MaterialId, MaterialId) {
        let library = MaterialLibrary::parse(
            r#"
            [Material Vehicle]
            Name=Vehicle
            Density=100
            Friction=100

            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let vehicle = materials.id_of("Vehicle").expect("vehicle exists");
        let earth = materials.id_of("Earth").expect("earth exists");
        (materials, vehicle, earth)
    }

    #[test]
    fn dig_free_pix_uses_get_pix_border_material() {
        let (materials, vehicle, _earth) = vehicle_earth_materials();
        let grid = PixelGrid::new(3, 2, vec![0; 6], vec![0], vec![None], vec![None]);
        let mut landscape = Landscape::new(3, vec![0; 3]).expect("landscape builds");
        landscape.set_world_height(2);
        landscape.set_pixel_grid(grid);
        landscape.set_vehicle_material(Some(vehicle));
        let before: Vec<_> = (0..2)
            .flat_map(|y| (0..3).map(move |x| (x, y)))
            .map(|(x, y)| landscape.pixel_grid().unwrap().byte_at(x, y))
            .collect();

        assert_eq!(landscape.dig_free_pix(1, 2, &materials), Some(vehicle));
        assert_eq!(landscape.dig_free_pix(-1, 0, &materials), Some(vehicle));
        assert_eq!(landscape.dig_free_pix(1, -1, &materials), None);

        landscape.set_border_open(3, 3, true, true);
        assert_eq!(landscape.dig_free_pix(1, 2, &materials), None);
        assert_eq!(landscape.dig_free_pix(-1, 2, &materials), None);
        let after: Vec<_> = (0..2)
            .flat_map(|y| (0..3).map(move |x| (x, y)))
            .map(|(x, y)| landscape.pixel_grid().unwrap().byte_at(x, y))
            .collect();
        assert_eq!(after, before, "border probes never write raster pixels");
    }

    #[test]
    fn dig_free_pix_keeps_exact_in_bounds_pix2mat_lookup() {
        let (materials, _vehicle, earth) = vehicle_earth_materials();
        let grid = PixelGrid::new(
            1,
            1,
            vec![1],
            vec![0, 100],
            vec![None, None],
            vec![None, None],
        );
        let mut landscape = Landscape::flat_with_material(1, 0, Some(earth));
        landscape.set_world_height(1);
        landscape.set_pixel_grid(grid);

        assert_eq!(
            landscape.material_at(0, 0),
            Some(earth),
            "the general lookup may use the column approximation"
        );
        assert_eq!(
            landscape.dig_free_pix(0, 0, &materials),
            None,
            "DigFreePix must keep the unresolved Pix2Mat result"
        );
    }

    #[test]
    fn border_material_reads_vehicle_at_closed_edges_like_get_pix() {
        // C4Landscape::GetPix border rules (C4Landscape.h:144-161) through
        // Pix2Mat (GetMat, C4Landscape.h:173-176): a closed border reads
        // MCVehic, which GBackMat maps to the Vehicle material; an open
        // border reads pix 0 (sky → MNone). Scenario defaults are
        // TopOpen=1, BottomOpen=0, LeftOpen=RightOpen=0
        // (C4Scenario.cpp:295-296).
        let (_materials, vehicle, earth) = vehicle_earth_materials();
        let mut landscape = Landscape::flat_with_material(10, 5, Some(earth));
        landscape.set_world_height(20);
        landscape.set_vehicle_material(Some(vehicle));

        // In bounds: the normal material lookup answers.
        assert_eq!(landscape.border_material_at(4, 10), Some(earth));
        assert_eq!(landscape.border_material_at(4, 2), None, "sky above");
        // Sides closed (LeftOpen/RightOpen = 0).
        assert_eq!(landscape.border_material_at(-1, 10), Some(vehicle));
        assert_eq!(landscape.border_material_at(10, 10), Some(vehicle));
        // Top open by default.
        assert_eq!(landscape.border_material_at(4, -1), None);
        // Bottom closed by default.
        assert_eq!(landscape.border_material_at(4, 20), Some(vehicle));
    }

    #[test]
    fn border_material_honours_open_flags_and_cpp_branch_order() {
        // GetPix checks x before y (C4Landscape.h:148-159): beyond a side,
        // LeftOpen/RightOpen are y thresholds (`y < LeftOpen` reads sky)
        // and the Top/BottomOpen flags never apply.
        let (_materials, vehicle, earth) = vehicle_earth_materials();
        let mut landscape = Landscape::flat_with_material(10, 5, Some(earth));
        landscape.set_world_height(20);
        landscape.set_vehicle_material(Some(vehicle));
        landscape.set_border_open(8, 12, false, true);

        // Left side: open above y=8, closed from there down.
        assert_eq!(landscape.border_material_at(-1, 7), None);
        assert_eq!(landscape.border_material_at(-1, 8), Some(vehicle));
        // Right side: open above y=12.
        assert_eq!(landscape.border_material_at(10, 11), None);
        assert_eq!(landscape.border_material_at(10, 12), Some(vehicle));
        // x precedence: (-1,-5) takes the LEFT branch (y < LeftOpen →
        // sky), even though the top is closed.
        assert_eq!(landscape.border_material_at(-1, -5), None);
        // Top closed, bottom open.
        assert_eq!(landscape.border_material_at(4, -1), Some(vehicle));
        assert_eq!(landscape.border_material_at(4, 20), None);
    }

    #[test]
    fn density_at_follows_the_same_border_rules() {
        // GBackDensity = Pix2Dens[GetPix] (C4Wrappers.h:169-172): the same
        // border pixel that GetMat maps to Vehicle reads as vehicle-solid
        // density; open borders read sky density 0.
        let (materials, vehicle, earth) = vehicle_earth_materials();
        let mut landscape = Landscape::flat_with_material(10, 5, Some(earth));
        landscape.set_world_height(20);
        landscape.set_vehicle_material(Some(vehicle));
        landscape.set_border_open(8, 0, false, true);

        assert_eq!(landscape.density_at(-1, 7, &materials), 0, "left open");
        assert_eq!(landscape.density_at(-1, 8, &materials), 100, "left shut");
        assert_eq!(landscape.density_at(4, -1, &materials), 100, "top shut");
        assert_eq!(landscape.density_at(4, 20, &materials), 0, "bottom open");
        // Defaults keep the previous behavior: side/bottom vehicle, top sky.
        let default = Landscape::flat_with_material(10, 5, Some(earth));
        assert_eq!(default.density_at(-1, 2, &materials), 100);
        assert_eq!(default.density_at(4, -1, &materials), 0);
        assert_eq!(default.density_at(4, 5, &materials), 100, "in-ground");
    }

    #[test]
    fn solidity_queries_honour_configured_open_borders_like_cpp() {
        // C++ oracle: GBackSolid is DensitySolid(GetDensity), and GetDensity
        // maps through GetPix's configured borders in x-before-y order
        // (src/C4Wrappers.h:169-177; src/C4Landscape.h:144-180).
        let (_materials, _vehicle, earth) = vehicle_earth_materials();
        let mut landscape = Landscape::flat_with_material(10, 5, Some(earth));
        landscape.set_world_height(20);
        landscape.set_border_open(8, 12, false, true);

        assert!(!landscape.is_solid_at(-1, 7), "left side is open above y=8");
        assert!(landscape.is_solid_at(-1, 8), "left side closes at y=8");
        assert!(
            !landscape.is_solid_at(10, 11),
            "right side is open above y=12"
        );
        assert!(landscape.is_solid_at(10, 12), "right side closes at y=12");
        assert!(
            !landscape.is_semi_solid_at(10, 11),
            "right side is open above y=12"
        );
        assert!(
            landscape.is_semi_solid_at(10, 12),
            "right side closes at y=12"
        );
        assert!(landscape.is_solid_at(4, -1), "configured top is closed");
        assert!(!landscape.is_solid_at(4, 20), "configured bottom is open");
        assert!(
            !landscape.is_solid_at(-1, -5),
            "x border takes precedence over the closed top"
        );
    }

    #[test]
    fn scan_side_open_finds_first_non_sky_pixel_like_cpp() {
        // C4Landscape::ScanSideOpen (C4Landscape.cpp:231-238): LeftOpen /
        // RightOpen become the first y with a non-zero pixel in columns 0
        // and Width-1 (AutoScanSideOpen, C4Landscape::ScenarioInit
        // C4Landscape.cpp:72-73).
        let (_materials, _vehicle, earth) = vehicle_earth_materials();
        let mut landscape =
            Landscape::with_default_material(3, vec![6, 9, 4], Some(earth)).expect("builds");
        landscape.set_world_height(12);
        landscape.scan_side_open();
        assert_eq!(landscape.left_open(), 6, "column 0 solid from y=6");
        assert_eq!(landscape.right_open(), 4, "column 2 solid from y=4");
    }
}
