use std::collections::HashMap;
use std::convert::TryFrom;
use std::mem;

use crate::material::TemperatureDirection;
use crate::{EnvironmentSettings, MaterialId, MaterialSet, Vector2};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

const C4M_VEHICLE: i32 = 100;
const C4M_BACKGROUND: i32 = 0;
/// DensitySolid threshold (C4M_Solid, C4Material.h:200).
const C4M_SOLID: i32 = 50;
/// DensityLiquid lower bound (C4M_Liquid, C4Material.h:203).
const C4M_LIQUID: i32 = 25;

/// Hex-string serde for the pixel byte plane (a JSON number array would be
/// ~10MB for a real map; hex keeps state exports tractable).
mod hex_bytes {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            text.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble"));
            text.push(char::from_digit((byte & 0xf) as u32, 16).expect("nibble"));
        }
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() % 2 != 0 {
            return Err(D::Error::custom("odd hex length"));
        }
        text.as_bytes()
            .chunks(2)
            .map(|pair| {
                let high = (pair[0] as char).to_digit(16);
                let low = (pair[1] as char).to_digit(16);
                match (high, low) {
                    (Some(high), Some(low)) => Ok(((high << 4) | low) as u8),
                    _ => Err(D::Error::custom("invalid hex digit")),
                }
            })
            .collect()
    }
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
    /// Row-major texmap-index bytes.
    #[serde(with = "hex_bytes")]
    bytes: Vec<u8>,
    /// TEXTURE name per texmap index (presentation only: the frontend
    /// samples the texture png per pixel).
    #[serde(default)]
    texture_names: Vec<Option<String>>,
    /// Bumped on every pixel write — the frontend's render cache key.
    #[serde(default)]
    revision: u64,
    /// Pix2Dens: density per texmap index (IFT stripped); index 0 and
    /// unmapped entries are sky (density 0).
    densities: Vec<i32>,
    /// Material NAME per texmap index — resolved into [`Self::materials`]
    /// once the engine's `MaterialSet` exists (`Engine::set_landscape`).
    material_names: Vec<Option<String>>,
    /// Pix2Mat: the engine `MaterialId` per texmap index.
    #[serde(default)]
    materials: Vec<Option<MaterialId>>,
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
        Self {
            width,
            height,
            bytes,
            densities,
            material_names,
            materials,
            texture_names,
            revision: 0,
        }
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
        &self.bytes
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
                    .is_some_and(|name| name.eq_ignore_ascii_case("Vehicle"))
            })
            .map(|index| index as u8)
    }

    /// Raw plane write (C4SolidMask's _SBackPix): bumps the revision.
    pub fn write_byte(&mut self, x: i32, y: i32, byte: u8) {
        self.set_byte(x, y, byte);
    }

    pub fn material_names(&self) -> &[Option<String>] {
        &self.material_names
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn byte_at(&self, x: i32, y: i32) -> Option<u8> {
        self.slot(x, y).map(|slot| self.bytes[slot])
    }

    fn density_of(&self, byte: u8) -> i32 {
        self.densities
            .get((byte & 0x7f) as usize)
            .copied()
            .unwrap_or(0)
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
    }

    /// The first texmap index carrying the given material (the
    /// Mat2PixColDefault stand-in for grid writes).
    fn byte_for_material(&self, material: MaterialId) -> Option<u8> {
        self.materials
            .iter()
            .position(|slot| *slot == Some(material))
            .map(|index| index as u8)
    }

    /// Any solid texmap byte (fallback fill when no material is known).
    fn any_solid_byte(&self) -> Option<u8> {
        self.densities
            .iter()
            .position(|&density| density >= C4M_SOLID)
            .map(|index| index as u8)
    }

    fn set_byte(&mut self, x: i32, y: i32, byte: u8) {
        if let Some(slot) = self.slot(x, y) {
            self.bytes[slot] = byte;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn clear_pixel(&mut self, x: i32, y: i32) {
        self.set_byte(x, y, 0);
    }

    /// The default tunnel byte (`Mat2PixColDefault(MTunnel)`): the first
    /// texmap slot named Tunnel.
    fn tunnel_byte(&self) -> u8 {
        self.material_names
            .iter()
            .position(|name| {
                name.as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Tunnel"))
            })
            .map(|index| index as u8)
            .unwrap_or(0)
    }

    /// C4Landscape::ClearPix (C4Landscape.cpp:880-888): an IFT pixel
    /// clears to the tunnel background (+IFT); a surface pixel to sky.
    pub fn clear_pix(&mut self, x: i32, y: i32) {
        let Some(byte) = self.byte_at(x, y) else {
            return;
        };
        if byte & 0x80 != 0 {
            self.set_byte(x, y, self.tunnel_byte() | 0x80);
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
}

#[derive(Debug, Error)]
pub enum LandscapeError {
    #[error("height map length {found} does not match width {width}")]
    InvalidHeightMap { width: u32, found: usize },
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Landscape {
    width: u32,
    surface: Vec<i32>,
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
    /// The per-pixel plane (C4Landscape Surface8). When present it is the
    /// TRUTH for solidity/material queries; the column model above stays
    /// maintained as the approximation legacy helpers consume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pixels: Option<PixelGrid>,
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
}

fn default_top_open() -> bool {
    true
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

    pub(crate) fn with_default_material(
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
            liquids: vec![LiquidColumn::default(); size],
            solid_materials: vec![default_material; size],
            default_solid_material: default_material,
            default_liquid_material: None,
            tunnels: HashMap::new(),
            world_height: None,
            pixels: None,
            left_open: 0,
            right_open: 0,
            top_open: true,
            bottom_open: false,
            vehicle_material: None,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
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
        self.pixels = Some(grid);
    }

    pub fn pixel_grid(&self) -> Option<&PixelGrid> {
        self.pixels.as_ref()
    }

    /// Resolve the grid's Pix2Mat table once the engine materials exist
    /// (UpdatePixMaps, C4Landscape.cpp:2832-2839).
    /// C4Landscape::DigFreePix (C4Landscape.cpp:936-944) on the pixel
    /// grid: returns the material at the pixel (C++ returns it even when
    /// nothing clears); clears only DigFree materials. `None` when no
    /// grid exists — callers keep the column-model fallback.
    pub fn dig_free_pix(&mut self, x: i32, y: i32, materials: &MaterialSet) -> Option<MaterialId> {
        self.pixels.as_ref()?;
        let material_id = self
            .pixels
            .as_ref()
            .and_then(|grid| grid.material_id_at(x, y));
        if let Some(id) = material_id {
            if materials
                .get_by_id(id)
                .map(|material| material.dig_free())
                .unwrap_or(false)
            {
                if let Some(grid) = self.pixels.as_mut() {
                    grid.clear_pix(x, y);
                }
            }
        }
        material_id
    }

    /// ClearPix on the grid (no diggable gate) — C4Landscape::ClearRect's
    /// per-pixel body (C4Landscape.cpp:2184-2194).
    pub fn clear_pix(&mut self, x: i32, y: i32) -> bool {
        match self.pixels.as_mut() {
            Some(grid) => {
                grid.clear_pix(x, y);
                true
            }
            None => false,
        }
    }

    /// The InsertMaterial dead-material write (C4Landscape.cpp:1218):
    /// `SetPix(tx, ty, Mat2PixColDefault(mat) + GBackIFT(tx, ty))` — the
    /// material byte keeps the CURRENT pixel's IFT bit.
    pub fn insert_material_pix(&mut self, x: i32, y: i32, material: MaterialId) -> bool {
        let Some(grid) = self.pixels.as_mut() else {
            return false;
        };
        let Some(byte) = grid.byte_for_material(material) else {
            return false;
        };
        let Some(current) = grid.byte_at(x, y) else {
            return false;
        };
        grid.write_byte(x, y, byte | (current & 0x80));
        true
    }

    /// C4SolidMask plumbing: the plane's Vehicle byte, raw reads and
    /// writes. None/no-op without a pixel grid (fixture worlds keep the
    /// mask-rect overlay).
    pub fn grid_vehicle_byte(&self) -> Option<u8> {
        self.pixels.as_ref().and_then(|grid| grid.vehicle_byte())
    }

    pub fn grid_byte_at(&self, x: i32, y: i32) -> Option<u8> {
        self.pixels.as_ref().and_then(|grid| grid.byte_at(x, y))
    }

    pub fn grid_write_byte(&mut self, x: i32, y: i32, byte: u8) {
        if let Some(grid) = self.pixels.as_mut() {
            grid.write_byte(x, y, byte);
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
        self.ensure_liquid_capacity();
        if let Some(column) = self.liquids.get_mut(x as usize) {
            *column = LiquidColumn::from_segments(segments);
        }
    }

    pub fn clear_liquid_column(&mut self, x: u32) {
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

    pub fn estimated_height(&self) -> i32 {
        self.world_height
            .unwrap_or_else(|| self.estimate_world_height())
    }

    /// Pin the real landscape height (`GBackHgt`). The legacy loader knows
    /// it exactly (map height × zoom); without it the estimate from surface
    /// depths is used.
    pub fn set_world_height(&mut self, height: i32) {
        self.world_height = Some(height.max(0));
    }

    pub fn apply_temperature_conversions(
        &mut self,
        materials: &MaterialSet,
        environment: &EnvironmentSettings,
        frame: u64,
    ) {
        if self.surface.is_empty() || materials.is_empty() {
            return;
        }
        self.ensure_material_capacity();
        self.ensure_liquid_capacity();
        let world_height = self.estimate_world_height();
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
            let surface_height = self.surface[index];
            let downward_temperature =
                environment.temperature_at_height(frame, surface_height, world_height);
            let upward_temperature = environment.temperature_at_height(
                frame,
                surface_height.saturating_add(1),
                world_height,
            );
            column_actions[index] = find_column_conversion(
                materials,
                material_id,
                downward_temperature,
                upward_temperature,
            );
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

        let temperature_lookup = |y: i32| environment.temperature_at_height(frame, y, world_height);
        for column in &mut self.liquids {
            let _ = column.apply_temperature_conversions(
                materials,
                self.default_liquid_material,
                &temperature_lookup,
            );
        }
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
    pub fn set_border_open(&mut self, left_open: i32, right_open: i32, top_open: bool, bottom_open: bool) {
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
            .unwrap_or(0)
            .max(0);
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
        if radius <= 0 || self.surface.is_empty() || materials.is_empty() {
            return result;
        }

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
                    *result.pixel_count_by_material.entry(material_id).or_insert(0) += 1;
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
                if let Some(target) = material.blast_shift_to_target() {
                    if target != material_id {
                        result.shift_candidates.push(BlastShiftCandidate {
                            column,
                            material: material_id,
                            target,
                            pixel_count: removed_height,
                        });
                    }
                }
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
        if self.surface.is_empty() || materials.is_empty() {
            return false;
        }

        let Some(surface_y) = self.surface_height(x) else {
            return false;
        };
        if y < surface_y {
            return false;
        }

        let Some(material_id) = self.solid_material_at(x) else {
            return false;
        };
        let Some(material) = materials.get_by_id(material_id) else {
            return false;
        };

        material.inflammable() > 0
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
        // The C++ GetPix border rules (C4Landscape.h:144-161, defaults from
        // C4SLandscape::Default): the top is open, the sides and bottom are
        // closed (MCVehic — solid). Without these, script loops that walk
        // until solid never terminate outside the landscape.
        if y < 0 {
            return false;
        }
        if x < 0 || x as u32 >= self.width {
            return true;
        }
        if y >= self.estimated_height() {
            return true;
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
    /// C4Landscape.cpp:2894-2908). Computed on demand from the column model
    /// instead of C++'s incrementally-maintained counter cache.
    fn pix_cnt_cell_occupied(&self, cell_x: i32, cell_y: i32, materials: &MaterialSet) -> bool {
        let top = cell_y * 15;
        let bottom = top + 15;
        let left = (cell_x * 17).max(0);
        let right = (cell_x * 17 + 17).min(self.width as i32);
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
        let fill = self
            .pixels
            .as_ref()
            .and_then(|grid| grid.byte_for_material(material));
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
        if self.pixels.is_some() {
            let material = self.material_at(x, y)?;
            let (top_x, top_y) = self.find_mat_top(material, x, y, materials);
            self.clear_pix(top_x, top_y);
            return Some((material, top_x, top_y));
        }
        self.extract_material_at(x, y)
            .map(|material| (material, x, y))
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

    pub fn incinerate_at(&mut self, x: i32, y: i32, materials: &MaterialSet) -> bool {
        if !self.can_incinerate(x, y, materials) {
            return false;
        }
        true
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

    pub fn resolve_collision(&self, position: Vector2, velocity: Vector2) -> CollisionResolution {
        // With the pixel plane there is NO surface snap at all: C++ has no
        // such mechanism — per-pixel movement contact governs motion, and
        // embedded objects simply sit (the snap is the column model's
        // stand-in for fixtures without real terrain).
        if self.pixels.is_some() {
            return CollisionResolution {
                position,
                velocity,
                collided: false,
                material: None,
            };
        }
        // Liquid is not ground: GBackLiquid pixels (density 25..50) never
        // eject an object to the column surface — classified maps carry
        // cave rivers BELOW the surface scalar (the surface snap itself is
        // a column-model stand-in; C++ resolves contact per pixel).
        if self.is_liquid_at(position.x, position.y) {
            return CollisionResolution {
                position,
                velocity,
                collided: false,
                material: None,
            };
        }
        match self.surface_height(position.x) {
            Some(surface_y) if position.y > surface_y => {
                let mut new_position = position;
                let mut new_velocity = velocity;
                new_position.y = surface_y;
                if new_velocity.y > 0 {
                    new_velocity.y = 0;
                }
                let material = self.solid_material_at(position.x);
                CollisionResolution {
                    position: new_position,
                    velocity: new_velocity,
                    collided: true,
                    material,
                }
            }
            _ => CollisionResolution {
                position,
                velocity,
                collided: false,
                material: None,
            },
        }
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
            left_open: i32,
            #[serde(default)]
            right_open: i32,
            #[serde(default = "default_top_open")]
            top_open: bool,
            #[serde(default)]
            bottom_open: bool,
            #[serde(default)]
            vehicle_material: Option<MaterialId>,
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
        landscape.liquids = data.liquids;
        landscape.solid_materials = data.solid_materials;
        landscape.default_liquid_material = data.default_liquid_material;
        landscape.tunnels = data.tunnels;
        landscape.world_height = data.world_height;
        landscape.pixels = data.pixels;
        landscape.left_open = data.left_open;
        landscape.right_open = data.right_open;
        landscape.top_open = data.top_open;
        landscape.bottom_open = data.bottom_open;
        landscape.vehicle_material = data.vehicle_material;
        Ok(landscape)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionResolution {
    pub position: Vector2,
    pub velocity: Vector2,
    pub collided: bool,
    pub material: Option<MaterialId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_resources::MaterialLibrary;

    /// A 100-wide world with ground from y=50 down and an explicit world
    /// height (GBackHgt) of 400, mirroring a real zoomed map landscape.
    fn flat_world() -> Landscape {
        let mut landscape = Landscape::flat(100, 50);
        landscape.set_world_height(400);
        landscape
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
    fn resolves_vertical_collision() {
        let landscape = Landscape::flat(10, 5);
        let position = Vector2::new(3, 8);
        let velocity = Vector2::new(0, 3);
        let resolution = landscape.resolve_collision(position, velocity);
        assert!(resolution.collided);
        assert_eq!(resolution.position, Vector2::new(3, 5));
        assert_eq!(resolution.velocity, Vector2::new(0, 0));
        assert_eq!(resolution.material, None);
    }

    #[test]
    fn ignores_points_above_surface() {
        let landscape = Landscape::flat(10, 5);
        let position = Vector2::new(3, 2);
        let velocity = Vector2::new(0, -1);
        let resolution = landscape.resolve_collision(position, velocity);
        assert!(!resolution.collided);
        assert_eq!(resolution.position, position);
        assert_eq!(resolution.velocity, velocity);
        assert_eq!(resolution.material, None);
    }

    #[test]
    fn resolves_material_with_default() {
        let material = MaterialId::new(0).expect("material id");
        let landscape = Landscape::flat_with_material(8, 12, Some(material));
        let position = Vector2::new(2, 20);
        let velocity = Vector2::new(0, 6);
        let resolution = landscape.resolve_collision(position, velocity);
        assert!(resolution.collided);
        assert_eq!(resolution.material, Some(material));
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
        landscape.apply_temperature_conversions(&materials, &environment, 0);

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
        landscape.apply_temperature_conversions(&materials, &environment, 0);

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
        landscape.apply_temperature_conversions(&materials, &environment, 0);

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
        landscape.apply_temperature_conversions(&materials, &environment, 0);

        let column = &landscape.liquids()[0];
        assert_eq!(column.segments(), &[LiquidSegment::new(3, 4)]);
    }

    #[test]
    fn climate_zone_gradient_affects_material_conversion() {
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

        let mut landscape = Landscape::flat_with_material(2, 20, Some(ice));
        landscape.set_height(0, 5);
        landscape.set_height(1, 100);

        let environment = EnvironmentSettings::new(0)
            .with_temperature(0)
            .with_climate(0)
            .with_temperature_range(40);

        landscape.apply_temperature_conversions(&materials, &environment, 0);

        assert_eq!(landscape.solid_material_at(0), Some(ice));
        assert_eq!(landscape.solid_material_at(1), Some(water));
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
            Inflammable=50

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

        let landscape_flammable = Landscape::flat_with_material(5, 60, Some(wood));
        assert!(landscape_flammable.can_incinerate(2, 65, &materials));
        assert!(!landscape_flammable.can_incinerate(2, 55, &materials));

        let landscape_non_flammable = Landscape::flat_with_material(5, 60, Some(stone));
        assert!(!landscape_non_flammable.can_incinerate(2, 65, &materials));
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
