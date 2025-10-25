use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::landscape::Landscape;
use crate::{MaterialId, MaterialSet};

const MAX_MASS_MOVERS: usize = 10_000;

#[derive(Debug, Clone)]
struct MassMover {
    material: MaterialId,
    x: i32,
    y: i32,
    active: bool,
}

#[derive(Debug, Clone)]
struct MassMoverSpawn {
    x: i32,
    y: i32,
    execute_immediately: bool,
}

#[derive(Debug, Default)]
pub struct MassMoverSet {
    movers: Vec<MassMover>,
    spawn_queue: Vec<MassMoverSpawn>,
    max_movers: usize,
}

impl MassMoverSet {
    pub fn new() -> Self {
        Self {
            movers: Vec::new(),
            spawn_queue: Vec::new(),
            max_movers: MAX_MASS_MOVERS,
        }
    }

    pub fn clear(&mut self) {
        self.movers.clear();
        self.spawn_queue.clear();
    }

    pub fn len(&self) -> usize {
        self.movers.len()
    }

    pub fn seed_from_landscape(&mut self, landscape: &Landscape, materials: &MaterialSet) {
        self.clear();
        for (x, column) in landscape.liquids().iter().enumerate() {
            for segment in column.segments() {
                let Some(material) = segment.material.or(landscape.default_liquid_material())
                else {
                    continue;
                };
                let Some(mat) = materials.get_by_id(material) else {
                    continue;
                };
                if !mat.instable() {
                    continue;
                }
                let (surface_x, surface_y) =
                    landscape.find_liquid_surface(material, x as i32, segment.top, materials);
                if self.movers.len() < self.max_movers {
                    self.movers
                        .push(MassMover::new(material, surface_x, surface_y));
                } else {
                    return;
                }
            }
        }
    }

    pub fn create(
        &mut self,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        x: i32,
        y: i32,
        execute_immediately: bool,
        rng: &mut ChaCha8Rng,
    ) -> bool {
        self.spawn_queue.push(MassMoverSpawn {
            x,
            y,
            execute_immediately,
        });
        let previous = self.movers.len();
        self.flush_spawn_queue(landscape, materials, rng);
        self.movers.len() > previous
    }

    pub fn execute(
        &mut self,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        rng: &mut ChaCha8Rng,
    ) {
        let mut index = 0;
        while index < self.movers.len() {
            let (alive, spawn) = {
                let mover = &mut self.movers[index];
                mover.execute(landscape, materials, rng)
            };
            if let Some(spawn) = spawn {
                self.spawn_queue.push(spawn);
            }
            if !alive {
                self.movers.swap_remove(index);
            } else {
                index += 1;
            }
        }
        self.flush_spawn_queue(landscape, materials, rng);
    }

    fn flush_spawn_queue(
        &mut self,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        rng: &mut ChaCha8Rng,
    ) {
        while let Some(spawn) = self.spawn_queue.pop() {
            let Some(index) = self.try_add_mover(landscape, materials, spawn.x, spawn.y) else {
                continue;
            };
            if spawn.execute_immediately {
                let (alive, new_spawn) = {
                    let mover = self
                        .movers
                        .get_mut(index)
                        .expect("mass mover index must exist");
                    mover.execute(landscape, materials, rng)
                };
                if let Some(extra) = new_spawn {
                    self.spawn_queue.push(extra);
                }
                if !alive {
                    self.movers.swap_remove(index);
                }
            }
        }
    }

    fn try_add_mover(
        &mut self,
        landscape: &Landscape,
        materials: &MaterialSet,
        x: i32,
        y: i32,
    ) -> Option<usize> {
        if self.movers.len() >= self.max_movers {
            return None;
        }
        let material_id = landscape.material_at(x, y)?;
        let material = materials.get_by_id(material_id)?;
        if !material.instable() {
            return None;
        }
        let (surface_x, surface_y) = landscape.find_liquid_surface(material_id, x, y, materials);
        let mover = MassMover::new(material_id, surface_x, surface_y);
        self.movers.push(mover);
        Some(self.movers.len() - 1)
    }
}

impl MassMover {
    fn new(material: MaterialId, x: i32, y: i32) -> Self {
        Self {
            material,
            x,
            y,
            active: true,
        }
    }

    fn execute(
        &mut self,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        rng: &mut ChaCha8Rng,
    ) -> (bool, Option<MassMoverSpawn>) {
        if !self.active {
            return (false, None);
        }

        let Some(current_material) = landscape.material_at(self.x, self.y) else {
            return (false, None);
        };
        if current_material != self.material {
            return (false, None);
        }

        let (surface_x, surface_y) =
            landscape.find_liquid_surface(self.material, self.x, self.y, materials);
        self.x = surface_x;
        self.y = surface_y;

        let Some(target) = find_liquid_target(landscape, materials, self.material, self.x, self.y)
        else {
            return (false, None);
        };

        let Some(removed_material) = landscape.remove_liquid_at(self.x, self.y) else {
            return (false, None);
        };

        if !landscape.insert_liquid_at(target.0, target.1, Some(removed_material)) {
            let _ = landscape.insert_liquid_at(self.x, self.y, Some(removed_material));
            return (false, None);
        }

        let spawn = MassMoverSpawn {
            x: target.0,
            y: target.1,
            execute_immediately: rnd3(rng) == 0,
        };

        (true, Some(spawn))
    }
}

fn find_liquid_target(
    landscape: &Landscape,
    materials: &MaterialSet,
    material: MaterialId,
    x: i32,
    y: i32,
) -> Option<(i32, i32)> {
    let material = materials.get_by_id(material)?;
    let density = material.density();
    let max_slide = material.max_slide().max(0);
    let ydir = 1;
    if landscape.density_at(x, y + ydir, materials) < density {
        return Some((x, y + ydir));
    }

    let mut cslide = 1;
    let mut left_active = true;
    let mut right_active = true;
    while cslide <= max_slide && (left_active || right_active) {
        if left_active {
            if landscape.density_at(x - cslide, y, materials) >= density {
                left_active = false;
            } else if landscape.density_at(x - cslide, y + ydir, materials) < density {
                return Some((x - 1, y));
            }
        }
        if right_active {
            if landscape.density_at(x + cslide, y, materials) >= density {
                right_active = false;
            } else if landscape.density_at(x + cslide, y + ydir, materials) < density {
                return Some((x + 1, y));
            }
        }
        cslide += 1;
    }

    None
}

fn rnd3(rng: &mut ChaCha8Rng) -> i32 {
    rng.gen_range(0..3) - 1
}
