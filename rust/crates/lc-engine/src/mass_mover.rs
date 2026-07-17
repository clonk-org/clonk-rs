use crate::{Engine, Landscape, MaterialId, MaterialSet};
use serde::{Deserialize, Serialize};

/// `C4MassMoverChunk` (C4MassMover.h:25).
pub(crate) const MASS_MOVER_CHUNK: i32 = 10_000;
const CHUNK: usize = MASS_MOVER_CHUNK as usize;

/// One `C4MassMover` slot (C4MassMover.h:29-41): a material pinned to the
/// pixel it was created on. The mover NEVER re-seeks the liquid surface —
/// it lives at (x, y) until that pixel stops holding `mat`
/// (C4MassMover.cpp:119).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MassMover {
    pub mat: MaterialId,
    pub x: i32,
    pub y: i32,
}

/// Mirror of `C4MassMoverSet` (C4MassMover.h:43-68): a fixed array of
/// `C4MassMoverChunk` (10000) slots with an advancing `CreatePtr` allocation
/// cursor (C4MassMover.cpp:67-88). `Count` follows the C++ ledger exactly:
/// `Init` bumps it even for a failed (sky) init (C4MassMover.cpp:99), `Cease`
/// decrements (:110), and the set `Execute` resets it and re-counts per speed
/// pass (:54-62) — live movers are counted TWICE per frame, so `Count` is not
/// the number of live movers; only the `Count == C4MassMoverChunk` equality
/// gate in `Create` (:69) consumes it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(from = "MassMoverSetSnapshot", into = "MassMoverSetSnapshot")]
pub struct MassMoverSet {
    slots: Vec<Option<MassMover>>,
    create_ptr: usize,
    count: i32,
}

/// Sparse serialized form: only the occupied slots (with their indices, so
/// the C4MassMoverChunk iteration order survives a save/load round-trip)
/// plus the `CreatePtr`/`Count` ledger.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct MassMoverSetSnapshot {
    #[serde(default)]
    create_ptr: usize,
    #[serde(default)]
    count: i32,
    #[serde(default)]
    movers: Vec<(u32, MassMover)>,
}

impl From<MassMoverSetSnapshot> for MassMoverSet {
    fn from(snapshot: MassMoverSetSnapshot) -> Self {
        let mut set = MassMoverSet {
            slots: Vec::new(),
            create_ptr: snapshot.create_ptr.min(CHUNK - 1),
            count: snapshot.count,
        };
        for (index, mover) in snapshot.movers {
            let index = index as usize;
            if index < CHUNK {
                set.ensure_slots();
                set.slots[index] = Some(mover);
            }
        }
        set
    }
}

impl From<MassMoverSet> for MassMoverSetSnapshot {
    fn from(set: MassMoverSet) -> Self {
        MassMoverSetSnapshot {
            create_ptr: set.create_ptr,
            count: set.count,
            movers: set
                .slots
                .iter()
                .enumerate()
                .filter_map(|(index, slot)| slot.map(|mover| (index as u32, mover)))
                .collect(),
        }
    }
}

impl MassMoverSet {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_slots(&mut self) {
        if self.slots.len() != CHUNK {
            self.slots.resize(CHUNK, None);
        }
    }

    /// `C4MassMoverSet::Default` (C4MassMover.cpp:173-179).
    pub fn clear(&mut self) {
        self.slots.clear();
        self.create_ptr = 0;
        self.count = 0;
    }

    /// The C++ `Count` ledger value (see the struct docs for its quirks).
    pub(crate) fn count(&self) -> i32 {
        self.count
    }

    /// `Count = 0` at the head of `C4MassMoverSet::Execute` (C4MassMover.cpp:54).
    pub(crate) fn reset_count(&mut self) {
        self.count = 0;
    }

    /// `Game.MassMover.Count++` — both the `Init` bump (C4MassMover.cpp:99,
    /// fires even when the pixel is sky) and the per-pass live-mover count
    /// (:62).
    pub(crate) fn bump_count(&mut self) {
        self.count += 1;
    }

    pub(crate) fn slot(&self, index: usize) -> Option<MassMover> {
        self.slots.get(index).copied().flatten()
    }

    /// The `C4MassMoverSet::Create` slot scan (C4MassMover.cpp:75-87): start
    /// AFTER `CreatePtr`, wrap at the chunk end, first free slot wins; the
    /// do-while tests every slot including `CreatePtr` itself. Read-only —
    /// `CreatePtr` only advances on a successful `Init` (`fill_slot`).
    pub(crate) fn find_free_slot(&self) -> Option<usize> {
        let start = self.create_ptr;
        let mut cptr = start;
        loop {
            cptr += 1;
            if cptr >= CHUNK {
                cptr = 0;
            }
            if self.slot(cptr).is_none() {
                return Some(cptr);
            }
            if cptr == start {
                return None;
            }
        }
    }

    /// The `Create` success branch (C4MassMover.cpp:82-83): store the mover
    /// and leave `CreatePtr` on its slot.
    pub(crate) fn fill_slot(&mut self, index: usize, mover: MassMover) {
        self.ensure_slots();
        self.slots[index] = Some(mover);
        self.create_ptr = index;
    }

    /// `C4MassMover::Cease` (C4MassMover.cpp:103-112): free the slot and
    /// decrement `Count`.
    pub(crate) fn cease(&mut self, index: usize) {
        if self
            .slots
            .get_mut(index)
            .and_then(Option::take)
            .is_some()
        {
            self.count -= 1;
        }
    }

    /// The C++ `CreatePtr` digest value (C4Control.cpp:454).
    pub fn create_ptr(&self) -> i32 {
        self.create_ptr as i32
    }

