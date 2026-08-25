// Contiguous slice 7 of 8 of the `scenario/tests` battery, spliced
// by `include!` from the parent module so every test id is unchanged.

    #[test]
    fn map_creation_rng_can_be_traced_without_moving_the_stream() {
        // Native traces map-creation draws like any other, because it has no
        // separate map generator -- it brackets the phase with FixRandom
        // instead (C4Landscape.cpp:579,735). An untraced map RNG here hides
        // that phase from LC_RUST_RNG_TRACE while the oracle records it, so
        // the two traces cannot be diffed across initialisation
        // (clonk-org/clonk-rs#1050).
        use crate::scenario::map::{legacy_map_creation_rng, legacy_map_creation_rng_traced};

        let traced = legacy_map_creation_rng_traced(0xC4, true);
        assert_ne!(traced.trace_index, 0, "arming must survive construction");

        let plain = legacy_map_creation_rng_traced(0xC4, false);
        assert_eq!(plain.trace_index, 0);
        assert_eq!(traced.count, plain.count);
        assert_eq!(traced.rnd3_ptr(), plain.rnd3_ptr());

        let existing = legacy_map_creation_rng(0xC4);
        assert_eq!(existing.count, plain.count);
        assert_eq!(existing.rnd3_ptr(), plain.rnd3_ptr());
    }

    #[test]
    fn weather_init_retains_the_precipitation_material_name() {
        // C4SWeather::CompileFunc stores `Precipitation` and
        // C4Weather::Init passes that exact name to LaunchCloud
        // (C4Scenario.cpp:390; C4Weather.cpp:55-57,205-211).
        let manifest =
            parse_legacy_scenario_text("[Head]\nTitle=Rain\n\n[Weather]\nPrecipitation=AcidRain\n").test_value();

        let weather = derive_legacy_weather_init(&manifest).test_value();
        assert_eq!(weather.precipitation, "AcidRain");
    }

    #[test]
    fn empty_c4sval_keys_match_core_component_defaults_at_runtime() {
        let manifest = parse_legacy_scenario_text(
            "[Head]\nTitle=Empty C4SVal\n\n[Landscape]\nGravity=\nMapZoom=\n\n[Weather]\nClimate=\n",
        ).test_value();

        let expected_gravity = LegacyC4SVal::new(0, 0, 10, 200);
        let expected_map_zoom = LegacyC4SVal::new(0, 0, 5, 15);
        let expected_climate = LegacyC4SVal::new(0, 10, 0, 100);
        assert_eq!(manifest.core.landscape.gravity, expected_gravity);
        assert_eq!(manifest.core.landscape.map_zoom, expected_map_zoom);
        assert_eq!(manifest.core.weather.climate, expected_climate);

        let (physics, gravity) = derive_legacy_physics(&manifest).test_value();
        assert_eq!(gravity, expected_gravity);
        assert_eq!(
            physics.expect("Landscape section yields physics").gravity,
            10,
            "empty Gravity evaluates/clamps to the C++ minimum"
        );
        assert_eq!(
            legacy_map_zoom_value(manifest.sections.get("landscape")),
            expected_map_zoom
        );

        let weather = derive_legacy_weather_init(&manifest).test_value();
        assert_eq!(weather.climate, expected_climate);
        let environment = derive_legacy_environment(&manifest).test_value();
        assert_eq!(environment.climate, 50);

        let absent = parse_legacy_scenario_text(
            "[Head]\nTitle=Absent C4SVal\n\n[Landscape]\nMapWidth=64\n\n[Weather]\nNoGamma=1\n",
        ).test_value();
        let (physics, gravity) = derive_legacy_physics(&absent).test_value();
        assert_eq!(gravity, LegacyC4SVal::new(100, 0, 10, 200));
        assert_eq!(
            physics.expect("Landscape section yields physics").gravity,
            100
        );
        assert_eq!(
            legacy_map_zoom_value(absent.sections.get("landscape")),
            LegacyC4SVal::new(10, 0, 5, 15)
        );
        let weather = derive_legacy_weather_init(&absent).test_value();
        assert_eq!(weather.climate, LegacyC4SVal::new(50, 10, 0, 100));
    }

    #[test]
    fn legacy_scenario_wind_c4sval_fields_are_retained_verbatim() {
        for (source, expected) in [
            ("0,70,-30,30", LegacyC4SVal::new(0, 70, -30, 30)),
            ("50,10,0,20", LegacyC4SVal::new(50, 10, 0, 20)),
            ("50,0,0,20", LegacyC4SVal::new(50, 0, 0, 20)),
        ] {
            let manifest = parse_legacy_scenario_text(&format!(
                "[Head]\nTitle=Wind\n\n[Weather]\nWind={source}\n"
            )).test_value();
            let weather = derive_legacy_weather_init(&manifest).test_value();
            let environment = derive_legacy_environment(&manifest).test_value();

            assert_eq!(weather.wind, expected);
            assert_eq!(environment.wind, expected.base());
            assert_eq!(environment.base_wind, expected.std);
            assert_eq!(environment.wind_variation, expected.rnd);
            assert_eq!(environment.wind_min, expected.min);
            assert_eq!(environment.wind_max, expected.max);

            let mut runtime = environment;
            let mut rng = crate::rng::LcgRng::seed_from_u64(0xC4);
            let mut mirror = rng.clone();
            let range = expected.rnd.wrapping_mul(2).wrapping_add(1);
            let raw = expected
                .std
                .wrapping_add(mirror.random(range))
                .wrapping_sub(expected.rnd);
            let bounded = if raw < expected.min {
                expected.min
            } else if raw > expected.max {
                expected.max
            } else {
                raw
            };
            runtime.advance_frame(&mut rng, 1_000);
            assert_eq!(runtime.wind_target, bounded);
            assert_eq!(rng, mirror);
            assert_eq!(runtime.base_wind, expected.std);

            let encoded = serde_json::to_string(&environment).test_value();
            let mut restored: EnvironmentSettings =
                serde_json::from_str(&encoded).test_value();
            restored.refresh_runtime_fields();
            assert_eq!(restored.base_wind, expected.std);
            assert_eq!(restored.wind_variation, expected.rnd);
            assert_eq!(restored.wind_min, expected.min);
            assert_eq!(restored.wind_max, expected.max);
        }
    }

    #[test]
    fn scenario_apply_replays_the_weather_init_ledger_like_cpp() {
        let dir = test_tempdir();
        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).test_value();
        write_test_file(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\n",
        );
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Windy.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        // GoldRush-shaped: NoInitialize=1 skips the rain block entirely
        // (C4Weather.cpp:49) — 8 draws total.
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Windy\nNoInitialize=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Weather]\nClimate=10,0\nStartSeason=44,0\nYearSpeed=0\nWind=0,75\n",
        );

        let mut engine = Engine::with_seed(7);
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        apply_test_scenario(&scenario, &mut engine);

        // Replay the exact C++ draw order on a twin RNG.
        let mut replay = crate::rng::LcgRng::seed_from_u64(7);
        // Landscape.ScenarioInit's Gravity draw precedes the weather
        // evaluates (C4Landscape.cpp:66).
        let _gravity = LegacyC4SVal::new(100, 0, 10, 200).evaluate(&mut replay);
        let season = LegacyC4SVal::new(44, 0, 0, 100).evaluate(&mut replay);
        let year_speed = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);
        let climate = 100 - LegacyC4SVal::new(10, 0, 0, 100).evaluate(&mut replay) - 50;
        let wind = LegacyC4SVal::new(0, 75, -100, 100).evaluate(&mut replay);
        let lightning = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);
        let meteorite = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);
        let volcano = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);
        let earthquake = LegacyC4SVal::new(0, 0, 0, 100).evaluate(&mut replay);

        let environment = engine.environment();
        assert_eq!(environment.season, season.clamp(0, 100));
        assert_eq!(environment.year_speed, year_speed);
        assert_eq!(environment.climate, climate);
        assert_eq!(
            (environment.wind, environment.wind_target),
            (wind, wind),
            "Wind = TargetWind = Wind.Evaluate (C4Weather.cpp:47)"
        );
        assert_eq!(environment.lightning, lightning);
        assert_eq!(environment.meteorite, meteorite);
        assert_eq!(environment.volcano, volcano);
        assert_eq!(environment.earthquake, earthquake);

        // After the draws, C4Game::Synchronize re-fixes the ledger
        // (C4Game.cpp:474,3695): the post-apply position is a FRESH
        // FixRandom(seed) stream, not the post-draw position.
        let mut fresh = crate::rng::LcgRng::seed_from_u64(7);
        assert_eq!(
            engine.debug_rng_clone().random(1_000_000),
            fresh.random(1_000_000),
            "game-start Synchronize re-fixes the ledger after the weather draws"
        );
    }

    // Objects.txt serializes the CURRENT shape per object (C4Shape::
    // CompileFunc into the [Object] section, C4Shape.cpp:495-515):
    // Vertices/VertexX/VertexY/VertexCNAT/VertexFriction load VERBATIM —
    // they are the post-Con/rotation effective shape, not a base to
    // re-transform. C++ keeps them until the next UpdateShape (which
    // recomputes from the def), so resting objects keep saved overrides
    // like VertexFriction=50 indefinitely.
    #[test]
    fn objects_txt_restores_saved_shape_vertices_verbatim_like_cpp() {
        let dir = test_tempdir();

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).test_value();
        // The def's own shape differs from the saved one: 1 vertex,
        // friction 30 — the 30-vs-50 live-diff class.
        write_test_file(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\nRotate=1\n\
             Vertices=1\nVertexX=0\nVertexY=0\nVertexFriction=30\n",
        );
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Verts.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Verts\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        );
        // 90: plain saved shape (3 vertices, friction 50) — verbatim. The
        //     fourth entries are dormant fixed-buffer values beyond VtxNum.
        // 91: ROTATED object — the saved vertices are already rotated;
        //     applying the spawn rotation again would double-rotate.
        // 92: omitted Vertices defaults to zero after C4Object::Clear; it does
        //     not restore the definition's one active vertex.
        // 93: explicit Vertices=0 likewise keeps an empty active prefix while
        //     retaining independently serialized dormant slot metadata.
        // 94: OwnVertices restores its untransformed original from slots 15+.
        write_test_file(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=90\nStatus=1\nX=10\nY=10\nWidth=17\nHeight=19\nOffset=-8,-9\nFireTop=6\nContactDensity=25\n\
             Vertices=3\nVertexX=2,-14,14,99\nVertexY=11,-4,-4,88\n\
             VertexCNAT=8,1,2,4\nVertexFriction=50,50,50,77\n\n\
             [Object]\nid=GOOD\nNumber=91\nStatus=1\nX=30\nY=10\nRotation=90\n\
             Vertices=1\nVertexX=-11\nVertexY=2\nVertexFriction=50\n\n\
             [Object]\nid=GOOD\nNumber=92\nStatus=1\nX=40\nY=10\nRotation=-9\n\n\
             [Object]\nid=GOOD\nNumber=93\nStatus=1\nX=50\nY=10\nVertices=0\n\
             VertexX=7\nVertexY=8\nVertexCNAT=2\nVertexFriction=90\n\n\
             [Object]\nid=GOOD\nNumber=94\nStatus=1\nX=60\nY=10\nVertices=1\nOwnVertices=1\n\
             VertexX=1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,42\n\
             VertexY=2,0,0,0,0,0,0,0,0,0,0,0,0,0,0,-5\n\
             VertexCNAT=0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,8\n\
             VertexFriction=10,0,0,0,0,0,0,0,0,0,0,0,0,0,0,66\n",
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        let idx = engine.test_object_index(ObjectId::new(90));
        let vertices = &engine.objects[idx].state.vertices;
        assert_eq!(
            engine.objects[idx].state.contact_density, 25,
            "saved live Shape.ContactDensity loads with the embedded shape"
        );
        assert_eq!(
            engine.objects[idx].current_shape_rect(),
            Some(crate::DefinitionRect::new(-8, -9, 17, 19))
        );
        let snapshot = engine.objects[idx].snapshot(None);
        assert_eq!(
            snapshot.current_shape,
            engine.objects[idx].current_shape_rect()
        );
        assert_eq!(snapshot.current_fire_top, Some(6));
        assert_eq!(vertices.len(), 3, "saved Vertices= count wins over the def");
        assert_eq!(
            (
                vertices[0].x,
                vertices[0].y,
                vertices[0].cnat,
                vertices[0].friction
            ),
            (2, 11, 8, 50),
            "saved vertex 0 loads verbatim incl. CNAT and friction"
        );
        assert_eq!(
            (vertices[1].x, vertices[1].y, vertices[1].friction),
            (-14, -4, 50)
        );
        assert_eq!(
            engine.objects[idx].state.shape_vertices.slots[3],
            crate::ObjectVertex::new(99, 88)
                .with_cnat(4)
                .with_friction(77),
            "mkArrayAdapt's fourth entries survive beyond saved VtxNum=3"
        );

        let idx = engine.test_object_index(ObjectId::new(91));
        let vertices = &engine.objects[idx].state.vertices;
        assert_eq!(
            engine.objects[idx].state.contact_density, 50,
            "missing saved ContactDensity uses C4Shape::Clear's solid default"
        );
        assert_eq!(
            (vertices[0].x, vertices[0].y),
            (-11, 2),
            "saved vertices are the ALREADY-rotated shape — no re-rotation at load"
        );
        assert_eq!(engine.objects[idx].state.rotation, 90);
        let idx = engine.test_object_index(ObjectId::new(92));
        assert_eq!(engine.objects[idx].state.rotation, -9);
        assert_eq!(engine.objects[idx].fixed_rotation, crate::itofix(-9));
        assert!(
            engine.objects[idx].state.vertices.is_empty(),
            "missing Vertices defaults to VtxNum=0 instead of falling back to the definition"
        );

        let idx = engine.test_object_index(ObjectId::new(93));
        assert!(
            engine.objects[idx].state.vertices.is_empty(),
            "explicit Vertices=0 remains an empty active shape"
        );
        assert_eq!(
            engine.objects[idx].state.shape_vertices.slots[0],
            crate::ObjectVertex::new(7, 8)
                .with_cnat(2)
                .with_friction(90),
            "zero active vertices do not discard dormant slot data"
        );

        let idx = engine.test_object_index(ObjectId::new(94));
        assert_eq!(
            engine.objects[idx].own_shape_vertices,
            Some(vec![crate::ObjectVertex::new(42, -5)
                .with_cnat(8)
                .with_friction(66)]),
            "OwnVertices rebuilds the original vertex copy from raw slots 15+"
        );
    }

    // C4Game::InitGame environment placements (C4Game.cpp:2493-2503):
    // scenarios without NoInitialize place [Landscape] Vegetation= on
    // surface soil (PlaceVegetation, C4Game.cpp:2962-3007), InEarth= into
    // earth pixels (PlaceInEarth, C4Game.cpp:2949-2960) and create the
    // [Environment] Objects= / [Game] Goals=/Rules= objects
    // (C4Game.cpp:3988-4018) — all through the synced ledger between the
    // Gravity draw and Weather.Init's.
    #[test]
    fn init_placements_populate_vegetation_inearth_and_rule_objects() {
        let dir = test_tempdir();

        let defs_root = dir.path().join("Defs.c4d");
        for (folder, core) in [
            (
                "Tree.c4d",
                // Growth admits the random-con draw (C4Game.cpp:2974).
                "[DefCore]\nid=TREE\nName=Tree\nCategory=2\nWidth=8\nHeight=12\n\
                 Offset=-4,-6\nVertices=1\nVertexX=0\nVertexY=0\nGrowth=4\nPlacement=0\n",
            ),
            (
                "Rock.c4d",
                "[DefCore]\nid=ROCK\nName=Rock\nCategory=16\nWidth=6\nHeight=6\nOffset=-3,-3\n",
            ),
            ("Goal.c4d", "[DefCore]\nid=GOAL\nName=Goal\nCategory=4096\n"),
            ("Rule.c4d", "[DefCore]\nid=RULE\nName=Rule\nCategory=8192\n"),
            (
                "Envr.c4d",
                "[DefCore]\nid=ENVR\nName=Envr\nCategory=16384\n",
            ),
        ] {
            let def_dir = defs_root.join(folder);
            std::fs::create_dir_all(&def_dir).test_value();
            write_test_file(def_dir.join("DefCore.txt"), core);
            write_test_definition_graphics(&def_dir);
        }

        let scenario_dir = dir.path().join("Placements.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Placements\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Game]\nGoals=GOAL=1;\nRules=RULE=1;\n\n\
             [Landscape]\nMapZoom=10\nMaterial=Earth\n\
             Vegetation=TREE=1;\nVegetationLevel=100,0\n\
             InEarth=ROCK=1;\nInEarthLevel=100,0\n\n\
             [Environment]\nObjects=ENVR=1;\n",
        );
        // 20x20 map, zoom 10 → 200x200 world: sky rows 0-9, earth 10-19
        // (surface at world y=100, inside PlaceVegetation's [50, hgt-50]).
        let mut rows: Vec<Vec<u8>> = Vec::new();
        for y in 0..20 {
            rows.push(vec![if y < 10 { 0u8 } else { 30 }; 20]);
        }
        let row_refs: Vec<&[u8]> = rows.iter().map(|row| row.as_slice()).collect();
        write_test_file(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&row_refs),
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "30=Earth-Smooth\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\nSoil=1\n",
        );
        write_test_texture(&materials, "Smooth");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\nName=Earth\nDensity=100\nSoil=1\n",
        ).test_value();
        let mut engine = Engine::with_seed(7);
        engine.configure_materials_from_library(&library);
        apply_test_scenario(&scenario, &mut engine);

        let count = |id: &str| {
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == id)
                .count()
        };
        // amt = (GBackWdt/50) * VegLevel/100 = 4 tries, 20 attempts each
        // over an all-soil surface: placements land.
        assert!(count("TREE") >= 1, "vegetation placed on surface soil");
        // amt = (200*200/5000) * 100/100 = 8 in-earth tries.
        assert!(count("ROCK") >= 1, "in-earth objects placed");
        assert_eq!(count("GOAL"), 1, "InitGoals creates the goal object");
        assert_eq!(count("RULE"), 1, "InitRules creates the rule object");
        assert_eq!(
            count("ENVR"),
            1,
            "InitEnvironment creates the environment object"
        );

        // Vegetation sits at the earth surface (y=100) + the 3+5 soil
        // probe offsets; in-earth rocks live inside the ground.
        let tree = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "TREE").test_value();
        assert!(
            (90..=130).contains(&tree.state.position.y),
            "tree y {} anchors at the surface",
            tree.state.position.y
        );
        let rock = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "ROCK").test_value();
        assert!(
            rock.state.position.y >= 100,
            "rock y {} is inside the earth",
            rock.state.position.y
        );

        // NoInitialize=1 skips the whole block (C4Game.cpp:2493).
        let scenario_txt = std::fs::read_to_string(scenario_dir.join("Scenario.txt")).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            scenario_txt.replace("Title=Placements", "Title=Placements\nNoInitialize=1"),
        );
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(7);
        engine.configure_materials_from_library(&library);
        apply_test_scenario(&scenario, &mut engine);
        let count = |id: &str| {
            engine
                .objects
                .iter()
                .filter(|object| object.definition_id == id)
                .count()
        };
        assert_eq!(count("TREE"), 0, "NoInitialize skips vegetation");
        assert_eq!(count("GOAL"), 0, "NoInitialize skips goals");
    }

    fn legacy_rule_goal_placement_scenario() -> (tempfile::TempDir, Scenario) {
        let dir = test_tempdir();
        let defs_root = dir.path().join("Defs.c4d");
        for (folder, id, category) in [
            ("Revivals.c4d", "RVLR", 8192),
            ("Energy.c4d", "ENRG", 8192),
            ("Race.c4d", "RACE", 4096),
            ("Melee.c4d", "MELE", 4096),
            ("GoalController.c4d", "GOAL", 8192),
        ] {
            let def_dir = defs_root.join(folder);
            std::fs::create_dir_all(&def_dir).test_value();
            write_test_file(
                def_dir.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory={category}\n"),
            );
            write_test_definition_graphics(&def_dir);
        }

        let scenario_dir = dir.path().join("RuleGoalSync.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=RuleGoalSync\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Game]\nMode=1\nGoals=RACE=1;\nRules=RVLR=1;\n\n\
             [Landscape]\nMapZoom=10\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[0, 0], &[0, 0]]),
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        (dir, scenario)
    }

    fn assert_converted_rule_goal_placements(engine: &Engine) {
        let snapshot = engine.snapshot();
        let count = |definition_id: &str| {
            snapshot
                .objects
                .iter()
                .filter(|object| object.definition_id == definition_id)
                .count()
        };
        assert_eq!(count("RVLR"), 1, "the authored rule is placed");
        assert_eq!(
            count("ENRG"),
            1,
            "the default StructNeedEnergy selector is converted and placed"
        );
        assert_eq!(count("RACE"), 1, "the authored goal is placed");
        assert_eq!(
            count("MELE"),
            1,
            "the legacy Mode selector is converted and placed"
        );
    }

    #[test]
    fn offline_game_start_places_converted_game_parameter_defaults() {
        // C4Scenario::Load converts the scenario in place before
        // C4GameParameters copies its default Rules/Goals
        // (oracle-src-pinned C4Scenario.cpp:86-97,503-540;
        // C4GameParameters.cpp:555-568). InitRules/InitGoals later consume
        // those parameters, never the pre-conversion lists
        // (C4Game.cpp:2511-2520,4016-4036).
        let (_dir, scenario) = legacy_rule_goal_placement_scenario();
        let mut engine = Engine::with_seed(7);

        scenario
            .apply_before_players(&mut engine).test_value();

        assert_converted_rule_goal_placements(&engine);
    }

    #[test]
    fn network_game_without_join_data_places_converted_game_parameter_defaults() {
        // A missing network override retains the same post-ConvertGoals
        // C4GameParameters defaults used by offline startup. JoinData may
        // replace these lists, but absence cannot expose pre-load Scenario.txt
        // state (oracle-src-pinned C4Scenario.cpp:86-97;
        // C4GameParameters.cpp:555-568; C4Game.cpp:4016-4036).
        let (_dir, scenario) = legacy_rule_goal_placement_scenario();
        let mut engine = Engine::with_seed(7);

        scenario
            .apply_before_players_for_game_start(&mut engine, true, None, None, None, None, None).test_value();

        assert_converted_rule_goal_placements(&engine);
    }

    #[test]
    fn network_game_start_retains_authoritative_empty_rule_goal_lists() {
        // JoinData compiles directly into C4GameParameters and its synchronized
        // lists remain authoritative at InitRules/InitGoals
        // (oracle-src-pinned C4GameParameters.cpp:555-568;
        // C4Game.cpp:4016-4036). An explicit empty list therefore must not
        // fall back to the converted scenario defaults.
        let (_dir, scenario) = legacy_rule_goal_placement_scenario();
        let authoritative = GameParameterRuleGoalLists::new(Vec::new(), Vec::new());
        let mut engine = Engine::with_seed(7);

        scenario
            .apply_before_players_for_game_start(
                &mut engine,
                true,
                None,
                None,
                None,
                Some(&authoritative),
                None,
            ).test_value();

        let snapshot = engine.snapshot();
        for definition_id in ["RVLR", "ENRG", "RACE", "MELE", "GOAL"] {
            assert!(
                snapshot
                    .objects
                    .iter()
                    .all(|object| object.definition_id != definition_id),
                "explicit empty JoinData suppresses {definition_id}"
            );
        }
    }

    // C4Game::InitRules/InitGoals read the synchronized
    // Game.Parameters.Rules/Goals lists, not the raw C4S.Game lists
    // (C4Game.cpp:4056-4076). This matters for legacy conversion: HarpoonRace
    // authors RVLR only, while C4SGame::ConvertGoals adds the default ENRG
    // rule to the parameters distributed in JoinData.
    #[test]
    fn network_game_start_places_synchronized_rules_and_goals_on_every_peer() {
        let (_dir, scenario) = legacy_rule_goal_placement_scenario();
        let parameters = scenario
            .lobby_metadata().test_value()
            .game_parameter_defaults();
        let synchronized = GameParameterRuleGoalLists::new(
            parameters
                .rules()
                .iter()
                .map(|entry| ScenarioIdListEntry::new(entry.id(), entry.count()))
                .collect(),
            parameters
                .goals()
                .iter()
                .map(|entry| ScenarioIdListEntry::new(entry.id(), entry.count()))
                .collect(),
        );
        assert_eq!(
            synchronized
                .rules()
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["RVLR", "ENRG"],
            "legacy conversion adds ENRG to the synchronized parameter list"
        );

        let mut host = Engine::with_seed(7);
        scenario
            .apply_before_players_for_game_start(
                &mut host,
                true,
                None,
                None,
                None,
                Some(&synchronized),
                None,
            ).test_value();
        let mut client = Engine::with_seed(7);
        scenario
            .apply_before_players_for_game_start(
                &mut client,
                true,
                None,
                None,
                None,
                Some(&synchronized),
                None,
            ).test_value();

        for engine in [&host, &client] {
            assert_eq!(
                engine
                    .snapshot()
                    .objects
                    .iter()
                    .filter(|object| object.definition_id == "ENRG")
                    .count(),
                1,
                "every peer places converted ENRG from Game.Parameters.Rules"
            );
        }
        assert_eq!(
            host.sync_check(0),
            client.sync_check(0),
            "host and client placement must produce the same synchronization digest"
        );
    }

    // Objects.txt Mobile/FixX/FixY/FixR/RDir ingestion
    // (C4Object.cpp:2762-2772): loaded objects keep the serialized Mobile
    // verbatim (default false) with the exact C4Fixed sub-pixel
    // position/rotation state, independent of the integer X/Y/Rotation.
    // A non-Mobile object with stale saved dirs stays frozen until the
    // Tick10 pulse wipes the dirs and re-snaps fix (C4Movement.cpp:576-587).
    #[test]
    fn objects_txt_restores_mobile_and_fixed_state_like_cpp() {
        let dir = test_tempdir();

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).test_value();
        write_test_file(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=16\nRotate=1\n",
        );
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Fixed.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Fixed\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\n",
        );
        // 40x40 world: sky everywhere, earth on the bottom row — the
        // objects at y=5 stay in free air.
        write_test_file(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 0, 0, 0],
                &[0, 0, 0, 0],
                &[0, 0, 0, 0],
                &[30, 30, 30, 30],
            ]),
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "30=Earth-Smooth\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        // 80: Mobile=1 flying right at 0.7 px/frame from x=15.25 —
        //     saved pairs keep x == fixtoi(fix_x) (round-to-nearest), so
        //     the sub-pixel stays under half. itofix(15)+0.25 = 999424;
        //     XDir 0.7 = legacy float bits f1060320051 -> raw 45875.
        // 81: Mobile absent (false) with STALE saved dirs — C++ keeps the
        //     dirs but never moves; the frame-10 pulse wipes them.
        // 82: rotating: Rotation=90, FixR = 90.25 deg (F5914624),
        //     RDir = raw 6554 (~0.1 deg/frame).
        // Category and Size are compiled object fields, not definition
        // fallbacks. Omitting Category would make FixObjectOrder repair the
        // rows to StaticBack (and SyncClearance would correctly zero XDir).
        write_test_file(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=80\nStatus=1\nCategory=16\nSize=100000\nX=15\nY=5\n\
             FixX=F999424\nFixY=F327680\nXDir=f1060320051\nMobile=1\n\n\
             [Object]\nid=GOOD\nNumber=81\nStatus=1\nCategory=16\nSize=100000\nX=25\nY=5\n\
             FixX=F1654784\nFixY=F327680\nXDir=F45875\n\n\
             [Object]\nid=GOOD\nNumber=82\nStatus=1\nCategory=16\nSize=100000\nX=35\nY=5\n\
             Rotation=90\nFixR=F5914624\nRDir=F6554\nMobile=1\n\n\
             [Object]\nid=GOOD\nNumber=83\nStatus=2\nCategory=16\nSize=100000\nX=45\nY=5\n\
             FixX=F2965504\nFixY=F327680\nRotation=12\nFixR=F802816\nXDir=F45875\nMobile=1\n\n\
             [Object]\nid=GOOD\nNumber=84\nStatus=2\nCategory=16\nSize=100000\nX=55\nY=5\n",
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        let idx_of = |engine: &Engine, number: u64| {
            engine.test_object_index(ObjectId::new(number))
        };

        // Ingestion snapshot before any tick.
        let mover = idx_of(&engine, 80);
        assert!(engine.objects[mover].state.mobile, "Mobile=1 sticks");
        assert_eq!(
            engine.objects[mover].fixed_velocity.x.val(),
            45_875,
            "lowercase-f float bits survive parsing and loaded spawn exactly"
        );
        // FixX/FixY load verbatim (C4Object.cpp:2762) but the game-start
        // SyncClearance collapses them to itofix(x,y) before InitPlayers
        // (C4Object.cpp:3810, C4Game.cpp:474) — 15 px exactly.
        assert_eq!(
            engine.objects[mover].fixed_position.x.val(),
            crate::math::itofix(15).val(),
            "SyncClearance reduces FixX to itofix(x) at game start"
        );
        assert_eq!(
            engine.objects[mover].state.position,
            Vector2::new(15, 5),
            "the integer X/Y stay independent of FixX/FixY"
        );
        let frozen = idx_of(&engine, 81);
        assert!(
            !engine.objects[frozen].state.mobile,
            "Mobile default false (C4Object.cpp:2772)"
        );
        assert_eq!(
            engine.objects[frozen].state.position,
            Vector2::new(25, 5),
            "the integer X/Y stay independent of the FixX/FixY sub-pixel"
        );
        assert_eq!(
            engine.objects[frozen].fixed_velocity.x.val(),
            45_875,
            "stale saved dirs load verbatim"
        );
        let spinner = idx_of(&engine, 82);
        assert_eq!(engine.objects[spinner].state.rotation, 90);
        // fix_r also collapses to itofix(r) at game start
        // (C4Object.cpp:3811).
        assert_eq!(
            engine.objects[spinner].fixed_rotation.val(),
            crate::math::itofix(90).val(),
            "FixR restores the exact rotation accumulator (C4Object.cpp:2764)"
        );
        assert_eq!(
            engine.objects[spinner].rotation_velocity.val(),
            6_554,
            "RDir restores the angular velocity (C4Object.cpp:2767)"
        );
        let inactive_saved = idx_of(&engine, 83);
        assert_eq!(
            engine.objects[inactive_saved].fixed_position.x.val(),
            2_965_504,
            "SyncClearance skips C4GameObjects::InactiveObjects"
        );
        assert_eq!(engine.objects[inactive_saved].fixed_rotation.val(), 802_816);
        assert_eq!(
            engine.objects[inactive_saved].fixed_velocity.x.val(),
            45_875
        );
        let inactive_default = idx_of(&engine, 84);
        assert_eq!(
            engine.objects[inactive_default].fixed_position,
            crate::math::FixedVec2::ZERO,
            "missing FixX/FixY retain C4Object::Default Fix0 when no action resolves"
        );

        // Frame 1: the Mobile mover integrates from its game-start
        // SyncClearance'd position (itofix(15) + 45875 = 1028915 -> 15.70
        // -> pixel 16, fixtoi rounds to nearest); the frozen object holds
        // position AND its stale dirs.
        engine.tick_without_snapshot().test_value();
        let mover = idx_of(&engine, 80);
        assert_eq!(
            engine.objects[mover].fixed_position.x.val(),
            crate::math::itofix(15).val() + 45_875
        );
        assert_eq!(engine.objects[mover].state.position.x, 16);
        let frozen = idx_of(&engine, 81);
        assert_eq!(engine.objects[frozen].state.position.x, 25);
        assert_eq!(engine.objects[frozen].fixed_velocity.x.val(), 45_875);

        // Frames 2-9: still frozen. Frame 10: the pulse wipes the stale
        // dirs and re-snaps fix to the integer position
        // (C4Movement.cpp:581-586).
        for _ in 2..=9 {
            engine.tick_without_snapshot().test_value();
        }
        let frozen = idx_of(&engine, 81);
        assert_eq!(engine.objects[frozen].fixed_velocity.x.val(), 45_875);
        engine.tick_without_snapshot().test_value();
        let frozen = idx_of(&engine, 81);
        assert!(engine.objects[frozen].state.mobile);
        assert_eq!(engine.objects[frozen].fixed_velocity.x.val(), 0);
        assert_eq!(
            engine.objects[frozen].fixed_position.x.val(),
            25 * 65536,
            "the pulse snaps fix_x to itofix(x), discarding the stale sub-pixel"
        );
    }

    #[test]
    fn static_map_classifies_materials_into_surface_and_liquid_columns() {
        // A static-map scenario without Map.bmp falls back to
        // Landscape.bmp (C4Landscape.cpp:593-601). Each map pixel byte is
        // a texmap index (IFT bit 0x80 stripped): index 0 = sky, else
        // TexMap.txt -> material -> density (PixCol2Mat/MatDensity,
        // C4Wrappers.h:110-145); liquid iff 25<=density<50, solid iff
        // density>=50 (C4Wrappers.h:68-81). The map zooms by MapZoom.
        // GoldRush's river bubbles depend on the liquid columns: their
        // LiquidCheck removes them when InLiquid() is false.
        let dir = test_tempdir();

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).test_value();
        write_test_file(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(good.join("Script.c"), "// fine\n");
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Liquid.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Liquid\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\nLiquid=Water-Smooth\n",
        );
        // Map (4x4): the middle columns are a CAVE river — an earth roof
        // over water over an earth bed (GoldRush's bubbles live in such an
        // underground river, below the column surface). Column 0 is open
        // ground, column 3 all sky.
        write_test_file(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 30, 30, 0],
                &[0, 20, 20, 0],
                &[30, 20, 20, 0],
                &[30, 30, 30, 0],
            ]),
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(
            materials.join("TexMap.txt"),
            "# table\n20=Water-Liquid\n30=Earth-Smooth\n",
        );
        write_test_file(
            materials.join("Water.c4m"),
            "[Material]\nName=Water\nDensity=25\n",
        );
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Liquid");
        write_test_texture(&materials, "Smooth");
        // A placed object INSIDE the pool: C4GameObjects::Load keeps
        // positions verbatim — no spawn-time surface ejection (GoldRush's
        // bubbles and fish sit in an underground river below the column
        // surface).
        write_test_file(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=77\nStatus=1\nCategory=0\nX=15\nY=15\n",
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let landscape = engine.landscape().test_value();

        let placed = engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.id == ObjectId::new(77))
            .cloned().test_value();
        assert_eq!(
            placed.position,
            Vector2::new(15, 15),
            "loaded objects keep their Objects.txt position (no surface snap)"
        );

        // Map column 1 (world x 10..20): earth roof row 0, water rows 1-2
        // (world y 10..30), earth bed row 3 (world y 30..40).
        assert!(landscape.is_liquid_at(15, 15), "river interior is liquid");
        assert!(
            landscape.is_liquid_at(15, 29),
            "river bottom edge is liquid"
        );
        assert!(!landscape.is_liquid_at(15, 5), "roof above the river");
        assert!(!landscape.is_liquid_at(15, 35), "earth bed is not liquid");
        assert!(landscape.is_solid_at(15, 35), "earth bed is solid");
        assert!(
            landscape.is_semi_solid_at(15, 15),
            "liquid counts as semi-solid (GBackSemiSolid)"
        );
        // Map column 0: earth from row 2 (world y 20).
        assert!(landscape.is_solid_at(5, 25));
        assert!(!landscape.is_liquid_at(5, 25));
        // Map column 3: all sky.
        assert!(!landscape.is_solid_at(35, 38), "sky column has no ground");
    }

    #[test]
    fn script_algorithm_calls_existing_named_function_per_pixel_with_cpp_arguments() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic algo_call;\n\
             func ScriptAlgoProbe(x, y, a, b) {\n\
                 if (!algo_call) algo_call = 0;\n\
                 var call = algo_call;\n\
                 algo_call++;\n\
                 Random(17);\n\
                 return x == (call % 3) * 100 && y == (call / 3) * 100\n\
                     && a == 17 && b == 29;\n\
             }\n",
        );
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Live ScriptAlgo\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=3,0,3,3\nMapHeight=2,0,2,2\n\
             MapZoom=10,2,5,15\nKeepMapCreator=1\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "map Test { seed=1; wdt=3px; hgt=2px;\n\
               overlay Probe { seed=2; algo=script; a=17; b=29;\n\
                               mat=Earth; tex=Rough; sub=0; };\n\
             };\n",
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        assert_eq!(
            engine
                .landscape()
                .and_then(Landscape::raster_state)
                .and_then(LandscapeRasterState::map)
                .map(|map| map.indices),
            Some(vec![1, 1, 1, 1, 1, 1]),
            "all six row-major calls receive their exact transformed arguments"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("algo_call")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(6)),
            "ScriptAlgo executes on the live linked host before initialization"
        );

        let mut map_rng = legacy_map_creation_rng(0);
        LegacyC4SVal::new(3, 0, 3, 3).evaluate(&mut map_rng);
        LegacyC4SVal::new(2, 0, 2, 2).evaluate(&mut map_rng);
        for _ in 0..6 {
            map_rng.random(17);
        }
        let expected_zoom = LegacyC4SVal::new(10, 2, 5, 15).evaluate(&mut map_rng) as u32 as i32;
        assert_eq!(
            engine
                .landscape()
                .and_then(Landscape::raster_state)
                .map(LandscapeRasterState::map_zoom),
            Some(expected_zoom),
            "ScriptAlgo Random calls precede MapZoom in the fixed map ledger"
        );
        let mut expected_game_rng = crate::rng::LcgRng::seed_from_u64(0);
        assert_eq!(
            engine.debug_rng_clone().random(1_000_000),
            expected_game_rng.random(1_000_000),
            "the fixed map epoch never leaks ScriptAlgo draws into gameplay RNG"
        );
    }

    #[test]
    fn plain_s2_map_renders_once_during_initial_activation() {
        // C4Landscape::CreateMapS2 performs one RenderTo after script linking
        // (C4Landscape.cpp:530-546). The eager Rust resource preview is that
        // render when no algo=script node needs the live scenario host.
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// plain map\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=One render\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=2,0,2,2\nMapHeight=1,0,1,1\nMapZoom=5\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "map Plain { seed=1; mat=Earth; tex=Rough; sub=0; };\n",
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        crate::map_creator_s2::S2_MAP_RENDER_COUNT.with(|count| count.set(0));
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario = load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        crate::map_creator_s2::S2_MAP_RENDER_COUNT.with(|count| assert_eq!(count.get(), 1));
    }

    #[test]
    fn script_algorithm_uses_cpp_truthiness_and_catches_per_pixel_errors() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic fault_calls;\n\
             func ScriptAlgoTruth(x, y, a, b) {\n\
                 if (!fault_calls) fault_calls = 0;\n\
                 if (x == 0) return nil;\n\
                 if (x == 100) return 0;\n\
                 if (x == 200) return false;\n\
                 if (x == 300) return -7;\n\
                 if (x == 400) return \"\";\n\
                 if (x == 500) return [];\n\
                 if (x == 600) { fault_calls++; FatalError(\"pixel\"); }\n\
                 return true;\n\
             }\n",
        );
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=ScriptAlgo truth\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=8,0,8,8\nMapHeight=1,0,1,1\nMapZoom=5\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "map Test { seed=1; wdt=8px; hgt=1px;\n\
               overlay Truth { seed=2; algo=script; mat=Earth; tex=Rough; sub=0; };\n\
             };\n",
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        assert_eq!(
            engine
                .landscape()
                .and_then(Landscape::raster_state)
                .and_then(LandscapeRasterState::map)
                .map(|map| map.indices),
            Some(vec![0, 0, 0, 1, 1, 1, 0, 1]),
            "nil/zero/false are false; negative and allocated empty values are true; errors are false"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("fault_calls")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(1)),
            "the mutation before the caught error persists and later pixels still execute"
        );
    }

    #[test]
    fn s2_map_callbacks_run_after_render_in_cpp_array_and_pixel_order() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic callback_trace;\nstatic callback_calls;\nstatic callback_random;\n\
             static callback_weather;\nstatic callback_draw_result;\nstatic callback_map_result;\n\
             func AddCallback(tag, x, y, zoom) {\n\
                 if (!callback_trace) callback_trace = \"\";\n\
                 callback_trace = Format(\"%s%d,%d,%d,%d;\", callback_trace, tag, x, y, zoom);\n\
                 return 1;\n\
             }\n\
             func OnDraw(x, y, zoom) {\n\
                 if (!callback_calls) {\n\
                     callback_random = Random(1000000);\n\
                     callback_weather = [GetWind(0, 0, true), GetTemperature(), GetClimate(), GetSeason()];\n\
                     callback_draw_result = DrawDefMap(0, 0, 10, 5, \"Named\");\n\
                     callback_map_result = DrawMap(0, 0, 15, 5,\n\
                         \"map Runtime { seed=8; Marked { x=2px; }; };\");\n\
                 }\n\
                 callback_calls++;\n\
                 return AddCallback(1, x, y, zoom);\n\
             }\n\
             func OnEval(x, y, zoom) { return AddCallback(2, x, y, zoom); }\n",
        );
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Map callbacks\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=3,0,3,3\nMapHeight=2,0,2,2\nMapZoom=5,0,5,5\nKeepMapCreator=0\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "overlay Marked { x=1px; y=0px; wdt=1px; hgt=1px; seed=2;\n\
                              mat=Earth; tex=Rough; sub=0;\n\
                              drawFn=OnDraw; evalFn=OnEval; };\n\
             map Named { seed=9; Marked; };\n\
             map Test { seed=1; mat=Earth; tex=Rough; sub=0;\n\
               Marked { x=0px; y=0px; wdt=2px; hgt=2px; };\n\
               overlay { x=1px; y=0px; wdt=1px; hgt=1px; seed=3;\n\
                         mat=Earth; tex=Rough; sub=0; };\n\
             };\n",
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("callback_trace")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::String(
                "1,3,3,5;1,-2,3,5;1,8,-2,5;1,3,-2,5;1,-2,-2,5;\
                 2,3,3,5;2,-2,3,5;2,8,-2,5;2,3,-2,5;2,-2,-2,5;"
                    .into()
            )),
            "drawFn's final-writer mask runs before evalFn in descending index order"
        );
        let mut replay = crate::rng::LcgRng::seed_from_u64(0);
        LegacyC4SVal::new(100, 0, 10, 200).evaluate(&mut replay);
        LegacyC4SVal::new(50, 30, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(50, 0, 0, 100).evaluate(&mut replay);
        let expected_callback_random = replay.random(1_000_000);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("callback_random")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(expected_callback_random)),
            "PostInitMap runs after Gravity and placements but before Weather.Init"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("callback_weather")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Array(vec![
                clonk_script::Value::Int(0);
                4
            ])),
            "PostInitMap still sees C4Weather::Default"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("callback_draw_result")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(1)),
            "the creator remains available to DrawDefMap until callbacks finish"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("callback_map_result")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(1)),
            "retained-template DrawMap also updates the live callback masks"
        );
        assert!(
            engine
                .landscape()
                .and_then(Landscape::raster_state)
                .and_then(LandscapeRasterState::map_creator)
                .is_none(),
            "callbacks survive to PostInitMap without KeepMapCreator"
        );

        let mut restore_engine = Engine::with_seed(0);
        scenario
            .apply_before_players_for_restore(&mut restore_engine).test_value();
        assert_eq!(
            restore_engine
                .script_globals
                .borrow()
                .get("callback_trace")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Nil),
            "save restoration must not replay original-scenario PostInitMap callbacks"
        );

        let scenario_core =
            std::fs::read_to_string(scenario_dir.join("Scenario.txt")).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            scenario_core.replace("Title=Map callbacks", "Title=Map callbacks\nNoInitialize=1"),
        );
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("callback_trace")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Nil),
            "NoInitialize skips PostInitMap together with environment placement"
        );
    }

    #[test]
    fn s2_callback_lookup_resolves_append_before_scenario_include() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some((
                "APPN",
                "#appendto GOOD\n\
                 func LinkedDraw(x, y, zoom) {\n\
                     linked_trace = Format(\"%d,%d,%d\", x, y, zoom);\n\
                     return 1;\n\
                 }\n",
            )),
            "#strict\n#include GOOD\nstatic linked_trace;\n",
        );
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Linked callback\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=1,0,1,1\nMapHeight=1,0,1,1\nMapZoom=5,0,5,5\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "map Test { seed=1; mat=Earth; tex=Rough; sub=0; drawFn=LinkedDraw; };",
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("linked_trace")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::String("-2,-2,5".into()))
        );
    }

    #[test]
    fn inactive_section_objects_reparse_source_and_frozen_groups_with_the_live_string_table() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        write_test_file(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict 2\nlocal probe;\n",
        );
        write_test_file(scenario_dir.join("Strings.txt"), "startup-only\r\n");
        let section_dir = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section_dir).test_value();
        let write_objects = |x: i32| {
            write_test_file(
                section_dir.join("Objects.txt"),
                format!(
                    "[Object]\nid=GOOD\nNumber=500\nStatus=1\nX={x}\nY=9\n\
                     LocalNamed=1;probe=S0\n"
                ),
            );
        };
        write_objects(10);

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        assert!(
            scenario
                .scenario_sections
                .iter()
                .find(|section| section.name.eq_ignore_ascii_case("next"))
                .expect("inactive section is discovered")
                .objects
                .is_empty(),
            "inactive Objects.txt must remain uncompiled during startup"
        );

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        engine.set_legacy_string_table(HashMap::from([(0, "first-activation-only".to_string())]));
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("first source activation succeeds"));
        let first = engine
            .object_snapshot(ObjectId::new(500)).test_value();
        assert_eq!(first.position.x, 10);
        assert_eq!(
            first.local_vars.get("probe"),
            Some(&clonk_script::Value::String("first-activation-only".into()))
        );

        assert!(engine
            .load_scenario_section("Main", 0, Vec::new())
            .expect("main reloads"));
        write_objects(77);
        engine.set_legacy_string_table(HashMap::from([(0, "second-activation-only".to_string())]));
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("second source activation succeeds"));
        let second = engine
            .object_snapshot(ObjectId::new(500)).test_value();
        assert_eq!(second.position.x, 77, "source Objects.txt is reparsed");
        assert_eq!(
            second.local_vars.get("probe"),
            Some(&clonk_script::Value::String(
                "second-activation-only".into()
            ))
        );
        drop(first);
        drop(second);

        assert!(engine
            .load_scenario_section("Main", 2, Vec::new())
            .expect("departing objects freeze"));
        assert!(
            !clonk_script::c4_string_registration_order(&engine.script_string_registrations)
                .iter()
                .any(|value| value == "second-activation-only"),
            "the raw frozen group must not retain structured C4String handles"
        );
        let frozen = engine
            .scenario_sections
            .get("next")
            .and_then(|section| section.frozen_group.clone()).test_value();
        assert!(
            engine
                .scenario_sections
                .get("next")
                .is_some_and(|section| section.saved_objects.is_none()),
            "serializer scratch snapshots are discarded after freezing"
        );
        let frozen_group = Group::from_raw_memory(PathBuf::from("SectNext.c4g"), frozen).test_value();
        let frozen_objects = String::from_utf8(
            frozen_group
                .read_file("Objects.txt")
                .expect("frozen Objects.txt exists"),
        ).test_value();
        let enum_id = frozen_objects
            .split_once("probe=S")
            .map(|(_, suffix)| {
                suffix
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse::<i32>()
                    .expect("frozen probe ID parses")
            }).test_value();

        // The source has changed again, but the temporary group is now the
        // authoritative section. Its S# value still resolves through the
        // current global table rather than its stale child Strings.txt.
        write_objects(99);
        engine.set_legacy_string_table(HashMap::from([(
            enum_id,
            "frozen-activation-only".to_string(),
        )]));
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("frozen activation succeeds"));
        let frozen = engine
            .object_snapshot(ObjectId::new(500)).test_value();
        assert_eq!(frozen.position.x, 77, "frozen group wins over source edits");
        assert_eq!(
            frozen.local_vars.get("probe"),
            Some(&clonk_script::Value::String(
                "frozen-activation-only".into()
            ))
        );
    }

    #[test]
    fn scenario_section_switch_installs_reset_compiler_defaults() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\n\
             Title=Main title\n\
             Version=4,6,4\n\
             MaxPlayer=7\n\
             SaveGame=0\n\
             \n\
             [Definitions]\n\
             Definition1=Defs.c4d\n\
             \n\
             [Game]\n\
             Goals=MELE=1\n\
             StructNeedEnergy=0\n\
             ValueOverloads=FISH=20\n\
             \n\
             [Landscape]\n\
             MapWidth=2,0,2,2\n\
             MapHeight=1,0,1,1\n\
             MapZoom=5,0,5,5\n\
             ShadeMaterials=1\n\
             \n\
             [Weather]\n\
             Wind=100,0,-100,100\n",
        );
        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(
            section.join("Scenario.txt"),
            "[Head]\n\
             Title=Ignored title\n\
             Version=9,9,9\n\
             MaxPlayer=99\n\
             SaveGame=1\n\
             \n\
             [Definitions]\n\
             Definition1=Ignored.c4d\n\
             \n\
             [Game]\n\
             ValueOverloads=ROCK=99\n\
             \n\
             [Landscape]\n\
             Sky=Clouds\n",
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert!(engine.scenario_values.is_melee());

        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("section load succeeds"));
        let values = engine.scenario_values.as_ref();
        assert!(!values.is_melee());
        assert_eq!(values.get("Goals", Some("Game"), 0), None);
        assert_eq!(
            values.get("Rules", Some("Game"), 0),
            Some(&ScenarioValue::C4Id("ENRG".to_string()))
        );
        assert_eq!(
            values.get("Wind", Some("Weather"), 0),
            Some(&ScenarioValue::Int(0))
        );
        assert_eq!(
            values.get("Wind", Some("Weather"), 1),
            Some(&ScenarioValue::Int(70))
        );
        assert_eq!(
            values.get("ShadeMaterials", Some("Landscape"), 0),
            Some(&ScenarioValue::Bool(false))
        );
        assert_eq!(
            values.get("Title", Some("Head"), 0),
            Some(&ScenarioValue::String("Main title".to_string()))
        );
        assert_eq!(
            values.get("MaxPlayer", Some("Head"), 0),
            Some(&ScenarioValue::Int(7))
        );
        assert_eq!(
            values.get("Definitions", Some("Definitions"), 0),
            Some(&ScenarioValue::String("Defs.c4d".to_string()))
        );
        assert_eq!(
            values.get("ValueOverloads", Some("Game"), 0),
            Some(&ScenarioValue::C4Id("FISH".to_string()))
        );
    }

    #[test]
    fn exact_save_binds_root_to_current_section_and_retains_departed_main() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Current cave\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Weather]\nWind=11,0,11,11\n",
        );
        write_test_file(
            scenario_dir.join("Game.txt"),
            "[Game]\nCurrentScenarioSection=Cave\n",
        );

        let departed_main = scenario_dir.join("SectMain.c4g");
        std::fs::create_dir_all(&departed_main).test_value();
        write_test_file(
            departed_main.join("Scenario.txt"),
            "[Weather]\nWind=22,0,22,22\n",
        );

        // SaveScenarioSections deletes the current child. If a malformed or
        // hand-edited group retains one anyway, root state still wins.
        let stale_cave = scenario_dir.join("SectCave.c4g");
        std::fs::create_dir_all(&stale_cave).test_value();
        write_test_file(
            stale_cave.join("Scenario.txt"),
            "[Weather]\nWind=99,0,99,99\n",
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        assert_eq!(
            scenario
                .scenario_sections
                .iter()
                .map(|section| section.name.to_ascii_lowercase())
                .collect::<Vec<_>>(),
            vec!["cave", "main"],
            "root is current Cave, SectMain remains an inactive section"
        );

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        // This fixture has no map resources, so install the real Surface8
        // that every native running landscape owns before exercising
        // C4S_SAVE_LANDSCAPE on both departures.
        let mut surface8 = Landscape::flat(100, 100);
        surface8.set_pixel_grid(crate::landscape::PixelGrid::new(
            100,
            100,
            vec![0; 10_000],
            vec![0],
            vec![None],
            vec![None],
        ));
        engine.set_landscape(surface8);
        assert_eq!(engine.debug_current_scenario_section(), "Cave");
        assert_eq!(
            engine.scenario_values.get("Wind", Some("Weather"), 0),
            Some(&ScenarioValue::Int(11)),
            "stale SectCave must not replace root Cave"
        );

        assert!(engine
            .load_scenario_section("Main", 3, Vec::new())
            .expect("departed Main loads"));
        assert_eq!(
            engine.scenario_values.get("Wind", Some("Weather"), 0),
            Some(&ScenarioValue::Int(22))
        );
        assert!(engine
            .load_scenario_section("Cave", 3, Vec::new())
            .expect("root Cave reloads"));
        assert_eq!(
            engine.scenario_values.get("Wind", Some("Weather"), 0),
            Some(&ScenarioValue::Int(11))
        );
    }

    #[test]
    fn scenario_section_executes_its_own_post_init_map_callbacks() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic section_trace;\n\
             func OnSection(x, y, zoom) {\n\
                 section_trace = Format(\"%d,%d,%d\", x, y, zoom);\n\
                 return 1;\n\
             }\n",
        );
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Section callback\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=1,0,1,1\nMapHeight=1,0,1,1\nMapZoom=5,0,5,5\nKeepMapCreator=1\n",
        );
        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(
            section.join("Scenario.txt"),
            "[Head]\nTitle=Next\n\n\
             [Landscape]\nMapWidth=1,0,1,1\nMapHeight=1,0,1,1\n\
             MapZoom=5,0,5,5\nKeepMapCreator=1\n",
        );
        write_test_file(
            section.join("Landscape.txt"),
            "map Next { seed=1; mat=Earth; tex=Rough; sub=0; drawFn=OnSection; };",
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("section load succeeds"));

        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("section_trace")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::String("-2,-2,5".into()))
        );
    }

    #[test]
    fn coreless_section_keeps_main_core_and_switches_landscape() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Coreless section\n\n\
             [Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nExactLandscape=1\nNewStyleLandscape=2\nGravity=137,0,137,137\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[0, 0]]),
        );

        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(
            section.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[0, 0, 0]]),
        );
        assert!(!section.join("Scenario.txt").exists());

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert_eq!(engine.landscape().map(Landscape::width), Some(2));
        assert_eq!(engine.physics().gravity, 137);

        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("coreless section switch succeeds"));
        assert_eq!(
            engine.landscape().map(Landscape::width),
            Some(3),
            "the inherited ExactLandscape core loads the section bitmap at pixel scale"
        );
        assert_eq!(
            engine.physics().gravity,
            137,
            "a missing section core leaves the inherited gravity unchanged"
        );
    }

    #[test]
    fn main_scenario_still_requires_scenario_core() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(section.join("Scenario.txt"), "[Head]\nTitle=Next\n");
        std::fs::remove_file(scenario_dir.join("Scenario.txt")).test_value();

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let error = match Scenario::load_from_path_with(&scenario_dir, &resolver) {
            Ok(_) => panic!("main scenario unexpectedly loaded without Scenario.txt"),
            Err(error) => error,
        };
        assert!(matches!(error, ScenarioError::LegacyCoreMissing));
    }

    #[test]
    fn invalid_section_name_lengths_abort_scenario_load() {
        for filename in [
            "Sect.c4g".to_string(),
            format!("Sect{}.c4g", "x".repeat(31)),
        ] {
            let dir = test_tempdir();
            let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
            std::fs::create_dir_all(scenario_dir.join(&filename)).test_value();
            let resolver = test_resolver(vec![dir.path().to_path_buf()]);
            let error = match Scenario::load_from_path_with(&scenario_dir, &resolver) {
                Ok(_) => panic!("scenario unexpectedly accepted section {filename}"),
                Err(error) => error,
            };
            assert!(matches!(
                error,
                ScenarioError::InvalidScenarioSectionName { path }
                    if path == Path::new(&filename)
            ));
        }

        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        let valid = format!("Sect{}.c4g", "x".repeat(30));
        std::fs::create_dir_all(scenario_dir.join(valid)).test_value();
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        load_test_scenario(&scenario_dir, &resolver);
    }

    #[test]
    fn keep_map_creator_section_reuses_main_tree_default_size_and_rng_position() {
        // CreateMapS2 reuses the creator retained by the main landscape:
        // ReadFile appends into that tree without reconstructing DefaultMap,
        // so the section can resolve main-file templates and takes no new
        // MapWdt/MapHgt draws (src/C4Landscape.cpp:531-546;
        // src/C4MapCreatorS2.cpp:633-644,741-751).
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Retained section map\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=2,0,2,2\nMapHeight=1,0,1,1\n\
             MapZoom=5,0,5,5\nKeepMapCreator=1\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "overlay Shared { mat=Earth; tex=Rough; sub=0; seed=7; }; \
             map Main { seed=11; };",
        );

        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(
            section.join("Scenario.txt"),
            "[Head]\nTitle=Next\n\n[Landscape]\nMapWidth=4,0,4,4\n\
             MapHeight=3,0,3,3\nMapZoom=51,50,1,101\n",
        );
        write_test_file(
            section.join("Landscape.txt"),
            "map Next { seed=13; Shared; };",
        );

        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario = Scenario::load_from_path_with_languages_and_seed_and_startup_player_count(
            &scenario_dir,
            &resolver,
            &["US"],
            0,
            1,
        ).test_value();
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("section load succeeds"));
        let raster = engine
            .landscape()
            .and_then(Landscape::raster_state).test_value();
        let map = raster.map().test_value();

        assert_eq!(
            (map.width, map.height),
            (2, 1),
            "section map clones the main creator's evaluated DefaultMap"
        );
        assert_eq!(
            map.indices,
            vec![1, 1],
            "section resolves the Shared overlay declared in the main file"
        );
        assert_eq!(
            raster.map_zoom(),
            1,
            "MapZoom follows the first section draw, with no fresh size draws"
        );
    }

    #[test]
    fn section_without_retained_creator_keeps_fresh_size_draws_and_tree() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Fresh section map\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=2,0,2,2\nMapHeight=1,0,1,1\n\
             MapZoom=5,0,5,5\nKeepMapCreator=0\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "overlay MainOnly { seed=7; }; map Main { seed=11; };",
        );

        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(
            section.join("Scenario.txt"),
            "[Head]\nTitle=Next\n\n[Landscape]\nMapWidth=4,0,4,4\n\
             MapHeight=3,0,3,3\nMapZoom=51,50,1,101\n",
        );
        write_test_file(
            section.join("Landscape.txt"),
            "map Next { mat=Earth; tex=Rough; sub=0; seed=13; };",
        );

        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario = Scenario::load_from_path_with_languages_and_seed_and_startup_player_count(
            &scenario_dir,
            &resolver,
            &["US"],
            0,
            1,
        ).test_value();
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("section load succeeds"));
        let raster = engine
            .landscape()
            .and_then(Landscape::raster_state).test_value();
        let map = raster.map().test_value();
        assert_eq!((map.width, map.height), (4, 3));
        assert_eq!(map.indices, vec![1; 12]);
        assert_eq!(
            raster.map_zoom(),
            44,
            "fresh creator consumes MapWdt/MapHgt before MapZoom"
        );
        assert!(
            engine
                .landscape()
                .and_then(Landscape::raster_state)
                .and_then(LandscapeRasterState::map_creator)
                .is_none(),
            "KeepMapCreator=0 discards the fresh section creator after PostInitMap"
        );
    }

    #[test]
    fn scenario_section_loads_pxs_and_mass_mover_c4b_components() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "#strict\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Binary section\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=1,0,1,1\nMapHeight=1,0,1,1\nMapZoom=5,0,5,5\n",
        );
        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(section.join("Scenario.txt"), "[Head]\nTitle=Next\n");
        write_test_file(
            section.join("Landscape.txt"),
            "map Next { seed=1; mat=Earth; tex=Rough; };",
        );
        let keep_section = scenario_dir.join("SectKeep.c4g");
        std::fs::create_dir_all(&keep_section).test_value();
        write_test_file(
            keep_section.join("Scenario.txt"),
            "[Head]\nTitle=Keep current landscape\n",
        );
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let mut pxs = vec![0; 4 + crate::pxs::PXS_CHUNK_SIZE * 20];
        pxs[..4].copy_from_slice(&1i32.to_le_bytes());
        for record in pxs[4..].chunks_exact_mut(20) {
            record[..4].copy_from_slice(&(-1i32).to_le_bytes());
        }
        let record = &mut pxs[4 + 3 * 20..4 + 4 * 20];
        for (field, value) in [0i32, 98_304, -147_456, 8_192, -32_768]
            .into_iter()
            .enumerate()
        {
            record[field * 4..field * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        write_test_file(scenario_dir.join("PXS.c4b"), pxs.clone());
        write_test_file(section.join("PXS.c4b"), pxs);
        let mut mover = Vec::new();
        for value in [0i32, 4, 7] {
            mover.extend_from_slice(&value.to_le_bytes());
        }
        write_test_file(scenario_dir.join("MassMover.c4b"), mover.clone());
        write_test_file(section.join("MassMover.c4b"), mover);

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        assert!(
            scenario
                .scenario_sections
                .iter()
                .find(|section| section.name.eq_ignore_ascii_case("keep"))
                .expect("mapless section discovered")
                .landscape
                .is_none(),
            "section core values alone do not set C++ LandscapeLoaded"
        );
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert!(engine.pxs_system.peek_slot(0, 3).is_some());
        assert_eq!(
            engine.mass_movers.slot(0).map(|mover| (mover.x, mover.y)),
            Some((4, 7))
        );
        engine.pxs_system.note_executed();
        assert!(engine
            .load_scenario_section("Keep", 0, Vec::new())
            .expect("mapless section load succeeds"));
        assert!(engine.pxs_system.peek_slot(0, 3).is_some());
        assert_eq!(
            engine.mass_movers.slot(0).map(|mover| (mover.x, mover.y)),
            Some((4, 7))
        );
        assert_eq!(engine.pxs_system.execute_count(), 1);
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("section load succeeds"));

        let pixel = engine
            .pxs_system
            .peek_slot(0, 3).test_value();
        assert_eq!(pixel.mat.raw(), 0);
        assert_eq!(
            [
                pixel.x.val(),
                pixel.y.val(),
                pixel.xdir.val(),
                pixel.ydir.val()
            ],
            [98_304, -147_456, 8_192, -32_768]
        );
        let mover = engine.mass_movers.slot(0).test_value();
        assert_eq!((mover.mat.index(), mover.x, mover.y), (0, 4, 7));
        assert_eq!(engine.mass_movers.count(), 1);
        assert_eq!(engine.mass_movers.create_ptr(), 0);
        assert_eq!(
            engine.pxs_system.execute_count(),
            1,
            "PXS Load leaves Count intact"
        );
    }

    #[test]
    fn scenario_section_s2_reuses_retained_main_template_and_default_map() {
        // CreateMapS2 appends a section Landscape.txt to the live creator.
        // Consequently the section can resolve a main-section template and
        // its new map keeps the creator's construction-time dimensions,
        // rather than evaluating the section's MapWidth/MapHeight values.
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Retained section creator\n\n\
             [Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\n\
             MapWidth=8,0,8,8\n\
             MapHeight=1,0,1,1\n\
             MapZoom=5,0,5,5\n\
             Gravity=100,0,100,100\n\
             AutoScanSideOpen=0\n\
             KeepMapCreator=1\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "overlay RetainedBand { mat=Earth; tex=Rough; sub=0; \
             wdt=50; hgt=100; seed=7; }; \
             map Main { seed=11; RetainedBand; };",
        );

        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(
            section.join("Scenario.txt"),
            "[Head]\nTitle=Next\n\n\
             [Landscape]\n\
             MapWidth=3,0,3,3\n\
             MapHeight=4,0,4,4\n\
             MapZoom=5,0,5,5\n\
             KeepMapCreator=1\n",
        );
        write_test_file(
            section.join("Landscape.txt"),
            "map Next { seed=13; RetainedBand { x=50; }; };",
        );

        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\nShape=0\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        let main_raster = engine
            .landscape()
            .and_then(Landscape::raster_state).test_value();
        let main_map = main_raster.map().test_value();
        assert_eq!((main_map.width, main_map.height), (8, 1));
        assert_eq!(main_map.indices, vec![1, 1, 1, 1, 0, 0, 0, 0]);
        assert!(
            main_raster.map_creator().is_some(),
            "main KeepMapCreator retains the creator for section overloads"
        );

        engine.rng.random(17);
        engine.rng.rnd3();
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("section load succeeds"));

        let landscape = engine.landscape().test_value();
        let raster = landscape.raster_state().test_value();
        let map = raster.map().test_value();
        assert_eq!(
            (map.width, map.height),
            (8, 1),
            "the retained DefaultMap wins over section dimensions 3x4"
        );
        assert_eq!(map.indices, vec![0, 0, 0, 0, 1, 1, 1, 1]);
        assert_eq!(
            (landscape.width(), landscape.estimated_height()),
            (100, 100)
        );
        assert_eq!(landscape.grid_byte_at(19, 2), Some(0));
        assert_eq!(landscape.grid_byte_at(20, 2), Some(1));
        assert_eq!(landscape.grid_byte_at(39, 2), Some(1));
        assert_eq!(landscape.grid_byte_at(40, 2), Some(0));
        let mut retained_creator = raster
            .map_creator().test_value()
            .clone();
        let mut classifier = MapPixelClassifier::from_runtime_state(raster.texmap().clone());
        let mut probe_rng = crate::rng::LcgRng::seed_from_u64(41);
        let probe_count = probe_rng.count;
        let retained_main = crate::map_creator_s2::render_named_s2_map_with_script_algo(
            &mut retained_creator,
            "Main",
            &mut classifier,
            8,
            1,
            &mut probe_rng,
            &mut crate::map_creator_s2::noop_script_algo,
        )
        .test_value();
        assert_eq!(retained_main.indices, vec![1, 1, 1, 1, 0, 0, 0, 0]);
        let appended_next = crate::map_creator_s2::render_named_s2_map_with_script_algo(
            &mut retained_creator,
            "Next",
            &mut classifier,
            8,
            1,
            &mut probe_rng,
            &mut crate::map_creator_s2::noop_script_algo,
        )
        .test_value();
        assert_eq!(appended_next.indices, vec![0, 0, 0, 0, 1, 1, 1, 1]);
        assert_eq!(
            probe_rng.count, probe_count,
            "fixed retained nodes consume no RNG when re-evaluated"
        );

        let mut expected = crate::rng::LcgRng::seed_from_u64(0);
        let _ = expected.random(1);
        expected.trace_index = engine.rng.trace_index;
        assert_eq!(engine.rng.count, 501);
        assert_eq!(engine.rng.rnd3_ptr(), 0);
        assert_eq!(
            engine.rng, expected,
            "the second FixRandom hides map creation and gravity consumes one draw"
        );
    }

    #[test]
    fn scenario_section_script_algorithm_uses_live_host_and_preserves_globals() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic section_calls;\n\
             func ScriptAlgoLive(x, y, a, b) {\n\
                 if (!section_calls) section_calls = 0;\n\
                 section_calls++;\n\
                 Random(7);\n\
                 return y == 0 && a == 5 && b == 9\n\
                     && x == (section_calls - 1) * 100;\n\
             }\n",
        );
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Section ScriptAlgo\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=2,0,2,2\nMapHeight=1,0,1,1\n\
             MapZoom=5,0,5,5\nKeepMapCreator=1\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.txt"),
            "map Main { seed=1; wdt=2px; hgt=1px; };\n",
        );

        let section = scenario_dir.join("SectNext.c4g");
        std::fs::create_dir_all(&section).test_value();
        write_test_file(
            section.join("Scenario.txt"),
            "[Head]\nTitle=Next\n\n[Landscape]\nMapZoom=5,0,5,5\nKeepMapCreator=1\n",
        );
        write_test_file(
            section.join("Landscape.txt"),
            "map Next { seed=2; wdt=2px; hgt=1px;\n\
               overlay Live { seed=3; algo=script; a=5; b=9;\n\
                              mat=Earth; tex=Rough; sub=0; };\n\
             };\n",
        );

        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).test_value();
        write_test_file(materials.join("TexMap.txt"), "1=Earth-Rough\n");
        write_test_file(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        );
        write_test_texture(&materials, "Rough");

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert!(engine
            .load_scenario_section("Next", 0, Vec::new())
            .expect("section load succeeds"));

        assert_eq!(
            engine
                .landscape()
                .and_then(Landscape::raster_state)
                .and_then(LandscapeRasterState::map)
                .map(|map| map.indices),
            Some(vec![1, 1]),
            "the section map executes ScriptAlgoLive in row-major order"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("section_calls")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(2)),
            "section restore keeps globals mutated by live ScriptAlgo calls"
        );
    }

    #[test]
    fn ancestor_texmap_overloads_global_water_material() {
        // C4Game::InitMaterialTexture walks the ordered NRT_Material chain
        // while each source's OverloadMaterials/OverloadTextures flags admit
        // the next one (C4Game.cpp:901-977). C4MaterialMap::Load prepends new
        // names while earlier sources win collisions (C4Material.cpp:263-299).
        // Hazard/CTF_DeepSea has this exact shape: its enclosing package owns
        // TexMap slot 3=Water-Liquid but only the final global Material.c4g
        // supplies Water.c4m. Missing that second source turns map byte 0x83
        // into sky, so C4Game::PlaceAnimal cannot place either shark.
        let dir = test_tempdir();

        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).test_value();
        write_test_file(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        );
        write_test_definition_graphics(&good);

        let package = dir.path().join("Pack.c4f");
        let scenario_dir = package.join("Deep.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Deep\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapZoom=10\n",
        );
        write_test_file(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[0x83, 0x83], &[0x83, 0x83]]),
        );

        let package_materials = package.join("Material.c4g");
        std::fs::create_dir_all(&package_materials).test_value();
        write_test_file(
            package_materials.join("TexMap.txt"),
            "OverloadMaterials\nOverloadTextures\n3=Water-Liquid\n4=PackStone-Smooth\n",
        );
        // Keep this source non-empty: reaching the global Water material must
        // be caused by OverloadMaterials, not the C++ zero-material fallback.
        write_test_file(
            package_materials.join("PackStone.c4m"),
            "[Material]\nName=PackStone\nDensity=100\n",
        );
        write_test_texture(&package_materials, "Liquid");
        write_test_texture(&package_materials, "Smooth");

        let global_materials = dir.path().join("Material.c4g");
        std::fs::create_dir_all(&global_materials).test_value();
        // Later C++ material resources must carry TexMap.txt so LoadFlags can
        // admit their contents; this table is not used for slot mappings.
        write_test_file(global_materials.join("TexMap.txt"), "# global table\n");
        write_test_file(
            global_materials.join("Water.c4m"),
            "[Material]\nName=Water\nDensity=25\n",
        );

        // Resolver order mirrors DeepSea: enclosing package, then content
        // root. There is deliberately no scenario-local Material.c4g.
        let resolver = test_resolver(vec![package, dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let landscape = engine.landscape().test_value();

        assert_eq!(
            landscape.grid_byte_at(5, 5),
            Some(0x83),
            "the package TexMap byte survives with its IFT bit"
        );
        assert!(
            landscape.is_liquid_at(5, 5),
            "global Water.c4m supplies slot 3 density through OverloadMaterials"
        );
        assert!(landscape.is_ift_at(5, 5));
    }

    #[test]
    fn same_group_duplicate_materials_retain_slots_across_overload_chain() {
        let dir = test_tempdir();
        let scenario_dir = dir.path().join("DuplicateMaterials.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        // C4MaterialMap::Load consumes the packed C4Group entry order
        // (src/C4Material.cpp:242-276). Keep A, B, C in that physical order
        // instead of inheriting host std::fs::read_dir order from a directory.
        let mut local = clonk_resources::MutableGroup::new("Material.c4g");
        local
            .add_file(
                "TexMap.txt",
                b"OverloadMaterials\nOverloadTextures\n".to_vec(),
            ).test_value();
        for (file, name, density, overlay) in [
            ("A.c4m", "Dup", 10, "Rough"),
            ("B.c4m", "dUp", 20, "Smooth"),
            ("C.c4m", "LocalOnly", 30, "Rough"),
        ] {
            local
                .add_file(
                    file,
                    format!(
                        "[Material]\nName={name}\nDensity={density}\nTextureOverlay={overlay}\n"
                    )
                    .into_bytes(),
                ).test_value();
        }
        for texture in ["Rough", "Smooth"] {
            local
                .add_file(
                    format!("{texture}.bmp"),
                    encode_indexed_bmp(&[&[0u8]]),
                ).test_value();
        }
        write_test_file(
            scenario_dir.join("Material.c4g"),
            local.pack().test_value(),
        );

        let installed_root = dir.path().join("Installed");
        std::fs::create_dir_all(&installed_root).test_value();
        let mut installed = clonk_resources::MutableGroup::new("Material.c4g");
        installed
            .add_file("TexMap.txt", b"# installed\n".to_vec()).test_value();
        installed
            .add_file(
                "Global.c4m",
                b"[Material]\nName=Global\nDensity=40\nTextureOverlay=Smooth\n".to_vec(),
            ).test_value();
        write_test_file(
            installed_root.join("Material.c4g"),
            installed.pack().test_value(),
        );

        let group = Group::open(&scenario_dir).test_value();
        let resolver = test_resolver(vec![installed_root]);
        let classifier = build_map_pixel_classifier(&group, &resolver)
            .expect("classifier load succeeds").test_value();
        let library = classifier.material_library().test_value();
        assert_eq!(
            library
                .iter()
                .map(|material| material.name())
                .collect::<Vec<_>>(),
            vec!["Global", "Dup", "dUp", "LocalOnly"]
        );
        assert_eq!(
            library
                .get("dup")
                .and_then(|material| material.int("density")),
            Some(10),
            "name lookup resolves the first same-load duplicate"
        );

        let materials = crate::MaterialSet::from_resource_library(library);
        assert_eq!(materials.len(), 4, "duplicates retain numeric slots");
        assert_eq!(materials.id_of("DUP").map(|id| id.index()), Some(1));
        assert_eq!(
            materials
                .iter()
                .map(|material| material.name())
                .collect::<Vec<_>>(),
            vec!["Global", "Dup", "dUp", "LocalOnly"]
        );

        let duplicate_defaults = classifier
            .state
            .default_material_entries
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("dup"))
            .map(|(_, slot)| *slot)
            .collect::<Vec<_>>();
        assert_eq!(duplicate_defaults.len(), 2);
        assert_eq!(
            classifier.state.default_material_entry("dup"),
            Some(duplicate_defaults[0])
        );
        assert_eq!(
            classifier.state.default_material_entry_by_index(1),
            Some(duplicate_defaults[0])
        );
        assert_eq!(
            classifier.state.default_material_entry_by_index(2),
            Some(duplicate_defaults[1]),
            "numeric material lookup keeps a later same-name material's own DefaultMatTex"
        );
        assert_eq!(
            classifier.state.match_texture_names[usize::from(duplicate_defaults[0])].as_deref(),
            Some("Rough")
        );
        assert_eq!(
            classifier.state.match_texture_names[usize::from(duplicate_defaults[1])].as_deref(),
            Some("Smooth")
        );
    }
