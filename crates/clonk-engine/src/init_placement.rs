//! C4Game environment init placements (src/C4Game.cpp:2493-2503):
//! InitVegetation, InitInEarth, InitAnimals, InitEnvironment, InitRules,
//! InitGoals — run after the definitions' InitializeDef callbacks and
//! before Weather.Init, gated on `!C4S.Head.NoInitialize && LandscapeLoaded`.
//! Every Random here comes from the synced ledger; the draw order is the
//! placement parity contract.

use crate::scenario::LegacyInitPlacement;
use crate::{
    DefinitionId, Engine, MaterialId, ObjectId, SpawnConfig, Vector2, CATEGORY_GOAL, FULL_CON,
    OWNER_NONE,
};

/// `maxvid`/`maxidlist` (C4Game.cpp:2931,3065,3080).
const MAX_VID: usize = 100;

impl Engine {
    fn gback_wdt(&self) -> i32 {
        self.landscape
            .as_ref()
            .map(|landscape| landscape.width() as i32)
            .unwrap_or(0)
    }

    fn gback_hgt(&self) -> i32 {
        self.landscape
            .as_ref()
            .map(|landscape| landscape.estimated_height())
            .unwrap_or(0)
    }

    fn gback_semi_solid(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .is_some_and(|landscape| landscape.is_semi_solid_at(x, y))
    }

    fn gback_solid(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .is_some_and(|landscape| landscape.is_solid_at(x, y))
    }

    fn gback_liquid(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .is_some_and(|landscape| landscape.is_liquid_at(x, y))
    }

    fn gback_ift(&self, x: i32, y: i32) -> bool {
        self.landscape
            .as_ref()
            .is_some_and(|landscape| landscape.is_ift_at(x, y))
    }

    fn gback_mat(&self, x: i32, y: i32) -> Option<MaterialId> {
        self.landscape
            .as_ref()
            .and_then(|landscape| landscape.border_material_at(x, y))
    }

    /// AboveSemiSolid (C4Landscape.cpp:1741-1761): nearest free above
    /// semi-solid, searching up and down simultaneously.
    fn above_semi_solid(&self, rx: i32, ry: &mut i32) -> bool {
        let gback_hgt = self.gback_hgt();
        let mut cy1 = *ry;
        let mut cy2 = *ry;
        let mut use_upwards_next_free = false;
        let mut use_downwards_next_solid = false;
        while cy1 >= 0 || cy2 < gback_hgt {
            if cy1 >= 0 {
                if self.gback_semi_solid(rx, cy1) {
                    use_upwards_next_free = true;
                } else if use_upwards_next_free {
                    *ry = cy1;
                    return true;
                }
            }
            if cy2 < gback_hgt {
                if !self.gback_semi_solid(rx, cy2) {
                    use_downwards_next_solid = true;
                } else if use_downwards_next_solid {
                    *ry = cy2;
                    return true;
                }
            }
            cy1 -= 1;
            cy2 += 1;
        }
        false
    }

    /// AboveSolid (C4Landscape.cpp:1763-1788): nearest free directly
    /// above solid.
    fn above_solid(&self, rx: i32, ry: &mut i32) -> bool {
        let gback_hgt = self.gback_hgt();
        let mut cy1 = *ry;
        let mut cy2 = *ry;
        while cy1 >= 0 || cy2 < gback_hgt {
            if cy1 >= 0 && !self.gback_semi_solid(rx, cy1) && self.gback_solid(rx, cy1 + 1) {
                *ry = cy1;
                return true;
            }
            if cy2 + 1 < gback_hgt
                && !self.gback_semi_solid(rx, cy2)
                && self.gback_solid(rx, cy2 + 1)
            {
                *ry = cy2;
                return true;
            }
            cy1 -= 1;
            cy2 += 1;
        }
        false
    }

