    #[test]
    fn shape_attach_record_preserves_cached_vehicle_material() {
        // C4Shape::Attach caches PixCol2Mat(GBackPix) in AttachMat together
        // with iAttachX/Y/Vtx (C4Shape.cpp:213-219,247-256). ShakeObjects
        // later distinguishes MVehic without resampling the landscape.
        let record = ShapeAttachRecord {
            mat_valid: true,
            mat_vehicle: true,
            x: 12,
            y: 34,
            vtx: 2,
        };

        let encoded = serde_json::to_string(&record).expect("attach record encodes");
        let decoded: ShapeAttachRecord =
            serde_json::from_str(&encoded).expect("attach record decodes");
        assert_eq!(decoded, record);

        let legacy: ShapeAttachRecord =
            serde_json::from_str(r#"{"mat_valid":true,"x":12,"y":34,"vtx":2}"#)
                .expect("pre-vehicle-bit attach record remains readable");
        assert!(!legacy.mat_vehicle);
    }

    #[test]
    fn shape_attach_caches_vehicle_material_at_the_probe() {
        // C4Shape::Attach stores PixCol2Mat(GBackPix(ax, ay)) at the exact
        // successful probe (C4Shape.cpp:213-219). A later ShakeObjects call
        // must therefore see MVehic even if the support has since changed.
        let library = MaterialLibrary::parse(
            r#"
            [Material Vehicle]
            Name=Vehicle
            Density=100
            Friction=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let vehicle = materials.id_of("Vehicle").expect("vehicle material exists");
        let mut engine = Engine::with_seed(1);
        engine.set_materials(materials);

        let mut definition =
            Definition::from_script("WALK", "Walker", "").expect("walker definition compiles");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([(
                "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            )]),
        );
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM)]);
        engine
            .register_definition(definition)
            .expect("walker definition registers");

        let mut pixels = vec![0_u8; 25];
        pixels[3 * 5 + 2] = 10;
        let mut densities = vec![0_i32; 128];
        densities[10] = 100;
        let mut names = vec![None; 128];
        names[10] = Some("Vehicle".to_string());
        let grid = landscape::PixelGrid::new(5, 5, pixels, densities, names, vec![None; 128]);
        let mut landscape = Landscape::new(5, vec![5; 5]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.border_material_at(2, 3)),
            Some(vehicle)
        );
        assert_eq!(
            engine
                .landscape()
                .map(|landscape| landscape.density_at(2, 3, engine.materials())),
            Some(100)
        );

        let walker = engine
            .spawn_object(
                SpawnConfig::new("WALK")
                    .with_position(Vector2::new(2, 2))
                    .with_action(ActionState::new("Walk"))
                    .with_category(CATEGORY_OBJECT)
                    .with_mobile(true),
            )
            .expect("walker spawns");
        let index = engine.find_object_index(walker).expect("walker exists");
        engine.tick_without_snapshot().expect("walker attachment executes");

        assert!(
            engine.objects[index].state.shape_attach.mat_valid,
            "attachment missing: pos={:?} vel={:?} mobile={} action={:?} t_attach={} vertices={:?}",
            engine.objects[index].state.position,
            engine.objects[index].state.velocity,
            engine.objects[index].state.mobile,
            engine.objects[index].state.action,
            engine.objects[index].state.t_attach,
            engine.objects[index].state.vertices
        );
        assert!(engine.objects[index].state.shape_attach.mat_vehicle);
    }

    #[test]
    fn direction_json_round_trip_preserves_raw_int32() {
        // C4Action::CompileFunc persists Action.Dir verbatim
        // (C4Action.cpp:45-54); save/snapshot JSON must do the same.
        let direction: Direction = serde_json::from_str("13").expect("direction decodes");
        assert_eq!(direction.to_script_value(), 13);
        assert_eq!(serde_json::to_string(&direction).expect("direction encodes"), "13");
    }

    #[test]
    fn command_direction_json_round_trip_preserves_raw_int32() {
        // C4Action::CompileFunc persists Action.ComDir verbatim
        // (C4Action.cpp:45-54); save/snapshot JSON must do the same.
        let direction: CommandDirection =
            serde_json::from_str("200").expect("command direction decodes");
        assert_eq!(direction.to_script_value(), 200);
        assert_eq!(
            serde_json::to_string(&direction).expect("command direction encodes"),
            "200"
        );
    }

    #[test]
    fn command_delta_preserves_raw_command_direction() {
        // C4Action::ComDir is a plain int32 assignment, including through
        // script-produced state updates (C4Script.cpp:792-796).
        let direction = value_to_command_direction("TEST", "Step", Value::Int(200))
            .expect("raw command direction converts");
        assert_eq!(direction.to_script_value(), 200);
    }

    #[test]
    fn raw_command_direction_skips_unmatched_movement_switch_like_cpp() {
        // C4Object::ExecAction's DFA_WALK switch has only COMD_* cases and
        // no default (C4Object.cpp:4785-4798), so a persisted raw value does
        // not accelerate or decelerate the object.
        let mut velocity = FixedVec2::new(itofix(2), itofix(3));
        apply_walk_physical_movement(
            &mut velocity,
            CommandDirection::from_raw(200),
            itofix(10),
        );
        assert_eq!(velocity, FixedVec2::new(itofix(2), itofix(3)));
        assert_eq!(CommandDirection::from_raw(200).axis_components(), (0, 0));
    }

    #[test]
    fn scale_attach_with_multidirection_dir_has_no_horizontal_side() {
        // DFA_SCALE independently tests Dir==DIR_Left and Dir==DIR_Right
        // before adding an attachment side (C4Object.cpp:4852-4853).
        let direction = Direction::from_script_value(8);
        let attach = procedure_t_attach(ActionProcedure::Scale, false, direction, 0, 0);
        assert_eq!(attach & (CNAT_LEFT | CNAT_RIGHT), 0);
    }
    use clonk_engine::math::C4Fixed;
    use clonk_engine::rng::LcgRng;
    use clonk_engine::scenario::{ClearObjectObjective, CreateObjectObjective, ScenarioObjectives};
    use clonk_resources::MaterialLibrary;
    use clonk_script::Value;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use tempfile::NamedTempFile;

    const STATEFUL_SCRIPT: &str = r#"#strict 3
    global func Initialize(state, random)
    {
        var vx = state.velocity[0] + (random % 5);
        var phase = random % 3;
        return {
            velocity = [vx, state.velocity[1]],
            energy = state.energy + (random % 7),
            action = { name = "Active", phase = phase }
        };
    }

    global func Step(state, frame, random)
    {
        var vx = state.velocity[0] + (random % 3) - 1;
        var energy = state.energy + (random % 5) - 2;
        if (energy < 0)
        {
            energy = 0;
        }
        return {
            velocity = [vx, state.velocity[1]],
            energy = energy
        };
    }
    "#;

    const BASIC_OBJECT_SCRIPT: &str = r#"
    global func Initialize(state, random) { return 0; }
    global func Step(state, frame, random) { return 0; }
    "#;

    #[test]
    fn blast_circle_emits_particles_for_blastable_materials() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            BlastFree=1
            Blast2PXSRatio=2
            SplashRate=15
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(7);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(17, 40, Some(earth)));

        let result = engine
            .blast_circle(Vector2::new(8, 40), 4, None)
            .expect("blast applies");
        let removed = result
            .removed_by_material
            .get(&earth)
            .copied()
            .unwrap_or_default();
        assert!(removed > 0, "expected blast to remove material");

        let snapshot = engine.snapshot();
        assert!(
            !snapshot.particles.is_empty(),
            "expected blast to emit particles"
        );
        assert_eq!(snapshot.particles[0].definition_id, "material/pxs/earth");
        assert_eq!(snapshot.particles[0].parameter_b, earth.index() as i32);
        Ok(())
    }

    #[test]
    fn blast_cast_amounts_derive_from_the_pre_blast_circle_count() -> Result<(), EngineError> {
        // C4Landscape::BlastFree computes cast amounts from BlastMatCount —
        // the PRE-removal in-circle pixel count (C4Landscape.cpp:1048-1055,
        // 1066-1079) — not from what was actually cleared. A material with
        // BlastFree=0 still casts BlastMatCount/Blast2PXSRatio particles.
        let library = MaterialLibrary::parse(
            r#"
            [Material Rock]
            Name=Rock
            Density=150
            Friction=100
            Blast2PXSRatio=2
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let rock = materials.id_of("Rock").expect("rock exists");
        let mut engine = Engine::with_seed(7);
        engine.set_materials(materials);
        let mut world = Landscape::flat_with_material(17, 40, Some(rock));
        world.set_world_height(80);
        engine.set_landscape(world);

        let result = engine
            .blast_circle(Vector2::new(8, 40), 4, None)
            .expect("blast applies");
        assert!(
            result.removed_by_material.is_empty(),
            "BlastFree=0 removes nothing"
        );
        let pre_count = result
            .pixel_count_by_material
            .get(&rock)
            .copied()
            .unwrap_or_default();
        assert_eq!(pre_count, 25, "solid half circle of r=4");
        assert_eq!(
            engine.pxs_system.count() as i32,
            pre_count / 2,
            "PXS.Cast(mat, BlastMatCount/Blast2PXSRatio) (C4Landscape.cpp:1075-1078)"
        );
        Ok(())
    }

    #[test]
    fn blast_circle_clears_only_the_exact_cpp_raster_pixels() {
        // C4Landscape::BlastFree walks the r=2 circle pixel-by-pixel, and
        // BlastFreePix calls ClearPix on each BlastFree material
        // (C4Landscape.cpp:958-978, 1022-1063). ClearPix preserves IFT as
        // Tunnel|IFT (C4Landscape.cpp:880-888); pixels outside that exact
        // scan must remain untouched.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            BlastFree=1

            [Material Tunnel]
            Name=Tunnel
            Density=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(7);
        engine.set_materials(materials);

        let mut densities = vec![0; 128];
        densities[10] = 100;
        let mut names = vec![None; 128];
        names[10] = Some("Earth".to_owned());
        names[20] = Some("Tunnel".to_owned());
        let mut bytes = vec![10; 7 * 7];
        bytes[3 * 7 + 3] |= 0x80;
        let grid = landscape::PixelGrid::new(7, 7, bytes, densities, names, vec![None; 128]);
        let mut world = Landscape::new(7, vec![0; 7]).expect("landscape builds");
        world.set_world_height(7);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let result = engine
            .blast_circle(Vector2::new(3, 3), 2, None)
            .expect("blast applies");

        assert_eq!(result.pixel_count_by_material.get(&earth), Some(&10));
        assert_eq!(result.removed_by_material.get(&earth), Some(&10));
        let landscape = engine.landscape().expect("landscape remains set");
        for (x, y) in [
            (3, 1),
            (2, 2),
            (3, 2),
            (1, 3),
            (2, 3),
            (4, 3),
            (2, 4),
            (3, 4),
            (3, 5),
        ] {
            assert_eq!(
                landscape.material_at(x, y),
                None,
                "BlastFree must clear in-circle pixel ({x}, {y})"
            );
        }
        assert_eq!(
            landscape.grid_byte_at(3, 3),
            Some(20 | 0x80),
            "ClearPix must retain the tunnel-background IFT byte"
        );
        assert_eq!(landscape.material_at(3, 0), Some(earth));
        assert_eq!(landscape.material_at(3, 6), Some(earth));
    }

    #[test]
    fn zero_radius_blast_clears_the_center_pixel() {
        // The inclusive C4Landscape::BlastFree loops execute once at rad=0
        // (C4Landscape.cpp:1028-1046), so BlastFreePix still clears center.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            BlastFree=1
        "#,
        )
        .expect("material library parses");
        let mut engine = Engine::with_seed(7);
        engine.set_materials(MaterialSet::from_resource_library(&library));

        let mut densities = vec![0; 2];
        densities[1] = 100;
        let names = vec![None, Some("Earth".to_owned())];
        let grid = landscape::PixelGrid::new(
            3,
            3,
            vec![1; 9],
            densities,
            names,
            vec![None; 2],
        );
        let mut world = Landscape::new(3, vec![0; 3]).expect("landscape builds");
        world.set_world_height(3);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        engine
            .blast_circle(Vector2::new(1, 1), 0, None)
            .expect("zero-radius blast applies");
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.material_at(1, 1)),
            None
        );
        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.material_at(1, 0)),
            engine.materials().id_of("Earth")
        );
    }

    #[test]
    fn invalid_sky_blast_shift_consumes_no_rng_and_keeps_the_pixel() {
        // CrossMap resolves BlastShiftTo through GetIndexMatTex
        // (C4Material.cpp:474-479). "Sky" is not a material, so resolution
        // returns byte 0 (C4Texture.cpp:361-368); BlastFreePix's byte gate
        // therefore skips both Random and SetPix (C4Landscape.cpp:947-953).
        let library = MaterialLibrary::parse(
            r#"
            [Material Granite]
            Name=Granite
            Density=100
            BlastShiftTo=Sky
        "#,
        )
        .expect("material library parses");
        let mut engine = Engine::with_seed(23);
        engine.set_materials(MaterialSet::from_resource_library(&library));

        let grid = landscape::PixelGrid::new(
            3,
            3,
            vec![1; 9],
            vec![0, 100],
            vec![None, Some("Granite".to_owned())],
            vec![None; 2],
        );
        let mut world = Landscape::new(3, vec![0; 3]).expect("landscape builds");
        world.set_world_height(3);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let rng_before = (engine.rng.count, engine.rng.hold, engine.rng.rnd3_ptr());
        engine
            .blast_circle(Vector2::new(1, 1), 0, None)
            .expect("zero-radius blast applies");

        assert_eq!(
            engine
                .landscape()
                .and_then(|landscape| landscape.grid_byte_at(1, 1)),
            Some(1),
            "unresolved BlastShiftTo byte 0 performs no landscape write"
        );
        assert_eq!(
            (engine.rng.count, engine.rng.hold, engine.rng.rnd3_ptr()),
            rng_before,
            "unresolved BlastShiftTo byte 0 performs no Random draw"
        );
    }

    fn free_rect_test_engine(
        width: u32,
        height: u32,
        bytes: Vec<u8>,
        densities: Vec<i32>,
        script_body: &str,
    ) -> Engine {
        let mut engine = Engine::with_seed(23);
        let slots = densities.len();
        let grid = landscape::PixelGrid::new(
            width,
            height,
            bytes,
            densities,
            vec![None; slots],
            vec![None; slots],
        );
        let mut world = Landscape::new(width, vec![height as i32; width as usize])
            .expect("FreeRect landscape builds");
        world.set_world_height(height as i32);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);
        engine
            .load_scenario_script_with_convention(
                "FreeRect density probe",
                &format!("#strict 2\nfunc Probe() {{ {script_body} }}"),
                true,
            )
            .expect("FreeRect probe loads");
        engine
    }

    #[test]
    fn free_rect_density_switch_bands_and_exact_density_match_cpp() {
        let cases = [
            (
                "C4M_Liquid",
                vec![1, 0, 0, 0, 5, 6, 7, 8, 9],
            ),
            (
                "C4M_Solid",
                vec![1, 2, 3, 4, 0, 0, 7, 8, 9],
            ),
            (
                "C4M_Vehicle",
                vec![1, 2, 3, 4, 5, 6, 0, 0, 9],
            ),
            ("37", vec![1, 2, 0, 4, 5, 6, 7, 8, 9]),
        ];
        for (density, expected) in cases {
            let mut engine = free_rect_test_engine(
                9,
                1,
                (1_u8..=9).collect(),
                vec![0, 24, 25, 37, 49, 50, 99, 100, 1000, 1001],
                &format!("FreeRect(0, 0, 9, 1, {density});"),
            );

            engine
                .call_scenario_script_function("Probe", Vec::new())
                .expect("density-filtered FreeRect runs");
            assert_eq!(
                engine.debug_landscape_plane().expect("pixel plane exists").2,
                expected,
                "FreeRect density selector {density}"
            );
        }
    }

    #[test]
    fn free_rect_omitted_and_zero_density_use_plain_clear_all() {
        let mut engine = free_rect_test_engine(
            3,
            2,
            vec![1, 2, 3, 1, 2, 3],
            vec![0, 25, 50, 100],
            "FreeRect(0, 0, 3, 1); FreeRect(0, 1, 3, 1, 0);",
        );

        engine
            .call_scenario_script_function("Probe", Vec::new())
            .expect("plain FreeRect forms run");
        assert_eq!(
            engine.debug_landscape_plane().expect("pixel plane exists").2,
            vec![0; 6]
        );
    }

    #[test]
    fn free_rect_density_advances_rnd3_for_every_zero_width_row() {
        let mut engine = free_rect_test_engine(
            1,
            20,
            vec![1; 20],
            vec![0, 25],
            "FreeRect(0, 0, 0, 20, C4M_Solid);",
        );
        let mut expected_rng = engine.rng.clone();
        let mut saw_zero_first_draw = false;
        let mut saw_nonzero_first_draw = false;
        for _ in 0..20 {
            if expected_rng.rnd3() != 0 {
                saw_nonzero_first_draw = true;
                expected_rng.rnd3();
            } else {
                saw_zero_first_draw = true;
            }
        }
        assert!(saw_zero_first_draw, "fixture covers the one-draw row arm");
        assert!(
            saw_nonzero_first_draw,
            "fixture covers the two-draw row arm"
        );

        engine
            .call_scenario_script_function("Probe", Vec::new())
            .expect("zero-width density FreeRect runs");

        assert_eq!(
            engine.debug_landscape_plane().expect("pixel plane exists").2,
            vec![1; 20]
        );
        assert_eq!(
            engine.rng, expected_rng,
            "every row consumes one Rnd3 and a second exactly when the first is nonzero"
        );
    }

    #[test]
    fn free_rect_is_visible_to_same_call_landscape_queries() {
        let mut engine = free_rect_test_engine(
            2,
            1,
            vec![1, 1],
            vec![0, 50],
            "if (GBackSolid(0, 0)) FreeRect(0, 0, 1, 1, C4M_Solid); \
             if (!GBackSolid(0, 0)) FreeRect(1, 0, 1, 1, C4M_Solid);",
        );
        assert_eq!(engine.debug_landscape_density(0, 0), Some(50));

        engine
            .call_scenario_script_function("Probe", Vec::new())
            .expect("same-call landscape query runs");
        assert_eq!(
            engine.debug_landscape_plane().expect("pixel plane exists").2,
            vec![0, 0],
            "the second clear proves GBackSolid saw the first synchronous clear"
        );
    }

    #[test]
    fn captured_blast_free_rng_advances_once_through_host_and_fold() {
        // BlastFree consumes its per-pixel Random calls before returning to
        // script. The authoritative fold must replay those captured choices
        // without drawing them a second time.
        let library = MaterialLibrary::parse(
            r#"
            [Material Granite]
            Name=Granite
            Density=110
            BlastShiftTo=Earth

            [Material Earth]
            Name=Earth
            Density=90
        "#,
        )
        .expect("blast materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let mut engine = Engine::with_seed(29);
        engine.set_materials(materials);
        let grid = landscape::PixelGrid::new(
            7,
            7,
            vec![1; 7 * 7],
            vec![0, 110, 90],
            vec![None, Some("Granite".to_owned()), Some("Earth".to_owned())],
            vec![None; 3],
        );
        let mut world = Landscape::new(7, vec![0; 7]).expect("blast landscape builds");
        world.set_world_height(7);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let probe = Definition::from_script(
            "BPRB",
            "Blast replay probe",
            r#"#strict 3
func Probe()
{
    BlastFree(3, 3, 2, 1);
    return Random(1000);
}
"#,
        )
        .expect("blast probe compiles");
        engine
            .register_definition(probe)
            .expect("blast probe registers");
        let probe = engine
            .spawn_object(SpawnConfig::new("BPRB"))
            .expect("blast probe spawns");

        // The complete r=2 raster scan contains ten Granite pixels. Each
        // BlastShiftTo source consumes Random(10), followed by the script's
        // own Random(1000).
        let mut expected_rng = engine.rng.clone();
        let mut expected_shifts = 0;
        for _ in 0..10 {
            if expected_rng.random(10) < 2 {
                expected_shifts += 1;
            }
        }
        assert!(expected_shifts > 0, "seed fixture exercises captured writes");
        let expected_tail = expected_rng.random(1000);

        let probe_index = engine.find_object_index(probe).expect("blast probe exists");
        assert_eq!(
            engine
                .call_object_function(probe_index, "Probe", Vec::new())
                .expect("BlastFree probe runs"),
            Value::Int(expected_tail),
            "the callback tail observes BlastFree's synchronous RNG position"
        );
        assert_eq!(
            engine.rng, expected_rng,
            "the authoritative replay consumes no duplicate blast draws"
        );
        assert_eq!(
            engine
                .debug_landscape_plane()
                .expect("blast raster remains")
                .2
                .iter()
                .filter(|byte| **byte == 2)
                .count(),
            expected_shifts,
            "the fold applies each captured BlastShiftTo choice once"
        );
    }

    #[test]
    fn shake_free_host_fold_creates_each_pxs_once() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
        "#,
        )
        .expect("shake material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("Earth exists");
        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        let grid = landscape::PixelGrid::new(
            5,
            5,
            vec![1; 5 * 5],
            vec![0, 80],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        );
        let mut world = Landscape::new(5, vec![0; 5]).expect("shake landscape builds");
        world.set_world_height(5);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let probe = Definition::from_script(
            "SPRB",
            "Shake replay probe",
            "#strict 3\nfunc Probe() { return ShakeFree(2, 2, 2); }",
        )
        .expect("shake probe compiles");
        engine
            .register_definition(probe)
            .expect("shake probe registers");
        let probe = engine
            .spawn_object(SpawnConfig::new("SPRB"))
            .expect("shake probe spawns");
        let probe_index = engine.find_object_index(probe).expect("shake probe exists");

        assert_eq!(
            engine
                .call_object_function(probe_index, "Probe", Vec::new())
                .expect("ShakeFree probe runs"),
            Value::Nil
        );
        assert_eq!(
            engine.pxs_system.count(),
            9,
            "the nine pixels in the C++ r=2 scan become PXS exactly once"
        );
        assert!(engine.pxs_system.iter().all(|pxs| pxs.mat == earth));
    }

    #[test]
    fn dig_free_rect_host_fold_credits_material_contents_once() {
        // Two fresh pixels per call and a conversion threshold of three make
        // duplicate host/fold credit observable: the first call must not cast,
        // while the second crosses the retained-content threshold exactly once.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=3
        "#,
        )
        .expect("dig material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        let grid = landscape::PixelGrid::new(
            4,
            1,
            vec![1; 4],
            vec![0, 80],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        );
        let mut world = Landscape::new(4, vec![1; 4]).expect("dig landscape builds");
        world.set_world_height(1);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let digger = Definition::from_script(
            "DGRR",
            "Digger",
            "#strict 3\nfunc Dig(int x) { return DigFreeRect(x, 0, 2, 1); }",
        )
        .expect("digger compiles");
        let gem = Definition::from_script("GEM_", "Gem", "#strict 3\n")
            .expect("gem compiles");
        engine
            .register_definition(digger)
            .expect("digger registers");
        engine.register_definition(gem).expect("gem registers");
        let digger = engine
            .spawn_object(SpawnConfig::new("DGRR"))
            .expect("digger spawns");
        let digger_index = engine.find_object_index(digger).expect("digger exists");

        assert_eq!(
            engine
                .call_object_function(digger_index, "Dig", vec![Value::Int(0)])
                .expect("first DigFreeRect runs"),
            Value::Nil
        );
        assert_eq!(
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == "GEM_" && !object.destroyed)
                .count(),
            0,
            "two pixels are credited once and remain below the ratio-three threshold"
        );

        let digger_index = engine.find_object_index(digger).expect("digger remains");
        assert_eq!(
            engine
                .call_object_function(digger_index, "Dig", vec![Value::Int(2)])
                .expect("second DigFreeRect runs"),
            Value::Nil
        );
        assert_eq!(
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == "GEM_" && !object.destroyed)
                .count(),
            1,
            "the second two-pixel credit crosses the threshold once"
        );
    }

    #[test]
    fn blast_free_runs_object_lifecycle_before_pxs_and_caller_rng() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            BlastFree=1
            Blast2Object=DEBR
            Blast2ObjectRatio=1
            Blast2PXSRatio=1
        "#,
        )
        .expect("blast material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("Earth exists");
        let mut engine = Engine::with_seed(41);
        engine.set_materials(materials);
        let grid = landscape::PixelGrid::new(
            1,
            1,
            vec![1],
            vec![0, 80],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        );
        let mut world = Landscape::new(1, vec![1]).expect("blast landscape builds");
        world.set_world_height(1);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let probe = Definition::from_script(
            "BPR2",
            "Blast lifecycle probe",
            r#"#strict 3
func Probe()
{
    BlastFree(0, 0, 0, 8);
    var debris = FindObject(DEBR);
    return [!!debris, debris->Read(), GetController(debris), Random(1000)];
}
"#,
        )
        .expect("blast probe compiles");
        let mut debris = Definition::from_script(
            "DEBR",
            "Debris",
            r#"#strict 3
local construction_random;
func Construction() { construction_random = Random(1000); }
func Read() { return construction_random; }
"#,
        )
        .expect("debris compiles");
        debris.set_rotateable(1);
        engine
            .register_definition(probe)
            .expect("blast probe registers");
        engine
            .register_definition(debris)
            .expect("debris registers");
        let probe = engine
            .spawn_object(SpawnConfig::new("BPR2"))
            .expect("blast probe spawns");

        let mut expected_rng = engine.rng.clone();
        let _rotation_velocity = expected_rng.random(3);
        let _ydir = expected_rng.random(61);
        let _xdir = expected_rng.random(61);
        let _rotation = expected_rng.random(360);
        let expected_construction = expected_rng.random(1_000);
        let _pxs_ydir = expected_rng.random(61);
        let _pxs_xdir = expected_rng.random(61);
        let expected_tail = expected_rng.random(1_000);

        let probe_index = engine.find_object_index(probe).expect("blast probe exists");
        assert_eq!(
            engine
                .call_object_function(probe_index, "Probe", Vec::new())
                .expect("BlastFree lifecycle probe runs"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(expected_construction),
                Value::Int(7),
                Value::Int(expected_tail),
            ]),
            "Blast2Object is findable after Construction and before the caller resumes"
        );
        assert_eq!(engine.rng, expected_rng, "every blast draw runs exactly once");
        assert_eq!(engine.pxs_system.count(), 1);
        assert!(engine.pxs_system.iter().all(|pxs| pxs.mat == earth));
        assert_eq!(
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == "DEBR" && !object.destroyed)
                .count(),
            1
        );
        assert_eq!(
            engine
                .debug_landscape_plane()
                .expect("blast raster remains")
                .2,
            vec![0],
            "the captured terrain write folds once"
        );
    }

    #[test]
    fn dig_free_recomputes_creator_geometry_between_material_lifecycles() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEMA
            Dig2ObjectRatio=1

            [Material Rock]
            Name=Rock
            Density=100
            DigFree=1
            Dig2Object=GEMB
            Dig2ObjectRatio=1
        "#,
        )
        .expect("dig materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let mut engine = Engine::with_seed(43);
        engine.set_materials(materials);
        let grid = landscape::PixelGrid::new(
            2,
            1,
            vec![1, 2],
            vec![0, 80, 100],
            vec![None, Some("Earth".to_owned()), Some("Rock".to_owned())],
            vec![None; 3],
        );
        let mut world = Landscape::new(2, vec![1; 2]).expect("dig landscape builds");
        world.set_world_height(1);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let mut digger = Definition::from_script(
            "DGR2",
            "Digger",
            r#"#strict 3
func Probe()
{
    DigFreeRect(0, 0, 2, 1);
    return [FindObject(GEMA)->Read(), FindObject(GEMB)->Read(), Random(1000)];
}
"#,
        )
        .expect("digger compiles");
        digger.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 2)));
        let first = Definition::from_script(
            "GEMA",
            "First gem",
            r#"#strict 3
local construction_random;
func Construction(object creator)
{
    construction_random = Random(1000);
    SetPosition(10, 20, creator);
    SetShape(-1, 3, 4, 7, creator);
}
func Read() { return construction_random; }
"#,
        )
        .expect("first gem compiles");
        let second = Definition::from_script(
            "GEMB",
            "Second gem",
            r#"#strict 3
local construction_random;
func Construction() { construction_random = Random(1000); }
func Read() { return construction_random; }
"#,
        )
        .expect("second gem compiles");
        engine.register_definition(digger).expect("digger registers");
        engine.register_definition(first).expect("first gem registers");
        engine
            .register_definition(second)
            .expect("second gem registers");
        let digger = engine
            .spawn_object(SpawnConfig::new("DGR2"))
            .expect("digger spawns");

        let mut expected_rng = engine.rng.clone();
        let _first_rotation = expected_rng.random(360);
        let expected_first_construction = expected_rng.random(1_000);
        let _second_rotation = expected_rng.random(360);
        let expected_second_construction = expected_rng.random(1_000);
        let expected_tail = expected_rng.random(1_000);

        let digger_index = engine.find_object_index(digger).expect("digger exists");
        assert_eq!(
            engine
                .call_object_function(digger_index, "Probe", Vec::new())
                .expect("DigFreeRect lifecycle probe runs"),
            Value::Array(vec![
                Value::Int(expected_first_construction),
                Value::Int(expected_second_construction),
                Value::Int(expected_tail),
            ])
        );
        assert_eq!(engine.rng, expected_rng, "dig lifecycle draws run once");
        let first = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "GEMA")
            .expect("first gem exists");
        let second = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "GEMB")
            .expect("second gem exists");
        assert_eq!(
            first.state.position,
            Vector2::ZERO,
            "initial NewObject growth preserves the raw y=0 shape bottom"
        );
        assert_eq!(
            second.state.position,
            Vector2::new(10, 30),
            "the second cast observes the first Construction's move and shape write"
        );
    }

    #[test]
    fn sequential_effect_callbacks_share_dig_material_contents() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=3
        "#,
        )
        .expect("dig material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let mut engine = Engine::with_seed(47);
        engine.set_materials(materials);
        let grid = landscape::PixelGrid::new(
            4,
            1,
            vec![1; 4],
            vec![0, 80],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        );
        let mut world = Landscape::new(4, vec![1; 4]).expect("dig landscape builds");
        world.set_world_height(1);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let mut digger = Definition::from_script(
            "DFX2",
            "Effect digger",
            r#"#strict 3
func Arm()
{
    AddEffect("First", this(), 200, 1, this());
    AddEffect("Second", this(), 100, 1, this());
}
func FxFirstTimer() { DigFreeRect(0, 0, 2, 1); return 0; }
func FxSecondTimer() { DigFreeRect(2, 0, 2, 1); return 0; }
"#,
        )
        .expect("effect digger compiles");
        digger.set_c4_callback_convention(true);
        engine.register_definition(digger).expect("digger registers");
        engine
            .register_definition(simple_definition("GEM_"))
            .expect("gem registers");
        let digger = engine
            .spawn_object(SpawnConfig::new("DFX2"))
            .expect("digger spawns");
        let digger_index = engine.find_object_index(digger).expect("digger exists");
        engine
            .call_object_function(digger_index, "Arm", Vec::new())
            .expect("effects arm");

        engine.frame = 4;
        engine
            .tick_without_snapshot()
            .expect("shared-content timer frame succeeds");
        assert_eq!(
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == "GEM_" && !object.destroyed)
                .count(),
            1,
            "two callbacks in one effect batch accumulate 2 + 2 before conversion"
        );
    }

    #[test]
    fn construction_and_initialize_share_dig_material_contents_before_insertion() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=2
        "#,
        )
        .expect("dig material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let mut engine = Engine::with_seed(53);
        engine.set_materials(materials);
        let grid = landscape::PixelGrid::new(
            2,
            1,
            vec![1; 2],
            vec![0, 80],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        );
        let mut world = Landscape::new(2, vec![1; 2]).expect("dig landscape builds");
        world.set_world_height(1);
        world.set_pixel_grid(grid);
        engine.set_landscape(world);

        let mut digger = Definition::from_script(
            "DINI",
            "Lifecycle digger",
            r#"#strict 3
func Construction() { DigFreeRect(0, 0, 1, 1); }
func Initialize() { DigFreeRect(1, 0, 1, 1); }
"#,
        )
        .expect("lifecycle digger compiles");
        digger.set_c4_callback_convention(true);
        engine.register_definition(digger).expect("digger registers");
        let mut gem = Definition::from_script(
            "GEM_",
            "Lifecycle gem",
            r#"#strict 3
local creator_found;
func Construction(object creator)
{
    creator_found = FindObject(DINI) == creator;
}
"#,
        )
        .expect("gem compiles");
        gem.set_c4_callback_convention(true);
        engine
            .register_definition(gem)
            .expect("gem registers");

        engine
            .spawn_object(SpawnConfig::new("DINI").with_loaded(true))
            .expect("existing lifecycle digger loads without callbacks");
        let digger = engine
            .spawn_object(SpawnConfig::new("DINI"))
            .expect("lifecycle digger spawns");
        assert!(engine.find_object_index(digger).is_some());
        let gem = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "GEM_" && !object.destroyed)
            .expect("conversion gem exists");
        assert_eq!(
            gem.state.local_vars.get("creator_found"),
            Some(&Value::Bool(true)),
            "a nested Dig2Object Construction sees its pending creator in the C++ master list"
        );
        assert_eq!(
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == "GEM_" && !object.destroyed)
                .count(),
            1,
            "Initialize inherits Construction's pre-insertion material credit"
        );
    }

    fn free_rect_mask_test_engine(script_body: &str) -> (Engine, ObjectId, MaterialId) {
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
        .expect("FreeRect mask materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let vehicle = materials.id_of("Vehicle").expect("Vehicle exists");
        let mut bytes = vec![0u8; 20 * 20];
        bytes[10 * 20 + 10] = 1;
        bytes[10 * 20 + 11] = 1 | 0x80;
        let grid = landscape::PixelGrid::new(
            20,
            20,
            bytes,
            vec![0, 100, 100],
            vec![None, Some("Earth".into()), Some("Vehicle".into())],
            vec![None; 3],
        );
        let mut world = Landscape::new(20, vec![0; 20]).expect("landscape builds");
        world.set_world_height(20);
        world.set_pixel_grid(grid);

        let mut mask = Definition::from_script(
            "MASK",
            "FreeRect mask",
            &format!("#strict 2\npublic func Probe() {{ {script_body} }}"),
        )
        .expect("mask script compiles");
        mask.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 1)));
        mask.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 2, 1, 0, 0)));

        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        engine.set_landscape(world);
        engine.register_definition(mask).expect("mask registers");
        let id = engine
            .spawn_object(SpawnConfig::new("MASK").with_position(Vector2::new(10, 10)))
            .expect("mask spawns");
        let index = engine.find_object_index(id).expect("mask exists");
        engine.objects[index].state.position = Vector2::new(10, 10);
        engine.objects[index].fixed_position = FixedVec2::from_ints(10, 10);
        engine.update_solid_mask(index);
        assert_eq!(
            engine
                .debug_solid_mask_buffer(id.as_u64())
                .expect("mask is baked"),
            vec![1, 1 | 0x80]
        );
        assert_eq!(
            engine.debug_landscape_plane().expect("pixel plane exists").2
                [10 * 20 + 10..10 * 20 + 12],
            [2, 2]
        );
        (engine, id, vehicle)
    }

    #[test]
    fn free_rect_plain_clear_repairs_mask_and_updates_saved_background() {
        let (mut engine, mask, vehicle) = free_rect_mask_test_engine(
            "FreeRect(10, 10, 2, 1); return [GBackSolid(0, 0), \
             GBackSolid(1, 0), GetMaterial(0, 0), GetMaterial(1, 0)];",
        );
        let mut expected_rng = engine.rng.clone();
        if expected_rng.rnd3() != 0 {
            expected_rng.rnd3();
        }
        let probe_index = engine.find_object_index(mask).expect("mask remains");
        let result = engine
            .call_object_function(probe_index, "Probe", Vec::new())
            .expect("plain masked FreeRect runs");

        assert_eq!(
            result,
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(true),
                Value::Int(vehicle.index() as i32),
                Value::Int(vehicle.index() as i32),
            ]),
            "same-call reads see FinishChange's repaired Vehicle mask"
        );
        assert_eq!(engine.rng, expected_rng, "one source-row Rnd3 arm");
        assert_eq!(engine.pxs_system.count(), 0);

        let index = engine.find_object_index(mask).expect("mask remains");
        assert_eq!(
            engine.debug_landscape_plane().expect("pixel plane exists").2
                [10 * 20 + 10..10 * 20 + 12],
            [2, 2],
            "the authoritative fold also re-puts Vehicle"
        );
        assert_eq!(
            engine
                .debug_solid_mask_buffer(mask.as_u64())
                .expect("mask remains baked"),
            vec![0, 0x80],
            "Repair saves the cleared sky/IFT background"
        );

        engine.remove_solid_mask(index);
        engine.objects[index].state.position = Vector2::new(15, 15);
        engine.objects[index].fixed_position = FixedVec2::from_ints(15, 15);
        engine.update_solid_mask(index);
        assert_eq!(
            engine.debug_landscape_plane().expect("pixel plane exists").2
                [10 * 20 + 10..10 * 20 + 12],
            [0, 0x80],
            "moving the owner uncovers the cleared background"
        );
    }

    #[test]
    fn free_rect_density_over_mask_stays_raw() {
        let (mut engine, mask, _vehicle) = free_rect_mask_test_engine(
            "FreeRect(10, 10, 2, 1, C4M_Vehicle); return [GBackSolid(0, 0), \
             GBackSolid(1, 0), GetMaterial(0, 0), GetMaterial(1, 0)];",
        );
        let mut expected_rng = engine.rng.clone();
        if expected_rng.rnd3() != 0 {
            expected_rng.rnd3();
        }
        let probe_index = engine.find_object_index(mask).expect("mask remains");
        let result = engine
            .call_object_function(probe_index, "Probe", Vec::new())
            .expect("density masked FreeRect runs");

        assert_eq!(
            result,
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Int(-1),
                Value::Int(-1),
            ]),
            "ClearRectDensity reads and clears the raw Vehicle bytes"
        );
        assert_eq!(engine.rng, expected_rng, "one source-row Rnd3 arm");

        let index = engine.find_object_index(mask).expect("mask remains");
        assert_eq!(
            engine.debug_landscape_plane().expect("pixel plane exists").2
                [10 * 20 + 10..10 * 20 + 12],
            [0, 0]
        );
        assert_eq!(
            engine
                .debug_solid_mask_buffer(mask.as_u64())
                .expect("stale bake remains attached"),
            vec![1, 1 | 0x80],
            "density clear has no PrepareChange/Repair bracket"
        );
        engine.remove_solid_mask(index);
        assert_eq!(
            engine.debug_landscape_plane().expect("pixel plane exists").2
                [10 * 20 + 10..10 * 20 + 12],
            [0, 0],
            "raw-cleared mask pixels are not restored later"
        );
    }

    #[test]
    fn free_rect_column_fallback_ignores_dig_free_and_clears_liquids() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Granite]
            Name=Granite
            Density=80
            DigFree=0

            [Material Water]
            Name=Water
            Density=25
            DigFree=0
        "#,
        )
        .expect("FreeRect column materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let granite = materials.id_of("Granite").expect("Granite exists");
        let water = materials.id_of("Water").expect("Water exists");
        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        let mut world = Landscape::flat_with_material(4, 5, Some(granite));
        world.set_world_height(10);
        world.set_solid_material(2, Some(water));
        world.set_liquid_column(
            3,
            vec![LiquidSegment::with_material(2, 4, Some(water))],
        );
        engine.set_landscape(world);
        engine
            .load_scenario_script_with_convention(
                "FreeRect column probe",
                "#strict 2\nfunc Probe() {\n\
                 FreeRect(0, 5, 1, 3, C4M_Solid);\n\
                 FreeRect(1, 5, 1, 3);\n\
                 FreeRect(2, 5, 1, 3, C4M_Solid);\n\
                 FreeRect(3, 2, 1, 2, C4M_Liquid);\n\
                 }",
                true,
            )
            .expect("FreeRect column probe loads");

        engine
            .call_scenario_script_function("Probe", Vec::new())
            .expect("FreeRect column probe runs");
        let landscape = engine.landscape().expect("column landscape remains");
        assert_eq!(
            landscape.surface(),
            [8, 8, 5, 5],
            "both clear arms ignore Material.DigFree while density filtering survives"
        );
        assert_eq!(landscape.liquid_material_at(3, 2), None);
        assert_eq!(landscape.liquid_material_at(3, 3), None);
        assert_eq!(landscape.liquid_material_at(3, 4), Some(water));
    }

    #[test]
    fn free_rect_consumes_rnd3_before_next_same_call_native() {
        let mut source = Definition::from_script(
            "FRNG",
            "FreeRect RNG source",
            "#strict 2\npublic func Probe() {\n\
             FreeRect(0, 0, 0, 1, C4M_Solid);\n\
             return Split2Components();\n\
             }",
        )
        .expect("FreeRect RNG source compiles");
        source.set_components(vec![DefinitionComponent {
            id: "PART".to_string(),
            count: 1,
        }]);
        let mut part =
            Definition::from_script("PART", "FreeRect RNG part", "").expect("PART compiles");
        part.set_rotateable(1);

        let mut engine = Engine::with_seed(2);
        engine.register_definition(source).expect("source registers");
        engine.register_definition(part).expect("PART registers");
        engine.set_landscape(Landscape::flat(1, 1));
        let source = engine
            .spawn_object(SpawnConfig::new("FRNG"))
            .expect("source spawns");

        let mut expected_rng = engine.rng.clone();
        if expected_rng.rnd3() != 0 {
            expected_rng.rnd3();
        }
        let expected_rdir = expected_rng.rnd3();
        let expected_ydir = expected_rng.rnd3();
        let expected_xdir = expected_rng.rnd3();
        let expected_rotation = expected_rng.random(360);
        assert_eq!(
            (expected_rdir, expected_ydir, expected_xdir),
            (0, 1, -1),
            "seed fixture distinguishes synchronous from deferred ordering"
        );

        let source_index = engine.find_object_index(source).expect("source exists");
        assert_eq!(
            engine
                .call_object_function(source_index, "Probe", Vec::new())
                .expect("FreeRect then Split2Components runs"),
            Value::Bool(true)
        );
        let piece = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "PART")
            .expect("component piece exists");
        assert_eq!(
            piece.fixed_velocity,
            FixedVec2::from_ints(expected_xdir, expected_ydir)
        );
        assert_eq!(piece.rotation_velocity, itofix(expected_rdir));
        assert_eq!(piece.state.rotation, expected_rotation);
        assert_eq!(engine.rng.rnd3_ptr(), 5, "FreeRect draws are not replayed");
        assert_eq!(engine.rng, expected_rng);
    }

    #[test]
    fn blast_cast_fan_out_matches_the_cpp_evaluate_loop() -> Result<(), EngineError> {
        // C4Landscape::BlastFree evaluate loop (C4Landscape.cpp:1065-1079):
        // materials in INDEX order; within a material BlastCastObjects runs
        // BEFORE PXS.Cast. BlastCastObjects (C4Game.cpp:1723-1735) draws 4
        // per object in argument-evaluation order — rdir = itofix(Random(3)
        // + 1), ydir = FIXED10(Random(61) - 40), xdir = FIXED10(Random(61)
        // - 30), angle = Random(360) — creating each object INLINE with
        // owner NO_OWNER and the blast controller.
        let library = MaterialLibrary::parse(
            r#"
            [Material Dust]
            Name=Dust
            Density=100
            Friction=25
            BlastFree=1
            Blast2PXSRatio=5

            [Material Ruby]
            Name=Ruby
            Density=120
            Friction=40
            BlastFree=1
            Blast2Object=GEM0
            Blast2ObjectRatio=4
            Blast2PXSRatio=5
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let dust = materials.id_of("Dust").expect("dust exists");
        let ruby = materials.id_of("Ruby").expect("ruby exists");
        let mut engine = Engine::with_seed(13);
        engine.set_materials(materials);
        // Script-less rotateable def: no callback draws disturb the stream.
        let mut gem = Definition::from_script("GEM0", "GEM0", "").expect("compiles");
        gem.set_rotateable(1);
        engine.register_definition(gem).expect("gem registers");
        let mut world = Landscape::flat_with_material(17, 40, Some(dust));
        world.set_world_height(80);
        for column in 9..17 {
            world.set_solid_material(column, Some(ruby));
        }
        engine.set_landscape(world);

        let mut mirror = engine.rng.clone();
        let controller = 3;
        let result = engine
            .blast_circle(Vector2::new(8, 40), 4, Some(controller))
            .expect("blast applies");
        // Pre-blast counts: dust x∈4..=8 → 17, ruby x∈9..=11 → 8.
        assert_eq!(result.pixel_count_by_material.get(&dust), Some(&17));
        assert_eq!(result.pixel_count_by_material.get(&ruby), Some(&8));

        // Dust (index 0): no Blast2Object → PXS.Cast(dust, 17/5 = 3, …, 60)
        // draws Random(61) twice per particle (C4PXS.cpp:309-322).
        for _ in 0..3 {
            mirror.random(61);
            mirror.random(61);
        }
        // Ruby (index 1): 8/4 = 2 objects FIRST, 4 draws each…
        let mut expected_objects = Vec::new();
        for _ in 0..2 {
            let r4 = mirror.random(3);
            let r3 = mirror.random(61);
            let r2 = mirror.random(61);
            let r1 = mirror.random(360);
            expected_objects.push((r1, r2, r3, r4));
        }
        // …then PXS.Cast(ruby, 8/5 = 1, …).
        mirror.random(61);
        mirror.random(61);
        assert_eq!(engine.rng, mirror, "synced draw stream matches C++");

        let gems: Vec<&Object> = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "GEM0")
            .collect();
        assert_eq!(gems.len(), 2);
        for (object, (r1, r2, r3, r4)) in gems.iter().zip(expected_objects) {
            assert_eq!(object.state.rotation, r1.rem_euclid(360));
            assert_eq!(object.fixed_velocity.x, math::fixed10(r2 - 30), "xdir");
            assert_eq!(object.fixed_velocity.y, math::fixed10(r3 - 40), "ydir");
            assert_eq!(object.rotation_velocity, math::itofix(r4 + 1), "rdir");
            assert_eq!(object.state.owner, OWNER_NONE, "CreateObject NO_OWNER");
            assert_eq!(object.state.controller, controller, "iByPlayer");
        }
        assert_eq!(engine.pxs_system.count(), 4, "3 dust + 1 ruby particles");
        Ok(())
    }

    #[test]
    fn blast_object_cast_consumes_draws_for_unknown_definitions() -> Result<(), EngineError> {
        // C4Game::BlastCastObjects evaluates the 4 Random draws as call
        // ARGUMENTS before CreateObject's C4Id2Def check (C4Game.cpp:
        // 1726-1733, 1142-1148): an unloaded id spawns nothing but the
        // stream advances — and the following PXS cast still lines up.
        let library = MaterialLibrary::parse(
            r#"
            [Material Emerald]
            Name=Emerald
            Density=120
            Friction=40
            BlastFree=1
            Blast2Object=MISS
            Blast2ObjectRatio=4
            Blast2PXSRatio=5
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let emerald = materials.id_of("Emerald").expect("emerald exists");
        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        let mut world = Landscape::flat_with_material(17, 40, Some(emerald));
        world.set_world_height(80);
        engine.set_landscape(world);

        let mut mirror = engine.rng.clone();
        let result = engine
            .blast_circle(Vector2::new(8, 40), 4, None)
            .expect("blast applies");
        assert_eq!(result.pixel_count_by_material.get(&emerald), Some(&25));
        // 25/4 = 6 objects worth of draws, definition never loaded…
        for _ in 0..6 {
            mirror.random(3);
            mirror.random(61);
            mirror.random(61);
            mirror.random(360);
        }
        // …then PXS.Cast(emerald, 25/5 = 5).
        for _ in 0..5 {
            mirror.random(61);
            mirror.random(61);
        }
        assert_eq!(engine.rng, mirror, "unknown-def draws are consumed");
        assert!(
            engine.objects.is_empty(),
            "C4Id2Def null spawns no objects"
        );
        assert_eq!(engine.pxs_system.count(), 5);
        Ok(())
    }

    #[test]
    fn set_landscape_resolves_pixel_grid_materials_like_update_pix_maps() {
        // UpdatePixMaps fills Pix2Mat by resolving each texmap entry's
        // material NAME against the loaded material map
        // (C4Landscape.cpp:2832-2839 + C4TextureMap::GetEntry) — the grid
        // carries names from TexMap.txt until the engine MaterialSet
        // exists; set_landscape is where ids resolve so GBackMat
        // (C4Wrappers.h:120-129) answers per pixel.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Water]
            Name=Water
            Density=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(3);
        engine.set_materials(materials);

        let mut densities = vec![0i32; 128];
        densities[20] = 25;
        densities[30] = 100;
        let mut names: Vec<Option<String>> = vec![None; 128];
        names[20] = Some("Water".into());
        names[30] = Some("Earth".into());
        // 2x2 world: water in the left column, earth in the right.
        let grid = landscape::PixelGrid::new(2, 2, vec![20, 30, 20, 30], densities, names, vec![None; 128]);
        let mut landscape = Landscape::new(2, vec![0, 0]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let landscape = engine.landscape().expect("landscape set");
        assert_eq!(
            landscape.material_at(0, 0),
            Some(water),
            "Pix2Mat resolves liquid pixels to the engine material id"
        );
        assert_eq!(
            landscape.material_at(1, 1),
            Some(earth),
            "Pix2Mat resolves solid pixels to the engine material id"
        );
    }

    #[test]
    fn set_landscape_resolves_the_vehicle_border_material_like_mvehic() {
        // MVehic = Material.Get("Vehicle") (C4Game::InitMaterialTexture,
        // C4Game.cpp:1669); GetPix's closed borders read MCVehic which
        // GBackMat maps back to that material (C4Landscape.h:144-161,
        // 173-176).
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
        let mut engine = Engine::with_seed(1);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(10, 5, Some(earth)));
        let landscape = engine.landscape().expect("landscape set");
        assert_eq!(
            landscape.border_material_at(-1, 3),
            Some(vehicle),
            "closed side reads the Vehicle material"
        );
        assert_eq!(landscape.border_material_at(4, -1), None, "top open");
    }

    #[test]
    fn blast_circle_spawns_objects_for_material_reactions() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Rock]
            Name=Rock
            Density=110
            Friction=35
            BlastFree=1
            Blast2Object=GEM0
            Blast2ObjectRatio=2
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let rock = materials.id_of("Rock").expect("rock exists");
        let mut engine = Engine::with_seed(11);
        engine.set_materials(materials);
        engine
            .register_definition(simple_definition("GEM0"))
            .expect("gem definition registers");
        engine.set_landscape(Landscape::flat_with_material(17, 40, Some(rock)));

        let controller = 1;
        let before_snapshot = engine.snapshot();
        let existing_ids: HashSet<_> = before_snapshot
            .objects
            .iter()
            .map(|object| object.id)
            .collect();
        let result = engine
            .blast_circle(Vector2::new(8, 40), 4, Some(controller))
            .expect("blast applies");
        let pre_count = result
            .pixel_count_by_material
            .get(&rock)
            .copied()
            .unwrap_or_default();
        assert!(pre_count > 0, "expected in-circle rock pixels");

        let ratio = 2;
        let expected_spawns = pre_count / ratio;
        assert!(
            expected_spawns > 0,
            "expected blast to spawn objects for the counted material"
        );
        let after_snapshot = engine.snapshot();
        let new_objects: Vec<_> = after_snapshot
            .objects
            .iter()
            .filter(|object| !existing_ids.contains(&object.id))
            .collect();
        assert_eq!(
            new_objects.len() as i32,
            expected_spawns,
            "blast should spawn one object per {:?} counted pixels",
            ratio
        );

        for object in new_objects {
            assert_eq!(
                object.definition_id, "GEM0",
                "blast should spawn configured definition"
            );
            // FIXED10(Random(61)-30) / FIXED10(Random(61)-40)
            // (C4Game.cpp:1730-1731): ±3.0 / -4.0..+2.0 as integers.
            assert!(
                (-3..=3).contains(&object.velocity.x),
                "expected horizontal velocity to follow the FIXED10 range"
            );
            assert!(
                (-4..=2).contains(&object.velocity.y),
                "expected vertical velocity to follow the FIXED10 range"
            );
            assert!(
                (0..360).contains(&object.rotation),
                "expected rotation to be normalised"
            );
            // CreateObject(id, nullptr, NO_OWNER, …, iByPlayer)
            // (C4Game.cpp:1733): the blast controller is the CONTROLLER,
            // not the owner.
            assert_eq!(object.owner, OWNER_NONE, "owner is NO_OWNER");
            assert_eq!(
                object.controller, controller,
                "controller carries the blasting player"
            );
        }
        Ok(())
    }

    #[test]
    fn apply_landscape_operations_executes_blast_circle() -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
            BlastFree=1
            Blast2PXSRatio=2
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(5);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(17, 40, Some(earth)));

        engine.apply_landscape_operations(vec![LandscapeOperation::BlastCircle {
            center: Vector2::new(8, 40),
            radius: 4,
            controller: Some(1),
        }]);

        let snapshot = engine.snapshot();
        assert!(
            snapshot
                .particles
                .iter()
                .any(|particle| particle.definition_id == "material/pxs/earth"),
            "blast operation should emit earth particles"
        );
        Ok(())
    }

    #[test]
    fn apply_landscape_operations_extracts_one_liquid_pixel() {
        // FnExtractLiquid's deferred half is the real
        // C4Landscape::ExtractMaterial mutation (C4Script.cpp:2194-2199;
        // C4Landscape.cpp:1130-1156).
        let library = MaterialLibrary::parse("[Material Water]\nName=Water\nDensity=25\n")
            .expect("water material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let mut landscape = Landscape::flat(12, 20);
        landscape.set_liquid_column(
            5,
            vec![LiquidSegment {
                top: 10,
                bottom: 14,
                material: Some(water),
            }],
        );
        let mut engine = Engine::with_seed(1);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        assert!(engine.debug_landscape_is_liquid(5, 11));

        engine.apply_landscape_operations(vec![LandscapeOperation::ExtractLiquid {
            position: Vector2::new(5, 11),
        }]);

        assert!(!engine.debug_landscape_is_liquid(5, 10));
        assert!(
            engine.debug_landscape_is_liquid(5, 11),
            "ExtractMaterial's FindMatTop clears the surface, not the probed interior pixel"
        );
    }

    #[test]
    fn cross_check_hit_damages_and_flings_like_cpp() -> Result<(), EngineError> {
        // CrossCheck reverse area check, Hit branch (C4GameObjects.cpp:148,
        // 167-184): an OCF_HitSpeed2 object of category C4D_Object overlapping
        // an alive object deals "realistic" hit energy
        // fixtoi((dX²+dY²)*Mass/5), reduced to 1/3 (min 1); the victim takes
        // DoEnergy(-e/5) and is flung by (xdir*50/tmass, -|ydir/2|*50/tmass)
        // with tmass = max(victim mass, 50).
        let mut engine = Engine::with_seed(40);
        let mut victim_def = simple_definition("Clonk");
        victim_def.set_mass(100);
        victim_def.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        // Hit victims are livings: the Hit branch needs OCF_Alive, which
        // needs Category & C4D_Living (SetOCF, C4Object.cpp:600-605).
        victim_def.set_category(CATEGORY_LIVING);
        engine.register_definition(victim_def)?;
        let mut rock_def = simple_definition("Rock");
        rock_def.set_category(CATEGORY_OBJECT);
        rock_def.set_mass(50);
        engine.register_definition(rock_def)?;

        let victim = engine.spawn_object(
            SpawnConfig::new("Clonk")
                .with_position(Vector2::new(50, 50))
                .with_alive(true)
                .with_energy(100_000),
        )?;
        let _rock = engine.spawn_object(
            SpawnConfig::new("Rock")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(5, 0)),
        )?;

        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        let energy_before = engine.objects[victim_idx].state.energy;
        engine.cross_check(1)?;

        // dX = itofix(5): hit energy = fixtoi(itofix(25)*50/5) = 250,
        // reduced: max(250/3, 1) = 83, energy change = -(83/5) = -16% =
        // -16000 raw (DoEnergy fExact=false, C4Object.cpp:1347).
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.energy,
            energy_before - 16_000,
            "hit energy applied"
        );
        // fling: xdir = itofix(5)*50/100 = itofix(2.5), ydir = 0; no
        // Tumble/Jump actions on the def → raw velocity (C4Object.cpp:1612-1625)
        assert_eq!(
            engine.objects[victim_idx].fixed_velocity.x,
            math::C4Fixed::from_raw(math::itofix(5).val() * 50 / 100),
            "flung horizontally"
        );
        assert_eq!(
            engine.objects[victim_idx].fixed_velocity.y,
            math::C4Fixed::ZERO
        );
        Ok(())
    }

    #[test]
    fn cross_check_zero_hit_tracks_striker_controller_not_owner() -> Result<(), EngineError> {
        // EngObjHit is the one DoEnergy cause that updates the kill trace
        // even when integer division reduces iChange to zero. CrossCheck
        // attributes that hit to the hitting object's Controller, not Owner.
        let mut engine = Engine::with_seed(41);
        let mut victim_def = simple_definition("Clonk");
        victim_def.set_category(CATEGORY_LIVING);
        victim_def.set_mass(100);
        victim_def.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        engine.register_definition(victim_def)?;
        let mut rock_def = simple_definition("Rock");
        rock_def.set_category(CATEGORY_OBJECT);
        rock_def.set_mass(50);
        engine.register_definition(rock_def)?;

        let victim = engine.spawn_object(
            SpawnConfig::new("Clonk")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(5, 0))
                .with_alive(true)
                .with_energy(100_000)
                .with_controller(3),
        )?;
        let _rock = engine.spawn_object(
            SpawnConfig::new("Rock")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(5, 0))
                .with_owner(4)
                .with_controller(9),
        )?;
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[victim_idx].last_energy_loss_cause = 7;

        engine.cross_check(1)?;

        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.energy, 100_000,
            "equal velocities produce an exact zero-percent hit change"
        );
        assert_eq!(
            engine.objects[victim_idx].last_energy_loss_cause, 9,
            "a zero-change EngObjHit still records the hitter's controller, not its owner"
        );
        Ok(())
    }

    #[test]
    fn cross_check_fling_uses_live_caused_by_and_cpp_tumble_direction() -> Result<(), EngineError> {
        // DoEnergy marks the hit before Fx*Damage. Start both objects with
        // Controller 9 and a pre-existing killer 7, so that first mark is
        // suppressed by UpdatLastEnergyLossCause's self-damage guard. The
        // hook then changes the victim Controller and the striker's live
        // Controller; C4Object::Fling must re-read the latter and attribute
        // the living victim to 11. Its Tumble SetDir(txdir < 0) quirk maps a
        // negative x velocity to DIR_Right (C4Object.cpp:1641-1645).
        let victim_script = r#"#strict
func FxRedirectDamage(pTarget, iNumber, iChange, iCause, iCausedBy)
{
    SetController(-1, pTarget);
    SetController(11, FindObject(ROCK));
    return iChange;
}
"#;
        let mut victim_def =
            Definition::from_script("CLNK", "Clonk", victim_script).expect("victim compiles");
        victim_def.set_c4_callback_convention(true);
        victim_def.set_category(CATEGORY_LIVING);
        victim_def.set_mass(100);
        victim_def.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        victim_def.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_directions(2),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_directions(2),
                ),
            ]),
        );
        let mut rock_def = simple_definition("ROCK");
        rock_def.set_category(CATEGORY_OBJECT);
        rock_def.set_mass(50);

        let mut engine = Engine::with_seed(42);
        engine.register_player(PlayerConfig::new(11, "redirected striker"))?;
        engine.register_definition(victim_def)?;
        engine.register_definition(rock_def)?;
        let victim = engine.spawn_object(
            SpawnConfig::new("CLNK")
                .with_position(Vector2::new(50, 50))
                .with_alive(true)
                .with_energy(100_000)
                .with_controller(9)
                .with_action(ActionState::new("Walk"))
                .add_effect(EffectState::new("Redirect")),
        )?;
        let rock = engine.spawn_object(
            SpawnConfig::new("ROCK")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(-5, 0))
                .with_controller(9),
        )?;
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        let command_target = i32::try_from(victim.as_u64()).expect("test object id fits C4 int");
        engine.objects[victim_idx].state.effects[0].command_target = Some(command_target);
        engine.objects[victim_idx].last_energy_loss_cause = 7;

        engine.cross_check(1)?;

        let victim_idx = engine.find_object_index(victim).expect("victim remains");
        let rock_idx = engine.find_object_index(rock).expect("striker remains");
        assert_eq!(engine.objects[victim_idx].state.controller, OWNER_NONE);
        assert_eq!(engine.objects[rock_idx].state.controller, 11);
        assert_eq!(
            engine.objects[victim_idx].last_energy_loss_cause, 11,
            "Fling re-attributes the living victim from the striker's live Controller"
        );
        assert_eq!(engine.objects[victim_idx].state.action.name, "Tumble");
        assert_eq!(
            engine.objects[victim_idx].state.direction,
            Direction::Right,
            "negative fling x uses C++ SetDir(true) == DIR_Right"
        );
        Ok(())
    }

    #[test]
    fn fling_respects_dead_no_other_action_and_uses_raw_velocity_fallback() {
        let mut definition =
            Definition::from_script("FLCK", "Fling-locked actor", "#strict\n")
                .expect("fling actor compiles");
        definition.set_category(CATEGORY_LIVING);
        definition.set_mass(100);
        definition.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Dead".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_no_other_action(true),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(43);
        engine
            .register_definition(definition)
            .expect("fling actor registers");
        let mut rock = simple_definition("FLRK");
        rock.set_category(CATEGORY_OBJECT);
        rock.set_mass(50);
        engine
            .register_definition(rock)
            .expect("fling rock registers");
        let object = engine
            .spawn_object(
                SpawnConfig::new("FLCK")
                    .with_position(Vector2::new(50, 50))
                    .with_alive(true)
                    .with_energy(100_000)
                    .with_action(ActionState::new("Dead"))
                    .with_direction(Direction::Right)
                    .with_mobile(false),
            )
            .expect("fling actor spawns");
        engine
            .spawn_object(
                SpawnConfig::new("FLRK")
                    .with_position(Vector2::new(50, 50))
                    .with_velocity(Vector2::new(5, 0))
                    .with_controller(9),
            )
            .expect("fling rock spawns");
        let index = engine.find_object_index(object).expect("actor exists");
        let attach = CNAT_LEFT | CNAT_BOTTOM;
        engine.objects[index].state.t_attach = attach;
        engine.objects[index].frame_t_attach = attach;

        engine
            .cross_check(3)
            .expect("Tick3 cross-check flings a flight-procedure victim");

        let index = engine.find_object_index(object).expect("actor remains");
        let object = &engine.objects[index];
        assert_eq!(object.state.energy, 84_000, "the hit reached Fling");
        assert_eq!(object.state.action.name, "Dead");
        assert_eq!(object.state.direction, Direction::Right);
        assert_eq!(
            object.fixed_velocity,
            FixedVec2::new(
                C4Fixed::from_raw(itofix(5).val() * 50 / 100),
                C4Fixed::ZERO,
            )
        );
        assert!(object.state.mobile);
        assert_eq!(object.state.t_attach, CNAT_LEFT);
        assert_eq!(object.frame_t_attach, CNAT_LEFT);
    }

    #[test]
    fn cross_check_dead_raw_fling_sets_controller_and_clears_bottom_attach(
    ) -> Result<(), EngineError> {
        // A lethal DoEnergy refreshes OCF before CrossCheck calls Fling.
        // C4Object::Fling therefore takes its non-alive/uncontained arm and
        // assigns the striker's Controller. DoEnergy must already have made
        // that Controller (not Owner) visible through GetKiller in Death.
        // With no Tumble or Jump action, the raw fallback mobilizes and
        // clears only CNAT_Bottom from Action.t_attach
        // (C4Object.cpp:1641-1650).
        let mut engine = Engine::with_seed(43);
        engine.register_player(PlayerConfig::new(1, "Rock owner"))?;
        engine.register_player(PlayerConfig::new(2, "Rock controller"))?;
        let mut victim_def = Definition::from_script(
            "CLNK",
            "Clonk",
            "#strict\nlocal death_killer;\nfunc Death() { death_killer = GetKiller(); return 1; }\n",
        )?;
        victim_def.set_category(CATEGORY_LIVING);
        victim_def.set_mass(100);
        victim_def.set_physical(PhysicalInfo {
            energy: 1_000,
            ..PhysicalInfo::default()
        });
        engine.register_definition(victim_def)?;
        let mut rock_def = simple_definition("ROCK");
        rock_def.set_category(CATEGORY_OBJECT);
        rock_def.set_mass(50);
        engine.register_definition(rock_def)?;

        let victim = engine.spawn_object(
            SpawnConfig::new("CLNK")
                .with_position(Vector2::new(50, 50))
                .with_alive(true)
                .with_energy(1_000)
                .with_controller(3),
        )?;
        let _rock = engine.spawn_object(
            SpawnConfig::new("ROCK")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(5, 0))
                .with_owner(1)
                .with_controller(2),
        )?;
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        let attach = CNAT_LEFT | CNAT_BOTTOM;
        engine.objects[victim_idx].state.t_attach = attach;
        engine.objects[victim_idx].frame_t_attach = attach;

        engine.cross_check(1)?;

        let victim_idx = engine.find_object_index(victim).expect("victim remains");
        let victim = &engine.objects[victim_idx];
        assert!(!victim.state.alive, "the hit is lethal before Fling");
        assert_eq!(
            victim.last_energy_loss_cause, 2,
            "DoEnergy attributes the lethal hit to the striker's Controller"
        );
        assert_eq!(
            victim.state.local_vars.get("death_killer"),
            Some(&Value::Int(2)),
            "GetKiller exposes the Controller during the Death callback"
        );
        assert_eq!(
            victim.state.controller, 2,
            "non-alive uncontained Fling takes the striker's Controller"
        );
        assert!(victim.state.mobile, "the raw fallback mobilizes");
        assert_eq!(victim.state.t_attach, CNAT_LEFT);
        assert_eq!(victim.frame_t_attach, CNAT_LEFT);
        Ok(())
    }

    #[test]
    fn energy_loss_to_zero_assigns_death_like_cpp() -> Result<(), EngineError> {
        // C4Object::DoEnergy (C4Object.cpp:1361-1363): an alive object whose
        // energy first reaches zero dies. AssignDeath (C4Object.cpp:1137-1177)
        // sets the "Dead" action, clears commands, ejects contents at the
        // object's position, and runs the Death callback with the death
        // causing player (the last energy-loss cause).
        let mut engine = Engine::with_seed(90);
        let mut clonk_def = Definition::from_script(
            "Clonk",
            "Clonk",
            r#"
            func Death(by) { return 1; }
            "#,
        )?;
        clonk_def.set_crew_member(true);
        clonk_def.set_physical(PhysicalInfo {
            energy: 5_000,
            ..PhysicalInfo::default()
        });
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Dead".to_string(), ActionSpec::default());
        clonk_def.configure_actions(Some("Idle".to_string()), specs);
        engine.register_definition(clonk_def)?;
        engine.register_definition(simple_definition("Gem"))?;

        let clonk = engine.spawn_object(
            SpawnConfig::new("Clonk")
                .with_position(Vector2::new(50, 50))
                .with_alive(true)
                .with_energy(5_000),
        )?;
        let gem = engine.spawn_object(
            SpawnConfig::new("Gem")
                .with_position(Vector2::new(50, 50))
                .with_container(clonk),
        )?;

        let idx = engine.find_object_index(clonk).expect("clonk exists");
        engine
            .change_object_energy(idx, -3, C4FX_CALL_ENG_SCRIPT, 7)
            .expect("energy change succeeds");
        assert!(engine.objects[idx].state.alive, "energy 2000 raw left");
        engine
            .change_object_energy(idx, -2, C4FX_CALL_ENG_SCRIPT, 7)
            .expect("energy change succeeds");
        let idx = engine.find_object_index(clonk).expect("clonk exists");
        assert!(!engine.objects[idx].state.alive, "dead at zero energy");
        assert_eq!(engine.objects[idx].state.action.name, "Dead");
        assert!(
            engine.objects[idx].state.contents.is_empty(),
            "contents lost"
        );
        let gem_idx = engine.find_object_index(gem).expect("gem exists");
        assert_eq!(engine.objects[gem_idx].state.container, None, "gem ejected");
        assert_eq!(
            engine.objects[gem_idx].state.position,
            Vector2::new(50, 50),
            "ejected at the dying object's position"
        );

        // Death is not re-assigned (already dead, C4Object.cpp:1141)
        engine
            .change_object_energy(idx, -1, C4FX_CALL_ENG_SCRIPT, 9)
            .expect("energy change succeeds");
        assert_eq!(engine.objects[idx].last_energy_loss_cause, 9);
        Ok(())
    }

    #[test]
    fn assign_death_exits_contents_with_zero_motion_and_resets_view_range(
    ) -> Result<(), EngineError> {
        let script = r#"#strict 2
local death_view_range;
func Death()
{
    death_view_range = GetObjectVal("PlrViewRange", 0, this());
    return 1;
}
"#;
        let mut corpse_definition = Definition::from_script("DCOR", "Death corpse", script)?;
        corpse_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Dead".to_string(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(91);
        engine.register_player(PlayerConfig::new(0, "Owner"))?;
        engine.register_definition(corpse_definition)?;
        engine.register_definition(simple_definition("DITM"))?;

        let corpse = engine.spawn_object(
            SpawnConfig::new("DCOR")
                .with_owner(0)
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(50, 50))
                .with_fixed_velocity(FixedVec2::new(itofix(7), itofix(-3)))
                .with_plr_view_range(333)
                .with_alive(true),
        )?;
        let item = engine.spawn_object(
            SpawnConfig::new("DITM")
                .with_loaded(true)
                .with_container(corpse)
                .with_status(ObjectStatus::Inactive)
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(47, 53))
                .with_fixed_position(FixedVec2::new(
                    itofix(47) + math::C4Fixed::from_raw(123),
                    itofix(53) - math::C4Fixed::from_raw(321),
                ))
                .with_fixed_velocity(FixedVec2::new(itofix(-4), itofix(6)))
                .with_rotation(73)
                .with_fixed_rotation(itofix(73))
                .with_rotation_velocity(itofix(2))
                .with_in_liquid(true)
                .with_mobile(false),
        )?;
        let item_idx = engine.find_object_index(item).expect("item exists");
        let item_position = engine.objects[item_idx].state.position;
        let item_fixed_position = engine.objects[item_idx].fixed_position;
        assert_ne!(
            item_fixed_position,
            FixedVec2::from_ints(item_position.x, item_position.y)
        );
        assert_ne!(engine.objects[item_idx].fixed_velocity, FixedVec2::ZERO);
        assert_ne!(engine.objects[item_idx].fixed_rotation, math::C4Fixed::ZERO);
        assert_ne!(
            engine.objects[item_idx].rotation_velocity,
            math::C4Fixed::ZERO
        );

        let corpse_idx = engine.find_object_index(corpse).expect("corpse exists");
        engine.assign_death(corpse_idx, false)?;

        let item_idx = engine.find_object_index(item).expect("item remains");
        let item_state = &engine.objects[item_idx];
        assert_eq!(item_state.state.container, None);
        assert_eq!(item_state.state.position, item_position);
        assert_eq!(
            item_state.fixed_position,
            FixedVec2::from_ints(item_position.x, item_position.y),
            "Exit(x, y) snaps a loaded item's subpixel position to its integer coordinates"
        );
        assert_eq!(item_state.state.rotation, 0);
        assert_eq!(item_state.fixed_rotation, math::C4Fixed::ZERO);
        assert_eq!(item_state.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(item_state.state.velocity, Vector2::ZERO);
        assert_eq!(item_state.rotation_velocity, math::C4Fixed::ZERO);
        assert!(item_state.state.mobile);
        assert!(!item_state.state.in_liquid);

        let corpse_state = engine.object_snapshot(corpse).expect("corpse remains");
        assert_eq!(corpse_state.plr_view_range, 0);
        assert_eq!(
            corpse_state.local_vars.get("death_view_range"),
            Some(&Value::Int(0)),
            "nonliving range is cleared before Death"
        );

        let living = engine.spawn_object(
            SpawnConfig::new("DCOR")
                .with_owner(0)
                .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                .with_plr_view_range(333)
                .with_alive(true),
        )?;
        let living_idx = engine.find_object_index(living).expect("living exists");
        engine.assign_death(living_idx, false)?;
        let living_state = engine
            .object_snapshot(living)
            .expect("living corpse remains");
        assert_eq!(living_state.plr_view_range, 333);
        assert_eq!(
            living_state.local_vars.get("death_view_range"),
            Some(&Value::Int(333)),
            "owned living FoW objects retain their range for dead-view decay"
        );

        let ownerless = engine.spawn_object(
            SpawnConfig::new("DCOR")
                .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                .with_plr_view_range(333)
                .with_alive(true),
        )?;
        let ownerless_idx = engine
            .find_object_index(ownerless)
            .expect("ownerless living exists");
        engine.assign_death(ownerless_idx, false)?;
        let ownerless_state = engine
            .object_snapshot(ownerless)
            .expect("ownerless corpse remains");
        assert_eq!(ownerless_state.plr_view_range, 0);
        assert_eq!(
            ownerless_state.local_vars.get("death_view_range"),
            Some(&Value::Int(0)),
            "a living object without a valid owner has no death-view exemption"
        );
        Ok(())
    }

    #[test]
    fn dead_living_fow_view_range_decays_and_restores_runtime_membership(
    ) -> Result<(), EngineError> {
        // C4Player::Execute keeps dead living objects in the runtime-only
        // FoWViewObjs list and subtracts ten after control/menu processing.
        // The list is rebuilt from saved PlrViewRange values after loading
        // (C4Player.cpp:214-226; C4ObjectList.cpp:597-604).
        fn corpse_definition() -> Result<Definition, EngineError> {
            let mut definition = Definition::from_script("FOWC", "FoW corpse", "#strict 2\n")?;
            definition.set_crew_member(true);
            definition.configure_actions(
                Some("Idle".to_string()),
                HashMap::from([
                    ("Idle".to_string(), ActionSpec::default()),
                    ("Dead".to_string(), ActionSpec::default()),
                ]),
            );
            Ok(definition)
        }

        let mut engine = Engine::with_seed(92);
        engine.register_player(PlayerConfig::new(0, "Owner"))?;
        engine.register_definition(corpse_definition()?)?;
        engine.frame = 1; // avoid unrelated Tick35 elimination work
        let corpse = engine.spawn_object(
            SpawnConfig::new("FOWC")
                .with_owner(0)
                .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                .with_crew_member(true)
                .with_plr_view_range(500)
                .with_alive(true),
        )?;
        let corpse_index = engine.find_object_index(corpse).expect("corpse exists");
        engine.assign_death(corpse_index, false)?;
        assert_eq!(
            engine
                .object_snapshot(corpse)
                .expect("corpse remains")
                .plr_view_range,
            500
        );

        engine.tick_player_systems()?;
        assert_eq!(
            engine
                .object_snapshot(corpse)
                .expect("corpse remains")
                .plr_view_range,
            490
        );

        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("nonzero view range serializes");
        let decoded = EngineState::from_json_str(&encoded).expect("view range state decodes");
        assert_eq!(
            decoded
                .objects
                .iter()
                .find(|object| object.snapshot.id == corpse)
                .map(|object| object.snapshot.plr_view_range),
            Some(490)
        );

        let mut restored = Engine::with_seed(93);
        restored.register_definition(corpse_definition()?)?;
        restored.restore_state(&decoded)?;
        for _ in 0..48 {
            restored.tick_player_systems()?;
        }
        assert_eq!(
            restored
                .object_snapshot(corpse)
                .expect("restored corpse remains")
                .plr_view_range,
            10
        );
        restored.tick_player_systems()?;
        assert_eq!(
            restored
                .object_snapshot(corpse)
                .expect("restored corpse remains")
                .plr_view_range,
            0
        );
        restored.tick_player_systems()?;
        assert_eq!(
            restored
                .object_snapshot(corpse)
                .expect("removed FoW target remains as a corpse")
                .plr_view_range,
            0,
            "removal from FoWViewObjs stops further decay"
        );
        Ok(())
    }

    #[test]
    fn assign_death_runs_exit_callbacks_in_contents_order_before_death() -> Result<(), EngineError>
    {
        let corpse_script = r#"#strict 2
local trace, remaining, added;
func Mark(int step) { trace = trace * 10 + step; return 1; }
func Ejection(object item)
{
    item->MarkEjection(this());
    if (!added)
    {
        var new_item = CreateContents(OITM);
        new_item->SetMarker(7);
        added = 1;
    }
    remaining = remaining * 10 + ContentsCount();
    return 1;
}
func Death() { return Mark(5); }
"#;
        let item_script = r#"#strict 2
local marker;
func SetMarker(int value) { marker = value; return 1; }
func MarkEjection(object old_container)
{
    return old_container->Mark(marker);
}
func Departure(object old_container)
{
    return old_container->Mark(marker + 1);
}
"#;
        let mut corpse_definition = Definition::from_script("DORD", "Death order", corpse_script)?;
        corpse_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Dead".to_string(), ActionSpec::default()),
            ]),
        );
        let item_definition = Definition::from_script("OITM", "Ordered item", item_script)?;

        let mut engine = Engine::with_seed(92);
        engine.register_definition(corpse_definition)?;
        engine.register_definition(item_definition)?;
        let corpse = engine.spawn_object(SpawnConfig::new("DORD").with_alive(true))?;
        let marker_three = engine.spawn_object(
            SpawnConfig::new("OITM")
                .with_container(corpse)
                .with_local_vars(HashMap::from([("marker".to_string(), Value::Int(3))])),
        )?;
        let marker_one = engine.spawn_object(
            SpawnConfig::new("OITM")
                .with_container(corpse)
                .with_local_vars(HashMap::from([("marker".to_string(), Value::Int(1))])),
        )?;
        let corpse_idx = engine.find_object_index(corpse).expect("corpse exists");
        assert_eq!(
            engine.objects[corpse_idx].state.contents,
            vec![marker_one, marker_three],
            "same-definition stContents insertion puts the newest item first"
        );

        engine.assign_death(corpse_idx, false)?;

        let corpse_state = engine.object_snapshot(corpse).expect("corpse remains");
        assert!(corpse_state.contents.is_empty());
        assert_eq!(
            corpse_state.local_vars.get("trace"),
            Some(&Value::Int(1_278_345))
        );
        assert_eq!(
            corpse_state.local_vars.get("remaining"),
            Some(&Value::Int(210)),
            "each Ejection observes callback-added contents in the live list"
        );
        for item in [marker_one, marker_three] {
            assert_eq!(
                engine
                    .object_snapshot(item)
                    .expect("item remains")
                    .container,
                None
            );
        }
        Ok(())
    }

    #[test]
    fn assign_death_updates_crew_info_before_ejection() -> Result<(), EngineError> {
        let crew_script = r#"#strict 2
local replacement, count_seen, replacement_joined;
func SetReplacement(object target) { replacement = target; return 1; }
func Ejection(object item)
{
    count_seen = GetObjectInfoCoreVal("DeathCount", "ObjectInfo");
    replacement_joined = MakeCrewMember(replacement, 0);
    return 1;
}
"#;
        let mut crew_definition = Definition::from_script("DCRW", "Death crew", crew_script)?;
        crew_definition.set_crew_member(true);
        crew_definition.set_category(CATEGORY_OBJECT | CATEGORY_LIVING);
        crew_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Dead".to_string(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(93);
        engine.register_definition(crew_definition)?;
        engine.register_definition(simple_definition("CITM"))?;
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("DCRW".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine.join_player(lifecycle_join_config(
            "Death-info owner",
            vec![player_file::CrewInfo {
                id: "DCRW".to_string(),
                name: "Veteran".to_string(),
                death_count: 4,
                total_playing_time: 17,
                ..Default::default()
            }],
        ))?;
        let crew = engine.player(0).expect("player joins").crew()[0];
        let original_link = engine.capture_state().crew_info_links[&crew];
        let replacement = engine.spawn_object(
            SpawnConfig::new("DCRW")
                .with_owner(0)
                .with_crew_member(false)
                .with_alive(true),
        )?;
        let crew_idx = engine.find_object_index(crew).expect("crew exists");
        engine.call_object_function(
            crew_idx,
            "SetReplacement",
            vec![Value::Object(replacement.as_u64())],
        )?;
        engine.spawn_object(SpawnConfig::new("CITM").with_container(crew))?;
        engine.game_time = 23;

        engine.assign_death(crew_idx, false)?;

        let corpse = engine.object_snapshot(crew).expect("dead crew remains");
        assert_eq!(corpse.local_vars.get("count_seen"), Some(&Value::Int(5)));
        assert_eq!(
            corpse.local_vars.get("replacement_joined"),
            Some(&Value::Bool(true))
        );
        let state = engine.capture_state();
        assert_eq!(state.crew_info_links[&crew], original_link);
        assert_eq!(state.crew_object_infos[&crew].death_count, 5);
        let replacement_link = state.crew_info_links[&replacement];
        assert_ne!(
            replacement_link, original_link,
            "HasDied is already set, so Ejection cannot recycle the dead info"
        );
        assert_eq!(state.crew_info_rosters[&0].len(), 2);
        let dead_info = &state.crew_info_rosters[&0][original_link.roster_index];
        assert!(dead_info.has_died);
        assert_eq!(dead_info.death_count, 5);
        assert!(!dead_info.in_action);
        assert_eq!(dead_info.total_playing_time, 40);
        let encoded = state
            .to_json_string()
            .expect("death-count state serializes");
        let decoded = EngineState::from_json_str(&encoded)
            .expect("death-count state deserializes");
        assert_eq!(
            decoded.crew_info_rosters[&0][original_link.roster_index].death_count,
            5
        );
        assert_eq!(decoded.crew_object_infos[&crew].death_count, 5);
        Ok(())
    }

    #[test]
    fn assign_death_permanently_unlinks_owner_crew_across_ticks_and_cursor_controls(
    ) -> Result<(), EngineError> {
        // AssignDeath removes the corpse through the owning player's
        // ClearPointers(this, true). GetCrew/GetCrewCount and CursorLeft/
        // CursorRight all consume that authoritative list; no later player
        // execution may reconstruct it from the object's legacy CrewMember
        // bit (C4Object.cpp:1194-1196; C4Player.cpp:57-69).
        let script = r#"#strict 2
func Probe(object corpse)
{
    return [GetCrewCount(0), GetCrew(0, 0) == this(),
            GetCrew(0, 1), GetCrew(0, 0) == corpse];
}
func Share() { return SetCrewStatus(1, true); }
func Recruit() { return MakeCrewMember(this(), 0); }
"#;
        let mut definition = Definition::from_script("DCRW", "Death crew", script)?;
        definition.set_crew_member(true);
        definition.set_category(CATEGORY_OBJECT | CATEGORY_LIVING);
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Dead".to_string(), ActionSpec::default()),
            ]),
        );

        let crew_info = |name: &str| player_file::CrewInfo {
            id: "DCRW".to_string(),
            name: name.to_string(),
            ..Default::default()
        };
        let mut engine = Engine::with_seed(155);
        engine.register_definition(definition)?;
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("DCRW".to_string(), 2)];
        engine.set_player_starts(vec![start]);
        engine.join_player(lifecycle_join_config(
            "Death-list owner",
            vec![crew_info("First"), crew_info("Second")],
        ))?;
        engine.register_player(PlayerConfig::new(1, "Foreign roster"))?;

        let crew = engine.player(0).expect("player joins").crew().to_vec();
        assert_eq!(crew.len(), 2);
        let corpse = crew[0];
        let survivor = crew[1];
        let corpse_link = engine.capture_state().crew_info_links[&corpse];
        engine.set_crew_cursor(0, Some(corpse))?;
        let corpse_index = engine.find_object_index(corpse).expect("crew exists");
        assert_eq!(
            engine.call_object_function(corpse_index, "Share", Vec::new())?,
            Value::Int(1)
        );
        engine.set_crew_cursor(1, Some(corpse))?;
        engine.game_time = 17;

        engine.assign_death(corpse_index, false)?;

        let corpse_state = engine.object_snapshot(corpse).expect("body persists");
        assert!(!corpse_state.alive);
        assert!(
            corpse_state.crew_member,
            "the compatibility union bit stays set for a foreign Crew link"
        );
        assert_eq!(engine.player(0).expect("player exists").crew(), &[survivor]);
        assert_eq!(engine.crew_cursor(0), Some(survivor));
        assert_eq!(engine.player(1).expect("foreign player").crew(), &[corpse]);
        assert_eq!(engine.crew_cursor(1), Some(corpse));
        let survivor_index = engine
            .find_object_index(survivor)
            .expect("survivor exists");
        assert_eq!(
            engine.call_object_function(
                survivor_index,
                "Probe",
                vec![Value::Object(corpse.as_u64())],
            )?,
            Value::Array(vec![
                Value::Int(1),
                Value::Bool(true),
                Value::Nil,
                Value::Bool(false),
            ])
        );

        for _ in 0..3 {
            engine.tick_without_snapshot()?;
            assert_eq!(
                engine.player(0).expect("player exists").crew(),
                &[survivor],
                "the corpse must not re-enter during player execution"
            );
            assert_eq!(engine.player(1).expect("foreign player").crew(), &[corpse]);
            assert_eq!(engine.crew_cursor(1), Some(corpse));
        }
        assert_eq!(
            engine.call_object_function(
                survivor_index,
                "Probe",
                vec![Value::Object(corpse.as_u64())],
            )?,
            Value::Array(vec![
                Value::Int(1),
                Value::Bool(true),
                Value::Nil,
                Value::Bool(false),
            ]),
            "GetCrew/GetCrewCount stay pruned after later frames"
        );
        for command in [COM_CURSOR_RIGHT, COM_CURSOR_LEFT] {
            engine.player_direct_com(0, command, 0)?;
            assert_eq!(engine.crew_cursor(0), Some(survivor));
        }

        let state = engine.capture_state();
        assert_eq!(
            state
                .players
                .iter()
                .find(|player| player.id == 0)
                .expect("owner state")
                .crew,
            vec![survivor]
        );
        assert_eq!(
            state
                .players
                .iter()
                .find(|player| player.id == 1)
                .expect("foreign state")
                .crew,
            vec![corpse]
        );
        let dead_info = &state.crew_info_rosters[&0][corpse_link.roster_index];
        assert!(dead_info.has_died);
        assert_eq!(dead_info.death_count, 1);
        assert!(!dead_info.in_action);
        assert_eq!(dead_info.total_playing_time, 17);

        let replacement = engine.spawn_object(
            SpawnConfig::new("DCRW")
                .with_owner(0)
                .with_alive(true)
                .with_crew_member(false)
                .with_action(ActionState::new("Idle")),
        )?;
        let replacement_index = engine
            .find_object_index(replacement)
            .expect("replacement exists");
        assert_eq!(
            engine.call_object_function(replacement_index, "Recruit", Vec::new())?,
            Value::Bool(true)
        );
        let replacement_link = engine.capture_state().crew_info_links[&replacement];
        assert_ne!(
            replacement_link, corpse_link,
            "GetIdle must never recycle an info whose HasDied flag is set"
        );
        assert!(!engine.player(0).expect("player exists").crew().contains(&corpse));
        Ok(())
    }

    #[test]
    fn assign_death_refreshes_ocf_before_dead_action_callbacks() -> Result<(), EngineError> {
        // SetAction("Dead") refreshes OCF before the new StartCall and old
        // AbortCall (C4Object.cpp:4141,4173). AssignDeath has already cleared
        // Alive, so neither callback may observe the stale OCF_Alive bit.
        let script = r#"#strict
local start_ocf_alive, abort_ocf_alive;
protected func DeadStart() { start_ocf_alive = GetOCF() & OCF_Alive; }
protected func WalkAbort() { abort_ocf_alive = GetOCF() & OCF_Alive; }
"#;
        let mut definition = Definition::from_script("DCOF", "Death OCF", script)?;
        definition.set_category(CATEGORY_LIVING | CATEGORY_OBJECT);
        definition.set_c4_callback_convention(true);
        definition.configure_actions(
            None,
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_abort_call("WalkAbort"),
                ),
                (
                    "Dead".to_string(),
                    ActionSpec::default().with_start_call("DeadStart"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(91);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("DCOF")
                .with_category(CATEGORY_LIVING | CATEGORY_OBJECT)
                .with_alive(true)
                .with_action(ActionState::new("Walk")),
        )?;
        let idx = engine.find_object_index(id).expect("object exists");
        assert_ne!(engine.objects[idx].state.ocf & ocf::ALIVE, 0);

        engine.assign_death(idx, false)?;

        let object = engine.object_snapshot(id).expect("dead object remains");
        assert_eq!(object.local_vars.get("start_ocf_alive"), Some(&Value::Int(0)));
        assert_eq!(object.local_vars.get("abort_ocf_alive"), Some(&Value::Int(0)));
        Ok(())
    }

    #[test]
    fn sync_check_excludes_inactive_and_deleted_status_objects() -> Result<(), EngineError> {
        // C4ControlSyncCheck::Set reads Game.Objects.ObjectCount and each
        // in-bounds sector's ObjectShapes.ObjectCount. Both list counts skip
        // Status==0 objects, while C4OS_INACTIVE objects live in the separate
        // InactiveObjects list and have no sector links.
        let mut definition = simple_definition("SCST");
        definition.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        definition.set_ocf_base(ocf::GRAB);

        let mut engine = Engine::with_seed(79);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.register_definition(definition)?;
        let active = engine.spawn_object(
            SpawnConfig::new("SCST").with_position(Vector2::new(25, 25)),
        )?;
        let inactive = engine.spawn_object(
            SpawnConfig::new("SCST")
                .with_position(Vector2::new(75, 25))
                .with_status(ObjectStatus::Inactive),
        )?;

        let packet = engine.sync_check(0);
        assert_eq!((packet.object_count, packet.sector_shape_sum), (1, 1));
        let sectors = engine.sectors.as_ref().expect("sectors initialized");
        assert!(!sectors
            .object_ids(SectorKey::Inside { x: 1, y: 0 })
            .contains(&inactive));
        assert!(!sectors
            .shape_ids(SectorKey::Inside { x: 1, y: 0 })
            .contains(&inactive));
        assert!(engine
            .at_object(Vector2::new(75, 25), ocf::GRAB, None)
            .is_none());

        engine.apply_object_update(
            active,
            ObjectUpdate::new().with_status(ObjectStatus::Deleted),
        )?;
        let packet = engine.sync_check(0);
        assert_eq!(
            (packet.object_count, packet.sector_shape_sum),
            (0, 0),
            "Status-zero removal tombstones stop contributing immediately"
        );
        assert!(engine
            .at_object(Vector2::new(25, 25), ocf::GRAB, None)
            .is_none());
        Ok(())
    }

    #[test]
    fn sync_check_omits_outside_sector_shape_memberships() -> Result<(), EngineError> {
        // C4LSectors::getShapeSum iterates only Sectors[0..Size); SectorOut
        // retains its membership list but is deliberately absent from the
        // digest.
        let mut definition = simple_definition("SCOT");
        definition.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        definition.set_ocf_base(ocf::GRAB);

        let mut engine = Engine::with_seed(79);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.register_definition(definition)?;
        let outside = engine.spawn_object(
            SpawnConfig::new("SCOT").with_position(Vector2::new(150, 25)),
        )?;

        let packet = engine.sync_check(0);
        assert_eq!(packet.object_count, 1);
        assert_eq!(packet.sector_shape_sum, 0);
        let sectors = engine.sectors.as_ref().expect("sectors initialized");
        assert!(sectors
            .shape_ids(SectorKey::Outside)
            .contains(&outside));
        Ok(())
    }

    #[test]
    fn sync_check_digest_and_state_machine_match_cpp() -> Result<(), EngineError> {
        // C4ControlSyncCheck::Set (C4Control.cpp:445-468): Random3 is the
        // Rnd3 ring pointer, RandomCount the synced draw count, AllCrewPosX
        // sums fixtoi(fix_x, 100) (centipixels) over the players' crew
        // lists. C4GameControl::Ticks (C4GameControl.cpp:326-332) advances
        // ControlTick every ControlRate frames and requests a sync check
        // every SyncRate frames; old checks drop after 50 frames
        // (C4GameControl.cpp:508-522, C4SyncCheckMaxKeep).
        let mut engine = Engine::with_seed(80);
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        let mut crew_def = simple_definition("Clonk");
        crew_def.set_crew_member(true);
        engine.register_definition(crew_def)?;
        let crew = engine.spawn_object(
            SpawnConfig::new("Clonk")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(10, 10)),
        )?;
        // give the crew sub-pixel x so the centipixel precision is visible
        let idx = engine.find_object_index(crew).expect("crew exists");
        engine.objects[idx].fixed_position.x =
            math::itofix(10) + math::C4Fixed::from_raw(math::itofix(1).val() / 4); // 10.25

        engine.tick_without_snapshot()?; // builds crew lists
        let packet = engine.sync_check(0);
        assert_eq!(packet.random3, engine.rng.rnd3_ptr());
        assert_eq!(packet.random_count, engine.rng.count);
        assert_eq!(
            packet.crew_positions_sum,
            math::fixtoi_prec(engine.objects[idx].fixed_position.x, 100),
            "centipixel crew sum over the player's crew list"
        );
        assert_eq!(packet.object_count, 1);
        assert_eq!(
            packet.object_enumeration_index, 1,
            "ObjectEnumerationIndex is the last assigned object number"
        );
        let initial_network_data = InitialNetworkGameData::from_engine(&engine)
            .expect("modeled engine state captures as initial network data");
        assert_eq!(
            packet.object_enumeration_index,
            initial_network_data.object_enumeration_index,
            "sync-check and Game.txt report the same C++ allocator high-water mark"
        );

        // ControlRate gating: with rate 2, ControlTick advances on even frames.
        let mut gated = Engine::with_seed(81);
        gated.control_rate = 2;
        for _ in 0..4 {
            gated.tick_without_snapshot()?;
        }
        assert_eq!(gated.control_tick, 2, "frames 2 and 4 advance the tick");

        // SyncRate: the digest is queued on frame % 100 == 0 and pruned
        // after 50 frames.
        let mut machine = Engine::with_seed(82);
        machine.sync_rate = 10;
        for _ in 0..10 {
            machine.tick_without_snapshot()?;
        }
        assert!(machine.get_sync_check(10).is_some(), "queued on frame 10");
        // strict cutoff (C4GameControl.cpp:519: frame < FrameCounter - 50):
        // check 10 survives the frame-60 prune and drops at the frame-70 one.
        for _ in 0..50 {
            machine.tick_without_snapshot()?;
        }
        assert!(
            machine.get_sync_check(10).is_some(),
            "10 >= 60 - 50 keeps it at frame 60"
        );
        for _ in 0..10 {
            machine.tick_without_snapshot()?;
        }
        assert!(
            machine.get_sync_check(10).is_none(),
            "pruned once frame - 50 exceeds it"
        );
        assert!(machine.get_sync_check(60).is_some());

        // Remote comparison (C4ControlSyncCheck::Execute, C4Control.cpp:469+):
        // matching digest → ok; tampered digest → synchronization loss.
        let local = machine.get_sync_check(60).cloned().expect("local check");
        assert!(machine.register_remote_sync_check(local.clone()));
        let mut shifted_tick = local.clone();
        shifted_tick.control_tick += 1;
        assert!(
            !machine.register_remote_sync_check(shifted_tick.clone()),
            "live control compares ControlTick"
        );
        machine.set_replay_control(true);
        assert!(
            machine.register_remote_sync_check(shifted_tick),
            "replay control exempts only ControlTick"
        );
        let mut tampered = local;
        tampered.random_count += 1;
        assert!(!machine.register_remote_sync_check(tampered));
        Ok(())
    }

    #[test]
    fn network_control_timing_starts_at_join_tick_and_uses_cpp_cadence() -> Result<(), EngineError> {
        // The joining client copies Parameters.ControlRate and initializes
        // ControlTick from JoinData::iStartCtrlTick (C4Network2.cpp:1607-1608;
        // C4GameControlNetwork.cpp:46-52). C4GameControl::Ticks then advances
        // that tick only on FrameCounter % ControlRate == 0
        // (C4GameControl.cpp:326-329).
        let mut engine = Engine::with_seed(83);
        let timing = NetworkControlTiming::new(9, 2).expect("C++ host control rate is valid");
        engine.initialize_network_control_timing(timing);

        assert_eq!(engine.sync_check(0).control_tick, 9);
        engine.tick_without_snapshot()?;
        assert_eq!(engine.frame(), 1);
        assert_eq!(engine.sync_check(0).control_tick, 9);
        engine.tick_without_snapshot()?;
        assert_eq!(engine.frame(), 2);
        assert_eq!(engine.sync_check(0).control_tick, 10);
        engine.tick_without_snapshot()?;
        assert_eq!(engine.sync_check(0).control_tick, 10);
        engine.tick_without_snapshot()?;
        assert_eq!(engine.frame(), 4);
        assert_eq!(engine.sync_check(0).control_tick, 11);
        Ok(())
    }

    #[test]
    fn network_control_timing_rejects_rates_outside_cpp_host_bounds() {
        // A normal C++ host bounds Config.Network.ControlRate to
        // 1..=C4MaxControlRate (C4GameControl.cpp:224-226), where
        // C4MaxControlRate is 20 (C4Constants.h:43). JoinData is copied
        // directly (C4Network2.cpp:1607), so malformed peers must be rejected
        // rather than silently normalized to a different synchronized rate.
        assert!(NetworkControlTiming::new(9, 1).is_ok());
        assert!(NetworkControlTiming::new(9, 20).is_ok());
        assert!(NetworkControlTiming::new(9, 0).is_err());
        assert!(NetworkControlTiming::new(9, 21).is_err());
    }

    #[test]
    fn sync_check_pxs_count_includes_pixels_that_deactivate_during_execute() {
        // C4PXSSystem::Execute resets Count, then increments it AFTER every
        // live slot's Execute call (C4PXS.cpp:212-234). A PXS that deactivates
        // during that call is therefore absent from storage but still present
        // in C4ControlSyncCheck::PXSCount for this frame (C4Control.cpp:453).
        let library = MaterialLibrary::parse(
            r#"
            [Material Sand]
            Name=Sand
            Density=25
            Friction=10
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let sand = materials.id_of("Sand").expect("sand exists");
        let mut engine = Engine::with_seed(83);
        engine.set_materials(materials);

        // With no landscape GBackWdt/GBackHgt are zero, so this valid PXS
        // deactivates in C4PXS::Execute's out-of-bounds check.
        assert!(engine.pxs_system.create(
            sand,
            math::C4Fixed::ZERO,
            math::C4Fixed::ZERO,
            math::C4Fixed::ZERO,
            math::C4Fixed::ZERO,
        ));
        engine.tick_pxs();
        assert_eq!(engine.pxs_system.iter().count(), 0, "pixel deactivated");
        assert_eq!(
            engine.sync_check(0).pxs_count,
            1,
            "Count records the executed slot even though it died"
        );

        engine.tick_pxs();
        assert_eq!(
            engine.sync_check(0).pxs_count,
            0,
            "the next Execute resets Count before scanning an empty system"
        );
    }

    #[test]
    fn incinerate_burn_turn_to_and_contents_ejection_match_cpp() -> Result<(), EngineError> {
        // fxFireStart (C4Effect.cpp:579-594): BurnTurnTo changes the
        // definition when fire is caused; contents are ejected at the
        // object's position unless IncompleteActivity or NoBurnDecay.
        let mut engine = Engine::with_seed(95);
        let mut hut_def = simple_definition("Hut");
        hut_def.set_burn_turn_to(Some("Ruin".to_string()));
        engine.register_definition(hut_def)?;
        engine.register_definition(simple_definition("Ruin"))?;
        engine.register_definition(simple_definition("Gem"))?;

        let hut =
            engine.spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(30, 30)))?;
        let gem = engine.spawn_object(
            SpawnConfig::new("Gem")
                .with_position(Vector2::new(30, 30))
                .with_container(hut),
        )?;

        let idx = engine.find_object_index(hut).expect("hut exists");
        assert!(engine.incinerate_object(idx, 1, false, None)?);
        let idx = engine.find_object_index(hut).expect("hut exists");
        assert_eq!(
            engine.objects[idx].definition_id, "Ruin",
            "BurnTurnTo changed the definition"
        );
        assert!(engine.objects[idx].state.on_fire);
        assert!(engine.objects[idx].state.contents.is_empty());
        let gem_idx = engine.find_object_index(gem).expect("gem exists");
        assert_eq!(engine.objects[gem_idx].state.container, None, "ejected");
        assert_eq!(engine.objects[gem_idx].state.position, Vector2::new(30, 30));

        // NoBurnDecay keeps the contents (C4Effect.cpp:588).
        let mut keeper_def = simple_definition("Chest");
        keeper_def.set_fire_properties(0, true, false);
        engine.register_definition(keeper_def)?;
        let chest =
            engine.spawn_object(SpawnConfig::new("Chest").with_position(Vector2::new(50, 30)))?;
        let coin = engine.spawn_object(
            SpawnConfig::new("Gem")
                .with_position(Vector2::new(50, 30))
                .with_container(chest),
        )?;
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert!(engine.incinerate_object(chest_idx, 1, false, None)?);
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(engine.objects[chest_idx].state.contents, vec![coin]);
        Ok(())
    }

    #[test]
    fn fire_start_ejects_through_exit_and_detaches_attachers_on_both_paths(
    ) -> Result<(), EngineError> {
        // FnFxFireStart writes the causing player before the real Exit, so
        // Departure observes the new controller. It then finds every
        // DFA_ATTACH object whose primary target is the burning object and
        // calls SetAction(ActIdle), including the normal AbortCall. Exercise
        // both Rust entry points: direct engine ignition and script-host
        // Incinerate/inherited native dispatch.
        for via_script in [false, true] {
            let path = if via_script { "compat" } else { "engine" };
            let mut engine = Engine::with_seed(96);
            engine.register_player(PlayerConfig::new(7, "Incinerator"))?;
            engine.register_definition(Definition::from_script(
                "ACTR",
                "Actor",
                "#strict\nfunc Ignite(pTarget) { return Incinerate(pTarget); }\n",
            )?)?;

            let mut burner_definition = Definition::from_script(
                "BURN",
                "Burner",
                r#"#strict
local ejection_count, ejection_controller;
func Ejection(pContent)
{
    ejection_count = ejection_count + 1;
    ejection_controller = pContent->GetController();
    return 1;
}
"#,
            )?;
            burner_definition.set_c4_callback_convention(true);
            engine.register_definition(burner_definition)?;

            let mut content_definition = Definition::from_script(
                "ITEM",
                "Item",
                r#"#strict
local departure_count, departure_controller, departure_container;
func Departure(pOldContainer)
{
    departure_count = departure_count + 1;
    departure_controller = GetController();
    departure_container = pOldContainer;
    return 1;
}
"#,
            )?;
            content_definition.set_c4_callback_convention(true);
            engine.register_definition(content_definition)?;

            let mut attached_definition = Definition::from_script(
                "ATCH",
                "Attached",
                r#"#strict
static detach_order;
local marker, deactivate_on_abort, remove_on_abort, abort_count, abort_saw_idle, set_action_shadow_calls;
func ResetDetachOrder() { detach_order = 0; return 1; }
func GetDetachOrder() { return detach_order; }
func SetAction(szAction)
{
    set_action_shadow_calls = set_action_shadow_calls + 1;
    return 0;
}
func AttachAbort()
{
    detach_order = detach_order * 10 + marker;
    abort_count = abort_count + 1;
    abort_saw_idle = ActIdle();
    if (remove_on_abort) RemoveObject(this());
    if (deactivate_on_abort) SetObjectStatus(2, this(), false);
    return 1;
}
"#,
            )?;
            attached_definition.set_c4_callback_convention(true);
            attached_definition.configure_actions(
                Some("Idle".to_string()),
                HashMap::from([
                    ("Idle".to_string(), ActionSpec::default()),
                    (
                        "Attach".to_string(),
                        ActionSpec::default()
                            .with_procedure("ATTACH")
                            .with_abort_call("AttachAbort"),
                    ),
                ]),
            );
            engine.register_definition(attached_definition)?;
            engine.register_definition(simple_definition("ANCR"))?;

            let actor = engine.spawn_object(
                SpawnConfig::new("ACTR")
                    .with_category(CATEGORY_OBJECT)
                    .with_controller(7),
            )?;
            let burner = engine.spawn_object(
                SpawnConfig::new("BURN").with_position(Vector2::new(30, 30)),
            )?;
            let content = engine.spawn_object(
                SpawnConfig::new("ITEM")
                    .with_container(burner)
                    .with_controller(2)
                    .with_rotation(73)
                    .with_rotation_velocity(itofix(20))
                    .with_in_liquid(true)
                    .with_mobile(false)
                    .with_loaded(true),
            )?;
            let anchor = engine.spawn_object(SpawnConfig::new("ANCR"))?;

            let mut attached_action = ActionState::new("Attach");
            attached_action.target = Some(burner);
            attached_action.phase = 6;
            let attached = engine.spawn_object(
                SpawnConfig::new("ATCH")
                    .with_action(attached_action)
                    .with_local_vars(HashMap::from([
                        ("marker".to_string(), Value::Int(1)),
                        ("set_action_shadow_calls".to_string(), Value::Int(0)),
                    ]))
                    .with_loaded(true),
            )?;

            let mut target2_action = ActionState::new("Attach");
            target2_action.target = Some(anchor);
            target2_action.target2 = Some(burner);
            target2_action.phase = 8;
            let target2_attached = engine.spawn_object(
                SpawnConfig::new("ATCH")
                    .with_action(target2_action)
                    .with_local_vars(HashMap::from([
                        ("marker".to_string(), Value::Int(2)),
                        ("set_action_shadow_calls".to_string(), Value::Int(0)),
                    ]))
                    .with_loaded(true),
            )?;

            let mut unrelated_action = ActionState::new("Attach");
            unrelated_action.target = Some(anchor);
            unrelated_action.phase = 4;
            let unrelated = engine.spawn_object(
                SpawnConfig::new("ATCH")
                    .with_action(unrelated_action)
                    .with_local_vars(HashMap::from([(
                        "marker".to_string(),
                        Value::Int(3),
                    )]))
                    .with_loaded(true),
            )?;
            let content_idx = engine.find_object_index(content).expect("content exists");
            engine.objects[content_idx].fixed_velocity =
                FixedVec2::new(itofix(11), itofix(-7));
            engine.objects[content_idx].state.velocity = Vector2::new(11, -7);
            engine.objects[content_idx].fixed_rotation = itofix(73);
            assert_ne!(engine.objects[content_idx].fixed_velocity, FixedVec2::ZERO);
            assert_ne!(engine.objects[content_idx].fixed_rotation, C4Fixed::ZERO);
            let attached_idx = engine.find_object_index(attached).expect("attacher exists");
            engine
                .call_object_function(attached_idx, "ResetDetachOrder", Vec::new())
                .expect("detach order resets");

            if via_script {
                let actor_idx = engine.find_object_index(actor).expect("actor exists");
                assert_eq!(
                    engine.call_object_function(
                        actor_idx,
                        "Ignite",
                        vec![object_reference_value(burner)],
                    )?,
                    Value::Bool(true),
                    "{path} ignition succeeds"
                );
            } else {
                let burner_idx = engine.find_object_index(burner).expect("burner exists");
                assert!(
                    engine.incinerate_object(burner_idx, 7, false, None)?,
                    "{path} ignition succeeds"
                );
            }

            let burner_idx = engine.find_object_index(burner).expect("burner remains");
            assert!(engine.objects[burner_idx].state.contents.is_empty());
            assert_eq!(
                engine.objects[burner_idx]
                    .state
                    .local_vars
                    .get("ejection_count"),
                Some(&Value::Int(1)),
                "{path} uses the Ejection seam"
            );
            assert_eq!(
                engine.objects[burner_idx]
                    .state
                    .local_vars
                    .get("ejection_controller"),
                Some(&Value::Int(7)),
                "{path} updates Controller before Ejection"
            );

            let content_idx = engine.find_object_index(content).expect("content remains");
            let content_state = &engine.objects[content_idx].state;
            assert_eq!(content_state.container, None, "{path} content exits");
            assert_eq!(
                content_state.controller, 7,
                "{path} content receives the fire cause"
            );
            assert_eq!(
                content_state.local_vars.get("departure_count"),
                Some(&Value::Int(1)),
                "{path} runs Departure"
            );
            assert_eq!(
                content_state.local_vars.get("departure_controller"),
                Some(&Value::Int(7)),
                "{path} updates Controller before Exit callbacks"
            );
            assert_eq!(
                content_state.local_vars.get("departure_container"),
                Some(&object_reference_value(burner))
            );
            assert_eq!(content_state.rotation, 0, "{path} uses real Exit");
            assert_eq!(
                engine.objects[content_idx].fixed_velocity,
                FixedVec2::ZERO,
                "{path} Exit clears xdir and ydir"
            );
            assert_eq!(content_state.velocity, Vector2::ZERO);
            assert_eq!(engine.objects[content_idx].fixed_rotation, C4Fixed::ZERO);
            assert_eq!(
                engine.objects[content_idx].rotation_velocity,
                C4Fixed::ZERO,
                "{path} Exit clears rotational velocity"
            );
            assert!(content_state.mobile, "{path} Exit mobilizes content");
            assert!(!content_state.in_liquid, "{path} Exit clears InLiquid");

            for (candidate, label) in [(attached, "Target"), (target2_attached, "Target2")] {
                let candidate_idx = engine
                    .find_object_index(candidate)
                    .expect("matching attacher remains");
                let candidate_state = &engine.objects[candidate_idx].state;
                assert_eq!(
                    candidate_state.action.name, "Idle",
                    "{path} detaches every DFA_ATTACH {label} match"
                );
                assert_eq!(
                    candidate_state.local_vars.get("abort_count"),
                    Some(&Value::Int(1)),
                    "{path} dispatches every Attach AbortCall"
                );
                assert_eq!(
                    candidate_state.local_vars.get("abort_saw_idle"),
                    Some(&Value::Bool(true)),
                    "the idle action is live before AbortCall"
                );
                assert_eq!(
                    candidate_state.local_vars.get("set_action_shadow_calls"),
                    Some(&Value::Int(0)),
                    "native detach bypasses a script SetAction shadow"
                );
            }
            let attached_idx = engine.find_object_index(attached).expect("attacher remains");
            assert_eq!(
                engine
                    .call_object_function(attached_idx, "GetDetachOrder", Vec::new())
                    .expect("detach order reads"),
                Value::Int(21),
                "{path} follows forward main-list order: newer Target2 peer before Target peer"
            );
            assert_eq!(
                engine.objects[attached_idx].state.action.target,
                Some(burner),
                "SetAction(ActIdle) preserves an unsupplied primary target"
            );
            let target2_idx = engine
                .find_object_index(target2_attached)
                .expect("Target2 attacher remains");
            assert_eq!(
                engine.objects[target2_idx].state.action.target2,
                Some(burner),
                "FindObject action-target filtering includes Target2"
            );

            let unrelated_idx = engine
                .find_object_index(unrelated)
                .expect("unrelated attacher remains");
            assert_eq!(engine.objects[unrelated_idx].state.action.name, "Attach");
            assert_eq!(
                engine.objects[unrelated_idx].state.action.target,
                Some(anchor),
                "{path} only detaches objects targeting the burner"
            );

            let mut incomplete_definition = simple_definition("INCO");
            incomplete_definition.set_incomplete_activity(true);
            engine.register_definition(incomplete_definition)?;
            let mut no_decay_definition = simple_definition("NBDC");
            no_decay_definition.set_fire_properties(0, true, false);
            engine.register_definition(no_decay_definition)?;

            for (definition_id, label, marker) in
                [("INCO", "IncompleteActivity", 4), ("NBDC", "NoBurnDecay", 5)]
            {
                let gated_burner = engine.spawn_object(
                    SpawnConfig::new(definition_id)
                        .with_position(Vector2::new(50 + marker * 5, 30)),
                )?;
                let mut gated_action = ActionState::new("Attach");
                gated_action.target = Some(gated_burner);
                let gated_attacher = engine.spawn_object(
                    SpawnConfig::new("ATCH")
                        .with_action(gated_action)
                        .with_local_vars(HashMap::from([(
                            "marker".to_string(),
                            Value::Int(marker),
                        )]))
                        .with_loaded(true),
                )?;

                if via_script {
                    let actor_idx = engine.find_object_index(actor).expect("actor exists");
                    assert_eq!(
                        engine.call_object_function(
                            actor_idx,
                            "Ignite",
                            vec![object_reference_value(gated_burner)],
                        )?,
                        Value::Bool(true)
                    );
                } else {
                    let burner_idx = engine
                        .find_object_index(gated_burner)
                        .expect("gated burner exists");
                    assert!(engine.incinerate_object(burner_idx, 7, false, None)?);
                }

                let attacher_idx = engine
                    .find_object_index(gated_attacher)
                    .expect("gated attacher remains");
                let state = &engine.objects[attacher_idx].state;
                assert_eq!(
                    state.action.name, "Attach",
                    "{path} {label} keeps targeting attachers attached"
                );
                assert_eq!(state.action.target, Some(gated_burner));
                assert!(state
                    .local_vars
                    .get("abort_count")
                    .is_none_or(|value| matches!(value, Value::Nil | Value::Int(0))));
            }

            // C++ performs one fresh FindObject(..., previous) walk per
            // match. If the first attacher's Abort removes that previous
            // object from Game.Objects, the next lookup cannot find its
            // cursor and stops instead of detaching the remaining peer.
            let live_cursor_burner = engine.spawn_object(
                SpawnConfig::new("BURN").with_position(Vector2::new(90, 30)),
            )?;
            let mut tail_action = ActionState::new("Attach");
            tail_action.target = Some(live_cursor_burner);
            let tail_attacher = engine.spawn_object(
                SpawnConfig::new("ATCH")
                    .with_action(tail_action)
                    .with_local_vars(HashMap::from([(
                        "marker".to_string(),
                        Value::Int(7),
                    )]))
                    .with_loaded(true),
            )?;
            let mut cursor_action = ActionState::new("Attach");
            cursor_action.target = Some(live_cursor_burner);
            let cursor_attacher = engine.spawn_object(
                SpawnConfig::new("ATCH")
                    .with_action(cursor_action)
                    .with_local_vars(HashMap::from([
                        ("marker".to_string(), Value::Int(8)),
                        ("deactivate_on_abort".to_string(), Value::Bool(true)),
                    ]))
                    .with_loaded(true),
            )?;
            engine
                .call_object_function(attached_idx, "ResetDetachOrder", Vec::new())
                .expect("detach order resets");

            if via_script {
                let actor_idx = engine.find_object_index(actor).expect("actor exists");
                assert_eq!(
                    engine.call_object_function(
                        actor_idx,
                        "Ignite",
                        vec![object_reference_value(live_cursor_burner)],
                    )?,
                    Value::Bool(true)
                );
            } else {
                let burner_idx = engine
                    .find_object_index(live_cursor_burner)
                    .expect("live-cursor burner exists");
                assert!(engine.incinerate_object(burner_idx, 7, false, None)?);
            }

            let cursor_idx = engine
                .find_object_index(cursor_attacher)
                .expect("cursor attacher remains allocated");
            assert_eq!(
                engine.objects[cursor_idx].state.status,
                ObjectStatus::Inactive,
                "{path} Abort removes the FindObject cursor from the main list"
            );
            assert_eq!(engine.objects[cursor_idx].state.action.name, "Idle");
            assert_eq!(
                engine.objects[cursor_idx].state.local_vars.get("abort_count"),
                Some(&Value::Int(1))
            );
            assert_eq!(
                engine.objects[cursor_idx]
                    .state
                    .local_vars
                    .get("abort_saw_idle"),
                Some(&Value::Bool(true))
            );
            let tail_idx = engine
                .find_object_index(tail_attacher)
                .expect("tail attacher remains");
            assert_eq!(engine.objects[tail_idx].state.status, ObjectStatus::Normal);
            assert_eq!(
                engine.objects[tail_idx].state.action.name, "Attach",
                "{path} stops when the previous FindObject cursor disappears"
            );
            assert_eq!(
                engine.objects[tail_idx].state.action.target,
                Some(live_cursor_burner)
            );
            assert!(engine.objects[tail_idx]
                .state
                .local_vars
                .get("abort_count")
                .is_none_or(|value| matches!(value, Value::Nil | Value::Int(0))));
            assert_eq!(
                engine
                    .call_object_function(attached_idx, "GetDetachOrder", Vec::new())
                    .expect("detach order reads"),
                Value::Int(8)
            );

            // AssignRemoval leaves its Status=Deleted link in Game.Objects
            // until DeleteObjects. Unlike an inactive cursor, that link is
            // still found as pFindNext and iteration continues after it.
            let deleted_cursor_burner = engine.spawn_object(
                SpawnConfig::new("BURN").with_position(Vector2::new(110, 30)),
            )?;
            let mut after_deleted_action = ActionState::new("Attach");
            after_deleted_action.target = Some(deleted_cursor_burner);
            let after_deleted = engine.spawn_object(
                SpawnConfig::new("ATCH")
                    .with_action(after_deleted_action)
                    .with_local_vars(HashMap::from([(
                        "marker".to_string(),
                        Value::Int(9),
                    )]))
                    .with_loaded(true),
            )?;
            let mut deleted_cursor_action = ActionState::new("Attach");
            deleted_cursor_action.target = Some(deleted_cursor_burner);
            let _deleted_cursor = engine.spawn_object(
                SpawnConfig::new("ATCH")
                    .with_action(deleted_cursor_action)
                    .with_local_vars(HashMap::from([
                        ("marker".to_string(), Value::Int(6)),
                        ("remove_on_abort".to_string(), Value::Bool(true)),
                    ]))
                    .with_loaded(true),
            )?;
            engine
                .call_object_function(attached_idx, "ResetDetachOrder", Vec::new())
                .expect("detach order resets");

            if via_script {
                let actor_idx = engine.find_object_index(actor).expect("actor exists");
                assert_eq!(
                    engine.call_object_function(
                        actor_idx,
                        "Ignite",
                        vec![object_reference_value(deleted_cursor_burner)],
                    )?,
                    Value::Bool(true)
                );
            } else {
                let burner_idx = engine
                    .find_object_index(deleted_cursor_burner)
                    .expect("deleted-cursor burner exists");
                assert!(engine.incinerate_object(burner_idx, 7, false, None)?);
            }

            let after_deleted_idx = engine
                .find_object_index(after_deleted)
                .expect("attacher after deleted cursor remains");
            assert_eq!(engine.objects[after_deleted_idx].state.action.name, "Idle");
            assert_eq!(
                engine.objects[after_deleted_idx]
                    .state
                    .local_vars
                    .get("abort_count"),
                Some(&Value::Int(1))
            );
            assert_eq!(
                engine
                    .call_object_function(attached_idx, "GetDetachOrder", Vec::new())
                    .expect("detach order reads"),
                Value::Int(69),
                "{path} continues past a deleted cursor link"
            );
        }
        Ok(())
    }

    #[test]
    fn fire_start_rehomes_contents_through_enter_on_both_paths() -> Result<(), EngineError> {
        // A contained burning object sends its contents through the complete
        // Enter transfer: content RejectEntrance, old-container Ejection,
        // content Departure, destination Collection2, then content Entrance.
        // The fire cause is installed before that transfer; ordinary
        // nonliving Enter then adopts the destination container's controller.
        for via_script in [false, true] {
            let path = if via_script { "compat" } else { "engine" };
            let mut engine = Engine::with_seed(97);
            engine.register_player(PlayerConfig::new(5, "Parent owner"))?;
            engine.register_player(PlayerConfig::new(7, "Incinerator"))?;
            engine.register_definition(Definition::from_script(
                "ACTR",
                "Actor",
                "#strict\nfunc Ignite(pTarget) { return Incinerate(pTarget); }\n",
            )?)?;

            let mut parent_definition = Definition::from_script(
                "PRNT",
                "Parent",
                r#"#strict
func Collection2(pContent) { pContent->Mark(3); return 1; }
"#,
            )?;
            parent_definition.set_c4_callback_convention(true);
            engine.register_definition(parent_definition)?;

            let mut burner_definition = Definition::from_script(
                "BURN",
                "Burner",
                r#"#strict
func Ejection(pContent) { pContent->Mark(1); return 1; }
"#,
            )?;
            burner_definition.set_c4_callback_convention(true);
            engine.register_definition(burner_definition)?;

            let mut content_definition = Definition::from_script(
                "ITEM",
                "Item",
                r#"#strict
local callback_order, reject_container, reject_controller, departure_controller, departure_container, entrance_container;
func Mark(iStep) { callback_order = callback_order * 10 + iStep; return 1; }
func RejectEntrance(pContainer)
{
    Mark(5);
    reject_container = pContainer;
    reject_controller = GetController();
    return 0;
}
func Departure(pOldContainer)
{
    Mark(2);
    departure_controller = GetController();
    departure_container = pOldContainer;
    return 1;
}
func Entrance(pContainer)
{
    Mark(4);
    entrance_container = pContainer;
    return 1;
}
"#,
            )?;
            content_definition.set_c4_callback_convention(true);
            engine.register_definition(content_definition)?;

            let actor = engine.spawn_object(
                SpawnConfig::new("ACTR")
                    .with_category(CATEGORY_OBJECT)
                    .with_controller(7),
            )?;
            let parent = engine.spawn_object(
                SpawnConfig::new("PRNT")
                    .with_position(Vector2::new(80, 40))
                    .with_controller(5),
            )?;
            let burner = engine.spawn_object(
                SpawnConfig::new("BURN")
                    .with_position(Vector2::new(10, 10))
                    .with_container(parent),
            )?;
            let content = engine.spawn_object(
                SpawnConfig::new("ITEM")
                    .with_container(burner)
                    .with_controller(2),
            )?;

            if via_script {
                let actor_idx = engine.find_object_index(actor).expect("actor exists");
                assert_eq!(
                    engine.call_object_function(
                        actor_idx,
                        "Ignite",
                        vec![object_reference_value(burner)],
                    )?,
                    Value::Bool(true)
                );
            } else {
                let burner_idx = engine.find_object_index(burner).expect("burner exists");
                assert!(engine.incinerate_object(burner_idx, 7, false, None)?);
            }

            let content_idx = engine.find_object_index(content).expect("content remains");
            let content_state = &engine.objects[content_idx].state;
            assert_eq!(content_state.container, Some(parent), "{path} enters parent");
            assert_eq!(
                content_state.position,
                Vector2::new(80, 40),
                "{path} copies parent motion"
            );
            assert_eq!(
                content_state.local_vars.get("callback_order"),
                Some(&Value::Int(51234)),
                "{path} runs RejectEntrance -> Ejection -> Departure -> Collection2 -> Entrance"
            );
            assert_eq!(
                content_state.local_vars.get("reject_container"),
                Some(&object_reference_value(parent))
            );
            assert_eq!(
                content_state.local_vars.get("reject_controller"),
                Some(&Value::Int(7)),
                "the fire cause is assigned before RejectEntrance"
            );
            assert_eq!(
                content_state.local_vars.get("departure_controller"),
                Some(&Value::Int(7)),
                "the fire cause is assigned before Enter starts"
            );
            assert_eq!(
                content_state.local_vars.get("departure_container"),
                Some(&object_reference_value(burner))
            );
            assert_eq!(
                content_state.local_vars.get("entrance_container"),
                Some(&object_reference_value(parent))
            );
            assert_eq!(
                content_state.controller, 5,
                "nonliving Enter finally adopts the parent controller"
            );
        }
        Ok(())
    }

    #[test]
    fn contained_burn_turn_to_runs_the_change_def_lifecycle_on_both_fire_paths(
    ) -> Result<(), EngineError> {
        // FnFxFireStart calls ChangeDef before ejecting contents
        // (C4Effect.cpp:579-594). A contained object therefore silently
        // exits at (0,0), resets its old action, swaps definitions, asks the
        // NEW RejectEntrance and, when accepted, silently re-enters the saved
        // parent at the Unsorted stContents tail (C4Object.cpp:1207-1254).
        for via_script in [false, true] {
            for reject_reentry in [false, true] {
                let path = if via_script { "compat" } else { "engine" };
                let reentry = if reject_reentry { "veto" } else { "accept" };
                let label = format!("{path}/{reentry}");
                let mut engine = Engine::with_seed(98);
                engine.register_player(PlayerConfig::new(5, "Parent owner"))?;
                engine.register_player(PlayerConfig::new(7, "Incinerator"))?;
                engine.register_definition(Definition::from_script(
                    "ACTR",
                    "Actor",
                    "#strict\nfunc Ignite(pTarget) { return Incinerate(pTarget); }\n",
                )?)?;

                let mut parent_definition = Definition::from_script(
                    "PRNT",
                    "Parent",
                    r#"#strict
local ejection_count, collection_count;
func Ejection(pObject) { ejection_count += 1; return(1); }
func Collection2(pObject) { collection_count += 1; return(1); }
"#,
                )?;
                parent_definition.set_c4_callback_convention(true);
                engine.register_definition(parent_definition)?;

                let mut burner_definition = Definition::from_script(
                    "BURN",
                    "Burner",
                    r#"#strict
local abort_count, departure_count;
func OldAbort() { abort_count += 1; return(1); }
func Departure(pContainer) { departure_count += 1; return(1); }
"#,
                )?;
                burner_definition.set_c4_callback_convention(true);
                burner_definition.set_category(CATEGORY_OBJECT);
                burner_definition.set_burn_turn_to(Some("ASH1".to_string()));
                burner_definition.configure_actions(
                    Some("Idle".to_string()),
                    HashMap::from([
                        ("Idle".to_string(), ActionSpec::default()),
                        (
                            "Work".to_string(),
                            ActionSpec::default().with_abort_call("OldAbort"),
                        ),
                    ]),
                );
                engine.register_definition(burner_definition)?;

                let reject_result = i32::from(reject_reentry);
                let mut ash_definition = Definition::from_script(
                    "ASH1",
                    "Ash",
                    &format!(
                        r#"#strict
local reject_count, entrance_count, ejection_count;
func RejectEntrance(pContainer) {{ reject_count += 1; return({reject_result}); }}
func Entrance(pContainer) {{ entrance_count += 1; return(1); }}
func Ejection(pObject) {{ ejection_count += 1; return(1); }}
"#
                    ),
                )?;
                ash_definition.set_c4_callback_convention(true);
                ash_definition.set_category(CATEGORY_STRUCTURE);
                ash_definition.set_rotateable(1);
                engine.register_definition(ash_definition)?;

                let mut peer_definition = simple_definition("PEER");
                peer_definition.set_category(CATEGORY_VEHICLE);
                engine.register_definition(peer_definition)?;
                let mut content_definition = Definition::from_script(
                    "ITEM",
                    "Item",
                    r#"#strict
local departure_count, entrance_count;
func Departure(pContainer) { departure_count += 1; return(1); }
func Entrance(pContainer) { entrance_count += 1; return(1); }
"#,
                )?;
                content_definition.set_c4_callback_convention(true);
                content_definition.set_category(CATEGORY_VEHICLE);
                engine.register_definition(content_definition)?;

                let actor = engine.spawn_object(
                    SpawnConfig::new("ACTR")
                        .with_category(CATEGORY_OBJECT)
                        .with_controller(7),
                )?;
                let parent_position = Vector2::new(80, 40);
                let parent_velocity = FixedVec2::new(
                    C4Fixed::from_raw(itofix(3).val() + 321),
                    C4Fixed::from_raw(itofix(-4).val() - 654),
                );
                let parent = engine.spawn_object(
                    SpawnConfig::new("PRNT")
                        .with_position(parent_position)
                        .with_fixed_position(FixedVec2::new(
                            C4Fixed::from_raw(itofix(80).val() + 123),
                            C4Fixed::from_raw(itofix(40).val() + 456),
                        ))
                        .with_fixed_velocity(parent_velocity)
                        .with_rotation_velocity(C4Fixed::from_raw(777))
                        .with_controller(5)
                        .with_local_vars(HashMap::from([
                            ("ejection_count".to_string(), Value::Int(0)),
                            ("collection_count".to_string(), Value::Int(0)),
                        ])),
                )?;
                let peer_a = engine.spawn_object(
                    SpawnConfig::new("PEER")
                        .with_category(CATEGORY_VEHICLE)
                        .with_container(parent),
                )?;
                let peer_b = engine.spawn_object(
                    SpawnConfig::new("PEER")
                        .with_category(CATEGORY_VEHICLE)
                        .with_container(parent),
                )?;
                let burner = engine.spawn_object(
                    SpawnConfig::new("BURN")
                        .with_category(CATEGORY_VEHICLE)
                        .with_container(parent)
                        .with_action(ActionState::new("Work"))
                        .with_local_vars(HashMap::from([
                            ("abort_count".to_string(), Value::Int(0)),
                            ("departure_count".to_string(), Value::Int(0)),
                            ("reject_count".to_string(), Value::Int(0)),
                            ("entrance_count".to_string(), Value::Int(0)),
                            ("ejection_count".to_string(), Value::Int(0)),
                        ])),
                )?;
                let content = engine.spawn_object(
                    SpawnConfig::new("ITEM")
                        .with_category(CATEGORY_VEHICLE)
                        .with_container(burner)
                        .with_controller(2)
                        .with_local_vars(HashMap::from([
                            ("departure_count".to_string(), Value::Int(0)),
                            ("entrance_count".to_string(), Value::Int(0)),
                        ])),
                )?;

                let parent_idx = engine.find_object_index(parent).expect("parent exists");
                assert_eq!(
                    engine.objects[parent_idx].state.contents,
                    vec![burner, peer_b, peer_a],
                    "{label}: fixture starts with the burner at the sorted front"
                );
                let burner_idx = engine.find_object_index(burner).expect("burner exists");
                {
                    let burner = &mut engine.objects[burner_idx];
                    burner.fixed_velocity = FixedVec2::new(itofix(8), itofix(9));
                    burner.state.velocity = Vector2::new(8, 9);
                    burner.state.rotation = 27;
                    burner.fixed_rotation = itofix(27);
                    burner.rotation_velocity = itofix(6);
                    burner.state.mobile = false;
                    burner.state.in_liquid = true;
                }

                if via_script {
                    let actor_idx = engine.find_object_index(actor).expect("actor exists");
                    assert_eq!(
                        engine.call_object_function(
                            actor_idx,
                            "Ignite",
                            vec![object_reference_value(burner)],
                        )?,
                        Value::Bool(true),
                        "{label}: ignition succeeds"
                    );
                } else {
                    let burner_idx = engine.find_object_index(burner).expect("burner exists");
                    assert!(
                        engine.incinerate_object(burner_idx, 7, false, None)?,
                        "{label}: ignition succeeds"
                    );
                }

                let burner_idx = engine.find_object_index(burner).expect("burner remains");
                let burner_state = &engine.objects[burner_idx];
                assert_eq!(burner_state.definition_id, "ASH1", "{label}: BurnTurnTo");
                assert_eq!(
                    burner_state.state.category, CATEGORY_VEHICLE,
                    "{label}: ChangeDef preserves the object's category"
                );
                assert!(burner_state.state.on_fire, "{label}: fire starts");
                assert!(
                    burner_state.state.contents.is_empty(),
                    "{label}: content leaves"
                );
                for (name, expected, message) in [
                    ("abort_count", 1, "old-action AbortCall runs exactly once"),
                    ("departure_count", 0, "silent Exit suppresses Departure"),
                    ("reject_count", 1, "new RejectEntrance runs exactly once"),
                    ("entrance_count", 0, "silent Enter suppresses Entrance"),
                    (
                        "ejection_count",
                        1,
                        "only the later content Exit calls Ejection",
                    ),
                ] {
                    assert_eq!(
                        burner_state.state.local_vars.get(name),
                        Some(&Value::Int(expected)),
                        "{label}: {message}"
                    );
                }
                assert_eq!(
                    burner_state.state.rotation, 0,
                    "{label}: silent Exit clears r"
                );
                assert_eq!(burner_state.fixed_rotation, C4Fixed::ZERO);
                assert_eq!(
                    burner_state.rotation_velocity,
                    C4Fixed::ZERO,
                    "{label}: CopyMotion does not replace the cleared rdir"
                );
                assert!(burner_state.state.mobile, "{label}: silent Exit mobilizes");
                assert!(
                    !burner_state.state.in_liquid,
                    "{label}: silent Exit clears InLiquid"
                );

                let parent_idx = engine.find_object_index(parent).expect("parent remains");
                let parent_state = &engine.objects[parent_idx].state;
                assert_eq!(
                    parent_state.local_vars.get("ejection_count"),
                    Some(&Value::Int(0)),
                    "{label}: silent ChangeDef exit does not call parent Ejection"
                );
                assert_eq!(
                    parent_state.local_vars.get("collection_count"),
                    Some(&Value::Int(i32::from(!reject_reentry))),
                    "{label}: only an accepted content transfer calls Collection2"
                );

                let content_idx = engine.find_object_index(content).expect("content remains");
                let content_object = &engine.objects[content_idx];
                let content_state = &content_object.state;
                assert_eq!(
                    content_state.local_vars.get("departure_count"),
                    Some(&Value::Int(1)),
                    "{label}: fire ejection runs Departure once"
                );
                assert_eq!(
                    content_state.local_vars.get("entrance_count"),
                    Some(&Value::Int(i32::from(!reject_reentry))),
                    "{label}: content Entrance follows only an accepted parent transfer"
                );

                if reject_reentry {
                    assert_eq!(
                        burner_state.state.container, None,
                        "{label}: veto leaves outside"
                    );
                    assert_eq!(burner_state.state.position, Vector2::ZERO);
                    assert_eq!(burner_state.fixed_position, FixedVec2::ZERO);
                    assert_eq!(burner_state.fixed_velocity, FixedVec2::ZERO);
                    assert_eq!(parent_state.contents, vec![peer_b, peer_a]);
                    assert_eq!(
                        content_state.container, None,
                        "{label}: content exits to world"
                    );
                    assert_eq!(content_state.position, parent_position);
                    assert_eq!(content_object.fixed_velocity, FixedVec2::ZERO);
                    assert_eq!(
                        content_state.controller, 7,
                        "{label}: fire cause is retained"
                    );
                } else {
                    assert_eq!(burner_state.state.container, Some(parent));
                    assert_eq!(burner_state.state.position, parent_position);
                    assert_eq!(
                        burner_state.fixed_position,
                        FixedVec2::from_ints(parent_position.x, parent_position.y),
                        "{label}: CopyMotion snaps fix_x/fix_y to integer parent position"
                    );
                    assert_eq!(burner_state.fixed_velocity, parent_velocity);
                    assert_eq!(
                        parent_state.contents,
                        vec![content, peer_b, peer_a, burner],
                        "{label}: content sorts first and Unsorted burner remains at tail"
                    );
                    assert_eq!(content_state.container, Some(parent));
                    assert_eq!(content_state.position, parent_position);
                    assert_eq!(content_object.fixed_velocity, parent_velocity);
                    assert_eq!(
                        content_state.controller, 5,
                        "{label}: parent adopts content"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn incinerate_object_matches_cpp_start_semantics() -> Result<(), EngineError> {
        // C4Object::Incinerate (C4Object.cpp:1230-1241) + fxFireStart core
        // (C4Effect.cpp:560-641): already burning → false; dead livings don't
        // burn; in extinguishing material → no fire and NO FirePhase draw
        // (the extinguisher check precedes it); otherwise OnFire is set and
        // FirePhase = Random(MaxFirePhase) consumes one synced draw
        // (C4Effect.cpp:633-634, MaxFirePhase = 15).
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
            Extinguisher=-1

            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(70);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(40, 30, Some(earth)));
        engine.register_definition(simple_definition("Tree"))?;
        let tree =
            engine.spawn_object(SpawnConfig::new("Tree").with_position(Vector2::new(10, 10)))?;
        let idx = engine.find_object_index(tree).expect("tree exists");

        let mut mirror = engine.rng.clone();
        let expected_phase = mirror.random(15);
        assert!(engine.incinerate_object(idx, 1, false, None)?);
        assert!(engine.objects[idx].state.on_fire);
        assert_eq!(engine.objects[idx].state.fire_phase, expected_phase);
        assert_eq!(engine.objects[idx].state.fire_caused_by, 1);
        assert_eq!(engine.rng, mirror, "one FirePhase draw");

        // already burning → false, no draw (C4Object.cpp:1233)
        assert!(!engine.incinerate_object(idx, 2, false, None)?);
        assert_eq!(engine.rng, mirror);
        assert_eq!(engine.objects[idx].state.fire_caused_by, 1);

        // dead living → false (C4Object.cpp:1235)
        let mut dead_def = simple_definition("Corpse");
        dead_def.set_crew_member(true);
        dead_def.set_category(CATEGORY_LIVING);
        engine.register_definition(dead_def)?;
        let corpse = engine.spawn_object(
            SpawnConfig::new("Corpse")
                .with_position(Vector2::new(20, 10))
                .with_alive(false),
        )?;
        let corpse_idx = engine.find_object_index(corpse).expect("corpse exists");
        assert!(!engine.incinerate_object(corpse_idx, 1, false, None)?);
        assert!(!engine.objects[corpse_idx].state.on_fire);

        // Submerged in extinguisher material: the constructor still hands
        // back the allocated number (so Incinerate succeeds), while the
        // denied Start leaves no live fire state and consumes no draw
        // (C4Effect.cpp:128-133, 574-583).
        if let Some(landscape) = engine.landscape.as_mut() {
            landscape.set_liquid_column(30, vec![LiquidSegment::with_material(5, 12, Some(water))]);
        }
        let soaked =
            engine.spawn_object(SpawnConfig::new("Tree").with_position(Vector2::new(30, 8)))?;
        let soaked_idx = engine.find_object_index(soaked).expect("soaked exists");
        let mirror = engine.rng.clone();
        assert!(engine.incinerate_object(soaked_idx, 1, false, None)?);
        assert!(!engine.objects[soaked_idx].state.on_fire);
        assert_eq!(engine.rng, mirror, "no draw when extinguished at start");
        Ok(())
    }

    #[test]
    fn incinerate_creates_fire_effect_entry_like_cpp() -> Result<(), EngineError> {
        // C4Object::Incinerate (C4Object.cpp:1257-1266): fire is a real
        // C4Effect entry — `new C4Effect(this, C4Fx_Fire "Fire",
        // C4Fx_FirePriority 100, C4Fx_FireTimer 1, ...)` — whose vars carry
        // [FireMode, CausedBy, Blasted, IncineratingObject]
        // (C4Effect.cpp:628-631). The mode comes from the ~FireMode script
        // answer, else the category default (C4Effect.cpp:609-626;
        // C4Effects.h:70-74: StructVeh=1, LivingVeg=2, Object=3).
        let mut engine = Engine::with_seed(23);
        engine.register_definition(simple_definition("Bush"))?;
        let bush =
            engine.spawn_object(SpawnConfig::new("Bush").with_position(Vector2::new(10, 10)))?;
        let idx = engine.find_object_index(bush).expect("bush exists");
        assert!(engine.incinerate_object(idx, 3, true, None)?);
        let fire = {
            let effects = &engine.objects[idx].state.effects;
            assert_eq!(effects.len(), 1, "exactly one Fire effect entry");
            effects[0].clone()
        };
        assert_eq!(fire.name, "Fire", "C4Fx_Fire");
        assert_eq!(fire.priority, 100, "C4Fx_FirePriority");
        assert_eq!(fire.interval, 1, "C4Fx_FireTimer");
        assert!(fire.number > 0, "allocated per-object number");
        assert_eq!(
            fire.vars(),
            &[
                // default StaticBack category → C4Fx_FireMode_LivingVeg
                EffectVarValue::Int(2),
                EffectVarValue::Int(3),
                EffectVarValue::Bool(true),
                EffectVarValue::Nil,
            ],
            "FxFireVar Mode/CausedBy/Blasted/IncineratingObj"
        );

        // vehicles default to C4Fx_FireMode_StructVeh (C4Effect.cpp:617-618);
        // the incinerating object rides var 3 (C4Effect.cpp:631).
        let mut cart_def = simple_definition("Cart");
        cart_def.set_category(CATEGORY_VEHICLE);
        engine.register_definition(cart_def)?;
        let cart =
            engine.spawn_object(SpawnConfig::new("Cart").with_position(Vector2::new(20, 10)))?;
        let cart_idx = engine.find_object_index(cart).expect("cart exists");
        assert!(engine.incinerate_object(cart_idx, 1, false, Some(bush))?);
        let cart_fire = engine.objects[cart_idx].state.effects[0].clone();
        assert_eq!(cart_fire.vars()[0], EffectVarValue::Int(1), "StructVeh");
        assert_eq!(
            cart_fire.vars()[3],
            EffectVarValue::Object(bush.as_u64()),
            "incinerating object stored"
        );

        // a ~FireMode script answer is read through C4Value::getInt
        // (C4Effect.cpp:611). A raw Bool retains its full Data.Int payload;
        // this out-of-range seven therefore falls back to Object mode
        // (C4Effect.cpp:622-626), rather than being canonicalized to one.
        let hot_def = Definition::from_script(
            "Torch",
            "Torch",
            r#"
            func FireMode() { return CastBool(7); }
            "#,
        )
        .expect("script compiles");
        engine.register_definition(hot_def)?;
        let torch =
            engine.spawn_object(SpawnConfig::new("Torch").with_position(Vector2::new(30, 10)))?;
        let torch_idx = engine.find_object_index(torch).expect("torch exists");
        assert!(engine.incinerate_object(torch_idx, 1, false, None)?);
        assert_eq!(
            engine.objects[torch_idx].state.effects[0].vars()[0],
            EffectVarValue::Int(3),
            "out-of-range raw-Bool FireMode callback answer"
        );

        // refused incinerations leave no entry: a repeat is denied by the
        // already-burning check (C4Object.cpp:1259) …
        assert!(!engine.incinerate_object(idx, 5, false, None)?);
        assert_eq!(engine.objects[idx].state.effects.len(), 1, "no duplicate");
        Ok(())
    }

    #[test]
    fn fire_burns_through_the_effect_timer_exactly_once_per_frame() -> Result<(), EngineError> {
        // The burn is driven by the fire effect's own timer — priority 100,
        // interval 1 (C4Object::Incinerate, C4Object.cpp:1263-1265) —
        // whose FnFxFireTimer calls C4Object::ExecFire once per elapsed
        // frame (C4Effect.cpp:643-658; iTime advance C4Effect.cpp:339-342).
        // The entry must survive the tick: the engine timer exists, so the
        // "no timer function: mark dead" arm (C4Effect.cpp:358-360) must
        // NOT fire.
        let mut engine = Engine::with_seed(29);
        engine.register_definition(simple_definition("Hut"))?;
        let hut =
            engine.spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(10, 10)))?;
        let idx = engine.find_object_index(hut).expect("hut exists");
        assert!(engine.incinerate_object(idx, 1, false, None)?);
        let con_before = engine.objects[idx].state.construction;
        let phase_before = engine.objects[idx].state.fire_phase;
        engine.tick_without_snapshot()?;
        let idx = engine.find_object_index(hut).expect("hut survives");
        assert_eq!(
            engine.objects[idx].state.construction,
            con_before - 100,
            "DoCon(-100) exactly once per frame (C4Object.cpp:779-781)"
        );
        assert_eq!(
            engine.objects[idx].state.fire_phase,
            (phase_before + 1) % 15,
            "FirePhase advances once (C4Object.cpp:770)"
        );
        let fire = engine.objects[idx]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Fire")
            .cloned()
            .expect("fire effect survives its timer");
        assert_eq!(fire.timer, 1, "iTime elapsed once");
        engine.tick_without_snapshot()?;
        let idx = engine.find_object_index(hut).expect("hut survives");
        assert_eq!(
            engine.objects[idx].state.construction,
            con_before - 200,
            "second frame burns exactly once more"
        );
        Ok(())
    }

    #[test]
    fn fire_decay_runs_docon_shape_and_bottom_update() -> Result<(), EngineError> {
        // ExecFire delegates decay to DoCon(-100) (src/C4Object.cpp:776-778).
        // DoCon then runs UpdateFace(true) and keeps a straight object's old
        // shape bottom fixed (src/C4Object.cpp:1414-1483). Starting from a
        // four-pixel full-con shape, the first decay crosses from construction
        // step 100 to 99: Jolt shrinks the shape/vertex to three pixels and the
        // integer center moves down one pixel so the bottom remains at y=8.
        // Crossing full -> partial first SetAction("Idle")s at y=4, which
        // resynchronizes fix_y; the later UpdatePos changes only integer y.
        let mut definition = simple_definition("BurningStructure");
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 2, 4)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 4).with_cnat(CNAT_BOTTOM)]);
        definition.set_components(vec![DefinitionComponent {
            id: "WOOD".to_string(),
            count: 2,
        }]);

        let mut engine = Engine::with_seed(29);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("BurningStructure")
                .with_position(Vector2::new(3, 8))
                .with_construction(FULL_CON),
        )?;
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].state.position.y, 4);
        assert!(engine.incinerate_object(idx, 1, false, None)?);

        engine.tick_without_snapshot()?;

        let object = engine.object_snapshot(id).expect("object survives");
        assert_eq!(object.construction, FULL_CON - 100);
        assert_eq!(object.position.y, 5, "DoCon preserves shape bottom");
        assert_eq!(object.vertices[0].y, 3, "UpdateFace jolts vertices");
        assert_eq!(object.components.get("WOOD"), Some(&1));
        let idx = engine.find_object_index(id).expect("object survives");
        assert_eq!(
            engine.objects[idx].fixed_position.y,
            itofix(4),
            "SetAction resynchronizes before UpdatePos preserves fixed y"
        );
        Ok(())
    }

    #[test]
    fn fire_timer_extinguishes_in_extinguisher_material_and_kills_the_effect(
    ) -> Result<(), EngineError> {
        // ExecFire's Tick5 background arm (C4Object.cpp:797-806) calls
        // C4Object::Extinguish in extinguishing material, which kills the
        // fire effect (C4Object.cpp:1269-1301) — FnFxFireStop clears the
        // OnFire flag (C4Effect.cpp:787). The Random(3) inflame draw still
        // runs after the extinguish (C4Object.cpp:803-804). Cover both the
        // direct native timer and a global script override that reaches the
        // engine FxFireTimer through inherited().
        fn run(inherited_timer: bool) -> Result<(), EngineError> {
            let library = MaterialLibrary::parse(
                r#"
                [Material Water]
                Name=Water
                Density=25
                Friction=0
                Extinguisher=-1
                "#,
            )
            .expect("material library parses");
            let materials = MaterialSet::from_resource_library(&library);
            let water = materials.id_of("Water").expect("water exists");
            let mut engine = Engine::with_seed(31);
            engine.set_materials(materials);
            let mut hut_definition = if inherited_timer {
                let mut definition = Definition::from_script(
                    "Hut",
                    "Hut",
                    "#strict\nglobal func FxFireTimer(target, number, time) { return inherited(target, number, time); }\n",
                )?;
                definition.set_c4_callback_convention(true);
                definition
            } else {
                simple_definition("Hut")
            };
            hut_definition.set_physical(PhysicalInfo {
                energy: 100_000,
                ..PhysicalInfo::default()
            });
            engine.register_definition(hut_definition)?;
            let hut = engine
                .spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(10, 10)))?;
            let idx = engine.find_object_index(hut).expect("hut exists");
            assert!(engine.incinerate_object(idx, 1, false, None)?);

            // Flood the spot after ignition, then run to the next Tick5.
            let mut landscape = Landscape::flat_with_material(40, 30, None);
            landscape
                .set_liquid_column(10, vec![LiquidSegment::with_material(5, 12, Some(water))]);
            engine.set_landscape(landscape);
            while engine.frame % 5 != 4 {
                engine.tick_without_snapshot()?;
            }
            engine.tick_without_snapshot()?;

            let path = if inherited_timer { "inherited" } else { "native" };
            let idx = engine.find_object_index(hut).expect("hut survives");
            assert!(!engine.objects[idx].state.on_fire, "{path}: extinguished");
            if inherited_timer {
                assert!(
                    engine.objects[idx]
                        .state
                        .effects
                        .iter()
                        .any(|effect| effect.name == "Fire" && effect.priority == 0),
                    "inherited: the killed callback node remains linked dead until execute",
                );
                engine.tick_without_snapshot()?;
            }
            let idx = engine.find_object_index(hut).expect("hut survives");
            assert!(
                !engine.objects[idx]
                    .state
                    .effects
                    .iter()
                    .any(|effect| effect.name == "Fire"),
                "{path}: the fire effect was killed by the extinguish",
            );
            Ok(())
        }

        run(false)?;
        run(true)
    }

    #[test]
    fn fire_timer_extinguishes_living_in_valid_base_on_tick5() -> Result<(), EngineError> {
        // ExecFire advances FirePhase, then its Tick5 base arm extinguishes a
        // living object contained in an object whose Base names any present
        // player. Extinguish does not return from ExecFire: that frame still
        // performs decay, Tick10 damage and Tick5 energy
        // (C4Object.cpp:768-785). Cover both the native timer and a script
        // overload that reaches the same engine function via inherited().
        let mut native_definition = Definition::from_script(
            "NativeBaseFire",
            "Native base fire",
            r#"#strict
local abort_saw_fire, damage_saw_fire;
func WorkAbort()
{
    abort_saw_fire = !!GetEffect("Fire", this());
    return 1;
}
func Damage(int change, int caused_by)
{
    damage_saw_fire = !!GetEffect("Fire", this());
    return 1;
}
"#,
        )?;
        native_definition.set_c4_callback_convention(true);
        native_definition.set_category(CATEGORY_LIVING);
        native_definition.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        native_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Work".to_string(),
                    ActionSpec::default().with_abort_call("WorkAbort"),
                ),
            ]),
        );
        let mut inherited_definition = Definition::from_script(
            "InheritedBaseFire",
            "Inherited base fire",
            r#"#strict
local abort_saw_fire, damage_saw_fire;
func WorkAbort()
{
    abort_saw_fire = !!GetEffect("Fire", this());
    return 1;
}
func Damage(int change, int caused_by)
{
    damage_saw_fire = !!GetEffect("Fire", this());
    return 1;
}
func FxFireTimer(object target, int number, int time)
{
    return inherited(target, number, time);
}
"#,
        )?;
        inherited_definition.set_c4_callback_convention(true);
        inherited_definition.set_category(CATEGORY_LIVING);
        inherited_definition.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        inherited_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Work".to_string(),
                    ActionSpec::default().with_abort_call("WorkAbort"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(33);
        engine.register_player(PlayerConfig::new(7, "Base owner"))?;
        engine.register_definition(simple_definition("Base"))?;
        engine.register_definition(native_definition)?;
        engine.register_definition(inherited_definition)?;
        let base = engine.spawn_object(SpawnConfig::new("Base"))?;
        engine.apply_object_update(base, ObjectUpdate::new().with_base(7))?;
        let mut burning = Vec::new();
        for definition in ["NativeBaseFire", "InheritedBaseFire"] {
            let id = engine.spawn_object(
                SpawnConfig::new(definition)
                    .with_category(CATEGORY_LIVING)
                    .with_container(base)
                    .with_energy(100_000),
            )?;
            let idx = engine.find_object_index(id).expect("living object exists");
            assert!(engine.incinerate_object(idx, 7, false, None)?);
            burning.push((id, definition));
        }

        // Keep the scenario bit disabled across the first Tick5 pulse, then
        // enable it for frame 10 so the extinguish and every remaining burn
        // arm execute in the same observable frame.
        engine.set_base_extinguish_enabled(false);
        while engine.frame() < 9 {
            engine.tick_without_snapshot()?;
        }
        for (id, path) in &burning {
            let object = engine.object_snapshot(*id).expect("burning object remains");
            assert!(object.on_fire, "{path} respects a disabled base bit");
            assert_eq!(object.container, Some(base));
            engine.apply_object_update(
                *id,
                ObjectUpdate::new()
                    .with_construction(FULL_CON)
                    .with_action("Work"),
            )?;
        }
        let before = burning
            .iter()
            .map(|(id, _)| {
                let object = engine.object_snapshot(*id).expect("object remains");
                (
                    *id,
                    (
                        object.construction,
                        object.damage,
                        object.energy,
                        object.fire_phase,
                    ),
                )
            })
            .collect::<HashMap<_, _>>();

        engine.set_base_extinguish_enabled(true);
        engine.tick_without_snapshot()?;
        assert_eq!(engine.frame(), 10);

        for (id, path) in burning {
            let object = engine.object_snapshot(id).expect("extinguished object remains");
            assert!(
                !object.on_fire,
                "{path} extinguishes in its valid base; effects={:?}",
                object.effects
            );
            assert!(
                !object
                    .effects
                    .iter()
                    .any(|effect| effect.name == "Fire" && effect.priority != 0),
                "{path} kills the numbered Fire effect"
            );
            let (construction, damage, energy, fire_phase) = before[&id];
            assert_eq!(
                object.construction,
                construction - 100,
                "{path} still runs decay after the base extinguish"
            );
            assert_eq!(
                object.damage,
                damage + 2,
                "{path} still runs Tick10 damage after the base extinguish"
            );
            assert_eq!(
                object.energy,
                energy - 1_000,
                "{path} still runs Tick5 energy after the base extinguish"
            );
            assert_eq!(
                object.fire_phase,
                (fire_phase + 1) % MAX_FIRE_PHASE,
                "{path} retains the phase advance that precedes extinguish"
            );
            assert_eq!(
                object.local_vars.get("abort_saw_fire"),
                Some(&Value::Bool(false)),
                "{path} extinguishes before DoCon's action callback"
            );
            assert_eq!(
                object.local_vars.get("damage_saw_fire"),
                Some(&Value::Bool(false)),
                "{path} extinguishes before Tick10 damage"
            );
        }
        Ok(())
    }

    #[test]
    fn incinerate_and_extinguish_script_functions_manage_the_fire_effect(
    ) -> Result<(), EngineError> {
        // FnIncinerate (C4Script.cpp:245-252): the target defaults to the
        // caller and iCausedBy is the CALLING object's controller.
        // FnExtinguish (C4Script.cpp:264-270) extinguishes all fires via
        // C4Object::Extinguish(0) (C4Object.cpp:1269-1301) — killing the
        // "Fire" effect and clearing OnFire through the engine-internal
        // FnFxFireStop (C4Effect.cpp:787).
        let mut engine = Engine::with_seed(37);
        engine.register_definition(
            Definition::from_script(
                "ACTR",
                "Actor",
                "#strict\nfunc Ignite(pVictim) { return Incinerate(pVictim); }\nfunc Quench(pVictim) { return Extinguish(pVictim); }\n",
            )
            .expect("actor compiles"),
        )?;
        engine.register_definition(simple_definition("Hut"))?;
        let actor =
            engine.spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))?;
        let hut = engine.spawn_object(SpawnConfig::new("Hut"))?;
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_idx].state.controller = 5;
        let hut_value = Value::Object(hut.as_u64());

        let result =
            engine.call_object_function(actor_idx, "Ignite", vec![hut_value.clone()])?;
        assert_eq!(result, Value::Bool(true), "Incinerate reports success");
        let hut_idx = engine.find_object_index(hut).expect("hut exists");
        assert!(engine.objects[hut_idx].state.on_fire);
        assert_eq!(
            engine.objects[hut_idx].state.fire_caused_by, 5,
            "caused by the caller's controller"
        );
        assert!(engine.objects[hut_idx]
            .state
            .effects
            .iter()
            .any(|effect| effect.name == "Fire"));

        let result = engine.call_object_function(actor_idx, "Quench", vec![hut_value])?;
        assert_eq!(result, Value::Bool(true), "Extinguish reports success");
        let hut_idx = engine.find_object_index(hut).expect("hut exists");
        assert!(!engine.objects[hut_idx].state.on_fire, "flag cleared");
        assert!(
            engine.objects[hut_idx]
                .state
                .effects
                .iter()
                .any(|effect| effect.name == "Fire" && effect.priority == 0),
            "the killed Fire node stays linked dead"
        );
        engine.tick_without_snapshot()?;
        let hut_idx = engine.find_object_index(hut).expect("hut exists");
        assert!(!engine.objects[hut_idx]
            .state
            .effects
            .iter()
            .any(|effect| effect.name == "Fire"));
        Ok(())
    }

    #[test]
    fn remove_effect_fire_extinguishes_like_the_engine_fire_stop() -> Result<(), EngineError> {
        // RemoveEffect("Fire", obj) → C4Effect::Kill → the engine-internal
        // FnFxFireStop clears OnFire (C4Effect.cpp:787); with fDoNoCalls
        // the Stop is skipped and the flag survives (FnRemoveEffect,
        // C4Script.cpp:5493-5507).
        let mut engine = Engine::with_seed(41);
        engine.register_definition(
            Definition::from_script(
                "ACTR",
                "Actor",
                "#strict\nfunc Douse(pVictim) { return RemoveEffect(\"Fire\", pVictim); }\n",
            )
            .expect("actor compiles"),
        )?;
        engine.register_definition(simple_definition("Hut"))?;
        let actor =
            engine.spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))?;
        let hut = engine.spawn_object(SpawnConfig::new("Hut"))?;
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        let hut_idx = engine.find_object_index(hut).expect("hut exists");
        assert!(engine.incinerate_object(hut_idx, 1, false, None)?);
        let result = engine.call_object_function(
            actor_idx,
            "Douse",
            vec![Value::Object(hut.as_u64())],
        )?;
        assert_eq!(result, Value::Bool(true));
        let hut_idx = engine.find_object_index(hut).expect("hut exists");
        assert!(!engine.objects[hut_idx].state.on_fire, "OnFire cleared");
        assert!(engine.objects[hut_idx]
            .state
            .effects
            .iter()
            .any(|effect| effect.name == "Fire" && effect.priority == 0));
        engine.tick_without_snapshot()?;
        let hut_idx = engine.find_object_index(hut).expect("hut exists");
        assert!(!engine.objects[hut_idx]
            .state
            .effects
            .iter()
            .any(|effect| effect.name == "Fire"));
        Ok(())
    }

    #[test]
    fn global_fx_fire_timer_overload_shadows_and_chains_to_the_engine() -> Result<(), EngineError>
    {
        // FxFire* are engine functions (AddFunc, C4Script.cpp:6994-6997): a
        // GLOBAL FxFireTimer overloads them — internal Fire effects have no
        // command target and resolve from Game.ScriptEngine. A definition-
        // local same-name function is invisible there (C4Effect.cpp:31-56).
        // The burn only runs when the global overload chains via inherited.
        let mut engine = Engine::with_seed(43);
        let mut barn_definition = Definition::from_script(
            "BARN",
            "Barn",
            "#strict\nglobal func FxFireTimer(pObj, iNumber, iTime) { if (GetID(pObj) == BARN) return inherited(pObj, iNumber, iTime); return 0; }\n",
        )
        .expect("barn compiles");
        barn_definition.set_c4_callback_convention(true);
        engine.register_definition(barn_definition)?;
        engine.register_definition(
            Definition::from_script("SHED", "Shed", "").expect("shed compiles"),
        )?;
        let barn = engine.spawn_object(SpawnConfig::new("BARN"))?;
        let shed = engine.spawn_object(SpawnConfig::new("SHED"))?;
        let barn_idx = engine.find_object_index(barn).expect("barn exists");
        let shed_idx = engine.find_object_index(shed).expect("shed exists");
        assert!(engine.incinerate_object(barn_idx, 1, false, None)?);
        assert!(engine.incinerate_object(shed_idx, 1, false, None)?);
        let barn_con = engine.objects[barn_idx].state.construction;
        let shed_con = engine.objects[shed_idx].state.construction;
        engine.tick_without_snapshot()?;
        let barn_idx = engine.find_object_index(barn).expect("barn survives");
        let shed_idx = engine.find_object_index(shed).expect("shed survives");
        assert_eq!(
            engine.objects[barn_idx].state.construction,
            barn_con - 100,
            "inherited() chains to the engine FnFxFireTimer burn"
        );
        assert_eq!(
            engine.objects[shed_idx].state.construction,
            shed_con,
            "an overload that swallows the call replaces the engine burn"
        );
        assert!(
            engine.objects[shed_idx]
                .state
                .effects
                .iter()
                .any(|effect| effect.name == "Fire"),
            "FX_OK keeps the effect alive"
        );
        Ok(())
    }

    #[test]
    fn burning_object_emits_its_fire_particles_on_every_fourth_execution(
    ) -> Result<(), EngineError> {
        // FnFxFireTimer's emitter (C4Effect.cpp:660-769) runs after the burn
        // arms, gated on `iTime % 4` outside C4Fx_FireMode_Object. A burning
        // 16x16 structure therefore stays particle-free for three executions
        // and then spawns its double set — 2 `Fire`, 6 additive `Fire2`.
        let mut engine = Engine::with_seed(43);
        for name in ["Fire", "Fire2"] {
            engine
                .register_particle_definition(
                    particles::ParticleDefCore {
                        name: name.into(),
                        init_fn: "StdInit".into(),
                        exec_fn: "StdExec".into(),
                        draw_fn: "Std".into(),
                        delay: 1, // no fxStdInit life draw, and no decay
                        repeats: 1000,
                        ..Default::default()
                    },
                    10,
                    1.0,
                )
                .expect("fire def registers");
        }
        let mut barn = Definition::from_script("BARN", "Barn", "")?;
        barn.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        barn.set_fire_properties(0, true, true);
        engine.register_definition(barn)?;
        let object = engine.spawn_object(
            SpawnConfig::new("BARN")
                .with_category(CATEGORY_STRUCTURE)
                .with_position(Vector2::new(200, 300)),
        )?;
        let index = engine.find_object_index(object).expect("barn exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);

        for execution in 1..4 {
            engine.tick_without_snapshot()?;
            assert!(
                engine.particle_system().particles().is_empty(),
                "execution {execution} is inside the iTime % 4 gate",
            );
        }
        engine.tick_without_snapshot()?;

        let names: Vec<&str> = engine
            .particle_system()
            .particles()
            .iter()
            .map(|particle| particle.def_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Fire", "Fire", "Fire2", "Fire2", "Fire2", "Fire2", "Fire2", "Fire2"],
        );
        assert!(
            engine
                .particle_system()
                .particles()
                .iter()
                .all(|particle| matches!(
                    particle.layer,
                    ParticleLayer::ObjectBack(id) | ParticleLayer::ObjectFront(id) if id == object
                )),
            "every particle is dealt to the burning object's own lists",
        );
        Ok(())
    }

    #[test]
    fn burning_object_smokes_on_the_defs_smoke_rate_cadence() -> Result<(), EngineError> {
        // C4Object::ExecFire's "Effects" arm (C4Object.cpp:785-793):
        //   smoke_level = 2 * Shape.Wdt / 3
        //   smoke_rate  = 50 * smoke_level / Def->SmokeRate
        //   smoke when (FrameCounter + Number * 7) % max(smoke_rate, 3) == 0
        // A 16-wide object at the default SmokeRate=100 gives smoke_level 10
        // and a period of 5. Smoke() itself is the "Smoke" particle
        // (C4Effect.cpp:859-865), whose `a` is that level.
        let mut engine = Engine::with_seed(70);
        engine
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Smoke".into(),
                    init_fn: "StdInit".into(),
                    exec_fn: "StdExec".into(),
                    draw_fn: "Std".into(),
                    delay: 1,
                    repeats: 1000,
                    ..Default::default()
                },
                10,
                1.0,
            )
            .expect("smoke def registers");
        let mut hut = Definition::from_script("SMK1", "Smoky hut", "")?;
        hut.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        hut.set_fire_properties(0, true, true);
        engine.register_definition(hut)?;
        let object =
            engine.spawn_object(SpawnConfig::new("SMK1").with_category(CATEGORY_STRUCTURE))?;
        let index = engine.find_object_index(object).expect("hut exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);

        let mut smoking_frames = Vec::new();
        for _ in 0..15 {
            let before = engine.particle_system().particles().len();
            engine.tick_without_snapshot()?;
            if engine.particle_system().particles().len() > before {
                smoking_frames.push(engine.frame());
            }
        }

        assert!(
            !smoking_frames.is_empty(),
            "a burning object with a SmokeRate smokes",
        );
        let period = 5;
        let phase = smoking_frames[0] % period;
        for frame in &smoking_frames {
            assert_eq!(
                frame % period,
                phase,
                "smoke lands on one residue class of the SmokeRate period: {smoking_frames:?}",
            );
        }
        assert!(
            smoking_frames.len() >= 2,
            "the cadence repeats within 15 frames: {smoking_frames:?}",
        );
        let level = engine
            .particle_system()
            .particles()
            .iter()
            .find(|particle| particle.def_name == "Smoke")
            .map(|particle| particle.a)
            .expect("a Smoke particle exists");
        assert_eq!(level.to_bits(), 10.0f32.to_bits(), "2 * Shape.Wdt / 3");
        Ok(())
    }

    #[test]
    fn a_zero_smoke_rate_definition_never_smokes_while_burning() -> Result<(), EngineError> {
        // `if (smoke_rate)` (C4Object.cpp:788): SmokeRate=0 is the opt-out,
        // and it must not divide by zero on the way there.
        let mut engine = Engine::with_seed(71);
        engine
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Smoke".into(),
                    init_fn: "StdInit".into(),
                    exec_fn: "StdExec".into(),
                    draw_fn: "Std".into(),
                    delay: 1,
                    repeats: 1000,
                    ..Default::default()
                },
                10,
                1.0,
            )
            .expect("smoke def registers");
        let mut hut = Definition::from_script("SMK0", "Smokeless hut", "")?;
        hut.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        hut.set_fire_properties(0, true, true);
        hut.set_smoke_rate(0);
        engine.register_definition(hut)?;
        let object =
            engine.spawn_object(SpawnConfig::new("SMK0").with_category(CATEGORY_STRUCTURE))?;
        let index = engine.find_object_index(object).expect("hut exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);
        for _ in 0..15 {
            engine.tick_without_snapshot()?;
        }
        assert!(engine.particle_system().particles().is_empty());
        Ok(())
    }

    #[test]
    fn an_inherited_fx_fire_timer_smokes_like_the_native_path() -> Result<(), EngineError> {
        // The smoke arm lives in ExecFire, which both feeders port, so a
        // script FxFireTimer overload chaining inherited() must smoke on the
        // same cadence rather than losing it (C4Object.cpp:785-793).
        // Global, not definition-scope: the engine Fire effect carries no
        // command target, so only a global overload shadows the timer.
        let script = "#strict\n\
             global func FxFireTimer(pObj, iNumber, iTime)\n\
             {\n\
                 return inherited(pObj, iNumber, iTime);\n\
             }\n";
        let mut engine = Engine::with_seed(72);
        engine
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Smoke".into(),
                    init_fn: "StdInit".into(),
                    exec_fn: "StdExec".into(),
                    draw_fn: "Std".into(),
                    delay: 1,
                    repeats: 1000,
                    ..Default::default()
                },
                10,
                1.0,
            )
            .expect("smoke def registers");
        let mut hut = Definition::from_script("SMK2", "Overloaded hut", script)?;
        hut.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        hut.set_fire_properties(0, true, true);
        hut.set_c4_callback_convention(true);
        engine.register_definition(hut)?;
        let object =
            engine.spawn_object(SpawnConfig::new("SMK2").with_category(CATEGORY_STRUCTURE))?;
        let index = engine.find_object_index(object).expect("hut exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);

        // Sample per tick: a global-layer particle's survival is its own
        // (well covered) story, and what this test is about is how often the
        // compat feeder emits.
        let mut puffs = 0;
        let mut level = None;
        let mut layer_ok = true;
        for _ in 0..15 {
            let before = engine.particle_system().particles().len();
            engine.tick_without_snapshot()?;
            if let Some(particle) = engine
                .particle_system()
                .particles()
                .iter()
                .find(|particle| particle.def_name == "Smoke")
            {
                if engine.particle_system().particles().len() > before {
                    puffs += 1;
                    level = Some(particle.a);
                    layer_ok &= matches!(particle.layer, ParticleLayer::Global);
                }
            }
        }
        assert!(puffs > 0, "the inherited chain keeps the ExecFire smoke arm");
        assert_eq!(
            level.map(f32::to_bits),
            Some(10.0f32.to_bits()),
            "2 * Shape.Wdt / 3, same level as the native path",
        );
        assert!(
            layer_ok,
            "Smoke() passes no target, so it uses the global list",
        );

        // The two feeders are mutually exclusive per fire effect
        // (engine/tick.rs's `native_fire` branch), so the overload must
        // produce the SAME amount of smoke, not double it. Both objects are
        // 16 wide at the default SmokeRate, so both smoke on a period of 5
        // whatever phase their object number puts them on.
        let (mut native, _) = burning_smoker(74, 16, 100)?;
        let mut native_puffs = 0;
        for _ in 0..15 {
            let before = native.particle_system().particles().len();
            native.tick_without_snapshot()?;
            if native.particle_system().particles().len() > before {
                native_puffs += 1;
            }
        }
        assert_eq!(
            puffs, native_puffs,
            "the inherited chain smokes exactly once per execution",
        );

        // Non-vacuity: an overload that swallows the call instead of chaining
        // produces no smoke at all, so the assertions above are answering for
        // the compat feeder rather than the native one.
        let swallow = "#strict\n\
             global func FxFireTimer(pObj, iNumber, iTime) { return -1; }\n";
        let mut silent = Engine::with_seed(77);
        silent
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Smoke".into(),
                    init_fn: "StdInit".into(),
                    exec_fn: "StdExec".into(),
                    draw_fn: "Std".into(),
                    delay: 1,
                    repeats: 1000,
                    ..Default::default()
                },
                10,
                1.0,
            )
            .expect("smoke def registers");
        let mut swallowed = Definition::from_script("SMK4", "Swallowed", swallow)?;
        swallowed.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        swallowed.set_fire_properties(0, true, true);
        swallowed.set_c4_callback_convention(true);
        silent.register_definition(swallowed)?;
        let quiet =
            silent.spawn_object(SpawnConfig::new("SMK4").with_category(CATEGORY_STRUCTURE))?;
        let index = silent.find_object_index(quiet).expect("object exists");
        assert!(silent.incinerate_object(index, OWNER_NONE, false, None)?);
        for _ in 0..15 {
            silent.tick_without_snapshot()?;
        }
        assert!(
            silent.particle_system().particles().is_empty(),
            "a swallowing overload replaces the engine arm entirely",
        );
        Ok(())
    }

    #[test]
    fn switching_fire_particles_off_still_lets_script_create_them(
    ) -> Result<(), EngineError> {
        // C++ `FireParticles=false` only leaves pFire1/pFire2 null
        // (C4Particles.cpp:483-489), which stops `FnFxFireTimer`'s automatic
        // emitter. `CreateParticle("Fire2", ...)` looks the def up itself
        // (C4Script.cpp FnCreateParticle) and is unaffected — so the switch
        // must never become a blanket hide.
        let script = "#strict\n\
             func Flare()\n\
             {\n\
                 return CreateParticle(\"Fire2\", 0, 0, 0, -10, 20, 0);\n\
             }\n";
        let mut engine = engine_with_fire_particle_defs(73);
        engine.set_fire_particles(false);
        let mut torch = Definition::from_script("TRC2", "Torch", script)?;
        torch.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        torch.set_fire_properties(0, true, true);
        engine.register_definition(torch)?;
        let object =
            engine.spawn_object(SpawnConfig::new("TRC2").with_category(CATEGORY_STRUCTURE))?;
        let index = engine.find_object_index(object).expect("torch exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);
        for _ in 0..8 {
            engine.tick_without_snapshot()?;
        }
        assert!(
            engine.particle_system().particles().is_empty(),
            "the automatic emitter is silenced",
        );

        let index = engine.find_object_index(object).expect("torch survives");
        engine.call_object_function(index, "Flare", Vec::new())?;
        engine.tick_without_snapshot()?;
        assert!(
            engine
                .particle_system()
                .particles()
                .iter()
                .any(|particle| particle.def_name == "Fire2"),
            "script-created Fire2 is unaffected by the switch",
        );
        Ok(())
    }

    /// An engine with only the `Smoke` particle def registered, plus a
    /// burning `SMKT` structure of the given shape width and SmokeRate.
    fn burning_smoker(
        seed: u64,
        shape_width: i32,
        smoke_rate: i32,
    ) -> Result<(Engine, ObjectId), EngineError> {
        let mut engine = Engine::with_seed(seed);
        engine
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Smoke".into(),
                    init_fn: "StdInit".into(),
                    exec_fn: "StdExec".into(),
                    draw_fn: "Std".into(),
                    delay: 1,
                    repeats: 1000,
                    ..Default::default()
                },
                10,
                1.0,
            )
            .expect("smoke def registers");
        let mut definition = Definition::from_script("SMKT", "Smoker", "")?;
        definition.set_shape_rect(Some(DefinitionRect::new(
            -shape_width / 2,
            -8,
            shape_width,
            16,
        )));
        definition.set_fire_properties(0, true, true);
        definition.set_smoke_rate(smoke_rate);
        engine.register_definition(definition)?;
        let object =
            engine.spawn_object(SpawnConfig::new("SMKT").with_category(CATEGORY_STRUCTURE))?;
        let index = engine.find_object_index(object).expect("smoker exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);
        Ok((engine, object))
    }

    #[test]
    fn a_narrow_burning_object_floors_its_smoke_period_at_three_frames(
    ) -> Result<(), EngineError> {
        // `std::max<int32_t>(smoke_rate, 3)` (C4Object.cpp:791) is what keeps
        // the modulus away from zero. An 8-wide object computes level 5 and a
        // raw period of 2; a 1-wide one computes level 0 and a raw period of
        // 0, which would be a divide by zero without the floor.
        for (width, level) in [(8, 5), (1, 0)] {
            let (mut engine, _) = burning_smoker(80 + width as u64, width, 100)?;
            let mut smoking_frames = Vec::new();
            for _ in 0..12 {
                let before = engine.particle_system().particles().len();
                engine.tick_without_snapshot()?;
                if engine.particle_system().particles().len() > before {
                    smoking_frames.push(engine.frame());
                }
            }
            assert!(
                !smoking_frames.is_empty(),
                "width {width} still smokes on the floored period",
            );
            let phase = smoking_frames[0] % 3;
            for frame in &smoking_frames {
                assert_eq!(frame % 3, phase, "width {width}: {smoking_frames:?}");
            }
            assert_eq!(
                engine
                    .particle_system()
                    .particles()
                    .first()
                    .expect("a Smoke particle exists")
                    .a
                    .to_bits(),
                (level as f32).to_bits(),
                "width {width}: 2 * Shape.Wdt / 3",
            );
        }
        Ok(())
    }

    #[test]
    fn a_fast_burning_object_smokes_on_every_execution() -> Result<(), EngineError> {
        // `|| (Abs(xdir) > 2)` (C4Object.cpp:791) short-circuits the cadence
        // for a fast mover. The comparison is against `itofix(2)` on the raw
        // fixed value (Fixed.h:185), and it is strict — a port that rounded
        // through `fixtoi` would fire one step early and silently.
        let sample = |seed: u64, xdir: C4Fixed| -> Result<usize, EngineError> {
            let (mut engine, object) = burning_smoker(seed, 16, 100)?;
            let index = engine.find_object_index(object).expect("smoker exists");
            let mut smoking = 0;
            for _ in 0..6 {
                let index = engine.find_object_index(object).unwrap_or(index);
                engine.objects[index].set_fixed_velocity(FixedVec2::new(xdir, C4Fixed::ZERO));
                let before = engine.particle_system().particles().len();
                engine.tick_without_snapshot()?;
                if engine.particle_system().particles().len() > before {
                    smoking += 1;
                }
            }
            Ok(smoking)
        };

        // Exactly itofix(2) is NOT greater than itofix(2): cadence only.
        let at_threshold = sample(90, itofix(2))?;
        // One raw unit past it is.
        let past_threshold = sample(91, C4Fixed::from_raw(itofix(2).val() + 1))?;
        assert_eq!(
            past_threshold, 6,
            "a fast object smokes on every execution",
        );
        assert!(
            at_threshold < past_threshold,
            "the comparison is strict: {at_threshold} at the threshold vs \
             {past_threshold} past it",
        );
        Ok(())
    }

    #[test]
    fn the_smoke_cadence_wraps_like_cpp_int32_instead_of_trapping(
    ) -> Result<(), EngineError> {
        // `2 * Shape.Wdt` and `50 * smoke_level` are plain int32_t
        // (C4Object.cpp:786,790), and a negative SmokeRate divides straight
        // through. C++ wraps and keeps drawing; Rust must not trap on a path
        // a script-set shape or DefCore can reach.
        for smoke_rate in [i32::MIN, -1, 1, i32::MAX] {
            let (mut engine, _) = burning_smoker(95, i32::MAX, smoke_rate)?;
            for _ in 0..6 {
                engine.tick_without_snapshot()?;
            }
        }
        Ok(())
    }

    #[test]
    fn a_burning_object_smokes_without_the_fire_particle_defs_loaded(
    ) -> Result<(), EngineError> {
        // The smoke arm lives in ExecFire, which runs before
        // `IsFireParticleLoaded` is consulted (C4Effect.cpp:658 then :660-661),
        // so an installation with no Fire/Fire2 defs still smokes.
        let (mut engine, _) = burning_smoker(92, 16, 100)?;
        assert!(!engine.particle_system().is_fire_particle_loaded());
        for _ in 0..12 {
            engine.tick_without_snapshot()?;
        }
        assert!(
            engine
                .particle_system()
                .particles()
                .iter()
                .any(|particle| particle.def_name == "Smoke"),
            "smoke does not depend on the fire particle defs",
        );
        Ok(())
    }

    #[test]
    fn the_inherited_smoke_arm_honours_the_opt_out_and_the_fast_mover_branch(
    ) -> Result<(), EngineError> {
        // Both feeder paths carry their own copy of C4Object.cpp:788-791, so
        // the compat one needs its `if (smoke_rate)` opt-out and its
        // `|| Abs(xdir) > 2` escape pinned too — the native path's tests say
        // nothing about it.
        let script = "#strict\n\
             global func FxFireTimer(pObj, iNumber, iTime)\n\
             {\n\
                 return inherited(pObj, iNumber, iTime);\n\
             }\n";
        let build = |seed: u64, smoke_rate: i32| -> Result<(Engine, ObjectId), EngineError> {
            let mut engine = Engine::with_seed(seed);
            engine
                .register_particle_definition(
                    particles::ParticleDefCore {
                        name: "Smoke".into(),
                        init_fn: "StdInit".into(),
                        exec_fn: "StdExec".into(),
                        draw_fn: "Std".into(),
                        delay: 1,
                        repeats: 1000,
                        ..Default::default()
                    },
                    10,
                    1.0,
                )
                .expect("smoke def registers");
            let mut definition = Definition::from_script("SMK3", "Overloaded", script)?;
            definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
            definition.set_fire_properties(0, true, true);
            definition.set_smoke_rate(smoke_rate);
            definition.set_c4_callback_convention(true);
            engine.register_definition(definition)?;
            let object =
                engine.spawn_object(SpawnConfig::new("SMK3").with_category(CATEGORY_STRUCTURE))?;
            let index = engine.find_object_index(object).expect("object exists");
            assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);
            Ok((engine, object))
        };

        // SmokeRate=0 opts the overload out exactly as it does the native path.
        let (mut silent, _) = build(75, 0)?;
        for _ in 0..15 {
            silent.tick_without_snapshot()?;
        }
        assert!(silent.particle_system().particles().is_empty());

        // Past itofix(2) the cadence is bypassed and it smokes every tick.
        let (mut fast, object) = build(76, 100)?;
        let index = fast.find_object_index(object).expect("object exists");
        for _ in 0..5 {
            let index = fast.find_object_index(object).unwrap_or(index);
            fast.objects[index].set_fixed_velocity(FixedVec2::new(
                C4Fixed::from_raw(itofix(2).val() + 1),
                C4Fixed::ZERO,
            ));
            fast.tick_without_snapshot()?;
        }
        assert_eq!(
            fast.particle_system().particles().len(),
            5,
            "the overload's fast-mover escape fires on every execution",
        );
        Ok(())
    }

    /// A definition-less engine wired with the stock `Fire`/`Fire2` particle
    /// defs, so `IsFireParticleLoaded` (C4Particles.h:214) answers true.
    fn engine_with_fire_particle_defs(seed: u64) -> Engine {
        let mut engine = Engine::with_seed(seed);
        for name in ["Fire", "Fire2"] {
            engine
                .register_particle_definition(
                    particles::ParticleDefCore {
                        name: name.into(),
                        init_fn: "StdInit".into(),
                        exec_fn: "StdExec".into(),
                        draw_fn: "Std".into(),
                        delay: 1, // no fxStdInit life draw, and no decay
                        repeats: 1000,
                        ..Default::default()
                    },
                    10,
                    1.0,
                )
                .expect("fire def registers");
        }
        engine
    }

    #[test]
    fn object_mode_fire_emits_particles_on_every_execution() -> Result<(), EngineError> {
        // C4Effect.cpp:673-674 exempts C4Fx_FireMode_Object from the
        // `iTime % 4` gate "except for objects (e.g.: Projectiles)", so a
        // burning plain object trails fire on every single execution.
        let mut engine = engine_with_fire_particle_defs(51);
        let mut arrow = Definition::from_script("ARRW", "Arrow", "")?;
        arrow.set_shape_rect(Some(DefinitionRect::new(-4, -4, 8, 8)));
        arrow.set_fire_properties(0, true, true);
        engine.register_definition(arrow)?;
        let object = engine.spawn_object(
            SpawnConfig::new("ARRW")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(10, 10)),
        )?;
        let index = engine.find_object_index(object).expect("arrow exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);

        engine.tick_without_snapshot()?;
        // iCount = int(sqrt(64) / 4) = 2, so the double set is 4.
        assert_eq!(engine.particle_system().particles().len(), 4);
        engine.tick_without_snapshot()?;
        assert_eq!(
            engine.particle_system().particles().len(),
            8,
            "the second execution emits again without waiting for iTime % 4",
        );
        Ok(())
    }

    #[test]
    fn contained_burning_object_emits_no_fire_particles() -> Result<(), EngineError> {
        // "no gfx for contained" (C4Effect.cpp:676-677): the burn arms still
        // run inside a container, only the particles are suppressed.
        let mut engine = engine_with_fire_particle_defs(52);
        let mut torch = Definition::from_script("TRCH", "Torch", "")?;
        torch.set_shape_rect(Some(DefinitionRect::new(-4, -4, 8, 8)));
        torch.set_fire_properties(0, true, true);
        engine.register_definition(torch)?;
        let mut chest = Definition::from_script("CHST", "Chest", "")?;
        chest.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        engine.register_definition(chest)?;

        let container = engine.spawn_object(SpawnConfig::new("CHST"))?;
        let torch = engine.spawn_object(SpawnConfig::new("TRCH").with_category(CATEGORY_OBJECT))?;
        let torch_index = engine.find_object_index(torch).expect("torch exists");
        engine.objects[torch_index].state.container = Some(container);
        assert!(engine.incinerate_object(torch_index, OWNER_NONE, false, None)?);

        let fire_phase = engine.objects[torch_index].state.fire_phase;
        engine.tick_without_snapshot()?;
        let torch_index = engine.find_object_index(torch).expect("torch survives");
        assert!(
            engine.particle_system().particles().is_empty(),
            "a contained object draws no fire particles",
        );
        assert_ne!(
            engine.objects[torch_index].state.fire_phase, fire_phase,
            "ExecFire still ran; only the emitter returned early",
        );
        Ok(())
    }

    #[test]
    fn burning_object_emits_nothing_when_fire_particles_are_switched_off(
    ) -> Result<(), EngineError> {
        // "special effects only if loaded" (C4Effect.cpp:660-661):
        // SetDefParticles leaves pFire1/pFire2 null when
        // Config.Graphics.FireParticles is off (C4Particles.cpp:483-489), so
        // the emitter never runs — while the burn itself is unaffected.
        let mut engine = engine_with_fire_particle_defs(53);
        engine.set_fire_particles(false);
        let mut barn = Definition::from_script("BARN", "Barn", "")?;
        barn.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        barn.set_fire_properties(0, true, true);
        engine.register_definition(barn)?;
        let object =
            engine.spawn_object(SpawnConfig::new("BARN").with_category(CATEGORY_STRUCTURE))?;
        let index = engine.find_object_index(object).expect("barn exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);

        for _ in 0..8 {
            engine.tick_without_snapshot()?;
        }
        let index = engine.find_object_index(object).expect("barn survives");
        assert!(engine.particle_system().particles().is_empty());
        assert!(
            engine.objects[index].state.on_fire,
            "the object is still burning; only its particles are suppressed",
        );
        Ok(())
    }

    #[test]
    fn burning_object_reads_its_fire_mode_from_the_effects_first_variable(
    ) -> Result<(), EngineError> {
        // FxFireVarMode is EffectVars[0] (C4Effect.cpp:670-671). Rewriting it
        // to C4Fx_FireMode_Object lifts the `iTime % 4` gate for the next
        // execution, which is the cheapest observable proof the emitter reads
        // that variable rather than re-deriving the mode.
        let mut engine = engine_with_fire_particle_defs(54);
        let mut barn = Definition::from_script("BARN", "Barn", "")?;
        barn.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        barn.set_fire_properties(0, true, true);
        engine.register_definition(barn)?;
        let object =
            engine.spawn_object(SpawnConfig::new("BARN").with_category(CATEGORY_STRUCTURE))?;
        let index = engine.find_object_index(object).expect("barn exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);
        assert_eq!(
            engine.objects[index].state.effects[0].vars.first(),
            Some(&EffectVarValue::Int(C4FX_FIRE_MODE_STRUCT_VEH)),
            "a C4D_Structure defaults to struct/vehicle mode",
        );

        engine.objects[index].state.effects[0]
            .set_var(0, EffectVarValue::Int(C4FX_FIRE_MODE_OBJECT));
        engine.tick_without_snapshot()?;
        assert_eq!(
            engine.particle_system().particles().len(),
            8,
            "object mode emits on the first execution, inside iTime % 4",
        );
        Ok(())
    }

    #[test]
    fn inherited_fx_fire_timer_emits_the_same_fire_particles_as_the_native_path(
    ) -> Result<(), EngineError> {
        // A script FxFireTimer overload that chains inherited() lands in
        // compat::effects::fx_fire_timer instead of Engine::exec_object_fire
        // (registration.rs registers the host function). Both feed the one
        // emitter, so the overload must not cost the object its particles.
        //
        // The overload has to be GLOBAL: C4Object::Incinerate builds the Fire
        // effect with no command target (C4Object.cpp:1266), so
        // C4Effect::GetCallbackScript resolves the timer on the engine script
        // (C4Effect.cpp:42-57). A definition-scope FxFireTimer is never
        // dispatched for it, and a test written that way silently exercises
        // the native feeder instead.
        let script = "#strict\n\
             global func FxFireTimer(pObj, iNumber, iTime)\n\
             {\n\
                 return inherited(pObj, iNumber, iTime);\n\
             }\n";
        let mut engine = engine_with_fire_particle_defs(55);
        let mut hut = Definition::from_script("HUT1", "Hut", script)?;
        hut.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        hut.set_fire_properties(0, true, true);
        hut.set_c4_callback_convention(true);
        engine.register_definition(hut)?;
        let object = engine.spawn_object(
            SpawnConfig::new("HUT1")
                .with_category(CATEGORY_STRUCTURE)
                .with_position(Vector2::new(120, 90)),
        )?;
        let index = engine.find_object_index(object).expect("hut exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);

        for _ in 0..3 {
            engine.tick_without_snapshot()?;
            assert!(
                engine.particle_system().particles().is_empty(),
                "inside the iTime % 4 gate",
            );
        }
        engine.tick_without_snapshot()?;
        let names: Vec<&str> = engine
            .particle_system()
            .particles()
            .iter()
            .map(|particle| particle.def_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["Fire", "Fire", "Fire2", "Fire2", "Fire2", "Fire2", "Fire2", "Fire2"],
            "the inherited chain reaches the same double set",
        );
        Ok(())
    }

    #[test]
    fn the_fire_particle_gate_counts_effect_time_not_the_game_frame(
    ) -> Result<(), EngineError> {
        // `iTime % 4` (C4Effect.cpp:673-674) is the effect's own clock, which
        // C4Effect::Execute increments per effect (C4Effect.cpp:340-345) — not
        // the global frame the Tick5/Tick10 burn arms read. Igniting off-phase
        // separates the two: the effect's fourth execution lands on a game
        // frame that is not a multiple of four, and a frame-driven gate would
        // instead fire one execution early, on frame 4.
        let mut engine = engine_with_fire_particle_defs(57);
        let mut barn = Definition::from_script("BRN2", "Barn", "")?;
        barn.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        barn.set_fire_properties(0, true, true);
        engine.register_definition(barn)?;
        let object =
            engine.spawn_object(SpawnConfig::new("BRN2").with_category(CATEGORY_STRUCTURE))?;

        // Burn the first two frames so effect time trails the frame by two.
        for _ in 0..2 {
            engine.tick_without_snapshot()?;
        }
        let index = engine.find_object_index(object).expect("barn exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);

        for execution in 1..4 {
            engine.tick_without_snapshot()?;
            assert!(
                engine.particle_system().particles().is_empty(),
                "execution {execution} is inside the gate even though the game \
                 frame passed a multiple of four",
            );
        }
        engine.tick_without_snapshot()?;
        let index = engine.find_object_index(object).expect("barn survives");
        assert_eq!(
            engine.objects[index].state.effects[0].timer, 4,
            "the effect's own clock reached four on its fourth execution",
        );
        assert!(!engine.particle_system().particles().is_empty());
        Ok(())
    }

    #[test]
    fn removing_a_burning_object_releases_its_attached_fire_particles(
    ) -> Result<(), EngineError> {
        // C4Object::Clear drops both attached lists (C4Object.cpp:272-273).
        // Nothing iterates a removed object's particle layer, so without that
        // release the particles leak and their def's Count climbs until
        // MaxCount refuses every later one (C4Particles.cpp:389-391) — which
        // would silently switch fire off for the rest of the round.
        let mut engine = engine_with_fire_particle_defs(56);
        let mut hut = Definition::from_script("HUT2", "Hut", "")?;
        hut.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        hut.set_fire_properties(0, true, true);
        engine.register_definition(hut)?;
        let object =
            engine.spawn_object(SpawnConfig::new("HUT2").with_category(CATEGORY_STRUCTURE))?;
        let index = engine.find_object_index(object).expect("hut exists");
        assert!(engine.incinerate_object(index, OWNER_NONE, false, None)?);
        for _ in 0..4 {
            engine.tick_without_snapshot()?;
        }
        assert!(!engine.particle_system().particles().is_empty());
        let live_before: i32 = ["Fire", "Fire2"]
            .iter()
            .filter_map(|name| engine.particle_system().get_def(name))
            .map(|definition| definition.count)
            .sum();
        assert!(live_before > 0, "the defs are counting their live particles");

        // AssignRemoval; the tick's retain sweep is what runs C4Object::Clear.
        let index = engine.find_object_index(object).expect("hut survives");
        engine.objects[index].destroyed = true;
        engine.tick_without_snapshot()?;
        assert!(engine.find_object_index(object).is_none(), "the hut is gone");

        assert!(
            engine.particle_system().particles().is_empty(),
            "the removed object's attached particles are released",
        );
        for name in ["Fire", "Fire2"] {
            assert_eq!(
                engine
                    .particle_system()
                    .get_def(name)
                    .expect("def registered")
                    .count,
                0,
                "{name} gets its MaxCount budget back",
            );
        }
        Ok(())
    }

    #[test]
    fn fire_timer_normalizes_invalid_attribution_on_native_and_inherited_paths(
    ) -> Result<(), EngineError> {
        // FnFxFireTimer validates the stored fire-cause player for every
        // execution, passing NO_OWNER to both Tick10 DoDamage and Tick5
        // DoEnergy when that player no longer exists. Give each burning
        // object a different, valid controller so a NO_OWNER value encoded
        // through the script DoDamage/DoEnergy seam cannot silently fall
        // back to the caller's controller. Cover the native timer and a
        // script FxFireTimer overload that chains through inherited().
        let native_script = r#"#strict
local damage_cause;
func Damage(iChange, iCausedBy)
{
    damage_cause = iCausedBy;
    return 1;
}
"#;
        let inherited_script = r#"#strict
local damage_cause;
func Damage(iChange, iCausedBy)
{
    damage_cause = iCausedBy;
    return 1;
}
func FxFireTimer(pObj, iNumber, iTime)
{
    return inherited(pObj, iNumber, iTime);
}
"#;

        let mut native_definition =
            Definition::from_script("NTMR", "Native timer", native_script)?;
        native_definition.set_fire_properties(0, true, false);
        native_definition.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        let mut inherited_definition =
            Definition::from_script("ITMR", "Inherited timer", inherited_script)?;
        inherited_definition.set_fire_properties(0, true, false);
        inherited_definition.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(44);
        engine.register_player(PlayerConfig::new(5, "Object controller"))?;
        engine.register_player(PlayerConfig::new(7, "Valid fire cause"))?;
        engine.register_definition(native_definition)?;
        engine.register_definition(inherited_definition)?;

        let invalid_native = engine.spawn_object(
            SpawnConfig::new("NTMR")
                .with_category(CATEGORY_OBJECT)
                .with_controller(5),
        )?;
        let invalid_inherited = engine.spawn_object(
            SpawnConfig::new("ITMR")
                .with_category(CATEGORY_OBJECT)
                .with_controller(5),
        )?;
        let valid_native = engine.spawn_object(
            SpawnConfig::new("NTMR")
                .with_category(CATEGORY_OBJECT)
                .with_controller(5),
        )?;
        let valid_inherited = engine.spawn_object(
            SpawnConfig::new("ITMR")
                .with_category(CATEGORY_OBJECT)
                .with_controller(5),
        )?;

        for id in [invalid_native, invalid_inherited] {
            let idx = engine.find_object_index(id).expect("invalid-cause object exists");
            assert!(engine.incinerate_object(idx, 99, false, None)?);
        }
        for id in [valid_native, valid_inherited] {
            let idx = engine.find_object_index(id).expect("valid-cause object exists");
            assert!(engine.incinerate_object(idx, 7, false, None)?);
        }

        while engine.frame < 10 {
            engine.tick_without_snapshot()?;
        }

        for (id, path) in [
            (invalid_native, "native"),
            (invalid_inherited, "inherited"),
        ] {
            let idx = engine.find_object_index(id).expect("invalid-cause object remains");
            assert_eq!(
                engine.objects[idx].state.local_vars.get("damage_cause"),
                Some(&Value::Int(OWNER_NONE)),
                "{path} Tick10 damage receives NO_OWNER"
            );
            assert_eq!(
                engine.objects[idx].last_energy_loss_cause,
                OWNER_NONE,
                "{path} Tick5 energy attribution receives NO_OWNER"
            );
            let fire = engine.objects[idx]
                .state
                .effects
                .iter()
                .find(|effect| effect.name == "Fire")
                .expect("fire effect remains");
            assert_eq!(
                fire.vars()[1],
                EffectVarValue::Int(99),
                "validation is per timer call and does not rewrite the stored cause"
            );
        }

        for (id, path) in [(valid_native, "native"), (valid_inherited, "inherited")] {
            let idx = engine.find_object_index(id).expect("valid-cause object remains");
            assert_eq!(
                engine.objects[idx].state.local_vars.get("damage_cause"),
                Some(&Value::Int(7)),
                "{path} preserves a valid fire cause for damage"
            );
            assert_eq!(
                engine.objects[idx].last_energy_loss_cause, 7,
                "{path} preserves a valid fire cause for energy"
            );
        }
        Ok(())
    }

    #[test]
    fn add_effect_fire_runs_the_engine_fire_start() -> Result<(), EngineError> {
        // AddEffect("Fire", pObj, 100, 1, ...) resolves the engine
        // FnFxFireStart (C4Effect ctor pFnStart, C4Effect.cpp:118-133 +
        // AddFunc C4Script.cpp:6994): the target ignites — OnFire, the
        // FirePhase draw, effect vars [mode, causedBy, blasted, incObj]
        // (C4Effect.cpp:609-634) — and AddEffect returns the number. A
        // denied start (extinguishing material) marks the effect dead but
        // still returns its allocated number (C4Effect.cpp:128-136).
        let mut engine = Engine::with_seed(47);
        engine.register_definition(
            Definition::from_script(
                "ACTR",
                "Actor",
                "#strict\nfunc Torch(pVictim) { return AddEffect(\"Fire\", pVictim, 100, 1, 0, 0, 7, false); }\n",
            )
            .expect("actor compiles"),
        )?;
        engine.register_definition(simple_definition("Hut"))?;
        let actor =
            engine.spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))?;
        let hut = engine.spawn_object(SpawnConfig::new("Hut"))?;
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        let mut mirror = engine.rng.clone();
        let expected_phase = mirror.random(15);
        let result =
            engine.call_object_function(actor_idx, "Torch", vec![Value::Object(hut.as_u64())])?;
        let hut_idx = engine.find_object_index(hut).expect("hut exists");
        assert!(engine.objects[hut_idx].state.on_fire, "ignited");
        assert_eq!(engine.objects[hut_idx].state.fire_phase, expected_phase);
        assert_eq!(engine.objects[hut_idx].state.fire_caused_by, 7);
        assert_eq!(engine.rng, mirror, "one FirePhase draw");
        let fire = engine.objects[hut_idx]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Fire")
            .cloned()
            .expect("fire effect entry");
        assert_eq!(result, Value::Int(fire.number), "AddEffect hands back the number");
        assert_eq!(
            fire.vars(),
            &[
                EffectVarValue::Int(2),
                EffectVarValue::Int(7),
                EffectVarValue::Bool(false),
                EffectVarValue::Nil,
            ]
        );
        Ok(())
    }

    #[test]
    fn checked_fire_add_ignites_inside_add_effect_like_cpp() -> Result<(), EngineError> {
        // AddEffect("Fire") with same/higher-priority effects present runs
        // the Fx*Effect check chain first (C4Effect ctor,
        // C4Effect.cpp:97-116). A passing check then invokes the engine
        // FnFxFireStart before the constructor and AddEffect return
        // (C4Effect.cpp:118-136).
        let mut engine = Engine::with_seed(53);
        engine.register_definition(
            Definition::from_script(
                "BARN",
                "Barn",
                "#strict\nfunc FxShieldEffect(szNew, pObj, iNumber) { return 0; }\nfunc Kindle() { AddEffect(\"Shield\", this(), 200, 0); return AddEffect(\"Fire\", this(), 100, 1, 0, 0, 9); }\n",
            )
            .expect("barn compiles"),
        )?;
        let barn = engine.spawn_object(SpawnConfig::new("BARN"))?;
        let barn_idx = engine.find_object_index(barn).expect("barn exists");
        let _ = engine.call_object_function(barn_idx, "Kindle", Vec::new())?;
        let barn_idx = engine.find_object_index(barn).expect("barn exists");
        assert!(
            engine.objects[barn_idx].state.on_fire,
            "the checked add runs the engine start before AddEffect returns"
        );
        let con_before = engine.objects[barn_idx].state.construction;
        engine.tick_without_snapshot()?;
        let barn_idx = engine.find_object_index(barn).expect("barn survives");
        assert!(engine.objects[barn_idx].state.on_fire, "ignited");
        assert_eq!(engine.objects[barn_idx].state.fire_caused_by, 9);
        assert_eq!(
            engine.objects[barn_idx].state.construction,
            con_before - 100,
            "the first execution starts AND burns"
        );
        let fire = engine.objects[barn_idx]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Fire")
            .expect("fire effect survives");
        assert_eq!(fire.vars()[0], EffectVarValue::Int(2), "mode written");
        assert_eq!(fire.vars()[1], EffectVarValue::Int(9), "cause remapped");
        Ok(())
    }

    #[test]
    fn incinerate_honors_higher_priority_effect_deny() -> Result<(), EngineError> {
        let script = r#"#strict 2
func InstallShield() { return AddEffect("Shield", this(), 200, 0, this()); }
func Ignite() { return Incinerate(); }
func FxShieldEffect(szNew, pTarget, iNumber, iUnused, iCause, fBlasted, pIncinerating, iUnused2)
{
    if (szNew == "Fire") return -1;
    return 0;
}
func Incineration(iCause) { return 1; }
"#;
        let call_log: Arc<Mutex<Vec<(String, Vec<Value>)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                call_log
                    .lock()
                    .unwrap()
                    .push((name.to_string(), args.to_vec()));
            });
        }
        let mut definition =
            Definition::from_script("FIRE_DENY", "Fire deny", script).expect("script compiles");
        definition.set_c4_callback_convention(true);
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(73);
        engine.register_definition(definition)?;
        let direct = engine.spawn_object(
            SpawnConfig::new("FIRE_DENY")
                .with_category(CATEGORY_OBJECT)
                .with_controller(9),
        )?;
        let scripted = engine.spawn_object(
            SpawnConfig::new("FIRE_DENY")
                .with_category(CATEGORY_OBJECT)
                .with_controller(9),
        )?;

        let mut shield_numbers = HashMap::new();
        for id in [direct, scripted] {
            let idx = engine.find_object_index(id).expect("target exists");
            let result = engine.call_object_function(idx, "InstallShield", Vec::new())?;
            let Value::Int(number) = result else {
                panic!("Shield AddEffect returned {result:?}");
            };
            assert!(number > 0);
            shield_numbers.insert(id, number);
        }
        call_log.lock().unwrap().clear();
        let rng_before = engine.rng.clone();
        let fire_before = [direct, scripted].map(|id| {
            let idx = engine.find_object_index(id).expect("target exists");
            (
                engine.objects[idx].state.fire_phase,
                engine.objects[idx].state.fire_caused_by,
            )
        });

        let direct_idx = engine.find_object_index(direct).expect("direct target exists");
        assert!(!engine.incinerate_object(direct_idx, 9, false, None)?);
        let scripted_idx = engine
            .find_object_index(scripted)
            .expect("scripted target exists");
        assert_eq!(
            engine.call_object_function(scripted_idx, "Ignite", Vec::new())?,
            Value::Bool(false)
        );

        assert_eq!(engine.rng, rng_before, "denied fire consumes no RNG");
        for (slot, id) in [direct, scripted].into_iter().enumerate() {
            let idx = engine.find_object_index(id).expect("target survives");
            let object = &engine.objects[idx];
            assert!(!object.state.on_fire);
            assert_eq!(
                (object.state.fire_phase, object.state.fire_caused_by),
                fire_before[slot]
            );
            let shield = object
                .state
                .effects
                .iter()
                .find(|effect| effect.name == "Shield" && effect.priority != 0)
                .expect("the denying Shield remains active");
            assert_eq!(shield.priority, 200);
            assert_eq!(
                shield.number,
                shield_numbers[&id],
                "the denying effect is unchanged"
            );
            assert!(object
                .state
                .effects
                .iter()
                .any(|effect| effect.name == "Fire" && effect.priority == 0));
        }

        engine.tick_without_snapshot()?;
        for id in [direct, scripted] {
            let idx = engine.find_object_index(id).expect("target survives");
            assert_eq!(
                engine.objects[idx]
                    .state
                    .effects
                    .iter()
                    .map(|effect| effect.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["Shield"]
            );
        }

        let calls: Vec<_> = call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name.starts_with("Fx") || name == "Incineration")
            .cloned()
            .collect();
        assert_eq!(calls.len(), 2);
        for ((name, args), id) in calls.iter().zip([direct, scripted]) {
            assert_eq!(name, "FxShieldEffect");
            assert_eq!(
                args,
                &vec![
                    Value::String("Fire".to_string().into()),
                    Value::Object(id.as_u64()),
                    Value::Int(shield_numbers[&id]),
                    Value::Nil,
                    Value::Int(9),
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                ]
            );
        }
        Ok(())
    }

    #[test]
    fn incinerate_global_fx_fire_start_matches_add_effect() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(79);
        let script = r#"#strict 2
