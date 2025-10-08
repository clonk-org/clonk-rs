use crate::Vector2;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LandscapeError {
    #[error("height map length {found} does not match width {width}")]
    InvalidHeightMap { width: u32, found: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Landscape {
    width: u32,
    surface: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LandscapeCommand {
    LowerRange { start: i32, end: i32, height: i32 },
}

impl Landscape {
    pub fn new(width: u32, surface: Vec<i32>) -> Result<Self, LandscapeError> {
        if width as usize != surface.len() {
            return Err(LandscapeError::InvalidHeightMap {
                width,
                found: surface.len(),
            });
        }
        Ok(Self { width, surface })
    }

    pub fn flat(width: u32, height: i32) -> Self {
        Self {
            width,
            surface: vec![height; width as usize],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn surface(&self) -> &[i32] {
        &self.surface
    }

    pub fn set_height(&mut self, x: u32, height: i32) {
        if let Some(slot) = self.surface.get_mut(x as usize) {
            *slot = height;
        }
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

    pub fn resolve_collision(&self, position: Vector2, velocity: Vector2) -> CollisionResolution {
        match self.surface_height(position.x) {
            Some(surface_y) if position.y > surface_y => {
                let mut new_position = position;
                let mut new_velocity = velocity;
                new_position.y = surface_y;
                if new_velocity.y > 0 {
                    new_velocity.y = 0;
                }
                CollisionResolution {
                    position: new_position,
                    velocity: new_velocity,
                    collided: true,
                }
            }
            _ => CollisionResolution {
                position,
                velocity,
                collided: false,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionResolution {
    pub position: Vector2,
    pub velocity: Vector2,
    pub collided: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_vertical_collision() {
        let landscape = Landscape::flat(10, 5);
        let position = Vector2::new(3, 8);
        let velocity = Vector2::new(0, 3);
        let resolution = landscape.resolve_collision(position, velocity);
        assert!(resolution.collided);
        assert_eq!(resolution.position, Vector2::new(3, 5));
        assert_eq!(resolution.velocity, Vector2::new(0, 0));
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
}