    /// SemiAboveSolid (C4Landscape.cpp:1790-1815): nearest free/semi
    /// above solid.
    fn semi_above_solid(&self, rx: i32, ry: &mut i32) -> bool {
        let gback_hgt = self.gback_hgt();
        let mut cy1 = *ry;
        let mut cy2 = *ry;
        while cy1 >= 0 || cy2 < gback_hgt {
            if cy1 >= 0 && !self.gback_solid(rx, cy1) && self.gback_solid(rx, cy1 + 1) {
                *ry = cy1;
                return true;
            }
            if cy2 + 1 < gback_hgt && !self.gback_solid(rx, cy2) && self.gback_solid(rx, cy2 + 1) {
                *ry = cy2;
                return true;
            }
            cy1 -= 1;
            cy2 += 1;
        }
        false
    }

    /// FindLiquidHeight (C4Landscape.cpp:1817-1842).
    fn find_liquid_height(&self, cx: i32, ry: &mut i32, hgt: i32) -> bool {
        let gback_hgt = self.gback_hgt();
        let mut cy1 = *ry;
        let mut cy2 = *ry;
        let mut rl1 = 0;
        let mut rl2 = 0;
        while cy1 >= 0 || cy2 < gback_hgt {
            if cy1 >= 0 {
                if self.gback_liquid(cx, cy1) {
                    rl1 += 1;
                    if rl1 >= hgt {
                        *ry = cy1 + hgt / 2;
                        return true;
                    }
                } else {
                    rl1 = 0;
                }
            }
            if cy2 + 1 < gback_hgt {
                if self.gback_liquid(cx, cy2) {
                    rl2 += 1;
                    if rl2 >= hgt {
                        *ry = cy2 - hgt / 2;
                        return true;
                    }
                } else {
                    rl2 = 0;
                }
            }
            cy1 -= 1;
            cy2 += 1;
        }
        false
    }

    /// FindSolidGround (C4Landscape.cpp:1848-1876): a `width` run of
    /// solid ground; returns bottom center of the surface found.
    fn find_solid_ground(&self, rx: &mut i32, ry: &mut i32, width: i32) -> bool {
        let gback_wdt = self.gback_wdt();
        let mut found = false;
        let mut cx1 = *rx;
        let mut cx2 = *rx;
        let mut cy1 = *ry;
        let mut cy2 = *ry;
        let mut rl1 = 0;
        let mut rl2 = 0;
        while cx1 > 0 || cx2 < gback_wdt {
            if cx1 >= 0 {
                if self.above_solid(cx1, &mut cy1) {
                    rl1 += 1;
                } else {
                    rl1 = 0;
                }
            }
            if cx2 < gback_wdt {
                if self.above_solid(cx2, &mut cy2) {
                    rl2 += 1;
                } else {
                    rl2 = 0;
                }
            }
            if rl1 >= width {
                *rx = cx1 + rl1 / 2;
                *ry = cy1;
                found = true;
                break;
            }
            if rl2 >= width {
                *rx = cx2 - rl2 / 2;
                *ry = cy2;
                found = true;
                break;
            }
            cx1 -= 1;
            cx2 += 1;
        }
        if found {
            let x = *rx;
            self.above_semi_solid(x, ry);
        }
        found
    }

    /// FindSurfaceLiquid (C4Landscape.cpp:1878-1912): a `width` run of
    /// surface liquid at least `height` deep.
    fn find_surface_liquid(&self, rx: &mut i32, ry: &mut i32, width: i32, height: i32) -> bool {
        let gback_wdt = self.gback_wdt();
        let mut found = false;
        let mut cx1 = *rx;
        let mut cx2 = *rx;
        let mut cy1 = *ry;
        let mut cy2 = *ry;
        let mut rl1 = 0;
        let mut rl2 = 0;
        while cx1 > 0 || cx2 < gback_wdt {
            if cx1 > 0 {
                if !self.above_semi_solid(cx1, &mut cy1) {
                    cx1 = -1; // Abort left
                } else {
                    let lokay = (0..height).all(|cnt| self.gback_liquid(cx1, cy1 + 1 + cnt));
                    if lokay {
                        rl1 += 1;
                    } else {
                        rl1 = 0;
                    }
                }
            }
            if cx2 < gback_wdt {
                if !self.above_semi_solid(cx2, &mut cy2) {
                    cx2 = gback_wdt; // Abort right
                } else {
                    let lokay = (0..height).all(|cnt| self.gback_liquid(cx2, cy2 + 1 + cnt));
                    if lokay {
                        rl2 += 1;
                    } else {
                        rl2 = 0;
                    }
                }
            }
            if rl1 >= width {
                *rx = cx1 + rl1 / 2;
                *ry = cy1;
                found = true;
                break;
            }
            if rl2 >= width {
                *rx = cx2 - rl2 / 2;
                *ry = cy2;
                found = true;
                break;
            }
            cx1 -= 1;
            cx2 += 1;
        }
        if found {
            let x = *rx;
            self.above_semi_solid(x, ry);
        }
        found
    }