    /// Number of occupied slots (test/diagnostic helper — NOT the C++
    /// `Count` ledger).
    pub fn live_movers(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    pub(crate) fn check_instability_range_for_landscape(
        &mut self,
        landscape: &Landscape,
        materials: &MaterialSet,
        tx: i32,
        ty: i32,
    ) {
        if !self.check_instability_for_landscape(landscape, materials, tx, ty) {
            self.check_instability_for_landscape(landscape, materials, tx, ty - 1);
            self.check_instability_for_landscape(landscape, materials, tx, ty - 2);
            self.check_instability_for_landscape(landscape, materials, tx - 1, ty);
            self.check_instability_for_landscape(landscape, materials, tx + 1, ty);
        }
    }

    fn check_instability_for_landscape(
        &mut self,
        landscape: &Landscape,
        materials: &MaterialSet,
        x: i32,
        y: i32,
    ) -> bool {
        let instable = landscape
            .material_at(x, y)
            .and_then(|id| materials.get_by_id(id))
            .map(|material| material.instable())
            .unwrap_or(false);
        instable && self.create_for_landscape(landscape, x, y).is_some()
    }

    fn create_for_landscape(
        &mut self,
        landscape: &Landscape,
        x: i32,
        y: i32,
    ) -> Option<usize> {
        if self.count() == MASS_MOVER_CHUNK {
            return None;
        }
        let index = self.find_free_slot()?;
        let (width, height) = landscape
            .grid_dimensions()
            .unwrap_or((landscape.width() as i32, landscape.estimated_height()));
        if !(0..width).contains(&x) || !(0..height).contains(&y) {
            return None;
        }
        let material = landscape.material_at(x, y);
        self.bump_count();
        let material = material?;
        self.fill_slot(
            index,
            MassMover {
                mat: material,
                x,
                y,
            },
        );
        Some(index)
    }

    /// `C4MassMoverSet::Consolidate` (C4MassMover.cpp:219-247): pack live
    /// movers down over free slots (preserving order) and reset `CreatePtr`.
    pub(crate) fn consolidate(&mut self) {
        let mut spot: Option<usize> = None;
        for ptr in 0..self.slots.len() {
            if self.slots[ptr].is_none() {
                if spot.is_none() {
                    spot = Some(ptr);
                }
            } else if let Some(mut empty) = spot {
                self.slots[empty] = self.slots[ptr].take();
                // Advance the empty spot (as far as ptr)
                while empty < ptr && self.slots[empty].is_some() {
                    empty += 1;
                }
                spot = (empty < ptr).then_some(empty);
            }
        }
        self.create_ptr = 0;
    }

    /// `C4MassMoverSet::Synchronize` (C4MassMover.cpp:249-252).
    pub fn synchronize(&mut self) {
        self.consolidate();
    }
}

impl Engine {
    /// `C4Landscape::CheckInstability` (C4Landscape.cpp:860-867): only an
    /// Instable material at the exact pixel creates a mover — the Instable
    /// gate lives HERE, not in `C4MassMover::Init`.
    pub(crate) fn check_instability(&mut self, tx: i32, ty: i32) -> bool {
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        self.mass_movers.check_instability_for_landscape(
            landscape,
            &self.materials,
            tx,
            ty,
        )
    }

    /// `C4Landscape::CheckInstabilityRange` (C4Landscape.cpp:869-878): try
    /// the pixel itself; ONLY if that fails, probe (tx,ty-1), (tx,ty-2),
    /// (tx-1,ty), (tx+1,ty) — all four, in that order.
    pub(crate) fn check_instability_range(&mut self, tx: i32, ty: i32) {
        if let Some(landscape) = self.landscape.as_ref() {
            self.mass_movers.check_instability_range_for_landscape(
                landscape,
                &self.materials,
                tx,
                ty,
            );
        }
    }

    /// `C4MassMoverSet::Create` (C4MassMover.cpp:67-89) plus
    /// `C4MassMover::Init` (:91-101). Admission takes ANY material at the
    /// exact coordinates — no Instable gate, no snap-to-surface; only the
    /// bounds check and the sky (`MNone`) rejection. `Count` bumps even on
    /// the sky rejection (:99), and `CreatePtr` advances only on success.
    pub(crate) fn mass_mover_create(&mut self, x: i32, y: i32, execute: bool) -> bool {
        let Some(landscape) = self.landscape.as_ref() else {
            return false;
        };
        let Some(index) = self
            .mass_movers
            .create_for_landscape(landscape, x, y)
        else {
            return false;
        };
        if execute {
            self.execute_mass_mover(index);
        }
        true
    }

    /// `C4MassMoverSet::Execute` (C4MassMover.cpp:50-65): reset `Count`,
    /// then two speed passes, each walking the slots DESCENDING from the
    /// last chunk entry, counting and executing every live mover.
    pub(crate) fn tick_mass_movers(&mut self) {
        if self.landscape.is_none() {
            return;
        }
        self.mass_movers.reset_count();
        for _speed in 0..2 {
            for index in (0..CHUNK).rev() {
                if self.mass_movers.slot(index).is_some() {
                    self.mass_movers.bump_count();
                    self.execute_mass_mover(index);
                }
            }
        }
    }

    /// `C4MassMover::Execute` (C4MassMover.cpp:114-157). Returns the C++
    /// bool (false = the mover died).
    fn execute_mass_mover(&mut self, index: usize) -> bool {
        let Some(MassMover { mat, x, y }) = self.mass_movers.slot(index) else {
            return false;
        };

        // Lost target material (:119) — the mover is pinned to its creation
        // pixel; it dies rather than re-seeking the surface.
        let current = self
            .landscape
            .as_ref()
            .and_then(|landscape| landscape.material_at(x, y));
        if current != Some(mat) {
            self.mass_movers.cease(index);
            return false;
        }

        // Check for transfer target space via FindMatPath (:121-124).
        let (density, max_slide) = self
            .materials
            .get_by_id(mat)
            .map(|material| (material.density(), material.max_slide()))
            .unwrap_or((0, 0));
        let (mut tx, mut ty) = (x, y);
        let found = self
            .landscape
            .as_ref()
            .map(|landscape| {
                landscape.find_mat_path(&mut tx, &mut ty, 1, density, max_slide, &self.materials)
            })
            .unwrap_or(false);

        if !found {
            // Contact material reaction check (:126-132):
            // corrosion/evaporation/inflammation below, left, right.
            for (dx, dy) in [(0, 1), (-1, 0), (1, 0)] {
                let result = self.execute_mass_move_reaction(mat, x, y, x + dx, y + dy);
                if result.consumes_material() {
                    // mrfConvert meeMassMove (C4Material.cpp:654-657):
                    // conversion-transfer — the mover's material spawns as
                    // a PXS at the mover position before the extraction.
                    if matches!(result, crate::material::MaterialReactionExecution::Converted(_)) {
                        self.pxs_system.create(
                            mat,
                            crate::math::itofix(x),
                            crate::math::itofix(y),
                            crate::math::C4Fixed::ZERO,
                            crate::math::C4Fixed::ZERO,
                        );
                    }
                    // material has been used up (:130)
                    let _ = self.extract_material(x, y);
                    return true;
                }
            }
            // No space, die (:135)
            self.mass_movers.cease(index);
            return false;
        }

        // Save back material that is about to be overwritten (:138-141).
        let omat = if self.landscape_insert_thrust {
            self.landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(tx, ty))
        } else {
            None
        };

        // Transfer mass (:143-147): Random(10) truthy (9-in-10) writes the
        // pixel directly (SBackPix keeps the target's IFT); the 1-in-10
        // zero draw routes through the full InsertMaterial with vy=1.
        if self.rng.random(10) != 0 {
            if let Some(extracted) = self.extract_material(x, y) {
                let materials = &self.materials;
                if let Some(landscape) = self.landscape.as_mut() {
                    // SBackPix(tx, ty, Mat2PixColDefault(mat) + GBackIFT) (:145);
                    // the target pixel is overwritten and never restored.
                    if !landscape.insert_material_pix(tx, ty, extracted) {
                        // column-model fixture worlds keep the column write
                        let _ =
                            landscape.insert_material_pixel_at(tx, ty, extracted, materials);
                    }
                }
            }
        } else if let Some(extracted) = self.extract_material(x, y) {
            self.insert_material(extracted, tx, ty, 0, 1);
        }

