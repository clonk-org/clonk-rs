    #[test]
    fn sectors_index_spawned_objects_by_point_and_shape_area() {
        let mut definition = simple_definition("Crate");
        definition.set_shape_rect(Some(DefinitionRect::new(-10, -5, 70, 10)));

        let mut engine = Engine::with_seed(31);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(20, 20)))
            .expect("spawn succeeds");

        let sectors = engine.sectors.as_ref().expect("sectors initialized");
        assert_eq!(
            sectors.object_ids(sector::SectorKey::Inside { x: 0, y: 0 }),
            &[id]
        );
        assert_eq!(
            sectors.shape_ids(sector::SectorKey::Inside { x: 0, y: 0 }),
            &[id]
        );
        assert_eq!(
            sectors.shape_ids(sector::SectorKey::Inside { x: 1, y: 0 }),
            &[id]
        );
        assert!(sectors
            .shape_ids(sector::SectorKey::Inside { x: 2, y: 0 })
            .is_empty());
        let area = sectors.area(DefinitionRect::new(0, 0, 100, 50));
        assert_eq!(sectors.shape_ids_in_area(&area), vec![id]);
        assert_eq!(sectors.shape_sum(), 2);
    }

    #[test]
    fn sectors_track_object_position_updates_across_sector_boundaries() {
        let mut engine = Engine::with_seed(32);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(simple_definition("Mover"))
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Mover").with_position(Vector2::new(10, 10)))
            .expect("spawn succeeds");
        engine
            .apply_object_update(id, ObjectUpdate::new().with_position(Vector2::new(70, 10)))
            .expect("update succeeds");

        let sectors = engine.sectors.as_ref().expect("sectors initialized");
        assert!(sectors
            .object_ids(sector::SectorKey::Inside { x: 0, y: 0 })
            .is_empty());
        assert_eq!(
            sectors.object_ids(sector::SectorKey::Inside { x: 1, y: 0 }),
            &[id]
        );
    }

    #[test]
    fn sectors_remove_deleted_objects_from_membership_lists() {
        let mut engine = Engine::with_seed(33);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(simple_definition("Rock"))
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Rock").with_position(Vector2::new(10, 10)))
            .expect("spawn succeeds");
        engine
            .apply_object_update(id, ObjectUpdate::new().with_status(ObjectStatus::Deleted))
            .expect("delete succeeds");

        let sectors = engine.sectors.as_ref().expect("sectors initialized");
        assert!(sectors
            .object_ids(sector::SectorKey::Inside { x: 0, y: 0 })
            .is_empty());
        assert!(sectors
            .shape_ids(sector::SectorKey::Inside { x: 0, y: 0 })
            .is_empty());
    }

    #[test]
    fn at_object_uses_point_sector_and_shape_test() {
        let mut definition = simple_definition("Target");
        definition.set_shape_rect(Some(DefinitionRect::new(-10, -5, 20, 10)));
        definition.set_ocf_base(ocf::GRAB);

        let mut engine = Engine::with_seed(34);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(definition)
            .expect("definition registers");

        // Spawn y is the con-0 bottom (C4Object.cpp:1462-1468): 15 - (10 - 5)
        // puts the center at (40,10).
        let id = engine
            .spawn_object(SpawnConfig::new("Target").with_position(Vector2::new(40, 15)))
            .expect("spawn succeeds");

        let hit = engine
            .at_object(Vector2::new(31, 10), ocf::GRAB, None)
            .expect("object found at point");
        assert_eq!(hit.1, id);
        assert_ne!(hit.2 & ocf::GRAB, 0);
        assert!(engine
            .at_object(Vector2::new(29, 10), ocf::GRAB, None)
            .is_none());
    }

    #[test]
    fn at_object_finds_shapes_crossing_the_probe_sector() {
        let mut definition = simple_definition("TallTarget");
        definition.set_shape_rect(Some(DefinitionRect::new(-10, -20, 20, 40)));
        definition.set_ocf_base(ocf::GRAB);

        let mut engine = Engine::with_seed(34);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(definition)
            .expect("definition registers");

        // Spawn y is the con-0 bottom: 60 - (40 - 20) puts the center at
        // y=40 (sector row 0), while the shape reaches y=59 (row 1).
        let id = engine
            .spawn_object(SpawnConfig::new("TallTarget").with_position(Vector2::new(40, 60)))
            .expect("target spawns");
        let hit = engine
            .at_object(Vector2::new(40, 55), ocf::GRAB, None)
            .expect("ObjectShapes finds a target whose center is in the adjacent sector");

        assert_eq!(hit.1, id);
        assert_ne!(hit.2 & ocf::GRAB, 0);
    }

    #[test]
    fn at_object_finds_low_construction_site_through_minimum_build_top() {
        let mut definition = simple_definition("BuildSite");
        definition.set_shape_rect(Some(DefinitionRect::new(-14, -28, 28, 56)));
        definition.set_constructable(true);

        let mut engine = Engine::with_seed(35);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("BuildSite")
                    .with_position(Vector2::new(40, 60))
                    .with_construction(1_000),
            )
            .expect("construction site spawns");
        let site = engine.object_snapshot(id).expect("construction site exists");
        let probe = Vector2::new(site.position.x, site.position.y - 15);
        assert_ne!(
            probe.y / sector::SECTOR_HEIGHT,
            site.position.y / sector::SECTOR_HEIGHT,
            "the regression probe must cross a sector boundary"
        );

        let hit = engine
            .at_object(probe, ocf::CONSTRUCT, None)
            .expect("C4Object::addtop makes the low construction site buildable");
        assert_eq!(hit.1, id);
        assert_ne!(hit.2 & ocf::CONSTRUCT, 0);
    }

    #[test]
    fn at_object_exclusive_candidate_blocks_later_matches() {
        let mut blocker = simple_definition("Blocker");
        blocker.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        blocker.set_ocf_base(ocf::EXCLUSIVE);
        let mut target = simple_definition("Target");
        target.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        target.set_ocf_base(ocf::GRAB);

        let mut engine = Engine::with_seed(35);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(blocker)
            .expect("blocker definition registers");
        engine
            .register_definition(target)
            .expect("target definition registers");

        // Spawn y is the con-0 bottom (C4Object.cpp:1462-1468): 25 - (10 - 5)
        // puts both centers at (20,20), so the probe point lies inside both.
        // C4ObjectList::Add inserts the later same-category object before
        // the existing different-ID object in the forward master list
        // (C4ObjectList.cpp:138-174). Sector ObjectShapes follows that
        // master order, so the second-spawned blocker must be visited first.
        engine
            .spawn_object(SpawnConfig::new("Target").with_position(Vector2::new(20, 25)))
            .expect("target spawns");
        engine
            .spawn_object(SpawnConfig::new("Blocker").with_position(Vector2::new(20, 25)))
            .expect("blocker spawns");

        assert!(engine
            .at_object(Vector2::new(20, 20), ocf::GRAB, None)
            .is_none());
    }

    #[test]
    fn at_object_exclude_skips_wrong_layer_exclusive_before_same_layer_match() {
        let mut blocker = simple_definition("LayerBlocker");
        blocker.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        blocker.set_ocf_base(ocf::EXCLUSIVE);
        let mut target = simple_definition("LayerTarget");
        target.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        target.set_ocf_base(ocf::GRAB);
        let mut exclude = simple_definition("LayerExclude");
        exclude.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        exclude.set_ocf_base(ocf::GRAB);

        let mut engine = Engine::with_seed(35);
        engine.set_landscape(Landscape::flat(120, 120));
        engine
            .register_definition(simple_definition("LayerKey"))
            .expect("layer definition registers");
        engine
            .register_definition(blocker)
            .expect("blocker definition registers");
        engine
            .register_definition(target)
            .expect("target definition registers");
        engine
            .register_definition(exclude)
            .expect("exclude definition registers");
        let foreign_layer = engine
            .spawn_object(SpawnConfig::new("LayerKey"))
            .expect("foreign layer spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("LayerTarget").with_position(Vector2::new(20, 25)),
            )
            .expect("target spawns");
        let blocker = engine
            .spawn_object(
                SpawnConfig::new("LayerBlocker")
                    .with_position(Vector2::new(20, 25))
                    .with_layer(foreign_layer),
            )
            .expect("foreign blocker spawns");
        let exclude = engine
            .spawn_object(
                SpawnConfig::new("LayerExclude").with_position(Vector2::new(20, 25)),
            )
            .expect("exclude object spawns");

        let relevant_order = engine
            .sectors
            .as_ref()
            .expect("sectors initialized")
            .shape_ids(SectorKey::Inside { x: 0, y: 0 })
            .iter()
            .copied()
            .filter(|id| [exclude, blocker, target].contains(id))
            .collect::<Vec<_>>();
        assert_eq!(relevant_order, vec![exclude, blocker, target]);
        assert_eq!(
            engine
                .at_object(Vector2::new(20, 20), ocf::GRAB, None)
                .map(|(_, id, _)| id),
            Some(exclude),
            "without an exclude object no identity or layer filter applies"
        );
        assert_eq!(
            engine
                .at_object(Vector2::new(20, 20), ocf::GRAB, Some(exclude))
                .map(|(_, id, _)| id),
            Some(target),
            "exclude layer filtering runs before Exclusive blocking"
        );
    }

    #[test]
    fn cross_check_collection_uses_sector_area_candidates() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(36);
        engine.set_landscape(Landscape::flat(160, 160));
        let mut crew_definition = Definition::from_script("Crew", "Crew", BASIC_OBJECT_SCRIPT)?;
        crew_definition.set_crew_member(true);
        crew_definition.set_shape_rect(Some(DefinitionRect::new(-30, -10, 80, 20)));
        crew_definition.set_collection_rect(Some(DefinitionRect::new(-30, -10, 80, 20)));
        engine.register_definition(crew_definition)?;

        let mut item_definition = Definition::from_script("Gem", "Gem", BASIC_OBJECT_SCRIPT)?;
        item_definition.set_collectible(true);
        engine.register_definition(item_definition)?;

        // Spawn y is the con-0 bottom (C4Object.cpp:1462-1468): 30 - (20 - 10)
        // keeps the crew center at (70,20) beside the shapeless Gem.
        let crew = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_alive(true)
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(70, 30)),
        )?;
        let item =
            engine.spawn_object(SpawnConfig::new("Gem").with_position(Vector2::new(115, 20)))?;

        // Collection runs on Tick3 frames only (C4GameObjects.cpp:144-148).
        for _ in 0..3 {
            engine.tick_without_snapshot()?;
        }

        let item_snapshot = engine.object_snapshot(item).expect("item snapshot");
        assert_eq!(item_snapshot.container, Some(crew));
        Ok(())
    }

    #[test]
    fn shape_bottom_vertex_contact_stops_before_solid_surface() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=50
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Crate");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -2, 2, 4)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 2)
            .with_cnat(CNAT_BOTTOM)
            .with_friction(100)]);
        definition.set_contact_density(50);

        let mut engine = Engine::with_seed(31);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 12, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 8)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(4)));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(5, 9));
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.y, itofix(9));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    #[test]
    fn shape_horizontal_contact_redirects_force_like_cpp() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Crate");
        definition.set_shape_vertices(vec![ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT)]);
        definition.set_contact_density(50);

        let mut engine = Engine::with_seed(37);
        engine.set_materials(materials);
        let mut surface = vec![20; 12];
        surface[6] = 0;
        let mut landscape =
            Landscape::new_with_material(12, surface, Some(earth)).expect("landscape constructs");
        landscape.fill_solid_material(Some(earth));
        engine.set_landscape(landscape);
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 10)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(4), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(4, 10));
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(4));
        assert_eq!(
            engine.objects[idx].fixed_velocity.x,
            itofix(4) - fixed100(50)
        );
        assert_eq!(engine.objects[idx].fixed_velocity.y, -fixed100(50));
    }

    #[test]
    fn vehicle_density_boundary_below_contact_density_allows_motion_like_cpp() {
        // Mirrors src/C4Movement.cpp:260-281 horizontal per-pixel loop:
        // `ContactCheck(ctx, y)` gates `DoMotion(ctx - x, 0)`. Contact is
        // `GBackDensity >= ContactDensity` through src/C4Movement.cpp:166-182
        // and src/C4Shape.cpp:389.
        //
        // Hand-derived golden: src/C4Landscape.h:144-150 returns MCVehic for a
        // closed left border, and src/C4Material.h:200 defines C4M_Vehicle = 100.
        // With ContactDensity = 101, 100 >= 101 is false, so C++ takes DoMotion
        // at src/C4Movement.cpp:281 and moves x from 0 to -1 without redirecting.
        // ExecMovement then immediately runs its origin bounds check and calls
        // AssignDeath(true)+AssignRemoval (src/C4Movement.cpp:598-617).
        let mut definition = simple_definition("Probe");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_LEFT)]);
        definition.set_contact_density(101);

        let mut engine = Engine::with_seed(53);
        engine.set_landscape(Landscape::flat(8, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Probe")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 5)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(-itofix(1), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        assert!(
            snapshot.object(id).is_none(),
            "the unbounded object is removed in the same tick after crossing x < 0"
        );
    }

    #[test]
    fn border_bound_sides_clamps_fixed_target_and_velocity() {
        let mut definition = simple_definition("Bounded");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        definition.set_border_bound(C4D_BORDER_SIDES);

        let mut engine = Engine::with_seed(41);
        engine.set_landscape(Landscape::flat(10, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Bounded")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(8, 5)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(5), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position.x, 9);
        let idx = engine.find_object_index(id).expect("object exists");
        // TargetBounds clamps the INT step target only (C4Movement.cpp:
        // 128-150) — fix_x keeps the momentum-advanced value.
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(13));
        assert_eq!(engine.objects[idx].fixed_velocity.x, C4Fixed::ZERO);
    }

    #[test]
    fn script_set_position_bounds_check_never_moves_vertically_like_cpp() {
        // FnSetPosition's fCheckBounds runs C4Object::BoundsCheck
        // (C4Script.cpp:470-476 -> C4Object.h:392-395), which is only
        // SideBounds + VerticalBounds (C4Movement.cpp:185-229): pLayer and
        // map-border clamping gated on Def->BorderBound. Landscape solidity
        // never enters it, so a Clonk (BorderBound=1, sides only) keeps its y
        // when EkeReloaded's bullet shoves it a pixel sideways:
        //   SetPosition(GetX(victim) + direction, GetY(victim), victim, 1)
        // (content/EkeReloaded.c4d/Ammo.c4d/Cartridges.c4d/Bullet.c4d
        // func HitCreature).
        let script = r#"#strict 3
func Shove(victim) {
    var before = GetY(victim);
    SetPosition(GetX(victim) + 1, GetY(victim), victim, 1);
    return GetY(victim) - before;
}
"#;
        let mut definition =
            Definition::from_script("BPOS", "Bounds probe", script).expect("probe compiles");
        // The shipped Clonk shape and vertices
        // (content/Objects.c4d/Crew.c4d/Clonk.c4d/DefCore.txt: Width=16
        // Height=20 Offset=-8,-10 VertexX/Y/CNAT).
        definition.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        definition.set_shape_vertices(vec![
            ObjectVertex::new(0, 2),
            ObjectVertex::new(0, -7).with_cnat(CNAT_TOP),
            ObjectVertex::new(0, 9).with_cnat(CNAT_BOTTOM),
            ObjectVertex::new(-2, -3).with_cnat(CNAT_LEFT),
            ObjectVertex::new(2, -3).with_cnat(CNAT_RIGHT),
            ObjectVertex::new(-4, 3).with_cnat(CNAT_LEFT),
            ObjectVertex::new(4, 3).with_cnat(CNAT_RIGHT),
        ]);
        definition.set_border_bound(C4D_BORDER_SIDES);

        let mut engine = Engine::with_seed(58);
        // Ground at y=30, with a full-height wall in column 17 — one pixel to
        // the right of where the victim stands, so a surface-height clamp
        // would yank it up to the top of the map.
        let mut surface = vec![30; 40];
        surface[17] = 0;
        engine.set_landscape(
            Landscape::new_with_material(40, surface, None).expect("landscape constructs"),
        );
        engine
            .register_definition(definition)
            .expect("definition registers");

        let spawn = |engine: &mut Engine, at: Vector2| -> ObjectId {
            let id = engine
                .spawn_object(
                    SpawnConfig::new("BPOS")
                        .with_category(CATEGORY_OBJECT)
                        .with_position(at)
                        .with_construction(FULL_CON),
                )
                .expect("object spawns");
            // Place exactly, so the spawn's own bottom-edge convention stays
            // out of the assertions below.
            engine
                .apply_object_update(id, ObjectUpdate::new().with_position(at))
                .expect("placement applies");
            id
        };
        let victim = spawn(&mut engine, Vector2::new(16, 20));
        let shooter = spawn(&mut engine, Vector2::new(30, 20));

        let idx = engine.find_object_index(shooter).expect("shooter exists");
        assert_eq!(
            engine
                .call_object_function(idx, "Shove", vec![Value::Object(victim.as_u64())])
                .expect("the shove runs"),
            Value::Int(0),
            "a sides-only BorderBound has no vertical term in BoundsCheck"
        );
        let object = engine.object_snapshot(victim).expect("victim survives");
        assert_eq!(object.position, Vector2::new(17, 20));
    }

    #[test]
    fn script_set_position_bounds_check_clamps_to_the_map_border_like_cpp() {
        // SideBounds' landscape arm is
        // TargetBounds(x, 0 - Shape.x, GBackWdt + Shape.x, CNAT_Left,
        // CNAT_Right) (C4Movement.cpp:202-204), and TargetBounds zeroes xdir
        // and runs Contact() at each limit it hits (C4Movement.cpp:128-163).
        // Shape.x is negative, so a 16-wide Clonk stops with its centre eight
        // pixels inside either edge — not at the vertex extents.
        let script = r#"#strict 3
local left_calls, right_calls;
func Yank(victim, x) {
    SetPosition(x, GetY(victim), victim, 1);
    return GetX(victim);
}
protected func ContactLeft() { left_calls = 1; return 0; }
protected func ContactRight() { right_calls = 1; return 0; }
"#;
        let mut definition =
            Definition::from_script("BPSX", "Border probe", script).expect("probe compiles");
        definition.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        definition.set_border_bound(C4D_BORDER_SIDES);
        definition.set_contact_function_calls(true);
        definition.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(59);
        engine.set_landscape(Landscape::flat(40, 60));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let victim = engine
            .spawn_object(
                SpawnConfig::new("BPSX")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 20))
                    .with_construction(FULL_CON),
            )
            .expect("victim spawns");
        let shooter = engine
            .spawn_object(
                SpawnConfig::new("BPSX")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 20))
                    .with_construction(FULL_CON),
            )
            .expect("shooter spawns");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[victim_idx].set_fixed_velocity(FixedVec2::new(itofix(3), itofix(2)));

        let shooter_idx = engine.find_object_index(shooter).expect("shooter exists");
        assert_eq!(
            engine
                .call_object_function(
                    shooter_idx,
                    "Yank",
                    vec![Value::Object(victim.as_u64()), Value::Int(-50)],
                )
                .expect("the yank runs"),
            Value::Int(8),
            "low limit is 0 - Shape.x"
        );
        let victim_idx = engine.find_object_index(victim).expect("victim survives");
        // Only the contacted axis stops (C4Movement.cpp:135-142).
        assert_eq!(engine.objects[victim_idx].fixed_velocity.x, C4Fixed::ZERO);
        assert_eq!(engine.objects[victim_idx].fixed_velocity.y, itofix(2));

        assert_eq!(
            engine
                .call_object_function(
                    shooter_idx,
                    "Yank",
                    vec![Value::Object(victim.as_u64()), Value::Int(999)],
                )
                .expect("the yank runs"),
            Value::Int(32),
            "high limit is GBackWdt + Shape.x"
        );

        let object = engine.object_snapshot(victim).expect("victim survives");
        assert_eq!(object.local_vars.get("left_calls"), Some(&Value::Int(1)));
        assert_eq!(object.local_vars.get("right_calls"), Some(&Value::Int(1)));
    }

    #[test]
    fn layer_border_bound_clamps_horizontal_target_like_cpp() {
        use std::sync::{Arc, Mutex};

        // Mirrors src/C4Movement.cpp:185-196. For a non-static object, C++ applies
        // layer-side TargetBounds when `pLayer->Def->BorderBound & C4D_Border_Layer`:
        // low  = layer.x + layer.Shape.x - object.Shape.x
        // high = layer.x + layer.Shape.x + layer.Shape.Wdt + object.Shape.x
        //
        // Hand-derived golden for this setup: layer.x=20, layer.Shape.x=-1,
        // layer.Shape.Wdt=10, object.Shape.x=0, so high=29. `fix_x += xdir`
        // targets x=33, SideBounds clamps ctcox to 29 and zeroes xdir via
        // TargetBounds at src/C4Movement.cpp:147-155, then the per-pixel loop
        // moves from x=28 to x=29.
        let mut layer_definition = simple_definition("Layer");
        layer_definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 10, 10)));
        layer_definition.set_border_bound(C4D_BORDER_LAYER);

        let call_log = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                if name == "ContactLeft" || name == "ContactRight" {
                    call_log.lock().unwrap().push(name.to_string());
                }
            });
        }
        let mut mover_definition = Definition::from_script(
            "Mover",
            "Mover",
            r#"
            global func ContactLeft() { return 0; }
            global func ContactRight() { return 0; }
            "#,
        )
        .expect("mover script compiles");
        mover_definition.set_debugger_hooks(hooks);
        mover_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        mover_definition.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(57);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(layer_definition)
            .expect("layer definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let layer_id = engine
            .spawn_object(
                SpawnConfig::new("Layer")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 10)),
            )
            .expect("layer spawns");
        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(28, 10))
                    .with_layer(layer_id),
            )
            .expect("mover spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(5), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position.x, 29);

        let idx = engine.find_object_index(mover_id).expect("object exists");
        // The fixed coordinate keeps the momentum-advanced value
        // (TargetBounds clamps the INT target only).
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(33));
        assert_eq!(engine.objects[idx].fixed_velocity.x, C4Fixed::ZERO);
        assert_eq!(
            *call_log.lock().unwrap(),
            vec!["ContactRight".to_string()]
        );
    }

    #[test]
    fn layer_border_bound_uses_half_construction_live_shape() {
        // Mirrors src/C4Movement.cpp:185-196 with a stretch-growth layer at
        // Con=50%. C++ UpdateShape scales (-2,-2,12,12) to (-1,-1,6,6), so
        // the non-static right bound is 20-1+6+0=25. Using the definition
        // rect instead would produce 30 and let the mover travel too far.
        let mut layer_definition = simple_definition("HalfLayer");
        layer_definition.set_shape_rect(Some(DefinitionRect::new(-2, -2, 12, 12)));
        layer_definition.set_stretch_growth(true);
        layer_definition.set_border_bound(C4D_BORDER_LAYER);

        let mut mover_definition = simple_definition("HalfLayerMover");
        mover_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);

        let mut engine = Engine::with_seed(129);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(layer_definition)
            .expect("layer definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let layer_id = engine
            .spawn_object(
                SpawnConfig::new("HalfLayer")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 10))
                    .with_construction(FULL_CON / 2),
            )
            .expect("layer spawns");
        assert_eq!(
            engine.object_current_shape_rect(layer_id),
            Some(DefinitionRect::new(-1, -1, 6, 6))
        );

        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("HalfLayerMover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(24, 10))
                    .with_layer(layer_id),
            )
            .expect("mover spawns");
        let idx = engine.find_object_index(mover_id).expect("mover exists");
        engine.objects[idx]
            .set_fixed_velocity(FixedVec2::new(itofix(5), C4Fixed::ZERO));
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(
            snapshot.object(mover_id).expect("mover present").position.x,
            25
        );

        let idx = engine.find_object_index(mover_id).expect("mover exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(29));
        assert_eq!(engine.objects[idx].fixed_velocity.x, C4Fixed::ZERO);
    }

    #[test]
    fn inverted_layer_bounds_clamp_low_then_high_like_cpp() {
        use std::sync::{Arc, Mutex};

        // A layer narrower than the mover inverts the non-static limits:
        // low=20-1-(-3)=22, high=20-1+2+(-3)=18. C++ TargetBounds uses
        // independent if arms, so target 21 first calls ContactLeft at 22,
        // then ContactRight at 18 and finally moves the object to x=18.
        let mut layer_definition = simple_definition("NarrowLayer");
        layer_definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        layer_definition.set_border_bound(C4D_BORDER_LAYER);

        let call_log = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                if name == "ContactLeft" || name == "ContactRight" {
                    call_log.lock().unwrap().push(name.to_string());
                }
            });
        }
        let mut mover_definition = Definition::from_script(
            "WideMover",
            "WideMover",
            r#"
            global func ContactLeft() { SetXDir(10); return 0; }
            global func ContactRight() { return 0; }
            "#,
        )
        .expect("mover script compiles");
        mover_definition.set_debugger_hooks(hooks);
        mover_definition.set_shape_rect(Some(DefinitionRect::new(-3, -1, 6, 2)));
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        mover_definition.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(61);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(layer_definition)
            .expect("layer definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let layer_id = engine
            .spawn_object(
                SpawnConfig::new("NarrowLayer")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 10)),
            )
            .expect("layer spawns");
        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("WideMover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 10))
                    .with_layer(layer_id),
            )
            .expect("mover spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position.x, 18);
        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(21));
        // ContactLeft's SetXDir is erased by the second arm before
        // ContactRight, exactly like C++ TargetBounds.
        assert_eq!(engine.objects[idx].fixed_velocity.x, C4Fixed::ZERO);
        assert_eq!(
            *call_log.lock().unwrap(),
            vec!["ContactLeft".to_string(), "ContactRight".to_string()]
        );
    }

    #[test]
    fn solid_mask_vehicle_density_blocks_per_pixel_contact_like_cpp() {
        // Mirrors src/C4Movement.cpp:260-282: the horizontal per-pixel loop
        // aborts before `DoMotion` when `ContactCheck(ctx, y)` reports contact.
        // `C4SolidMask::Put` writes solid-mask pixels as MCVehic at
        // src/C4SolidMask.cpp:66-104, and C4Material.h:200 defines vehicle
        // density as 100.
        //
        // Hand-derived golden: blocker.x=5, blocker.Shape.x=0, SolidMask.tx=0,
        // so its one-pixel mask is put at world (5,5). The mover tests candidate
        // (5,5), and 100 >= ContactDensity 50 is contact, so C++ keeps x=4,
        // rewinds fix_x to itofix(4), and RedirectForce moves FIXED100(50) from
        // xdir to ydir at C4Movement.cpp:277.
        let mut blocker_definition =
            Definition::from_script("Blocker", "Blocker", "").expect("script compiles");
        blocker_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        blocker_definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut mover_definition = simple_definition("Mover");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);

        let mut engine = Engine::with_seed(59);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(blocker_definition)
            .expect("blocker definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 5)),
            )
            .expect("mover spawns");
        // Spawn y is the con-0 bottom (C4Object.cpp:1462-1468): 6 - (1 + 0)
        // keeps the blocker center — and its solid mask — at (5,5).
        engine
            .spawn_object(
                SpawnConfig::new("Blocker")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 6)),
            )
            .expect("blocker spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position, Vector2::new(5, 5));

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(5));
        assert_eq!(engine.objects[idx].fixed_velocity.x, itofix(1));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    #[test]
    fn solid_mask_transparent_bitmap_pixel_allows_motion_like_cpp() -> Result<(), EngineError> {
        // Mirrors src/C4SolidMask.cpp:401-411: the object solid-mask bitmap is
        // copied from definition graphics transparency, and src/C4SolidMask.cpp:
        // 80-104 only writes MCVehic for non-transparent mask pixels. The
        // movement loop at src/C4Movement.cpp:260-282 therefore takes `DoMotion`
        // when `ContactCheck(ctx, y)` probes a transparent source pixel.
        //
        // Hand-derived golden: Blocker's SolidMask=0,0,2,1,0,0 at object (5,5)
        // covers world x=5..6, but graphics pixel 0 is transparent and pixel 1
        // is opaque. The mover's one-step candidate vertex probes (5,5), so
        // C++ sees background density 0 < ContactDensity 50 and moves to x=5
        // without redirecting xdir into ydir.
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Blocker.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=BLCK\nName=Blocker\nCategory=C4D_Object\nWidth=2\nHeight=1\nOffset=0,0\nSolidMask=0,0,2,1,0,0\n",
        )
        .expect("write defcore");
        let mut image = image::RgbaImage::new(2, 1);
        image.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
        image
            .save(def_dir.join("Graphics.png"))
            .expect("write graphics");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let blocker_definition = Definition::from_resource(&resource)?;

        let mut mover_definition = simple_definition("Mover");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);

        let mut engine = Engine::with_seed(69);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine.register_definition(blocker_definition)?;
        engine.register_definition(mover_definition)?;

        let mover_id = engine.spawn_object(
            SpawnConfig::new("Mover")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(4, 5)),
        )?;
        // Spawn y is the con-0 bottom (C4Object.cpp:1462-1468): 6 - (1 + 0)
        // keeps the blocker center — and its mask origin — at (5,5).
        engine.spawn_object(
            SpawnConfig::new("BLCK")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(5, 6)),
        )?;
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick()?;
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position, Vector2::new(5, 5));

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(5));
        assert_eq!(engine.objects[idx].fixed_velocity.x, itofix(1));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        Ok(())
    }

    // C4SolidMask is built from def-graphics transparency
    // (C4SolidMask.cpp:411): IsPixTransparent is `(dwPix >> 24) >= 128` on
    // the INVERTED internal alpha (C4Surface.cpp:718-724;
    // png_set_invert_alpha, StdPNGLibpng.cpp:139-140) — solid <=> PNG
    // alpha >= 128. Anti-aliased mask edges (alpha 1..127) stay passable:
    // the GoldRush _FWS force-field posts carry 126-alpha edge columns,
    // and baking those walled the bison one pixel early (f48 wall: rust
    // 4230.0 vs cpp 4231.0 — C++ contacts the 204-alpha core column).
    #[test]
    fn solid_mask_semitransparent_pixels_stay_passable_like_cpp() -> Result<(), EngineError> {
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Post.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=POST\nName=Post\nCategory=C4D_Object\nWidth=3\nHeight=1\nOffset=0,0\nSolidMask=0,0,3,1,0,0\n",
        )
        .expect("write defcore");
        let mut image = image::RgbaImage::new(3, 1);
        image.put_pixel(0, 0, image::Rgba([255, 255, 255, 126])); // anti-aliased edge
        image.put_pixel(1, 0, image::Rgba([255, 255, 255, 128])); // threshold: lowest solid
        image.put_pixel(2, 0, image::Rgba([255, 255, 255, 204])); // body
        image
            .save(def_dir.join("Graphics.png"))
            .expect("write graphics");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let post_definition = Definition::from_resource(&resource)?;

        let mut mover_definition = simple_definition("Mover");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);

        let mut engine = Engine::with_seed(69);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine.register_definition(post_definition)?;
        engine.register_definition(mover_definition)?;

        // The post's mask row tracks its FIXED position (oy = obj->y +
        // Shape.y + ty, C4SolidMask.cpp:67): spawned at (5,6) the 3x1 mask
        // covers x=5..7 at y=6 (alpha 126 / 128 / 204); the mover probes
        // that row.
        let mover_id = engine.spawn_object(
            SpawnConfig::new("Mover")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(4, 6)),
        )?;
        engine.spawn_object(
            SpawnConfig::new("POST")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(5, 6)),
        )?;
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));
        engine.objects[idx].state.mobile = true;

        // Tick 1: the step onto the 126-alpha column is FREE — C++ sees a
        // transparent mask pixel there (C4Surface.cpp:723) and DoMotions.
        engine.tick_without_snapshot()?;
        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(
            engine.objects[idx].state.position,
            Vector2::new(5, 6),
            "alpha 126 < 128 must not bake solid — the mover walks onto it"
        );
        assert_eq!(engine.objects[idx].fixed_velocity.x, itofix(1));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);

        // Tick 2: the step onto the 128-alpha column contacts — 128 is the
        // lowest solid PNG alpha (255-128=127 < 128 is not transparent).
        engine.tick_without_snapshot()?;
        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(
            engine.objects[idx].state.position,
            Vector2::new(5, 6),
            "alpha 128 bakes solid — the horizontal move aborts"
        );
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(5));
        Ok(())
    }

    #[test]
    fn contact_callback_script_error_tolerated_like_cpp_fail_safe_exec() {
        // C4Object::Contact runs the Contact* callbacks via C4Object::Call
        // (C4Movement.cpp:112-119) with fPassErrors=false: a script error
        // logs and the frame continues (C4AulExec fail-safe) — it must not
        // abort the tick (the GoldRush wild horse's ContactRight hit this).
        let script = r#"
            global func ContactRight() { return NoSuchFunctionAnywhere(); }
        "#;

        let mut blocker_definition =
            Definition::from_script("Blocker", "Blocker", "").expect("script compiles");
        blocker_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        blocker_definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut mover_definition =
            Definition::from_script("Mover", "Mover", script).expect("script compiles");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);
        mover_definition.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(61);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(blocker_definition)
            .expect("blocker definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 5)),
            )
            .expect("mover spawns");
        engine
            .spawn_object(
                SpawnConfig::new("Blocker")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 6)),
            )
            .expect("blocker spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));
        engine.objects[idx].state.mobile = true;

        engine
            .tick_without_snapshot()
            .expect("a Contact* script error must not abort the tick");
    }

    #[test]
    fn definition_from_resource_carries_rotated_solidmasks_like_cpp() -> Result<(), EngineError> {
        // The C4Def compile threads RotatedSolidmasks (src/C4Def.cpp:414)
        // through to the UpdateSolidMask gate (src/C4Object.cpp:5655).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Elevator.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=ELEV\nName=Elevator\nCategory=C4D_Object\nWidth=4\nHeight=4\nOffset=0,0\nSolidMask=0,0,4,4,0,0\nRotatedSolidmasks=1\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;
        assert!(definition.rotated_solid_masks());
        Ok(())
    }

    #[test]
    fn definition_load_paths_receive_the_defaulted_shape_picture() -> Result<(), EngineError> {
        // C4DefCore::Load resolves an absent Picture before either modern or
        // legacy scenario code copies the core onto C4Def (C4Def.cpp:221-223).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("PictureDefault.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=PICT\nWidth=42\nHeight=48\nOffset=-21,-24\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let expected = DefinitionPicture {
            x: 0,
            y: 0,
            width: 42,
            height: 48,
        };

        let definition = Definition::from_resource(&resource)?;
        assert_eq!(definition.picture(), Some(expected));

        let mut legacy_definition = Definition::from_script("PICT", "Picture Default", "")?;
        Engine::apply_resource_core(&mut legacy_definition, &resource.core);
        assert_eq!(legacy_definition.picture(), Some(expected));
        Ok(())
    }

    #[test]
    fn definition_from_resource_carries_top_face_like_cpp() -> Result<(), EngineError> {
        // C4DefCore::CompileFunc stores TopFace on C4Def (src/C4Def.cpp:306),
        // and C4Object::UpdateFace consumes that same target rect
        // (src/C4Object.cpp:368-377).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("ElevatorCar.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=ELEC\nName=Elevator Car\nTopFace=0,1,24,26,-12,-13\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;
        assert_eq!(
            definition.top_face(),
            Some(DefinitionTargetRect::new(0, 1, 24, 26, -12, -13))
        );
        Ok(())
    }

    #[test]
    fn definition_from_resource_carries_signed_move_to_range_like_cpp() -> Result<(), EngineError> {
        // C4Def stores MoveToRange as a signed int (src/C4Def.cpp:400);
        // C4Command applies only positive values later (:213-215).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Mover.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=MOVE\nName=Mover\nMoveToRange=-7\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;

        assert_eq!(definition.move_to_range(), -7);
        Ok(())
    }

    #[test]
    fn definition_from_resource_carries_defcore_version_like_cpp() -> Result<(), EngineError> {
        // C4Def owns the five-component rC4XVer parsed by
        // C4DefCore::CompileFunc (src/C4Def.h:190; src/C4Def.cpp:254).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Versioned.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=VERS\nName=Versioned\nVersion=4,9,1,3,27\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;

        assert_eq!(definition.version(), [4, 9, 1, 3, 27]);

        let mut legacy_definition = Definition::from_script("VERS", "Versioned", "")?;
        Engine::apply_resource_core(&mut legacy_definition, &resource.core);
        assert_eq!(legacy_definition.version(), [4, 9, 1, 3, 27]);
        Ok(())
    }

    #[test]
    fn apply_resource_core_preserves_complete_control_and_ocf_metadata_like_cpp(
    ) -> Result<(), EngineError> {
        // C4DefCore::CompileFunc stores these fields directly on C4Def
        // (src/C4Def.cpp:298-460). The legacy scenario loader compiles the
        // script first and then applies that same core, so it must retain the
        // complete Definition::from_resource metadata set.
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("CompleteCore.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=CORE\nName=Complete Core\nCategory=C4D_Living\n\
VehicleControl=3\nBase=1\nNoComponentMass=1\nComponents=WOOD=2;METL=1\n\
Exclusive=1\nEdible=1\nPrey=1\nAttractLightning=1\nNoFight=1\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let mut definition = Definition::from_script("CORE", "Complete Core", "")?;

        Engine::apply_resource_core(&mut definition, &resource.core);

        assert_eq!(definition.vehicle_control(), 3);
        assert!(definition.can_be_base());
        assert!(definition.no_component_mass());
        assert_eq!(
            definition.components(),
            &[
                DefinitionComponent {
                    id: DefinitionId::from("WOOD"),
                    count: 2,
                },
                DefinitionComponent {
                    id: DefinitionId::from("METL"),
                    count: 1,
                },
            ]
        );
        assert!(definition.is_exclusive());

        let mut engine = Engine::with_seed(1);
        engine.register_definition(definition)?;
        let object = engine.spawn_object(
            SpawnConfig::new("CORE")
                .with_category(CATEGORY_LIVING)
                .with_alive(true),
        )?;
        let ocf = engine.object_snapshot(object).expect("object exists").ocf;
        assert_ne!(ocf & ocf::EXCLUSIVE, 0);
        assert_ne!(ocf & ocf::EDIBLE, 0);
        assert_ne!(ocf & ocf::PREY, 0);
        assert_ne!(ocf & ocf::ATTRACT_LIGHTNING, 0);
        assert_eq!(ocf & ocf::FIGHT_READY, 0);
        Ok(())
    }

    #[test]
    fn negative_defcore_values_preserve_native_runtime_semantics() -> Result<(), EngineError> {
        // C4DefCore::CompileFunc stores these fields as signed int32 values;
        // C4DefCore::Load clamps only Mass after compilation (C4Def.cpp).
        // The distinct negatives make both definition-loading paths prove
        // that none of the other fields were collapsed to zero or unsigned.
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Signed.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=SIGN\nName=Signed\nCategory=C4D_Object\n\
Width=2\nHeight=2\nOffset=-1,-1\nCollection=-1,-1,2,2\nConstruction=1\n\
Value=-11\nMass=-99\nCollectionLimit=-12\nContactIncinerate=-13\n\
Grab=-14\nRotate=-15\nBorderBound=-1\nUprightAttach=-255\n\
Basement=-18\nConSizeOff=-20\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");

        assert_eq!(resource.core.value, -11);
        assert_eq!(resource.core.mass, 0, "Mass alone is clamped after load");
        assert_eq!(resource.core.collection_limit, -12);
        assert_eq!(resource.core.contact_incinerate, -13);
        assert_eq!(resource.core.grab, -14);
        assert_eq!(resource.core.rotateable, -15);
        assert_eq!(resource.core.border_bound, -1);
        assert_eq!(resource.core.upright_attach, -255);
        assert_eq!(resource.core.basement, -18);
        assert_eq!(resource.core.con_size_off, -20);

        let direct_definition = Definition::from_resource(&resource)?;
        let mut legacy_definition = Definition::from_script(
            "SIGN",
            "Signed",
            "func Incineration(int caused_by) { return(1); }",
        )?;
        Engine::apply_resource_core(&mut legacy_definition, &resource.core);
        let assert_signed_fields = |definition: &Definition| {
            assert_eq!(definition.value(), -11);
            assert_eq!(definition.mass(), 0);
            assert_eq!(definition.collection_limit(), -12);
            assert_eq!(definition.contact_incinerate(), -13);
            assert_eq!(definition.grab(), -14);
            assert_eq!(definition.rotateable(), -15);
            assert_eq!(definition.border_bound(), -1);
            assert_eq!(definition.upright_attach(), -255);
            assert_eq!(definition.basement(), -18);
            assert_eq!(definition.construction_offset(), -20);
        };
        assert_signed_fields(&direct_definition);
        assert_signed_fields(&legacy_definition);

        // UprightAttach remains the raw int32 in Action.t_attach; only the
        // later C4Shape::Attach call narrows that mask to its uint8_t input.
        let mut upright_engine = Engine::with_seed(155);
        upright_engine.set_physics(PhysicsSettings::new(0, 200, -200));
        let mut upright_definition = direct_definition.clone();
        upright_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_LEFT)]);
        upright_definition.configure_actions(
            Some("Float".to_string()),
            HashMap::from([(
                "Float".to_string(),
                ActionSpec::default().with_procedure("FLOAT"),
            )]),
        );
        upright_engine.register_definition(upright_definition)?;
        let mut wall_pixels = vec![0_u8; 25];
        for y in 0..5 {
            wall_pixels[y * 5 + 1] = 1;
        }
        let wall_grid = landscape::PixelGrid::new(
            5,
            5,
            wall_pixels,
            vec![0, 100],
            vec![None, Some("Wall".to_string())],
            vec![None; 2],
        );
        let mut wall = Landscape::new(5, vec![5; 5]).expect("wall landscape builds");
        wall.set_pixel_grid(wall_grid);
        upright_engine.set_landscape(wall);
        let upright_id = upright_engine.spawn_object(
            SpawnConfig::new("SIGN")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(2, 2))
                .with_construction(FULL_CON)
                .with_action(ActionState::new("Float"))
                .with_loaded(true),
        )?;
        let upright_index = upright_engine
            .find_object_index(upright_id)
            .expect("upright object exists");
        upright_engine.tick_without_snapshot()?;
        assert_eq!(
            upright_engine.objects[upright_index].state.t_attach,
            -255_i32 as u32
        );
        assert_eq!(
            upright_engine.objects[upright_index].state.shape_attach.x, 1,
            "C4Shape::Attach consumes the low-byte CNAT_Left mask"
        );
        assert!(
            upright_engine.objects[upright_index]
                .state
                .shape_attach
                .mat_valid
        );

        // CollectionLimit compares the nonnegative content count directly to
        // the signed limit, so a negative limit is already full. Grab and
        // Rotate use C++ integer truthiness, while ContactIncinerate requires
        // a positive value for OCF_Inflammable.
        let mut behavior_engine = Engine::with_seed(156);
        behavior_engine.set_physics(PhysicsSettings::new(0, 200, -200));
        behavior_engine.register_definition(legacy_definition.clone())?;
        let behavior_id = behavior_engine.spawn_object(
            SpawnConfig::new("SIGN")
                .with_category(CATEGORY_OBJECT)
                .with_construction(FULL_CON)
                .with_rotation_velocity(itofix(4))
                .with_mobile(true),
        )?;
        let behavior = behavior_engine
            .object_snapshot(behavior_id)
            .expect("signed object exists");
        assert_eq!(behavior.ocf & ocf::COLLECTION, 0);
        assert_eq!(behavior.ocf & ocf::INFLAMMABLE, 0);
        assert_ne!(behavior.ocf & ocf::GRAB, 0);
        assert_ne!(behavior.ocf & ocf::ROTATE, 0);
        behavior_engine.tick_without_snapshot()?;
        assert_ne!(
            behavior_engine
                .object_snapshot(behavior_id)
                .expect("signed object survives")
                .rotation,
            0,
            "negative Rotate remains truthy and has no positive angle cap"
        );

        // Material fire tests ContactIncinerate for nonzero, unlike the
        // positive-only OCF path above.
        let library =
            MaterialLibrary::parse("[Material Lava]\nName=Lava\nDensity=0\nIncindiary=1\n")
                .expect("material library parses");
        let mut lava_engine = Engine::with_seed(157);
        lava_engine.set_materials(MaterialSet::from_resource_library(&library));
        lava_engine.set_landscape(exec_life_material_landscape(
            20,
            3,
            "Lava",
            &[(1, 0), (1, 1), (1, 2)],
        ));
        lava_engine.set_physics(PhysicsSettings::new(0, 200, -200));
        lava_engine.register_definition(legacy_definition.clone())?;
        let lava_id = lava_engine.spawn_object(
            SpawnConfig::new("SIGN")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(1, 1))
                .with_construction(FULL_CON),
        )?;
        lava_engine.frame = 9;
        lava_engine.tick_without_snapshot()?;
        assert!(
            lava_engine
                .object_snapshot(lava_id)
                .expect("lava object survives")
                .on_fire,
            "negative ContactIncinerate remains nonzero for incendiary material"
        );

        // BorderBound is a raw signed bit mask. -1 enables the top bit and
        // clamps the same fixed-motion target as the native engine.
        let mut border_engine = Engine::with_seed(158);
        border_engine.set_landscape(Landscape::flat(10, 20));
        border_engine.set_physics(PhysicsSettings::new(0, 20, -20));
        border_engine.register_definition(legacy_definition.clone())?;
        let border_id = border_engine.spawn_object(
            SpawnConfig::new("SIGN")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(5, 2))
                .with_construction(FULL_CON),
        )?;
        let border_index = border_engine
            .find_object_index(border_id)
            .expect("border object exists");
        border_engine.objects[border_index].set_fixed_velocity(FixedVec2::new(
            C4Fixed::ZERO,
            -itofix(5),
        ));
        border_engine.objects[border_index].state.mobile = true;
        border_engine.tick_without_snapshot()?;
        assert_eq!(
            border_engine
                .object_snapshot(border_id)
                .expect("border object survives")
                .position
                .y,
            1
        );

        // ConstructionCheck subtracts ConSizeOff from the shape height.
        // The blocker is inside the expanded negative-offset rectangle but
        // well above the ordinary two-pixel construction rectangle.
        let mut site_engine = Engine::with_seed(159);
        site_engine.set_landscape(Landscape::flat(40, 30));
        site_engine.register_definition(legacy_definition)?;
        let mut ordinary = simple_definition("ZERO");
        ordinary.set_category(CATEGORY_OBJECT);
        ordinary.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        ordinary.set_constructable(true);
        site_engine.register_definition(ordinary)?;
        let mut blocker = simple_definition("BLCK");
        blocker.set_category(CATEGORY_OBJECT);
        blocker.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        site_engine.register_definition(blocker)?;
        let site = Vector2::new(20, 30);
        assert!(site_engine.construction_site_valid("SIGN", site));
        assert!(site_engine.construction_site_valid("ZERO", site));
        site_engine.spawn_object(
            SpawnConfig::new("BLCK")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(20, 15)),
        )?;
        assert!(!site_engine.construction_site_valid("SIGN", site));
        assert!(site_engine.construction_site_valid("ZERO", site));
        Ok(())
    }

    #[test]
    fn definition_version_fallback_matches_cpp_compare_version() -> Result<(), EngineError> {
        // C4Def::Load replaces versions older than 4.0 with 4.9.10.7
        // (src/C4Def.cpp:573-581). CompareVersion ignores the build slot
        // when the left build is non-positive (src/C4GameVersion.h:66-79).
        let mut definition = Definition::from_script("VERS", "Versioned", "")?;
        assert_eq!(definition.version(), DEFAULT_DEFINITION_VERSION);
        definition.set_version([3, 99, 99, 99, 99]);
        assert_eq!(definition.version(), DEFAULT_DEFINITION_VERSION);

        definition.set_version([4, 0, 0, 0, -1]);
        assert_eq!(definition.version(), [4, 0, 0, 0, -1]);
        Ok(())
    }

    #[test]
    fn definition_from_resource_carries_auto_context_menu_like_cpp() -> Result<(), EngineError> {
        // C4DefCore::CompileFunc reads AutoContextMenu into the definition
        // (src/C4Def.cpp:416); C4Object::AutoContextMenu consults that flag
        // on the containing definition (src/C4Object.cpp:2049-2056).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Hut.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=HUT3\nName=Hut\nCategory=C4D_Structure\nAutoContextMenu=1\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;

        assert!(definition.auto_context_menu());
        Ok(())
    }

    #[test]
    fn definition_load_paths_carry_signed_no_get_like_cpp() -> Result<(), EngineError> {
        // C4DefCore::CompileFunc retains NoGet as an int32_t with default 0
        // (src/C4Def.cpp:412; src/C4Def.h:264). Both resource-loading paths
        // must expose its nonzero menu-exclusion semantics.
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Locked.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=LOCK\nName=Locked\nNoGet=-2\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");

        let definition = Definition::from_resource(&resource)?;
        assert!(definition.no_get());

        let mut legacy_definition = Definition::from_script("LOCK", "Locked", "")?;
        assert!(!legacy_definition.no_get(), "Definition default is false");
        Engine::apply_resource_core(&mut legacy_definition, &resource.core);
        assert!(legacy_definition.no_get());
        Ok(())
    }

    #[test]
    fn legacy_resource_core_retains_chop_ocf_like_cpp() -> Result<(), EngineError> {
        // Legacy scenario loading compiles Script.c first and then applies
        // the parsed DefCore wholesale. `Chop=1` must survive that second
        // path so UpdateOCF exposes OCF_Chop for a standing StaticBack tree
        // (C4Def.cpp:378; C4Object.cpp:705-710).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Tree2.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=TRE2\nName=Tree2\nCategory=C4D_StaticBack\nWidth=40\nHeight=56\nOffset=-20,-28\nChop=1\n",
        )
        .expect("write defcore");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let mut definition =
            Definition::from_script("TRE2", "Tree2", "").expect("compile legacy script");

        Engine::apply_resource_core(&mut definition, &resource.core);

        let mut engine = Engine::with_seed(4);
        engine.register_definition(definition)?;
        let tree = engine.spawn_object(
            SpawnConfig::new("TRE2")
                .with_category(CATEGORY_STATIC_BACK)
                .with_position(Vector2::new(40, 60)),
        )?;
        assert_ne!(
            engine.object_snapshot(tree).expect("tree exists").ocf & ocf::CHOP,
            0,
            "the legacy scenario Definition path must retain DefCore Chop=1"
        );
        Ok(())
    }

    /// A pixel-grid landscape whose texmap carries a Vehicle slot (byte
    /// 2) so C4SolidMask baking is active (grid mode, `put_solid_mask`);
    /// byte 1 is solid Earth, byte 0 sky.
    fn vehicle_grid_landscape(width: u32, height: u32) -> Landscape {
        let densities = vec![0, 100, 100];
        let names = vec![None, Some("Earth".into()), Some("Vehicle".into())];
        let grid = landscape::PixelGrid::new(
            width,
            height,
            vec![0u8; (width * height) as usize],
            densities,
            names,
            vec![None; 3],
        );
        let mut landscape =
            Landscape::new(width, vec![0; width as usize]).expect("landscape builds");
        landscape.set_world_height(height as i32);
        landscape.set_pixel_grid(grid);
        landscape
    }

    /// Every landscape pixel currently carrying the Vehicle byte
    /// (row-major order).
    fn vehicle_pixels(engine: &Engine) -> Vec<(i32, i32)> {
        let landscape = engine.landscape().expect("landscape set");
        let (width, height) = landscape.grid_dimensions().expect("grid mode");
        let vehicle = landscape.grid_vehicle_byte().expect("vehicle byte");
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|&(x, y)| landscape.grid_byte_at(x, y) == Some(vehicle))
            .collect()
    }

    fn one_pixel_sprite(alpha: u8) -> DefinitionSpriteImage {
        DefinitionSpriteImage {
            width: 1,
            height: 1,
            pixels: Arc::from([0, 0, 0, alpha]),
            color_mask: None,
        }
    }

    fn solid_mask_sprite(width: u32, height: u32, alphas: &[u8]) -> DefinitionSpriteImage {
        assert_eq!(alphas.len(), (width * height) as usize);
        let pixels: Arc<[u8]> = alphas
            .iter()
            .flat_map(|&alpha| [0, 0, 0, alpha])
            .collect::<Vec<_>>()
            .into();
        DefinitionSpriteImage {
            width,
            height,
            pixels,
            color_mask: None,
        }
    }

    fn movement_mask_definition(
        id: &str,
        width: i32,
        contact_vertex_x: i32,
    ) -> Definition {
        let mut definition = simple_definition(id);
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, width, 1)));
        definition.set_shape_vertices(vec![
            ObjectVertex::new(contact_vertex_x, 0).with_cnat(CNAT_RIGHT),
        ]);
        definition.set_solid_mask(Some(DefinitionTargetRect::new(
            0, 0, width, 1, 0, 0,
        )));
        definition.set_contact_density(50);
        definition
    }

    #[test]
    fn dig_free_runs_before_movers_own_baked_mask_is_removed() {
        // C4Object::DoMovement runs DigFree while the mover's mask is still
        // put (C4Movement.cpp:227-245); MCVehic is non-diggable, and the
        // saved Earth+IFT byte must survive the tail remove/re-put cycle.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            DigFree=1

            [Material Vehicle]
            Name=Vehicle
            Density=100
            DigFree=0
        "#,
        )
        .expect("materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let mut landscape = vehicle_grid_landscape(20, 20);
        landscape.grid_write_byte(10, 10, 0x81);
        landscape.grid_write_byte(11, 10, 1);

        let mut drill = movement_mask_definition("DRIL", 1, 0);
        // The 2x1 DigFreeRect includes one unmasked Earth sentinel, proving
        // the dig ran while only the mask-covered first pixel was shielded.
        drill.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 1)));
        drill.configure_actions(
            Some("Drill".to_string()),
            HashMap::from([(
                "Drill".to_string(),
                ActionSpec::default().with_dig_free(1),
            )]),
        );

        let mut engine = Engine::with_seed(61);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(drill).expect("drill registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("DRIL")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_action(ActionState::new("Drill"))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("drill spawns");
        let index = engine.find_object_index(id).expect("drill exists");
        engine.update_solid_mask(index);
        assert_eq!(engine.debug_solid_mask_buffer(id.as_u64()), Some(vec![0x81]));
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(10, 10)),
            Some(2)
        );

        engine.tick_without_snapshot().expect("stationary dig frame succeeds");

        let index = engine.find_object_index(id).expect("drill remains");
        assert_eq!(engine.debug_solid_mask_buffer(id.as_u64()), Some(vec![0x81]));
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(10, 10)),
            Some(2),
            "DigFree must see the put Vehicle pixel"
        );
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(11, 10)),
            Some(0),
            "the same DigFreeRect clears adjacent unmasked Earth"
        );
        engine.remove_solid_mask(index);
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(10, 10)),
            Some(0x81),
            "mask removal restores the undug Earth byte with IFT"
        );
    }

    #[test]
    fn blast_free_leaves_the_landscape_under_a_solid_mask_intact_like_cpp() {
        // A flint thrown at an elevator case resting on stone leaves the
        // stone under the case's floor behind. That is what C++ does, so it
        // is pinned rather than fixed. C4Game::Explosion reaches BlastFree
        // directly (C4Effect.cpp:919) and BlastFree — unlike ClearRect
        // (C4Landscape.cpp:2171-2181) — carries no PrepareChange/FinishChange
        // bracket (C4Landscape.cpp:1022-1062), so it scans the *masked*
        // plane. Every put SolidMask pixel reads MCVehic, i.e. material
        // MVehic, and BlastFreePix clears only when
        // Game.Material.Map[mat].BlastFree is set (C4Landscape.cpp:941-960);
        // the engine's Vehicle material never sets it. C4SolidMask::Remove
        // then restores the background byte saved before the blast
        // (C4SolidMask.cpp:241-260).
        let library = MaterialLibrary::parse(
            r#"
            [Material Rock]
            Name=Rock
            Density=50
            BlastFree=1

            [Material Vehicle]
            Name=Vehicle
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let rock = materials.id_of("Rock").expect("rock exists");

        let grid = landscape::PixelGrid::new(
            20,
            20,
            vec![0u8; 400],
            vec![0, 50, 100],
            vec![None, Some("Rock".into()), Some("Vehicle".into())],
            vec![None; 3],
        );
        let mut world = Landscape::new(20, vec![0; 20]).expect("landscape builds");
        world.set_world_height(20);
        world.set_pixel_grid(grid);
        // One undiggable seam: the r=2 crater around (10,10) exposes
        // (8,10)/(9,10), shields (10,10)/(11,10) under the case floor, and
        // never reaches the (12,10) sentinel.
        for x in 8..13 {
            world.grid_write_byte(x, 10, 1);
        }

        let mut case = simple_definition("ELEV");
        case.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 1)));
        case.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 2, 1, 0, 0)));

        let mut engine = Engine::with_seed(43);
        engine.set_materials(materials);
        engine.set_landscape(world);
        engine.register_definition(case).expect("case registers");
        let id = engine
            .spawn_object(SpawnConfig::new("ELEV").with_position(Vector2::new(10, 10)))
            .expect("case spawns");
        let index = engine.find_object_index(id).expect("case exists");
        // Park the case flush on the seam, the way an elevator sits at its
        // lowest position; spawn placement would otherwise lift it clear.
        engine.objects[index].state.position = Vector2::new(10, 10);
        engine.objects[index].fixed_position = FixedVec2::from_ints(10, 10);
        engine.update_solid_mask(index);
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10), (11, 10)]);
        assert_eq!(
            engine.debug_solid_mask_buffer(id.as_u64()),
            Some(vec![1; 2])
        );

        let result = engine
            .blast_circle(Vector2::new(10, 10), 2, None)
            .expect("blast applies");

        let vehicle = engine.materials().id_of("Vehicle").expect("vehicle exists");
        assert_eq!(result.pixel_count_by_material.get(&rock), Some(&2));
        assert_eq!(
            result.pixel_count_by_material.get(&vehicle),
            Some(&2),
            "BlastMatCount books the masked pixels as MVehic, not as Rock"
        );
        assert_eq!(result.removed_by_material.get(&rock), Some(&2));

        let landscape = engine.landscape().expect("landscape remains set");
        for x in [8, 9] {
            assert_eq!(
                landscape.material_at(x, 10),
                None,
                "exposed Rock inside the crater is blasted free at ({x}, 10)"
            );
        }
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(10, 10), (11, 10)],
            "BlastFreePix leaves MCVehic alone"
        );

        engine.remove_solid_mask(index);
        let landscape = engine.landscape().expect("landscape remains set");
        for x in [10, 11] {
            assert_eq!(
                landscape.material_at(x, 10),
                Some(rock),
                "mask removal re-exposes the Rock the blast never saw at ({x}, 10)"
            );
        }
        assert_eq!(landscape.material_at(12, 10), Some(rock));
    }

    #[test]
    fn mover_contact_switches_own_mask_at_first_domotion() {
        // Before the first successful DoMotion, ContactCheck sees the own
        // bake. DoMotion removes it before changing x, so every later step
        // in the same frame sees the restored background.
        for (vertex_x, velocity, expected_x, expected_contact) in
            [(-1, 1, 10, CNAT_RIGHT), (-2, 2, 12, 0)]
        {
            let mut engine = Engine::with_seed(62);
            engine.set_landscape(vehicle_grid_landscape(24, 20));
            engine.set_physics(PhysicsSettings::new(0, 20, -20));
            engine
                .register_definition(movement_mask_definition("MASK", 1, vertex_x))
                .expect("mask mover registers");
            let id = engine
                .spawn_object(
                    SpawnConfig::new("MASK")
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(10, 10))
                        .with_fixed_position(FixedVec2::from_ints(10, 10))
                        .with_fixed_velocity(FixedVec2::new(
                            itofix(velocity),
                            C4Fixed::ZERO,
                        ))
                        .with_mobile(true)
                        .with_loaded(true),
                )
                .expect("mask mover spawns");
            let index = engine.find_object_index(id).expect("mask mover exists");
            engine.update_solid_mask(index);

            engine.tick_without_snapshot().expect("movement frame succeeds");

            let index = engine.find_object_index(id).expect("mask mover remains");
            assert_eq!(engine.objects[index].state.position.x, expected_x);
            assert_eq!(engine.objects[index].fixed_position.x, itofix(expected_x));
            assert_eq!(
                engine.objects[index].frame_t_contact & CNAT_RIGHT,
                expected_contact,
                "vertex={vertex_x}, velocity={velocity}"
            );
        }
    }

    #[test]
    fn later_contact_callback_sees_background_restored_by_first_domotion() {
        // The first candidate step is free and therefore removes the old
        // solid-mask bake before committing x=11. The second candidate hits
        // Earth through the +1 contact vertex at world x=13. ContactRight
        // runs while the object is still at x=11, so its relative (-1,0)
        // probe is exactly the old mask cell (10,10). C++ reads the live
        // Surface8 there: sky, not the stale MCVehic byte from movement entry.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Vehicle]
            Name=Vehicle
            Density=100
        "#,
        )
        .expect("materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let mut landscape = vehicle_grid_landscape(24, 20);
        landscape.grid_write_byte(13, 10, 1);

        let mut mover = Definition::from_script(
            "CBMS",
            "Callback mask mover",
            r#"
            #strict 2
            local old_mask_solid, old_mask_material;
            global func ContactRight()
            {
                old_mask_solid = GBackSolid(-1, 0);
                old_mask_material = GetMaterial(-1, 0);
                return 0;
            }
            "#,
        )
        .expect("mover script compiles");
        mover.set_category(CATEGORY_OBJECT);
        mover.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        mover.set_shape_vertices(vec![ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT)]);
        mover.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        mover.set_contact_density(50);
        mover.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(65);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(mover).expect("mover registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("CBMS")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_fixed_velocity(FixedVec2::new(itofix(2), C4Fixed::ZERO))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("mover spawns");
        let index = engine.find_object_index(id).expect("mover exists");
        engine.update_solid_mask(index);
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(10, 10)),
            Some(2),
            "the old-position mask is baked before movement"
        );

        engine.tick_without_snapshot().expect("two-step movement frame succeeds");

        let object = engine.object_snapshot(id).expect("mover remains");
        assert_eq!(object.position, Vector2::new(11, 10));
        assert_eq!(
            object.local_vars.get("old_mask_solid"),
            Some(&Value::Bool(false)),
            "GBackSolid in the later contact callback sees restored sky"
        );
        assert_eq!(
            object.local_vars.get("old_mask_material"),
            Some(&Value::Int(-1)),
            "GetMaterial in the later contact callback sees MNone, not Vehicle"
        );
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(10, 10)),
            Some(0),
            "the tail re-put leaves the old mask cell restored"
        );
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(11, 10)),
            Some(2),
            "the mask is re-put at the committed position"
        );
    }

    #[test]
    fn movement_callback_recreated_mask_drops_stale_rider_backup_like_cpp() {
        // DoMotion's Remove(true,true) stores the rider backup inside the
        // exact C4SolidMask instance (oracle-src-pinned src/C4Movement.cpp:
        // 121-126; src/C4SolidMask.cpp:276-305). SetSolidMask deletes that
        // instance and immediately creates/re-puts a replacement
        // (src/C4Object.cpp:3809-3817), so DoMovement's final
        // UpdateSolidMask(true) cannot carry the old instance's rider
        // (src/C4Movement.cpp:443-445; src/C4SolidMask.cpp:178-195).
        let mut landscape = vehicle_grid_landscape(24, 20);
        landscape.grid_write_byte(13, 10, 1);

        let mut mover = Definition::from_script(
            "RMSK",
            "Recreated mask mover",
            r#"
            #strict 2
            local contact_calls;
            protected func ContactRight()
            {
                ++contact_calls;
                SetSolidMask(0, 0, 1, 1);
                return 0;
            }
            "#,
        )
        .expect("mover script compiles");
        mover.set_c4_callback_convention(true);
        mover.set_category(CATEGORY_OBJECT);
        mover.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        mover.set_shape_vertices(vec![
            ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT),
        ]);
        mover.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        mover.set_sprite_image(Some(one_pixel_sprite(255)));
        mover.set_contact_density(50);
        mover.set_contact_function_calls(true);

        let mut rider = simple_definition("RIDR");
        rider.set_category(CATEGORY_OBJECT);
        rider.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        rider.set_shape_vertices(vec![
            ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM),
        ]);
        rider.set_contact_density(50);

        let mut engine = Engine::with_seed(66);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(mover).expect("mover registers");
        engine.register_definition(rider).expect("rider registers");
        let mover = engine
            .spawn_object(
                SpawnConfig::new("RMSK")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_fixed_velocity(FixedVec2::new(itofix(2), C4Fixed::ZERO))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("mover spawns");
        let mover_index = engine.find_object_index(mover).expect("mover exists");
        engine.update_solid_mask(mover_index);
        let rider = engine
            .spawn_object(
                SpawnConfig::new("RIDR")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 9))
                    .with_fixed_position(FixedVec2::from_ints(10, 9))
                    .with_loaded(true),
            )
            .expect("rider spawns");
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);

        let definition_id = engine.objects[mover_index].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("mover definition remains")
            .action_library()
            .clone();
        engine
            .exec_object_movement(mover_index, &actions, &definition_id, &[])
            .expect("contacting movement succeeds");

        let mover = engine.object_snapshot(mover).expect("mover remains");
        assert_eq!(mover.position, Vector2::new(11, 10));
        assert_eq!(
            mover.local_vars.get("contact_calls"),
            Some(&Value::Int(1)),
            "the second candidate invokes the mask-recreating callback"
        );
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(11, 10)],
            "the callback's replacement mask remains put at the committed position"
        );
        assert_eq!(
            engine.object_snapshot(rider).expect("rider remains").position,
            Vector2::new(10, 9),
            "the replacement mask must not inherit the deleted instance's rider backup"
        );
    }

    #[test]
    fn solid_mask_rider_capture_respects_contact_density_boundary() {
        fn move_platform(contact_density: i32) -> (Vector2, Vector2) {
            let mut platform = movement_mask_definition("PLAT", 3, -2);
            platform.set_category(CATEGORY_OBJECT);
            let mut rider = simple_definition("RIDE");
            rider.set_category(CATEGORY_OBJECT);
            rider.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
            rider.set_shape_vertices(vec![
                ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM),
            ]);
            rider.set_contact_density(contact_density);

            let mut engine = Engine::with_seed(63);
            engine.set_landscape(vehicle_grid_landscape(30, 20));
            engine.set_physics(PhysicsSettings::new(0, 20, -20));
            engine
                .register_definition(platform)
                .expect("platform registers");
            engine.register_definition(rider).expect("rider registers");
            let platform = engine
                .spawn_object(
                    SpawnConfig::new("PLAT")
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(10, 10))
                        .with_fixed_position(FixedVec2::from_ints(10, 10))
                        .with_fixed_velocity(FixedVec2::new(itofix(2), C4Fixed::ZERO))
                        .with_mobile(true)
                        .with_loaded(true),
                )
                .expect("platform spawns");
            let platform_index = engine
                .find_object_index(platform)
                .expect("platform exists");
            engine.update_solid_mask(platform_index);
            let rider = engine
                .spawn_object(
                    SpawnConfig::new("RIDE")
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(10, 9))
                        .with_fixed_position(FixedVec2::from_ints(10, 9))
                        .with_contact_density(contact_density)
                        .with_loaded(true),
                )
                .expect("rider spawns");

            engine.tick_without_snapshot().expect("platform movement succeeds");

            (
                engine
                    .object_snapshot(platform)
                    .expect("platform remains")
                    .position,
                engine.object_snapshot(rider).expect("rider remains").position,
            )
        }

        for (contact_density, expected_rider) in
            [(50, Vector2::new(12, 9)), (51, Vector2::new(10, 9))]
        {
            let (platform, rider) = move_platform(contact_density);
            assert_eq!(platform, Vector2::new(12, 10));
            assert_eq!(rider, expected_rider, "ContactDensity={contact_density}");
        }
    }

    #[test]
    fn fully_off_landscape_solid_mask_put_still_restores_rider() {
        fn plane_bytes(engine: &Engine) -> Vec<u8> {
            let landscape = engine.landscape().expect("landscape remains");
            let (width, height) = landscape.grid_dimensions().expect("grid mode");
            (0..height)
                .flat_map(|y| (0..width).map(move |x| landscape.grid_byte_at(x, y).unwrap()))
                .collect()
        }

        // The platform starts with its one-pixel mask on the final landscape
        // column, then moves onto the inclusive right boundary. C++ records
        // that fully clipped put as MaskPut=true and therefore carries the
        // backed-up rider even though no Vehicle pixel is written.
        let mut platform = movement_mask_definition("EDGE", 1, -2);
        platform.set_category(CATEGORY_OBJECT);
        let mut rotated = movement_mask_definition("ROFF", 1, -2);
        rotated.set_category(CATEGORY_OBJECT);
        rotated.set_rotated_solid_masks(true);
        let mut rider = simple_definition("RIDE");
        rider.set_category(CATEGORY_OBJECT);
        rider.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        rider.set_shape_vertices(vec![
            ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM),
        ]);
        rider.set_contact_density(50);

        let mut landscape = vehicle_grid_landscape(16, 20);
        landscape.set_border_open(20, 20, true, false);
        landscape.grid_write_byte(3, 3, 1);
        let mut engine = Engine::with_seed(67);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(platform)
            .expect("platform registers");
        engine
            .register_definition(rotated)
            .expect("rotated mask registers");
        engine.register_definition(rider).expect("rider registers");
        let platform = engine
            .spawn_object(
                SpawnConfig::new("EDGE")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(15, 10))
                    .with_fixed_position(FixedVec2::from_ints(15, 10))
                    .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("platform spawns");
        let platform_index = engine
            .find_object_index(platform)
            .expect("platform exists");
        engine.update_solid_mask(platform_index);
        let rider = engine
            .spawn_object(
                SpawnConfig::new("RIDE")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(15, 9))
                    .with_fixed_position(FixedVec2::from_ints(15, 9))
                    .with_loaded(true),
            )
            .expect("rider spawns");
        assert_eq!(vehicle_pixels(&engine), vec![(15, 10)]);

        engine.tick_without_snapshot().expect("edge movement succeeds");

        assert_eq!(
            engine
                .object_snapshot(platform)
                .expect("platform remains on inclusive boundary")
                .position,
            Vector2::new(16, 10)
        );
        assert_eq!(
            engine.object_snapshot(rider).expect("rider remains").position,
            Vector2::new(16, 9),
            "the empty put still restores the captured rider by the movement delta"
        );
        let platform_index = engine
            .find_object_index(platform)
            .expect("platform remains");
        assert_eq!(
            engine.debug_solid_mask_is_put(platform.as_u64()),
            Some(true),
            "the fully clipped mask remains explicitly put without raster data"
        );
        assert_eq!(engine.debug_solid_mask_buffer(platform.as_u64()), None);
        assert!(vehicle_pixels(&engine).is_empty());

        let plane_before_remove = plane_bytes(&engine);
        engine.remove_solid_mask(platform_index);
        let plane_after_remove = plane_bytes(&engine);
        assert_eq!(
            plane_after_remove, plane_before_remove,
            "removing an empty put is a complete raster no-op"
        );
        assert_eq!(
            engine.debug_solid_mask_is_put(platform.as_u64()),
            Some(false)
        );
        assert_eq!(engine.debug_solid_mask_buffer(platform.as_u64()), None);

        // Rotation has a separate clipping branch and must record the same
        // logical put without attempting a negative-sized allocation.
        let rotated = engine
            .spawn_object(
                SpawnConfig::new("ROFF")
                    .with_position(Vector2::new(30, 10))
                    .with_rotation(45)
                    .with_loaded(true),
            )
            .expect("offscreen rotated mask spawns");
        assert_eq!(
            engine.debug_solid_mask_is_put(rotated.as_u64()),
            Some(true)
        );
        assert_eq!(engine.debug_solid_mask_buffer(rotated.as_u64()), None);
        let plane_before_remove = plane_bytes(&engine);
        let rotated_index = engine
            .find_object_index(rotated)
            .expect("rotated mask remains");
        engine.remove_solid_mask(rotated_index);
        assert_eq!(plane_bytes(&engine), plane_before_remove);
        assert_eq!(
            engine.debug_solid_mask_is_put(rotated.as_u64()),
            Some(false)
        );
    }

    #[test]
    fn solid_mask_riders_restore_in_sector_shape_order() {
        // Native capture walks the expanded platform rect's ObjectShapes
        // lists sector by sector. A is in sector 0 and B in sector 1 even
        // though loaded object-vec order is B, A. The order is observable:
        // A's offset mask initially blocks B's destination and must move
        // away before B is restored (C4SolidMask.cpp:282-305,184-194).
        let mut platform = movement_mask_definition("PLAT", 20, -2);
        platform.set_category(CATEGORY_OBJECT);

        let rider_definition = |id: &str, own_mask| {
            let mut rider = simple_definition(id);
            rider.set_category(CATEGORY_OBJECT);
            rider.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
            rider.set_shape_vertices(vec![
                ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM),
            ]);
            rider.set_contact_density(50);
            if own_mask {
                rider.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 2, 0)));
            }
            rider
        };

        let mut engine = Engine::with_seed(66);
        engine.set_landscape(vehicle_grid_landscape(120, 60));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(platform).expect("platform registers");
        engine
            .register_definition(rider_definition("RIDB", false))
            .expect("plain rider registers");
        engine
            .register_definition(rider_definition("RIDA", true))
            .expect("masked rider registers");

        let platform = engine
            .spawn_object(
                SpawnConfig::new("PLAT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(45, 30))
                    .with_fixed_position(FixedVec2::from_ints(45, 30))
                    .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("platform spawns");
        let platform_index = engine
            .find_object_index(platform)
            .expect("platform exists");
        engine.update_solid_mask(platform_index);
        let rider_b = engine
            .spawn_object(
                SpawnConfig::new("RIDB")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 29))
                    .with_fixed_position(FixedVec2::from_ints(50, 29))
                    .with_loaded(true),
            )
            .expect("plain rider spawns first");
        let rider_b_index = engine.find_object_index(rider_b).expect("plain rider exists");
        let rider_a = engine
            .spawn_object(
                SpawnConfig::new("RIDA")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(49, 29))
                    .with_fixed_position(FixedVec2::from_ints(49, 29))
                    .with_loaded(true),
            )
            .expect("masked rider spawns second");
        let rider_a_index = engine.find_object_index(rider_a).expect("masked rider exists");
        engine.update_solid_mask(rider_a_index);
        assert!(
            rider_b_index < rider_a_index,
            "object-vector order is deliberately the reverse of sector order"
        );

        engine.tick_without_snapshot().expect("platform movement succeeds");

        assert_eq!(
            engine.object_snapshot(platform).expect("platform remains").position,
            Vector2::new(46, 30)
        );
        assert_eq!(
            engine.object_snapshot(rider_a).expect("masked rider remains").position,
            Vector2::new(50, 29),
            "sector-0 rider moves first, taking its own mask away"
        );
        assert_eq!(
            engine.object_snapshot(rider_b).expect("plain rider remains").position,
            Vector2::new(51, 29),
            "sector-1 rider can then enter the vacated mask pixel"
        );
    }

    #[test]
    fn no_motion_frame_still_cycles_overlapping_solid_mask_bakes() {
        let mut engine = Engine::with_seed(64);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(movement_mask_definition("MSKA", 1, 0))
            .expect("first mask registers");
        engine
            .register_definition(movement_mask_definition("MSKB", 1, 0))
            .expect("second mask registers");
        let first = engine
            .spawn_object(
                SpawnConfig::new("MSKA")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("first mask spawns");
        let second = engine
            .spawn_object(
                SpawnConfig::new("MSKB")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_mobile(false)
                    .with_loaded(true),
            )
            .expect("second mask spawns");
        let first_index = engine.find_object_index(first).expect("first exists");
        let second_index = engine.find_object_index(second).expect("second exists");
        engine.remove_solid_mask(first_index);
        engine.remove_solid_mask(second_index);
        engine.update_solid_mask(first_index);
        engine.update_solid_mask(second_index);
        assert_eq!(engine.debug_solid_mask_buffer(first.as_u64()), Some(vec![0]));
        assert_eq!(engine.debug_solid_mask_buffer(second.as_u64()), Some(vec![2]));

        let definition_id = engine.objects[first_index].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("first mask definition remains")
            .action_library()
            .clone();
        let solid_mask_indices = (0..engine.objects.len()).collect::<Vec<_>>();
        engine
            .exec_object_movement(
                first_index,
                &actions,
                &definition_id,
                &solid_mask_indices,
            )
            .expect("no-motion DoMovement succeeds");

        assert_eq!(engine.debug_solid_mask_buffer(first.as_u64()), Some(vec![2]));
        assert_eq!(engine.debug_solid_mask_buffer(second.as_u64()), Some(vec![0]));
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);
    }

    fn switching_mask_definition() -> Definition {
        let mut gate = Definition::from_script(
            "GATE",
            "Gate",
            r#"
            #strict 2
            public func SwitchMask() { return SetSolidMask(1, 0, 1, 1); }
            public func SwitchOther(object target) { return target->SwitchMask(); }
            "#,
        )
        .expect("gate script compiles");
        gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        gate.set_sprite_image(Some(DefinitionSpriteImage {
            width: 2,
            height: 1,
            pixels: Arc::from([0, 0, 0, 0, 255, 255, 255, 255]),
            color_mask: None,
        }));
        gate
    }

    fn persisted_mask_gate_definition() -> Definition {
        let mut gate = Definition::from_script(
            "SGAT",
            "Saved gate",
            r#"
            #strict 2
            public func ShiftMask() { return SetSolidMask(0, 0, 1, 1, 1, 0); }
            public func OpenMask() { return SetSolidMask(0, 0, 0, 0); }
            "#,
        )
        .expect("saved gate script compiles");
        gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        gate.set_sprite_image(Some(one_pixel_sprite(255)));
        gate
    }

    #[test]
    fn solid_mask_state_capture_cleans_and_restore_reputs_like_cpp() {
        // Landscape persistence brackets Surface8 serialization with
        // RemoveSolidMasks(false,false) / PutSolidMasks (C4Game.cpp:
        // 4137-4147). The material buffer itself is NoSave; the object's
        // effective SolidMask rect is saved and used to create a fresh bake.
        let mut landscape = vehicle_grid_landscape(20, 20);
        landscape.grid_write_byte(10, 10, 1);
        landscape.grid_write_byte(11, 10, 1);

        let mut engine = Engine::with_seed(27);
        engine.set_landscape(landscape);
        engine
            .register_definition(persisted_mask_gate_definition())
            .expect("saved gate registers");
        let gate = engine
            .spawn_object(
                SpawnConfig::new("SGAT")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("saved gate spawns");
        let gate_index = engine.find_object_index(gate).expect("saved gate exists");
        engine.update_solid_mask(gate_index);
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);

        engine
            .call_object_function(gate_index, "ShiftMask", Vec::new())
            .expect("gate mask shifts");
        assert_eq!(vehicle_pixels(&engine), vec![(11, 10)]);

        let rng_before = engine.debug_rng_clone();
        let state = engine.capture_state();
        let saved_landscape = state.landscape.as_ref().expect("landscape captured");
        assert_eq!(saved_landscape.grid_byte_at(10, 10), Some(1));
        assert_eq!(saved_landscape.grid_byte_at(11, 10), Some(1));
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(11, 10)],
            "capture must not remove the live mask"
        );
        assert_eq!(engine.debug_rng_clone(), rng_before);

        engine.restore_state(&state).expect("state restores");
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(11, 10)],
            "restore must re-put the saved effective mask"
        );
        assert_eq!(engine.debug_rng_clone(), rng_before);

        let gate_index = engine.find_object_index(gate).expect("gate restored");
        engine
            .call_object_function(gate_index, "OpenMask", Vec::new())
            .expect("restored gate opens");
        let restored = engine.landscape().expect("restored landscape");
        assert_eq!(restored.grid_byte_at(10, 10), Some(1));
        assert_eq!(restored.grid_byte_at(11, 10), Some(1));
        assert!(vehicle_pixels(&engine).is_empty());
    }

    #[test]
    fn restore_snapshot_does_not_reput_over_its_runtime_baked_landscape() {
        // SimulationSnapshot is a live frame projection, not the C++
        // landscape-persistence bracket: its plane already contains masks.
        // Feeding it through restore_state must not put a second, default
        // mask on top of an already-baked runtime override.
        let mut landscape = vehicle_grid_landscape(20, 20);
        landscape.grid_write_byte(10, 10, 1);
        landscape.grid_write_byte(11, 10, 1);

        let mut engine = Engine::with_seed(28);
        engine.set_landscape(landscape);
        engine
            .register_definition(persisted_mask_gate_definition())
            .expect("snapshot gate registers");
        let gate = engine
            .spawn_object(
                SpawnConfig::new("SGAT")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("snapshot gate spawns");
        let gate_index = engine.find_object_index(gate).expect("snapshot gate exists");
        engine.update_solid_mask(gate_index);
        engine
            .call_object_function(gate_index, "ShiftMask", Vec::new())
            .expect("snapshot gate mask shifts");
        assert_eq!(vehicle_pixels(&engine), vec![(11, 10)]);

        let snapshot = engine.snapshot();
        engine
            .restore_snapshot(&snapshot)
            .expect("runtime snapshot restores");
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(11, 10)],
            "restore_snapshot must not add the definition-default mask"
        );
    }

    #[test]
    fn solid_mask_restore_reputs_before_loaded_ocf_recomputation() {
        // C4GameObjects::Load runs UpdateFaces (which puts masks) before
        // SetOCF. A mask over sky therefore contributes OCF_InSolid to its
        // own restored object even though the persisted plane is clean.
        let mut engine = Engine::with_seed(29);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine
            .register_definition(persisted_mask_gate_definition())
            .expect("OCF gate registers");
        let gate = engine
            .spawn_object(
                SpawnConfig::new("SGAT")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("OCF gate spawns");
        let gate_index = engine.find_object_index(gate).expect("OCF gate exists");
        engine.update_solid_mask(gate_index);

        let state = engine.capture_state();
        assert_eq!(
            state
                .landscape
                .as_ref()
                .expect("clean OCF landscape captured")
                .grid_byte_at(10, 10),
            Some(0)
        );
        engine.restore_state(&state).expect("OCF state restores");
        assert_ne!(
            engine.object_snapshot(gate).expect("OCF gate restores").ocf & ocf::IN_SOLID,
            0,
            "loaded SetOCF must see the re-put mask"
        );
    }

    #[test]
    fn set_solid_mask_callback_rebakes_the_landscape_like_cpp() {
        // FnSetSolidMask calls C4Object::SetSolidMask, which removes the old
        // mask and immediately creates and puts the new one
        // (C4Script.cpp:271-278; C4Object.cpp:3809-3818). Goldrush's CTWR
        // UpdateTransferZone switches from a transparent saved source pixel
        // to an opaque one during game-start synchronization.
        let mut engine = Engine::with_seed(7);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine
            .register_definition(switching_mask_definition())
            .expect("gate registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("GATE")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("gate spawns");
        assert_eq!(vehicle_pixels(&engine), Vec::<(i32, i32)>::new());

        let idx = engine.find_object_index(id).expect("gate exists");
        let result = engine
            .call_object_function(idx, "SwitchMask", Vec::new())
            .expect("SetSolidMask callback succeeds");
        assert_eq!(result, Value::Bool(true));

        assert_eq!(
            engine.debug_solid_mask_override(id.as_u64()),
            Some(Some((1, 0, 1, 1)))
        );
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);
    }

    #[test]
    fn solid_mask_without_shape_uses_zero_shape_offset_like_cpp() -> Result<(), EngineError> {
        // C4DefCore::Default and C4Shape::CompileFunc leave every omitted
        // Shape member at zero. C4Def::Load still validates SolidMask against
        // Graphics.png, and C4SolidMask::Put adds the zero shape x/y to the
        // target offset (C4Def.cpp:117-133,206-243,727-733;
        // C4Shape.cpp:496-508; C4SolidMask.cpp:63-76).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("ShapelessMask.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=ZMSK\nName=Shapeless mask\nCategory=C4D_Object\nSolidMask=1,0,2,1,-2,3\n",
        )
        .expect("write DefCore");
        std::fs::write(
            def_dir.join("Script.c"),
            b"#strict 2\npublic func Reput() { SetSolidMask(1, 0, 2, 1, -2, 3); return GBackSolid(-2, 3); }\n",
        )
        .expect("write script");
        image::RgbaImage::from_pixel(3, 1, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Graphics.png"))
            .expect("write graphics");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;
        assert_eq!(definition.shape_rect(), None, "Shape must remain omitted");
        assert_eq!(
            definition.solid_mask(),
            Some(DefinitionTargetRect::new(1, 0, 2, 1, -2, 3)),
            "the valid bitmap-bounded mask must survive loading"
        );

        let mut engine = Engine::with_seed(70);
        engine.set_landscape(vehicle_grid_landscape(30, 30));
        engine.register_definition(definition)?;
        let object =
            engine.spawn_object(SpawnConfig::new("ZMSK").with_position(Vector2::new(10, 10)))?;
        assert_eq!(vehicle_pixels(&engine), vec![(8, 13), (9, 13)]);
        assert_eq!(
            engine.debug_solid_mask_buffer(object.as_u64()),
            Some(vec![0, 0])
        );

        let index = engine.find_object_index(object).expect("object exists");
        assert_eq!(
            engine.call_object_function(index, "Reput", Vec::new())?,
            Value::Bool(true),
            "Reput must see the callback-private zero-offset mask synchronously"
        );
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(8, 13), (9, 13)],
            "callback-time SetSolidMask must use the same zero shape offset"
        );
        assert_eq!(
            engine.debug_solid_mask_buffer(object.as_u64()),
            Some(vec![0, 0])
        );

        // A non-grid landscape cannot rasterize CreateObject's live mask.
        // Stuck must therefore consume pending_solid_mask's placement before
        // the host outcome materializes the new object.
        let mut pending_engine = Engine::with_seed(71);
        pending_engine.set_landscape(Landscape::flat_with_material(30, 30, None));
        pending_engine.register_definition(Definition::from_resource(&resource)?)?;
        let mut caller = Definition::from_script(
            "CALL",
            "Caller",
            r#"#strict 2
            public func ProbePendingMask() {
                CreateObject(ZMSK, 0, 0, -1);
                return Stuck();
            }
            "#,
        )?;
        caller.set_shape_vertices(vec![ObjectVertex::new(-2, 3)]);
        caller.set_contact_density(50);
        pending_engine.register_definition(caller)?;
        let caller_id = pending_engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(10, 8)))?;
        assert!(
            !pending_engine
                .landscape()
                .expect("landscape remains")
                .is_solid_at(8, 11),
            "the pending-mask probe starts in sky"
        );
        let caller_index = pending_engine
            .find_object_index(caller_id)
            .expect("caller exists");
        assert_eq!(
            pending_engine.call_object_function(caller_index, "ProbePendingMask", Vec::new())?,
            Value::Bool(true),
            "Stuck must see CreateObject's pending zero-offset mask before materialization"
        );
        Ok(())
    }

    #[test]
    fn set_solid_mask_clamps_negative_origin_and_stores_height_disable_like_cpp() {
        // CheckSolidMaskRect moves a negative source origin to zero, but its
        // width/height limits use the OLD coordinates. Thus (-1,-1,3,3) on
        // a 2x2 bitmap remains 3x3. The retained out-of-bitmap row/column
        // read as zero from GetPixDw and are solid under C++'s inverted-alpha
        // transparency test (C4Object.cpp:3820-3827; C4SolidMask.cpp:400-412).
        let mut definition = Definition::from_script(
            "CLMP",
            "Clamped mask",
            r#"
            #strict 2
            public func ClipMask() { return SetSolidMask(-1, -1, 3, 3, 4, 5); }
            public func DisableMask() { return SetSolidMask(0, 3, 2, 1, 4, 5); }
            "#,
        )
        .expect("clamped-mask script compiles");
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 3, 3)));
        definition.set_sprite_image(Some(solid_mask_sprite(
            2,
            2,
            &[
                0, 255, // row 0: transparent, solid
                0, 0, // row 1: transparent, transparent
            ],
        )));

        let mut engine = Engine::with_seed(40);
        engine.set_landscape(vehicle_grid_landscape(30, 30));
        engine
            .register_definition(definition)
            .expect("clamped-mask definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("CLMP")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("clamped-mask object spawns");
        let index = engine.find_object_index(id).expect("object exists");
        assert!(vehicle_pixels(&engine).is_empty());

        let result = engine
            .call_object_function(index, "ClipMask", Vec::new())
            .expect("negative-origin SetSolidMask succeeds");
        assert_eq!(result, Value::Bool(true));
        assert_eq!(
            engine.debug_solid_mask_override(id.as_u64()),
            Some(Some((0, 0, 3, 3)))
        );
        let clipped_pixels = vec![
            (15, 15),
            (16, 15),
            (16, 16),
            (14, 17),
            (15, 17),
            (16, 17),
        ];
        assert_eq!(vehicle_pixels(&engine), clipped_pixels);
        assert_eq!(
            engine.debug_solid_mask_buffer(id.as_u64()),
            Some(vec![2, 0, 0, 2, 2, 0, 0, 0, 0])
        );

        // Callback preview already clamps; an authoritative rebuild proves
        // that the clamped rectangle, rather than the raw request, persisted.
        engine.update_solid_mask(index);
        assert_eq!(vehicle_pixels(&engine), clipped_pixels);
        assert_eq!(
            engine.debug_solid_mask_buffer(id.as_u64()),
            Some(vec![2, 0, 0, 2, 2, 0, 0, 0, 0])
        );

        engine
            .call_object_function(index, "DisableMask", Vec::new())
            .expect("out-of-height SetSolidMask succeeds");
        // height=min(1, 2-3)=-1; CheckSolidMaskRect then forces width to 0
        // while retaining that negative height and both target offsets.
        assert_eq!(
            engine.debug_solid_mask_override(id.as_u64()),
            Some(Some((0, 3, 0, -1)))
        );
        engine.update_solid_mask(index);
        assert!(vehicle_pixels(&engine).is_empty());
        assert_eq!(engine.debug_solid_mask_buffer(id.as_u64()), None);
    }

    #[test]
    fn nested_set_solid_mask_callback_rebakes_the_target_like_cpp() {
        // An object-targeted script call mutates that same live C4Object in
        // C++, so its nested SetSolidMask also re-puts the target mask before
        // returning (C4Script.cpp:271-278; C4Object.cpp:3809-3818).
        let mut engine = Engine::with_seed(8);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine
            .register_definition(switching_mask_definition())
            .expect("gate registers");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("GATE")
                    .with_position(Vector2::new(5, 5))
                    .with_loaded(true),
            )
            .expect("caller spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("GATE")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("target spawns");
        assert_eq!(vehicle_pixels(&engine), Vec::<(i32, i32)>::new());

        let idx = engine.find_object_index(caller).expect("caller exists");
        let result = engine
            .call_object_function(
                idx,
                "SwitchOther",
                vec![Value::Object(target.as_u64())],
            )
            .expect("nested SetSolidMask succeeds");

        assert_eq!(result, Value::Bool(true));
        assert_eq!(
            engine.debug_solid_mask_override(target.as_u64()),
            Some(Some((1, 0, 1, 1)))
        );
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);
    }

    #[test]
    fn set_graphics_rebuilds_solid_mask_from_the_active_bitmap_like_cpp() {
        // C4Object::SetGraphics selects a C4DefGraphics from the requested
        // source definition and immediately calls UpdateGraphics(true), which
        // deletes/recreates the solid mask (src/C4Object.cpp:5908-5923,
        // :381-402). C4SolidMask then samples that ACTIVE bitmap
        // (src/C4SolidMask.cpp:400-412).
        let mut gate = Definition::from_script(
            "GATE",
            "Gate",
            r#"
            #strict 2
            public func Switch() { var no_object; return SetGraphics("2", no_object, SKIN); }
            public func Reset() { var no_name; return SetGraphics(no_name); }
            "#,
        )
        .expect("gate script compiles");
        gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));
        gate.set_sprite_image(Some(one_pixel_sprite(0)));

        let mut skin = simple_definition("SKIN");
        skin.set_sprite_image(Some(one_pixel_sprite(0)));
        skin.set_sprite_variants(HashMap::from([(
            "2".to_string(),
            one_pixel_sprite(255),
        )]));

        let mut engine = Engine::with_seed(7);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine.register_definition(gate).expect("gate registers");
        engine.register_definition(skin).expect("skin registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("GATE")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("gate spawns");
        assert_eq!(vehicle_pixels(&engine), Vec::<(i32, i32)>::new());

        let idx = engine.find_object_index(id).expect("gate exists");
        engine
            .call_object_function(idx, "Switch", Vec::new())
            .expect("SetGraphics callback succeeds");
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);

        let idx = engine.find_object_index(id).expect("gate exists");
        engine
            .call_object_function(idx, "Reset", Vec::new())
            .expect("default graphics restore succeeds");
        assert_eq!(vehicle_pixels(&engine), Vec::<(i32, i32)>::new());
    }

    const GRAPHICS_BOUNDS_REFLECTION_SCRIPT: &[u8] = br#"#strict 2
