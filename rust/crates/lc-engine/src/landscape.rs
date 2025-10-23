use std::convert::TryFrom;
use std::mem;

use crate::material::TemperatureDirection;
use crate::{MaterialId, MaterialSet, Vector2};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

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

    fn with_default_material(
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
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn surface(&self) -> &[i32] {
        &self.surface
    }

    pub fn liquids(&self) -> &[LiquidColumn] {
        &self.liquids
    }

    pub fn set_height(&mut self, x: u32, height: i32) {
        if let Some(slot) = self.surface.get_mut(x as usize) {
            *slot = height;
        }
    }

    pub fn set_liquid_column(&mut self, x: u32, segments: Vec<LiquidSegment>) {
        if self.liquids.len() != self.surface.len() {
            self.liquids
                .resize(self.surface.len(), LiquidColumn::default());
        }
        if let Some(column) = self.liquids.get_mut(x as usize) {
            *column = LiquidColumn::from_segments(segments);
        }
    }

    pub fn clear_liquid_column(&mut self, x: u32) {
        if let Some(column) = self.liquids.get_mut(x as usize) {
            column.clear();
        }
    }

    pub fn set_solid_material(&mut self, column: u32, material: Option<MaterialId>) {
        self.ensure_material_capacity();
        if let Some(slot) = self.solid_materials.get_mut(column as usize) {
            *slot = material;
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

    pub fn apply_temperature_conversions(
        &mut self,
        materials: &MaterialSet,
        ambient_temperature: i32,
    ) {
        if self.surface.is_empty() || materials.is_empty() {
            return;
        }
        self.ensure_material_capacity();
        let original_default = self.default_solid_material;

        let evaluate_conversion = |material_id: MaterialId| -> Option<MaterialId> {
            for direction in [
                TemperatureDirection::Downwards,
                TemperatureDirection::Upwards,
            ] {
                if let Some(outcome) = materials.evaluate_temperature_conversion(
                    material_id,
                    direction,
                    ambient_temperature,
                ) {
                    if let crate::material::TemperatureTarget::Material(target_id) = outcome.target
                    {
                        if target_id != material_id {
                            return Some(target_id);
                        }
                    }
                }
            }
            None
        };

        let mut new_default = original_default;
        if let Some(default_id) = original_default {
            if let Some(converted) = evaluate_conversion(default_id) {
                new_default = Some(converted);
            }
        }

        for slot in &mut self.solid_materials {
            let current = match (*slot, original_default) {
                (Some(id), _) => Some(id),
                (None, Some(default_id)) => Some(default_id),
                (None, None) => None,
            };
            if let Some(material_id) = current {
                if let Some(converted) = evaluate_conversion(material_id) {
                    *slot = Some(converted);
                }
            }
        }

        self.default_solid_material = new_default;
    }

    fn column_material(&self, index: usize) -> Option<MaterialId> {
        self.solid_materials
            .get(index)
            .copied()
            .flatten()
            .or(self.default_solid_material)
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
            if let Some(slot) = self.surface.get_mut(x as usize) {
                if target_height > *slot {
                    *slot = target_height;
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
        if let Some(slot) = self.surface.get_mut(index) {
            if *slot < target_height {
                *slot = target_height;
            }
        }
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
        match self.surface_height(x) {
            Some(surface_y) => y >= surface_y,
            None => false,
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

    pub fn resolve_collision(&self, position: Vector2, velocity: Vector2) -> CollisionResolution {
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
                if segment.top <= last.bottom + 1 {
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
        if bottom < top {
            Self {
                top: bottom,
                bottom: top,
            }
        } else {
            Self { top, bottom }
        }
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
        landscape.apply_temperature_conversions(&materials, 10);

        assert_eq!(landscape.solid_material_at(0), Some(water));
        assert_eq!(landscape.solid_material_at(1), Some(water));
        assert_eq!(landscape.default_solid_material(), Some(water));
    }
}