global func FxFireStart(pTarget, iNumber, iTemp, iCause, fBlasted, pIncinerating, iUnused) { return 0; }
func ViaIncinerate() { return Incinerate(); }
func ViaAddEffect(pIncinerating) { var no_value; return AddEffect("Fire", this(), 100, 1, no_value, no_value, 7, true, pIncinerating, no_value); }
func FireMode() { return 3; }
func Incineration(iCause) { return 1; }
"#;
        let call_log: Arc<Mutex<Vec<(String, Vec<Value>)>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, args| {
                call_log
                    .lock()
                    .unwrap()
                    .push((name.to_string(), args.to_vec()));
            });
        }
        let mut definition = Definition::from_script("FIRE_START", "Fire start", script)?;
        definition.set_c4_callback_convention(true);
        definition.set_debugger_hooks(hooks);
        engine.register_definition(definition)?;

        let ids: [ObjectId; 3] = std::array::from_fn(|_| {
            engine
                .spawn_object(
                    SpawnConfig::new("FIRE_START")
                        .with_category(CATEGORY_OBJECT)
                        .with_controller(7),
                )
                .expect("target spawns")
        });
        let rng_before = engine.rng.clone();
        let fire_before = ids.map(|id| {
            let idx = engine.find_object_index(id).expect("target exists");
            (
                engine.objects[idx].state.fire_phase,
                engine.objects[idx].state.fire_caused_by,
            )
        });

        let source = ids[1];
        let direct_idx = engine.find_object_index(ids[0]).expect("direct target exists");
        assert!(engine.incinerate_object(direct_idx, 7, true, Some(source))?);
        let script_idx = engine.find_object_index(ids[1]).expect("script target exists");
        assert_eq!(
            engine.call_object_function(script_idx, "ViaIncinerate", Vec::new())?,
            Value::Bool(true)
        );
        let add_idx = engine.find_object_index(ids[2]).expect("add target exists");
        assert_eq!(
            engine.call_object_function(
                add_idx,
                "ViaAddEffect",
                vec![Value::Object(source.as_u64())],
            )?,
            Value::Int(1)
        );

        assert_eq!(engine.rng, rng_before, "the script override consumes no RNG");
        for (slot, id) in ids.into_iter().enumerate() {
            let idx = engine.find_object_index(id).expect("target survives");
            let object = &engine.objects[idx];
            assert!(!object.state.on_fire, "the native start was replaced");
            assert_eq!(
                (object.state.fire_phase, object.state.fire_caused_by),
                fire_before[slot]
            );
            assert_eq!(object.state.effects.len(), 1);
            let fire = &object.state.effects[0];
            assert_eq!(fire.name, "Fire");
            assert_eq!(fire.number, 1);
            assert_eq!(fire.priority, 100);
            assert_eq!(fire.interval, 1);
        }

        let calls: Vec<_> = call_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| {
                matches!(name.as_str(), "FxFireStart" | "FireMode" | "Incineration")
            })
            .cloned()
            .collect();
        assert_eq!(calls.len(), 3);
        let expected_args = [
            vec![
                Value::Object(ids[0].as_u64()),
                Value::Int(1),
                Value::Nil,
                Value::Int(7),
                Value::Bool(true),
                Value::Object(source.as_u64()),
                Value::Nil,
            ],
            vec![
                Value::Object(ids[1].as_u64()),
                Value::Int(1),
                Value::Nil,
                Value::Int(7),
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ],
            vec![
                Value::Object(ids[2].as_u64()),
                Value::Int(1),
                Value::Nil,
                Value::Int(7),
                Value::Bool(true),
                Value::Object(source.as_u64()),
                Value::Nil,
            ],
        ];
        for ((name, args), expected) in calls.iter().zip(expected_args) {
            assert_eq!(name, "FxFireStart");
            assert_eq!(args, &expected);
        }
        Ok(())
    }

    #[test]
    fn foreign_incinerate_bypasses_add_effect_script_shadows() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(83);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/AddEffect.c".to_string(),
                "global func AddEffect() { return 0; }\n".to_string(),
            )]),
            1
        );

        let mut igniter = Definition::from_script(
            "IGNITER",
            "Igniter",
            "#strict 2\nfunc Ignite(pTarget) { return Incinerate(pTarget); }\n",
        )?;
        igniter.set_c4_callback_convention(true);
        let mut local_shadow = Definition::from_script(
            "LOCAL_SHADOW",
            "Local shadow",
            "#strict 2\nfunc AddEffect() { return 0; }\n",
        )?;
        local_shadow.set_c4_callback_convention(true);
        engine.register_definition(igniter)?;
        engine.register_definition(local_shadow)?;
        engine.register_definition(simple_definition("PLAIN_TARGET"))?;

        let actor = engine.spawn_object(
            SpawnConfig::new("IGNITER")
                .with_category(CATEGORY_OBJECT)
                .with_controller(7),
        )?;
        let local_target = engine.spawn_object(
            SpawnConfig::new("LOCAL_SHADOW").with_category(CATEGORY_OBJECT),
        )?;
        let global_target = engine.spawn_object(
            SpawnConfig::new("PLAIN_TARGET").with_category(CATEGORY_OBJECT),
        )?;
        let actor_idx = engine.find_object_index(actor).expect("actor exists");

        for target in [local_target, global_target] {
            assert_eq!(
                engine.call_object_function(
                    actor_idx,
                    "Ignite",
                    vec![Value::Object(target.as_u64())],
                )?,
                Value::Bool(true)
            );
            let target_idx = engine.find_object_index(target).expect("target survives");
            let object = &engine.objects[target_idx];
            assert!(object.state.on_fire);
            assert_eq!(object.state.fire_caused_by, 7);
            assert_eq!(object.state.effects.len(), 1);
            assert_eq!(object.state.effects[0].name, "Fire");
            assert_eq!(object.state.effects[0].priority, 100);
            assert_eq!(object.state.effects[0].interval, 1);
        }
        Ok(())
    }

    #[test]
    fn incinerate_effect_check_errors_are_fail_safe() -> Result<(), EngineError> {
        let script = r#"#strict 2
func InstallBroken() { return AddEffect("Broken", this(), 200, 0, this()); }
func FxBrokenEffect(szNew, pTarget, iNumber)
{
    MissingEffectFunction();
    return -1;
}
func Incineration(iCause) { return 1; }
"#;
        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                if matches!(name, "FxBrokenEffect" | "Incineration") {
                    call_log.lock().unwrap().push(name.to_string());
                }
            });
        }
        let mut definition =
            Definition::from_script("BROKEN_CHECK", "Broken check", script)?;
        definition.set_c4_callback_convention(true);
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(89);
        engine.register_definition(definition)?;
        let target = engine.spawn_object(
            SpawnConfig::new("BROKEN_CHECK")
                .with_category(CATEGORY_OBJECT)
                .with_controller(7),
        )?;
        let idx = engine.find_object_index(target).expect("target exists");
        assert!(matches!(
            engine.call_object_function(idx, "InstallBroken", Vec::new())?,
            Value::Int(number) if number > 0
        ));
        call_log.lock().unwrap().clear();

        let mut mirror = engine.rng.clone();
        let expected_phase = mirror.random(15);
        assert!(engine.incinerate_object(idx, 7, false, None)?);
        assert_eq!(engine.rng, mirror);
        let object = &engine.objects[idx];
        assert!(object.state.on_fire);
        assert_eq!(object.state.fire_phase, expected_phase);
        assert_eq!(object.state.fire_caused_by, 7);
        assert_eq!(
            object
                .state
                .effects
                .iter()
                .map(|effect| effect.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Fire", "Broken"]
        );
        assert_eq!(
            call_log.lock().unwrap().as_slice(),
            ["FxBrokenEffect", "Incineration"]
        );
        Ok(())
    }

    #[test]
    fn bubble_script_function_creates_fxu1_in_liquid_like_cpp() -> Result<(), EngineError> {
        // FnBubble (C4Script.cpp:2188-2192 + AddFunc :6718): the
        // caller-relative point goes to BubbleOut (C4Effect.cpp:847-857) —
        // a bubble only from semi-solid (submerged) spots, creating one
        // FXU1 object.
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let mut engine = Engine::with_seed(59);
        engine.set_materials(materials);
        let mut landscape = Landscape::flat_with_material(40, 30, None);
        landscape.set_liquid_column(10, vec![LiquidSegment::with_material(5, 12, Some(water))]);
        engine.set_landscape(landscape);
        engine.register_definition(simple_definition("FXU1"))?;
        engine.register_definition(
            Definition::from_script(
                "ACTR",
                "Actor",
                "#strict\nfunc Blub(iX, iY) { Bubble(iX, iY); }\n",
            )
            .expect("actor compiles"),
        )?;
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_position(Vector2::new(0, 0)))?;
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        // submerged spot → one FXU1
        let _ = engine.call_object_function(
            actor_idx,
            "Blub",
            vec![Value::Int(10), Value::Int(8)],
        )?;
        let bubbles: Vec<&Object> = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "FXU1")
            .collect();
        assert_eq!(bubbles.len(), 1, "one bubble from the submerged spot");
        assert_eq!(bubbles[0].state.position, Vector2::new(10, 8));
        // open air → no bubbles from nowhere (C4Effect.cpp:850)
        let _ = engine.call_object_function(
            actor_idx,
            "Blub",
            vec![Value::Int(30), Value::Int(2)],
        )?;
        let count = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "FXU1")
            .count();
        assert_eq!(count, 1, "no bubble in open air");
        Ok(())
    }

    #[test]
    fn bubble_cap_uses_sync_mode_or_configured_smoke_level() -> Result<(), EngineError> {
        fn fixture(
            smoke_level: Option<i32>,
            network_game: bool,
            recording_active: bool,
            initial_bubbles: usize,
        ) -> Result<(Engine, ObjectId), EngineError> {
            let library = MaterialLibrary::parse(
                r#"
                [Material Water]
                Name=Water
                Density=25
                Friction=0
            "#,
            )
            .expect("material library parses");
            let materials = MaterialSet::from_resource_library(&library);
            let water = materials.id_of("Water").expect("water exists");
            let mut engine = Engine::with_seed(212);
            if let Some(smoke_level) = smoke_level {
                engine.set_smoke_level(smoke_level);
            }
            engine.set_network_game(network_game);
            engine.set_recording_active(recording_active);
            engine.set_materials(materials);
            let mut landscape = Landscape::flat_with_material(40, 30, None);
            landscape
                .set_liquid_column(10, vec![LiquidSegment::with_material(5, 12, Some(water))]);
            engine.set_landscape(landscape);
            engine.register_definition(simple_definition("FXU1"))?;
            engine.register_definition(Definition::from_script(
                "ACTR",
                "Actor",
                "#strict\nfunc Blub() { Bubble(10, 8); }\n",
            )?)?;
            let actor = engine.spawn_object(SpawnConfig::new("ACTR"))?;
            for _ in 0..initial_bubbles {
                engine.spawn_object(SpawnConfig::new("FXU1"))?;
            }
            Ok((engine, actor))
        }

        fn call_and_count(engine: &mut Engine, actor: ObjectId) -> Result<usize, EngineError> {
            let actor_idx = engine.find_object_index(actor).expect("actor exists");
            let _ = engine.call_object_function(actor_idx, "Blub", Vec::new())?;
            Ok(engine
                .objects
                .iter()
                .filter(|object| object.definition_id == "FXU1")
                .count())
        }

        // Local non-record play uses Config.Graphics.SmokeLevel, whose
        // default is 200 rather than the synchronized fixed limit 150.
        let (mut local_default, actor) = fixture(None, false, false, 199)?;
        assert_eq!(call_and_count(&mut local_default, actor)?, 200);
        assert_eq!(call_and_count(&mut local_default, actor)?, 200);

        // A custom local setting is consumed directly, including values
        // below the sync limit.
        let (mut local_custom, actor) = fixture(Some(3), false, false, 2)?;
        assert_eq!(call_and_count(&mut local_custom, actor)?, 3);
        assert_eq!(call_and_count(&mut local_custom, actor)?, 3);

        // Network and active-recording sync modes both force 150 regardless
        // of the process-local graphics setting.
        let (mut network, actor) = fixture(Some(3), true, false, 149)?;
        assert_eq!(call_and_count(&mut network, actor)?, 150);
        assert_eq!(call_and_count(&mut network, actor)?, 150);

        let (mut recording, actor) = fixture(Some(3), false, true, 149)?;
        assert_eq!(call_and_count(&mut recording, actor)?, 150);
        assert_eq!(call_and_count(&mut recording, actor)?, 150);
        Ok(())
    }

    #[test]
    fn incinerate_in_extinguisher_leaves_fire_dead_until_execute() -> Result<(), EngineError> {
        // fxFireStart deny (C4Effect.cpp:574-607 + ctor :128-133): in
        // extinguishing material the Start returns -1 and the freshly
        // created effect dies without a Stop call. The constructor still
        // reports its allocated number, so Incinerate succeeds.
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
            Extinguisher=-1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let mut engine = Engine::with_seed(24);
        engine.set_materials(materials);
        let mut landscape = Landscape::flat_with_material(40, 30, None);
        landscape.set_liquid_column(10, vec![LiquidSegment::with_material(5, 12, Some(water))]);
        engine.set_landscape(landscape);
        engine.register_definition(simple_definition("Tree"))?;
        let tree =
            engine.spawn_object(SpawnConfig::new("Tree").with_position(Vector2::new(10, 8)))?;
        let idx = engine.find_object_index(tree).expect("tree exists");
        assert!(engine.incinerate_object(idx, 1, false, None)?);
        assert!(!engine.objects[idx].state.on_fire);
        assert!(
            engine.objects[idx]
                .state
                .effects
                .iter()
                .any(|effect| effect.name == "Fire" && effect.priority == 0),
            "the Start-denied node stays linked dead"
        );
        engine.tick_without_snapshot()?;
        let idx = engine.find_object_index(tree).expect("tree exists");
        assert!(engine.objects[idx].state.effects.is_empty());
        Ok(())
    }

    #[test]
    fn exec_fire_burns_objects_like_cpp() -> Result<(), EngineError> {
        // C4Object::ExecFire (C4Object.cpp:766-810): every frame FirePhase
        // cycles mod 15 and Con decays by 100 raw units (unless NoBurnDecay);
        // Tick10 deals +2 damage (unless NoBurnDamage); Tick5 drains 1
        // energy; Tick5 over valid landscape material extinguishes in
        // extinguisher material and otherwise draws Random(3) for landscape
        // inflammation.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(71);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(40, 30, Some(earth)));
        let mut hut_definition = simple_definition("Hut");
        hut_definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine.register_definition(hut_definition)?;
        // in open air: the Tick5 background-material block never fires
        let hut = engine.spawn_object(
            SpawnConfig::new("Hut")
                .with_position(Vector2::new(10, 10))
                .with_energy(50_000),
        )?;
        let idx = engine.find_object_index(hut).expect("hut exists");
        assert!(engine.incinerate_object(idx, 1, false, None)?);
        let phase_after_start = engine.objects[idx].state.fire_phase;
        let con_before = engine.objects[idx].state.construction;
        let mirror = engine.rng.clone();

        let fire_number = engine.objects[idx].state.effects[0].number;
        // frame 1: neither Tick5 nor Tick10 — only phase + decay
        engine.exec_object_fire(idx, 1, fire_number);
        assert_eq!(
            engine.objects[idx].state.fire_phase,
            (phase_after_start + 1) % 15
        );
        assert_eq!(engine.objects[idx].state.construction, con_before - 100);
        assert_eq!(engine.objects[idx].state.energy, 50_000);
        assert_eq!(engine.objects[idx].state.damage, 0);
        assert_eq!(engine.rng, mirror, "no draws in open air off-tick");

        // frame 5: Tick5 → energy -1 (air: no background draw)
        engine.exec_object_fire(idx, 5, fire_number);
        assert_eq!(engine.objects[idx].state.energy, 49_000);
        // frame 10: Tick10 + Tick5 → damage +2, energy -1
        engine.exec_object_fire(idx, 10, fire_number);
        assert_eq!(engine.objects[idx].state.damage, 2);
        assert_eq!(engine.objects[idx].state.energy, 48_000);
        assert_eq!(engine.rng, mirror, "still no draws in open air");

        // Buried in earth (below the flat surface at y = 30): Tick5 draws
        // Random(3) for landscape inflammation (C4Object.cpp:797-805).
        let buried = engine.spawn_object(
            SpawnConfig::new("Hut")
                .with_position(Vector2::new(20, 35))
                .with_energy(50),
        )?;
        let buried_idx = engine.find_object_index(buried).expect("buried exists");
        assert!(engine.incinerate_object(buried_idx, 1, false, None)?);
        let buried_fire = engine.objects[buried_idx].state.effects[0].number;
        let mut mirror = engine.rng.clone();
        engine.exec_object_fire(buried_idx, 15, buried_fire);
        mirror.random(3);
        assert_eq!(engine.rng, mirror, "Tick5 inflame draw over material");
        assert!(
            engine.objects[buried_idx].state.on_fire,
            "earth does not extinguish"
        );
        Ok(())
    }

    #[test]
    fn cross_check_contact_incineration_on_tick35() -> Result<(), EngineError> {
        // CrossCheck pass 1, incineration arm (C4GameObjects.cpp:106-125):
        // on Tick35 frames an OCF_OnFire object standing at an
        // OCF_Inflammable object's shape incinerates it when
        // !Random(ContactIncinerate), attributing the original fire cause.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(72);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(80, 60, Some(earth)));
        // GetFireCausePlr only forwards VALID players (C4Object.cpp:6193-6203)
        engine.register_player(PlayerConfig::new(7, "P7"))?;
        let mut torch_def = simple_definition("Torch");
        torch_def.set_fire_properties(1, false, false);
        engine.register_definition(torch_def)?;
        let mut tree_def = simple_definition("Tree");
        tree_def.set_fire_properties(1, false, false); // Random(1) == 0 always
        tree_def.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 16)));
        engine.register_definition(tree_def)?;
        engine.register_definition(simple_definition("FireLayer"))?;
        let source_layer = engine.spawn_object(SpawnConfig::new("FireLayer"))?;
        let other_layer = engine.spawn_object(SpawnConfig::new("FireLayer"))?;

        let torch = engine.spawn_object(
            SpawnConfig::new("Torch")
                .with_position(Vector2::new(40, 20))
                .with_layer(source_layer),
        )?;
        // Spawn y is the con-0 bottom (C4Object.cpp:1462-1468): 28 - (16 - 8)
        // keeps the tree center at (41,20), on top of the shapeless torch.
        let tree = engine.spawn_object(
            SpawnConfig::new("Tree")
                .with_position(Vector2::new(41, 28))
                .with_layer(other_layer),
        )?;
        let torch_idx = engine.find_object_index(torch).expect("torch exists");
        assert!(engine.incinerate_object(torch_idx, 7, false, None)?);

        // Not a Tick35 frame: nothing happens, no draws.
        let mirror = engine.rng.clone();
        engine.cross_check(34)?;
        let tree_idx = engine.find_object_index(tree).expect("tree exists");
        assert!(!engine.objects[tree_idx].state.on_fire);
        assert_eq!(engine.rng, mirror);

        // A different pLayer is rejected by AtObject before the contact
        // chance, so even a Tick35 pass consumes no draw.
        let mirror = engine.rng.clone();
        engine.cross_check(35)?;
        let tree_idx = engine.find_object_index(tree).expect("tree exists");
        assert!(!engine.objects[tree_idx].state.on_fire);
        assert_eq!(engine.rng, mirror, "cross-layer contact draws nothing");

        engine.apply_object_update(tree, ObjectUpdate::new().with_layer(source_layer))?;
        // Same layer on Tick35: Random(ContactIncinerate=1) == 0 →
        // incinerate, which draws the new fire's FirePhase. The fire cause
        // carries over (GetFireCausePlr).
        let mut mirror = engine.rng.clone();
        mirror.random(1);
        mirror.random(15);
        engine.cross_check(35)?;
        let tree_idx = engine.find_object_index(tree).expect("tree exists");
        assert!(engine.objects[tree_idx].state.on_fire, "tree caught fire");
        assert_eq!(engine.objects[tree_idx].state.fire_caused_by, 7);
        assert_eq!(engine.rng, mirror, "contact draw then FirePhase draw");
        Ok(())
    }

    #[test]
    fn cross_check_fight_pass_engages_hostile_fight_ready_objects() -> Result<(), EngineError> {
        // CrossCheck pass 1 (C4GameObjects.cpp:97-138): on Tick5 frames,
        // FightReady objects standing at a hostile FightReady object's shape
        // start fighting both ways (ObjectActionFight = SetActionByName
        // "Fight" with target, C4ObjectCom.cpp:157-160), unless a RejectFight
        // callback vetoes (C4GameObjects.cpp:131-132).
        fn fighter_def(id: &str, script: &str) -> Result<Definition, EngineError> {
            let mut definition = Definition::from_script(id, id, script)?;
            definition.set_crew_member(true);
            // Fighters are livings: OCF_FightReady needs OCF_Alive, which
            // needs Category & C4D_Living (SetOCF, C4Object.cpp:600-610).
            definition.set_category(CATEGORY_LIVING);
            definition.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 16)));
            let mut specs = HashMap::new();
            specs.insert("Idle".to_string(), ActionSpec::default());
            specs.insert("Fight".to_string(), ActionSpec::default());
            definition.configure_actions(Some("Idle".to_string()), specs);
            Ok(definition)
        }
        const PLAIN: &str = r#"
        global func Initialize(state, random) { return 0; }
        "#;

        let mut engine = Engine::with_seed(50);
        engine.register_definition(fighter_def("KnightA", PLAIN)?)?;
        engine.register_definition(fighter_def("KnightB", PLAIN)?)?;
        engine.register_definition(simple_definition("FightLayer"))?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        engine.set_hostility(1, 2, true)?;
        let layer_a = engine.spawn_object(SpawnConfig::new("FightLayer"))?;
        let layer_b = engine.spawn_object(SpawnConfig::new("FightLayer"))?;

        let knight_a = engine.spawn_object(
            SpawnConfig::new("KnightA")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50))
                .with_layer(layer_a),
        )?;
        let knight_b = engine.spawn_object(
            SpawnConfig::new("KnightB")
                .with_owner(2)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(52, 50))
                .with_layer(layer_b),
        )?;

        // Frame 4 is not a Tick5 frame: nothing happens.
        engine.cross_check(4)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");

        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        let idx_b = engine.find_object_index(knight_b).expect("knight B");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");
        assert_ne!(engine.objects[idx_b].state.action.name, "Fight");

        engine.apply_object_update(knight_b, ObjectUpdate::new().with_layer(layer_a))?;
        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        let idx_b = engine.find_object_index(knight_b).expect("knight B");
        assert_eq!(engine.objects[idx_a].state.action.name, "Fight");
        assert_eq!(engine.objects[idx_b].state.action.name, "Fight");
        assert_eq!(engine.objects[idx_a].state.action.target, Some(knight_b));
        assert_eq!(engine.objects[idx_b].state.action.target, Some(knight_a));

        // Friendly players never fight (C4PlayerList::Hostile,
        // C4PlayerList.cpp:82-92).
        let mut engine = Engine::with_seed(51);
        engine.register_definition(fighter_def("KnightA", PLAIN)?)?;
        engine.register_definition(fighter_def("KnightB", PLAIN)?)?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        let knight_a = engine.spawn_object(
            SpawnConfig::new("KnightA")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50)),
        )?;
        let _knight_b = engine.spawn_object(
            SpawnConfig::new("KnightB")
                .with_owner(2)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(52, 50)),
        )?;
        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");

        // A truthy RejectFight callback on either side vetoes the fight.
        let mut engine = Engine::with_seed(52);
        engine.register_definition(fighter_def(
            "KnightA",
            r#"
            func RejectFight(enemy) { return 1; }
            "#,
        )?)?;
        engine.register_definition(fighter_def("KnightB", PLAIN)?)?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        engine.set_hostility(1, 2, true)?;
        let knight_a = engine.spawn_object(
            SpawnConfig::new("KnightA")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50)),
        )?;
        let _knight_b = engine.spawn_object(
            SpawnConfig::new("KnightB")
                .with_owner(2)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(52, 50)),
        )?;
        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");
        Ok(())
    }

    #[test]
    fn cross_check_contained_fight_runs_on_tick10() -> Result<(), EngineError> {
        // CrossCheck pass 3 (C4GameObjects.cpp:199-230): contained FightReady
        // objects in the same container fight hostile company on Tick10
        // frames — with no RejectFight veto. Pass 1 explicitly skips
        // contained objects (C4GameObjects.cpp:114), so frame 5 does nothing.
        fn fighter_def(id: &str) -> Result<Definition, EngineError> {
            let mut definition = Definition::from_script(
                id,
                id,
                r#"
                global func Initialize(state, random) { return 0; }
                "#,
            )?;
            definition.set_crew_member(true);
            // Fighters are livings: OCF_FightReady needs OCF_Alive, which
            // needs Category & C4D_Living (SetOCF, C4Object.cpp:600-610).
            definition.set_category(CATEGORY_LIVING);
            definition.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 16)));
            let mut specs = HashMap::new();
            specs.insert("Idle".to_string(), ActionSpec::default());
            specs.insert("Fight".to_string(), ActionSpec::default());
            definition.configure_actions(Some("Idle".to_string()), specs);
            Ok(definition)
        }

        let mut engine = Engine::with_seed(60);
        engine.register_definition(fighter_def("KnightA")?)?;
        engine.register_definition(fighter_def("KnightB")?)?;
        engine.register_definition(simple_definition("Hut"))?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        engine.set_hostility(1, 2, true)?;

        let hut =
            engine.spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(50, 50)))?;
        let knight_a = engine.spawn_object(
            SpawnConfig::new("KnightA")
                .with_owner(1)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50))
                .with_container(hut),
        )?;
        let knight_b = engine.spawn_object(
            SpawnConfig::new("KnightB")
                .with_owner(2)
                .with_crew_member(true)
                .with_alive(true)
                .with_position(Vector2::new(50, 50))
                .with_container(hut),
        )?;

        // Tick5 frame: pass 1 skips contained objects.
        engine.cross_check(5)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        assert_ne!(engine.objects[idx_a].state.action.name, "Fight");

        // Tick10 frame: contained fight engages both ways.
        engine.cross_check(10)?;
        let idx_a = engine.find_object_index(knight_a).expect("knight A");
        let idx_b = engine.find_object_index(knight_b).expect("knight B");
        assert_eq!(engine.objects[idx_a].state.action.name, "Fight");
        assert_eq!(engine.objects[idx_b].state.action.name, "Fight");
        assert_eq!(engine.objects[idx_a].state.action.target, Some(knight_b));
        assert_eq!(engine.objects[idx_b].state.action.target, Some(knight_a));
        Ok(())
    }

    #[test]
    fn cross_check_contained_fight_stops_after_the_first_hostile_content() -> Result<(), EngineError>
    {
        // C4GameObjects.cpp:222-227 aborts obj1's inner Contents walk when
        // obj1 is still contained after ObjectActionFight. This deliberately
        // preserves the C++ copy/paste quirk: A targets B, the first hostile
        // content, rather than continuing on to C.
        let mut fighter = Definition::from_script("Knight", "Knight", "#strict 2\n")?;
        fighter.set_category(CATEGORY_LIVING);
        let mut specs = HashMap::new();
        specs.insert("Idle".to_string(), ActionSpec::default());
        specs.insert("Fight".to_string(), ActionSpec::default());
        fighter.configure_actions(Some("Idle".to_string()), specs);

        let mut engine = Engine::new();
        engine.register_definition(fighter)?;
        engine.register_definition(simple_definition("Hut"))?;
        engine.register_player(PlayerConfig::new(1, "P1"))?;
        engine.register_player(PlayerConfig::new(2, "P2"))?;
        engine.set_hostility(1, 2, true)?;

        let hut = engine.spawn_object(SpawnConfig::new("Hut"))?;
        let knight_a = engine.spawn_object(
            SpawnConfig::new("Knight")
                .with_owner(1)
                .with_alive(true)
                .with_container(hut),
        )?;
        // Enter C before B so same-definition stContents insertion leaves
        // [B, C, A]. stMain has the same B/C/A forward order, making A the
        // last outer object and its own final target directly observable.
        let knight_c = engine.spawn_object(
            SpawnConfig::new("Knight")
                .with_owner(2)
                .with_alive(true)
                .with_container(hut),
        )?;
        let knight_b = engine.spawn_object(
            SpawnConfig::new("Knight")
                .with_owner(2)
                .with_alive(true)
                .with_container(hut),
        )?;
        assert_eq!(
            engine.object_snapshot(hut).expect("hut exists").contents,
            vec![knight_b, knight_c, knight_a]
        );
        let fighters = engine
            .debug_exec_order()
            .into_iter()
            .rev()
            .filter(|id| [knight_a, knight_b, knight_c].contains(id))
            .collect::<Vec<_>>();
        assert_eq!(fighters, vec![knight_b, knight_c, knight_a]);

        engine.cross_check(10)?;

        let action_target = |object_id| {
            let index = engine.find_object_index(object_id).expect("knight exists");
            assert_eq!(engine.objects[index].state.action.name, "Fight");
            engine.objects[index].state.action.target
        };
        assert_eq!(action_target(knight_a), Some(knight_b));
        assert_eq!(action_target(knight_b), Some(knight_a));
        assert_eq!(action_target(knight_c), Some(knight_a));
        Ok(())
    }

    #[test]
    fn cross_check_hit_respects_query_catch_blow() -> Result<(), EngineError> {
        // C4GameObjects.cpp:168: a truthy QueryCatchBlow callback on the
        // victim suppresses the hit entirely.
        let mut engine = Engine::with_seed(41);
        let mut victim_def = Definition::from_script(
            "Guard",
            "Guard",
            r#"
            func QueryCatchBlow(by) { return 1; }
            "#,
        )
        .expect("script compiles");
        victim_def.set_mass(100);
        engine.register_definition(victim_def)?;
        let mut rock_def = simple_definition("Rock");
        rock_def.set_category(CATEGORY_OBJECT);
        rock_def.set_mass(50);
        engine.register_definition(rock_def)?;

        let victim = engine.spawn_object(
            SpawnConfig::new("Guard")
                .with_position(Vector2::new(50, 50))
                .with_alive(true)
                .with_energy(100),
        )?;
        let _rock = engine.spawn_object(
            SpawnConfig::new("Rock")
                .with_position(Vector2::new(50, 50))
                .with_velocity(Vector2::new(5, 0)),
        )?;

        engine.cross_check(1)?;
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.energy, 100,
            "QueryCatchBlow rejected the blow"
        );
        assert_eq!(
            engine.objects[victim_idx].fixed_velocity.x,
            math::C4Fixed::ZERO,
            "no fling on rejected blow"
        );
        Ok(())
    }

    #[test]
    fn cross_check_reverse_area_hit_uses_live_con_scaled_shape() -> Result<(), EngineError> {
        // CrossCheck's pass-2 sector query uses C4Object::Area, whose Top/Height
        // accessors expand a short construction shape upward to 18 pixels. The
        // final Inside check does not: C++ tests the raw live Shape
        // (C4GameObjects.cpp:156-160). Put the rock in that expanded-only band
        // so a Con=50 victim rejects it while the same full-Con shape accepts it.
        fn run_case(construction: i32) -> Result<(Option<Value>, i32), EngineError> {
            let mut engine = Engine::with_seed(199);
            let mut victim_def = Definition::from_script(
                "Victim",
                "Victim",
                "#strict 2\nlocal query_calls;\nfunc QueryCatchBlow(by) { query_calls += 1; return 0; }\n",
            )?;
            victim_def.set_category(CATEGORY_LIVING);
            victim_def.set_mass(100);
            victim_def.set_physical(PhysicalInfo {
                energy: 100_000,
                ..PhysicalInfo::default()
            });
            victim_def.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
            engine.register_definition(victim_def)?;

            let mut rock_def = simple_definition("Rock");
            rock_def.set_category(CATEGORY_OBJECT);
            rock_def.set_mass(50);
            engine.register_definition(rock_def)?;

            let victim = engine.spawn_object(
                SpawnConfig::new("Victim")
                    .with_loaded(true)
                    .with_alive(true)
                    .with_energy(100_000)
                    .with_construction(construction)
                    .with_position(Vector2::new(50, 50)),
            )?;
            let _rock = engine.spawn_object(
                SpawnConfig::new("Rock")
                    .with_loaded(true)
                    .with_position(Vector2::new(50, 42))
                    .with_velocity(Vector2::new(5, 0)),
            )?;

            let victim_idx = engine.find_object_index(victim).expect("victim exists");
            let expected_shape = if construction == FULL_CON {
                DefinitionRect::new(-10, -10, 20, 20)
            } else {
                DefinitionRect::new(-10, -5, 20, 10)
            };
            assert_eq!(
                engine.objects[victim_idx].current_shape_rect(),
                Some(expected_shape),
                "the test must distinguish the live and definition shapes"
            );

            engine.cross_check(1)?;

            let victim_idx = engine.find_object_index(victim).expect("victim remains");
            let victim = &engine.objects[victim_idx];
            Ok((
                victim.state.local_vars.get("query_calls").cloned(),
                victim.state.energy,
            ))
        }

        let (partial_calls, partial_energy) = run_case(FULL_CON / 2)?;
        assert_eq!(
            partial_calls, None,
            "the rock is outside the Con=50 live Shape"
        );
        assert_eq!(partial_energy, 100_000);

        let (full_calls, full_energy) = run_case(FULL_CON)?;
        assert_eq!(
            full_calls,
            Some(Value::Int(1)),
            "the unchanged full-Con Shape still receives the hit"
        );
        assert!(full_energy < 100_000);
        Ok(())
    }

    #[test]
    fn cross_check_collection_prefers_master_forward_living_collector() -> Result<(), EngineError> {
        // CrossCheck's outer loop walks the C++ main list First->Next. Its
        // category-descending order puts Living before StaticBack even though
        // the StaticBack collector was spawned first (C4GameObjects.cpp:151;
        // C4ObjectList.cpp:165-175).
        let mut engine = Engine::new();
        let mut collector = simple_definition("Collector");
        collector.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
        collector.set_collection_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_definition(collector)?;
        let mut item = simple_definition("Item");
        item.set_category(CATEGORY_OBJECT);
        item.set_collectible(true);
        engine.register_definition(item)?;

        // Fresh shaped objects start at their Con=0 bottom. For this rect,
        // y=60 grows to center y=50; keep the shapeless item at that center.
        let collector_position = Vector2::new(50, 60);
        let item_position = Vector2::new(50, 50);
        let static_collector = engine.spawn_object(
            SpawnConfig::new("Collector")
                .with_category(CATEGORY_STATIC_BACK)
                .with_position(collector_position),
        )?;
        let living_collector = engine.spawn_object(
            SpawnConfig::new("Collector")
                .with_category(CATEGORY_LIVING)
                .with_alive(true)
                .with_position(collector_position),
        )?;
        let item = engine.spawn_object(SpawnConfig::new("Item").with_position(item_position))?;

        assert_eq!(
            engine.debug_exec_order(),
            vec![static_collector, living_collector, item]
        );

        for collector in [static_collector, living_collector] {
            let index = engine
                .find_object_index(collector)
                .expect("collector exists");
            assert_ne!(engine.object_ocf_at_index(index) & ocf::COLLECTION, 0);
        }
        let item_index = engine.find_object_index(item).expect("item exists");
        assert_ne!(engine.object_ocf_at_index(item_index) & ocf::CARRYABLE, 0);

        engine.cross_check(3)?;

        assert_eq!(
            engine.object_snapshot(item).expect("item exists").container,
            Some(living_collector)
        );
        assert!(engine
            .object_snapshot(static_collector)
            .expect("StaticBack collector exists")
            .contents
            .is_empty());
        Ok(())
    }

    #[test]
    fn cross_check_hits_newest_same_definition_victim_first() -> Result<(), EngineError> {
        // stMain inserts a newer same-definition object before the existing
        // cluster in forward order. B must therefore receive both hit
        // callbacks before A (C4ObjectList.cpp:151-175;
        // C4GameObjects.cpp:151,167-179).
        let victim_script = r#"#strict 2
local query_order, catch_order;
func QueryCatchBlow(by)
{
    query_order = GetGravity();
    SetGravity(query_order + 1);
    return 0;
}
func CatchBlow(level, by)
{
    catch_order = GetGravity();
    SetGravity(catch_order + 1);
    return 1;
}
"#;
        let mut engine = Engine::new();
        let mut physics = engine.physics();
        physics.gravity = 40;
        engine.set_physics(physics);
        let mut victim = Definition::from_script("Victim", "Victim", victim_script)?;
        victim.set_category(CATEGORY_LIVING);
        victim.set_mass(100);
        victim.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        victim.set_shape_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        engine.register_definition(victim)?;
        let mut rock = simple_definition("Rock");
        rock.set_category(CATEGORY_OBJECT);
        rock.set_mass(50);
        engine.register_definition(rock)?;

        // The shaped victims grow upward from y=55 to center y=50; the
        // shapeless rock is placed directly at that center.
        let victim_spawn_position = Vector2::new(50, 55);
        let rock_position = Vector2::new(50, 50);
        let victim_a = engine.spawn_object(
            SpawnConfig::new("Victim")
                .with_alive(true)
                .with_energy(100_000)
                .with_position(victim_spawn_position),
        )?;
        let victim_b = engine.spawn_object(
            SpawnConfig::new("Victim")
                .with_alive(true)
                .with_energy(100_000)
                .with_position(victim_spawn_position),
        )?;
        let rock = engine.spawn_object(
            SpawnConfig::new("Rock")
                .with_position(rock_position)
                .with_velocity(Vector2::new(5, 0)),
        )?;

        assert_eq!(engine.debug_exec_order(), vec![victim_a, victim_b, rock]);
        for victim in [victim_a, victim_b] {
            let index = engine.find_object_index(victim).expect("victim exists");
            assert_ne!(engine.object_ocf_at_index(index) & ocf::ALIVE, 0);
        }
        let rock_index = engine.find_object_index(rock).expect("rock exists");
        assert_ne!(engine.object_ocf_at_index(rock_index) & ocf::HIT_SPEED2, 0);

        engine.cross_check(1)?;

        let callback_orders = |object_id| {
            let index = engine.find_object_index(object_id).expect("victim exists");
            let locals = &engine.objects[index].state.local_vars;
            (
                locals.get("query_order").cloned(),
                locals.get("catch_order").cloned(),
            )
        };
        assert_eq!(
            callback_orders(victim_b),
            (Some(Value::Int(40)), Some(Value::Int(41)))
        );
        assert_eq!(
            callback_orders(victim_a),
            (Some(Value::Int(42)), Some(Value::Int(43)))
        );
        assert_eq!(engine.physics().gravity, 44);
        Ok(())
    }