    /// FindLiquid (C4Landscape.cpp:1914-1934).
    fn find_liquid(&self, rx: &mut i32, ry: &mut i32, width: i32, height: i32) -> bool {
        let gback_wdt = self.gback_wdt();
        let mut cx1 = *rx;
        let mut cx2 = *rx;
        let mut cy1 = *ry;
        let mut cy2 = *ry;
        let mut rl1 = 0;
        let mut rl2 = 0;
        while cx1 > 0 || cx2 < gback_wdt {
            if cx1 > 0 {
                if self.find_liquid_height(cx1, &mut cy1, height) {
                    rl1 += 1;
                } else {
                    rl1 = 0;
                }
            }
            if cx2 < gback_wdt {
                if self.find_liquid_height(cx2, &mut cy2, height) {
                    rl2 += 1;
                } else {
                    rl2 = 0;
                }
            }
            if rl1 >= width {
                *rx = cx1 + rl1 / 2;
                *ry = cy1;
                return true;
            }
            if rl2 >= width {
                *rx = cx2 - rl2 / 2;
                *ry = cy2;
                return true;
            }
            cx1 -= 1;
            cx2 += 1;
        }
        false
    }

    /// The def's Shape.Wdt/Hgt, Placement and Growth for placement
    /// decisions (C4Id2Def) — None = unknown definition.
    fn placement_def(&self, id: &str) -> Option<(i32, i32, i32, i32)> {
        self.definitions.get(&DefinitionId::from(id)).map(|def| {
            let shape = def.shape_rect();
            (
                shape.map(|rect| rect.width).unwrap_or(0),
                shape.map(|rect| rect.height).unwrap_or(0),
                def.placement(),
                def.growth(),
            )
        })
    }

    /// The `Soil` flag of a material (C4Material `Soil=`).
    fn material_is_soil(&self, material: MaterialId) -> bool {
        self.materials
            .get_by_id(material)
            .and_then(|material| material.definition().int("Soil"))
            .unwrap_or(0)
            != 0
    }

    /// `Game.CreateObject(id, nullptr, NO_OWNER, x, y, r)` for the init
    /// placements: the engine spawn path runs the C++ NewObject semantics
    /// (DoCon initial adjust + Construction/Initialize callbacks).
    fn init_create_object(&mut self, id: &str, x: i32, y: i32, rotation: i32) -> Option<ObjectId> {
        // C4Id2Def failure: silent nullptr (C4Game.cpp:1146).
        if !self.definitions.contains_key(&DefinitionId::from(id)) {
            return None;
        }
        let config = SpawnConfig::new(id.to_string())
            .with_position(Vector2::new(x, y))
            .with_rotation(rotation)
            .with_owner(OWNER_NONE);
        self.spawn_object_with_initial_lifecycle(config, None)
            .unwrap_or_default()
    }

    /// `Game.CreateObjectConstruction(id, nullptr, NO_OWNER, x, by, con)`
    /// with fTerrain=false (PlaceVegetation, C4Game.cpp:3004,3021).
    fn init_create_object_construction(
        &mut self,
        id: &str,
        x: i32,
        bottom_y: i32,
        con: i32,
    ) -> Option<ObjectId> {
        if !self.definitions.contains_key(&DefinitionId::from(id)) {
            return None;
        }
        let config = SpawnConfig::new(id.to_string())
            .with_position(Vector2::new(x, bottom_y))
            .with_owner(OWNER_NONE)
            .with_construction(con);
        self.spawn_object_with_initial_lifecycle(config, None)
            .unwrap_or_default()
    }