func ProbeGraphicsBounds() {
    var no_definition;
    return [
        GetDefCoreVal("SolidMask", "DefCore", no_definition, 0),
        GetDefCoreVal("SolidMask", "DefCore", no_definition, 1),
        GetDefCoreVal("SolidMask", "DefCore", no_definition, 2),
        GetDefCoreVal("SolidMask", "DefCore", no_definition, 3),
        GetDefCoreVal("SolidMask", "DefCore", no_definition, 4),
        GetDefCoreVal("SolidMask", "DefCore", no_definition, 5),
        GetDefCoreVal("TopFace", "DefCore", no_definition, 0),
        GetDefCoreVal("TopFace", "DefCore", no_definition, 1),
        GetDefCoreVal("TopFace", "DefCore", no_definition, 2),
        GetDefCoreVal("TopFace", "DefCore", no_definition, 3),
        GetDefCoreVal("TopFace", "DefCore", no_definition, 4),
        GetDefCoreVal("TopFace", "DefCore", no_definition, 5)
    ];
}
"#;

    #[test]
    fn invalid_base_defcore_graphics_bounds_clear_runtime_and_reflection_like_cpp(
    ) -> Result<(), EngineError> {
        // C4Def::Load validates the raw SolidMask against bitmap pixels and
        // TopFace against bitmap/Scale logical coordinates, defaulting all
        // six reflected fields before objects or the renderer see either
        // invalid rectangle (C4Def.cpp:727-741).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("InvalidGraphicsBounds.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=BADS\nName=Invalid graphics bounds\nCategory=C4D_Object\nWidth=64\nHeight=64\nOffset=0,0\nScale=200\nSolidMask=0,0,9999,9999,9,10\nTopFace=31,0,2,1,7,8\n",
        )
        .expect("write DefCore");
        std::fs::write(
            def_dir.join("Script.c"),
            GRAPHICS_BOUNDS_REFLECTION_SCRIPT,
        )
        .expect("write reflection script");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Graphics.png"))
            .expect("write graphics");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;
        assert_eq!(definition.solid_mask(), None);
        assert_eq!(definition.top_face(), None);

        let mut engine = Engine::with_seed(41);
        engine.set_landscape(vehicle_grid_landscape(128, 128));
        engine.register_definition(definition)?;
        assert_eq!(engine.definition_top_face("BADS"), None);
        let object = engine.spawn_object(
            SpawnConfig::new("BADS")
                .with_position(Vector2::new(64, 64))
                .with_loaded(true),
        )?;
        let index = engine.find_object_index(object).expect("object exists");
        assert_eq!(
            engine.call_object_function(index, "ProbeGraphicsBounds", Vec::new())?,
            Value::Array(vec![Value::Int(0); 12])
        );
        assert!(vehicle_pixels(&engine).is_empty());
        assert_eq!(engine.debug_solid_mask_buffer(object.as_u64()), None);
        Ok(())
    }

    #[test]
    fn valid_base_defcore_graphics_bounds_preserve_exact_edges_like_cpp(
    ) -> Result<(), EngineError> {
        // SolidMask uses raw bitmap pixels, while TopFace uses logical
        // bitmap/Scale coordinates. C++ accepts exact right/bottom edges.
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("ValidGraphicsBounds.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=EDGE\nName=Exact graphics edges\nCategory=C4D_Object\nWidth=64\nHeight=64\nOffset=0,0\nScale=200\nSolidMask=0,0,64,64,9,10\nTopFace=0,0,32,32,7,8\n",
        )
        .expect("write DefCore");
        std::fs::write(
            def_dir.join("Script.c"),
            GRAPHICS_BOUNDS_REFLECTION_SCRIPT,
        )
        .expect("write reflection script");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 255]))
            .save(def_dir.join("Graphics.png"))
            .expect("write graphics");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;
        assert_eq!(
            definition.solid_mask(),
            Some(DefinitionTargetRect::new(0, 0, 64, 64, 9, 10))
        );
        assert_eq!(
            definition.top_face(),
            Some(DefinitionTargetRect::new(0, 0, 32, 32, 7, 8))
        );

        let mut engine = Engine::with_seed(42);
        engine.register_definition(definition)?;
        let object = engine.spawn_object(SpawnConfig::new("EDGE"))?;
        let index = engine.find_object_index(object).expect("object exists");
        assert_eq!(
            engine.call_object_function(index, "ProbeGraphicsBounds", Vec::new())?,
            Value::Array(vec![
                Value::Int(0),
                Value::Int(0),
                Value::Int(64),
                Value::Int(64),
                Value::Int(9),
                Value::Int(10),
                Value::Int(0),
                Value::Int(0),
                Value::Int(32),
                Value::Int(32),
                Value::Int(7),
                Value::Int(8),
            ])
        );
        Ok(())
    }

    #[test]
    fn solid_mask_init_and_graphics_changes_clamp_without_reexpanding_like_cpp() {
        // C4Object::Init copies the definition rectangle into the object and
        // clamps that per-object copy before the first UpdateFace/Put. Use a
        // synthetic definition: the resource loader may reject an invalid
        // DefCore rectangle before object initialization can exercise this.
        {
            let mut definition = simple_definition("ICLP");
            definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 4, 2)));
            definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 4, 2, 0, 0)));
            definition.set_sprite_image(Some(solid_mask_sprite(3, 2, &[255; 6])));

            let mut engine = Engine::with_seed(41);
            engine.set_landscape(vehicle_grid_landscape(30, 30));
            engine
                .register_definition(definition)
                .expect("init-clamp definition registers");
            let id = engine
                .spawn_object(
                    SpawnConfig::new("ICLP")
                        .with_position(Vector2::new(10, 12)),
                )
                .expect("init-clamp object spawns");
            let index = engine.find_object_index(id).expect("object exists");
            assert_eq!(
                engine.debug_solid_mask_override(id.as_u64()),
                Some(Some((0, 0, 3, 2)))
            );
            let initial_pixels = vec![
                (10, 10),
                (11, 10),
                (12, 10),
                (10, 11),
                (11, 11),
                (12, 11),
            ];
            assert_eq!(vehicle_pixels(&engine), initial_pixels);
            assert_eq!(
                engine.debug_solid_mask_buffer(id.as_u64()),
                Some(vec![0; 6])
            );
            engine.update_solid_mask(index);
            assert_eq!(vehicle_pixels(&engine), initial_pixels);
        }

        // Start this object with a fully valid 4x2 mask, shrink its active
        // bitmap to 2x1, then return to 4x2. CheckSolidMaskRect mutates the
        // object rectangle, so the final graphics change must not restore the
        // definition's original 4x2 dimensions (C4Object.cpp:381-402).
        let mut gate = Definition::from_script(
            "GCLP",
            "Graphics-clamped gate",
            r#"
            #strict 2
            public func Switch() { var no_object; return SetGraphics("small", no_object, SKIN); }
            public func Same() { var no_value; return SetGraphics("SMALL", no_value, SKIN, 0, 0, no_value, 123); }
            public func Reset() { var no_name; return SetGraphics(no_name); }
            "#,
        )
        .expect("graphics-clamp script compiles");
        gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 4, 2)));
        gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 4, 2, 0, 0)));
        gate.set_sprite_image(Some(solid_mask_sprite(4, 2, &[255; 8])));

        let mut skin = simple_definition("SKIN");
        skin.set_sprite_variants(HashMap::from([(
            "small".to_string(),
            solid_mask_sprite(2, 1, &[255; 2]),
        )]));

        let mut engine = Engine::with_seed(42);
        engine.set_landscape(vehicle_grid_landscape(30, 30));
        engine.register_definition(gate).expect("gate registers");
        engine.register_definition(skin).expect("skin registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("GCLP")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("graphics-clamp gate spawns");
        let index = engine.find_object_index(id).expect("gate exists");
        assert_eq!(
            vehicle_pixels(&engine),
            vec![
                (10, 10),
                (11, 10),
                (12, 10),
                (13, 10),
                (10, 11),
                (11, 11),
                (12, 11),
                (13, 11),
            ]
        );
        assert_eq!(
            engine.debug_solid_mask_buffer(id.as_u64()),
            Some(vec![0; 8])
        );

        engine
            .call_object_function(index, "Switch", Vec::new())
            .expect("smaller named graphics succeeds");
        assert_eq!(
            engine.debug_solid_mask_override(id.as_u64()),
            Some(Some((0, 0, 2, 1)))
        );
        let small_pixels = vec![(10, 10), (11, 10)];
        assert_eq!(vehicle_pixels(&engine), small_pixels);
        engine.update_solid_mask(index);
        assert_eq!(vehicle_pixels(&engine), small_pixels);
        assert_eq!(
            engine.debug_solid_mask_buffer(id.as_u64()),
            Some(vec![0; 2])
        );

        engine
            .call_object_function(index, "Same", Vec::new())
            .expect("same named graphics succeeds");
        let base_graphics = engine
            .object_snapshot(id)
            .expect("gate remains")
            .base_graphics
            .expect("named graphics remain selected");
        assert_eq!(base_graphics.graphics_name.as_deref(), Some("small"));
        assert_eq!(base_graphics.blit_mode, 0, "base SetGraphics ignores blit mode");
        assert_eq!(
            engine.debug_solid_mask_override(id.as_u64()),
            Some(Some((0, 0, 2, 1)))
        );
        engine.update_solid_mask(index);
        assert_eq!(vehicle_pixels(&engine), small_pixels);

        engine
            .call_object_function(index, "Reset", Vec::new())
            .expect("default graphics restore succeeds");
        assert_eq!(
            engine.debug_solid_mask_override(id.as_u64()),
            Some(Some((0, 0, 2, 1)))
        );
        engine.update_solid_mask(index);
        assert_eq!(vehicle_pixels(&engine), small_pixels);
        assert_eq!(
            engine.debug_solid_mask_buffer(id.as_u64()),
            Some(vec![0; 2])
        );
    }

    #[test]
    fn solid_mask_removal_fires_instability_on_restored_pixels() {
        // C4SolidMask::Remove with fCauseInstability (C4SolidMask.cpp:
        // 255-257): every restored mask pixel gets a CheckInstabilityRange —
        // water freed from under a vehicle mask immediately re-arms its
        // mass mover. All rust removal paths mirror C++ Remove(true, ...)
        // callers (C4Object.cpp:5652/5667, C4Movement.cpp:123/545).
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Water]
            Name=Water
            Density=25
            Instable=1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");

        let densities = vec![0, 100, 100, 25];
        let names = vec![
            None,
            Some("Earth".into()),
            Some("Vehicle".into()),
            Some("Water".into()),
        ];
        let mut bytes = vec![0u8; 400];
        bytes[10 * 20 + 10] = 3; // water at (10,10)
        let grid = landscape::PixelGrid::new(20, 20, bytes, densities, names, vec![None; 4]);
        let mut landscape = Landscape::new(20, vec![0; 20]).expect("landscape builds");
        landscape.set_pixel_grid(grid);

        let mut definition = simple_definition("Bar");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, 0, 3, 1)));
        definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 3, 1, 0, 0)));

        let mut engine = Engine::with_seed(7);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Bar").with_position(Vector2::new(10, 10)))
            .expect("bar spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.position = Vector2::new(10, 10);
        engine.objects[idx].fixed_position = FixedVec2::from_ints(10, 10);
        engine.update_solid_mask(idx);
        assert!(
            vehicle_pixels(&engine).contains(&(10, 10)),
            "the mask covers the water pixel"
        );
        assert_eq!(engine.mass_movers.live_movers(), 0);

        engine.remove_solid_mask(idx);

        let landscape = engine.landscape().expect("landscape set");
        assert_eq!(
            landscape.material_at(10, 10),
            Some(water),
            "the water pixel restored from the mask buffer"
        );
        assert!(
            engine.mass_movers.live_movers() >= 1,
            "the restored water pixel re-armed a mass mover"
        );
    }

    #[test]
    fn game_synchronize_reowns_overlapping_masks_without_instability_like_cpp() {
        // C4GameObjects::Synchronize removes every mask First->Next with
        // Remove(false, false), then re-puts them in that same order. Rust's
        // exec list is the reverse master list, so spawning loaded B then A
        // makes A the canonical owner even though the masks start put B->A.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Water]
            Name=Water
            Density=25
            Instable=1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);

        let mut bytes = vec![0u8; 400];
        bytes[10 * 20 + 10] = 3;
        let grid = landscape::PixelGrid::new(
            20,
            20,
            bytes,
            vec![0, 100, 100, 25],
            vec![
                None,
                Some("Earth".into()),
                Some("Vehicle".into()),
                Some("Water".into()),
            ],
            vec![None; 4],
        );
        let mut landscape = Landscape::new(20, vec![0; 20]).expect("landscape builds");
        landscape.set_world_height(20);
        landscape.set_pixel_grid(grid);

        let mut mask = simple_definition("Mask");
        mask.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        mask.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = Engine::with_seed(33);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine.register_definition(mask).expect("mask registers");
        let b = engine
            .spawn_object(
                SpawnConfig::new("Mask")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("B spawns");
        let a = engine
            .spawn_object(
                SpawnConfig::new("Mask")
                    .with_position(Vector2::new(10, 10))
                    .with_loaded(true),
            )
            .expect("A spawns");
        assert_eq!(engine.debug_exec_order(), vec![b, a]);

        // Loaded object face setup puts masks as each object joins the list,
        // so the initial ownership order is already B then A.
        assert_eq!(engine.debug_solid_mask_buffer(b.as_u64()), Some(vec![3]));
        assert_eq!(engine.debug_solid_mask_buffer(a.as_u64()), Some(vec![2]));
        assert_eq!(engine.mass_movers.live_movers(), 0);

        engine
            .execute_synchronize_control(false, false)
            .expect("game synchronization succeeds");

        assert_eq!(engine.debug_exec_order(), vec![b, a]);
        assert_eq!(
            engine.debug_solid_mask_buffer(a.as_u64()),
            Some(vec![3]),
            "master-list-first A must own the saved Water background"
        );
        assert_eq!(
            engine.debug_solid_mask_buffer(b.as_u64()),
            Some(vec![2]),
            "later B must record the Vehicle sentinel at the overlap"
        );
        assert_eq!(
            engine
                .landscape()
                .expect("landscape set")
                .grid_byte_at(10, 10),
            Some(2)
        );
        assert_eq!(
            engine.mass_movers.live_movers(),
            0,
            "Remove(false, false) must not probe restored instable pixels"
        );
    }

    #[test]
    fn rotated_solid_mask_bakes_inverse_rotated_pixels_like_cpp() {
        // Mirrors the rotated branch of C4SolidMask::Put
        // (src/C4SolidMask.cpp:108-174), gated by Def->RotatedSolidmasks
        // (src/C4Object.cpp:5655).
        //
        // Hand-derived golden: 3x1 bar mask (shape rect -1,0,3,1;
        // SolidMask=0,0,3,1,0,0), object center (10,10), r=90.
        // MatBuffPitch = int(sqrt(9+1))+1 = 4 (ctor, src/C4SolidMask.cpp:415).
        // Sin(-90) = -1, Cos(-90) = 0 (SineTable exact at multiples of 90),
        // centerx = -1+0+1 = 0, centery = 0+0+0 = 0, so
        // xstart = 10+0-2 = 8, ystart = 8 (src/C4SolidMask.cpp:114-117).
        // Per cell (xcnt,ycnt) of the 4x4 square (:130-173):
        //   iMx = fixtoi((ycnt-2)*Ma2) + 1 = ycnt-1   in [0,3) => ycnt 1..=3
        //   iMy = fixtoi((xcnt-2)*Mb1) + 0 = 2-xcnt   in [0,1) => xcnt = 2
        // The horizontal bar bakes as the vertical bar (10,9),(10,10),(10,11).
        let mut definition = simple_definition("Bar");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, 0, 3, 1)));
        definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 3, 1, 0, 0)));
        definition.set_rotated_solid_masks(true);

        let mut engine = Engine::with_seed(7);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Bar").with_position(Vector2::new(10, 10)))
            .expect("bar spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.position = Vector2::new(10, 10);
        engine.objects[idx].fixed_position = FixedVec2::from_ints(10, 10);
        engine.objects[idx].state.rotation = 90;
        engine.objects[idx].fixed_rotation = itofix(90);
        engine.update_solid_mask(idx);

        assert_eq!(vehicle_pixels(&engine), vec![(10, 9), (10, 10), (10, 11)]);
    }

    #[test]
    fn script_set_r_reflows_and_same_angle_rebakes_solid_mask_like_cpp() {
        // FnSetR calls C4Object::SetRotation, which removes the put mask,
        // resets r/fix_r, then UpdateFace(true) recreates and puts the mask
        // at the new angle (C4Script.cpp:738-746; C4Object.cpp:357-380,
        // 5637-5647). SetRotation does this even when nr equals the current
        // angle, so a missing bake must be restored by same-angle SetR.
        let mut definition = Definition::from_script(
            "Bar",
            "Bar",
            r#"#strict
            public func Rotate90() { return SetR(90); }
            "#,
        )
        .expect("bar script compiles");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, 0, 3, 1)));
        definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 3, 1, 0, 0)));
        definition.set_rotated_solid_masks(true);

        let mut engine = Engine::with_seed(9);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine
            .register_definition(definition)
            .expect("bar registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Bar").with_position(Vector2::new(10, 10)))
            .expect("bar spawns");
        let idx = engine.find_object_index(id).expect("bar exists");
        engine.objects[idx].state.position = Vector2::new(10, 10);
        engine.objects[idx].fixed_position = FixedVec2::from_ints(10, 10);
        engine.update_solid_mask(idx);
        assert_eq!(vehicle_pixels(&engine), vec![(9, 10), (10, 10), (11, 10)]);

        let result = engine
            .call_object_function(idx, "Rotate90", Vec::new())
            .expect("SetR callback succeeds");
        assert_eq!(result, Value::Bool(true));
        assert_eq!(engine.object_snapshot(id).expect("bar survives").rotation, 90);
        assert_eq!(vehicle_pixels(&engine), vec![(10, 9), (10, 10), (10, 11)]);

        let idx = engine.find_object_index(id).expect("bar remains");
        engine.remove_solid_mask(idx);
        assert!(vehicle_pixels(&engine).is_empty());

        let result = engine
            .call_object_function(idx, "Rotate90", Vec::new())
            .expect("same-angle SetR callback succeeds");
        assert_eq!(result, Value::Bool(true));
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(10, 9), (10, 10), (10, 11)],
            "same-angle SetR must still rebuild and put the solid mask"
        );
    }

    #[test]
    fn rotated_solid_mask_at_45_degrees_bakes_diamond_superset_like_cpp() {
        // Mirrors src/C4SolidMask.cpp:130-173 at a non-cardinal angle.
        //
        // Hand-derived golden: 3x3 mask (shape rect -1,-1,3,3;
        // SolidMask=0,0,3,3,0,0), object center (10,10), r=45.
        // MatBuffPitch = int(sqrt(18))+1 = 5; SineTable[4500] = 46340, so
        // Ma1=Ma2=Mb2=46340, Mb1=-46340; centerx=centery=0 =>
        // xstart=ystart=8. Per 5x5 cell: iMx = fixtoi((xcnt-2+ycnt-2)*
        // 46340)+1, iMy = fixtoi((-(xcnt-2)+ycnt-2)*46340)+1; e.g. cell
        // (2,0): iMx = fixtoi(-92680)+1 = 0, iMy = fixtoi(-92680+92680)+1
        // = 1 => hit at (10,8); cell (0,0): iMx = fixtoi(-185360)+1 = -2
        // => miss. The rotated square covers the unrotated 3x3 PLUS the
        // four diagonal corner pixels — the enlarged diamond.
        let mut definition = simple_definition("Sqr");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 3, 3)));
        definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 3, 3, 0, 0)));
        definition.set_rotated_solid_masks(true);

        let mut engine = Engine::with_seed(7);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Sqr").with_position(Vector2::new(10, 10)))
            .expect("square spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.position = Vector2::new(10, 10);
        engine.objects[idx].fixed_position = FixedVec2::from_ints(10, 10);
        engine.objects[idx].state.rotation = 45;
        engine.objects[idx].fixed_rotation = itofix(45);
        engine.update_solid_mask(idx);

        assert_eq!(
            vehicle_pixels(&engine),
            vec![
                (10, 8),
                (9, 9),
                (10, 9),
                (11, 9),
                (8, 10),
                (9, 10),
                (10, 10),
                (11, 10),
                (12, 10),
                (9, 11),
                (10, 11),
                (11, 11),
                (10, 12),
            ]
        );
    }

    #[test]
    fn rotated_solid_mask_removal_restores_landscape_exactly_like_cpp() {
        // Mirrors src/C4SolidMask.cpp:240-259: Remove restores the saved
        // background bytes from the MatBuffPitch-pitched buffer wherever
        // the buffer byte is not MCVehic — for a rotated bake that is
        // exactly the inverse-rotation hit set of Put.
        let mut definition = simple_definition("Sqr");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 3, 3)));
        definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 3, 3, 0, 0)));
        definition.set_rotated_solid_masks(true);

        let mut engine = Engine::with_seed(7);
        let mut landscape = vehicle_grid_landscape(20, 20);
        // Earth under three of the diamond pixels (mask hits) and one
        // outside it (never touched).
        landscape.grid_write_byte(10, 8, 1);
        landscape.grid_write_byte(10, 10, 1);
        landscape.grid_write_byte(9, 11, 1);
        landscape.grid_write_byte(3, 3, 1);
        let original = landscape.pixel_grid().expect("grid set").bytes().to_vec();
        engine.set_landscape(landscape);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Sqr").with_position(Vector2::new(10, 10)))
            .expect("square spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.position = Vector2::new(10, 10);
        engine.objects[idx].fixed_position = FixedVec2::from_ints(10, 10);
        engine.objects[idx].state.rotation = 45;
        engine.objects[idx].fixed_rotation = itofix(45);
        engine.update_solid_mask(idx);
        assert!(
            vehicle_pixels(&engine).contains(&(10, 8)),
            "rotated mask must be baked before the removal is meaningful"
        );

        engine.remove_solid_mask(idx);

        let restored = engine
            .landscape()
            .expect("landscape set")
            .pixel_grid()
            .expect("grid set")
            .bytes()
            .to_vec();
        assert_eq!(restored, original);
    }

    #[test]
    fn rotation_without_rotated_solidmasks_removes_the_mask_like_cpp() {
        // Mirrors the C4Object::UpdateSolidMask gate
        // (src/C4Object.cpp:5648-5668): without Def->RotatedSolidmasks a
        // rotated object falls through to "remove and destroy mask".
        let mut definition = simple_definition("Sqr");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 3, 3)));
        definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 3, 3, 0, 0)));

        let mut engine = Engine::with_seed(7);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Sqr").with_position(Vector2::new(10, 10)))
            .expect("square spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.position = Vector2::new(10, 10);
        engine.objects[idx].fixed_position = FixedVec2::from_ints(10, 10);
        engine.update_solid_mask(idx);
        assert!(
            !vehicle_pixels(&engine).is_empty(),
            "the unrotated mask must bake first"
        );

        engine.objects[idx].state.rotation = 45;
        engine.objects[idx].fixed_rotation = itofix(45);
        engine.update_solid_mask(idx);

        assert_eq!(vehicle_pixels(&engine), Vec::<(i32, i32)>::new());
        assert!(engine.objects[idx].solid_mask_bake.is_none());
    }

    #[test]
    fn removing_overlapping_mask_reputs_rotated_mask_pixels_like_cpp() {
        // Mirrors src/C4SolidMask.cpp:262-273: Remove re-puts every other
        // put mask across the freed rect; for a rotated mask the re-put
        // membership is the same inverse-rotation sample as Put
        // (src/C4SolidMask.cpp:144-167 with RegularPut false).
        //
        // Order matters: the unrotated 1x1 blocker puts FIRST and owns
        // the shared pixel's Earth backup; the rotated bar puts second
        // and stores MCVehic there (unused for restore). Removing the
        // blocker restores Earth, then the re-put of the rotated bar
        // must claim the pixel back (and refresh its buffer with Earth,
        // C4SolidMask.cpp:156-160), or the bar would be left with a hole.
        let mut bar = simple_definition("Bar");
        bar.set_shape_rect(Some(DefinitionRect::new(-1, 0, 3, 1)));
        bar.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 3, 1, 0, 0)));
        bar.set_rotated_solid_masks(true);
        let mut blocker = simple_definition("Blk");
        blocker.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        blocker.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = Engine::with_seed(7);
        let mut landscape = vehicle_grid_landscape(20, 20);
        landscape.grid_write_byte(10, 10, 1);
        let original = landscape.pixel_grid().expect("grid set").bytes().to_vec();
        engine.set_landscape(landscape);
        engine.register_definition(bar).expect("bar registers");
        engine
            .register_definition(blocker)
            .expect("blocker registers");

        // Blocker puts first: its 1x1 mask owns the Earth backup at (10,10).
        let blocker_id = engine
            .spawn_object(SpawnConfig::new("Blk").with_position(Vector2::new(10, 10)))
            .expect("blocker spawns");
        let blocker_idx = engine
            .find_object_index(blocker_id)
            .expect("blocker exists");
        engine.objects[blocker_idx].state.position = Vector2::new(10, 10);
        engine.objects[blocker_idx].fixed_position = FixedVec2::from_ints(10, 10);
        engine.update_solid_mask(blocker_idx);

        // Rotated bar puts second: vertical bar (10,9),(10,10),(10,11).
        let bar_id = engine
            .spawn_object(SpawnConfig::new("Bar").with_position(Vector2::new(10, 10)))
            .expect("bar spawns");
        let bar_idx = engine.find_object_index(bar_id).expect("bar exists");
        engine.objects[bar_idx].state.position = Vector2::new(10, 10);
        engine.objects[bar_idx].fixed_position = FixedVec2::from_ints(10, 10);
        engine.objects[bar_idx].state.rotation = 90;
        engine.objects[bar_idx].fixed_rotation = itofix(90);
        engine.update_solid_mask(bar_idx);
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(10, 9), (10, 10), (10, 11)],
            "both masks put; the shared pixel is baked once"
        );

        // Removing the blocker restores Earth at (10,10) and then the
        // re-put of the rotated bar must write MCVehic back.
        engine.remove_solid_mask(blocker_idx);
        assert_eq!(
            vehicle_pixels(&engine),
            vec![(10, 9), (10, 10), (10, 11)],
            "the rotated mask re-put reclaims the freed shared pixel"
        );

        // Removing the bar restores the original landscape, including
        // the Earth byte the re-put refreshed into the bar's buffer.
        engine.remove_solid_mask(bar_idx);
        let restored = engine
            .landscape()
            .expect("landscape set")
            .pixel_grid()
            .expect("grid set")
            .bytes()
            .to_vec();
        assert_eq!(restored, original);
    }

    #[test]
    fn definition_from_resource_get_desc_returns_trimmed_description() -> Result<(), EngineError> {
        // C4Def exposes the loaded Desc text through GetDesc
        // (C4Def.h:321,355); Context offers Info only when that text is
        // nonempty (C4ObjectMenu.cpp:410-423).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Hut3.c4d");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=HUT3\nName=Wooden Hut\n",
        )
        .expect("write DefCore");
        std::fs::write(def_dir.join("DescUS.txt"), b"  A safe home base.  \r\n")
            .expect("write description");
        std::fs::write(
            def_dir.join("Script.c"),
            b"#strict 2\npublic func ReadDesc() { return GetDesc(); }\n",
        )
        .expect("write definition script");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;

        assert_eq!(definition.description(), Some("A safe home base."));
        let mut engine = Engine::with_seed(0);
        engine.register_definition(definition)?;
        let hut = engine.spawn_object(SpawnConfig::new("HUT3"))?;
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        assert_eq!(
            engine.call_object_function(hut_index, "ReadDesc", Vec::new())?,
            Value::String("A safe home base.".into())
        );
        Ok(())
    }

    #[test]
    fn definition_exposes_portrait_and_rank_symbols_for_the_hud() -> Result<(), EngineError> {
        // C4Def loads Portrait*.* (C4CFN_Portraits, src/C4Components.h:88,
        // C4Def::LoadPortraits src/C4Def.cpp:1245-1259) and Rank.png
        // (pRankSymbols, src/C4Def.cpp:684-691); the HUD cursor info draws
        // both (C4ObjectInfo::Draw, src/C4ObjectInfo.cpp:308-341).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Crew.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=CRWT\nName=CrewTest\nCategory=C4D_Object\nColorByOwner=1\n",
        )
        .expect("write defcore");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 0]))
            .save(def_dir.join("Portrait1.png"))
            .expect("write portrait");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([136, 136, 136, 255]))
            .save(def_dir.join("Overlay1.png"))
            .expect("write portrait overlay");
        image::RgbaImage::from_pixel(3, 1, image::Rgba([70, 80, 90, 255]))
            .save(def_dir.join("PortraitCaptain1.png"))
            .expect("write named portrait");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([40, 50, 60, 255]))
            .save(def_dir.join("Rank.png"))
            .expect("write rank symbols");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;

        let mut engine = Engine::new();
        engine.register_definition(definition)?;

        let raw_portrait = engine
            .definition_portrait_image("CRWT")
            .expect("HUD portrait exposed");
        assert_eq!(
            raw_portrait.pixels().as_ref(),
            [0_u8, 0, 0, 0].repeat(4).as_slice(),
            "C4Surface loading canonicalizes fully-transparent portrait pixels"
        );
        let portrait = engine
            .definition_portrait_graphics_image("CRWT")
            .expect("color-aware portrait exposed");
        assert_eq!((portrait.width(), portrait.height()), (2, 2));
        assert_eq!(
            portrait.color_mask().as_deref(),
            Some([136, 136, 136, 255].repeat(4).as_slice()),
            "Portrait1's full Overlay1 RGBA reaches presentation"
        );
        let named = engine
            .definition_named_portrait_graphics_image("CRWT", "captain1")
            .expect("named portrait exposed case-insensitively");
        assert_eq!((named.width(), named.height()), (3, 1));
        let rank = engine
            .definition_rank_symbols_image("CRWT")
            .expect("rank symbols exposed");
        assert_eq!((rank.width(), rank.height()), (4, 2));
        Ok(())
    }

    #[test]
    fn contact_right_callback_runs_before_redirect_and_next_rng_consumer_like_cpp() {
        // Mirrors src/C4Movement.cpp:271-278: horizontal contact calls
        // ContactCheck before RedirectForce. ContactCheck runs shape contact and
        // then contact callbacks in src/C4Movement.cpp:166-182 via
        // C4Object::Contact at src/C4Movement.cpp:112-119.
        //
        // Hand-derived golden for seed 61: Engine startup does Randomize3(), i.e.
        // 500 calls to Random(3). ContactRight then consumes Random(100) = 13.
        // The following Step random argument is therefore the next
        // Random(i32::MAX) = 30827. ContactRight's SetXDir(40) runs before
        // RedirectForce, so xdir is itofix(4) - FIXED100(50), not the old xdir
        // redirect result.
        let script = r#"#strict 3
            global func ContactRight()
            {
                SetXDir(40);
                return Random(100);
            }

            global func Step(state, frame, random)
            {
                return { energy = random };
            }
        "#;

        let mut blocker_definition =
            Definition::from_script("Blocker", "Blocker", "").expect("script compiles");
        blocker_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        blocker_definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut mover_definition =
            Definition::from_script("Mover", "Mover", script).expect("script compiles");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);
        mover_definition.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(61);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(blocker_definition)
            .expect("blocker definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 5)),
            )
            .expect("mover spawns");
        // Spawn y is the con-0 bottom (C4Object.cpp:1462-1468): 6 - (1 + 0)
        // keeps the blocker center — and its solid mask — at (5,5).
        engine
            .spawn_object(
                SpawnConfig::new("Blocker")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 6)),
            )
            .expect("blocker spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position, Vector2::new(5, 5));
        assert_eq!(object.energy, 22513);

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(5));
        assert_eq!(engine.objects[idx].fixed_velocity.x, itofix(1));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    #[test]
    fn contact_callback_mutations_affect_later_movement_probes() {
        // The later vertical arm must use ContactRight's new
        // C4Shape::ContactDensity rather than the movement-entry value.
        {
            let mut landscape = vehicle_grid_landscape(20, 20);
            landscape.grid_write_byte(7, 5, 1);
            landscape.grid_write_byte(5, 7, 1);

            let mut definition = Definition::from_script(
                "L217",
                "Live movement density",
                r#"#strict 2
                    global func ContactRight()
                    {
                        SetContactDensity(101);
                        SetYDir(20);
                        return 0;
                    }
                "#,
            )
            .expect("density fixture compiles");
            definition.set_shape_vertices(vec![
                ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT),
                ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM),
            ]);
            definition.set_contact_density(50);
            definition.set_contact_function_calls(true);

            let mut engine = Engine::with_seed(217);
            engine.set_landscape(landscape);
            engine.set_physics(PhysicsSettings::new(0, 20, -20));
            engine
                .register_definition(definition)
                .expect("density definition registers");
            let object_id = engine
                .spawn_object(
                    SpawnConfig::new("L217")
                        .with_loaded(true)
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(5, 5))
                        .with_fixed_position(FixedVec2::from_ints(5, 5))
                        .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                        .with_mobile(true),
                )
                .expect("density mover spawns");

            engine
                .tick_without_snapshot()
                .expect("density movement succeeds");
            let object = engine.object_snapshot(object_id).expect("mover remains");
            assert_eq!(object.position, Vector2::new(5, 7));
            assert_eq!(object.contact_density, 101);
        }

        // Rotation follows translation in the same DoMovement. ChangeDef's
        // tighter Rotateable limit is therefore authoritative immediately.
        {
            let mut landscape = vehicle_grid_landscape(20, 20);
            landscape.grid_write_byte(7, 5, 1);

            let mut old = Definition::from_script(
                "LOLD",
                "Old rotation limit",
                r#"#strict 2
                    global func ContactRight()
                    {
                        SetXDir(0);
                        ChangeDef(LNEW);
                        SetRDir(100);
                        return 0;
                    }
                "#,
            )
            .expect("old rotation fixture compiles");
            old.set_shape_vertices(vec![ObjectVertex::new(1, 0).with_cnat(CNAT_RIGHT)]);
            old.set_contact_density(50);
            old.set_contact_function_calls(true);
            old.set_rotateable(100);

            let mut new = Definition::from_script("LNEW", "New rotation limit", "")
                .expect("new rotation fixture compiles");
            new.set_rotateable(20);

            let mut engine = Engine::with_seed(217);
            engine.set_landscape(landscape);
            engine.set_physics(PhysicsSettings::new(0, 20, -20));
            engine
                .register_definition(old)
                .expect("old rotation definition registers");
            engine
                .register_definition(new)
                .expect("new rotation definition registers");
            let object_id = engine
                .spawn_object(
                    SpawnConfig::new("LOLD")
                        .with_loaded(true)
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(5, 5))
                        .with_fixed_position(FixedVec2::from_ints(5, 5))
                        .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                        .with_mobile(true),
                )
                .expect("rotation mover spawns");

            engine
                .tick_without_snapshot()
                .expect("rotation movement succeeds");
            let object = engine.object_snapshot(object_id).expect("mover remains");
            assert_eq!(object.definition_id, "LNEW");
            assert_eq!(object.position, Vector2::new(6, 5));
            let index = engine.find_object_index(object_id).expect("mover index");
            assert_eq!(
                object.rotation,
                20,
                "rdir={:?} ocf={}",
                engine.objects[index].rotation_velocity,
                object.ocf
            );
        }
    }

    #[test]
    fn liquid_entry_splash_amount_uses_live_shape_area_like_cpp() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Instable=1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);

        for (
            label,
            definition_rect,
            construction,
            stretch_growth,
            rotateable,
            rotation,
            expected_rect,
            expected_amount,
        ) in [
            (
                "half-built stretch-growth object",
                DefinitionRect::new(-5, -5, 10, 10),
                FULL_CON / 2,
                true,
                0,
                0,
                DefinitionRect::new(-2, -2, 5, 5),
                2,
            ),
            (
                "rotated small object",
                DefinitionRect::new(-3, -3, 6, 6),
                FULL_CON,
                false,
                1,
                45,
                DefinitionRect::new(-6, -6, 12, 12),
                14,
            ),
            (
                "full-con unrotated object",
                DefinitionRect::new(-3, -3, 6, 6),
                FULL_CON,
                false,
                1,
                0,
                DefinitionRect::new(-3, -3, 6, 6),
                3,
            ),
        ] {
            let mut definition =
                Definition::from_script("L125", "Splash mover", "").expect("mover compiles");
            definition.set_shape_rect(Some(definition_rect));
            definition.set_stretch_growth(stretch_growth);
            definition.set_rotateable(rotateable);
            definition.set_float_line(1);
            definition.set_mass(20);

            let mut engine = Engine::with_seed(125);
            engine.set_materials(materials.clone());
            let mut bytes = vec![0_u8; 40 * 40];
            // The solid cap makes Splash's surface probe semi-solid, so the
            // PXS extraction arm is skipped and each amount unit consumes
            // exactly its two unconditional BubbleOut coordinate draws.
            bytes[19 * 40..20 * 40].fill(30);
            for y in 20..40 {
                bytes[y * 40..y * 40 + 40].fill(20);
            }
            let mut densities = vec![0_i32; 128];
            densities[20] = 25;
            densities[30] = 100;
            let mut names = vec![None; 128];
            names[20] = Some("Water".to_string());
            names[30] = Some("Earth".to_string());
            let grid = landscape::PixelGrid::new(
                40,
                40,
                bytes,
                densities,
                names,
                vec![None; 128],
            );
            let mut landscape = Landscape::new(40, vec![0; 40]).expect("landscape builds");
            landscape.set_world_height(40);
            landscape.set_pixel_grid(grid);
            engine.set_landscape(landscape);
            engine.set_physics(PhysicsSettings::new(0, 20, -20));
            engine
                .register_definition(definition)
                .expect("mover definition registers");

            let mover = engine
                .spawn_object(
                    SpawnConfig::new("L125")
                        .with_loaded(true)
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(20, 5))
                        .with_fixed_position(FixedVec2::from_ints(20, 5))
                        .with_construction(construction)
                        .with_rotation(rotation),
                )
                .expect("mover spawns");
            assert_eq!(
                engine.object_current_shape_rect(mover),
                Some(expected_rect),
                "{label}"
            );
            let mover_idx = engine.find_object_index(mover).expect("mover exists");
            engine.objects[mover_idx]
                .set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(18)));
            engine.refresh_object_ocf(mover_idx);
            assert_ne!(
                engine.objects[mover_idx].state.ocf & clonk_engine::ocf::HIT_SPEED2,
                0,
                "{label}"
            );
            let definition_id = engine.objects[mover_idx].definition_id.clone();
            let actions = engine
                .definition(&definition_id)
                .expect("mover definition exists")
                .action_library()
                .clone();
            let rng_before = engine.rng.count;
            let pxs_before = engine.pxs_system.count();

            assert!(
                engine
                    .exec_object_movement(mover_idx, &actions, &definition_id, &[])
                    .expect("movement succeeds")
                    .alive,
                "{label}"
            );

            let mover_idx = engine.find_object_index(mover).expect("mover survives");
            assert_eq!(
                engine.objects[mover_idx].state.position,
                Vector2::new(20, 23),
                "{label}"
            );
            assert!(engine.objects[mover_idx].state.in_liquid, "{label}");
            assert_eq!(
                engine.rng.count - rng_before,
                expected_amount * 2,
                "two synced BubbleOut draws per splash iteration: {label}"
            );
            assert_eq!(
                engine.pxs_system.count() - pxs_before,
                0,
                "the solid surface cap disables liquid extraction: {label}"
            );
        }
    }

    fn assert_contact_callback_preserves_liquid_entry_splash_like_cpp(
        script: &str,
        callback_present: bool,
    ) {
        // C4Object::Execute computes OCF once before command/action/movement
        // (src/C4Object.cpp:1058-1066). ContactCheck may Call a Contact*
        // function during movement (src/C4Movement.cpp:112-119, :166-182),
        // but a missing or no-op callback does not invoke SetOCF. The later
        // liquid-entry Splash therefore still reads the pre-collision
        // OCF_HitSpeed2 bit (src/C4Movement.cpp:449-456). Goldrush's WIPF
        // #564 freezes this sequence at frame 403.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Water]
            Name=Water
            Density=25
            Instable=1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);

        let mut mover_definition =
            Definition::from_script("Mover", "Mover", script).expect("script compiles");
        mover_definition.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 5).with_cnat(CNAT_BOTTOM)]);
        mover_definition.set_contact_density(50);
        mover_definition.set_contact_function_calls(true);
        mover_definition.set_float_line(1);
        mover_definition.set_mass(20);

        let mut engine = Engine::with_seed(67);
        engine.set_materials(materials);
        let mut bytes = vec![0u8; 40 * 40];
        for y in 20..26 {
            for x in 0..40 {
                bytes[y * 40 + x] = 20;
            }
        }
        for x in 0..40 {
            bytes[26 * 40 + x] = 30;
        }
        let mut densities = vec![0; 128];
        densities[20] = 25;
        densities[30] = 100;
        let mut names = vec![None; 128];
        names[20] = Some("Water".into());
        names[30] = Some("Earth".into());
        let grid = landscape::PixelGrid::new(
            40,
            40,
            bytes,
            densities,
            names,
            vec![None; 128],
        );
        let mut landscape = Landscape::new(40, vec![0; 40]).expect("landscape builds");
        landscape.set_world_height(40);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(simple_definition("FXU1"))
            .expect("bubble definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");
        assert_eq!(
            engine
                .definition("Mover")
                .expect("mover definition exists")
                .has_function("ContactBottom"),
            callback_present,
            "the fixture must exercise the requested callback path"
        );

        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 18))
                    .with_fixed_position(FixedVec2::from_ints(20, 18))
                    .with_loaded(true),
            )
            .expect("mover spawns");
        let idx = engine.find_object_index(mover_id).expect("mover exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(3)));
        engine.objects[idx].state.mobile = true;
        engine.refresh_object_ocf(idx);
        assert_ne!(
            engine.objects[idx].state.ocf & clonk_engine::ocf::HIT_SPEED2,
            0,
            "the pre-movement OCF carries HitSpeed2"
        );
        let definition_id = engine.objects[idx].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("mover definition exists")
            .action_library()
            .clone();
        let rng_before = engine.rng.count;

        assert!(
            engine
                .exec_object_movement(idx, &actions, &definition_id, &[])
                .expect("movement succeeds")
                .alive
        );

        let idx = engine.find_object_index(mover_id).expect("mover survives");
        assert_eq!(engine.objects[idx].state.position, Vector2::new(20, 20));
        assert_eq!(
            movement_hit_speed_flags(engine.objects[idx].fixed_velocity)
                & clonk_engine::ocf::HIT_SPEED2,
            0,
            "the collision must first reduce the live velocity below HitSpeed2"
        );
        assert!(engine.objects[idx].state.in_liquid);
        assert!(
            engine.rng.count - rng_before >= 20,
            "Splash amount 10 must consume at least its 10 pairs of BubbleOut draws"
        );
        assert!(
            engine
                .objects
                .iter()
                .any(|object| object.definition_id == "FXU1"),
            "the cached HitSpeed2 gate must create submerged FXU1 bubbles"
        );
    }

    #[test]
    fn missing_contact_callback_preserves_liquid_entry_splash_like_cpp() {
        assert_contact_callback_preserves_liquid_entry_splash_like_cpp("", false);
    }

    #[test]
    fn no_op_contact_callback_preserves_liquid_entry_splash_like_cpp() {
        // WIPF #565 at pinned Goldrush frame 1382 runs ContactLeft after
        // collision has slowed its live velocity, but C++ retains the
        // Execute-start OCF_HitSpeed2 through the later Splash gate
        // (C4Object.cpp:1082-1093; C4Movement.cpp:166-182,449-456).
        assert_contact_callback_preserves_liquid_entry_splash_like_cpp(
            "protected func ContactBottom() { return 0; }",
            true,
        );
    }

    #[test]
    fn corner_scale_probes_dispatch_contact_callbacks_like_cpp() {
        // C++ CornerScaleOkay performs a full C4Object::ContactCheck for every
        // probe (src/C4ObjectCom.cpp:167-179). ContactCheck synchronously calls
        // ContactLeft/Right/Top/Bottom in bit order and stops on a truthy return
        // (src/C4Movement.cpp:166-182). The pinned GoldRush differential at
        // frame 309 freezes the result: failed Fish probes run ContactTop,
        // leaving COMD_Down before the range-6 corner scale succeeds.
        let script = r#"
            local probeCount;

            protected func ContactTop()
            {
                probeCount = probeCount + 1;
                SetComDir(COMD_Down());
                return 1;
            }
        "#;
        let mut fish = Definition::from_script("FISH", "Fish", script).expect("script compiles");
        fish.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_TOP)]);
        fish.set_contact_density(50);
        fish.set_contact_function_calls(true);
        fish.configure_actions(
            Some("Swim".to_string()),
            HashMap::from([
                (
                    "Swim".to_string(),
                    ActionSpec::default().with_procedure("SWIM"),
                ),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
            ]),
        );

        let mut landscape = vehicle_grid_landscape(24, 24);
        landscape.set_world_height(24);
        for (x, y) in [(12, 8), (13, 7), (14, 6), (15, 5)] {
            landscape.grid_write_byte(x, y, 1);
        }

        let mut engine = Engine::with_seed(0);
        engine.set_landscape(landscape);
        engine.register_definition(fish).expect("fish registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("FISH")
                    .with_position(Vector2::new(10, 10))
                    .with_action(ActionState::new("Swim"))
                    .with_direction(Direction::Right)
                    .with_command_direction(CommandDirection::Right)
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_loaded(true),
            )
            .expect("fish spawns");
        let idx = engine.find_object_index(id).expect("fish exists");
        let definition_id = engine.objects[idx].definition_id.clone();
        assert_eq!(engine.objects[idx].state.position, Vector2::new(10, 10));
        assert_eq!(
            engine.objects[idx].state.vertices,
            vec![ObjectVertex::new(0, 0).with_cnat(CNAT_TOP)]
        );
        let landscape = engine.landscape.as_ref().expect("landscape set");
        assert_eq!(landscape.density_at(12, 8, &engine.materials), 100);
        assert_eq!(landscape.density_at(16, 4, &engine.materials), 0);

        assert!(
            engine
                .object_action_corner_scale(
                    idx,
                    &definition_id,
                    ActionProcedure::Swim,
                    &[],
                )
                .expect("corner scale action applies")
        );
        let object = &engine.objects[idx];
        assert_eq!(object.state.position, Vector2::new(16, 4));
        assert_eq!(object.state.action.name, "Walk");
        assert_eq!(object.state.direction, Direction::Right);
        assert_eq!(
            object.state.local_vars.get("probeCount"),
            Some(&Value::Int(4)),
            "ranges 2 through 5 each perform a full ContactCheck"
        );
        assert_eq!(
            object.state.command_direction,
            CommandDirection::Down,
            "failed corner probes must run Fish ContactTop before the free range-6 probe"
        );
    }

    #[test]
    fn engine_internal_jump_sites_use_hook_args_and_unstick_on_fallback(
    ) -> Result<(), EngineError> {
        fn jump_definition(id: &str, script: &str) -> Result<Definition, EngineError> {
            let mut definition = Definition::from_script(id, id, script)?;
            definition.set_c4_callback_convention(true);
            definition.configure_actions(
                Some("Idle".to_owned()),
                HashMap::from([
                    ("Idle".to_owned(), ActionSpec::default()),
                    (
                        "Walk".to_owned(),
                        ActionSpec::default().with_procedure("WALK"),
                    ),
                    (
                        "Scale".to_owned(),
                        ActionSpec::default().with_procedure("SCALE"),
                    ),
                    (
                        "Jump".to_owned(),
                        ActionSpec::default().with_procedure("FLIGHT"),
                    ),
                ]),
            );
            Ok(definition)
        }

        let hook_script = r#"#strict
local jump_calls, jump_xdir, jump_ydir, jump_by_com;
protected func OnActionJump(int xdir, int ydir, bool by_com)
{
    jump_calls++;
    jump_xdir = xdir;
    jump_ydir = ydir;
    jump_by_com = by_com;
    return true;
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.register_definition(jump_definition("HOOK", hook_script)?)?;
        engine.register_definition(jump_definition("FALL", "#strict")?)?;
        let hook_definition_id = "HOOK".to_owned();
        let fallback_definition_id = "FALL".to_owned();

        let hooked_wall = engine.spawn_object(
            SpawnConfig::new("HOOK")
                .with_action(ActionState::new("Walk"))
                .with_command_direction(CommandDirection::Right)
                .with_fixed_velocity(FixedVec2::new(itofix(6), itofix(-2))),
        )?;
        let hooked_wall_idx = engine
            .find_object_index(hooked_wall)
            .expect("hooked wall walker exists");
        engine.objects[hooked_wall_idx].state.t_attach = CNAT_BOTTOM | CNAT_LEFT;
        engine.objects[hooked_wall_idx].frame_t_attach = CNAT_BOTTOM | CNAT_LEFT;
        engine.exec_contact_action(
            hooked_wall_idx,
            CNAT_LEFT,
            &hook_definition_id,
            &[],
        )?;

        let hooked_wall_idx = engine
            .find_object_index(hooked_wall)
            .expect("hooked wall walker survives");
        let hooked_wall = &engine.objects[hooked_wall_idx];
        assert_eq!(hooked_wall.state.action.name, "Walk");
        assert_eq!(
            hooked_wall.fixed_velocity,
            FixedVec2::new(itofix(6), itofix(-2))
        );
        assert_eq!(hooked_wall.state.t_attach, CNAT_BOTTOM | CNAT_LEFT);
        assert_eq!(hooked_wall.frame_t_attach, CNAT_BOTTOM | CNAT_LEFT);
        assert_eq!(
            hooked_wall.state.local_vars.get("jump_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            hooked_wall.state.local_vars.get("jump_xdir"),
            Some(&Value::Int(300))
        );
        assert_eq!(
            hooked_wall.state.local_vars.get("jump_ydir"),
            Some(&Value::Int(-200))
        );
        assert_eq!(
            hooked_wall.state.local_vars.get("jump_by_com"),
            Some(&Value::Nil)
        );

        let hooked_no_attach = engine.spawn_object(
            SpawnConfig::new("HOOK")
                .with_action(ActionState::new("Walk"))
                .with_fixed_velocity(FixedVec2::new(itofix(-4), itofix(-5))),
        )?;
        let hooked_no_attach_idx = engine
            .find_object_index(hooked_no_attach)
            .expect("hooked no-attach walker exists");
        engine.objects[hooked_no_attach_idx].state.t_attach = CNAT_BOTTOM | CNAT_RIGHT;
        engine.objects[hooked_no_attach_idx].frame_t_attach = CNAT_BOTTOM | CNAT_RIGHT;
        let hook_actions = engine
            .definition("HOOK")
            .expect("hook definition exists")
            .action_library()
            .clone();
        engine.apply_no_attach_action(
            hooked_no_attach_idx,
            &hook_definition_id,
            &hook_actions,
            &[],
        )?;

        let hooked_no_attach_idx = engine
            .find_object_index(hooked_no_attach)
            .expect("hooked no-attach walker survives");
        let hooked_no_attach = &engine.objects[hooked_no_attach_idx];
        assert_eq!(hooked_no_attach.state.action.name, "Walk");
        assert_eq!(
            hooked_no_attach.fixed_velocity,
            FixedVec2::new(itofix(-4), itofix(-5))
        );
        assert_eq!(hooked_no_attach.state.t_attach, CNAT_BOTTOM | CNAT_RIGHT);
        assert_eq!(hooked_no_attach.frame_t_attach, CNAT_BOTTOM | CNAT_RIGHT);
        assert_eq!(
            hooked_no_attach.state.local_vars.get("jump_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            hooked_no_attach.state.local_vars.get("jump_xdir"),
            Some(&Value::Int(-400))
        );
        assert_eq!(
            hooked_no_attach.state.local_vars.get("jump_ydir"),
            Some(&Value::Int(-500))
        );
        assert_eq!(
            hooked_no_attach.state.local_vars.get("jump_by_com"),
            Some(&Value::Nil)
        );

        let hooked_scale = engine.spawn_object(
            SpawnConfig::new("HOOK")
                .with_action(ActionState::new("Scale"))
                .with_direction(Direction::Left)
                .with_command_direction(CommandDirection::Stop)
                .with_fixed_velocity(FixedVec2::new(itofix(7), itofix(-3))),
        )?;
        let hooked_scale_idx = engine
            .find_object_index(hooked_scale)
            .expect("hooked scaler exists");
        engine.apply_no_attach_action(
            hooked_scale_idx,
            &hook_definition_id,
            &hook_actions,
            &[],
        )?;
        let hooked_scale = &engine.objects[hooked_scale_idx];
        assert_eq!(hooked_scale.state.action.name, "Scale");
        assert_eq!(
            hooked_scale.fixed_velocity,
            FixedVec2::new(itofix(7), itofix(-3))
        );
        assert_eq!(
            hooked_scale.state.local_vars.get("jump_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            hooked_scale.state.local_vars.get("jump_xdir"),
            Some(&Value::Int(100))
        );
        assert_eq!(
            hooked_scale.state.local_vars.get("jump_ydir"),
            Some(&Value::Nil)
        );
        assert_eq!(
            hooked_scale.state.local_vars.get("jump_by_com"),
            Some(&Value::Nil)
        );

        let fallback_wall = engine.spawn_object(
            SpawnConfig::new("FALL")
                .with_action(ActionState::new("Walk"))
                .with_command_direction(CommandDirection::Right)
                .with_fixed_velocity(FixedVec2::new(itofix(6), itofix(-2))),
        )?;
        let fallback_wall_idx = engine
            .find_object_index(fallback_wall)
            .expect("fallback wall walker exists");
        engine.objects[fallback_wall_idx].state.t_attach = CNAT_BOTTOM | CNAT_LEFT;
        engine.objects[fallback_wall_idx].frame_t_attach = CNAT_BOTTOM | CNAT_LEFT;
        engine.exec_contact_action(
            fallback_wall_idx,
            CNAT_LEFT,
            &fallback_definition_id,
            &[],
        )?;
        let fallback_wall = &engine.objects[fallback_wall_idx];
        assert_eq!(fallback_wall.state.action.name, "Jump");
        assert_eq!(
            fallback_wall.fixed_velocity,
            FixedVec2::new(itofix(3), itofix(-2))
        );
        assert!(fallback_wall.state.mobile);
        assert_eq!(fallback_wall.state.t_attach, CNAT_LEFT);
        assert_eq!(fallback_wall.frame_t_attach, CNAT_LEFT);

        Ok(())
    }

    #[test]
    fn contact_action_rereads_live_ocf_after_ceiling_tumble_callback() {
        // ContactAction snapshots iProcedure/fDisabled/pPhysical, but it reads
        // the object's OCF again for each directional arm. A ceiling tumble's
        // StartCall can therefore clear HitSpeed3 before the later left-wall
        // arm is evaluated (oracle-src-pinned src/C4Object.cpp:4324-4330,
        // 4383-4414,4424-4439; src/C4ObjectCom.cpp:74-79).
        let script = r#"#strict 3
local tumble_starts;

protected func OnTumbleStart()
{
    ++tumble_starts;
    SetXDir(0, nil, 100);
    SetYDir(0, nil, 100);
    SetCategory(C4D_Object);
    SetAction("Flight");
    return(0);
}
"#;
        let mut definition =
            Definition::from_script("CAOC", "ContactAction live OCF", script)
                .expect("contact action probe compiles");
        definition.set_c4_callback_convention(true);
        definition.set_category(CATEGORY_OBJECT);
        definition.configure_actions(
            Some("Idle".to_owned()),
            HashMap::from([
                ("Idle".to_owned(), ActionSpec::default()),
                (
                    "Flight".to_owned(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                (
                    "Tumble".to_owned(),
                    ActionSpec::default().with_start_call("OnTumbleStart"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("contact action probe registers");
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("CAOC")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Flight"))
                    .with_fixed_velocity(FixedVec2::new(itofix(6), C4Fixed::ZERO))
                    .with_loaded(true),
            )
            .expect("contact action probe spawns");
        let index = engine
            .find_object_index(object_id)
            .expect("contact action probe exists");
        engine.refresh_object_ocf(index);
        assert_ne!(
            engine.objects[index].state.ocf & ocf::HIT_SPEED3,
            0,
            "the ceiling arm must enter its high-speed tumble"
        );
        let definition_id = engine.objects[index].definition_id.clone();

        engine
            .exec_contact_action(index, CNAT_TOP | CNAT_LEFT, &definition_id, &[])
            .expect("contact action executes");

        let object = &engine.objects[engine
            .find_object_index(object_id)
            .expect("contact action probe survives")];
        assert_eq!(
            object.state.local_vars.get("tumble_starts"),
            Some(&Value::Int(1)),
            "the callback-cleared live OCF prevents a second wall tumble"
        );
        assert_eq!(
            object.state.ocf & ocf::HIT_SPEED3,
            0,
            "SetCategory's callback-time SetOCF observes the zeroed dirs"
        );
    }

    #[test]
    fn stopped_scaler_no_attach_jumps_away_from_each_wall() {
        let mut scaler = simple_definition("STSC");
        scaler.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Scale".to_string(),
                    ActionSpec::default().with_procedure("SCALE"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(0);
        engine.register_definition(scaler).expect("scaler registers");
        let actions = engine
            .definition("STSC")
            .expect("scaler definition exists")
            .action_library()
            .clone();

        for (direction, expected_x) in [(Direction::Left, 1), (Direction::Right, -1)] {
            let id = engine
                .spawn_object(
                    SpawnConfig::new("STSC")
                        .with_action(ActionState::new("Scale"))
                        .with_direction(direction)
                        .with_command_direction(CommandDirection::Stop),
                )
                .expect("scaler spawns");
            let idx = engine.find_object_index(id).expect("scaler exists");
            engine.objects[idx]
                .set_fixed_velocity(FixedVec2::new(itofix(7), itofix(-3)));
            let definition_id = engine.objects[idx].definition_id.clone();

            engine
                .apply_no_attach_action(idx, &definition_id, &actions, &[])
                .expect("no-attach transition succeeds");

            let object = &engine.objects[idx];
            assert_eq!(object.state.action.name, "Jump");
            assert_eq!(
                object.fixed_velocity,
                FixedVec2::new(itofix(expected_x), C4Fixed::ZERO),
                "a stopped {direction:?} scaler jumps away from its wall"
            );
            assert_eq!(object.state.velocity, Vector2::new(expected_x, 0));
        }
    }

    #[test]
    fn upward_scaler_corner_scales_when_attachment_is_lost() {
        // C++ NoAttachAction tries ObjectActionCornerScale before its Jump
        // fallback whenever DFA_SCALE is moving upward
        // (src/C4Object.cpp:4282-4289). A successful corner probe changes to
        // Walk and moves over the rim (src/C4ObjectCom.cpp:191-217).
        let mut scaler = Definition::from_script("SCLR", "Scaler", "#strict\n")
            .expect("script compiles");
        scaler.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_LEFT)]);
        scaler.set_contact_density(50);
        scaler.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Scale".to_string(),
                    ActionSpec::default().with_procedure("SCALE"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
            ]),
        );

        let mut landscape = vehicle_grid_landscape(24, 24);
        landscape.set_world_height(24);
        let mut engine = Engine::with_seed(0);
        engine.set_landscape(landscape);
        engine.register_definition(scaler).expect("scaler registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("SCLR")
                    .with_position(Vector2::new(10, 10))
                    .with_action(ActionState::new("Scale"))
                    .with_direction(Direction::Left)
                    .with_command_direction(CommandDirection::Up)
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_loaded(true),
            )
            .expect("scaler spawns");
        let idx = engine.find_object_index(id).expect("scaler exists");
        let actions = engine
            .definition("SCLR")
            .expect("scaler definition exists")
            .action_library()
            .clone();
        let definition_id = engine.objects[idx].definition_id.clone();
        assert_eq!(
            actions.procedure_for_action("Scale"),
            ActionProcedure::Scale
        );
        engine
            .apply_no_attach_action(idx, &definition_id, &actions, &[])
            .expect("no-attach transition succeeds");

        let object = &engine.objects[idx];
        assert_eq!(object.state.action.name, "Walk");
        assert_eq!(object.state.position, Vector2::new(3, 3));
        assert_eq!(object.state.velocity, Vector2::ZERO);
    }

    type HitCallLog =
        std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<clonk_script::Value>)>>>;

    fn hit_gate_probe_definition(id: &str) -> (Definition, HitCallLog) {
        let calls: HitCallLog = Default::default();
        let mut hooks = DebuggerHooks::new();
        {
            let calls = std::sync::Arc::clone(&calls);
            hooks.set_on_call(move |name, args| {
                if matches!(name, "Hit" | "Hit2" | "Hit3") {
                    calls
                        .lock()
                        .unwrap()
                        .push((name.to_string(), args.to_vec()));
                }
            });
        }
        let mut definition = Definition::from_script(
            id,
            id,
            r#"
            #strict 2
            protected func Hit(int x, int y) { return 1; }
            protected func Hit2(int x, int y) { return 1; }
            protected func Hit3(int x, int y) { return 1; }
            "#,
        )
        .expect("hit probe script compiles");
        definition.set_debugger_hooks(hooks);
        (definition, calls)
    }

    #[test]
    fn lethal_contact_assigns_death_before_later_cnat_and_vertical_redirect() {
        // C4Object::ContactCheck dispatches Left/Right/Top/Bottom synchronously
        // (oracle-src-pinned src/C4Movement.cpp:166-182). ContactLeft's
        // DoEnergy reaches zero and therefore completes AssignDeath before
        // ContactRight runs (src/C4Object.cpp:1164-1205,1372-1393). Only after
        // the complete ContactCheck does vertical movement inspect !Alive and
        // redirect ydir into rdir (src/C4Movement.cpp:284-321).
        let script = r#"#strict 3
local contact_order, death_rdir, right_alive, right_action, right_rdir;

protected func ContactLeft()
{
    contact_order = 1;
    DoEnergy(-1);
    return(0);
}

protected func Death()
{
    contact_order = contact_order * 10 + 2;
    death_rdir = GetRDir(nil, 100);
    return(0);
}

protected func ContactRight()
{
    contact_order = contact_order * 10 + 3;
    right_alive = GetAlive();
    right_action = GetAction();
    right_rdir = GetRDir(nil, 100);
    return(0);
}

protected func ContactBottom()
{
    contact_order = contact_order * 10 + 4;
    return(0);
}
"#;
        let library = MaterialLibrary::parse(
            r#"
[Material Earth]
Name=Earth
Density=100
"#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut definition =
            Definition::from_script("DCNT", "Death contact ordering", script)
                .expect("contact definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_contact_function_calls(true);
        definition.set_contact_density(50);
        definition.set_rotateable(360);
        definition.set_shape_vertices(vec![
            ObjectVertex::new(1, 1).with_cnat(CNAT_LEFT | CNAT_RIGHT | CNAT_BOTTOM),
        ]);
        definition.set_physical(PhysicalInfo {
            energy: 10_000,
            ..PhysicalInfo::default()
        });
        definition.configure_actions(
            Some("Flight".to_owned()),
            HashMap::from([
                (
                    "Flight".to_owned(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                ("Dead".to_owned(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 7, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("contact definition registers");
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("DCNT")
                    .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                    .with_position(Vector2::new(5, 5))
                    .with_energy(1_000)
                    .with_alive(true)
                    .with_action(ActionState::new("Flight"))
                    .with_mobile(true),
            )
            .expect("contact object spawns");
        let index = engine
            .find_object_index(object_id)
            .expect("contact object exists");
        engine.objects[index]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(1)));
        engine.refresh_object_ocf(index);
        let definition_id = engine.objects[index].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("contact definition remains registered")
            .action_library()
            .clone();

        engine
            .exec_object_movement(index, &actions, &definition_id, &[])
            .expect("lethal contact movement executes");

        let object = engine
            .object_snapshot(object_id)
            .expect("AssignDeath retains the corpse");
        assert_eq!(object.energy, 0);
        assert!(!object.alive);
        assert_eq!(object.action.name, "Dead");
        assert_eq!(
            object.local_vars.get("contact_order"),
            Some(&Value::Int(1234)),
            "Death completes between the first and second Contact* callbacks"
        );
        assert_eq!(
            object.local_vars.get("right_alive"),
            Some(&Value::Bool(false)),
            "the later ContactRight observes AssignDeath's Alive=false"
        );
        assert_eq!(
            object.local_vars.get("right_action"),
            Some(&Value::String("Dead".to_owned().into())),
            "the later ContactRight observes AssignDeath's Dead action"
        );
        assert_eq!(
            object.local_vars.get("death_rdir"),
            Some(&Value::Int(0)),
            "Death runs before vertical contact redirects ydir into rdir"
        );
        assert_eq!(
            object.local_vars.get("right_rdir"),
            Some(&Value::Int(0)),
            "all Contact* callbacks finish before vertical redirection"
        );
        assert_eq!(
            object.rotation_velocity,
            Some(fixed100(-50)),
            "the post-callback !Alive branch redirects 0.5 ydir into -0.5 rdir"
        );
    }

    #[test]
    fn gravity_crossing_hit2_threshold_uses_cached_pre_action_ocf() {
        // UpdateOCF sees ydir=1.9 (HitSpeed1 only), then DFA_FLIGHT adds
        // GravAccel=0.2 before DoMovement. The contact therefore passes
        // ydir=2.1 to Hit, but the cached gate must not also call Hit2
        // (C4Object.cpp:1083-1093; C4Movement.cpp:250-252,477-483).
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let (mut definition, calls) = hit_gate_probe_definition("GravityHitProbe");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -2, 2, 4)));
        definition.set_shape_vertices(vec![
            ObjectVertex::new(-1, 1).with_cnat(CNAT_LEFT | CNAT_BOTTOM),
            ObjectVertex::new(1, 1).with_cnat(CNAT_RIGHT | CNAT_BOTTOM),
        ]);
        definition.set_contact_density(50);
        definition.configure_actions(
            Some("Flight".to_string()),
            HashMap::from([(
                "Flight".to_string(),
                ActionSpec::default().with_procedure("FLIGHT"),
            )]),
        );

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 7, Some(earth)));
        engine.set_physics(PhysicsSettings::new(100, 20, -20));
        engine
            .register_definition(definition)
            .expect("probe definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("GravityHitProbe")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 5))
                    .with_action(ActionState::new("Flight")),
            )
            .expect("probe spawns");
        let idx = engine.find_object_index(id).expect("probe exists");
        engine.objects[idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, fixed100(190)));
        engine.objects[idx].state.mobile = true;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.objects[idx].state.ocf & ocf::HIT_SPEED1, 0);
        assert_eq!(engine.objects[idx].state.ocf & ocf::HIT_SPEED2, 0);

        let snapshot = engine.tick().expect("gravity/contact tick succeeds");

        assert_eq!(snapshot.object(id).expect("probe survives").position.y, 5);
        // The script parameter frame canonicalizes a raw integer zero to Nil;
        // both expose the same C4 integer payload through `_getInt()`.
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [(
                "Hit".to_string(),
                vec![clonk_script::Value::Nil, clonk_script::Value::Int(210)],
            )],
            "the cached tier and the post-gravity argument use different clocks"
        );
    }

    #[test]
    fn rejected_rotation_preserves_contact_callback_position_like_cpp() {
        // A rejected rotation saves and restores only Shape and the integer
        // rotation, then calls UpdatePos. It does not restore x/y changed by
        // ContactCheck's synchronous Contact* callback
        // (oracle-src-pinned src/C4Movement.cpp:372-436, especially :394-420).
        let script = r#"#strict 3
local contact_calls;

protected func ContactRight()
{
    contact_calls = 1;
    SetPosition(8, 3);
    return(0);
}
"#;
        let library = MaterialLibrary::parse(
            r#"
[Material Earth]
Name=Earth
Density=100
"#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut definition =
            Definition::from_script("RPOS", "Rotation callback position", script)
                .expect("rotation probe compiles");
        definition.set_c4_callback_convention(true);
        definition.set_contact_function_calls(true);
        definition.set_contact_density(50);
        definition.set_rotateable(360);
        definition
            .set_shape_vertices(vec![ObjectVertex::new(2, 0).with_cnat(CNAT_RIGHT)]);
        definition.configure_actions(
            Some("Idle".to_owned()),
            HashMap::from([("Idle".to_owned(), ActionSpec::default())]),
        );

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        let mut surface = vec![20; 12];
        surface[6] = 0;
        engine.set_landscape(
            Landscape::new_with_material(12, surface, Some(earth))
                .expect("landscape constructs"),
        );
        engine
            .register_definition(definition)
            .expect("rotation probe registers");
        assert!(
            engine
                .definition("RPOS")
                .expect("rotation probe definition exists")
                .has_function("ContactRight"),
            "the fixture exposes its contact callback"
        );
        assert!(
            engine
                .definition("RPOS")
                .expect("rotation probe definition exists")
                .contact_function_calls(),
            "the fixture enables Contact* dispatch"
        );
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("RPOS")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 10))
                    .with_fixed_position(FixedVec2::from_ints(4, 10))
                    .with_action(ActionState::new("Idle"))
                    .with_mobile(true),
            )
            .expect("rotation probe spawns");
        let index = engine
            .find_object_index(object_id)
            .expect("rotation probe exists");
        engine.objects[index].rotation_velocity = itofix(1);
        engine.refresh_object_ocf(index);
        let definition_id = engine.objects[index].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("rotation probe definition remains registered")
            .action_library()
            .clone();

        engine
            .exec_object_movement(index, &actions, &definition_id, &[])
            .expect("rotation contact movement executes");

        let index = engine
            .find_object_index(object_id)
            .expect("rotation probe survives");
        let object = &engine.objects[index];
        assert_eq!(
            object.state.local_vars.get("contact_calls"),
            Some(&Value::Int(1)),
            "the attempted rotation reaches ContactRight exactly once: \
             rotation={} rdir={:?} ocf={} vertices={:?} position={:?}",
            object.state.rotation,
            object.rotation_velocity,
            object.state.ocf,
            object.state.vertices,
            object.state.position
        );
        assert_eq!(
            object.state.position,
            Vector2::new(8, 3),
            "the rejected rotation must preserve ContactRight's SetPosition"
        );
        assert_eq!(object.fixed_position, FixedVec2::from_ints(8, 3));
        assert_eq!(object.state.rotation, 0);
        assert_eq!(object.fixed_rotation, C4Fixed::ZERO);
        assert_eq!(
            object.state.vertices,
            vec![ObjectVertex::new(2, 0).with_cnat(CNAT_RIGHT)],
            "the rejected trial still restores the pre-rotation Shape"
        );
    }

    #[test]
    fn accepted_attached_rotation_clears_attach_material_in_final_update_face_like_cpp() {
        // Every accepted degree first runs UpdateShape and then Shape.Attach,
        // leaving Shape.AttachMat set to the attached material. fTurned makes
        // DoMovement finish with UpdateFace(true), whose UpdateShape copies
        // the definition's MNone AttachMat while retaining iAttachX/Y/Vtx
        // (oracle-src-pinned src/C4Movement.cpp:372-436,485-489;
        // src/C4Object.cpp:322-344,357-380; src/C4Shape.cpp:421-441).
        let library = MaterialLibrary::parse(
            r#"
[Material Earth]
Name=Earth
Density=100
"#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut definition = simple_definition("RATT");
        definition.set_rotateable(360);
        definition.set_contact_density(50);
        definition
            .set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 10, Some(earth)));
        engine
            .register_definition(definition)
            .expect("rotation probe registers");
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("RATT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 8))
                    .with_fixed_position(FixedVec2::from_ints(10, 8))
                    .with_mobile(true),
            )
            .expect("rotation probe spawns");
        let index = engine
            .find_object_index(object_id)
            .expect("rotation probe exists");
        engine.objects[index].frame_t_attach = CNAT_BOTTOM;
        engine.objects[index].state.t_attach = CNAT_BOTTOM;
        engine.objects[index].rotation_velocity = itofix(1);
        engine.refresh_object_ocf(index);
        let definition_id = engine.objects[index].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("rotation probe definition remains registered")
            .action_library()
            .clone();

        engine
            .exec_object_movement(index, &actions, &definition_id, &[])
            .expect("attached rotation executes");

        let object = &engine.objects[engine
            .find_object_index(object_id)
            .expect("rotation probe survives")];
        assert_eq!(object.state.rotation, 5, "the rotation must be accepted");
        assert_eq!(
            (object.state.shape_attach.x, object.state.shape_attach.y),
            (10, 10),
            "the successful Shape.Attach coordinates remain cached"
        );
        assert!(
            !object.state.shape_attach.mat_valid,
            "the trailing UpdateFace(true) resets AttachMat to MNone"
        );
    }

    #[test]
    fn accepted_rotation_rebuilds_shape_after_hit_like_cpp() {
        // Hit runs before fTurned's final UpdateFace(true). Consequently a
        // SetShape made by Hit is visible inside that callback but is then
        // discarded when UpdateFace rebuilds Shape from the live definition
        // (oracle-src-pinned src/C4Movement.cpp:372-436,472-490;
        // src/C4Object.cpp:322-344,357-376).
        let script = r#"#strict 3
local hit_calls, hit_rotation;

protected func Hit()
{
    ++hit_calls;
    hit_rotation = GetR();
    SetShape(-7, -8, 14, 16);
    return(0);
}
"#;
        let library = MaterialLibrary::parse(
            r#"
[Material Earth]
Name=Earth
Density=100
"#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut definition =
            Definition::from_script("RHIT", "Rotation Hit ordering", script)
                .expect("rotation Hit probe compiles");
        definition.set_c4_callback_convention(true);
        definition.set_contact_density(50);
        definition.set_rotateable(360);
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        definition
            .set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        definition.configure_actions(
            Some("Idle".to_owned()),
            HashMap::from([("Idle".to_owned(), ActionSpec::default())]),
        );

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        let mut surface = vec![20; 12];
        surface[5] = 0;
        engine.set_landscape(
            Landscape::new_with_material(12, surface, Some(earth))
                .expect("landscape constructs"),
        );
        engine
            .register_definition(definition)
            .expect("rotation Hit probe registers");
        let object_id = engine
            .spawn_object(
                SpawnConfig::new("RHIT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 5))
                    .with_fixed_position(FixedVec2::from_ints(4, 5))
                    .with_action(ActionState::new("Idle"))
                    .with_mobile(true),
            )
            .expect("rotation Hit probe spawns");
        let index = engine
            .find_object_index(object_id)
            .expect("rotation Hit probe exists");
        engine.objects[index]
            .set_fixed_velocity(FixedVec2::new(fixed100(190), C4Fixed::ZERO));
        engine.objects[index].rotation_velocity = itofix(1);
        engine.refresh_object_ocf(index);
        assert_ne!(
            engine.objects[index].state.ocf & ocf::HIT_SPEED1,
            0,
            "the entry OCF must gate Hit"
        );
        let definition_id = engine.objects[index].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("rotation Hit definition remains registered")
            .action_library()
            .clone();

        engine
            .exec_object_movement(index, &actions, &definition_id, &[])
            .expect("contacting rotation executes");

        let object = &engine.objects[engine
            .find_object_index(object_id)
            .expect("rotation Hit probe survives")];
        assert_eq!(object.state.position, Vector2::new(4, 5));
        assert_eq!(object.state.rotation, 5, "the free rotation is accepted");
        assert_eq!(
            object.state.local_vars.get("hit_calls"),
            Some(&Value::Int(1)),
            "the horizontal collision dispatches Hit once"
        );
        assert_eq!(
            object.state.local_vars.get("hit_rotation"),
            Some(&Value::Int(5)),
            "Hit observes the accepted rotation before the final UpdateFace"
        );
        assert_eq!(
            object.state.shape_override, None,
            "the final UpdateFace(true) discards Hit's SetShape override"
        );
        assert_ne!(
            object.current_shape_rect(),
            Some(DefinitionRect::new(-7, -8, 14, 16)),
            "the final shape no longer exposes Hit's SetShape geometry"
        );
    }

    #[test]
    fn action_zeroed_velocity_keeps_cached_hit2_gate_on_rotation_contact() {
        // UpdateOCF sees ydir=2.1 (HitSpeed1+2), DFA_KNEEL zeroes ydir,
        // and the copied Wheel geometry still registers a contact while
        // rotating. Hit and Hit2 must both run with movement-entry args 0,0.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let (mut definition, calls) = hit_gate_probe_definition("KneelHitProbe");
        definition.set_rotateable(360);
        definition.set_shape_vertices(vec![ObjectVertex::new(2, 0).with_cnat(CNAT_RIGHT)]);
        definition.set_contact_density(50);
        definition.configure_actions(
            Some("Kneel".to_string()),
            HashMap::from([(
                "Kneel".to_string(),
                ActionSpec::default().with_procedure("KNEEL"),
            )]),
        );

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        let mut surface = vec![20; 12];
        surface[6] = 0;
        let mut landscape =
            Landscape::new_with_material(12, surface, Some(earth)).expect("landscape constructs");
        landscape.fill_solid_material(Some(earth));
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("probe definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("KneelHitProbe")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 10))
                    .with_action(ActionState::new("Kneel")),
            )
            .expect("probe spawns");
        let idx = engine.find_object_index(id).expect("probe exists");
        engine.objects[idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, fixed100(210)));
        engine.objects[idx].rotation_velocity = itofix(1);
        engine.objects[idx].state.mobile = true;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.objects[idx].state.ocf & ocf::HIT_SPEED1, 0);
        assert_ne!(engine.objects[idx].state.ocf & ocf::HIT_SPEED2, 0);
        assert_eq!(engine.objects[idx].state.ocf & ocf::HIT_SPEED3, 0);

        let snapshot = engine.tick().expect("kneel/contact tick succeeds");

        assert_eq!(snapshot.object(id).expect("probe survives").rotation, 0);
        // Zero-valued C4 parameters are represented as Nil at this hook.
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                (
                    "Hit".to_string(),
                    vec![clonk_script::Value::Nil, clonk_script::Value::Nil],
                ),
                (
                    "Hit2".to_string(),
                    vec![clonk_script::Value::Nil, clonk_script::Value::Nil],
                ),
            ],
            "cached HitSpeed2 survives the action's velocity zeroing"
        );
    }

    #[test]
    fn hit_callbacks_run_after_contact_with_old_velocity_args_like_cpp() {
        // Mirrors src/C4Movement.cpp:247-252,468-478: movement stores oldxdir,
        // oldydir, and old_ocf before stepping; after contact and NoAttachAction,
        // it calls Hit/Hit2/Hit3 in that order based on the old OCF hit-speed
        // bits, passing fixtoi(oldxdir, 100), fixtoi(oldydir, 100). The hit-speed
        // thresholds are src/C4Movement.cpp:35-38; the flags are set from
        // C4Object::GetSpeed() = abs(xdir)+abs(ydir) at src/C4Object.cpp:588-592.
        //
        // Hand-derived golden for seed 63: Engine startup does Randomize3(), i.e.
        // 500 calls to Random(3). No contact callback consumes RNG here, so the
        // following Step random argument is Random(i32::MAX) = 36328. With
        // oldxdir = itofix(2), oldydir = 0, C++ sets HitSpeed1 and HitSpeed2 but
        // not HitSpeed3. The callback arguments are (200, 0), so Hit subtracts
        // 210 energy and Hit2 subtracts 220; Step encodes the total callback
        // delta plus RNG as 430 + 36328 = 36758.
        let script = r#"#strict 3
            global func Hit(x, y)
            {
                DoEnergy(0 - (10 + x + y), nil, true);
                return nil;
            }

            global func Hit2(x, y)
            {
                DoEnergy(0 - (20 + x + y), nil, true);
                return nil;
            }

            global func Hit3(x, y)
            {
                DoEnergy(0 - (40 + x + y), nil, true);
                return nil;
            }

            global func Step(state, frame, random)
            {
                return { energy = 1000000 - state.energy + random };
            }
        "#;

        let mut blocker_definition =
            Definition::from_script("Blocker", "Blocker", "").expect("script compiles");
        blocker_definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        blocker_definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut mover_definition =
            Definition::from_script("Mover", "Mover", script).expect("script compiles");
        mover_definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_RIGHT)]);
        mover_definition.set_contact_density(50);
        mover_definition.set_physical(PhysicalInfo {
            energy: 1_000_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(63);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
            .register_definition(blocker_definition)
            .expect("blocker definition registers");
        engine
            .register_definition(mover_definition)
            .expect("mover definition registers");

        let mover_id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 5))
                    .with_energy(1000000),
            )
            .expect("mover spawns");
        // Spawn y is the con-0 bottom (C4Object.cpp:1462-1468): 6 - (1 + 0)
        // keeps the blocker center — and its solid mask — at (5,5).
        engine
            .spawn_object(
                SpawnConfig::new("Blocker")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 6)),
            )
            .expect("blocker spawns");
        let idx = engine.find_object_index(mover_id).expect("object exists");
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(itofix(2), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(mover_id).expect("object present");
        assert_eq!(object.position, Vector2::new(6, 5));
        assert_eq!(object.energy, 36328);

        let idx = engine.find_object_index(mover_id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_position.x, itofix(6));
        assert_eq!(engine.objects[idx].fixed_velocity.x, itofix(2));
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    #[test]
    fn construction_jolt_updates_vertices_and_preserves_bottom_like_cpp() -> Result<(), EngineError>
    {
        // Mirrors src/C4Object.cpp:1401-1428: DoCon stores the old shape bottom,
        // changes Con, then calls UpdateFace(true) -> UpdateShape(true).
        // UpdateShape copies definition vertices at src/C4Object.cpp:320-333 and
        // non-stretch construction growth calls C4Shape::Jolt, whose vertex path
        // scales only VtxY at src/C4Shape.cpp:121-127. Finally DoCon preserves
        // the old bottom edge for straight objects at src/C4Object.cpp:1462-1468.
        //
        // Hand-derived golden: the spawn y 8 is the con-0 bottom, so the
        // full-con center is 8 - (4 + 0) = 4 (C4Object.cpp:1462-1468) and the
        // bottom stays 8. Changing Con from FullCon to FullCon/2 jolts Hgt
        // 4->2 and VtxY 4->2, then bottom preservation moves y to 8 - 2 - 0 = 6.
        let mut definition = simple_definition("Structure");
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 4)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 4).with_cnat(CNAT_BOTTOM)]);

        let mut engine = Engine::with_seed(65);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("Structure")
                .with_position(Vector2::new(3, 8))
                .with_construction(FULL_CON),
        )?;

        engine.apply_object_update(id, ObjectUpdate::new().with_construction(FULL_CON / 2))?;

        let object = engine.object_snapshot(id).expect("object present");
        assert_eq!(object.construction, FULL_CON / 2);
        assert_eq!(object.position, Vector2::new(3, 6));
        assert_eq!(object.vertices[0].y, 2);
        assert_eq!(object.vertices[0].cnat, CNAT_BOTTOM);
        Ok(())
    }

    #[test]
    fn script_docon_bottom_adjust_does_not_resync_fixed_position() -> Result<(), EngineError> {
        // GoldRush frame 3327: DHRS's Decay EndCall runs Decaying -> DoCon(-4)
        // (content/Western.c4d/Animals.c4d/Horse.c4d/Dead.c4d/ActMap.txt and
        // Script.c). C4Object::DoCon stretches the 25px shape to 24px and
        // bottom-adjusts integer y through UpdatePos, which does not write
        // fix_y (src/C4Object.cpp:1414-1515,346-354). Thus y moves 100->101
        // while the already-snapped fixed position remains exactly 100.
        let script = r#"#strict
func Decaying() {
    DoCon(-4);
    return(1);
}
"#;
        let mut definition = Definition::from_script("DHRS", "Dead Horse", script)?;
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 25)));
        definition.set_stretch_growth(true);
        definition.set_components(vec![
            DefinitionComponent {
                id: "SKIN".to_string(),
                count: 2,
            },
            DefinitionComponent {
                id: "BBON".to_string(),
                count: 2,
            },
            DefinitionComponent {
                id: "RMET".to_string(),
                count: 3,
            },
        ]);

        let mut engine = Engine::with_seed(73);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("DHRS")
                .with_position(Vector2::new(20, 125))
                .with_construction(FULL_CON),
        )?;
        engine.apply_object_update(id, ObjectUpdate::new().with_position(Vector2::new(20, 100)))?;
        let idx = engine.find_object_index(id).expect("dead horse exists");

        engine.call_object_function(idx, "Decaying", Vec::new())?;

        let object = engine.object_snapshot(id).expect("dead horse survives");
        assert_eq!(object.construction, 96_000);
        assert_eq!(object.position.y, 101, "DoCon keeps the old shape bottom");
        assert_eq!(object.components.get("SKIN"), Some(&1));
        assert_eq!(object.components.get("BBON"), Some(&1));
        assert_eq!(object.components.get("RMET"), Some(&2));
        let idx = engine.find_object_index(id).expect("dead horse survives");
        assert_eq!(
            engine.objects[idx].fixed_position.y,
            itofix(100),
            "UpdatePos leaves fix_y stale"
        );
        Ok(())
    }

    #[test]
    fn script_set_action_after_docon_resynchronizes_fixed_position() -> Result<(), EngineError> {
        // Operation order matters inside one script call: DoCon first shifts
        // only integer y and makes the object incomplete. The following valid
        // C4Object::SetAction call succeeds but is coerced to ActIdle, then
        // snaps fix_x/fix_y to the adjusted position (src/C4Object.cpp:
        // 1414-1515, 4111-4144). The staged Rust fold must preserve both the
        // incomplete-activity coercion and the later fixed-position resync.
        let script = r#"#strict
func DecayThenExist() {
    DoCon(-4);
    return(SetAction("Exist"));
}
"#;
        let mut definition = Definition::from_script("DHRS", "Dead Horse", script)?;
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 25)));
        definition.set_stretch_growth(true);
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Exist".to_string(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(73);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("DHRS")
                .with_position(Vector2::new(20, 125))
                .with_construction(FULL_CON),
        )?;
        engine.apply_object_update(id, ObjectUpdate::new().with_position(Vector2::new(20, 100)))?;
        let idx = engine.find_object_index(id).expect("dead horse exists");

        assert_eq!(
            engine.call_object_function(idx, "DecayThenExist", Vec::new())?,
            Value::Bool(true),
            "the requested slot is valid even though SetAction coerces it to ActIdle"
        );

        let object = engine.object_snapshot(id).expect("dead horse survives");
        assert_eq!(object.construction, 96_000);
        assert_eq!(object.position.y, 101);
        assert_eq!(object.action.name, "Idle");
        let idx = engine.find_object_index(id).expect("dead horse survives");
        assert_eq!(engine.objects[idx].fixed_position.y, itofix(101));
        Ok(())
    }

    #[test]
    fn script_sequential_docon_and_position_resync_match_cpp() -> Result<(), EngineError> {
        // Every call observes the preceding live shape/position. ForcePosition
        // also resets fix_x/fix_y even when its integer destination is
        // unchanged (C4Object.cpp:1414-1515; C4Movement.cpp:536-542).
        let script = r#"#strict 3
local first_y, second_y, adjusted_y;
func Twice() {
    DoCon(-4); first_y = GetY();
    DoCon(-4); second_y = GetY();
    return second_y;
}
func Snap() {
    DoCon(-4); adjusted_y = GetY();
    SetPosition(GetX(), GetY());
    return adjusted_y;
}
func Sequence() {
    DoCon(-4); first_y = GetY();
    SetPosition(70, 80);
    DoCon(-4); second_y = GetY();
    return second_y;
}
"#;
        let mut definition = Definition::from_script("DHRS", "Dead Horse", script)?;
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 25)));
        definition.set_stretch_growth(true);
        let mut engine = Engine::with_seed(75);
        engine.register_definition(definition)?;

        let spawn_at_100 = |engine: &mut Engine| -> Result<ObjectId, EngineError> {
            let id = engine.spawn_object(
                SpawnConfig::new("DHRS")
                    .with_position(Vector2::new(20, 125))
                    .with_construction(FULL_CON),
            )?;
            engine.apply_object_update(
                id,
                ObjectUpdate::new().with_position(Vector2::new(20, 100)),
            )?;
            Ok(id)
        };

        let twice = spawn_at_100(&mut engine)?;
        let idx = engine.find_object_index(twice).expect("object exists");
        assert_eq!(
            engine.call_object_function(idx, "Twice", Vec::new())?,
            Value::Int(102)
        );
        let object = engine.object_snapshot(twice).expect("object survives");
        assert_eq!(object.construction, 92_000);
        assert_eq!(object.position.y, 102);
        assert_eq!(object.local_vars.get("first_y"), Some(&Value::Int(101)));
        assert_eq!(object.local_vars.get("second_y"), Some(&Value::Int(102)));
        let idx = engine.find_object_index(twice).expect("object survives");
        assert_eq!(engine.objects[idx].fixed_position.y, itofix(100));

        let snap = spawn_at_100(&mut engine)?;
        let idx = engine.find_object_index(snap).expect("object exists");
        assert_eq!(
            engine.call_object_function(idx, "Snap", Vec::new())?,
            Value::Int(101)
        );
        let idx = engine.find_object_index(snap).expect("object survives");
        assert_eq!(engine.objects[idx].state.position.y, 101);
        assert_eq!(engine.objects[idx].fixed_position.y, itofix(101));

        let sequence = spawn_at_100(&mut engine)?;
        let idx = engine.find_object_index(sequence).expect("object exists");
        assert_eq!(
            engine.call_object_function(idx, "Sequence", Vec::new())?,
            Value::Int(81)
        );
        let idx = engine.find_object_index(sequence).expect("object survives");
        assert_eq!(engine.objects[idx].state.construction, 92_000);
        assert_eq!(engine.objects[idx].state.position, Vector2::new(70, 81));
        assert_eq!(
            engine.objects[idx].fixed_position,
            FixedVec2::from_ints(70, 80)
        );
        Ok(())
    }

    #[test]
    fn script_docon_updates_an_explicit_foreign_object() -> Result<(), EngineError> {
        // FnDoCon forwards its optional pObj directly to C4Object::DoCon
        // (src/C4Script.cpp:480-484). GoldRush's Indian reproduction creates
        // a child, enters it into the tipi, then calls DoCon(-40, pIndian);
        // the foreign child must shrink synchronously, with DoCon preserving
        // the fixed position established by Enter's CopyMotion.
        let parent_script = r#"#strict
local observed_con, observed_mass;
func Reproduce() {
    var child = CreateObject(CHLD, 0, 0, -1);
    Enter(this(), child);
    DoCon(-40, child);
    observed_con = GetCon(child);
    observed_mass = GetMass(child);
    return(1);
}
"#;
        let parent = Definition::from_script("PARN", "Parent", parent_script)?;
        let mut child = Definition::from_script(
            "CHLD",
            "Child",
            r#"#strict 3
local intercepted;
func DoCon(int amount) { intercepted = amount; return false; }
"#,
        )?;
        child.set_shape_rect(Some(DefinitionRect::new(0, -10, 10, 20)));
        child.set_stretch_growth(true);
        child.set_incomplete_activity(true);
        child.set_mass(10);

        let mut engine = Engine::with_seed(79);
        engine.register_definition(parent)?;
        engine.register_definition(child)?;
        let parent_id = engine.spawn_object(
            SpawnConfig::new("PARN").with_position(Vector2::new(20, 100)),
        )?;
        let parent_idx = engine
            .find_object_index(parent_id)
            .expect("parent exists");

        engine.call_object_function(parent_idx, "Reproduce", Vec::new())?;

        let child = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "CHLD")
            .expect("child exists");
        assert_eq!(child.state.container, Some(parent_id));
        assert_eq!(child.state.construction, 60_000);
        assert_eq!(child.state.position.y, 104);
        assert_eq!(child.fixed_position.y, itofix(100));
        assert_eq!(child.state.local_vars.get("intercepted"), None);
        let parent = engine.object_snapshot(parent_id).expect("parent survives");
        assert_eq!(parent.local_vars.get("observed_con"), Some(&Value::Int(60)));
        assert_eq!(parent.local_vars.get("observed_mass"), Some(&Value::Int(6)));
        Ok(())
    }

    #[test]
    fn script_docon_refreshes_mass_and_value_sort_keys_inline() -> Result<(), EngineError> {
        // DoCon calls UpdateMass before returning. C4SO_Mass reads that live
        // cached field, while C4SO_Value calls GetValue (including live Con
        // scaling), so shrinking 100 -> 50 moves the target below a 75 peer
        // in both ascending sorts (C4Object.cpp:497-505,2118-2139;
        // C4FindObject.cpp:924-932).
        let caller = Definition::from_script(
            "CALL",
            "Caller",
            r#"#strict 3
local mass_first, value_first;
func ShrinkAndSort(object target) {
    DoCon(-50, target);
    var by_mass = FindObjects([C4FO_Category, 16], [C4SO_Mass]);
    mass_first = GetX(by_mass[0]);
    var by_value = FindObjects([C4FO_Category, 16], [C4SO_Value]);
    value_first = GetX(by_value[0]);
    return mass_first * 1000 + value_first;
}
"#,
        )?;
        let mut target = simple_definition("TARG");
        target.set_mass(100);
        target.set_value(100);
        let mut peer = simple_definition("PEER");
        peer.set_mass(75);
        peer.set_value(75);

        let mut engine = Engine::with_seed(81);
        engine.register_definition(caller)?;
        engine.register_definition(target)?;
        engine.register_definition(peer)?;
        let caller = engine.spawn_object(SpawnConfig::new("CALL"))?;
        let target = engine.spawn_object(
            SpawnConfig::new("TARG")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(100, 0)),
        )?;
        engine.spawn_object(
            SpawnConfig::new("PEER")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(200, 0)),
        )?;

        let index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine.call_object_function(
                index,
                "ShrinkAndSort",
                vec![Value::Object(target.as_u64())],
            )?,
            Value::Int(100_100)
        );
        let caller = engine.object_snapshot(caller).expect("caller survives");
        assert_eq!(caller.local_vars.get("mass_first"), Some(&Value::Int(100)));
        assert_eq!(caller.local_vars.get("value_first"), Some(&Value::Int(100)));
        Ok(())
    }

    #[test]
    fn script_docon_recomputes_construction_ocf_bits_and_idle_procedure_inline(
    ) -> Result<(), EngineError> {
        let script = r#"#strict 3
local observed_ocf, observed_procedure, completion_ocf;
func Decay() {
    DoCon(-1);
    observed_ocf = GetOCF();
    observed_procedure = GetProcedure();
    return observed_ocf;
}
func Grow() { DoCon(1); observed_ocf = GetOCF(); return observed_ocf; }
func Completion() { completion_ocf = GetOCF(); return true; }
"#;
        let mut definition = Definition::from_script("OCFP", "OCF probe", script)?;
        definition.set_constructable(true);
        definition.set_category(CATEGORY_LIVING);
        definition.set_rotateable(1);
        definition.set_entrance_rect(Some(DefinitionRect::new(0, 0, 4, 4)));
        definition.set_collection_rect(Some(DefinitionRect::new(0, 0, 4, 4)));
        definition.set_attract_lightning(true);
        definition.set_line_connect(
            LINE_CONNECT_POWER_CONSUMER | LINE_CONNECT_POWER_GENERATOR,
        );
        definition.configure_actions(
            Some("Work".to_string()),
            HashMap::from([
                (
                    "Work".to_string(),
                    ActionSpec::default()
                        .with_procedure("ATTACH")
                        .with_disabled(true),
                ),
                ("Idle".to_string(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(83);
        engine.register_definition(definition)?;
        let decaying = engine.spawn_object(
            SpawnConfig::new("OCFP")
                .with_construction(FULL_CON)
                .with_alive(true)
                .with_action(ActionState::new("Work")),
        )?;
        let index = engine.find_object_index(decaying).expect("probe exists");
        let decayed_ocf = engine
            .call_object_function(index, "Decay", Vec::new())?
            .as_c4_int()
            .expect("GetOCF returns int") as u32;
        let full_only = ocf::FULL_CON
            | ocf::ENTRANCE
            | ocf::COLLECTION
            | ocf::LINE_CONSTRUCT
            | ocf::ATTRACT_LIGHTNING
            | ocf::POWER_CONSUMER
            | ocf::POWER_SUPPLY
            | ocf::CONTAINER;
        assert_eq!(decayed_ocf & full_only, 0);
        assert_ne!(decayed_ocf & ocf::CONSTRUCT, 0);
        assert_ne!(decayed_ocf & ocf::ROTATE, 0);
        assert_ne!(
            decayed_ocf & ocf::FIGHT_READY,
            0,
            "forced Idle runs SetOCF again and restores FightReady"
        );
        let decaying = engine.object_snapshot(decaying).expect("probe survives");
        assert_eq!(decaying.action.name, "Idle");
        assert_eq!(decaying.local_vars.get("observed_procedure"), Some(&Value::Nil));

        let growing = engine.spawn_object(
            SpawnConfig::new("OCFP")
                .with_construction(FULL_CON - FULL_CON / 100)
                .with_alive(true)
                .with_action(ActionState::new("Idle")),
        )?;
        let index = engine.find_object_index(growing).expect("probe exists");
        let grown_ocf = engine
            .call_object_function(index, "Grow", Vec::new())?
            .as_c4_int()
            .expect("GetOCF returns int") as u32;
        assert_eq!(grown_ocf & full_only, full_only);
        assert_eq!(grown_ocf & ocf::CONSTRUCT, 0);
        assert_ne!(grown_ocf & ocf::ROTATE, 0);
        let growing = engine.object_snapshot(growing).expect("probe survives");
        assert_eq!(
            growing.local_vars.get("completion_ocf"),
            Some(&Value::Int(grown_ocf as i32)),
            "Completion observes the SetOCF result before DoCon returns"
        );
        Ok(())
    }

    #[test]
    fn script_docon_foreign_idle_procedure_and_collection_gate_are_live(
    ) -> Result<(), EngineError> {
        let caller = Definition::from_script(
            "CALL",
            "Caller",
            r#"#strict 3
local foreign_action;
func DecayForeign(object target) {
    DoCon(-1, target);
    foreign_action = GetAction(target);
    return GetProcedure(target);
}
"#,
        )?;
        let collector_script = r#"#strict 3
func DecayAndCollect(object item) {
    DoCon(-1);
    return Collect(item, this());
}
"#;
        let mut collector = Definition::from_script("SITE", "Site", collector_script)?;
        collector.set_collection_rect(Some(DefinitionRect::new(0, 0, 4, 4)));
        collector.configure_actions(
            Some("Work".to_string()),
            HashMap::from([
                (
                    "Work".to_string(),
                    ActionSpec::default().with_procedure("ATTACH"),
                ),
                ("Idle".to_string(), ActionSpec::default()),
            ]),
        );
        let mut engine = Engine::with_seed(85);
        engine.register_definition(caller)?;
        engine.register_definition(collector)?;
        engine.register_definition(simple_definition("ITEM"))?;
        let caller = engine.spawn_object(SpawnConfig::new("CALL"))?;
        let foreign = engine.spawn_object(
            SpawnConfig::new("SITE").with_action(ActionState::new("Work")),
        )?;
        let index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine.call_object_function(
                index,
                "DecayForeign",
                vec![Value::Object(foreign.as_u64())],
            )?,
            Value::Nil
        );
        let caller = engine.object_snapshot(caller).expect("caller survives");
        assert_eq!(
            caller.local_vars.get("foreign_action"),
            Some(&Value::String("Idle".into()))
        );

        let collector = engine.spawn_object(
            SpawnConfig::new("SITE").with_action(ActionState::new("Work")),
        )?;
        let item = engine.spawn_object(SpawnConfig::new("ITEM"))?;
        let index = engine.find_object_index(collector).expect("collector exists");
        assert_eq!(
            engine.call_object_function(
                index,
                "DecayAndCollect",
                vec![Value::Object(item.as_u64())],
            )?,
            Value::Bool(false),
            "partial non-IncompleteActivity objects lose Collection immediately"
        );
        assert_eq!(engine.object_snapshot(item).expect("item survives").container, None);
        Ok(())
    }

    #[test]
    fn script_docon_zero_removes_inactive_targets_and_refreshes_parent_collection(
    ) -> Result<(), EngineError> {
        let caller = Definition::from_script(
            "CALL",
            "Caller",
            r#"#strict 3
func RemoveInactive(object target) {
    DoCon(-100, target);
    return ObjectCount2([C4FO_Category, 0]);
}
"#,
        )?;
        let child = Definition::from_script(
            "CHLD",
            "Child",
            r#"#strict 3
func Vanish(object parent) { DoCon(-100); return GetOCF(parent); }
"#,
        )?;
        let mut parent = simple_definition("HOLD");
        parent.set_collection_rect(Some(DefinitionRect::new(0, 0, 4, 4)));
        parent.set_collection_limit(1);

        let mut engine = Engine::with_seed(87);
        engine.register_definition(caller)?;
        engine.register_definition(child)?;
        engine.register_definition(parent)?;
        let caller = engine.spawn_object(SpawnConfig::new("CALL"))?;
        let inactive = engine.spawn_object(SpawnConfig::new("CHLD"))?;
        engine.apply_object_update(
            inactive,
            ObjectUpdate::new().with_status(ObjectStatus::Inactive),
        )?;
        let inactive_bystander = engine.spawn_object(SpawnConfig::new("CHLD"))?;
        engine.apply_object_update(
            inactive_bystander,
            ObjectUpdate::new().with_status(ObjectStatus::Inactive),
        )?;
        let index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine.call_object_function(
                index,
                "RemoveInactive",
                vec![Value::Object(inactive.as_u64())],
            )?,
            Value::Int(1),
            "raw Status-zero counting excludes the removed inactive target"
        );
        assert_eq!(
            engine.object_snapshot(inactive).map(|object| object.status),
            Some(ObjectStatus::Deleted)
        );

        let parent = engine.spawn_object(SpawnConfig::new("HOLD"))?;
        let child = engine.spawn_object(SpawnConfig::new("CHLD").with_container(parent))?;
        let parent_before = engine.object_snapshot(parent).expect("parent exists");
        assert_eq!(parent_before.ocf & ocf::COLLECTION, 0, "limit is full");
        let index = engine.find_object_index(child).expect("child exists");
        let parent_ocf = engine
            .call_object_function(
                index,
                "Vanish",
                vec![Value::Object(parent.as_u64())],
            )?
            .as_c4_int()
            .expect("GetOCF returns int") as u32;
        assert_ne!(parent_ocf & ocf::COLLECTION, 0);
        assert_eq!(
            engine.object_snapshot(child).map(|object| object.status),
            Some(ObjectStatus::Deleted)
        );
        Ok(())
    }

    #[test]
    fn script_docon_full_crossing_runs_completion_initialize_inline_once(
    ) -> Result<(), EngineError> {
        // C4Object::DoCon calls Completion and Initialize before FnDoCon
        // returns. A later call while already complete runs neither callback
        // (C4Object.cpp:1506-1511).
        let script = r#"#strict 3
local lifecycle;
func Finish() {
    if (!lifecycle) lifecycle = 0;
    DoCon(1);
    lifecycle = lifecycle * 10 + 3;
    return lifecycle;
}
func Completion() { lifecycle = lifecycle * 10 + 1; return true; }
func Initialize() { lifecycle = lifecycle * 10 + 2; return true; }
"#;
        let definition = Definition::from_script("SYNC", "Synchronous DoCon", script)?;
        let mut engine = Engine::with_seed(101);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("SYNC").with_construction(FULL_CON - FULL_CON / 100),
        )?;

        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine.call_object_function(idx, "Finish", Vec::new())?,
            Value::Int(123),
            "Completion and Initialize precede the caller's next instruction"
        );
        let idx = engine.find_object_index(id).expect("object remains");
        assert_eq!(
            engine.call_object_function(idx, "Finish", Vec::new())?,
            Value::Int(1233),
            "an already-full object does not repeat either callback"
        );
        Ok(())
    }

    #[test]
    fn script_docon_completion_removal_suppresses_initialize() -> Result<(), EngineError> {
        // Each C4Object::Call re-checks Status. Completion may remove the
        // object, in which case the immediately following Initialize is a
        // no-op and consumes no synchronized RNG (C4Object.cpp:1506-1511,
        // 2224-2227).
        let script = r#"#strict 3
func Finish() { DoCon(1); return true; }
func Completion() { RemoveObject(); return true; }
func Initialize() { Random(100); return true; }
"#;
        let definition = Definition::from_script("GONE", "Removed on completion", script)?;
        let mut engine = Engine::with_seed(103);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("GONE").with_construction(FULL_CON - FULL_CON / 100),
        )?;
        let rng_before = engine.rng.clone();

        let idx = engine.find_object_index(id).expect("object exists");
        engine.call_object_function(idx, "Finish", Vec::new())?;

        assert_eq!(
            engine.object_snapshot(id).map(|object| object.status),
            Some(ObjectStatus::Deleted)
        );
        assert_eq!(engine.rng, rng_before, "Initialize was suppressed");
        Ok(())
    }

    #[test]
    fn script_docon_decay_ejects_contents_clears_need_energy_and_idles_inline(
    ) -> Result<(), EngineError> {
        // Dropping below FullCon ejects every content, clears NeedEnergy and
        // switches a non-IncompleteActivity definition to ActIdle before the
        // script resumes (C4Object.cpp:1459-1472).
        let site_script = r#"#strict 3
local action_after, content_after, ejections;
func Decay() {
    DoCon(-1);
    action_after = GetAction();
    content_after = Contents();
    return true;
}
func Ejection(object child) { ++ejections; return true; }
"#;
        let item_script = r#"#strict 3
local departures;
func Departure(object old_container) { ++departures; return true; }
"#;
        let mut site = Definition::from_script("SITE", "Site", site_script)?;
        site.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Work".to_string(), ActionSpec::default()),
            ]),
        );
        let item = Definition::from_script("ITEM", "Item", item_script)?;
        let mut engine = Engine::with_seed(107);
        engine.register_definition(site)?;
        engine.register_definition(item)?;
        let site_id = engine.spawn_object(
            SpawnConfig::new("SITE")
                .with_construction(FULL_CON)
                .with_action(ActionState::new("Work")),
        )?;
        engine.apply_object_update(site_id, ObjectUpdate::new().with_need_energy(true))?;
        let item_id = engine.spawn_object(
            SpawnConfig::new("ITEM")
                .with_container(site_id)
                .with_velocity(Vector2::new(7, -3))
                .with_rotation(27),
        )?;

        let idx = engine.find_object_index(site_id).expect("site exists");
        engine.call_object_function(idx, "Decay", Vec::new())?;

        let site = engine.object_snapshot(site_id).expect("site survives");
        assert_eq!(site.construction, FULL_CON - FULL_CON / 100);
        assert!(!site.need_energy);
        assert_eq!(site.action.name, "Idle");
        assert_eq!(site.local_vars.get("action_after"), Some(&Value::String("Idle".into())));
        assert_eq!(site.local_vars.get("content_after"), Some(&Value::Nil));
        assert_eq!(site.local_vars.get("ejections"), Some(&Value::Int(1)));
        let item = engine.object_snapshot(item_id).expect("item survives ejection");
        assert_eq!(item.container, None);
        assert_eq!(item.velocity, Vector2::ZERO);
        assert_eq!(item.rotation, 0);
        assert_eq!(item.local_vars.get("departures"), Some(&Value::Int(1)));
        Ok(())
    }

    #[test]
    fn script_docon_decay_removes_grid_mask_before_ejection_callback(
    ) -> Result<(), EngineError> {
        // C4Object::DoCon removes a full object's mask and runs UpdateFace
        // before ejecting contents. Ejection and the caller's post-DoCon
        // tail therefore both see sky, even though Rust folds the object
        // update only after the outer VM call returns (C4Object.cpp:
        // 1447-1466).
        let script = r#"#strict 3
local during, after;
func Decay() { DoCon(-1); after = GBackSolid(0, 0); return after; }
func Ejection(object child) { during = GBackSolid(0, 0); return true; }
"#;
        let mut gate = Definition::from_script("GATE", "Gate", script)?;
        gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = Engine::with_seed(109);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine.register_definition(gate)?;
        engine.register_definition(simple_definition("ITEM"))?;
        let gate_id = engine.spawn_object(
            SpawnConfig::new("GATE")
                .with_loaded(true)
                .with_position(Vector2::new(10, 10)),
        )?;
        engine.spawn_object(SpawnConfig::new("ITEM").with_container(gate_id))?;
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);

        let index = engine.find_object_index(gate_id).expect("gate exists");
        assert_eq!(
            engine.call_object_function(index, "Decay", Vec::new())?,
            Value::Bool(false)
        );

        let gate = engine.object_snapshot(gate_id).expect("gate survives");
        assert_eq!(gate.local_vars.get("during"), Some(&Value::Bool(false)));
        assert_eq!(gate.local_vars.get("after"), Some(&Value::Bool(false)));
        assert!(vehicle_pixels(&engine).is_empty());
        Ok(())
    }

    #[test]
    fn script_docon_completion_sees_new_mask_at_adjusted_position(
    ) -> Result<(), EngineError> {
        // Crossing FullCon first puts the new mask through UpdateFace, then
        // keep-bottom moves this 1px object upward and re-puts it before
        // Completion. Both Completion and the caller's tail query the final
        // y=9 mask (C4Object.cpp:1450,1480-1511).
        let script = r#"#strict 3
local during, after;
func Grow() { DoCon(1); after = GBackSolid(0, 0); return after; }
func Completion() { during = GBackSolid(0, 0); return true; }
"#;
        let mut gate = Definition::from_script("GATE", "Gate", script)?;
        gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = Engine::with_seed(113);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine.register_definition(gate)?;
        let gate_id = engine.spawn_object(
            SpawnConfig::new("GATE")
                .with_loaded(true)
                .with_position(Vector2::new(10, 10))
                .with_construction(FULL_CON - FULL_CON / 100),
        )?;
        assert!(vehicle_pixels(&engine).is_empty());

        let index = engine.find_object_index(gate_id).expect("gate exists");
        assert_eq!(
            engine.call_object_function(index, "Grow", Vec::new())?,
            Value::Bool(true)
        );

        let gate = engine.object_snapshot(gate_id).expect("gate survives");
        assert_eq!(gate.position, Vector2::new(10, 9));
        assert_eq!(gate.local_vars.get("during"), Some(&Value::Bool(true)));
        assert_eq!(gate.local_vars.get("after"), Some(&Value::Bool(true)));
        assert_eq!(vehicle_pixels(&engine), vec![(10, 9)]);
        Ok(())
    }

    #[test]
    fn script_docon_completion_removal_clears_new_mask_inline(
    ) -> Result<(), EngineError> {
        // Completion may immediately AssignRemoval after DoCon has put and
        // keep-bottom-moved the newly full mask. AssignRemoval deletes that
        // mask synchronously, so the suspended Grow caller sees sky when it
        // resumes (C4Object.cpp:277-283,1506-1511).
        let script = r#"#strict 3
func Grow() { DoCon(1); return GBackSolid(0, 0); }
func Completion() { RemoveObject(); return true; }
"#;
        let mut gate = Definition::from_script("GATE", "Gate", script)?;
        gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = Engine::with_seed(119);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine.register_definition(gate)?;
        let gate_id = engine.spawn_object(
            SpawnConfig::new("GATE")
                .with_loaded(true)
                .with_position(Vector2::new(10, 10))
                .with_construction(FULL_CON - FULL_CON / 100),
        )?;

        let index = engine.find_object_index(gate_id).expect("gate exists");
        assert_eq!(
            engine.call_object_function(index, "Grow", Vec::new())?,
            Value::Bool(false)
        );
        assert_eq!(
            engine.object_snapshot(gate_id).map(|object| object.status),
            Some(ObjectStatus::Deleted)
        );
        assert!(vehicle_pixels(&engine).is_empty());
        Ok(())
    }

    #[test]
    fn assign_removal_keeps_grid_mask_through_recursive_child_destruction(
    ) -> Result<(), EngineError> {
        // AssignRemoval sets the parent's Status to zero before recursively
        // killing contents, but pSolidMaskData is removed only after that
        // loop and the pointer sweep (C4Object.cpp:276-313).
        let parent_script = r#"#strict 3
func Vanish() { RemoveObject(); return true; }
"#;
        let child_script = r#"#strict 3
local observer;
func Arm(object target) { observer = target; return true; }
func Destruction() { observer->Record(GBackSolid(0, 0)); return true; }
"#;
        let watcher_script = r#"#strict 3
local saw_mask;
func Record(bool solid) { saw_mask = solid; return true; }
"#;
        let mut parent = Definition::from_script("MASK", "Mask parent", parent_script)?;
        parent.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        parent.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = Engine::with_seed(123);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine.register_definition(parent)?;
        engine.register_definition(Definition::from_script(
            "CHLD",
            "Child",
            child_script,
        )?)?;
        engine.register_definition(Definition::from_script(
            "WATCH",
            "Watcher",
            watcher_script,
        )?)?;
        let parent = engine.spawn_object(
            SpawnConfig::new("MASK")
                .with_loaded(true)
                .with_position(Vector2::new(10, 10)),
        )?;
        let child = engine.spawn_object(
            SpawnConfig::new("CHLD")
                .with_container(parent)
                .with_position(Vector2::new(10, 10)),
        )?;
        let watcher = engine.spawn_object(SpawnConfig::new("WATCH"))?;
        let child_index = engine.find_object_index(child).expect("child exists");
        engine.call_object_function(
            child_index,
            "Arm",
            vec![Value::Object(watcher.as_u64())],
        )?;
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);

        let parent_index = engine.find_object_index(parent).expect("parent exists");
        engine.call_object_function(parent_index, "Vanish", Vec::new())?;

        let watcher = engine.object_snapshot(watcher).expect("watcher survives");
        assert_eq!(watcher.local_vars.get("saw_mask"), Some(&Value::Bool(true)));
        assert!(vehicle_pixels(&engine).is_empty());
        Ok(())
    }

    #[test]
    fn script_docon_decay_preserves_overlapping_grid_mask() -> Result<(), EngineError> {
        // Removing one mask temporarily restores its saved sky byte, then
        // C4SolidMask::Remove re-puts every overlapping mask and refreshes
        // that mask's material buffer. A line definition keeps the query at
        // the same position while construction drops below FullCon.
        let script = r#"#strict 3
func Decay() { DoCon(-1); return GBackSolid(0, 0); }
"#;
        let mut gate = Definition::from_script("GATE", "Gate", script)?;
        gate.set_line(1);
        gate.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        gate.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)));

        let mut engine = Engine::with_seed(127);
        engine.set_landscape(vehicle_grid_landscape(20, 20));
        engine.register_definition(gate)?;
        let decaying = engine.spawn_object(
            SpawnConfig::new("GATE")
                .with_loaded(true)
                .with_position(Vector2::new(10, 10)),
        )?;
        engine.spawn_object(
            SpawnConfig::new("GATE")
                .with_loaded(true)
                .with_position(Vector2::new(10, 10)),
        )?;
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);

        let index = engine.find_object_index(decaying).expect("gate exists");
        assert_eq!(
            engine.call_object_function(index, "Decay", Vec::new())?,
            Value::Bool(true)
        );
        assert_eq!(vehicle_pixels(&engine), vec![(10, 10)]);
        Ok(())
    }

    #[test]
    fn fire_decay_zero_runs_full_assign_removal_protocol() -> Result<(), EngineError> {
        // ExecFire's DoCon(-100) reaches AssignRemoval at Con zero. The
        // containing object's callback, the victim's Destruction/effect Stop,
        // and recursive child Destruction/effect Stop all run in that order.
        let container_script = r#"#strict 3
local order, stop_reason;
func ContentsDestruction(object victim) {
    if (!order) order = 0;
    order = order * 10 + 1;
    return true;
}
func Record(int step, int reason) {
    if (!order) order = 0;
    order = order * 10 + step;
    if (reason) stop_reason = reason;
    return true;
}
"#;
        let victim_script = r#"#strict 3
func Arm() { AddEffect("Witness", this(), 200, 0, this()); return true; }
func Destruction() {
    if (Contained()) Contained()->Record(2, 0);
    return true;
}
func FxWitnessStop(object target, int number, int reason) {
    if (Contained()) Contained()->Record(3, reason);
    return 0;
}
"#;
        let child_script = r#"#strict 3
local observer;
func Arm(object target) {
    observer = target;
    AddEffect("ChildWitness", this(), 150, 0, this());
    return true;
}
func Destruction() { observer->Record(4, 0); return true; }
func FxChildWitnessStop(object target, int number, int reason) {
    observer->Record(5, reason);
    return 0;
}
"#;
        let container = Definition::from_script("HOLD", "Holder", container_script)?;
        let mut victim = Definition::from_script("BURN", "Burning victim", victim_script)?;
        victim.set_c4_callback_convention(true);
        victim.set_incomplete_activity(true);
        let mut child = Definition::from_script("CHLD", "Recursive child", child_script)?;
        child.set_c4_callback_convention(true);
        let mut engine = Engine::with_seed(109);
        engine.register_definition(container)?;
        engine.register_definition(victim)?;
        engine.register_definition(child)?;
        let holder = engine.spawn_object(SpawnConfig::new("HOLD"))?;
        let burning = engine.spawn_object(
            SpawnConfig::new("BURN")
                .with_container(holder)
                .with_construction(FULL_CON / 1000),
        )?;
        let child = engine.spawn_object(SpawnConfig::new("CHLD").with_container(burning))?;
        let idx = engine.find_object_index(child).expect("child exists");
        engine.call_object_function(
            idx,
            "Arm",
            vec![Value::Object(holder.as_u64())],
        )?;
        let idx = engine.find_object_index(burning).expect("victim exists");
        engine.call_object_function(idx, "Arm", Vec::new())?;
        let idx = engine.find_object_index(burning).expect("victim remains");
        assert!(engine.incinerate_object(idx, 0, false, None)?);
        assert_eq!(
            engine
                .object_snapshot(holder)
                .expect("holder survives ignition")
                .local_vars
                .get("order"),
            Some(&Value::Int(3)),
            "Fire Start temporarily removes the higher-priority Witness effect"
        );

        engine.tick_without_snapshot()?;

        assert!(engine.object_snapshot(burning).is_none());
        assert!(engine.object_snapshot(child).is_none());
        let holder = engine.object_snapshot(holder).expect("holder survives");
        assert_eq!(holder.local_vars.get("order"), Some(&Value::Int(312345)));
        assert_eq!(holder.local_vars.get("stop_reason"), Some(&Value::Int(3)));
        Ok(())
    }

    #[test]
    fn fire_decay_removal_still_runs_same_frame_damage_and_energy_tail(
    ) -> Result<(), EngineError> {
        // ExecFire does not return after DoCon reaches zero. On a Tick10
        // frame the now-deleted tombstone still receives fire Damage(+2)
        // and DoEnergy(-1) before the background tail (C4Object.cpp:776-806).
        let mut engine = Engine::with_seed(111);
        let mut victim_definition = simple_definition("BURN");
        victim_definition.set_physical(PhysicalInfo {
            energy: 5_000,
            ..PhysicalInfo::default()
        });
        engine.register_definition(victim_definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("BURN")
                .with_construction(FULL_CON / 1000)
                .with_energy(5_000),
        )?;
        let idx = engine.find_object_index(id).expect("victim exists");
        assert!(engine.incinerate_object(idx, 0, false, None)?);
        let fire_number = engine.objects[idx]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == C4FX_FIRE)
            .map(|effect| effect.number)
            .expect("fire effect exists");

        engine.exec_object_fire(idx, 10, fire_number);

        assert!(engine.objects[idx].destroyed);
        assert_eq!(engine.objects[idx].state.status, ObjectStatus::Deleted);
        assert_eq!(engine.objects[idx].state.damage, 2);
        assert_eq!(engine.objects[idx].state.energy, 4_000);
        Ok(())
    }

    #[test]
    fn script_docon_rotated_structure_lifts_inline_by_cpp_step_formula(
    ) -> Result<(), EngineError> {
        let script = r#"#strict 3
func Grow() { DoCon(1); return GetY(); }
func RoundTrip() { DoCon(1); DoCon(-1); return GetY(); }
func RepositionAndGrow() { SetPosition(40, 300); DoCon(1); return GetY(); }
"#;
        let mut definition = Definition::from_script("ROTA", "Rotated structure", script)?;
        definition.set_category(CATEGORY_STRUCTURE);
        definition.set_shape_rect(Some(DefinitionRect::new(-10, -125, 20, 250)));
        definition.set_rotateable(1);
        let mut engine = Engine::with_seed(113);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("ROTA")
                .with_category(CATEGORY_STRUCTURE)
                .with_position(Vector2::new(40, 200))
                .with_rotation(90)
                .with_construction(49_500),
        )?;
        let idx = engine.find_object_index(id).expect("structure exists");
        let y_before = engine.objects[idx].state.position.y;
        let fixed_before = engine.objects[idx].fixed_position.y;

        let callback_y = engine.call_object_function(idx, "Grow", Vec::new())?;

        let object = engine.object_snapshot(id).expect("structure survives");
        // floor(50*250/100) - floor(49*250/100) = 3.
        assert_eq!(callback_y, Value::Int(y_before - 3));
        assert_eq!(object.position.y, y_before - 3);
        assert_eq!(object.construction, 50_500);
        let idx = engine.find_object_index(id).expect("structure survives");
        assert_eq!(engine.objects[idx].fixed_position.y, fixed_before);

        let round_trip = engine.spawn_object(
            SpawnConfig::new("ROTA")
                .with_category(CATEGORY_STRUCTURE)
                .with_position(Vector2::new(40, 200))
                .with_rotation(90)
                .with_construction(49_500),
        )?;
        let idx = engine.find_object_index(round_trip).expect("structure exists");
        let y_before = engine.objects[idx].state.position.y;
        let fixed_before = engine.objects[idx].fixed_position.y;
        assert_eq!(
            engine.call_object_function(idx, "RoundTrip", Vec::new())?,
            Value::Int(y_before - 3)
        );
        let idx = engine.find_object_index(round_trip).expect("structure survives");
        assert_eq!(engine.objects[idx].state.construction, 49_500);
        assert_eq!(engine.objects[idx].state.position.y, y_before - 3);
        assert_eq!(engine.objects[idx].fixed_position.y, fixed_before);

        let repositioned = engine.spawn_object(
            SpawnConfig::new("ROTA")
                .with_category(CATEGORY_STRUCTURE)
                .with_position(Vector2::new(40, 200))
                .with_rotation(90)
                .with_construction(49_500),
        )?;
        let idx = engine.find_object_index(repositioned).expect("structure exists");
        assert_eq!(
            engine.call_object_function(idx, "RepositionAndGrow", Vec::new())?,
            Value::Int(297)
        );
        let idx = engine.find_object_index(repositioned).expect("structure survives");
        assert_eq!(engine.objects[idx].state.position.y, 297);
        assert_eq!(engine.objects[idx].fixed_position.y, itofix(300));

        let loaded_turn = engine.spawn_object(
            SpawnConfig::new("ROTA")
                .with_loaded(true)
                .with_category(CATEGORY_STRUCTURE)
                .with_position(Vector2::new(40, 300))
                .with_rotation(360)
                .with_construction(49_500),
        )?;
        let idx = engine.find_object_index(loaded_turn).expect("loaded structure exists");
        assert_eq!(
            engine.call_object_function(idx, "Grow", Vec::new())?,
            Value::Int(297),
            "C++ tests raw r, so loaded r=360 takes the rotated lift branch"
        );
        Ok(())
    }

    #[test]
    fn script_setshape_updates_foreign_and_default_local_collision_rects() -> Result<(), EngineError>
    {
        // FnSetShape accepts any explicit object and calls UpdatePos after
        // writing the live rect (C4Script.cpp:5183-5196).
        let script = r#"#strict 3
func Reshape(object target) {
    return [SetShape(-7, -8, 70, 16, target), SetShape(-2, -3, 4, 6)];
}
"#;
        let mut caller = Definition::from_script("CALL", "Shape caller", script)?;
        caller.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 20)));
        let mut target = Definition::from_script("TARG", "Shape target", "#strict 3")?;
        target.set_shape_rect(Some(DefinitionRect::new(1, 2, 3, 4)));

        let mut engine = Engine::with_seed(124);
        engine.set_landscape(Landscape::flat(120, 120));
        engine.register_definition(caller)?;
        engine.register_definition(target)?;
        let caller_id =
            engine.spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(10, 20)))?;
        let target_id =
            engine.spawn_object(SpawnConfig::new("TARG").with_position(Vector2::new(20, 20)))?;
        let caller_idx = engine.find_object_index(caller_id).expect("caller exists");
        assert!(!engine
            .sectors
            .as_ref()
            .expect("sectors initialized")
            .shape_ids(SectorKey::Inside { x: 1, y: 0 })
            .contains(&target_id));

        assert_eq!(
            engine.call_object_function(
                caller_idx,
                "Reshape",
                vec![Value::Object(target_id.as_u64())],
            )?,
            Value::Array(vec![Value::Bool(true), Value::Bool(true)])
        );

        let caller_idx = engine
            .find_object_index(caller_id)
            .expect("caller survives");
        assert_eq!(
            engine.objects[caller_idx].current_shape_rect(),
            Some(DefinitionRect::new(-2, -3, 4, 6))
        );
        let target_idx = engine
            .find_object_index(target_id)
            .expect("target survives");
        assert_eq!(
            engine.objects[target_idx].current_shape_rect(),
            Some(DefinitionRect::new(-7, -8, 70, 16))
        );
        assert!(engine
            .sectors
            .as_ref()
            .expect("sectors initialized")
            .shape_ids(SectorKey::Inside { x: 1, y: 0 })
            .contains(&target_id));
        Ok(())
    }

    #[test]
    fn script_docon_update_shape_discards_setshape_override() -> Result<(), EngineError> {
        let script = r#"#strict 3
func ResetShape() {
    SetShape(-20, -30, 40, 60);
    DoCon(-1);
    return GetObjectVal("Width");
}
"#;
        let mut definition = Definition::from_script("SHAP", "Shape reset", script)?;
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 20)));
        definition.set_stretch_growth(true);
        let mut engine = Engine::with_seed(125);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(SpawnConfig::new("SHAP"))?;
        let idx = engine.find_object_index(id).expect("object exists");

        assert_eq!(
            engine.call_object_function(idx, "ResetShape", Vec::new())?,
            Value::Int(9)
        );
        let idx = engine.find_object_index(id).expect("object survives");
        assert_eq!(engine.objects[idx].state.shape_override, None);
        assert_eq!(
            engine.objects[idx].current_shape_rect(),
            Some(DefinitionRect::new(0, 0, 9, 19))
        );
        Ok(())
    }

    #[test]
    fn script_docon_oversize_grows_past_full_con_and_scales_shape(
    ) -> Result<(), EngineError> {
        let script = r#"#strict 3
func Grow() { DoCon(50); return GetCon(); }
"#;
        let mut definition = Definition::from_script("OVSZ", "Oversize", script)?;
        definition.set_oversize(true);
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 10, 20)));
        definition.set_shape_vertices(vec![ObjectVertex::new(4, 20)]);
        definition.set_stretch_growth(true);
        let mut engine = Engine::with_seed(127);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("OVSZ")
                .with_position(Vector2::new(30, 100))
                .with_construction(FULL_CON),
        )?;
        let idx = engine.find_object_index(id).expect("oversize object exists");
        let bottom_before = engine.objects[idx]
            .current_shape_rect()
            .map(|shape| engine.objects[idx].state.position.y + shape.y + shape.height)
            .expect("shape exists");

        assert_eq!(
            engine.call_object_function(idx, "Grow", Vec::new())?,
            Value::Int(150)
        );

        let object = engine.object_snapshot(id).expect("oversize object survives");
        assert_eq!(object.construction, 150_000);
        assert_eq!(object.vertices[0], ObjectVertex::new(6, 30));
        let idx = engine.find_object_index(id).expect("oversize object survives");
        let shape = engine.objects[idx].current_shape_rect().expect("shape exists");
        assert_eq!(shape, DefinitionRect::new(0, 0, 15, 30));
        assert_eq!(object.position.y + shape.y + shape.height, bottom_before);

        let loaded = engine.spawn_object(
            SpawnConfig::new("OVSZ")
                .with_loaded(true)
                .with_position(Vector2::new(60, 100))
                .with_construction(150_000),
        )?;
        let loaded = engine.object_snapshot(loaded).expect("loaded object survives");
        assert_eq!(loaded.construction, 150_000);
        assert_eq!(loaded.vertices[0], ObjectVertex::new(6, 30));
        Ok(())
    }

    #[test]
    fn creation_action_precedes_initial_and_placed_docon_fixed_position() -> Result<(), EngineError>
    {
        // GoldRush frame 367, WMPF #1595. C4Game::NewObject constructs the
        // raw y=516 object first, so Construction's SetAction snaps fix_y at
        // 516. Initial DoCon(FullCon,true) moves integer y to 512 without
        // touching fix_y; WMPF::Place then SetCon(10) moves integer y back to
        // 516 and likewise preserves fix_y (C4Game.cpp:1102-1142;
        // C4Object.cpp:1414-1515,4091-4169). Deferred creation must retain
        // that exact order instead of replaying SetAction at intermediate y.
        let parent = Definition::from_script(
            "PARN",
            "Parent",
            r#"#strict
func Seed() {
    var child = CreateObject(CHLD, 0, 0, -1);
    child->Place(this(), 10);
    return(1);
}
"#,
        )?;
        let mut child = Definition::from_script(
            "CHLD",
            "Wompf",
            r#"#strict
func Construction() {
    SetAction("Exist");
    return(1);
}
func Place(tree, growth) {
    SetCategory(1);
    SetActionTargets(tree);
    DoCon(growth - GetCon());
    return(1);
}
"#,
        )?;
        child.set_shape_rect(Some(DefinitionRect::new(-4, -4, 8, 8)));
        child.set_stretch_growth(true);
        child.set_incomplete_activity(true);
        child.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Exist".to_string(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(83);
        engine.register_definition(parent)?;
        engine.register_definition(child)?;
        let parent_id = engine.spawn_object(
            SpawnConfig::new("PARN").with_position(Vector2::new(2827, 516)),
        )?;
        let parent_idx = engine
            .find_object_index(parent_id)
            .expect("parent exists");

        engine.call_object_function(parent_idx, "Seed", Vec::new())?;

        let child = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "CHLD")
            .expect("placed child exists");
        assert_eq!(child.state.construction, 10_000);
        assert_eq!(child.state.action.name, "Exist");
        assert_eq!(child.state.action.target, Some(parent_id));
        assert_eq!(child.state.category & CATEGORY_STATIC_BACK, CATEGORY_STATIC_BACK);
        assert_eq!(child.state.position.y, 516);
        assert_eq!(
            child.fixed_position.y,
            itofix(516),
            "Construction SetAction snapped the raw pre-growth position"
        );
        Ok(())
    }

    #[test]
    fn build_full_con_crossing_runs_completion_then_initialize() -> Result<(), EngineError> {
        let mut builder = simple_definition("Builder");
        builder.set_category(CATEGORY_OBJECT);
        builder.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("Walk").with_length(1),
                ),
                (
                    "Build".to_string(),
                    ActionSpec::default().with_procedure("Build").with_length(1),
                ),
            ]),
        );
        builder.set_physical(PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        });
        let mut target = Definition::from_script(
            "Target",
            "Target",
            r#"#strict
local lifecycle;
protected func Completion() {
  lifecycle = lifecycle * 10 + 1;
  return(1);
}
protected func Initialize() {
  if (GetCon() < 100) return(1);
  lifecycle = lifecycle * 10 + 2;
  CreateObject(CHLD, 0, 0, GetOwner());
  return(1);
}
"#,
        )?;
        target.set_category(CATEGORY_STRUCTURE);
        target.set_mass(100);

        let mut engine = Engine::with_seed(69);
        engine.register_definition(builder)?;
        engine.register_definition(target)?;
        engine.register_definition(simple_definition("CHLD"))?;
        let target_id = engine.spawn_object(
            SpawnConfig::new("Target")
                .with_position(Vector2::new(40, 60))
                .with_construction(FULL_CON - 1),
        )?;
        let mut build = ActionState::new("Build");
        build.target = Some(target_id);
        let builder_id = engine.spawn_object(
            SpawnConfig::new("Builder")
                // A zero/default target Shape admits y in [-16,+16]. Keep
                // this completion fixture inside the native Build area.
                .with_position(Vector2::new(40, 60))
                .with_action(build),
        )?;

        engine.tick_without_snapshot()?;

        let target = engine.object_snapshot(target_id).expect("target survives");
        assert_eq!(target.construction, FULL_CON);
        assert_eq!(target.local_vars.get("lifecycle"), Some(&Value::Int(12)));
        assert_eq!(
            engine
                .snapshot()
                .objects
                .iter()
                .filter(|object| object.definition_id == "CHLD")
                .count(),
            1,
            "ELEV-style Initialize creates its child on live completion"
        );
        let builder = engine
            .object_snapshot(builder_id)
            .expect("builder survives crossing");
        assert_eq!(
            builder.action.name,
            "Build",
            "the crossing tick still sees Target::Build succeed"
        );
        assert_eq!(
            builder.action.time, 1,
            "pre-switch Action.Time increment must not repeat in the phase tail"
        );

        // The next Build frame sees an already-complete target and stops;
        // the following frame executes the resulting Walk action.
        engine.tick_without_snapshot()?;
        engine.tick_without_snapshot()?;
        assert_eq!(
            engine
                .object_snapshot(builder_id)
                .expect("builder survives recovery")
                .action
                .name,
            "Walk"
        );
        Ok(())
    }

    #[test]
    fn construction_owned_vertices_survive_restore_like_cpp() -> Result<(), EngineError> {
        // Mirrors src/C4Object.cpp:2769 and src/C4Shape.cpp:486-494: saved
        // objects persist the `OwnVertices` flag, and own original vertices are
        // stored separately from the active shape. UpdateShape then copies from
        // that own base at src/C4Object.cpp:326 before non-stretch construction
        // calls C4Shape::Jolt at src/C4Shape.cpp:121-127.
        //
        // Hand-derived golden: the definition base vertex is y=4, but the owned
        // base vertex is y=8. After restore, changing Con from FullCon to
        // FullCon/2 must jolt the owned base to y=4, not the definition base to
        // y=2. The spawn y 8 is the con-0 bottom, so the full-con center is
        // 8 - (4 + 0) = 4 (C4Object.cpp:1462-1468); the straight-object
        // bottom preserve then moves y to 8 - 2 - 0 = 6.
        let mut definition = simple_definition("OwnedShape");
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 4)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 4).with_cnat(CNAT_BOTTOM)]);

        let mut engine = Engine::with_seed(66);
        engine.register_definition(definition.clone())?;
        let id = engine.spawn_object(
            SpawnConfig::new("OwnedShape")
                .with_position(Vector2::new(3, 8))
                .with_construction(FULL_CON)
                .with_vertices(vec![ObjectVertex::new(0, 8).with_cnat(CNAT_BOTTOM)]),
        )?;

        let state = engine.capture_state();
        let mut restored = Engine::with_seed(67);
        restored.register_definition(definition)?;
        restored.restore_state(&state)?;

        restored.apply_object_update(id, ObjectUpdate::new().with_construction(FULL_CON / 2))?;

        let object = restored.object_snapshot(id).expect("object present");
        assert_eq!(object.construction, FULL_CON / 2);
        assert_eq!(object.position, Vector2::new(3, 10));
        assert_eq!(object.vertices[0].y, 4);
        assert_eq!(object.vertices[0].cnat, CNAT_BOTTOM);
        Ok(())
    }

    #[test]
    fn construction_stretch_growth_scales_x_axis_like_cpp() -> Result<(), EngineError> {
        // Mirrors src/C4Def.cpp:387 and src/C4Object.cpp:329-333: DefCore
        // `StretchGrowth` sets `Def->GrowthType`, so UpdateShape calls
        // C4Shape::Stretch instead of Jolt. Stretch scales x/y/w/h and VtxX/VtxY
        // at src/C4Shape.cpp:105-116, then DoCon preserves the straight-object        // bottom at src/C4Object.cpp:1462-1468.
        //
        // Hand-derived golden: shape x=2,w=6,h=4 and vertex (8,4) at 50%
        // construction stretch to shape x=1,w=3,h=2 and vertex (4,2). The
        // spawn y 8 is the con-0 bottom, so the full-con center is
        // 8 - (4 + 0) = 4 (C4Object.cpp:1462-1468); the old bottom is
        // y 4 + shape.y 0 + h 4 = 8 and bottom preservation moves y to
        // 8 - 2 - 0 = 6.
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Stretch.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=STRG\nName=Stretch\nCategory=C4D_Object\nWidth=6\nHeight=4\nOffset=2,0\nVertices=1\nVertexX=8\nVertexY=4\nVertexCNAT=8\nStretchGrowth=1\n",
        )
        .expect("write defcore");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load resource definition");
        let definition = Definition::from_resource(&resource)?;

        let mut engine = Engine::with_seed(68);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("STRG")
                .with_position(Vector2::new(3, 8))
                .with_construction(FULL_CON),
        )?;

        engine.apply_object_update(id, ObjectUpdate::new().with_construction(FULL_CON / 2))?;

        let object = engine.object_snapshot(id).expect("object present");
        assert_eq!(object.construction, FULL_CON / 2);
        assert_eq!(object.position, Vector2::new(3, 6));
        assert_eq!(object.vertices[0].x, 4);
        assert_eq!(object.vertices[0].y, 2);
        assert_eq!(object.vertices[0].cnat, CNAT_BOTTOM);
        Ok(())
    }

    #[test]
    fn definition_stretch_growth_exposes_growth_type() -> Result<(), EngineError> {
        // DefCore `StretchGrowth` compiles into C4Def::GrowthType
        // (src/C4Def.cpp:387); the frontend needs it to pick the DrawFace
        // con-scaling mode (Stretch vs Jolt, src/C4Object.cpp:329-333 and
        // the growth-type target stretch at src/C4Object.cpp:442-460).
        let mut stretch = simple_definition("STRG");
        stretch.set_stretch_growth(true);
        let mut engine = Engine::with_seed(1);
        engine.register_definition(stretch)?;
        engine.register_definition(simple_definition("JOLT"))?;
        assert!(engine.definition_stretch_growth("STRG"));
        assert!(!engine.definition_stretch_growth("JOLT"));
        assert!(!engine.definition_stretch_growth("NONE"));
        Ok(())
    }

    #[test]
    fn border_bound_vertical_clamps_fixed_target_and_velocity() {
        let mut definition = simple_definition("Bounded");
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        definition.set_border_bound(C4D_BORDER_TOP | C4D_BORDER_BOTTOM);

        let mut engine = Engine::with_seed(43);
        engine.set_landscape(Landscape::flat(10, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let top_id = engine
            .spawn_object(
                SpawnConfig::new("Bounded")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 2)),
            )
            .expect("spawn succeeds");
        let top_idx = engine.find_object_index(top_id).expect("object exists");
        engine.objects[top_idx].set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, -itofix(5)));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[top_idx].state.mobile = true;

        let bottom_id = engine
            .spawn_object(
                SpawnConfig::new("Bounded")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(6, 18)),
            )
            .expect("spawn succeeds");
        let bottom_idx = engine.find_object_index(bottom_id).expect("object exists");
        engine.objects[bottom_idx].set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(5)));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[bottom_idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let top = snapshot.object(top_id).expect("top object present");
        let bottom = snapshot.object(bottom_id).expect("bottom object present");
        assert_eq!(top.position.y, 1);
        assert_eq!(bottom.position.y, 19);

        let top_idx = engine.find_object_index(top_id).expect("object exists");
        // TargetBounds clamps the INT step target only — both fixed
        // coordinates keep their momentum-advanced values.
        assert_eq!(engine.objects[top_idx].fixed_position.y, itofix(-3));
        assert_eq!(engine.objects[top_idx].fixed_velocity.y, C4Fixed::ZERO);
        let bottom_idx = engine.find_object_index(bottom_id).expect("object exists");
        assert_eq!(engine.objects[bottom_idx].fixed_position.y, itofix(23));
        assert_eq!(engine.objects[bottom_idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    #[test]
    fn attached_shape_checks_attachment_without_momentum_and_forces_jump_on_loss() {
        use std::sync::{Arc, Mutex};

        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func OnSlideAbort(state, action) { return 0; }
        global func OnJumpStart(state, action) { return 0; }
        "#;
        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                if name == "OnSlideAbort" || name == "OnJumpStart" {
                    call_log.lock().unwrap().push(name.to_string());
                }
            });
        }

        let mut definition =
            Definition::from_script("Climber", "Climber", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert(
            "Slide".to_string(),
            ActionSpec::default()
                .with_attach(CNAT_BOTTOM)
                .with_abort_call("OnSlideAbort"),
        );
        actions.insert(
            "Jump".to_string(),
            ActionSpec::default().with_start_call("OnJumpStart"),
        );
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(47);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Climber")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 5))
                    .with_action(ActionState::new("Slide")),
            )
            .expect("spawn succeeds");
        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Jump");
        assert_eq!(object.velocity, Vector2::ZERO);

        let calls = call_log.lock().unwrap().clone();
        // SetActionByName("Jump") fires the new StartCall before the old
        // AbortCall (C4Object.cpp:4172-4208).
        assert_eq!(
            calls,
            vec!["OnJumpStart".to_string(), "OnSlideAbort".to_string()]
        );
    }

    #[test]
    fn attached_shape_keeps_action_when_attachment_is_still_present_without_momentum() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Climber");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert(
            "Slide".to_string(),
            ActionSpec::default().with_attach(CNAT_BOTTOM),
        );
        actions.insert("Jump".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(49);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 7, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Climber")
                    .with_position(Vector2::new(5, 5))
                    .with_action(ActionState::new("Slide")),
            )
            .expect("spawn succeeds");
        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Slide");
        assert_eq!(object.position, Vector2::new(5, 5));
    }