        // Reinsert material (thrusted aside) at (tx, ty+1) (:149-151).
        if let Some(omat) = omat {
            if self
                .materials
                .get_by_id(omat)
                .map(|material| material.density() > 0)
                .unwrap_or(false)
            {
                self.insert_material(omat, tx, ty + 1, 0, 0);
            }
        }

        // Create new mover at target (:153-154); `!Rnd3()` executes it
        // immediately one time in three.
        let execute = self.rng.rnd3() == 0;
        self.mass_mover_create(tx, ty, execute);

        true
    }

    /// `C4Landscape::ExtractMaterial` (C4Landscape.cpp:1148-1156): FindMatTop
    /// slides to the material surface, the TOP pixel clears, and
    /// CheckInstabilityRange fires at the CLEARED coordinates.
    pub(crate) fn extract_material(&mut self, fx: i32, fy: i32) -> Option<MaterialId> {
        let extracted = {
            let materials = &self.materials;
            self.landscape
                .as_mut()
                .and_then(|landscape| landscape.extract_material_probe(fx, fy, materials))
        };
        extracted.map(|(mat, top_x, top_y)| {
            self.check_instability_range(top_x, top_y);
            mat
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::landscape::Landscape;
    use crate::{MaterialInteractionEvent, MaterialSet};
    use lc_resources::MaterialLibrary;

    fn materials(source: &str) -> MaterialSet {
        MaterialSet::from_resource_library(
            &MaterialLibrary::parse(source).expect("material library parses"),
        )
    }

    fn engine_with(materials: MaterialSet, landscape: Landscape) -> Engine {
        let mut engine = Engine::with_seed(2);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine
    }

    fn water_materials() -> MaterialSet {
        materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Instable=1
            MaxSlide=0
            "#,
        )
    }

    /// Column fixture: a single water pixel at (1, 0) high above the flat
    /// terrain at height 10.
    fn water_drop_engine() -> Engine {
        let materials = water_materials();
        let water = materials.id_of("Water").expect("water material");
        let mut landscape = Landscape::flat(3, 10);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(0, 0, Some(water))],
        );
        engine_with(materials, landscape)
    }