    /// C4Game::PlaceInEarth (C4Game.cpp:2949-2960): 35 cheap tries at a
    /// random earth pixel; the rotation Random(360) draws only on an
    /// earth hit (argument evaluation precedes the C4Id2Def check).
    pub(crate) fn place_in_earth(&mut self, id: &str, earth_material: Option<MaterialId>) -> bool {
        let gback_wdt = self.gback_wdt();
        let gback_hgt = self.gback_hgt();
        for _ in 0..35 {
            let tx = self.rng.random(gback_wdt);
            let ty = self.rng.random(gback_hgt);
            if earth_material.is_some() && self.gback_mat(tx, ty) == earth_material {
                let rotation = self.rng.random(360);
                if self.init_create_object(id, tx, ty, rotation).is_some() {
                    return true;
                }
            }
        }
        false
    }

    /// C4Game::PlaceVegetation (C4Game.cpp:2962-3026).
    pub(crate) fn place_vegetation(
        &mut self,
        id: &str,
        x: i32,
        y: i32,
        wdt: i32,
        hgt: i32,
        growth: i32,
    ) -> Option<ObjectId> {
        // Get definition (C4Id2Def) — an unknown id draws nothing.
        let (shape_wdt, shape_hgt, def_placement, def_growth) = self.placement_def(id)?;
        let gback_hgt = self.gback_hgt();

        // No growth specified: full or random growth (C4Game.cpp:2971-2975).
        let mut growth = growth;
        if growth <= 0 {
            growth = FULL_CON;
            if def_growth != 0 && self.rng.random(3) == 0 {
                growth = self.rng.random(FULL_CON) + 1;
            }
        }

        match def_placement {
            // Surface soil (C4D_Place_Surface).
            0 => {
                for _ in 0..20 {
                    // Random hit within target area.
                    let tx = x + self.rng.random(wdt);
                    let mut ty = y + self.rng.random(hgt);
                    // Above IFT.
                    while ty > 0 && self.gback_ift(tx, ty) {
                        ty -= 1;
                    }
                    // Above semi solid.
                    if !self.above_semi_solid(tx, &mut ty) || !(50..=gback_hgt - 50).contains(&ty) {
                        continue;
                    }
                    // Free above.
                    if self.gback_semi_solid(tx, ty - shape_hgt)
                        || self.gback_semi_solid(tx, ty - shape_hgt / 2)
                    {
                        continue;
                    }
                    // Free upleft and upright.
                    if self.gback_semi_solid(tx - shape_wdt / 2, ty - shape_hgt * 2 / 3)
                        || self.gback_semi_solid(tx + shape_wdt / 2, ty - shape_hgt * 2 / 3)
                    {
                        continue;
                    }
                    // Soil check: two pix into ground.
                    let ty_probe = ty + 3;
                    let material = self.gback_mat(tx, ty_probe);
                    if let Some(material) = material {
                        if self.material_is_soil(material) {
                            if def_growth == 0 {
                                growth = FULL_CON;
                            }
                            return self.init_create_object_construction(
                                id,
                                tx,
                                ty_probe + 5,
                                growth,
                            );
                        }
                    }
                }
                None
            }
            // Underwater (C4D_Place_Liquid).
            1 => {
                let mut tx = x + self.rng.random(wdt);
                let mut ty = y + self.rng.random(hgt);
                if !self.find_surface_liquid(&mut tx, &mut ty, shape_wdt, shape_hgt)
                    && !self.find_liquid(&mut tx, &mut ty, shape_wdt, shape_hgt)
                {
                    return None;
                }
                // Liquid bottom.
                if !self.semi_above_solid(tx, &mut ty) {
                    return None;
                }
                self.init_create_object_construction(id, tx, ty + 3, growth)
            }
            // Undefined placement type.
            _ => None,
        }
    }

