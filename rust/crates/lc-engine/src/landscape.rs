use std::collections::HashMap;
use std::convert::TryFrom;
use std::mem;

use crate::material::TemperatureDirection;
use crate::{EnvironmentSettings, MaterialId, MaterialSet, Vector2};
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
    #[serde(skip)]
    mass_mover_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlastResult {
    pub removed_by_material: HashMap<MaterialId, i32>,
    pub affected_columns: Vec<(i32, i32)>,
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
            default_liquid_material: None,
            mass_mover_dirty: false,
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

    pub fn set_height(&mut self, x: u32, height: i32) {
        if let Some(slot) = self.surface.get_mut(x as usize) {
            if *slot != height {
                *slot = height;
                self.mark_mass_mover_dirty();
            }
        }
    }

    pub fn set_liquid_column(&mut self, x: u32, segments: Vec<LiquidSegment>) {
        self.ensure_liquid_capacity();
        if let Some(column) = self.liquids.get_mut(x as usize) {
            *column = LiquidColumn::from_segments(segments);
            self.mark_mass_mover_dirty();
        }
    }

    pub fn clear_liquid_column(&mut self, x: u32) {
        self.ensure_liquid_capacity();
        if let Some(column) = self.liquids.get_mut(x as usize) {
            column.clear();
            self.mark_mass_mover_dirty();
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
                self.mark_mass_mover_dirty();
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
        self.mark_mass_mover_dirty();
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
        self.mark_mass_mover_dirty();
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
        self.estimate_world_height()
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

        let mut changed = false;
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
                            changed = true;
                        }
                    }
                }
                TemperatureConversionAction::RemoveToSky { strength } => {
                    if *strength > 0 {
                        let new_height = self.surface[index].saturating_add(*strength);
                        if new_height != self.surface[index] {
                            self.surface[index] = new_height;
                            changed = true;
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
                            self.surface[index] = new_height;
                            changed = true;
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
                changed = true;
            } else {
                for index in default_change_indices {
                    if let Some(TemperatureConversionAction::ChangeMaterial { target, .. }) =
                        column_actions[index]
                    {
                        self.solid_materials[index] = Some(target);
                        changed = true;
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
                changed = true;
            }
        }

        let mut liquids_changed = false;
        let temperature_lookup = |y: i32| environment.temperature_at_height(frame, y, world_height);
        for column in &mut self.liquids {
            if column.apply_temperature_conversions(
                materials,
                self.default_liquid_material,
                &temperature_lookup,
            ) {
                liquids_changed = true;
            }
        }

        if liquids_changed {
            changed = true;
        }

        if changed {
            self.mark_mass_mover_dirty();
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

    fn mark_mass_mover_dirty(&mut self) {
        self.mass_mover_dirty = true;
    }

    pub fn take_mass_mover_dirty(&mut self) -> bool {
        let dirty = self.mass_mover_dirty;
        self.mass_mover_dirty = false;
        dirty
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
        if self.is_solid_at(x, y) {
            self.solid_material_at(x)
        } else {
            self.liquid_material_at(x, y)
        }
    }

    pub fn density_at(&self, x: i32, y: i32, materials: &MaterialSet) -> i32 {
        if x < 0 || x as u32 >= self.width {
            return i32::MAX;
        }
        if y < 0 {
            return 0;
        }
        match self.material_at(x, y) {
            Some(material_id) => materials
                .get_by_id(material_id)
                .map(|material| material.density())
                .unwrap_or(i32::MAX),
            None => 0,
        }
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
            self.mark_mass_mover_dirty();
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
            self.mark_mass_mover_dirty();
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
        let mut changed = false;
        for x in clamped_start..clamped_end {
            if let Some(slot) = self.surface.get_mut(x as usize) {
                if target_height > *slot {
                    *slot = target_height;
                    changed = true;
                }
            }
        }
        if changed {
            self.mark_mass_mover_dirty();
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
                self.mark_mass_mover_dirty();
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
                        result
                            .pixel_count_by_material
                            .entry(material_id)
                            .and_modify(|count| *count += removed_height)
                            .or_insert(removed_height);
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

            self.surface[index] = target_height;
            result
                .removed_by_material
                .entry(material_id)
                .and_modify(|count| *count += removed_height)
                .or_insert(removed_height);
            result.affected_columns.push((column, target_height));
        }

        if !result.affected_columns.is_empty() || !result.removed_by_material.is_empty() {
            self.mark_mass_mover_dirty();
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
            self.mark_mass_mover_dirty();
            return true;
        }
        self.surface[index] = target;
        self.solid_materials[index] = Some(material);
        self.mark_mass_mover_dirty();
        true
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
            self.mark_mass_mover_dirty();
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
        landscape.mass_mover_dirty = false;
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
}