    fn blocked_incinerating_mover_engine() -> (Engine, MaterialId) {
        let materials = materials(
            r#"
            [Material Oil]
            Name=Oil
            Density=25
            Instable=1
            MaxSlide=0
            Inflammable=1

            [Material Lava]
            Name=Lava
            Density=50
            Incindiary=1
            "#,
        );
        let oil = materials.id_of("Oil").expect("oil material");
        let lava = materials.id_of("Lava").expect("lava material");
        let mut landscape = Landscape::flat_with_material(3, 5, Some(lava));
        landscape.set_default_liquid_material(Some(oil));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(4, 4, Some(oil))],
        );
        let mut engine = engine_with(materials, landscape);
        engine
            .register_definition(
                crate::Definition::from_script(crate::FIRE_DEFINITION_ID, "Fire", "#strict\n")
                    .expect("FLAM definition compiles"),
            )
            .expect("FLAM definition registers");
        assert!(engine.mass_mover_create(1, 4, false));
        (engine, oil)
    }

    #[test]
    fn create_scans_slots_after_create_ptr_and_wraps() {
        // C4MassMoverSet::Create (C4MassMover.cpp:75-87): the scan starts
        // AFTER CreatePtr, so the first mover lands in slot 1, the next in
        // slot 2, and CreatePtr follows the filled slot.
        let mut engine = water_drop_engine();
        assert!(engine.mass_mover_create(1, 0, false));
        assert_eq!(engine.mass_movers.create_ptr(), 1);
        assert_eq!(engine.mass_movers.slot(1).map(|m| (m.x, m.y)), Some((1, 0)));
        assert!(engine.mass_mover_create(1, 0, false));
        assert_eq!(engine.mass_movers.create_ptr(), 2);
        assert_eq!(engine.mass_movers.slot(2).map(|m| (m.x, m.y)), Some((1, 0)));
    }

    #[test]
    fn init_admits_any_material_at_exact_coords() {
        // C4MassMover::Init (C4MassMover.cpp:91-101) takes ANY material at
        // the exact pixel — the Instable gate lives only in
        // CheckInstability. Solid earth at (0, 10) must be admitted.
        let materials = materials(
            r#"
            [Material Earth]
            Name=Earth
            Density=50
            "#,
        );
        let earth = materials.id_of("Earth").expect("earth material");
        let mut landscape = Landscape::flat_with_material(3, 10, Some(earth));
        landscape.set_default_liquid_material(None);
        landscape.set_world_height(20); // GBackHgt for the Init bounds check
        let mut engine = engine_with(materials, landscape);
        assert!(engine.mass_mover_create(0, 10, false));
        let mover = engine.mass_movers.slot(1).expect("mover admitted");
        assert_eq!((mover.x, mover.y, mover.mat), (0, 10, earth));
    }

    #[test]
    fn init_rejects_sky_but_still_bumps_count() {
        // C4MassMover::Init (C4MassMover.cpp:97-100): a sky pixel fails the
        // init — but Count++ has already happened (:99) and CreatePtr does
        // not advance.
        let mut engine = water_drop_engine();
        assert!(!engine.mass_mover_create(0, 0, false)); // (0,0) is sky
        assert_eq!(engine.mass_movers.count(), 1);
        assert_eq!(engine.mass_movers.create_ptr(), 0);
        assert_eq!(engine.mass_movers.live_movers(), 0);
    }

    #[test]
    fn init_rejects_out_of_bounds_without_count_bump() {
        // C4MassMover::Init (C4MassMover.cpp:93-95): the bounds check fails
        // BEFORE the Count++.
        let mut engine = water_drop_engine();
        assert!(!engine.mass_mover_create(-1, 0, false));
        assert!(!engine.mass_mover_create(3, 0, false));
        assert_eq!(engine.mass_movers.count(), 0);
    }

    #[test]
    fn create_gate_is_exact_equality_on_count() {
        // `if (Count == C4MassMoverChunk) return false;`
        // (C4MassMover.cpp:69) — a count ABOVE the chunk size does not
        // block creation (the double-counting ledger can exceed it).
        let mut engine = water_drop_engine();
        for _ in 0..MASS_MOVER_CHUNK {
            engine.mass_movers.bump_count();
        }
        assert_eq!(engine.mass_movers.count(), MASS_MOVER_CHUNK);
        assert!(!engine.mass_mover_create(1, 0, false));
        engine.mass_movers.bump_count();
        assert!(engine.mass_mover_create(1, 0, false));
    }

    #[test]
    fn check_instability_gates_on_instable_material() {
        // C4Landscape::CheckInstability (C4Landscape.cpp:860-867): only
        // Instable materials create movers at the check sites.
        let materials = materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Instable=1
            MaxSlide=0

            [Material Earth]
            Name=Earth
            Density=50
            "#,
        );
        let water = materials.id_of("Water").expect("water material");
        let earth = materials.id_of("Earth").expect("earth material");
        let mut landscape = Landscape::flat_with_material(3, 10, Some(earth));
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(0, 0, Some(water))],
        );
        let mut engine = engine_with(materials, landscape);
        assert!(!engine.check_instability(0, 10)); // solid earth: no mover
        assert_eq!(engine.mass_movers.live_movers(), 0);
        assert!(engine.check_instability(1, 0)); // instable water: mover
        assert_eq!(engine.mass_movers.live_movers(), 1);
    }

    #[test]
    fn check_instability_range_probes_in_cpp_order() {
        // C4Landscape::CheckInstabilityRange (C4Landscape.cpp:869-878):
        // (tx,ty) first; only when that fails, probe (tx,ty-1), (tx,ty-2),
        // (tx-1,ty), (tx+1,ty) — all four.
        let materials = water_materials();
        let water = materials.id_of("Water").expect("water material");

        // Case 1: water at the probe point itself — exactly ONE mover.
        let mut landscape = Landscape::flat(5, 10);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            2,
            vec![crate::LiquidSegment::with_material(4, 4, Some(water))],
        );
        let mut engine = engine_with(materials.clone(), landscape);
        engine.check_instability_range(2, 4);
        assert_eq!(engine.mass_movers.live_movers(), 1);
        let mover = engine.mass_movers.slot(1).expect("mover at probe point");
        assert_eq!((mover.x, mover.y), (2, 4));

        // Case 2: probe point dry, water on BOTH side probes — both fire
        // (no short-circuit among the fallback probes).
        let mut landscape = Landscape::flat(5, 10);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(4, 4, Some(water))],
        );
        landscape.set_liquid_column(
            3,
            vec![crate::LiquidSegment::with_material(4, 4, Some(water))],
        );
        let mut engine = engine_with(materials.clone(), landscape);
        engine.check_instability_range(2, 4);
        assert_eq!(engine.mass_movers.live_movers(), 2);
        // Fallback order is up, up-up, LEFT, RIGHT: slot 1 = (1,4), slot 2 = (3,4).
        let first = engine.mass_movers.slot(1).expect("left probe fired first");
        assert_eq!((first.x, first.y), (1, 4));
        let second = engine.mass_movers.slot(2).expect("right probe fired second");
        assert_eq!((second.x, second.y), (3, 4));

        // Case 3: water above at (tx, ty-1) — probed before the sides.
        let mut landscape = Landscape::flat(5, 10);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            2,
            vec![crate::LiquidSegment::with_material(3, 3, Some(water))],
        );
        let mut engine = engine_with(materials, landscape);
        engine.check_instability_range(2, 4);
        let mover = engine.mass_movers.slot(1).expect("up probe fired");
        assert_eq!((mover.x, mover.y), (2, 3));
    }

    #[test]
    fn mover_dies_when_pixel_no_longer_holds_its_material() {
        // C4MassMover::Execute (C4MassMover.cpp:119): the mover ceases as
        // soon as its creation pixel stops holding its material — it never
        // re-seeks the surface.
        let mut engine = water_drop_engine();
        assert!(engine.mass_mover_create(1, 0, false));
        // Drain the pixel out from under the mover.
        let materials = engine.materials.clone();
        let _ = engine
            .landscape
            .as_mut()
            .and_then(|landscape| landscape.extract_material_probe(1, 0, &materials));
        let rng_before = engine.rng.clone();
        engine.tick_mass_movers();
        assert_eq!(engine.mass_movers.live_movers(), 0);
        // The death consumes NO randomness (Cease draws nothing).
        assert_eq!(engine.rng.count, rng_before.count);
        assert_eq!(engine.rng.hold, rng_before.hold);
    }

    #[test]
    fn mover_stays_at_created_pixel_instead_of_reseeking_surface() {
        // The mover is created BELOW the liquid surface; C++ keeps it at
        // its pixel (C4MassMover.cpp:97-98 stores tx/ty; Execute reads the
        // stored x/y — no surface re-seek anywhere in :114-157).
        let materials = water_materials();
        let water = materials.id_of("Water").expect("water material");
        let mut landscape = Landscape::flat(3, 10);
        landscape.set_default_liquid_material(Some(water));
        // Water column rows 2..=4 at x=1; surface pixel is (1, 2).
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(2, 4, Some(water))],
        );
        let mut engine = engine_with(materials, landscape);
        assert!(engine.mass_mover_create(1, 4, false));
        let mover = engine.mass_movers.slot(1).expect("mover created");
        assert_eq!((mover.x, mover.y), (1, 4), "no snap to the surface at creation");
        engine.tick_mass_movers();
        // The mover may have moved material, but its own coordinates are
        // immutable for its whole life.
        if let Some(mover) = engine.mass_movers.slot(1) {
            assert_eq!((mover.x, mover.y), (1, 4));
        }
    }

    #[test]
    fn transfer_uses_random10_split_and_rnd3_spawn() {
        // C4MassMover::Execute (C4MassMover.cpp:144-154): one Random(10)
        // draw picks SBackPix (truthy) vs InsertMaterial (zero), then the
        // Rnd3 draw decides immediate execution of the target mover.
        let mut engine = water_drop_engine();
        assert!(engine.mass_mover_create(1, 0, false));

        let mut expected = engine.rng.clone();
        let _ = expected.random(10);
        let _ = expected.rnd3();
        let _ = expected.random(10);
        let _ = expected.rnd3();

        engine.tick_mass_movers();

        assert_eq!(engine.rng.count, expected.count);
        assert_eq!(engine.rng.hold, expected.hold);
    }

    #[test]
    fn slot_iteration_runs_two_descending_passes() {
        // C4MassMoverSet::Execute (C4MassMover.cpp:56-63): slots are walked
        // DESCENDING, twice. A mover created mid-pass in a HIGHER slot than
        // the cursor is only reached on the NEXT pass; the falling drop
        // therefore advances exactly two pixels per frame while both
        // spawned movers stay in ascending slots.
        let mut engine = water_drop_engine();
        assert!(engine.mass_mover_create(1, 0, false));
        engine.tick_mass_movers();
        // Pass 1: slot 1 moves the drop (1,0)->(1,1), spawns slot 2.
        // Pass 2: slot 2 moves (1,1)->(1,2) and spawns slot 3; slot 1 then
        // ceases (its pixel dried out). (Seed 2 draws no immediate-execute
        // Rnd3 zeros here — pinned by the RNG expectation test above.)
        let water = engine.materials.id_of("Water").expect("water");
        let at = |engine: &Engine, x: i32, y: i32| {
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(x, y))
        };
        assert_eq!(at(&engine, 1, 2), Some(water), "drop fell two pixels");
        assert_eq!(at(&engine, 1, 0), None);
        assert_eq!(at(&engine, 1, 1), None);
        assert_eq!(
            engine.mass_movers.slot(3).map(|m| (m.x, m.y)),
            Some((1, 2)),
            "pass-2 move spawned the slot-3 mover at the target"
        );
        assert_eq!(engine.mass_movers.slot(1), None, "the dried mover ceased");
    }

    #[test]
    fn count_ledger_double_counts_live_movers_across_passes() {
        // C4MassMoverSet::Execute (C4MassMover.cpp:54-62): Count resets to
        // zero, then EVERY live mover bumps it once per speed pass, plus
        // the Init bumps of movers created along the way.
        let mut engine = water_drop_engine();
        assert!(engine.mass_mover_create(1, 0, false));
        assert_eq!(engine.mass_movers.count(), 1);
        engine.tick_mass_movers();
        // Pass 1: slot 1 counted (+1), its move Inits slot 2 (+1).
        // Pass 2: slot 2 counted (+1), its move Inits slot 3 (+1); slot 1
        // counted (+1) then ceases (-1). Total: 4.
        assert_eq!(engine.mass_movers.count(), 4);
    }

    #[test]
    fn extract_material_fires_check_instability_range_at_cleared_pixel() {
        // C4Landscape::ExtractMaterial (C4Landscape.cpp:1148-1156) ends in
        // CheckInstabilityRange at the cleared coordinates — extraction
        // next to remaining liquid immediately re-arms a mover.
        let materials = water_materials();
        let water = materials.id_of("Water").expect("water material");
        let mut landscape = Landscape::flat(5, 10);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(4, 4, Some(water))],
        );
        landscape.set_liquid_column(
            2,
            vec![crate::LiquidSegment::with_material(4, 4, Some(water))],
        );
        let mut engine = engine_with(materials, landscape);
        let extracted = engine.extract_material(2, 4);
        assert_eq!(extracted, Some(water));
        // (2,4) is now dry; the (tx-1,ty) fallback probe finds (1,4).
        let mover = engine.mass_movers.slot(1).expect("range probe created a mover");
        assert_eq!((mover.x, mover.y), (1, 4));
    }

    #[test]
    fn synchronize_consolidates_slots_and_resets_create_ptr() {
        // C4MassMoverSet::Consolidate (C4MassMover.cpp:219-247) via
        // Synchronize (:249-252): live movers pack down in order and
        // CreatePtr resets to 0.
        let mut engine = water_drop_engine();
        let water = engine.materials.id_of("Water").expect("water");
        let mover = |x: i32, y: i32| MassMover { mat: water, x, y };
        engine.mass_movers.fill_slot(3, mover(1, 0));
        engine.mass_movers.fill_slot(7, mover(2, 0));
        engine.mass_movers.fill_slot(9, mover(0, 0));
        engine.mass_movers.synchronize();
        assert_eq!(engine.mass_movers.create_ptr(), 0);
        assert_eq!(engine.mass_movers.slot(0).map(|m| (m.x, m.y)), Some((1, 0)));
        assert_eq!(engine.mass_movers.slot(1).map(|m| (m.x, m.y)), Some((2, 0)));
        assert_eq!(engine.mass_movers.slot(2).map(|m| (m.x, m.y)), Some((0, 0)));
        assert_eq!(engine.mass_movers.slot(3), None);
        assert_eq!(engine.mass_movers.slot(7), None);
        assert_eq!(engine.mass_movers.slot(9), None);
    }

    #[test]
    fn blocked_mover_incinerate_spawns_flam_then_extracts_material_without_rng() {
        // mrfIncinerate on meeMassMove calls the full Landscape::Incinerate:
        // a successful FLAM creation consumes the reaction, then the mover's
        // pixel is extracted (C4Material.cpp:754-757; C4MassMover.cpp:126-132).
        let (mut engine, oil) = blocked_incinerating_mover_engine();
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(1, 4)),
            Some(oil),
            "fixture: blocked Oil mover"
        );
        let rng_before = (engine.rng.count, engine.rng.hold, engine.rng.rnd3_ptr());

        engine.tick_mass_movers();

        let fires = engine
            .objects
            .iter()
            .filter(|object| {
                !object.destroyed
                    && object.state.status.is_active()
                    && object.definition_id == crate::FIRE_DEFINITION_ID
            })
            .collect::<Vec<_>>();
        assert_eq!(fires.len(), 1, "exactly one FLAM is created");
        assert_eq!(fires[0].state.position, crate::Vector2::new(1, 4));
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(1, 4)),
            None,
            "the consumed Oil pixel is extracted"
        );
        assert_eq!(engine.mass_movers.live_movers(), 0);
        assert_eq!(
            (engine.rng.count, engine.rng.hold, engine.rng.rnd3_ptr()),
            rng_before,
            "Incinerate and FLAM creation consume no synced RNG"
        );
    }

    #[test]
    fn blocked_mover_incinerate_with_nearby_flam_is_unhandled_and_ceases() {
        // C4Landscape::Incinerate returns false when FindObject locates a
        // FLAM in (x-4,y-1,8,20). The mass-mover reaction must remain
        // unhandled: try its other contact directions, then Cease without
        // extracting the Oil pixel.
        let (mut engine, oil) = blocked_incinerating_mover_engine();
        engine
            .spawn_object(
                crate::SpawnConfig::new(crate::FIRE_DEFINITION_ID)
                    .with_position(crate::Vector2::new(1, 4)),
            )
            .expect("existing FLAM spawns inside the suppression rect");
        let rng_before = (engine.rng.count, engine.rng.hold, engine.rng.rnd3_ptr());

        engine.tick_mass_movers();

        let fire_count = engine
            .objects
            .iter()
            .filter(|object| {
                !object.destroyed
                    && object.state.status.is_active()
                    && object.definition_id == crate::FIRE_DEFINITION_ID
            })
            .count();
        assert_eq!(fire_count, 1, "the nearby FLAM suppresses creation");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(1, 4)),
            Some(oil),
            "an unhandled reaction leaves the Oil pixel in place"
        );
        assert_eq!(engine.mass_movers.live_movers(), 0, "the blocked mover ceases");
        assert_eq!(
            (engine.rng.count, engine.rng.hold, engine.rng.rnd3_ptr()),
            rng_before,
            "the failed Incinerate path consumes no synced RNG"
        );
    }

    #[test]
    fn blocked_mover_uses_massmove_corrosion_rng_before_dying() {
        // C4MassMover::Execute corrosion check (C4MassMover.cpp:126-132)
        // with the mrfCorrode meeMassMove draws (C4Material.cpp:699-714).
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
        let mut engine = engine_with(materials, landscape);
        assert!(engine.mass_mover_create(1, 4, false));

        let mut expected = engine.rng.clone();
        assert!(crate::material::evaluate_corrosion(
            100,
            100,
            None,
            &mut expected
        ));
        crate::material::consume_corrosion_effect_rng(&mut expected);

        engine.tick_mass_movers();

        assert_eq!(engine.rng.count, expected.count);
        assert_eq!(engine.rng.hold, expected.hold);
    }

    #[test]
    fn blocked_mover_runs_system_global_mass_move_script_reaction_like_cpp() {
        // mrfScript on meeMassMove (C4MassMover.cpp:126-132 +
        // C4Material.cpp:800-835): the reaction script runs at the
        // corrosion-check position with xdir=ydir=0 and pfPosChanged=null —
        // only the return value matters: truthy consumes the material
        // (ExtractMaterial), and the by-ref write-backs land in discarded
        // locals.
        let materials = materials(
            r#"
            [Material Goo]
            Name=Goo
            Density=25
            Instable=1
            MaxSlide=0

            [Reaction]
            Type=Script
            ScriptFunc=GooEats
            TargetSpec=Rock

            [Material Rock]
            Name=Rock
            Density=80
            "#,
        );
        let goo = materials.id_of("Goo").expect("goo material");
        let rock = materials.id_of("Rock").expect("rock material");
        let mut landscape = Landscape::flat_with_material(3, 5, Some(rock));
        landscape.set_default_liquid_material(Some(goo));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(4, 4, Some(goo))],
        );
        let mut engine = engine_with(materials, landscape);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/GooReaction.c".to_string(),
                r#"
                func IsBlockedGoo(xdir, ydir, ls_mat, event) {
                    // meeMassMove = 2; dirs are Fix0; the landscape side is
                    // a real material
                    if (event != 2) { return 0; }
                    if (xdir != 0) { return 0; }
                    if (ydir != 0) { return 0; }
                    if (ls_mat < 0) { return 0; }
                    return 1;
                }
                global func GooEats(x, y, lsx, lsy, xdir, ydir, pxs_mat, ls_mat, event) {
                    // A Game.ScriptEngine SFunc retains its declaring
                    // System.c4g host for ordinary local-helper lookup.
                    return IsBlockedGoo(xdir, ydir, ls_mat, event);
                }
                "#
                .to_string(),
            )]),
            1,
            "System.c4g reaction installs without any scenario script"
        );
        assert!(engine.mass_mover_create(1, 4, false));

        let had_goo = engine
            .landscape
            .as_ref()
            .and_then(|landscape| landscape.material_at(1, 4));
        assert_eq!(had_goo, Some(goo), "fixture: goo sits at (1,4)");

        engine.tick_mass_movers();

        let remaining = engine
            .landscape
            .as_ref()
            .and_then(|landscape| landscape.material_at(1, 4));
        assert_eq!(
            remaining, None,
            "the truthy script return consumed the goo (ExtractMaterial)"
        );
    }

    #[test]
    fn material_script_set_pre_send_is_deferred_to_the_fatal_tick_boundary() {
        let materials = materials(
            r#"
            [Material Goo]
            Name=Goo
            Density=25
            Instable=1
            MaxSlide=0

            [Reaction]
            Type=Script
            ScriptFunc=GooEats
            TargetSpec=Rock

            [Material Rock]
            Name=Rock
            Density=80
            "#,
        );
        let goo = materials.id_of("Goo").expect("goo material");
        let rock = materials.id_of("Rock").expect("rock material");
        let mut landscape = Landscape::flat_with_material(3, 5, Some(rock));
        landscape.set_default_liquid_material(Some(goo));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(4, 4, Some(goo))],
        );
        let mut engine = engine_with(materials, landscape);
        engine.set_network_game(true);
        engine
            .install_scenario_script(
                "Scenario",
                "global func GooEats() { SetPreSend(30); return 0; }",
            )
            .expect("scenario installs");
        assert!(engine.mass_mover_create(1, 4, false));

        engine.tick_mass_movers();
        let error = engine
            .tick()
            .expect_err("material fail-safe must retain the fatal boundary");
        assert!(matches!(
            error,
            crate::EngineError::RuntimeFlashProducerBoundary {
                producer: crate::RuntimeFlashProducerBoundary::ScriptTargetFps,
                recovery: None,
            }
        ));
    }

    /// Grid world: 5x5, water byte 20, earth byte 30 (DigFree).
    fn grid_engine(bytes: Vec<u8>) -> Engine {
        let materials = materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Instable=1
            MaxSlide=0

            [Material Earth]
            Name=Earth
            Density=50
            DigFree=1
            "#,
        );
        let mut densities = vec![0i32; 128];
        densities[20] = 25;
        densities[30] = 50;
        let mut names: Vec<Option<String>> = vec![None; 128];
        names[20] = Some("Water".into());
        names[30] = Some("Earth".into());
        let grid =
            crate::landscape::PixelGrid::new(5, 5, bytes, densities, names, vec![None; 128]);
        let mut landscape = Landscape::new(5, vec![5, 5, 5, 5, 5]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        let mut engine = Engine::with_seed(2);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine
    }

    #[test]
    fn temperature_scan_checks_instability_after_each_converted_pixel() {
        // C4Landscape::DoScan calls CheckInstabilityRange immediately after
        // every SetPix. When the left Water pixel freezes first, its right
        // fallback must therefore arm the still-liquid neighbor before that
        // neighbor is converted by the next scanned column.
        let materials = materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Instable=1
            BelowTempConvert=0
            BelowTempConvertTo=Ice
            TempConvStrength=0

            [Material Ice]
            Name=Ice
            Density=80
            "#,
        );
        let water = materials.id_of("Water").expect("water material");
        let ice = materials.id_of("Ice").expect("ice material");
        let water_byte = (water.index() + 1) as u8;
        let mut densities = vec![0; 128];
        let mut names = vec![None; 128];
        densities[water_byte as usize] = 25;
        densities[ice.index() + 1] = 80;
        names[water_byte as usize] = Some("Water".into());
        names[ice.index() + 1] = Some("Ice".into());
        let grid = crate::landscape::PixelGrid::new(
            2,
            2,
            vec![0, 0, water_byte, water_byte],
            densities,
            names,
            vec![None; 128],
        );
        let mut landscape = Landscape::new(2, vec![2, 2]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        let mut engine = engine_with(materials, landscape);
        engine.environment.temperature = -1;

        engine.apply_landscape_temperature_conversions();

        let landscape = engine.landscape.as_ref().expect("landscape remains");
        assert_eq!(landscape.material_at(0, 1), Some(ice));
        assert_eq!(landscape.material_at(1, 1), Some(ice));
        assert_eq!(engine.mass_movers.live_movers(), 1);
        let mover = engine.mass_movers.slot(1).expect("right Water was armed");
        assert_eq!((mover.mat, mover.x, mover.y), (water, 1, 1));
    }

    #[test]
    fn dig_free_pix_fires_check_instability_range() {
        // C4Landscape::DigFreePix (C4Landscape.cpp:936-944): the trailing
        // CheckInstabilityRange fires at the probed pixel — digging earth
        // NEXT to water re-arms the water via the (tx-1,ty) fallback probe.
        let mut bytes = vec![0u8; 25];
        bytes[2 * 5 + 1] = 20; // water at (1,2)
        bytes[2 * 5 + 2] = 30; // earth at (2,2)
        let mut engine = grid_engine(bytes);
        let water = engine.materials.id_of("Water").expect("water");
        let dug = engine.dig_free_pix(2, 2);
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(2, 2)),
            None,
            "DigFree earth cleared"
        );
        assert_eq!(
            dug.and_then(|id| engine.materials.get_by_id(id)).map(|m| m.density()),
            Some(50),
            "DigFreePix returns the dug material"
        );
        let mover = engine.mass_movers.slot(1).expect("side probe armed the water");
        assert_eq!((mover.mat, mover.x, mover.y), (water, 1, 2));
    }

    #[test]
    fn dig_free_pix_probes_even_undiggable_pixels() {
        // C4Landscape::DigFreePix (C4Landscape.cpp:942): the instability
        // probe runs even when nothing clears — probing sky next to water
        // still creates the mover.
        let mut bytes = vec![0u8; 25];
        bytes[2 * 5 + 1] = 20; // water at (1,2); (2,2) stays sky
        let mut engine = grid_engine(bytes);
        let dug = engine.dig_free_pix(2, 2);
        assert_eq!(dug, None);
        let mover = engine.mass_movers.slot(1).expect("probe fired on a sky dig");
        assert_eq!((mover.x, mover.y), (1, 2));
    }

    #[test]
    fn mass_move_poof_reaction_extracts_and_fires_instability() {
        // mrfPoof on meeMassMove (C4Material.cpp:669-670) runs a REAL
        // ExtractMaterial — whose CheckInstabilityRange (C4Landscape.cpp:
        // 1154) re-arms neighbouring instable material.
        let materials = materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Instable=1
            MaxSlide=0
            Extinguisher=1

            [Material Lava]
            Name=Lava
            Density=50
            Incindiary=1
            "#,
        );
        let mut densities = vec![0i32; 128];
        densities[20] = 25;
        densities[30] = 50;
        let mut names: Vec<Option<String>> = vec![None; 128];
        names[20] = Some("Water".into());
        names[30] = Some("Lava".into());
        let mut bytes = vec![0u8; 25];
        bytes[3 * 5 + 2] = 30; // lava at (2,3)
        bytes[3 * 5 + 1] = 20; // water at (1,3)
        let grid =
            crate::landscape::PixelGrid::new(5, 5, bytes, densities, names, vec![None; 128]);
        let mut landscape = Landscape::new(5, vec![5, 5, 5, 5, 5]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        let water = materials.id_of("Water").expect("water");
        let mut engine = Engine::with_seed(2);
        engine.set_materials(materials);
        engine.set_landscape(landscape);

        let result = engine.execute_mass_move_reaction(water, 2, 2, 2, 3);
        assert!(result.consumes_material(), "water+lava poofs");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(2, 3)),
            None,
            "the lava pixel was extracted"
        );
        let mover = engine.mass_movers.slot(1).expect("instability armed the water");
        assert_eq!((mover.mat, mover.x, mover.y), (water, 1, 3));
    }

    #[test]
    fn exec_masked_mass_move_occupies_slot_without_poof_or_rng() {
        // Water+Lava naturally selects builtin Poof. The custom pair entry
        // replaces that shared C++ table slot, but ExecMask=2 excludes
        // meeMassMove: the blocked mover gets an unhandled no-op, not the
        // builtin fallback (which would extract both pixels and draw Rnd3).
        let materials = materials(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Instable=1
            MaxSlide=0
            Extinguisher=1

            [Reaction]
            Type=Poof
            TargetSpec=Lava
            ExecMask=2

            [Material Lava]
            Name=Lava
            Density=50
            Incindiary=1
            "#,
        );
        let water = materials.id_of("Water").expect("water material");
        let lava = materials.id_of("Lava").expect("lava material");
        let reaction = materials.reaction_for_event(
            Some(water),
            Some(lava),
            MaterialInteractionEvent::MassMove,
        );
        assert_eq!(reaction.kind, crate::material::MaterialReactionKind::None);
        assert!(reaction.user_defined, "masked MassMove still owns the pair slot");

        let mut landscape = Landscape::flat_with_material(3, 5, Some(lava));
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(4, 4, Some(water))],
        );
        let mut engine = engine_with(materials, landscape);
        assert!(engine.mass_mover_create(1, 4, false));
        let mirror = engine.rng.clone();

        engine.tick_mass_movers();

        assert_eq!(engine.rng, mirror, "masked MassMove draws no reaction RNG");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(1, 4)),
            Some(water),
            "the mover material is not extracted"
        );
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(1, 5)),
            Some(lava),
            "the contacted landscape material is not poofed"
        );
        assert_eq!(engine.pxs_system.count(), 0);
        assert_eq!(engine.mass_movers.live_movers(), 0, "blocked mover simply ceases");
    }

    #[test]
    fn pxs_move_corrode_clears_in_place_and_fires_instability() {
        // mrfCorrode on meePXSMove (C4Material.cpp:731-733): ClearBackPix
        // (= ClearPix, C4Wrappers.h:92) clears the landscape pixel IN
        // PLACE — no FindMatTop — then CheckInstabilityRange fires at that
        // exact pixel.
        let materials = materials(
            r#"
            [Material Acid]
            Name=Acid
            Density=25
            Instable=1
            MaxSlide=0
            Corrosive=100

            [Material Water]
            Name=Water
            Density=25
            Instable=1
            MaxSlide=0

            [Material Rock]
            Name=Rock
            Density=80
            Corrode=100
            "#,
        );
        let mut densities = vec![0i32; 128];
        densities[20] = 25; // acid
        densities[25] = 25; // water
        densities[30] = 80; // rock
        let mut names: Vec<Option<String>> = vec![None; 128];
        names[20] = Some("Acid".into());
        names[25] = Some("Water".into());
        names[30] = Some("Rock".into());
        let mut bytes = vec![0u8; 25];
        bytes[2 * 5 + 2] = 30; // rock at (2,2) — above the corroded pixel
        bytes[3 * 5 + 2] = 30; // rock at (2,3) — the corroded pixel
        bytes[3 * 5 + 1] = 25; // water at (1,3)
        let grid =
            crate::landscape::PixelGrid::new(5, 5, bytes, densities, names, vec![None; 128]);
        let mut landscape = Landscape::new(5, vec![5, 5, 5, 5, 5]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        let acid = materials.id_of("Acid").expect("acid");
        let water = materials.id_of("Water").expect("water");
        let rock = materials.id_of("Rock").expect("rock");
        let mut engine = Engine::with_seed(2);
        engine.set_materials(materials);
        engine.set_landscape(landscape);

        let reaction = engine.materials.reaction_for_event(
            Some(acid),
            Some(rock),
            MaterialInteractionEvent::PxsMove,
        );
        let mut pixel = crate::pxs::Pxs {
            mat: acid,
            x: crate::math::itofix(2),
            y: crate::math::itofix(1),
            xdir: crate::math::C4Fixed::ZERO,
            ydir: crate::math::C4Fixed::ZERO,
        };
        let (mut x, mut y) = (2, 1);
        let mut pos_changed = false;
        let died = engine.execute_pxs_reaction(
            reaction,
            &mut x,
            &mut y,
            2,
            3,
            &mut pixel,
            Some(rock),
            MaterialInteractionEvent::PxsMove,
            &mut pos_changed,
        );
        assert!(died, "corrosion at 100/100 always consumes the PXS");
        let at = |engine: &Engine, x: i32, y: i32| {
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(x, y))
        };
        assert_eq!(at(&engine, 2, 3), None, "the corroded pixel cleared IN PLACE");
        assert_eq!(
            at(&engine, 2, 2),
            Some(rock),
            "no FindMatTop: the rock above the corroded pixel survives"
        );
        let mover = engine.mass_movers.slot(1).expect("instability armed the water");
        assert_eq!((mover.mat, mover.x, mover.y), (water, 1, 3));
    }

    #[test]
    fn blast_circle_probes_instability_across_the_crater() {
        // C4Landscape::BlastFree (C4Landscape.cpp:1056-1063): every pixel
        // of the blast circle runs BlastFreePix, which ends in
        // CheckInstabilityRange (:975) even when nothing was blasted free
        // — a blast INTO water arms movers.
        let materials = water_materials();
        let water = materials.id_of("Water").expect("water material");
        let mut landscape = Landscape::flat(5, 10);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(2, 2, Some(water))],
        );
        let mut engine = engine_with(materials, landscape);
        assert_eq!(engine.mass_movers.live_movers(), 0);
        let _ = engine.blast_circle(crate::Vector2::new(2, 2), 1, None);
        let mover = engine.mass_movers.slot(1).expect("blast probe armed the water");
        assert_eq!((mover.mat, mover.x, mover.y), (water, 1, 2));
    }

    #[test]
    fn shake_free_probes_instability_across_the_circle() {
        // C4Landscape::ShakeFree (C4Landscape.cpp:1017-1028): every pixel
        // runs ShakeFreePix, which ends in CheckInstabilityRange (:955)
        // unconditionally.
        let materials = water_materials();
        let water = materials.id_of("Water").expect("water material");
        let mut landscape = Landscape::flat(5, 10);
        landscape.set_default_liquid_material(Some(water));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(2, 2, Some(water))],
        );
        let mut engine = engine_with(materials, landscape);
        engine.execute_shake_circle_operation(crate::Vector2::new(2, 2), 1);
        let mover = engine.mass_movers.slot(1).expect("shake probe armed the water");
        assert_eq!((mover.mat, mover.x, mover.y), (water, 1, 2));
    }

    #[test]
    fn game_start_synchronize_consolidates_the_mover_set() {
        // C4Game::Synchronize calls MassMover.Synchronize()
        // (C4Game.cpp:3700) = Consolidate (C4MassMover.cpp:249-252).
        let mut engine = water_drop_engine();
        let water = engine.materials.id_of("Water").expect("water");
        engine.mass_movers.fill_slot(4, MassMover { mat: water, x: 1, y: 0 });
        engine
            .game_start_synchronize()
            .expect("game-start synchronization succeeds");
        assert_eq!(engine.mass_movers.create_ptr(), 0);
        assert_eq!(engine.mass_movers.slot(0).map(|m| (m.x, m.y)), Some((1, 0)));
        assert_eq!(engine.mass_movers.slot(4), None);
    }

    #[test]
    fn blocked_mover_convert_reaction_transfers_to_pxs() {
        // mrfConvert on meeMassMove (C4Material.cpp:654-657): conversion-
        // transfer — the MOVER's material spawns as a PXS at the mover
        // position (the convert target is ignored on this event) and the
        // reaction reports consumed, so Execute extracts the pixel.
        let materials = materials(
            r#"
            [Material Goo]
            Name=Goo
            Density=25
            Instable=1
            MaxSlide=0

            [Reaction]
            Type=Convert
            TargetSpec=Rock
            ConvertMat=Rock

            [Material Rock]
            Name=Rock
            Density=80
            "#,
        );
        let goo = materials.id_of("Goo").expect("goo material");
        let rock = materials.id_of("Rock").expect("rock material");
        let mut landscape = Landscape::flat_with_material(3, 5, Some(rock));
        landscape.set_default_liquid_material(Some(goo));
        landscape.set_liquid_column(
            1,
            vec![crate::LiquidSegment::with_material(4, 4, Some(goo))],
        );
        let mut engine = engine_with(materials, landscape);
        assert!(engine.mass_mover_create(1, 4, false));

        engine.tick_mass_movers();

        let pxs: Vec<_> = engine.pxs_system.iter().collect();
        assert_eq!(pxs.len(), 1, "exactly one conversion-transfer PXS");
        assert_eq!(pxs[0].mat, goo, "the PXS carries the MOVER's material");
        assert_eq!(
            (crate::math::fixtoi(pxs[0].x), crate::math::fixtoi(pxs[0].y)),
            (1, 4),
            "spawned at the mover position"
        );
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(1, 4)),
            None,
            "the consumed goo was extracted"
        );
    }

    #[test]
    fn snapshot_round_trip_preserves_slot_indices_and_ledger() {
        // Save/load keeps the slot ORDER (iteration order is
        // determinism-critical, C4MassMover.cpp:56-63) and the
        // CreatePtr/Count ledger.
        let mut engine = water_drop_engine();
        let water = engine.materials.id_of("Water").expect("water");
        engine.mass_movers.fill_slot(5, MassMover { mat: water, x: 1, y: 0 });
        engine.mass_movers.bump_count();
        let json = serde_json::to_string(&engine.mass_movers).expect("serializes");
        let restored: MassMoverSet = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(restored.create_ptr(), 5);
        assert_eq!(restored.count(), 1);
        assert_eq!(restored.slot(5).map(|m| (m.x, m.y)), Some((1, 0)));
        assert_eq!(restored.live_movers(), 1);
    }
}