    /// C4Game::PlaceAnimal (C4Game.cpp:3028-3061).
    pub(crate) fn place_animal(&mut self, id: &str) -> Option<ObjectId> {
        let (shape_wdt, shape_hgt, def_placement, _) = self.placement_def(id)?;
        let gback_wdt = self.gback_wdt();
        let gback_hgt = self.gback_hgt();
        let (x, y) = match def_placement {
            // Running free (C4D_Place_Surface).
            0 => {
                let mut x = self.rng.random(gback_wdt);
                let mut y = self.rng.random(gback_hgt);
                if !self.find_solid_ground(&mut x, &mut y, shape_wdt) {
                    return None;
                }
                (x, y)
            }
            // In liquid (C4D_Place_Liquid).
            1 => {
                let mut x = self.rng.random(gback_wdt);
                let mut y = self.rng.random(gback_hgt);
                if !self.find_surface_liquid(&mut x, &mut y, shape_wdt, shape_hgt)
                    && !self.find_liquid(&mut x, &mut y, shape_wdt, shape_hgt)
                {
                    return None;
                }
                (x, y + shape_hgt / 2)
            }
            // Floating in air (C4D_Place_Air).
            2 => {
                let x = self.rng.random(gback_wdt);
                let mut y = 0;
                while y < gback_hgt && !self.gback_semi_solid(x, y) {
                    y += 1;
                }
                if y <= 0 {
                    return None;
                }
                (x, self.rng.random(y))
            }
            _ => return None,
        };
        self.init_create_object(id, x, y, 0)
    }

    /// ListExpandValids (C4Game.cpp:2929-2947): every VALID id repeated
    /// `count` times, capped at 100 entries.
    fn list_expand_valids(&self, list: &[(String, i32)]) -> Vec<String> {
        let mut expanded = Vec::new();
        for (id, count) in list {
            if !self
                .definitions
                .contains_key(&DefinitionId::from(id.as_str()))
            {
                continue;
            }
            for _ in 0..*count {
                if expanded.len() < MAX_VID {
                    expanded.push(id.clone());
                }
            }
        }
        expanded
    }

    /// The MEarth material (C4Game::InitMaterialTexture, C4Game.cpp:
    /// 1675-1696): `[Landscape] Material=`, texture spec split off (the
    /// post-[359] fixed behavior).
    fn earth_material(&self, material: &str) -> Option<MaterialId> {
        let name = material.split('-').next().unwrap_or(material);
        self.materials.id_of(name)
    }

    /// The InitVegetation → InitGoals block of C4Game::InitGame
    /// (C4Game.cpp:2493-2503); the caller applies the
    /// `!NoInitialize && LandscapeLoaded` gate.
    pub(crate) fn run_legacy_init_placements(&mut self, placement: &LegacyInitPlacement) {
        let earth_material = self.earth_material(&placement.earth_material);

        // InitVegetation (C4Game.cpp:3078-3091): the VegLevel evaluate
        // draws even when the list is empty.
        let veg_amount =
            (self.gback_wdt() / 50) * placement.vegetation_level.evaluate(&mut self.rng) / 100;
        let vidlist = self.list_expand_valids(&placement.vegetation);
        if !vidlist.is_empty() {
            let gback_wdt = self.gback_wdt();
            let gback_hgt = self.gback_hgt();
            for _ in 0..veg_amount {
                let pick = self.rng.random(vidlist.len() as i32) as usize;
                let id = vidlist[pick].clone();
                self.place_vegetation(&id, 0, 0, gback_wdt, gback_hgt, -1);
            }
        }

        // InitInEarth (C4Game.cpp:3063-3076).
        let in_earth_amount = (self.gback_wdt() * self.gback_hgt() / 5000)
            * placement.in_earth_level.evaluate(&mut self.rng)
            / 100;
        let vidlist = self.list_expand_valids(&placement.in_earth);
        if !vidlist.is_empty() {
            for _ in 0..in_earth_amount {
                let pick = self.rng.random(vidlist.len() as i32) as usize;
                let id = vidlist[pick].clone();
                self.place_in_earth(&id, earth_material);
            }
        }

        // InitAnimals (C4Game.cpp:3093-3105): FreeLife then EarthNest,
        // entry order, no validity pre-filter (PlaceAnimal C4Id2Defs;
        // PlaceInEarth draws its tries regardless).
        for (id, count) in &placement.animals {
            for _ in 0..*count {
                self.place_animal(id);
            }
        }
        for (id, count) in &placement.nests {
            for _ in 0..*count {
                self.place_in_earth(id, earth_material);
            }
        }

        // InitEnvironment (C4Game.cpp:3988-3996): CreateObject(id, nullptr)
        // — C4Game.h defaults x=50, y=50, owner NO_OWNER.
        for (id, count) in &placement.environment {
            for _ in 0..*count {
                self.init_create_object(id, 50, 50, 0);
            }
        }

        // InitRules (C4Game.cpp:3998-4008): at least one per listed rule.
        for (id, count) in &placement.rules {
            for _ in 0..(*count).max(1) {
                self.init_create_object(id, 50, 50, 0);
            }
        }
        // InitRules immediately calls UpdateRules. It derives these flags
        // from the actual surviving CNMT/ENRG/FGRV objects, including
        // explicit Rules= entries when the obsolete realism fields are disabled
        // (C4Game.cpp:4016-4025,4038-4044).
        self.refresh_flag_removeable_rule();
        if self.objects.iter().any(|object| {
            object.definition_id == "CNMT" && !object.destroyed && object.state.status.is_active()
        }) {
            self.set_construction_needs_material(true);
        }
        if self.objects.iter().any(|object| {
            object.definition_id == "ENRG" && !object.destroyed && object.state.status.is_active()
        }) {
            self.set_structures_need_energy(true);
        }

        // InitGoals (C4Game.cpp:4010-4018).
        for (id, count) in &placement.goals {
            for _ in 0..*count {
                self.init_create_object(id, 50, 50, 0);
            }
        }
    }

