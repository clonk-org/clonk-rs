// Contiguous slice 11 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: objects, object state, misc.

    #[test]
    fn place_vegetation_uses_relative_surface_area_and_raw_growth_like_cpp() {
        // FnPlaceVegetation makes x/y caller-relative (C4Script.cpp:2487-2492),
        // then the surface arm draws x/y, applies AboveSemiSolid, checks Soil,
        // and passes the raw growth value to CreateObjectConstruction with
        // NO_OWNER (C4Game.cpp:2980-3022).
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\nName=Earth\nDensity=100\nSoil=1\n",
        )
        .expect("earth material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth material exists");
        let mut landscape = Landscape::flat_with_material(400, 160, Some(earth));
        landscape.set_world_height(300);
        let definitions = HashMap::from([(
            DefinitionId::from("TREE"),
            DefinitionMetadata {
                category: crate::CATEGORY_STATIC_BACK,
                shape: Some(DefinitionRect::new(-20, -28, 40, 56)),
                placement: 0,
                growth: 2,
                stretch_growth: true,
                ..DefinitionMetadata::default()
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            2,
            false,
        )
        .with_materials(Some(Rc::new(materials)));
        let caller = HostObjectContext {
            position: Vector2::new(200, 150),
            ..idle_object_context()
        };
        let mut expected_rng = LcgRng::new(17);
        assert_eq!(expected_rng.random(200), 94);
        assert_eq!(expected_rng.random(200), 2);
        let guard = enter_random_context(LcgRng::new(17));
        let (result, outcome) = with_effect_context(Some(caller), &[], world, 2, || {
            place_vegetation(&[
                Value::C4Id("TREE".into()),
                Value::Int(-100),
                Value::Int(-100),
                Value::Int(200),
                Value::Int(200),
                Value::Int(10),
            ])
        });
        let rng_after = guard.finish();

        assert_eq!(
            rng_after, expected_rng,
            "exactly the x/y draws are consumed"
        );
        assert_eq!(
            result.expect("PlaceVegetation succeeds"),
            object_reference_value(ObjectId::new(2))
        );
        assert_eq!(outcome.spawns.len(), 1);
        let spawn = &outcome.spawns[0];
        assert_eq!(spawn.definition_id, "TREE");
        assert_eq!(spawn.position, Vector2::new(194, 168));
        assert_eq!(
            spawn.construction, 10,
            "growth is FullCon scale, not percent"
        );
        assert_eq!(spawn.owner, OWNER_NONE);
        assert_eq!(spawn.controller, Some(OWNER_NONE));
    }

    #[test]
    fn place_vegetation_runs_construction_before_partial_growth_like_cpp() {
        // NewObject inserts the object, calls Construction at Con=0, then
        // DoCon(iGrowth, true); Completion+Initialize only run when that
        // transition reaches FullCon (C4Game.cpp:1135-1144;
        // C4Object.cpp:1428-1515). FnPlaceVegetation returns that live object.
        let script = r#"#strict
local iConstructionCon, iCompletion, iInitialized, iObservedCon, iCompletionRock;

protected func Construction()
{
    iConstructionCon = GetCon();
    SetComponent(ROCK, 0);
    DoCon(1);
    return(1);
}

protected func Completion()
{
    iCompletion = 1;
    iCompletionRock = GetComponent(ROCK);
    return(1);
}

protected func Initialize()
{
    iInitialized = 1;
    return(1);
}

public func ConstructionCon()
{
    return(iConstructionCon);
}

public func Seed()
{
    var child = PlaceVegetation(TREE, -100, -100, 200, 200, 10);
    iObservedCon = child->ConstructionCon();
    return(child);
}

public func SeedFull()
{
    return(PlaceVegetation(TREE, -100, -100, 200, 200, 100000));
}
"#;
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\nName=Earth\nDensity=100\nSoil=1\n",
        )
        .expect("earth material parses");
        let mut engine = crate::Engine::with_seed(17);
        engine.configure_materials_from_library(&library);
        let earth = engine
            .materials()
            .id_of("Earth")
            .expect("earth material exists");
        let mut landscape = Landscape::flat_with_material(400, 160, Some(earth));
        landscape.set_world_height(300);
        engine.set_landscape(landscape);

        let mut tree =
            crate::Definition::from_script("TREE", "Tree", script).expect("tree script compiles");
        tree.set_category(crate::CATEGORY_STATIC_BACK);
        tree.set_shape_rect(Some(DefinitionRect::new(-20, -28, 40, 56)));
        tree.set_placement(0);
        tree.set_growth(2);
        tree.set_stretch_growth(true);
        tree.set_components(vec![crate::DefinitionComponent {
            id: "ROCK".to_owned(),
            count: 100,
        }]);
        engine.register_definition(tree).expect("tree registers");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("TREE")
                    .with_position(Vector2::new(200, 160))
                    .with_category(crate::CATEGORY_STATIC_BACK),
            )
            .expect("caller tree spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let value = engine
            .call_object_function(caller_index, "Seed", Vec::new())
            .expect("Seed runs");
        let child_id = object_id_from_value(&value).expect("Seed returns child");
        let child_index = engine.find_object_index(child_id).expect("child exists");
        let child = &engine.objects[child_index].state;

        assert_eq!(
            child.construction, 1_010,
            "the initial DoCon adds raw growth after Construction's change"
        );
        assert_eq!(
            child.local_vars.get("iConstructionCon"),
            Some(&Value::Int(0)),
            "Construction observes the pre-growth Con=0 state"
        );
        assert_eq!(child.local_vars.get("iCompletion"), Some(&Value::Nil));
        assert_eq!(child.local_vars.get("iInitialized"), Some(&Value::Nil));
        assert_eq!(child.components.get("ROCK"), Some(&1));
        let caller_index = engine.find_object_index(caller).expect("caller remains");
        assert_eq!(
            engine.objects[caller_index]
                .state
                .local_vars
                .get("iObservedCon"),
            Some(&Value::Int(0)),
            "the Construction write is visible before PlaceVegetation returns"
        );

        let value = engine
            .call_object_function(caller_index, "SeedFull", Vec::new())
            .expect("SeedFull runs");
        let full_child = object_id_from_value(&value).expect("SeedFull returns child");
        let full_index = engine
            .find_object_index(full_child)
            .expect("full child exists");
        let full = &engine.objects[full_index].state;
        assert_eq!(full.construction, FULL_CON);
        assert_eq!(
            full.local_vars.get("iConstructionCon"),
            Some(&Value::Int(0))
        );
        assert_eq!(full.local_vars.get("iCompletion"), Some(&Value::Int(1)));
        assert_eq!(
            full.local_vars.get("iCompletionRock"),
            Some(&Value::Int(100))
        );
        assert_eq!(full.local_vars.get("iInitialized"), Some(&Value::Int(1)));
        assert_eq!(full.components.get("ROCK"), Some(&100));
    }

    #[test]
    fn place_vegetation_finds_liquid_bottom_like_cpp() {
        // C4D_Place_Liquid takes one random point, tries FindSurfaceLiquid
        // then FindLiquid, settles via SemiAboveSolid, and creates three
        // pixels into the bottom (C4Game.cpp:3027-3039;
        // C4Landscape.cpp:1860-1915,1772-1796).
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Rock]
            Name=Rock
            Density=100

            [Material Water]
            Name=Water
            Density=25
            "#,
        )
        .expect("materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let rock = materials.id_of("Rock").expect("rock exists");
        let water = materials.id_of("Water").expect("water exists");
        let mut landscape = Landscape::flat_with_material(100, 200, Some(rock));
        landscape.set_world_height(300);
        for x in 20..80 {
            for y in 100..200 {
                assert!(landscape.insert_liquid_at(x, y, Some(water)));
            }
        }
        let definitions = HashMap::from([(
            DefinitionId::from("PLNT"),
            DefinitionMetadata {
                shape: Some(DefinitionRect::new(-2, -3, 4, 6)),
                placement: 1,
                ..DefinitionMetadata::default()
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        )
        .with_materials(Some(Rc::new(materials)));
        let mut expected_rng = LcgRng::new(23);
        assert_eq!(expected_rng.random(1), 0);
        assert_eq!(expected_rng.random(1), 0);
        let guard = enter_random_context(LcgRng::new(23));
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            place_vegetation(&[
                Value::C4Id("PLNT".into()),
                Value::Int(50),
                Value::Int(120),
                Value::Int(1),
                Value::Int(1),
                Value::Int(500),
            ])
        });
        let rng_after = guard.finish();

        assert_eq!(rng_after, expected_rng, "the point costs exactly two draws");
        assert_eq!(
            result.expect("PlaceVegetation succeeds"),
            object_reference_value(ObjectId::new(1))
        );
        assert_eq!(outcome.spawns.len(), 1);
        assert_eq!(outcome.spawns[0].position, Vector2::new(49, 202));
        assert_eq!(outcome.spawns[0].construction, 500);
    }

    #[test]
    fn place_vegetation_default_growth_uses_cpp_random_gate() {
        // iGrowth<=0 first becomes FullCon; a definition with Growth then
        // has a 1-in-3 gate followed by Random(FullCon)+1
        // (C4Game.cpp:2988-2992), before the placement-point draws.
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\nName=Earth\nDensity=100\nSoil=1\n",
        )
        .expect("earth material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut landscape = Landscape::flat_with_material(400, 160, Some(earth));
        landscape.set_world_height(300);
        let definitions = HashMap::from([(
            DefinitionId::from("TREE"),
            DefinitionMetadata {
                shape: Some(DefinitionRect::new(-20, -28, 40, 56)),
                placement: 0,
                growth: 2,
                stretch_growth: true,
                ..DefinitionMetadata::default()
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        )
        .with_materials(Some(Rc::new(materials)));
        let mut expected_rng = LcgRng::new(2);
        assert_eq!(expected_rng.random(3), 0);
        let expected_growth = expected_rng.random(FULL_CON) + 1;
        assert_eq!(expected_growth, 29_217);
        assert_eq!(expected_rng.random(1), 0);
        assert_eq!(expected_rng.random(1), 0);
        let guard = enter_random_context(LcgRng::new(2));
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            place_vegetation(&[
                Value::C4Id("TREE".into()),
                Value::Int(200),
                Value::Int(50),
                Value::Int(1),
                Value::Int(1),
                Value::Int(0),
            ])
        });
        let rng_after = guard.finish();

        assert_eq!(rng_after, expected_rng);
        assert_eq!(
            result.expect("PlaceVegetation succeeds"),
            object_reference_value(ObjectId::new(1))
        );
        assert_eq!(outcome.spawns[0].construction, expected_growth);
    }

    #[test]
    fn create_contents_consumes_explicit_zero_container_before_count() {
        // Tutorial06 calls HUT3->CreateContents(METL, 0, 2) and WOOD with
        // count 4. The typed object slot converts integer zero to nullptr;
        // iCount remains the following argument (C4Script.cpp:1938-1951).
        // Object number 1 belongs to the active container; C++'s global
        // allocator therefore starts the first created content at 2.
        let (result, outcome) =
            with_object_host_context_with_world_and_next_id(HostWorldContext::default(), 2, || {
                create_contents(&[Value::C4Id("WOOD".into()), Value::Int(0), Value::Int(4)])
            });

        assert_eq!(
            result.expect("CreateContents accepts the null object slot"),
            object_reference_value(ObjectId::new(5))
        );
        assert_eq!(outcome.spawns.len(), 4);
        assert!(outcome.spawns.iter().all(|spawn| {
            spawn.definition_id == "WOOD" && spawn.container == Some(ObjectId::new(1))
        }));
    }

    #[test]
    fn create_construction_counts_tutorial06_cave_pixels_not_roof_surface() {
        // Tutorial06 is TopOpen=0: the valid ELEV site at (332,148) has a
        // solid cave roof above it and floor support below it. C++ counts
        // the actual pixels in the construction rectangle and its five-row
        // support strip (C4Landscape.cpp:1090-1098,2125-2158); it does not
        // treat the first solid surface in each column as solid forever.
        const WIDTH: u32 = 64;
        const HEIGHT: u32 = 200;
        let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize];
        for y in 0..25 {
            pixels[y * WIDTH as usize..(y + 1) * WIDTH as usize].fill(1);
        }
        for y in 150..HEIGHT as usize {
            pixels[y * WIDTH as usize..(y + 1) * WIDTH as usize].fill(1);
        }
        let mut densities = vec![0; 2];
        densities[1] = 100;
        let mut landscape =
            Landscape::new(WIDTH, vec![0; WIDTH as usize]).expect("Tutorial06 cave fixture builds");
        landscape.set_world_height(HEIGHT as i32);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            WIDTH,
            HEIGHT,
            pixels,
            densities,
            vec![None; 2],
            vec![None; 2],
        ));
        landscape.refresh_all_raster_columns();
        assert_eq!(landscape.surface_height(32), Some(0), "the roof is first");

        let definitions = HashMap::from([(
            DefinitionId::from("ELEV"),
            DefinitionMetadata {
                category: crate::CATEGORY_STRUCTURE,
                constructable: true,
                shape: Some(DefinitionRect::new(-14, -28, 28, 56)),
                ..DefinitionMetadata::default()
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::C4Id("ELEV".into()),
            Value::Int(32),
            Value::Int(148),
            Value::Int(1),
            Value::Int(1),
            Value::Bool(true),
            Value::Bool(true),
        ];

        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || create_construction(&args));

        assert_eq!(
            result.expect("CreateConstruction checks the cave site"),
            object_reference_value(ObjectId::new(1))
        );
        assert_eq!(outcome.spawns.len(), 1);
        assert!(matches!(
            outcome.landscape.as_slice(),
            [LandscapeOperation::PrepareConstructionTerrain {
                center_x: 32,
                bottom_y: 148,
                width: 28,
                height: 56,
                basement: 0,
            }]
        ));
    }

    #[test]
    fn create_construction_registers_spawn_when_site_valid() {
        let landscape = Landscape::flat(64, 50);
        let definitions = HashMap::from([(
            "WORK".to_string(),
            DefinitionMetadata {
                category: crate::CATEGORY_STRUCTURE,
                ocf_base: ocf::NORMAL,
                mass: 100,
                constructable: true,
                shape: Some(DefinitionRect::new(-10, -40, 20, 40)),
                components: vec![("WOOD".to_owned(), 4)],
                ..Default::default()
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::C4Id("WORK".into()),
            Value::Int(32),
            Value::Int(50),
            Value::Int(1),
            Value::Int(50),
            Value::Bool(false),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || create_construction(&args));
        let value = result.expect("CreateConstruction succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(1)));
        assert_eq!(outcome.spawns.len(), 1);
        let spawn = &outcome.spawns[0];
        assert_eq!(spawn.definition_id, "WORK");
        assert_eq!(spawn.position, Vector2::new(32, 50));
        assert_eq!(spawn.owner, 1);
        assert_eq!(spawn.construction, crate::FULL_CON / 2);
        assert_eq!(spawn.category, Some(crate::CATEGORY_STRUCTURE));
        let component_update = outcome
            .other_objects
            .iter()
            .find(|nested| nested.object_id == ObjectId::new(1))
            .and_then(|nested| nested.update.as_ref())
            .expect("initial DoCon stages the construction components");
        assert_eq!(
            component_update
                .components
                .as_ref()
                .and_then(|components| components.get("WOOD")),
            Some(&2)
        );
        assert_eq!(
            component_update.component_order.as_deref(),
            Some(["WOOD".to_owned()].as_slice())
        );
        assert_eq!(outcome.next_object_id, 2);
    }

    #[test]
    fn create_construction_uses_open_side_border_pixels_for_site_checks() {
        let mut landscape = Landscape::flat(64, 50);
        landscape.set_border_open(60, 0, true, false);
        let definitions = HashMap::from([(
            DefinitionId::from("WORK"),
            DefinitionMetadata {
                category: crate::CATEGORY_STRUCTURE,
                constructable: true,
                shape: Some(DefinitionRect::new(-10, -40, 20, 40)),
                ..DefinitionMetadata::default()
            },
        )]);
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::C4Id("WORK".into()),
            Value::Int(5),
            Value::Int(50),
            Value::Int(1),
            Value::Int(50),
            Value::Bool(false),
            Value::Bool(true),
        ];

        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || create_construction(&args));

        assert_eq!(
            result.expect("open side permits the partial construction rectangle"),
            object_reference_value(ObjectId::new(1))
        );
        assert_eq!(outcome.spawns.len(), 1);
        assert_eq!(outcome.spawns[0].position, Vector2::new(5, 50));
    }

    #[test]
    fn create_construction_prepares_terrain_before_the_construction_callback() {
        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            DigFree=1

            [Material Granite]
            Name=Granite
            Density=100
            DigFree=0
            "#,
        )
        .expect("materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let mut landscape = Landscape::new(40, vec![0; 40]).expect("landscape builds");
        landscape.set_world_height(40);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            40,
            40,
            vec![1; 40 * 40],
            vec![0, 100, 100],
            vec![None, Some("Earth".to_owned()), Some("Granite".to_owned())],
            vec![None; 3],
        ));

        let mut engine = crate::Engine::with_seed(5);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        let mut structure = crate::Definition::from_script(
            "HUT1",
            "Hut",
            r#"#strict
local saw_clear_footprint, saw_granite_basement;
protected func Construction()
{
    saw_clear_footprint = GBackSky(0, -4);
    saw_granite_basement = GetMaterial(0, 0) == Material("Granite");
}
"#,
        )
        .expect("structure script compiles");
        structure.set_category(crate::CATEGORY_STRUCTURE);
        structure.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 8)));
        structure.set_basement(1);
        engine
            .register_definition(structure)
            .expect("structure registers");
        engine
            .register_definition(
                crate::Definition::from_script(
                    "CALL",
                    "Builder",
                    "#strict\npublic func Build() { return CreateConstruction(HUT1, 0, 0, -1, 100, true, false); }",
                )
                .expect("builder script compiles"),
            )
            .expect("builder registers");
        let builder = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(20, 30)))
            .expect("builder spawns");
        let builder_index = engine.find_object_index(builder).expect("builder exists");

        let structure = engine
            .call_object_function(builder_index, "Build", Vec::new())
            .expect("construction succeeds");
        let structure = object_id_from_value(&structure).expect("structure returned");
        let structure = engine
            .object_snapshot(structure)
            .expect("structure survives");

        assert_eq!(
            structure.local_vars.get("saw_clear_footprint"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            structure.local_vars.get("saw_granite_basement"),
            Some(&Value::Bool(true))
        );
        assert_eq!(engine.debug_landscape_material_name(20, 26), None);
        assert_eq!(
            engine.debug_landscape_material_name(20, 30).as_deref(),
            Some("Granite")
        );
        assert_eq!(
            engine.debug_landscape_material_name(15, 26).as_deref(),
            Some("Earth"),
            "terrain outside the footprint remains untouched"
        );
    }

    #[test]
    fn create_construction_zero_completion_is_removed_before_return() {
        // NewObject starts at Con=0, then DoCon(iCon, true). With iCon=0,
        // DoCon calls AssignRemoval and NewObject returns nullptr after its
        // status re-check (C4Game.cpp:1110-1129; C4Object.cpp:1513-1517).
        let world = HostWorldContext::with_landscape(
            Vec::new(),
            None,
            HashMap::from([(
                DefinitionId::from("WORK"),
                DefinitionMetadata {
                    category: crate::CATEGORY_STRUCTURE,
                    constructable: true,
                    ..DefinitionMetadata::default()
                },
            )]),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::C4Id("WORK".into()),
            Value::Int(32),
            Value::Int(50),
            Value::Int(1),
            Value::Int(0),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || create_construction(&args));

        assert_eq!(result.expect("CreateConstruction completes"), Value::Nil);
        assert!(outcome.spawns.is_empty());
        assert_eq!(outcome.next_object_id, 2, "removed object consumed its id");
    }

    #[test]
    fn create_construction_returns_nil_when_site_blocked() {
        let landscape = Landscape::flat(64, 50);
        let workshop_metadata = DefinitionMetadata {
            category: crate::CATEGORY_STRUCTURE,
            ocf_base: ocf::NORMAL,
            mass: 100,
            constructable: true,
            shape: Some(DefinitionRect::new(-10, -40, 20, 40)),
            ..Default::default()
        };
        let definitions = HashMap::from([
            ("WORK".to_string(), workshop_metadata.clone()),
            ("EXST".to_string(), workshop_metadata),
        ]);
        let existing = HostWorldObject::with_category(
            ObjectId::new(10),
            "EXST",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            crate::CATEGORY_STRUCTURE,
            0,
            crate::FULL_CON,
            0,
            Vector2::new(32, 50),
            Vector2::ZERO,
            0,
            Vec::new(),
            0,
            0,
            0, // action_phase
            None,
            None,
        );
        let world = HostWorldContext::with_landscape(
            vec![existing],
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::C4Id("WORK".into()),
            Value::Int(32),
            Value::Int(50),
            Value::Int(1),
            Value::Int(0),
            Value::Bool(false),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || create_construction(&args));
        let value = result.expect("CreateConstruction completes");
        assert_eq!(value, Value::Nil);
        assert!(outcome.spawns.is_empty());
        assert_eq!(outcome.next_object_id, 1);
    }

    #[test]
    fn create_particle_registers_command() {
        let args = [
            Value::String("Smoke".into()),
            Value::Int(8),
            Value::Int(-4),
            Value::Int(20),
            Value::Int(-10),
            Value::Int(15),
            Value::Int(60),
        ];
        let (result, outcome) = with_object_host_context(|| create_particle(&args));
        let value = result.expect("CreateParticle succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Create(config) => {
                assert_eq!(config.definition_id, "Smoke");
                assert_eq!(config.position, FloatVector2::new(8.0, -4.0));
                assert_eq!(config.velocity, FloatVector2::new(2.0, -1.0));
                assert_eq!(config.parameter_a, 1.5);
                assert_eq!(config.parameter_b, 60);
                assert_eq!(config.life, 60);
                assert!(matches!(config.layer, ParticleLayer::Global));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn create_particle_with_object_sets_layer() {
        let target_id = ObjectId::new(5);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            target_id,
            "Torch",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [
            Value::String("Spark".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(30),
            object_reference_value(target_id),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || create_particle(&args));
        let value = result.expect("CreateParticle succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Create(config) => {
                assert!(matches!(
                    config.layer,
                    ParticleLayer::ObjectBack(id) if id == target_id
                ));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn create_particle_rejects_unknown_object() {
        let args = [
            Value::String("Spark".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(10),
            object_reference_value(ObjectId::new(99)),
        ];
        let (result, outcome) = with_object_host_context(|| create_particle(&args));
        let value = result.expect("CreateParticle handles missing object");
        assert_eq!(value, Value::Bool(false));
        assert!(outcome.particles.is_empty());
    }

    fn find_world_object(id: u64, definition: &str, x: i32, y: i32, owner: i32) -> HostWorldObject {
        HostWorldObject::new(
            ObjectId::new(id),
            definition,
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            owner,
            100,
            crate::FULL_CON,
            Vector2::new(x, y),
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
    }

    #[test]
    fn find_base_uses_stored_base_and_cpp_master_order() {
        // FnFindBase validates the player before C4Game::FindBase walks
        // Objects.First -> Next, skips Status=0, and selects the indexed
        // object whose stored C4Object::Base matches (C4Script.cpp:1976-1979;
        // C4Game.cpp:3732-3744). Runtime stMain insertion puts the newest
        // same-definition object first in that FORWARD master-list walk
        // (C4ObjectList.cpp:110-180).
        let definition = crate::Definition::from_script("HUT1", "Hut", "#strict\n")
            .expect("definition compiles");
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("player registers");
        engine
            .register_definition(definition)
            .expect("definition registers");

        let older = engine
            .spawn_object(crate::SpawnConfig::new("HUT1"))
            .expect("older hut spawns");
        let newer = engine
            .spawn_object(crate::SpawnConfig::new("HUT1"))
            .expect("newer hut spawns");
        let owned_non_base = engine
            .spawn_object(crate::SpawnConfig::new("HUT1").with_owner(0))
            .expect("owned non-base hut spawns");
        let deleted_base = engine
            .spawn_object(crate::SpawnConfig::new("HUT1"))
            .expect("deleted base hut spawns");
        let inactive_base = engine
            .spawn_object(crate::SpawnConfig::new("HUT1"))
            .expect("inactive base hut spawns");
        let older_index = engine.find_object_index(older).expect("older exists");
        let newer_index = engine.find_object_index(newer).expect("newer exists");
        let deleted_index = engine
            .find_object_index(deleted_base)
            .expect("deleted base exists");
        let inactive_index = engine
            .find_object_index(inactive_base)
            .expect("inactive base exists");
        engine.objects[older_index].state.base = 0;
        engine.objects[newer_index].state.base = 0;
        engine.objects[deleted_index].state.base = 0;
        engine.objects[deleted_index].state.status = ObjectStatus::Deleted;
        engine.objects[inactive_index].state.base = 0;
        engine.objects[inactive_index].state.status = ObjectStatus::Inactive;

        assert_eq!(
            engine.debug_exec_order(),
            [older, newer, owned_non_base, deleted_base, inactive_base]
        );
        let world = engine.host_world_context();
        let call = |args: Vec<Value>| {
            with_object_host_context_with_world(world.clone(), || find_base(&args))
                .0
                .expect("FindBase succeeds")
        };
        let first = call(vec![Value::Int(0)]);
        let second = call(vec![Value::Int(0), Value::Int(1)]);

        assert_eq!(
            object_id_from_value(&first),
            Some(newer),
            "Status, Base, not Owner/category, filters the forward master list"
        );
        assert_eq!(object_id_from_value(&second), Some(older));
        assert_eq!(
            object_id_from_value(&call(Vec::new())),
            Some(newer),
            "missing C4ValueInt slots default player/index to zero"
        );
        assert_eq!(
            call(vec![Value::Int(0), Value::Int(-1)]),
            Value::Nil,
            "a negative index decrements away from zero and never matches"
        );
        assert_eq!(
            call(vec![Value::Int(0), Value::Int(2)]),
            Value::Nil,
            "an out-of-range index returns nil"
        );
        assert_eq!(
            call(vec![Value::Int(7)]),
            Value::Nil,
            "FnFindBase rejects a player absent from Game.Players"
        );
    }

    #[test]
    fn get_base_reads_the_target_objects_stored_base() {
        // FnGetBase returns pObj->Base and falls back to the calling object;
        // without either object it returns NO_OWNER (C4Script.cpp:1406-1410).
        // Tutorial01 Script120 calls this global form with FindObject(HUT2).
        let definition = crate::Definition::from_script("HUT1", "Hut", "#strict\n")
            .expect("definition compiles");
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("player registers");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let hut = engine
            .spawn_object(crate::SpawnConfig::new("HUT1"))
            .expect("hut spawns");
        let hut_index = engine.find_object_index(hut).expect("hut exists");
        engine.objects[hut_index].state.base = 0;

        let world = engine.host_world_context();
        let (result, _) = with_effect_context(None, &[], world, 2, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    "#strict\nglobal func Probe(pObj) { return [GetBase(pObj), GetBase()]; }",
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[object_reference_value(hut)])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("GetBase succeeds"),
            Value::Array(vec![Value::Int(0), Value::Int(OWNER_NONE)])
        );
    }

    #[test]
    fn find_base_preserves_loaded_base_and_master_order() {
        // Base is compiled as part of C4Object (C4Object.cpp:2776), and
        // Objects.txt is decompiled back-to-front then loaded with stReverse
        // to reconstruct the same master list (C4ObjectList.cpp:507-529).
        // Therefore a save/restore must preserve both Base and the forward
        // FindBase traversal order (C4Game.cpp:3732-3744).
        let definition = crate::Definition::from_script("HUT1", "Hut", "#strict\n")
            .expect("definition compiles");
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("player registers");
        engine
            .register_definition(definition.clone())
            .expect("definition registers");
        let older = engine
            .spawn_object(crate::SpawnConfig::new("HUT1").with_loaded(true))
            .expect("older loaded hut spawns");
        let newer = engine
            .spawn_object(crate::SpawnConfig::new("HUT1").with_loaded(true))
            .expect("newer loaded hut spawns");
        let older_index = engine.find_object_index(older).expect("older exists");
        let newer_index = engine.find_object_index(newer).expect("newer exists");
        engine.objects[older_index].state.base = 0;
        engine.objects[newer_index].state.base = 0;

        let state = engine.capture_state();
        let mut restored = crate::Engine::with_seed(0);
        restored
            .register_definition(definition)
            .expect("definition registers for restore");
        restored.restore_state(&state).expect("state restores");
        let world = restored.host_world_context();
        let (first, _) = with_object_host_context_with_world(world, || find_base(&[Value::Int(0)]));

        assert_eq!(
            object_id_from_value(&first.expect("FindBase after restore succeeds")),
            Some(newer),
            "restored Base and forward master ordering stay authoritative"
        );
    }

    #[test]
    fn criterion_parsing_stops_at_first_falsy_par_like_cpp() {
        // CreateCriterionsFromPars stops at the first raw-falsy parameter
        // (`if (!Data) break;`, C4Script.cpp:1996): criteria after a nil
        // integer zero, or false argument are never parsed.
        let world = HostWorldContext::from_objects(vec![
            find_world_object(1, "ROCK", 10, 10, 1),
            find_world_object(2, "TREE", 50, 10, 2),
            find_world_object(3, "ROCK", 90, 10, 2),
        ]);
        for terminator in [Value::Nil, Value::Int(0), Value::Bool(false)] {
            // [ID ROCK], falsy, [Owner 2]: C++ uses only the ROCK criterion,
            // so both rock objects remain in the FindObjects result.
            let args = vec![
                Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
                terminator,
                Value::Array(vec![Value::Int(50), Value::Int(2)]),
            ];
            let (result, _) =
                with_object_host_context_with_world(world.clone(), || find_objects2(&args));
            let Value::Array(values) = result.expect("FindObjects succeeds") else {
                panic!("FindObjects should return an array");
            };
            assert_eq!(
                values.iter().map(object_id_from_value).collect::<Vec<_>>(),
                vec![Some(ObjectId::new(1)), Some(ObjectId::new(3))]
            );
        }
    }

    #[test]
    fn criterion_native_types_and_ten_parameter_cap_match_cpp() {
        let mut first_rock = find_world_object(1, "ROCK", 10, 10, 1);
        first_rock.category = crate::CATEGORY_LIVING;
        let mut tree = find_world_object(2, "TREE", 50, 10, 2);
        tree.category = crate::CATEGORY_STRUCTURE;
        let mut second_rock = find_world_object(3, "ROCK", 90, 10, 2);
        second_rock.category = crate::CATEGORY_STRUCTURE;
        let world = HostWorldContext::from_objects(vec![first_rock, tree, second_rock]);

        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                #strict 3
                func FalsyNil()   { return FindObjects(Find_ID(ROCK), nil, Find_Category(2)); }
                func FalsyInt()   { return FindObjects(Find_ID(ROCK), 0, Find_Category(2)); }
                func FalsyBool()  { return FindObjects(Find_ID(ROCK), false, Find_Category(2)); }
                func TruthyAny()  { return FindObjects(Find_ID(ROCK), 1, Find_Category(2)); }
                func BadFind()    { return FindObject2(Find_ID(ROCK), 1); }
                func BadCount()   { return ObjectCount2(Find_ID(ROCK), false); }
                func BadAfterNil(){ return FindObject2(Find_ID(ROCK), nil, 1); }
                func Extra() {
                    return FindObjects(
                        Find_ID(ROCK), Find_ID(ROCK), Find_ID(ROCK), Find_ID(ROCK),
                        Find_ID(ROCK), Find_ID(ROCK), Find_ID(ROCK), Find_ID(ROCK),
                        Find_ID(ROCK), Find_ID(ROCK), Find_Category(2));
                }
                "#,
            )
            .expect("criterion conversion probes compile");

        let call = |function: &str| {
            with_object_host_context_with_world(world.clone(), || {
                script.call(function, &[]).map_err(|error| match error {
                    clonk_script::ScriptError::Runtime(error) => error,
                    other => RuntimeError::new(other.to_string()),
                })
            })
            .0
        };
        let object_ids = |value: Value| {
            let Value::Array(values) = value else {
                panic!("FindObjects should return an array");
            };
            values.iter().map(object_id_from_value).collect::<Vec<_>>()
        };

        for function in ["FalsyNil", "FalsyInt", "FalsyBool"] {
            assert_eq!(
                object_ids(call(function).expect("falsy criterion scan succeeds")),
                vec![Some(ObjectId::new(1)), Some(ObjectId::new(3))],
                "{function}"
            );
        }
        assert_eq!(
            object_ids(call("TruthyAny").expect("Any criterion slot accepts int")),
            vec![Some(ObjectId::new(3))],
            "truthy non-arrays in FindObjects' Any slots are skipped, not terminators"
        );
        assert_eq!(
            object_ids(call("Extra").expect("extra criterion call succeeds")),
            vec![Some(ObjectId::new(1)), Some(ObjectId::new(3))],
            "the eleventh filtering criterion is evaluated but not passed to the native"
        );

        for (function, parameter) in [("BadFind", 2), ("BadCount", 2), ("BadAfterNil", 3)] {
            let error = call(function).expect_err("Array-typed native slot must reject scalar");
            assert!(
                error.message().contains(&format!("parameter {parameter}"))
                    && error.message().contains("expected \"array\""),
                "{function}: {}",
                error.message()
            );
        }
    }

    fn parsed_condition(entries: Vec<Value>) -> FindCondition {
        match FindCondition::parse(&Value::Array(entries)) {
            ParsedCriterion::Condition(condition) => condition,
            _ => panic!("expected a parsed condition"),
        }
    }

    #[test]
    fn find_condition_primitive_bounds_match_cpp_getbounds() {
        // GetBounds/UseShapes overrides (C4FindObject.h:93-94):
        // InRect → its rect, no shapes (C4FindObject.h:196);
        // AtPoint → 1x1 at the point, shapes (C4FindObject.h:203,211-212);
        // AtRect → its rect, shapes (C4FindObject.h:226-227);
        // OnLine → endpoint bounding box, shapes (C4FindObject.h:234-246);
        // Distance → enclosing square, NO shapes (C4FindObject.h:253,260-261);
        // all remaining criteria → no bounds (base default).
        assert_eq!(
            parsed_condition(vec![
                Value::Int(10),
                Value::Int(5),
                Value::Int(6),
                Value::Int(20),
                Value::Int(30),
            ])
            .bounds(),
            Some((DefinitionRect::new(5, 6, 20, 30), false))
        );
        assert_eq!(
            parsed_condition(vec![Value::Int(11), Value::Int(70), Value::Int(80)]).bounds(),
            Some((DefinitionRect::new(70, 80, 1, 1), true))
        );
        assert_eq!(
            parsed_condition(vec![
                Value::Int(12),
                Value::Int(5),
                Value::Int(6),
                Value::Int(20),
                Value::Int(30),
            ])
            .bounds(),
            Some((DefinitionRect::new(5, 6, 20, 30), true))
        );
        assert_eq!(
            parsed_condition(vec![
                Value::Int(13),
                Value::Int(90),
                Value::Int(10),
                Value::Int(20),
                Value::Int(45),
            ])
            .bounds(),
            Some((DefinitionRect::new(20, 10, 71, 36), true)),
            "OnLine: union of the two 1x1 endpoint rects (C4FindObject.h:234-237)"
        );
        assert_eq!(
            parsed_condition(vec![
                Value::Int(14),
                Value::Int(100),
                Value::Int(50),
                Value::Int(30),
            ])
            .bounds(),
            Some((DefinitionRect::new(70, 20, 61, 61), false)),
            "Distance: (x-r, y-r, 2r+1, 2r+1) (C4FindObject.h:253)"
        );
        assert_eq!(
            parsed_condition(vec![Value::Int(21), Value::Int(16)]).bounds(),
            None,
            "OCF has no bounds"
        );
        assert_eq!(
            parsed_condition(vec![
                Value::Int(1),
                Value::Array(vec![
                    Value::Int(10),
                    Value::Int(5),
                    Value::Int(6),
                    Value::Int(20),
                    Value::Int(30),
                ]),
            ])
            .bounds(),
            None,
            "Not never has bounds (no GetBounds override, C4FindObject.h:104-118)"
        );
    }

    #[test]
    fn find_condition_combinator_bounds_match_cpp_constructors() {
        let in_rect = |x, y, w, h| {
            Value::Array(vec![
                Value::Int(10),
                Value::Int(x),
                Value::Int(y),
                Value::Int(w),
                Value::Int(h),
            ])
        };
        let ocf = Value::Array(vec![Value::Int(21), Value::Int(16)]);
        let at_rect = Value::Array(vec![
            Value::Int(12),
            Value::Int(60),
            Value::Int(60),
            Value::Int(10),
            Value::Int(10),
        ]);

        // C4FindObjectAnd constructor (C4FindObject.cpp:411-434): intersect
        // the bounded children; boundless children are skipped.
        assert_eq!(
            parsed_condition(vec![
                Value::Int(2),
                in_rect(0, 0, 100, 100),
                ocf.clone(),
                in_rect(50, 40, 100, 100),
            ])
            .bounds(),
            Some((DefinitionRect::new(50, 40, 50, 60), false))
        );
        // A shapes child replaces any accumulated intersection and stops the
        // walk ("do not intersect an atpoint bound with an rect bound",
        // C4FindObject.cpp:417-425).
        assert_eq!(
            parsed_condition(vec![
                Value::Int(2),
                in_rect(0, 0, 100, 100),
                at_rect.clone()
            ])
            .bounds(),
            Some((DefinitionRect::new(60, 60, 10, 10), true))
        );

        // C4FindObjectOr constructor (C4FindObject.cpp:477-496): union of
        // the child bounds; any boundless or shapes child kills the bounds.
        assert_eq!(
            parsed_condition(vec![
                Value::Int(3),
                in_rect(0, 0, 20, 20),
                in_rect(80, 90, 20, 20),
            ])
            .bounds(),
            Some((DefinitionRect::new(0, 0, 100, 110), false))
        );
        assert_eq!(
            parsed_condition(vec![Value::Int(3), in_rect(0, 0, 20, 20), ocf]).bounds(),
            None,
            "boundless child → no Or bounds (C4FindObject.cpp:481)"
        );
        assert_eq!(
            parsed_condition(vec![Value::Int(3), in_rect(0, 0, 20, 20), at_rect]).bounds(),
            None,
            "shapes child → no Or bounds (C4FindObject.cpp:482-488)"
        );
    }

    /// A 150x120px world (3x3 sectors of 50px) so sector-walk order can
    /// diverge from master-list order.
    fn sectored_find_world(
        objects: Vec<HostWorldObject>,
        definitions: HashMap<DefinitionId, DefinitionMetadata>,
    ) -> HostWorldContext {
        let landscape = Landscape::new(150, vec![120; 150]).expect("landscape builds");
        HostWorldContext::with_landscape(
            objects,
            Some(landscape),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            10,
            false,
        )
    }

    #[test]
    fn find_shape_conditions_use_live_object_shape() {
        // C4FindObjectAtPoint/AtRect/OnLine read pObj->Shape directly,
        // while FnSetShape immediately calls UpdatePos to migrate the
        // ObjectShapes sector links (C4FindObject.cpp:550-565;
        // C4Script.cpp:5183-5194). Start in sector 1, force the old cache,
        // then move a foreign target's shape into sector 2 in the same VM
        // call. The post-write hits therefore require both the live
        // predicate geometry and synchronous sector migration.
        let target_script = r#"
            #strict 2
            public func ExpandForFind()
            {
                SetShape(60, -5, 20, 10);
                return true;
            }
        "#;
        let caller_script = r#"
            #strict 2
            func ShapeCounts()
            {
                return [
                    ObjectCount2([11, 140, 25]),
                    ObjectCount2([11, 145, 25]),
                    ObjectCount2([12, 144, 25, 1, 1]),
                    ObjectCount2([12, 145, 25, 1, 1]),
                    ObjectCount2([13, 130, 25, 140, 25]),
                    ObjectCount2([13, 130, 30, 150, 30]),
                    ObjectCount2([11, 75, 25]),
                    ObjectCount2([12, 73, 23, 4, 4]),
                    ObjectCount2([13, 70, 25, 80, 25])
                ];
            }
            public func Probe(object target)
            {
                var before = ShapeCounts();
                var changed = SetShape(60, -5, 10, 10, target);
                return [before, changed, ShapeCounts()];
            }
            public func VerifyPersistedShape() { return ShapeCounts(); }
            public func ProbeFuncMutation()
            {
                return ObjectCount2([2, [60, "ExpandForFind"], [11, 147, 25]]);
            }
        "#;

        let mut engine = crate::Engine::with_seed(0);
        engine
            .set_landscape(Landscape::new(200, vec![120; 200]).expect("sectored landscape builds"));
        let mut target_definition = crate::Definition::from_script("TARG", "Target", target_script)
            .expect("target definition compiles");
        target_definition.set_shape_rect(Some(DefinitionRect::new(-2, -2, 4, 4)));
        engine
            .register_definition(target_definition)
            .expect("target definition registers");
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller definition compiles"),
            )
            .expect("caller definition registers");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG").with_position(Vector2::new(75, 25)))
            .expect("target spawns");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(10, 100)))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        let before = Value::Array(vec![
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(0),
        ]);
        let after = Value::Array(vec![
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
        ]);
        assert_eq!(
            engine
                .call_object_function(caller_index, "Probe", vec![object_reference_value(target)],)
                .expect("same-call shape probe runs"),
            Value::Array(vec![before, Value::Bool(true), after.clone()])
        );
        assert_eq!(
            engine.object_current_shape_rect(target),
            Some(DefinitionRect::new(60, -5, 10, 10))
        );

        let caller_index = engine.find_object_index(caller).expect("caller remains");
        assert_eq!(
            engine
                .call_object_function(caller_index, "VerifyPersistedShape", Vec::new())
                .expect("persisted shape probe runs"),
            after,
            "the next callback's host sectors and predicates retain the live shape"
        );
        assert_eq!(
            engine
                .call_object_function(caller_index, "ProbeFuncMutation", Vec::new())
                .expect("Find_Func shape mutation probe runs"),
            Value::Int(1),
            "a later sibling criterion observes Find_Func's live SetShape write"
        );

        // This discriminator is true under C4Rect's two integer edge probes
        // but false under the former pixel-sampling approximation.
        assert!(rect_intersects_line_cpp(
            DefinitionRect::new(0, 0, 1, 1),
            -5,
            -5,
            2,
            1,
        ));
    }

    #[test]
    fn find_objects2_bounded_criteria_walk_sectors_not_the_master_list() {
        // C4FindObject::FindMany(Objs, Sct) (C4FindObject.cpp:310-355):
        // criteria with bounds walk the C4LArea sector lists — result order
        // is sector-major (row-major sectors, master-relative within each
        // sector), NOT master order. Boundless criteria keep the master
        // walk (C4FindObject.cpp:316-317).
        let world = sectored_find_world(
            vec![
                find_world_object(1, "ROCK", 80, 10, 1), // sector (1,0)
                find_world_object(2, "ROCK", 10, 10, 1), // sector (0,0)
                find_world_object(3, "ROCK", 20, 10, 1), // sector (0,0)
            ],
            HashMap::new(),
        );
        // [C4FO_InRect(10), 0, 0, 150, 40] — bounded, no shapes
        let bounded = vec![Value::Array(vec![
            Value::Int(10),
            Value::Int(0),
            Value::Int(0),
            Value::Int(150),
            Value::Int(40),
        ])];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_objects2(&bounded));
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![
                Some(ObjectId::new(2)),
                Some(ObjectId::new(3)),
                Some(ObjectId::new(1)),
            ],
            "sector (0,0) list first, then sector (1,0)"
        );
        // FindObject = first in the same walk (C4FindObject.cpp:296-306)
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_object2(&bounded));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(2))
        );

        // [C4FO_ID(20), ROCK] — no bounds → master-list order
        let boundless = vec![Value::Array(vec![
            Value::Int(20),
            Value::String("ROCK".into()),
        ])];
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&boundless));
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![
                Some(ObjectId::new(1)),
                Some(ObjectId::new(2)),
                Some(ObjectId::new(3)),
            ],
            "boundless criteria keep the master-list walk"
        );
    }

    #[test]
    fn attached_host_sector_snapshot_preserves_physical_lists_after_rank_refresh() {
        let objects = || {
            vec![
                find_world_object(1, "ROCK", 10, 10, 1),
                find_world_object(2, "ROCK", 20, 10, 1),
                find_world_object(3, "ROCK", 30, 10, 1),
            ]
        };
        let bounded = vec![Value::Array(vec![
            Value::Int(10),
            Value::Int(0),
            Value::Int(0),
            Value::Int(50),
            Value::Int(50),
        ])];

        let snapshot_objects = objects();
        let snapshot_landscape =
            Landscape::new(150, vec![120; 150]).expect("snapshot landscape builds");
        let definitions = HashMap::new();
        let (width, height) = crate::compat::landscape_extent(&snapshot_landscape);
        let mut sectors = build_host_sector_map(
            [1_usize, 0, 2]
                .into_iter()
                .map(|index| &snapshot_objects[index]),
            &definitions,
            width,
            height,
        );
        // SortByCategory refreshes only the rank oracle. Existing links in
        // each C4Sector::Objects list deliberately retain their old order.
        sectors.set_master_order([ObjectId::new(3), ObjectId::new(2), ObjectId::new(1)]);
        let world = sectored_find_world(objects(), HashMap::new())
            .with_master_order([ObjectId::new(3), ObjectId::new(2), ObjectId::new(1)])
            .with_sector_map(Some(sectors));
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&bounded));
        let Value::Array(values) = result.expect("bounded sector query succeeds") else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            [
                Some(ObjectId::new(2)),
                Some(ObjectId::new(1)),
                Some(ObjectId::new(3)),
            ],
            "the attached physical sector list survives a rank-only refresh"
        );

        let fallback = sectored_find_world(objects(), HashMap::new()).with_master_order([
            ObjectId::new(3),
            ObjectId::new(2),
            ObjectId::new(1),
        ]);
        let (result, _) = with_object_host_context_with_world(fallback, || find_objects2(&bounded));
        let Value::Array(values) = result.expect("fallback sector query succeeds") else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            [
                Some(ObjectId::new(3)),
                Some(ObjectId::new(2)),
                Some(ObjectId::new(1)),
            ],
            "an absent live snapshot rebuilds the sector lists in master-list order"
        );
    }

    /// `C4FindObject::Find`/`FindMany` walk `Objs.First -> Next`, the forward
    /// C4GameObjects master list (C4FindObject.cpp:188-216), never a
    /// storage/creation order. `C4ObjectList::Add(stMain)` inserts a new
    /// object ahead of the first same-category/same-id link
    /// (C4ObjectList.cpp:155-163), so equal-category siblings walk
    /// newest-first.
    ///
    /// Measured against the pinned oracle on EkeReloaded's Invasion: with
    /// Stippels 719..726 alive, `FindObjects(Find_ID(ST5B))` reports
    /// `726 725 724 723 722` in C++ while Rust reported `719 720 721 722 723`.
    #[test]
    fn find_objects2_boundless_walk_follows_master_list_not_storage_order() {
        let world = sectored_find_world(
            vec![
                find_world_object(1, "ROCK", 10, 10, 1),
                find_world_object(2, "ROCK", 20, 10, 1),
                find_world_object(3, "ROCK", 30, 10, 1),
            ],
            HashMap::new(),
        )
        .with_master_order([ObjectId::new(3), ObjectId::new(2), ObjectId::new(1)]);
        // [C4FO_ID(20), ROCK] — no bounds, so the walk is the master list.
        let boundless = vec![Value::Array(vec![
            Value::Int(20),
            Value::String("ROCK".into()),
        ])];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_objects2(&boundless));
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![
                Some(ObjectId::new(3)),
                Some(ObjectId::new(2)),
                Some(ObjectId::new(1)),
            ],
            "boundless criteria walk the master list, newest-first"
        );
        let (result, _) =
            with_object_host_context_with_world(world, || find_object2(&boundless));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(3)),
            "FindObject2 returns the first object of that same walk"
        );
    }

    /// `C4LSectors::Add` receives the live forward master list, so each
    /// `C4LSector::Objects` list is itself in master-list order
    /// (C4Sector.cpp:88-101; C4ObjectList.cpp:138-205). A sector-bounded
    /// query therefore reports newest-first *within* a sector.
    ///
    /// Measured against the pinned oracle: for one shared sector C++ reported
    /// `721 722 719 720` where Rust reported `721 719 722 720`.
    #[test]
    fn find_objects2_sector_lists_are_built_in_master_list_order() {
        let world = sectored_find_world(
            vec![
                find_world_object(1, "ROCK", 10, 10, 1),
                find_world_object(2, "ROCK", 20, 10, 1),
                find_world_object(3, "ROCK", 30, 10, 1),
            ],
            HashMap::new(),
        )
        .with_master_order([ObjectId::new(3), ObjectId::new(1), ObjectId::new(2)]);
        // [C4FO_InRect(10), 0, 0, 50, 50] — one sector holds all three.
        let bounded = vec![Value::Array(vec![
            Value::Int(10),
            Value::Int(0),
            Value::Int(0),
            Value::Int(50),
            Value::Int(50),
        ])];
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&bounded));
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![
                Some(ObjectId::new(3)),
                Some(ObjectId::new(1)),
                Some(ObjectId::new(2)),
            ],
            "the sector's own list carries master-list order"
        );
    }

    #[test]
    fn find_objects2_shape_criteria_walk_shape_lists_with_marker_dedup() {
        // UseShapes criteria walk the per-sector ObjectShapes lists
        // (C4FindObject.cpp:321-343): an object whose shape spans several
        // sectors sits in each of their lists but reports only at its FIRST
        // encounter (the Marker, C4FindObject.cpp:331-342).
        let definitions: HashMap<DefinitionId, DefinitionMetadata> = [
            (
                "SMLL".to_string(),
                DefinitionMetadata {
                    shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
                    ..DefinitionMetadata::default()
                },
            ),
            (
                "BIGG".to_string(),
                DefinitionMetadata {
                    shape: Some(DefinitionRect::new(-30, -5, 60, 10)),
                    ..DefinitionMetadata::default()
                },
            ),
        ]
        .into_iter()
        .collect();
        let world = sectored_find_world(
            vec![
                // rank 1: shape (88,8)-(92,12) → sector (1,0) only
                find_world_object(1, "SMLL", 90, 10, 1),
                // rank 2: shape (20,5)-(80,15) → sectors (0,0) AND (1,0)
                find_world_object(2, "BIGG", 50, 10, 1),
            ],
            definitions,
        );
        // [C4FO_AtRect(12), 0, 0, 120, 40] — bounded, shapes
        let args = vec![Value::Array(vec![
            Value::Int(12),
            Value::Int(0),
            Value::Int(0),
            Value::Int(120),
            Value::Int(40),
        ])];
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&args));
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![Some(ObjectId::new(2)), Some(ObjectId::new(1))],
            "sector (0,0) encounters BIGG first; its sector (1,0) repeat is deduped"
        );
    }

    #[test]
    fn find_object2_bounded_sort_walks_sector_lists_nested() {
        // C4FindObject::Find(Objs, Sct) with a sort (C4FindObject.cpp:
        // 283-307): each sector list yields its own best via the inner
        // Find(*pLst), and only the per-list winners meet the running best.
        // With C4SO_Random the uncached Compare draws value(obj1) then
        // value(obj2) per comparison (C4FindObject.cpp:834-842,914-917), so
        // the pairing — not just the draw count — is lockstep-relevant.
        let world = sectored_find_world(
            vec![
                find_world_object(1, "ROCK", 10, 10, 1), // sector (0,0): [1]
                find_world_object(2, "ROCK", 60, 10, 1), // sector (1,0): [2,3]
                find_world_object(3, "ROCK", 70, 10, 1),
            ],
            HashMap::new(),
        );
        let args = vec![
            Value::Array(vec![
                Value::Int(10),
                Value::Int(0),
                Value::Int(0),
                Value::Int(150),
                Value::Int(40),
            ]),
            Value::Array(vec![Value::Int(120)]), // C4SO_Random
        ];
        let rng = LcgRng::seed_from_u64(3);
        let mut mirror = rng.clone();
        let guard = enter_random_context(rng);
        let (result, _) = with_object_host_context_with_world(world, || find_object2(&args));
        let rng_after = guard.finish();
        // Nested walk: sector (0,0) seeds best=1 without a draw; sector
        // (1,0) draws Compare(3, 2) = (r1, r2) for its list winner, then
        // Compare(winner, 1) = (r3, r4) against the running best.
        let r1 = mirror.random(1 << 16);
        let r2 = mirror.random(1 << 16);
        let list_winner = if r2 - r1 > 0 { 3u64 } else { 2 };
        let r3 = mirror.random(1 << 16);
        let r4 = mirror.random(1 << 16);
        let expected = if r4 - r3 > 0 { list_winner } else { 1 };
        assert_eq!(rng_after, mirror, "exactly two Compare calls, four draws");
        assert_eq!(
            (expected, r2 - r1 > 0, r4 - r3 > 0),
            (2, false, true),
            "seed 3 discriminates the nested pairing from a flat running-best walk"
        );
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(expected))
        );
    }

    #[test]
    fn find_object2_bounded_sort_ties_keep_the_first_in_sector_order() {
        // Equal sort values compare to zero, so the incumbent stays
        // (C4FindObject.cpp:196-198) — the winner is the first match in
        // SECTOR-walk order, not master order.
        let world = sectored_find_world(
            vec![
                find_world_object(1, "ROCK", 80, 10, 1), // sector (1,0)
                find_world_object(2, "ROCK", 10, 10, 1), // sector (0,0)
                find_world_object(3, "ROCK", 20, 10, 1), // sector (0,0)
            ],
            HashMap::new(),
        );
        let args = vec![
            Value::Array(vec![
                Value::Int(10),
                Value::Int(0),
                Value::Int(0),
                Value::Int(150),
                Value::Int(40),
            ]),
            Value::Array(vec![Value::Int(140)]), // C4SO_Mass — all equal
        ];
        let (result, _) = with_object_host_context_with_world(world, || find_object2(&args));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(2)),
            "first match of the first sector list wins ties"
        );
    }

    #[test]
    fn find_object2_shape_sort_has_no_marker_so_spanning_shapes_compare_twice() {
        // C4FindObject::Find's UseShapes arm has NO marker
        // (C4FindObject.cpp:283-294, unlike FindMany:331-342): an object
        // whose shape spans two sectors sits in both lists and is compared
        // in both — its repeat costs Compare draws.
        let definitions: HashMap<DefinitionId, DefinitionMetadata> = [
            (
                "SMLL".to_string(),
                DefinitionMetadata {
                    shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
                    ..DefinitionMetadata::default()
                },
            ),
            (
                "BIGG".to_string(),
                DefinitionMetadata {
                    shape: Some(DefinitionRect::new(-30, -5, 60, 10)),
                    ..DefinitionMetadata::default()
                },
            ),
        ]
        .into_iter()
        .collect();
        let world = sectored_find_world(
            vec![
                // rank 1: sector (1,0) only
                find_world_object(1, "SMLL", 90, 10, 1),
                // rank 2: sectors (0,0) and (1,0)
                find_world_object(2, "BIGG", 50, 10, 1),
            ],
            definitions,
        );
        let args = vec![
            Value::Array(vec![
                Value::Int(12),
                Value::Int(0),
                Value::Int(0),
                Value::Int(120),
                Value::Int(40),
            ]),
            Value::Array(vec![Value::Int(120)]), // C4SO_Random
        ];
        let rng = LcgRng::seed_from_u64(7);
        let mut mirror = rng.clone();
        let guard = enter_random_context(rng);
        let (result, _) = with_object_host_context_with_world(world, || find_object2(&args));
        let rng_after = guard.finish();
        // Sector (0,0) list [2]: best=2, no draw. Sector (1,0) list [1,2]:
        // inner Compare(2, 1) draws (r1, r2); outer Compare(winner, 2)
        // draws (r3, r4) — 2 can even meet itself.
        let r1 = mirror.random(1 << 16);
        let r2 = mirror.random(1 << 16);
        let list_winner = if r2 - r1 > 0 { 2u64 } else { 1 };
        let r3 = mirror.random(1 << 16);
        let r4 = mirror.random(1 << 16);
        let expected = if r4 - r3 > 0 { list_winner } else { 2 };
        assert_eq!(
            rng_after, mirror,
            "four draws: the spanning shape is compared in BOTH sector lists"
        );
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(expected))
        );
    }

    #[test]
    fn find_criteria_prune_ensured_and_children_like_the_cpp_constructor() {
        // C4FindObjectAnd's constructor REMOVES ensured children
        // (C4FindObject.cpp:400-410) before Check ever runs: Find_Category(0)
        // is ensured (C4FindObject.cpp:587-590) yet its Check is always
        // false (:582-585) — pruning makes it act as always-true inside an
        // And (and CreateCriterionsFromPars' top-level And,
        // C4Script.cpp:2023-2026).
        let world = HostWorldContext::from_objects(vec![
            find_world_object(1, "ROCK", 10, 10, 1),
            find_world_object(2, "TREE", 50, 10, 1),
            find_world_object(3, "ROCK", 90, 10, 1),
        ]);
        let args = vec![
            Value::Array(vec![Value::Int(22), Value::Int(0)]), // ensured
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
        ];
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&args));
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![Some(ObjectId::new(1)), Some(ObjectId::new(3))],
            "the ensured Category(0) child must not veto the And"
        );
    }

    #[test]
    fn find_criteria_prune_impossible_or_children_for_the_bounds_decision() {
        // C4FindObjectOr's constructor removes impossible children
        // (C4FindObject.cpp:466-476) BEFORE summing bounds — an OCF(0)
        // child (impossible, C4FindObject.cpp:577-580) must not kill the
        // sibling rect's bounds, so the walk stays sector-ordered.
        let world = sectored_find_world(
            vec![
                find_world_object(1, "ROCK", 80, 10, 1), // sector (1,0)
                find_world_object(2, "ROCK", 10, 10, 1), // sector (0,0)
                find_world_object(3, "ROCK", 20, 10, 1), // sector (0,0)
            ],
            HashMap::new(),
        );
        let args = vec![Value::Array(vec![
            Value::Int(3), // C4FO_Or
            Value::Array(vec![
                Value::Int(10),
                Value::Int(0),
                Value::Int(0),
                Value::Int(150),
                Value::Int(40),
            ]),
            Value::Array(vec![Value::Int(21), Value::Int(0)]), // impossible
        ])];
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&args));
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![
                Some(ObjectId::new(2)),
                Some(ObjectId::new(3)),
                Some(ObjectId::new(1)),
            ],
            "the pruned Or keeps the InRect bounds → sector-walk order"
        );
    }

    #[test]
    fn object_count2_or_with_ensured_child_counts_the_full_list() {
        // C4FindObjectOr::IsEnsured (C4FindObject.cpp:514-520) + the Count
        // ensured shortcut (C4FindObject.cpp:233-234): an Or with an
        // ensured child counts every object without checking any.
        let world = HostWorldContext::from_objects(vec![
            find_world_object(1, "ROCK", 10, 10, 1),
            find_world_object(2, "TREE", 50, 10, 1),
            find_world_object(3, "ROCK", 90, 10, 1),
        ]);
        let args = vec![Value::Array(vec![
            Value::Int(3),
            Value::Array(vec![Value::Int(22), Value::Int(0)]), // ensured
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
        ])];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || object_count2(&args));
        assert_eq!(result.expect("ObjectCount2 succeeds"), Value::Int(3));

        // Same through the Func-criterion view path: the unknown-Func child
        // is impossible (C4FindObject.cpp:664-667), pruned from the Or —
        // the ensured Category(0) child remains and the shortcut fires.
        let args = vec![Value::Array(vec![
            Value::Int(3),
            Value::Array(vec![Value::Int(22), Value::Int(0)]),
            Value::Array(vec![Value::Int(60), Value::String("NoSuchFunc".into())]),
        ])];
        let (result, _) = with_object_host_context_with_world(world, || object_count2(&args));
        assert_eq!(result.expect("ObjectCount2 succeeds"), Value::Int(3));
    }

    #[test]
    fn legacy_find_object_rect_query_walks_the_master_list_not_sectors() {
        // C4Game::FindObject (C4Game.cpp:1334-1424) — the legacy
        // fixed-parameter FindObject — scans Objects.First, the MASTER
        // list, for EVERY query form; the rect arm returns the first
        // master-order object inside the rect (C4Game.cpp:1414-1416).
        // Sectors belong only to the criteria form (C4FindObject's
        // Find(Objs, Sct) arms).
        let world = sectored_find_world(
            vec![
                // ids clear of the harness caller (object 1, self-excluded
                // per C4Script.cpp:2125)
                find_world_object(11, "ROCK", 80, 10, 1), // sector (1,0)
                find_world_object(12, "ROCK", 10, 10, 1), // sector (0,0)
            ],
            HashMap::new(),
        )
        .with_master_order([ObjectId::new(12), ObjectId::new(11)]);
        let args = vec![
            Value::Nil,      // id
            Value::Int(0),   // x
            Value::Int(0),   // y
            Value::Int(150), // wdt
            Value::Int(40),  // hgt
        ];
        let (result, _) = with_object_host_context_with_world(world, || find_object(&args));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject succeeds")),
            Some(ObjectId::new(12)),
            "first MASTER-order match wins, not the sector-walk first"
        );
    }

    #[test]
    fn command_runtime_data_ranks_objects_in_cpp_master_list_order() {
        let mut inactive = find_world_object(13, "WOOD", 0, 0, 1);
        inactive.status = ObjectStatus::Inactive;
        let world = HostWorldContext::from_objects(vec![
            find_world_object(11, "WOOD", 3, 4, 1),
            find_world_object(12, "WOOD", -3, 4, 1),
            inactive,
        ])
        .with_master_order([ObjectId::new(12), ObjectId::new(11)]);

        let (result, _) = with_object_host_context_with_world(world, || {
            HOST_CONTEXT.with(|cell| {
                let borrow = cell.borrow();
                let context = borrow.as_ref().expect("host context installed");
                let (objects, _, _, _) = context.command_runtime_data(&HashMap::new(), None);
                assert_eq!(objects[&ObjectId::new(12)].master_list_order, 0);
                assert_eq!(objects[&ObjectId::new(11)].master_list_order, 1);
                assert_eq!(objects[&ObjectId::new(13)].status, ObjectStatus::Inactive);
            });
            Ok(())
        });
        result.expect("command runtime rank probe succeeds");
    }

    #[test]
    fn find_object2_condition_tree_matches_cpp() {
        // C4FindObject::CreateByValue (C4FindObject.cpp:37-162) +
        // CreateCriterionsFromPars (C4Script.cpp:1985-2034): criteria arrays
        // AND together; Not/Or nest; the first main-list match wins
        // (C4FindObject.cpp:188-194).
        let world = HostWorldContext::from_objects(vec![
            find_world_object(1, "ROCK", 10, 10, 1),
            find_world_object(2, "TREE", 50, 10, 2),
            find_world_object(3, "ROCK", 90, 10, 2),
        ]);
        // [C4FO_ID(20), "ROCK"] AND [C4FO_Owner(50), 2] → object 3
        let args = vec![
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
            Value::Array(vec![Value::Int(50), Value::Int(2)]),
        ];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_object2(&args));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(3))
        );

        // [C4FO_Not(1), [C4FO_ID, "ROCK"]] → first non-rock (object 2)
        let args = vec![Value::Array(vec![
            Value::Int(1),
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
        ])];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_object2(&args));
        assert_eq!(
            object_id_from_value(&result.expect("FindObject2 succeeds")),
            Some(ObjectId::new(2))
        );

        // [C4FO_Or(3), [ID TREE], [InRect around object 3]] → objects 2 and 3
        let args = vec![Value::Array(vec![
            Value::Int(3),
            Value::Array(vec![Value::Int(20), Value::String("TREE".into())]),
            Value::Array(vec![
                Value::Int(10),
                Value::Int(85),
                Value::Int(5),
                Value::Int(10),
                Value::Int(10),
            ]),
        ])];
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || object_count2(&args));
        assert_eq!(result.expect("ObjectCount2 succeeds"), Value::Int(2));

        // No valid criterions → script error (C4Script.cpp:2042-2043)
        let (result, _) =
            with_object_host_context_with_world(world, || find_object2(&[Value::Int(5)]));
        assert!(result.is_err());
    }

    #[test]
    fn find_objects2_sort_random_consumes_synced_draws_in_collection_order() {
        // C4SortObjectRandom::CompareGetValue draws the synced
        // Random(1 << 16) (C4FindObject.cpp:914-917) — once per object via
        // the PrepareCache pass in collection order
        // (C4FindObject.cpp:819-832), then a stable ascending sort.
        let world = HostWorldContext::from_objects(vec![
            find_world_object(1, "ROCK", 10, 10, 1),
            find_world_object(2, "ROCK", 50, 10, 1),
            find_world_object(3, "ROCK", 90, 10, 1),
        ]);
        let args = vec![
            Value::Array(vec![Value::Int(20), Value::String("ROCK".into())]),
            Value::Array(vec![Value::Int(120)]), // C4SO_Random
        ];
        let rng = LcgRng::seed_from_u64(99);
        let mut mirror = rng.clone();
        let guard = enter_random_context(rng);
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&args));
        let rng_after = guard.finish();
        let draws = [
            mirror.random(1 << 16),
            mirror.random(1 << 16),
            mirror.random(1 << 16),
        ];
        assert_eq!(rng_after, mirror, "exactly one draw per object, in order");
        // ascending by drawn value, stable
        let mut expected: Vec<(i32, u64)> = draws
            .iter()
            .zip([1u64, 2, 3])
            .map(|(&draw, id)| (draw, id))
            .collect();
        expected.sort_by_key(|&(draw, _)| draw);
        let Ok(Value::Array(values)) = result else {
            panic!("FindObjects returns array");
        };
        let ids: Vec<Option<ObjectId>> = values.iter().map(object_id_from_value).collect();
        let expected_ids: Vec<Option<ObjectId>> = expected
            .iter()
            .map(|&(_, id)| Some(ObjectId::new(id)))
            .collect();
        assert_eq!(ids, expected_ids);
    }

    #[test]
    fn find_objects2_sort_mass_and_reverse_match_cpp() {
        // C4SO_Mass sorts lightest first (C4FindObject.h:59, ascending by
        // CompareGetValue); C4SO_Reverse flips it (C4FindObject.cpp:856-869).
        let definitions: HashMap<DefinitionId, DefinitionMetadata> = [
            (
                "LGHT".to_string(),
                DefinitionMetadata {
                    mass: 10,
                    ..DefinitionMetadata::default()
                },
            ),
            (
                "HEVY".to_string(),
                DefinitionMetadata {
                    mass: 500,
                    ..DefinitionMetadata::default()
                },
            ),
        ]
        .into_iter()
        .collect();
        let world = HostWorldContext::with_landscape(
            vec![
                find_world_object(1, "HEVY", 10, 10, 1),
                find_world_object(2, "LGHT", 50, 10, 1),
            ],
            None,
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            10,
            false,
        );
        let all = Value::Array(vec![Value::Int(22), Value::Int(0xFFFF)]); // C4FO_Category any
        let args = vec![all.clone(), Value::Array(vec![Value::Int(140)])]; // C4SO_Mass
        let (result, _) =
            with_object_host_context_with_world(world.clone(), || find_objects2(&args));
        let Ok(Value::Array(values)) = result else {
            panic!("array result");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![Some(ObjectId::new(2)), Some(ObjectId::new(1))],
            "lightest first"
        );

        // [C4SO_Reverse(101), [C4SO_Mass]] → heaviest first
        let args = vec![
            all,
            Value::Array(vec![Value::Int(101), Value::Array(vec![Value::Int(140)])]),
        ];
        let (result, _) = with_object_host_context_with_world(world, || find_objects2(&args));
        let Ok(Value::Array(values)) = result else {
            panic!("array result");
        };
        assert_eq!(
            values.iter().map(object_id_from_value).collect::<Vec<_>>(),
            vec![Some(ObjectId::new(1)), Some(ObjectId::new(2))],
            "reverse: heaviest first"
        );
    }

    #[test]
    fn cast_particles_registers_cast_command_and_checks_def_registry() {
        // FnCastParticles (C4Script.cpp:4881-4903): args are
        // (name, amount, level, x, y, a0, a1, b0, b1, obj); a-values are
        // script ints /10; GetDef failure → false.
        let defs: std::collections::HashSet<String> = ["Mist".to_string()].into_iter().collect();
        let world = HostWorldContext::from_objects(vec![]).with_particle_defs(defs.clone());
        let args = [
            Value::String("Mist".into()),
            Value::Int(12),
            Value::Int(20),
            Value::Int(5),
            Value::Int(6),
            Value::Int(10),
            Value::Int(20),
            Value::Int(0x11223344),
            Value::Int(0x55667788),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || cast_particles(&args));
        assert_eq!(result.expect("CastParticles succeeds"), Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Cast {
                definition_id,
                amount,
                x,
                y,
                level,
                a0,
                b0,
                a1,
                b1,
                layer,
            } => {
                assert_eq!(definition_id, "Mist");
                assert_eq!(*amount, 12);
                assert_eq!(*level, 20);
                assert_eq!(x.to_bits(), 5.0f32.to_bits());
                assert_eq!(y.to_bits(), 6.0f32.to_bits());
                assert_eq!(a0.to_bits(), 1.0f32.to_bits());
                assert_eq!(a1.to_bits(), 2.0f32.to_bits());
                assert_eq!(*b0, 0x11223344);
                assert_eq!(*b1, 0x55667788);
                assert!(matches!(layer, ParticleLayer::Global));
            }
            other => panic!("unexpected particle command {other:?}"),
        }

        // Unknown def with a registry attached → false, no command
        // (C4Script.cpp:4893).
        let world = HostWorldContext::from_objects(vec![]).with_particle_defs(defs);
        let args = [
            Value::String("NoSuchDef".into()),
            Value::Int(1),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || cast_particles(&args));
        assert_eq!(result.expect("CastParticles succeeds"), Value::Bool(false));
        assert!(outcome.particles.is_empty());

        // No registry attached (legacy fixture context) → permissive.
        let args = [
            Value::String("Anything".into()),
            Value::Int(1),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
        ];
        let (result, outcome) = with_object_host_context(|| cast_particles(&args));
        assert_eq!(result.expect("CastParticles succeeds"), Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
    }

    #[test]
    fn cast_back_particles_targets_back_layer() {
        // FnCastBackParticles (C4Script.cpp:4905-4908) = FnCastAParticles
        // with fBack = true → the object's BackParticles list.
        let target_id = ObjectId::new(9);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            target_id,
            "Engine",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [
            Value::String("Exhaust".into()),
            Value::Int(3),
            Value::Int(10),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            object_reference_value(target_id),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || cast_back_particles(&args));
        assert_eq!(
            result.expect("CastBackParticles succeeds"),
            Value::Bool(true)
        );
        match &outcome.particles[0] {
            ParticleCommand::Cast { layer, .. } => {
                assert!(matches!(layer, ParticleLayer::ObjectBack(id) if *id == target_id));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn push_particles_registers_push_command_and_checks_def_registry() {
        // FnPushParticles (C4Script.cpp:4910-4923): nil name pushes all
        // particles; deltas are script ints /10; a named def that is not
        // loaded → false.
        let (result, outcome) = with_object_host_context(|| {
            push_particles(&[Value::Nil, Value::Int(15), Value::Int(-5)])
        });
        assert_eq!(result.expect("PushParticles succeeds"), Value::Bool(true));
        match &outcome.particles[0] {
            ParticleCommand::Push {
                definition_id,
                dxdir,
                dydir,
            } => {
                assert!(definition_id.is_none());
                assert_eq!(dxdir.to_bits(), 1.5f32.to_bits());
                assert_eq!(dydir.to_bits(), (-0.5f32).to_bits());
            }
            other => panic!("unexpected particle command {other:?}"),
        }

        let defs: std::collections::HashSet<String> = ["Spark".to_string()].into_iter().collect();
        let world = HostWorldContext::from_objects(vec![]).with_particle_defs(defs);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            push_particles(&[
                Value::String("Missing".into()),
                Value::Int(0),
                Value::Int(0),
            ])
        });
        assert_eq!(result.expect("PushParticles succeeds"), Value::Bool(false));
        assert!(outcome.particles.is_empty());
    }

    #[test]
    fn clear_particles_registers_command() {
        let (result, outcome) = with_object_host_context(|| clear_particles(&[]));
        let value = result.expect("ClearParticles succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Clear {
                definition_id,
                scope,
            } => {
                assert!(definition_id.is_none());
                assert!(matches!(scope, ParticleScope::Global));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn clear_particles_with_object_sets_scope() {
        let target_id = ObjectId::new(12);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            target_id,
            "Emitter",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )]);
        let args = [
            Value::String("Smoke".into()),
            object_reference_value(target_id),
        ];
        let (result, outcome) =
            with_object_host_context_with_world(world, || clear_particles(&args));
        let value = result.expect("ClearParticles succeeds");
        assert_eq!(value, Value::Bool(true));
        assert_eq!(outcome.particles.len(), 1);
        match &outcome.particles[0] {
            ParticleCommand::Clear {
                definition_id,
                scope,
            } => {
                assert_eq!(definition_id.as_deref(), Some("Smoke"));
                assert!(matches!(scope, ParticleScope::Object(id) if *id == target_id));
            }
            other => panic!("unexpected particle command {other:?}"),
        }
    }

    #[test]
    fn contained_returns_nil_when_object_has_no_container() {
        let (result, _) = with_object_host_context(|| contained(&[]));
        let value = result.expect("Contained without container succeeds");
        assert_eq!(value, Value::Nil);
    }

    #[test]
    fn contained_returns_container_reference() {
        let container_id = ObjectId::new(42);
        let object_id = ObjectId::new(7);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                container_id,
                "Chest",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                0,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                object_id,
                "Gem",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                0,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container_id),
            ),
        ]);
        let context = HostObjectContext {
            id: object_id,
            container: Some(container_id),
            ..idle_object_context()
        };
        let (result, _) = with_effect_context(Some(context), &[], world, 100, || contained(&[]));
        let value = result.expect("Contained with container succeeds");
        assert_eq!(value, object_reference_value(container_id));
    }

    #[test]
    fn contents_uses_raw_status_index_before_skipping_attached() {
        let container_id = ObjectId::new(100);
        let attached_id = ObjectId::new(101);
        let first_item_id = ObjectId::new(102);
        let deleted_id = ObjectId::new(103);
        let second_item_id = ObjectId::new(104);

        let container = HostWorldObject::new(
            container_id,
            "Crew",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![attached_id, deleted_id, first_item_id, second_item_id]);

        let attached = HostWorldObject::new(
            attached_id,
            "Banner",
            ObjectStatus::Normal,
            "Attach",
            None,
            None,
            Some("Attach".into()),
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let first_item = HostWorldObject::new(
            first_item_id,
            "FirstGem",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let deleted = HostWorldObject::new(
            deleted_id,
            "DeletedGem",
            ObjectStatus::Deleted,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let second_item = HostWorldObject::new(
            second_item_id,
            "SecondGem",
            ObjectStatus::Inactive,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![
            container,
            attached,
            first_item,
            deleted,
            second_item,
        ]);
        let context = HostObjectContext {
            id: container_id,
            action_name: "Walk".to_string(),
            direction: Direction::Right,
            ..idle_object_context()
        };

        let call_contents = |index, include_attached| {
            let args = [Value::Int(index), Value::Nil, Value::Bool(include_attached)];
            let (result, _) =
                with_effect_context(Some(context.clone()), &[], world.clone(), 200, || {
                    contents(&args)
                });
            result.expect("Contents succeeds")
        };

        // C++ first indexes [attached, first, second] after filtering only
        // Status==0, then advances if that selected raw slot is attached.
        assert_eq!(
            call_contents(0, false),
            object_reference_value(first_item_id)
        );
        assert_eq!(
            call_contents(1, false),
            object_reference_value(first_item_id)
        );
        assert_eq!(
            call_contents(2, false),
            object_reference_value(second_item_id)
        );
        assert_eq!(call_contents(3, false), Value::Nil);

        assert_eq!(call_contents(0, true), object_reference_value(attached_id));
        assert_eq!(
            call_contents(1, true),
            object_reference_value(first_item_id)
        );
        assert_eq!(
            call_contents(2, true),
            object_reference_value(second_item_id)
        );
        assert_eq!(call_contents(-1, false), Value::Nil);
    }

    #[test]
    fn contents_includes_attached_when_requested() {
        let container_id = ObjectId::new(110);
        let attached_id = ObjectId::new(111);

        let container = HostWorldObject::new(
            container_id,
            "Crew",
            ObjectStatus::Normal,
            "Walk",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![attached_id]);

        let attached = HostWorldObject::new(
            attached_id,
            "Banner",
            ObjectStatus::Normal,
            "Attach",
            None,
            None,
            Some("Attach".into()),
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, attached]);
        let context = HostObjectContext {
            id: container_id,
            action_name: "Walk".to_string(),
            direction: Direction::Right,
            ..idle_object_context()
        };

        let args = [Value::Nil, Value::Nil, Value::Bool(true)];
        let (result, _) = with_effect_context(Some(context), &[], world, 200, || contents(&args));
        let value = result.expect("Contents with attachments succeeds");
        assert_eq!(value, object_reference_value(attached_id));
    }

    #[test]
    fn contents_count_filters_by_definition() {
        let container_id = ObjectId::new(120);
        let gem_id = ObjectId::new(121);
        let hammer_id = ObjectId::new(122);

        let container = HostWorldObject::new(
            container_id,
            "CHST",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![gem_id, hammer_id]);

        let gem = HostWorldObject::new(
            gem_id,
            "GEM1",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let hammer = HostWorldObject::new(
            hammer_id,
            "HAMR",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, gem, hammer]);
        let context_all = HostObjectContext {
            id: container_id,
            ..idle_object_context()
        };
        let context_filtered = HostObjectContext {
            id: container_id,
            ..idle_object_context()
        };

        let (result, _) = with_effect_context(Some(context_all), &[], world.clone(), 300, || {
            contents_count(&[])
        });
        let value = result.expect("ContentsCount without filter succeeds");
        assert_eq!(value, Value::Int(2));

        let args = [Value::C4Id("GEM1".into())];
        let (filtered, _) = with_effect_context(Some(context_filtered), &[], world, 300, || {
            contents_count(&args)
        });
        let filtered_value = filtered.expect("ContentsCount with filter succeeds");
        assert_eq!(filtered_value, Value::Int(1));
    }

    #[test]
    fn find_contents_returns_matching_object() {
        let container_id = ObjectId::new(130);
        let gem_id = ObjectId::new(131);
        let hammer_id = ObjectId::new(132);

        let container = HostWorldObject::new(
            container_id,
            "CHST",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![hammer_id, gem_id]);

        let hammer = HostWorldObject::new(
            hammer_id,
            "HAMR",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let gem = HostWorldObject::new(
            gem_id,
            "GEM1",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, hammer, gem]);
        let context = HostObjectContext {
            id: container_id,
            ..idle_object_context()
        };

        let args = [Value::C4Id("GEM1".into())];
        let (result, _) =
            with_effect_context(Some(context), &[], world, 400, || find_contents(&args));
        let value = result.expect("FindContents succeeds");
        assert_eq!(value, object_reference_value(gem_id));
    }

    #[test]
    fn find_other_contents_returns_first_non_matching_object() {
        let container_id = ObjectId::new(140);
        let gem_id = ObjectId::new(141);
        let hammer_id = ObjectId::new(142);

        let container = HostWorldObject::new(
            container_id,
            "CHST",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_contents(vec![hammer_id, gem_id]);

        let gem = HostWorldObject::new(
            gem_id,
            "GEM1",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let hammer = HostWorldObject::new(
            hammer_id,
            "HAMR",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            0,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            Some(container_id),
        );

        let world = HostWorldContext::from_objects(vec![container, gem, hammer])
            .with_definition_metadata(Rc::new(HashMap::from([
                (DefinitionId::from("GEM1"), DefinitionMetadata::default()),
                (DefinitionId::from("HAMR"), DefinitionMetadata::default()),
            ])));
        let context = HostObjectContext {
            id: hammer_id,
            container: Some(container_id),
            energy: 0,
            ..idle_object_context()
        }
        .with_definition_id("HAMR");

        let args = [
            Value::C4Id("GEM1".into()),
            object_reference_value(container_id),
        ];
        let (result, _) =
            with_effect_context(Some(context.clone()), &[], world.clone(), 500, || {
                find_other_contents(&args)
            });
        let value = result.expect("FindOtherContents succeeds");
        assert_eq!(value, object_reference_value(hammer_id));

        let (same_call, _) = with_effect_context(Some(context), &[], world, 500, || {
            let changed = change_def(&[
                Value::C4Id("GEM1".into()),
                object_reference_value(hammer_id),
            ])?;
            let matching = find_contents(&args)?;
            let other = find_other_contents(&args)?;
            Ok::<_, RuntimeError>(Value::Array(vec![changed, matching, other]))
        });
        // ChangeDef's unsorted re-entry appends the hammer after the gem;
        // both now have GEM1, so FindOtherContents must find no child.
        assert_eq!(
            same_call.expect("same-call ChangeDef content searches succeed"),
            Value::Array(vec![
                Value::Bool(true),
                object_reference_value(gem_id),
                Value::Nil,
            ]),
            "both searches must see the live definition after contents re-insertion"
        );
    }

    #[test]
    fn remove_object_marks_destroy_flag() {
        let (result, outcome) = with_object_host_context(|| remove_object(&[]));
        assert_eq!(result.expect("RemoveObject succeeds"), Value::Bool(true));
        assert!(outcome.destroy_object);
    }

    #[test]
    fn assign_removal_ejects_contents_at_the_containers_same_call_position() {
        let container_script = r#"#strict
public func MoveThenRemove()
{
  SetPosition(345, 210);
  return RemoveObject(0, true);
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("BOX1", "Container", container_script)
                    .expect("container script compiles"),
            )
            .expect("container registers");
        engine
            .register_definition(
                crate::Definition::from_script("ITEM", "Item", "#strict\n")
                    .expect("item script compiles"),
            )
            .expect("item registers");

        let container = engine
            .spawn_object(SpawnConfig::new("BOX1").with_position(Vector2::new(120, 80)))
            .expect("container spawns");
        let child = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_position(Vector2::new(7, 11))
                    .with_container(container),
            )
            .expect("contained child spawns");
        let container_index = engine
            .find_object_index(container)
            .expect("container exists");

        assert_eq!(
            engine
                .call_object_function(container_index, "MoveThenRemove", Vec::new())
                .expect("same-call move and removal succeeds"),
            Value::Bool(true)
        );
        let child = engine
            .object_snapshot(child)
            .expect("ejected child survives");
        assert_eq!(child.container, None);
        assert_eq!(
            child.position,
            Vector2::new(345, 210),
            "AssignRemoval(true) must pass the container's live x/y to Exit"
        );
    }

    #[test]
    fn assign_removal_callback_menu_enumerates_status_zero_parent_link() {
        // AssignRemoval marks VICT Status=0 before recursively removing its
        // child, but VICT remains linked in GRAND until that recursion ends.
        // The child's Destruction callback opens the classic Activate menu;
        // its CalcDefValue call proves the raw iterator still saw VICT.
        let actor_script = r#"#strict 2
public func OpenDuringRemoval()
{
    SetCommand(this(), "Throw");
    ExecuteCommand();
    return true;
}
public func RemoveNested(object victim, object child)
{
    child->Arm(this());
    return RemoveObject(victim);
}
"#;
        let child_script = r#"#strict 2
local observer;
public func Arm(object target) { observer = target; return true; }
protected func Destruction()
{
    observer->OpenDuringRemoval();
    return true;
}
"#;
        let victim_script = r#"#strict 2
static calc_calls;
public func CalcDefValue(object base, int player)
{
    calc_calls++;
    return 10;
}
public func GetCalcCalls() { return calc_calls; }
"#;

        let mut engine = crate::Engine::with_seed(0);
        let mut grand = crate::Definition::from_script("GRND", "Grand", "#strict 2\n")
            .expect("grand container compiles");
        grand.set_category(crate::CATEGORY_STRUCTURE);
        engine.register_definition(grand).expect("grand registers");
        engine
            .register_definition(
                crate::Definition::from_script("ACTR", "Actor", actor_script)
                    .expect("actor compiles"),
            )
            .expect("actor registers");
        let mut victim = crate::Definition::from_script("VICT", "Victim", victim_script)
            .expect("victim compiles");
        victim.set_category(crate::CATEGORY_OBJECT);
        engine
            .register_definition(victim)
            .expect("victim registers");
        engine
            .register_definition(
                crate::Definition::from_script("CHLD", "Child", child_script)
                    .expect("child compiles"),
            )
            .expect("child registers");

        let grand = engine
            .spawn_object(SpawnConfig::new("GRND"))
            .expect("grand spawns");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_container(grand))
            .expect("actor spawns contained");
        let victim = engine
            .spawn_object(SpawnConfig::new("VICT").with_container(grand))
            .expect("victim spawns contained");
        let child = engine
            .spawn_object(SpawnConfig::new("CHLD").with_container(victim))
            .expect("child spawns nested");
        let witness = engine
            .spawn_object(SpawnConfig::new("VICT"))
            .expect("definition-static witness spawns");

        let actor_index = engine.find_object_index(actor).expect("actor remains");
        assert_eq!(
            engine
                .call_object_function(
                    actor_index,
                    "RemoveNested",
                    vec![
                        object_reference_value(victim),
                        object_reference_value(child)
                    ],
                )
                .expect("nested removal returns"),
            Value::Bool(true)
        );
        let witness_index = engine.find_object_index(witness).expect("witness remains");
        assert_eq!(
            engine
                .call_object_function(witness_index, "GetCalcCalls", Vec::new())
                .expect("definition-static counter reads"),
            Value::Int(1),
            "the live menu iterator must enumerate VICT while its Status-zero link remains"
        );
    }

    #[test]
    fn remove_object_eject_flag_controls_self_foreign_and_recursive_contents() {
        let container_script = r#"#strict
public func RemoveSelfWithEject() { return RemoveObject(0, 1); }
public func RemoveForeignWithEject(object target) { return RemoveObject(target, true); }
public func RemoveSelfWithFalse() { return RemoveObject(0, false); }
public func RemoveSelfWithoutEject() { return RemoveObject(); }
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("BOX1", "Container", container_script)
                    .expect("container script compiles"),
            )
            .expect("container registers");
        engine
            .register_definition(
                crate::Definition::from_script("ITEM", "Item", "#strict\n")
                    .expect("item script compiles"),
            )
            .expect("item registers");

        let self_position = Vector2::new(120, 80);
        let self_container = engine
            .spawn_object(SpawnConfig::new("BOX1").with_position(self_position))
            .expect("self container spawns");
        let self_child = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_position(Vector2::new(7, 11))
                    .with_container(self_container),
            )
            .expect("self child spawns");
        let self_index = engine
            .find_object_index(self_container)
            .expect("self container exists");

        assert_eq!(
            engine
                .call_object_function(self_index, "RemoveSelfWithEject", Vec::new())
                .expect("RemoveObject(0, 1) succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .object_snapshot(self_container)
                .expect("removed container remains delayed")
                .status,
            ObjectStatus::Deleted
        );
        let self_child = engine
            .object_snapshot(self_child)
            .expect("ejected child survives");
        assert_eq!(self_child.status, ObjectStatus::Normal);
        assert_eq!(self_child.container, None);
        assert_eq!(self_child.position, self_position);

        let caller = engine
            .spawn_object(SpawnConfig::new("BOX1"))
            .expect("foreign remover spawns");
        let foreign_position = Vector2::new(300, 160);
        let foreign_container = engine
            .spawn_object(SpawnConfig::new("BOX1").with_position(foreign_position))
            .expect("foreign container spawns");
        let foreign_child = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_position(Vector2::new(19, 23))
                    .with_container(foreign_container),
            )
            .expect("foreign child spawns");
        let caller_index = engine.find_object_index(caller).expect("remover exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "RemoveForeignWithEject",
                    vec![object_reference_value(foreign_container)],
                )
                .expect("foreign RemoveObject(target, true) succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .object_snapshot(foreign_container)
                .expect("foreign container remains delayed")
                .status,
            ObjectStatus::Deleted
        );
        let foreign_child = engine
            .object_snapshot(foreign_child)
            .expect("foreign child survives");
        assert_eq!(foreign_child.status, ObjectStatus::Normal);
        assert_eq!(foreign_child.container, None);
        assert_eq!(foreign_child.position, foreign_position);

        for function in ["RemoveSelfWithFalse", "RemoveSelfWithoutEject"] {
            let recursive_container = engine
                .spawn_object(SpawnConfig::new("BOX1"))
                .expect("recursive container spawns");
            let recursive_child = engine
                .spawn_object(SpawnConfig::new("ITEM").with_container(recursive_container))
                .expect("recursive child spawns");
            let recursive_grandchild = engine
                .spawn_object(SpawnConfig::new("ITEM").with_container(recursive_child))
                .expect("recursive grandchild spawns");
            let recursive_index = engine
                .find_object_index(recursive_container)
                .expect("recursive container exists");

            assert_eq!(
                engine
                    .call_object_function(recursive_index, function, Vec::new())
                    .expect("non-ejecting RemoveObject succeeds"),
                Value::Bool(true)
            );
            for removed in [recursive_container, recursive_child, recursive_grandchild] {
                assert_eq!(
                    engine
                        .object_snapshot(removed)
                        .expect("recursive removal remains delayed")
                        .status,
                    ObjectStatus::Deleted
                );
            }
        }
    }

    #[test]
    fn find_object_returns_first_matching_definition() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(1),
                "FLAG",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(10, 5),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(2),
                "ROCK",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(50, 5),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);

        let args = [Value::C4Id("FLAG".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        let value = result.expect("FindObject succeeds");
        assert_eq!(value, object_reference_value(ObjectId::new(1)));
    }

    #[test]
    fn find_object_has_no_owner_parameter_like_cpp() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(10),
                "DUMY",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                1,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(11),
                "DUMY",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                2,
                100,
                crate::FULL_CON,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        // FnFindObject has NO owner parameter — C++ always searches with
        // ANY_OWNER (C4Script.cpp:2133); only FindObjectOwner filters.
        let args = [
            Value::C4Id("DUMY".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Int(2),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        let value = result.expect("FindObject succeeds");
        assert_eq!(
            value,
            object_reference_value(ObjectId::new(10)),
            "the trailing int is beyond pFindNext and ignored; owner never filters"
        );
    }

    #[test]
    fn find_object_closest_mode_orders_by_distance() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(20),
                "DUMY",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(2, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(21),
                "DUMY",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(6, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            Value::C4Id("DUMY".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(-1),
        ];
        let (first_result, _) =
            with_effect_context(None, &[], world.clone(), 1, || find_object(&args));
        let first_value = first_result.expect("FindObject closest succeeds");
        assert_eq!(first_value, object_reference_value(ObjectId::new(20)));

        let mut find_next = ValueMap::new();
        find_next.insert("id".into(), Value::Int(20));
        let args_with_next = [
            Value::C4Id("DUMY".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(-1),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            // pFindNext is FindObject's 10th parameter (C4Script.cpp:2113).
            Value::Proplist(find_next),
        ];
        let (second_result, _) =
            with_effect_context(None, &[], world, 1, || find_object(&args_with_next));
        let second_value = second_result.expect("FindObject closest with next succeeds");
        assert_eq!(second_value, object_reference_value(ObjectId::new(21)));
    }

    #[test]
    fn find_object_closest_ties_keep_forward_master_order() {
        // C4Game::FindObject evaluates closest candidates while walking
        // Game.Objects.First -> Next and replaces the best only for a
        // strictly smaller distance (C4Game.cpp:1367-1424). Storage order
        // must not decide an equal-distance tie.
        let first_in_storage = HostWorldObject::new(
            ObjectId::new(30),
            "DUMY",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::new(-2, 0),
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );
        let first_in_master = HostWorldObject::new(
            ObjectId::new(31),
            "DUMY",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::new(2, 0),
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        );
        let world = HostWorldContext::from_objects([first_in_storage, first_in_master])
            .with_master_order([ObjectId::new(31), ObjectId::new(30)]);
        let args = [
            Value::C4Id("DUMY".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(-1),
        ];

        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));

        assert_eq!(
            result.expect("FindObject closest succeeds"),
            object_reference_value(ObjectId::new(31))
        );
    }

    #[test]
    fn find_object_respects_ocf_filter() {
        let matching_id = ObjectId::new(51);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                matching_id,
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )
            .with_ocf(ocf::AVAILABLE | ocf::ALIVE),
            HostWorldObject::new(
                ObjectId::new(52),
                "Dummy",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Int(ocf::AVAILABLE as i32),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        let value = result.expect("FindObject succeeds");
        assert_eq!(value, object_reference_value(matching_id));
    }

    #[test]
    fn find_object_point_uses_sector_shape_candidates() {
        let id = ObjectId::new(61);
        let mut definitions = HashMap::new();
        definitions.insert(
            "WIDE".to_string(),
            DefinitionMetadata {
                shape: Some(DefinitionRect::new(-10, -5, 20, 10)),
                ..DefinitionMetadata::default()
            },
        );
        let world = HostWorldContext::with_landscape(
            vec![HostWorldObject::new(
                id,
                "WIDE",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(40, 10),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )],
            Some(Landscape::flat(120, 120)),
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );

        let args = [
            Value::C4Id("WIDE".into()),
            Value::Int(31),
            Value::Int(10),
            Value::Int(0),
            Value::Int(0),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_object(&args));
        assert_eq!(
            result.expect("FindObject succeeds"),
            object_reference_value(id)
        );
    }

    #[test]
    fn find_objects_sector_range_uses_cpp_sector_enumeration_order() {
        // C4FindObject::FindMany with bounds and no sort pushes results in
        // AREA-ENUMERATION order — sector by sector (C4FindObject.cpp:
        // 344-353 via C4LArea::Next, C4Sector.cpp:264-277), NOT master-list
        // order. `first` ranks earlier but sits in sector 1 (x=70); `second`
        // sits in sector 0 (x=10) and is therefore encountered first.
        let first = ObjectId::new(71);
        let second = ObjectId::new(72);
        let world = HostWorldContext::with_landscape(
            vec![
                HostWorldObject::new(
                    first,
                    "DUMY",
                    ObjectStatus::Normal,
                    "Idle",
                    None,
                    None,
                    None,
                    OWNER_NONE,
                    100,
                    crate::FULL_CON,
                    Vector2::new(70, 10),
                    Vector2::ZERO,
                    Vec::new(),
                    0,
                    0,
                    None,
                ),
                HostWorldObject::new(
                    second,
                    "DUMY",
                    ObjectStatus::Normal,
                    "Idle",
                    None,
                    None,
                    None,
                    OWNER_NONE,
                    100,
                    crate::FULL_CON,
                    Vector2::new(10, 10),
                    Vector2::ZERO,
                    Vec::new(),
                    0,
                    0,
                    None,
                ),
            ],
            Some(Landscape::flat(120, 120)),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            1,
            false,
        );
        let args = [
            Value::C4Id("DUMY".into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(120),
            Value::Int(20),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_objects(&args));
        let value = result.expect("FindObjects succeeds");
        match value {
            Value::Array(entries) => {
                assert_eq!(
                    entries,
                    vec![
                        object_reference_value(second),
                        object_reference_value(first)
                    ]
                );
            }
            other => panic!("expected array, got {other:?}"),
        }
    }

    #[test]
    fn get_ocf_returns_object_mask() {
        // FnGetOCF returns pObj->OCF verbatim (C4Script.cpp:1354-1358);
        // the seeded mask mirrors a real SetOCF result, which always
        // carries OCF_Normal (C4Object.cpp:547-548).
        let ocf_mask = ocf::NORMAL | ocf::NOT_CONTAINED | ocf::AVAILABLE | ocf::ALIVE;
        let object_id = ObjectId::new(1);
        let world = HostWorldContext::from_objects(vec![HostWorldObject::new(
            object_id,
            "Dummy",
            ObjectStatus::Normal,
            "Idle",
            None,
            None,
            None,
            OWNER_NONE,
            100,
            crate::FULL_CON,
            Vector2::ZERO,
            Vector2::ZERO,
            Vec::new(),
            0,
            0,
            None,
        )
        .with_ocf(ocf_mask)]);

        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            100,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_alive(true)
        .with_base_graphics(None)
        .with_ocf(ocf_mask);

        let (result, _) = with_effect_context(Some(object_context), &[], world, 2, || get_ocf(&[]));
        let value = result.expect("GetOCF succeeds");
        let Value::Int(raw) = value else {
            panic!("expected integer mask, got {value:?}");
        };
        let mask = raw as u32;
        assert_eq!(
            mask, ocf_mask,
            "the cached mask comes back verbatim (C4Script.cpp:1357)"
        );
    }

    #[test]
    fn set_graphics_records_overlay_update() {
        let object_id = ObjectId::new(42);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_graphics_overlays(Vec::new())
        .with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_graphics(&[
                    Value::String("Default".into()),
                    Value::Nil,
                    Value::C4Id("CLNK".into()),
                    Value::Int(1),
                    Value::Int(GraphicsOverlayMode::Action as i32),
                    Value::String("Walk".into()),
                ])
            },
        );

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("object update expected");
        let overlays = update
            .graphics_overlays
            .expect("graphics overlay update expected");
        assert_eq!(overlays.len(), 1);
        let overlay = &overlays[0];
        assert_eq!(overlay.id, 1);
        assert_eq!(overlay.mode, GraphicsOverlayMode::Action);
        assert_eq!(overlay.definition.as_deref(), Some("CLNK"));
        assert_eq!(overlay.action.as_deref(), Some("Walk"));
    }

    #[test]
    fn set_graphics_converts_falsy_action_to_nil() {
        // A pre-strict-nil engine call Set0()s every falsy parameter before
        // converting it (C4AulExec.cpp:1364-1375), so FnSetGraphics' C4String*
        // action accepts integer zero as null (C4Script.cpp:4372). Hazard's
        // Sentry Gun relies on this exact optional slot
        // (Sentry Gun.c4d/Script.c:51).
        let (result, _) = with_object_host_context(|| {
            set_graphics(&[
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(1),
                Value::Int(GraphicsOverlayMode::Action as i32),
                Value::Int(0),
            ])
        });

        assert_eq!(
            result.expect("zero action converts to nil"),
            Value::Bool(false)
        );
    }

    #[test]
    fn set_graphics_on_an_existing_overlay_keeps_transform_and_modulation() {
        // C4GraphicsOverlay::Set reassigns mode/graphics/action/blit mode and
        // resets only iPhase; the comment "// (keep transform)" is explicit and
        // dwClrModulation is never touched (src/C4DefGraphics.cpp:682-693).
        // Rebuilding the overlay from scratch would silently drop a
        // SetObjDrawTransform applied before a graphics refresh.
        let object_id = ObjectId::new(9);
        let mut existing = ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Action)
            .with_definition(Some("Clonk".into()))
            .with_transform(Some(DrawTransform::from_components(2.0, 3.0, 4.0, 5.0)));
        existing.color_modulation = 0x0012_3456;
        existing.phase = 7;

        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_graphics_overlays(vec![existing])
        .with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_graphics(&[
                    Value::Nil,
                    Value::Nil,
                    Value::C4Id("Clonk".into()),
                    Value::Int(1),
                    Value::Int(GraphicsOverlayMode::IngamePicture as i32),
                ])
            },
        );

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let overlays = outcome
            .object_update
            .expect("object update expected")
            .graphics_overlays
            .expect("graphics overlay update expected");
        let overlay = overlays
            .iter()
            .find(|overlay| overlay.id == 1)
            .expect("overlay 1 survives the re-set");
        assert_eq!(overlay.mode, GraphicsOverlayMode::IngamePicture);
        assert_eq!(
            overlay.transform,
            Some(DrawTransform::from_components(2.0, 3.0, 4.0, 5.0)),
            "// (keep transform)"
        );
        assert_eq!(overlay.color_modulation, 0x0012_3456);
        assert_eq!(overlay.phase, 0, "Set resets iPhase");
    }

    #[test]
    fn set_graphics_returns_true_when_the_overlay_is_unchanged() {
        // FnSetGraphics returns true for every valid overlay it sets --
        // "// Okay, valid overlay set! return true;" -- and returns false only
        // when IsValid rejects the result (src/C4Script.cpp:4596-4603). Rust
        // forwarded set_graphics_overlay's "did anything change" bool, so
        // re-setting an identical overlay reported failure. Knights' WearShield
        // guards on this: `if(!SetGraphics(..., GFXOV_MODE_ExtraGraphics))
        // return();` (content/Knights.c4d/Crew.c4d/Knight.c4d/Script.c:1214).
        let object_id = ObjectId::new(11);
        let existing = ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Action)
            .with_definition(Some("Clonk".into()))
            .with_action(Some("Pointer".into()));

        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_graphics_overlays(vec![existing])
        .with_base_graphics(None);

        // Two identical calls in one scope: the second changes nothing.
        let call = || {
            set_graphics(&[
                Value::Nil,
                Value::Nil,
                Value::C4Id("Clonk".into()),
                Value::Int(1),
                Value::Int(GraphicsOverlayMode::Action as i32),
                Value::String("Pointer".into()),
            ])
        };
        let (results, _outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || -> Result<Vec<Value>, RuntimeError> { Ok(vec![call()?, call()?]) },
        );

        let results = results.expect("SetGraphics succeeds");
        assert_eq!(
            results,
            vec![Value::Bool(true), Value::Bool(true)],
            "re-setting an identical overlay is still a valid overlay set"
        );
    }

    #[test]
    fn set_graphics_removes_overlay_when_definition_missing() {
        let object_id = ObjectId::new(7);
        let overlay = ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Action)
            .with_definition(Some("Clonk".into()));
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_graphics_overlays(vec![overlay])
        .with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_graphics(&[
                    Value::String("Default".into()),
                    Value::Nil,
                    Value::Nil,
                    Value::Int(1),
                    Value::Int(GraphicsOverlayMode::Action as i32),
                ])
            },
        );

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("object update expected");
        let overlays = update
            .graphics_overlays
            .expect("graphics overlay update expected");
        assert!(overlays.is_empty());
    }

    #[test]
    fn set_graphics_updates_base_graphics() {
        let object_id = ObjectId::new(11);
        let definitions = {
            let mut map = HashMap::new();
            map.insert("CLON".to_string(), DefinitionMetadata::default());
            map.insert("BRIK".to_string(), DefinitionMetadata::default());
            map
        };
        let world = HostWorldContext::with_landscape(
            vec![HostWorldObject::new(
                object_id,
                "CLON",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )],
            None,
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            100,
            false,
        );

        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        );

        let (result, outcome) = with_effect_context(
            Some(object_context.with_base_graphics(None)),
            &[],
            world,
            100,
            || {
                set_graphics(&[
                    Value::String("Alt".into()),
                    Value::Nil,
                    Value::C4Id("BRIK".into()),
                    Value::Int(0),
                ])
            },
        );

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("object update expected");
        let base = update
            .base_graphics
            .expect("base graphics update expected")
            .expect("base graphics set");
        assert_eq!(base.definition, "BRIK");
        assert_eq!(base.graphics_name.as_deref(), Some("Alt"));
        assert_eq!(base.blit_mode, 0);
    }

    #[test]
    fn set_graphics_clears_base_graphics_when_nil() {
        let object_id = ObjectId::new(12);
        let definitions = {
            let mut map = HashMap::new();
            map.insert("CLON".to_string(), DefinitionMetadata::default());
            map
        };
        let world = HostWorldContext::with_landscape(
            vec![HostWorldObject::new(
                object_id,
                "CLON",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::ZERO,
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            )],
            None,
            definitions,
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            100,
            false,
        );

        let base = ObjectBaseGraphics {
            definition: "CLON".to_string(),
            graphics_name: Some("Alt".into()),
            blit_mode: 0,
        };

        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_base_graphics(Some(base));

        let (result, outcome) = with_effect_context(Some(object_context), &[], world, 100, || {
            set_graphics(&[Value::Nil, Value::Nil, Value::Nil, Value::Int(0)])
        });

        assert_eq!(result.expect("SetGraphics succeeds"), Value::Bool(true));
        let update = outcome.object_update.expect("object update expected");
        let base = update.base_graphics.expect("base graphics update expected");
        assert!(base.is_none());
    }

    #[test]
    fn set_obj_draw_transform_updates_object_transform() {
        let object_id = ObjectId::new(1);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        );

        let object_context = object_context.with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_obj_draw_transform(&[
                    Value::Int(866),
                    Value::Int(-500),
                    Value::Int(0),
                    Value::Int(500),
                    Value::Int(866),
                    Value::Int(0),
                ])
            },
        );

        assert_eq!(
            result.expect("SetObjDrawTransform succeeds"),
            Value::Bool(true)
        );
        let update = outcome.object_update.expect("object update expected");
        let transform = update
            .draw_transform
            .expect("transform update expected")
            .expect("transform set");
        assert_eq!(
            transform.matrix(),
            [
                866.0 / 1000.0,
                -0.5,
                0.0,
                0.5,
                866.0 / 1000.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ]
        );
    }

    #[test]
    fn set_obj_draw_transform_default_matrix_resets_base() {
        let object_id = ObjectId::new(2);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0,
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            Some(DrawTransform::from_components(2.0, 3.0, 4.0, 5.0)),
            None,
        );

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || set_obj_draw_transform(&[]),
        );

        assert_eq!(
            result.expect("SetObjDrawTransform succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            outcome
                .object_update
                .expect("object update expected")
                .draw_transform,
            Some(None)
        );
    }

    #[test]
    fn set_obj_draw_transform_updates_overlay_transform() {
        let object_id = ObjectId::new(5);
        let overlay = ObjectGraphicsOverlay::new(-2, GraphicsOverlayMode::Base);
        let zero_overlay = ObjectGraphicsOverlay::new(-3, GraphicsOverlayMode::Base);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0, // action_phase
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        )
        .with_graphics_overlays(vec![overlay, zero_overlay])
        .with_base_graphics(None);

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                let rotated = set_obj_draw_transform(&[
                    Value::Int(866),
                    Value::Int(-500),
                    Value::Int(125),
                    Value::Int(500),
                    Value::Int(866),
                    Value::Int(-250),
                    Value::Proplist({
                        let mut map = ValueMap::new();
                        map.insert("id".into(), Value::Int(object_id.as_u64() as i32));
                        map
                    }),
                    Value::Int(-2),
                ])?;
                let zero = set_obj_draw_transform(&[
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Proplist({
                        let mut map = ValueMap::new();
                        map.insert("id".into(), Value::Int(object_id.as_u64() as i32));
                        map
                    }),
                    Value::Int(-3),
                ])?;
                Ok::<_, RuntimeError>((rotated, zero))
            },
        );

        assert_eq!(
            result.expect("SetObjDrawTransform succeeds"),
            (Value::Bool(true), Value::Bool(true))
        );
        let update = outcome.object_update.expect("object update expected");
        let overlays = update
            .graphics_overlays
            .expect("graphics overlay update expected");
        let overlay = overlays
            .iter()
            .find(|overlay| overlay.id == -2)
            .expect("overlay present");
        let transform = overlay.transform.expect("overlay transform set");
        assert_eq!(
            transform.matrix(),
            [
                866.0 / 1000.0,
                -0.5,
                0.125,
                0.5,
                866.0 / 1000.0,
                -0.25,
                0.0,
                0.0,
                1.0,
            ]
        );
        let zero_overlay = overlays
            .iter()
            .find(|overlay| overlay.id == -3)
            .expect("zero overlay present");
        assert_eq!(
            zero_overlay
                .transform
                .expect("zero overlay transform remains allocated")
                .matrix(),
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]
        );
    }

    #[test]
    fn set_obj_draw_transform2_composes_all_nine_matrix_components() {
        let object_id = ObjectId::new(6);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0,
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            Some(DrawTransform::from_matrix([
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0,
            ])),
            None,
        );

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_obj_draw_transform2(&[
                    Value::Int(2000),
                    Value::Int(3000),
                    Value::Int(5000),
                    Value::Int(7000),
                    Value::Int(11000),
                    Value::Int(13000),
                    Value::Int(17000),
                    Value::Int(19000),
                    Value::Int(23000),
                ])
            },
        );

        assert_eq!(
            result.expect("SetObjDrawTransform2 succeeds"),
            Value::Bool(true)
        );
        let transform = outcome
            .object_update
            .expect("object update expected")
            .draw_transform
            .expect("transform update expected")
            .expect("transform set");
        assert_eq!(
            transform.matrix(),
            [49.0, 59.0, 74.0, 142.0, 173.0, 217.0, 254.0, 313.0, 395.0]
        );
    }

    #[test]
    fn set_obj_draw_transform2_retains_identity_matrix() {
        let object_id = ObjectId::new(7);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0,
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        );

        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || {
                set_obj_draw_transform2(&[
                    Value::Int(1000),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1000),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1000),
                ])
            },
        );

        assert_eq!(
            result.expect("SetObjDrawTransform2 succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            outcome
                .object_update
                .expect("object update expected")
                .draw_transform,
            Some(Some(DrawTransform::identity()))
        );
    }

    #[test]
    fn set_obj_draw_transform2_tenth_argument_is_the_overlay_id() {
        let object_id = ObjectId::new(8);
        let object_context = HostObjectContext::with_category(
            object_id,
            None,
            ObjectStatus::Normal,
            0,
            0,
            crate::FULL_CON,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            0,
            &[],
            "Idle",
            0,
            0,
            0,
            ActionLibrary::default(),
            Direction::Right,
            CommandDirection::Stop,
            0,
            None,
            None,
            &[],
            DEFAULT_CATEGORY,
            ocf::NORMAL,
            false,
            None,
            None,
        );
        let with_overlay =
            object_context
                .clone()
                .with_graphics_overlays(vec![ObjectGraphicsOverlay::new(
                    1,
                    GraphicsOverlayMode::Base,
                )]);
        let args = [
            Value::Int(1000),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(1000),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(1000),
            Value::Int(1),
        ];

        let (result, outcome) = with_effect_context(
            Some(with_overlay.clone()),
            &[],
            HostWorldContext::default(),
            100,
            || set_obj_draw_transform2(&args),
        );
        assert_eq!(
            result.expect("the tenth integer is not parsed as an object"),
            Value::Bool(true)
        );
        let overlays = outcome
            .object_update
            .expect("object update expected")
            .graphics_overlays
            .expect("overlay update expected");
        assert_eq!(overlays[0].transform, Some(DrawTransform::identity()));

        let (result, _) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            100,
            || set_obj_draw_transform2(&args),
        );
        assert_eq!(
            result.expect("a missing overlay is not an error"),
            Value::Bool(false)
        );

        let (result, _) = with_effect_context_with_state_and_definition(
            Some(with_overlay.clone()),
            Some(DefinitionId::from("TEST")),
            None,
            &[],
            HostWorldContext::default(),
            100,
            false,
            || set_obj_draw_transform2(&args),
        );
        assert_eq!(
            result.expect("a definition-only call is not an error"),
            Value::Bool(false),
            "a mutable carrier does not substitute for missing cthr->Obj"
        );

        let mut surplus = args.to_vec();
        surplus.push(Value::Int(0));
        let (result, _) = with_effect_context(
            Some(with_overlay),
            &[],
            HostWorldContext::default(),
            100,
            || set_obj_draw_transform2(&surplus),
        );
        let error = result.expect_err("an eleventh argument is not accepted");
        assert!(error.message().contains("additional arguments"));
    }

    #[test]
    fn object_count_returns_number_of_matches() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(30),
                "DUMY",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(31),
                "DUMY",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(10, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [Value::C4Id("DUMY".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || object_count(&args));
        let value = result.expect("ObjectCount succeeds");
        assert_eq!(value, Value::Int(2));
    }

    #[test]
    fn object_count_honours_owner_filter() {
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                ObjectId::new(40),
                "DUMY",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                1,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(41),
                "DUMY",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                2,
                100,
                crate::FULL_CON,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
        ]);
        let args = [
            Value::C4Id("DUMY".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            // iOwner is ObjectCount's 10th parameter (C4Script.cpp:2085).
            Value::Int(2),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || object_count(&args));
        let value = result.expect("ObjectCount owner succeeds");
        assert_eq!(value, Value::Int(1));
    }

    #[test]
    fn find_objects_returns_all_matches_in_order() {
        let container = ObjectId::new(40);
        let world = HostWorldContext::from_objects(vec![
            HostWorldObject::new(
                container,
                "CONT",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(0, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                None,
            ),
            HostWorldObject::new(
                ObjectId::new(41),
                "ITEM",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(3, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container),
            ),
            HostWorldObject::new(
                ObjectId::new(42),
                "ITEM",
                ObjectStatus::Normal,
                "Idle",
                None,
                None,
                None,
                OWNER_NONE,
                100,
                crate::FULL_CON,
                Vector2::new(5, 0),
                Vector2::ZERO,
                Vec::new(),
                0,
                0,
                Some(container),
            ),
        ]);
        let args = [
            Value::C4Id("ITEM".into()),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Int(ANY_CONTAINER_SENTINEL),
        ];
        let (result, _) = with_effect_context(None, &[], world, 1, || find_objects(&args));
        let value = result.expect("FindObjects succeeds");
        match value {
            Value::Array(entries) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0], object_reference_value(ObjectId::new(41)));
                assert_eq!(entries[1], object_reference_value(ObjectId::new(42)));
            }
            other => panic!("expected array, got {:?}", other),
        }
    }

    proptest! {
        #[test]
        fn do_energy_sequence_clamps_within_bounds(deltas in proptest::collection::vec(-200..=200i32, 0..16)) {
            let start_energy = DEFAULT_MAX_ENERGY;
            let expected = expected_energy_after_sequence(start_energy, &deltas);

            let sequence = deltas.clone();
            let object = object_host_context_with_physical_energy(
                start_energy,
                DEFAULT_MAX_ENERGY,
            );
            let (result, outcome) = with_effect_context(
                Some(object),
                &[],
                HostWorldContext::default(),
                1,
                move || {
                    for delta in sequence.iter().copied() {
                        let value = do_energy(&[Value::Int(delta)])?;
                        match value {
                            Value::Bool(true) => {}
                            Value::Bool(false) => {
                                return Err(RuntimeError::new("DoEnergy rejected update"));
                            }
                            other => {
                                return Err(RuntimeError::new(format!(
                                    "DoEnergy returned unexpected value: {}",
                                    other.type_name()
                                )));
                            }
                        }
                    }
                    Ok(Value::Nil)
                },
            );

            prop_assert!(result.is_ok());

            let final_energy = outcome
                .object_update
                .and_then(|update| update.energy)
                .unwrap_or(start_energy);

            prop_assert_eq!(final_energy, expected);
        }
    }

    // C4Object::DoEnergy model (C4Object.cpp:1345-1364): percent deltas
    // scale by C4MaxPhysical/100 and each change clamps to the fixture's
    // explicit Physical Energy ceiling.
    fn expected_energy_after_sequence(start: i32, deltas: &[i32]) -> i32 {
        let mut energy = start;
        for &delta in deltas {
            energy = crate::bound_energy(
                energy.saturating_add(delta.saturating_mul(LEGACY_MAX_PHYSICAL / 100)),
                DEFAULT_MAX_ENERGY,
            );
        }
        energy
    }

    #[test]
    fn add_global_effect_records_global_command() {
        let (result, outcome) =
            with_effect_context(None, &[], HostWorldContext::default(), 1, || {
                add_effect(&[Value::String("Glow".into()), Value::Nil, Value::Int(120)])
            });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert!(outcome.object.is_empty());
        assert_eq!(outcome.global.len(), 1);
        match &outcome.global[0] {
            EffectCommand::Add { effect, .. } => {
                assert_eq!(effect.name, "Glow");
                assert_eq!(effect.priority, 120);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn global_effect_queries_use_context_view() {
        let (result, _) = with_effect_context(
            None,
            &[],
            HostWorldContext::default(),
            1,
            || -> Result<Value, RuntimeError> {
                add_effect(&[Value::String("Glow".into()), Value::Nil, Value::Int(90)])?;
                get_effect(&[
                    Value::String("Glow".into()),
                    Value::Nil,
                    Value::Int(0),
                    Value::Int(1),
                ])
            },
        );

        let value = result.expect("GetEffect succeeds");
        assert_eq!(value, Value::String("Glow".into()));
    }

    #[test]
    fn remove_global_effect_handles_missing() {
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            remove_effect(&[Value::Nil, Value::Nil, Value::Int(0)])
        });

        let value = result.expect("RemoveEffect succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn effect_natives_extract_bool_payloads_from_every_c4valueint_slot() {
        // CheckConvertFunctionParameters accepts C4V_Bool for C4V_Int without
        // retagging it (C4Value.cpp:514-518). C4AulEngineFunc then extracts the
        // shared low Data.Int through C4ValueConv<C4ValueInt>::_FromC4V
        // (C4Value.h:317-322; C4Script.cpp:6170-6174).
        let raw_bool = Value::from_c4_bool_raw;
        let (result, _) = with_effect_context(
            None,
            &[],
            HostWorldContext::default(),
            1,
            || -> Result<Value, RuntimeError> {
                assert_eq!(
                    add_effect(&[
                        Value::String("Probe".into()),
                        Value::Nil,
                        Value::Bool(true),
                        raw_bool(2),
                    ])?,
                    Value::Int(1),
                    "Bool priority and raw-Bool interval extract as integers"
                );
                assert_eq!(
                    add_effect(&[
                        Value::String("Aux".into()),
                        Value::Nil,
                        Value::Bool(true),
                        raw_bool(3),
                    ])?,
                    Value::Int(2)
                );

                assert_eq!(
                    check_effect(&[
                        Value::String("Candidate".into()),
                        Value::Nil,
                        raw_bool(2),
                        raw_bool(3),
                    ])?,
                    Value::Int(0),
                    "CheckEffect extracts both integer slots"
                );
                assert_eq!(
                    get_effect_count(&[Value::Nil, Value::Nil, Value::Bool(true)])?,
                    Value::Int(2)
                );
                assert_eq!(
                    get_effect(&[
                        Value::String("Probe".into()),
                        Value::Nil,
                        Value::Bool(false),
                        raw_bool(3),
                        Value::Bool(true),
                    ])?,
                    Value::Int(2),
                    "Bool index/max-priority and raw-Bool query retain Data.Int"
                );

                assert_eq!(
                    change_effect(&[
                        Value::String("Probe".into()),
                        Value::Nil,
                        Value::Bool(false),
                        Value::String("Changed".into()),
                        raw_bool(4),
                    ])?,
                    Value::Bool(true)
                );
                assert_eq!(
                    get_effect(&[
                        Value::String("Changed".into()),
                        Value::Nil,
                        Value::Bool(false),
                        raw_bool(3),
                    ])?,
                    Value::Int(4),
                    "ChangeEffect extracts its Bool index and raw-Bool timer"
                );

                effect_var(&[raw_bool(2), Value::Nil, raw_bool(2), Value::Int(77)])?;
                assert_eq!(
                    effect_var(&[raw_bool(2), Value::Nil, raw_bool(2)])?,
                    Value::Int(77),
                    "EffectVar extracts both raw-Bool integer address slots"
                );
                assert_eq!(
                    effect_call(&[Value::Nil, raw_bool(2), Value::String("Missing".into()),])?,
                    Value::Nil,
                    "EffectCall accepts a raw-Bool effect number"
                );

                for name in [Value::String("Changed".into()), Value::Nil] {
                    assert_eq!(
                        remove_effect(&[name, Value::Nil, Value::Int(-1), Value::Bool(true),])?,
                        Value::Bool(false),
                        "a negative named index or effect number is a miss, not an error"
                    );
                }

                assert_eq!(
                    remove_effect(&[
                        Value::String("Changed".into()),
                        Value::Nil,
                        Value::Bool(false),
                        raw_bool(2),
                    ])?,
                    Value::Bool(true),
                    "RemoveEffect extracts its Bool index and raw-Bool flag"
                );
                get_effect_count(&[Value::Nil, Value::Nil])
            },
        );

        assert_eq!(
            result.expect("all typed effect arguments extract like C++"),
            Value::Int(1),
            "only Aux remains after removing Changed"
        );
    }

    #[test]
    fn fire_constructor_extracts_raw_bool_start_parameters_from_data_int() {
        let high_word_bool = Value::from_c4_bool_data_raw(1usize.checked_shl(32).unwrap_or(2));
        let (result, outcome) = with_object_host_context(|| {
            let random = enter_random_context(LcgRng::new(9));
            let result = add_effect(&[
                Value::String(crate::C4FX_FIRE.to_string().into()),
                Value::Object(1),
                Value::Int(crate::C4FX_FIRE_PRIORITY),
                Value::Int(crate::C4FX_FIRE_TIMER_INTERVAL),
                Value::Nil,
                Value::Nil,
                Value::from_c4_bool_raw(2),
                high_word_bool,
            ]);
            let _ = random.finish();
            result
        });

        assert!(matches!(result, Ok(Value::Int(number)) if number > 0));
        let effect = outcome
            .object
            .iter()
            .rev()
            .find_map(|command| match command {
                EffectCommand::Update(effect) if effect.name == crate::C4FX_FIRE => Some(effect),
                _ => None,
            })
            .expect("FxFireStart writes the live Fire effect vars");
        assert_eq!(effect.var(1), EffectVarValue::Int(2));
        assert_eq!(
            effect.var(2),
            EffectVarValue::Bool(usize::BITS <= 32),
            "native bool extraction reads the low Data.Int word, not full-union truthiness"
        );
    }

    #[test]
    fn typed_null_object_targets_the_global_effect_list() {
        // A strict caller can retain a transient C4V_C4Object tag with a zero
        // payload. C4ValueConv<C4Object *>::_FromC4V extracts nullptr, and all
        // effect natives consequently select Game.pGlobalEffects.
        let null_object = Value::Object(0);
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            let null_scope = determine_scope_from_state(&null_object)?;
            assert_eq!(null_scope, EffectScope::Global);
            assert_eq!(
                effect_callback_target_value(null_scope, &null_object),
                Value::Nil,
                "C4VObj(nullptr) canonicalizes the Fx callback target to nil"
            );
            let number = add_effect(&[
                Value::String("Global".into()),
                null_object.clone(),
                Value::Int(100),
                Value::Int(2),
            ])?;
            assert_eq!(number, Value::Int(1));
            assert_eq!(
                get_effect_count(&[Value::Nil, null_object.clone()])?,
                Value::Int(1)
            );
            assert_eq!(
                get_effect(&[
                    Value::String("Global".into()),
                    null_object.clone(),
                    Value::Int(0),
                    Value::Int(1),
                ])?,
                Value::String("Global".into())
            );
            assert_eq!(
                get_effect(&[Value::String("Global".into()), Value::Object(1),])?,
                Value::Nil,
                "the active object's list remains untouched"
            );
            assert_eq!(
                check_effect(&[
                    Value::String("Candidate".into()),
                    null_object.clone(),
                    Value::Int(50),
                    Value::Int(1),
                ])?,
                Value::Int(0)
            );
            assert_eq!(
                change_effect(&[
                    Value::String("Global".into()),
                    null_object.clone(),
                    Value::Int(0),
                    Value::String("Renamed".into()),
                    Value::Int(-1),
                ])?,
                Value::Bool(true)
            );

            effect_var(&[
                Value::Int(0),
                null_object.clone(),
                number.clone(),
                Value::Int(55),
            ])?;
            assert_eq!(
                effect_var(&[Value::Int(0), null_object.clone(), number.clone()])?,
                Value::Int(55)
            );
            assert_eq!(
                effect_call(&[null_object.clone(), number, Value::String("Missing".into()),])?,
                Value::Nil
            );
            assert_eq!(
                remove_effect(&[
                    Value::String("Renamed".into()),
                    null_object.clone(),
                    Value::Int(0),
                    Value::Bool(true),
                ])?,
                Value::Bool(true)
            );
            get_effect_count(&[Value::Nil, null_object])
        });

        assert_eq!(
            result.expect("typed null object consistently selects globals"),
            Value::Int(0)
        );
        assert!(outcome.object.is_empty());
        assert!(
            !outcome.global.is_empty(),
            "all mutations were recorded against the global list"
        );
    }

    #[test]
    fn synthetic_proplist_effect_target_still_selects_the_active_object() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| -> Result<Value, RuntimeError> {
            add_effect(&[
                Value::String("Synthetic".into()),
                state.clone(),
                Value::Int(100),
            ])?;
            Ok(Value::Array(vec![
                get_effect_count(&[Value::Nil, state])?,
                get_effect_count(&[Value::Nil, Value::Object(0)])?,
            ]))
        });

        assert_eq!(
            result.expect("synthetic target remains object-scoped"),
            Value::Array(vec![Value::Int(1), Value::Int(0)])
        );
        assert!(!outcome.object.is_empty());
        assert!(outcome.global.is_empty());
    }
