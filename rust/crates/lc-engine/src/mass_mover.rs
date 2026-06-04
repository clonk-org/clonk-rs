use crate::landscape::Landscape;
use crate::rng::LcgRng;
use crate::{MaterialId, MaterialSet};

const MAX_MASS_MOVERS: usize = 10_000;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

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
    landscape_insert_thrust: bool,
}

impl MassMoverSet {
    pub fn new() -> Self {
        Self {
            movers: Vec::new(),
            spawn_queue: Vec::new(),
            max_movers: MAX_MASS_MOVERS,
            landscape_insert_thrust: false,
        }
    }

    pub fn clear(&mut self) {
        self.movers.clear();
        self.spawn_queue.clear();
    }

    pub fn set_landscape_insert_thrust(&mut self, enabled: bool) {
        self.landscape_insert_thrust = enabled;
    }

    pub fn sync_signature(&self) -> i32 {
        let mut hash = FNV_OFFSET_BASIS;
        hash = fnv_update(hash, &(self.movers.len() as u32).to_le_bytes());
        for mover in &self.movers {
            hash = fnv_update(hash, &(mover.material.index() as u32).to_le_bytes());
            hash = fnv_update(hash, &mover.x.to_le_bytes());
            hash = fnv_update(hash, &mover.y.to_le_bytes());
            hash = fnv_update(hash, &[mover.active as u8]);
        }
        hash = fnv_update(hash, &(self.spawn_queue.len() as u32).to_le_bytes());
        for spawn in &self.spawn_queue {
            hash = fnv_update(hash, &spawn.x.to_le_bytes());
            hash = fnv_update(hash, &spawn.y.to_le_bytes());
            hash = fnv_update(hash, &[spawn.execute_immediately as u8]);
        }
        (hash & 0xFFFF_FFFF) as i32
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
    pub fn create(
        &mut self,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        x: i32,
        y: i32,
        execute_immediately: bool,
        rng: &mut LcgRng,
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
        rng: &mut LcgRng,
    ) {
        self.flush_spawn_queue(landscape, materials, rng);
        for _speed in (1..=2).rev() {
            let mut index = self.movers.len();
            while index > 0 {
                index -= 1;
                if index >= self.movers.len() {
                    continue;
                }
                if !self.execute_mover_at(index, landscape, materials, rng) {
                    self.movers.swap_remove(index);
                }
            }
        }
        self.flush_spawn_queue(landscape, materials, rng);
    }

    fn flush_spawn_queue(
        &mut self,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        rng: &mut LcgRng,
    ) {
        while let Some(spawn) = self.spawn_queue.pop() {
            let Some(index) = self.try_add_mover(landscape, materials, spawn.x, spawn.y) else {
                continue;
            };
            if spawn.execute_immediately && !self.execute_mover_at(index, landscape, materials, rng)
            {
                self.movers.swap_remove(index);
            }
        }
    }

    fn execute_mover_at(
        &mut self,
        index: usize,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        rng: &mut LcgRng,
    ) -> bool {
        let (alive, spawn) = {
            let mover = self
                .movers
                .get_mut(index)
                .expect("mass mover index must exist");
            mover.execute(landscape, materials, rng, self.landscape_insert_thrust)
        };
        if let Some(spawn) = spawn {
            self.spawn_mover(landscape, materials, rng, spawn);
        }
        alive
    }

    fn spawn_mover(
        &mut self,
        landscape: &mut Landscape,
        materials: &MaterialSet,
        rng: &mut LcgRng,
        spawn: MassMoverSpawn,
    ) {
        let Some(index) = self.try_add_mover(landscape, materials, spawn.x, spawn.y) else {
            return;
        };
        if spawn.execute_immediately && !self.execute_mover_at(index, landscape, materials, rng) {
            self.movers.swap_remove(index);
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
        rng: &mut LcgRng,
        landscape_insert_thrust: bool,
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
            for (dx, dy) in [(0, 1), (-1, 0), (1, 0)] {
                if materials
                    .execute_mass_move_reaction(
                        landscape,
                        self.material,
                        self.x,
                        self.y,
                        self.x + dx,
                        self.y + dy,
                        rng,
                    )
                    .consumes_material()
                {
                    let _ = landscape.extract_material_at(self.x, self.y);
                    return (true, None);
                }
            }
            return (false, None);
        };

        let displaced_material = if landscape_insert_thrust {
            landscape.material_at(target.0, target.1)
        } else {
            None
        };
        if landscape.material_at(target.0, target.1).is_some() {
            let _ = landscape.extract_material_at(target.0, target.1);
        }

        let _transfer_as_pixel = rng.random(10) != 0;
        let Some(removed_material) = landscape.extract_material_at(self.x, self.y) else {
            return (false, None);
        };

        let inserted =
            landscape.insert_material_pixel_at(target.0, target.1, removed_material, materials);
        if !inserted {
            let _ = landscape.insert_material_pixel_at(self.x, self.y, removed_material, materials);
            if let Some(displaced) = displaced_material {
                let _ =
                    landscape.insert_material_pixel_at(target.0, target.1, displaced, materials);
            }
            return (false, None);
        }

        if let Some(displaced) = displaced_material {
            if materials
                .get_by_id(displaced)
                .map(|material| material.density() > 0)
                .unwrap_or(false)
            {
                let _ = landscape.insert_material_pixel_at(
                    target.0,
                    target.1 + 1,
                    displaced,
                    materials,
                );
            }
        }

        let spawn = MassMoverSpawn {
            x: target.0,
            y: target.1,
            execute_immediately: rng.rnd3() == 0,
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

#[cfg(test)]
mod tests {
    use super::*;
    use lc_resources::MaterialLibrary;

    fn materials(source: &str) -> MaterialSet {
        MaterialSet::from_resource_library(
            &MaterialLibrary::parse(source).expect("material library parses"),
        )
    }

    #[test]
    fn transfer_consumes_random10_before_rnd3_for_each_move() {
        let materials = materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Instable=1
            MaxSlide=0
            "#,
        );
        let water = materials.id_of("Water").expect("water material");
        let mut landscape = Landscape::flat(3, 10);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(0, 0, Some(water))],
        );

        let mut movers = MassMoverSet::new();
        movers.seed_from_landscape(&landscape, &materials);
        let mut rng = LcgRng::seed_from_u64(2);

        let mut expected = LcgRng::seed_from_u64(2);
        let _ = expected.random(10);
        let _ = expected.rnd3();
        let _ = expected.random(10);
        let _ = expected.rnd3();

        movers.execute(&mut landscape, &materials, &mut rng);

        assert_eq!(rng.count, expected.count);
        assert_eq!(rng.hold, expected.hold);
    }

    #[test]
    fn blocked_mover_uses_massmove_corrosion_rng_before_dying() {
        let materials = materials(
            r#"
            [Material Acid]
            Name=Acid
            Density=25
            Instable=1
            MaxSlide=0
            Corrosive=100

            [Material Rock]
            Name=Rock
            Density=80
            Corrode=100
            "#,
        );
        let acid = materials.id_of("Acid").expect("acid material");
        let rock = materials.id_of("Rock").expect("rock material");
        let mut landscape = Landscape::flat_with_material(3, 5, Some(rock));
        landscape.set_default_liquid_material(Some(acid));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(4, 4, Some(acid))],
        );

        let mut movers = MassMoverSet::new();
        movers.seed_from_landscape(&landscape, &materials);
        let mut rng = LcgRng::seed_from_u64(2);

        let mut expected = LcgRng::seed_from_u64(2);
        assert!(crate::material::evaluate_corrosion(
            100,
            100,
            None,
            &mut expected
        ));
        crate::material::consume_corrosion_effect_rng(&mut expected);

        movers.execute(&mut landscape, &materials, &mut rng);

        assert_eq!(rng.count, expected.count);
        assert_eq!(rng.hold, expected.hold);
    }
}