    /// Fresh-game tail after Weather.Init: if any live C4D_Goal exists but
    /// no generic GOAL controller does, C++ creates one to drive CheckTime,
    /// Wait4End and RoundOver (C4Game.cpp:2531-2535).
    pub(crate) fn ensure_legacy_goal_controller(&mut self) {
        let has_goal = self.objects.iter().any(|object| {
            !object.destroyed
                && object.state.status.is_active()
                && object.state.category & CATEGORY_GOAL != 0
        });
        let has_controller = self.objects.iter().any(|object| {
            !object.destroyed
                && object.state.status.is_active()
                && object.definition_id.as_str() == "GOAL"
        });
        if has_goal && !has_controller {
            self.init_create_object("GOAL", 50, 50, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Definition, DefinitionRect, Value, CATEGORY_OBJECT};

    #[test]
    fn init_create_object_runs_cpp_new_object_lifecycle() {
        // C4Game::NewObject inserts the Con=0/raw-position object before
        // Construction(creator), then DoCon(FullCon, true) bottom-adjusts it
        // and calls Completion followed by Initialize (C4Game.cpp:1102-1146;
        // C4Object.cpp:1428-1515). Init placements pass a null creator.
        let script = r#"#strict
local construction_creator, construction_con, construction_y, construction_lookup;
local completion_y, callback_order;

protected func Construction(object creator)
{
    construction_creator = creator;
    construction_con = GetCon();
    construction_y = GetY();
    construction_lookup = Object(ObjectNumber());
    callback_order = 1;
}

protected func Completion()
{
    completion_y = GetY();
    callback_order = callback_order * 10 + 2;
}

protected func Initialize()
{
    callback_order = callback_order * 10 + 3;
}
"#;
        let mut definition =
            Definition::from_script("LIFE", "Lifecycle", script).expect("script compiles");
        definition.set_category(CATEGORY_OBJECT);
        definition.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
        definition.set_stretch_growth(true);

        let mut engine = Engine::with_seed(1);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let object_id = engine
            .init_create_object("LIFE", 20, 50, 0)
            .expect("object survives creation");
        let object = engine.object_snapshot(object_id).expect("object is live");

        assert_eq!(object.local_vars["construction_creator"], Value::Nil);
        assert_eq!(object.local_vars["construction_con"], Value::Int(0));
        assert_eq!(object.local_vars["construction_y"], Value::Int(50));
        assert_eq!(
            object.local_vars["construction_lookup"],
            Value::Object(object_id.as_u64())
        );
        assert_eq!(object.construction, FULL_CON);
        assert_eq!(object.position, Vector2::new(20, 47));
        assert_eq!(object.local_vars["completion_y"], Value::Int(47));
        assert_eq!(object.local_vars["callback_order"], Value::Int(123));
    }

    #[test]
    fn explicit_rule_objects_enable_cpp_engine_rule_flags() {
        let mut engine = Engine::with_seed(0);
        for id in ["CNMT", "ENRG"] {
            engine
                .register_definition(Definition::from_script(id, id, "").unwrap())
                .unwrap();
        }
        engine.set_construction_needs_material(false);
        engine.set_structures_need_energy(false);

        engine.run_legacy_init_placements(&LegacyInitPlacement {
            rules: vec![("CNMT".to_string(), 1), ("ENRG".to_string(), 1)],
            ..LegacyInitPlacement::default()
        });

        assert!(engine.construction_needs_material);
        assert!(engine.structures_need_energy());
    }

    #[test]
    fn init_create_object_construction_stops_before_completion_when_partial() {
        // CreateObjectConstruction still exposes Con=0/raw bottom to
        // Construction, but a partial initial DoCon does not cross FullCon
        // and therefore calls neither Completion nor Initialize
        // (C4Game.cpp:1191-1230; C4Object.cpp:1428-1515).
        let script = r#"#strict
local construction_con, construction_y, completion, initialized;
protected func Construction(object creator)
{
    construction_con = GetCon();
    construction_y = GetY();
}
protected func Completion() { completion = 1; }
protected func Initialize() { initialized = 1; }
"#;
        let mut definition =
            Definition::from_script("PART", "Partial", script).expect("script compiles");
        definition.set_category(CATEGORY_OBJECT);
        definition.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
        definition.set_stretch_growth(true);

        let mut engine = Engine::with_seed(2);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let object_id = engine
            .init_create_object_construction("PART", 20, 50, FULL_CON / 4)
            .expect("partial object survives creation");
        let object = engine.object_snapshot(object_id).expect("object is live");

        assert_eq!(object.local_vars["construction_con"], Value::Int(0));
        assert_eq!(object.local_vars["construction_y"], Value::Int(50));
        assert_eq!(object.construction, FULL_CON / 4);
        assert_eq!(object.position, Vector2::new(20, 49));
        assert_eq!(object.local_vars["completion"], Value::Nil);
        assert_eq!(object.local_vars["initialized"], Value::Nil);
    }

    #[test]
    fn init_creation_callbacks_are_fail_safe_and_removal_returns_none() {
        // C4Object::Call defaults to fPassErrors=false, so callback errors do
        // not abort NewObject; AssignRemoval in Construction does, returning
        // nullptr while retaining the consumed enumeration number
        // (C4Game.cpp:1117-1146; C4Object.cpp:2224-2247).
        let fail_safe_script = r#"#strict
local callback_order;
protected func Construction(object creator) { callback_order = 1; MissingConstruction(); }
protected func Completion() { callback_order = callback_order * 10 + 2; MissingCompletion(); }
protected func Initialize() { callback_order = callback_order * 10 + 3; }
"#;
        let removed_script = r#"#strict
protected func Construction(object creator) { Random(100); RemoveObject(); }
protected func Completion() { Random(100); }
protected func Initialize() { Random(100); }
"#;
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(
                Definition::from_script("SAFE", "Fail safe", fail_safe_script)
                    .expect("fail-safe script compiles"),
            )
            .expect("fail-safe definition registers");
        engine
            .register_definition(
                Definition::from_script("GONE", "Removed", removed_script)
                    .expect("removed script compiles"),
            )
            .expect("removed definition registers");

        let survivor = engine
            .init_create_object("SAFE", 10, 10, 0)
            .expect("callback errors are tolerated");
        assert_eq!(
            engine
                .object_snapshot(survivor)
                .expect("survivor is live")
                .local_vars["callback_order"],
            Value::Int(123)
        );

        let next_before = engine.capture_state().next_object_id;
        let mut expected_rng = engine.rng.clone();
        let _ = expected_rng.random(100);
        assert_eq!(engine.init_create_object("GONE", 10, 10, 0), None);
        assert_eq!(engine.rng, expected_rng, "later callbacks were suppressed");
        assert_eq!(
            engine.capture_state().next_object_id,
            next_before + 1,
            "removed creation still consumes its object number"
        );
    }
}
