// Contiguous slice 8 of 8 of the `scenario/tests` battery, spliced
// by `include!` from the parent module so every test id is unchanged.

    #[test]
    fn scenario_local_material_reaction_overrides_installed_material_like_cpp() {
        // Tutorial10's local FlyAshes.c4m adds a Convert/SemiSolid→Sky
        // reaction so volcanic ash vanishes on DuroLava instead of sinking
        // beneath it and isolating the pump's source endpoint. C++ loads the
        // scenario material group before the installed group; the first
        // definition of a duplicate material name wins (C4Game.cpp:901-977,
        // C4Material.cpp:263-299).
        let dir = tempdir().expect("tempdir");

        let definition = dir.path().join("Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&definition).expect("definition dir");
        std::fs::write(
            definition.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write definition");
        write_test_definition_graphics(&definition);

        let scenario_dir = dir.path().join("Tutorial10.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Tutorial10 material override\n\n\
             [Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapZoom=10\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[24, 59], &[24, 59]]),
        )
        .expect("write landscape");

        let local_materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&local_materials).expect("local materials");
        std::fs::write(
            local_materials.join("TexMap.txt"),
            "OverloadMaterials\nOverloadTextures\n24=DuroLava-Liquid\n59=FlyAshes-Smooth\n",
        )
        .expect("write local texmap");
        std::fs::write(
            local_materials.join("FlyAshes.c4m"),
            "[Material]\nName=FlyAshes\nDensity=50\n\n\
             [Reaction]\nType=Convert\nTargetSpec=SemiSolid\nExecMask=-1\nConvertMat=Sky\n",
        )
        .expect("write local FlyAshes override");

        let installed_materials = dir.path().join("Material.c4g");
        std::fs::create_dir_all(&installed_materials).expect("installed materials");
        std::fs::write(
            installed_materials.join("TexMap.txt"),
            "# installed table\n",
        )
        .expect("write installed texmap");
        std::fs::write(
            installed_materials.join("FlyAshes.c4m"),
            "[Material]\nName=FlyAshes\nDensity=50\n",
        )
        .expect("write installed FlyAshes");
        std::fs::write(
            installed_materials.join("DuroLava.c4m"),
            "[Material]\nName=DuroLava\nDensity=25\n",
        )
        .expect("write installed DuroLava");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let installed_group = Group::open(&installed_materials).expect("installed group opens");
        let installed_library = clonk_resources::MaterialLibrary::from_group(&installed_group)
            .expect("installed materials load");
        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&installed_library);
        scenario.apply(&mut engine).expect("scenario applies");

        let fly_ashes = engine
            .materials()
            .id_of("FlyAshes")
            .expect("FlyAshes is loaded");
        let duro_lava = engine
            .materials()
            .id_of("DuroLava")
            .expect("DuroLava is loaded from the admitted installed group");
        let reaction = engine
            .materials()
            .reaction(Some(fly_ashes), Some(duro_lava));
        assert!(
            reaction.user_defined,
            "the local reaction overrides defaults"
        );
        assert_eq!(
            reaction.kind,
            crate::material::MaterialReactionKind::Convert {
                target: None,
                depth: None,
            },
            "Tutorial10 FlyAshes converts to Sky on semi-solid DuroLava"
        );
    }

    #[test]
    fn legacy_scenario_threads_border_open_keys_into_the_landscape() {
        // C4Landscape::ScenarioInit (C4Landscape.cpp:67-73) copies the
        // Scenario.txt LeftOpen/RightOpen/TopOpen/BottomOpen keys onto the
        // landscape; AutoScanSideOpen=0 keeps the explicit side values.
        let dir = tempdir().expect("tempdir");
        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(good.join("Script.c"), "// fine\n").expect("write script");
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Borders.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Borders\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapZoom=10\nTopOpen=0\nBottomOpen=1\nLeftOpen=7\nRightOpen=9\nAutoScanSideOpen=0\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 0, 0, 0],
                &[0, 0, 0, 0],
                &[30, 30, 30, 0],
                &[30, 30, 30, 30],
            ]),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "# table\n30=Earth-Smooth\n")
            .expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let landscape = engine.landscape().expect("landscape loaded");
        assert_eq!(landscape.left_open(), 7);
        assert_eq!(landscape.right_open(), 9);
        assert!(!landscape.top_open());
        assert!(landscape.bottom_open());
    }

    #[test]
    fn cpp_runtime_landscape_preseeds_map_and_savegame_fields_override_scenario_init() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Runtime Landscape\nSaveGame=1\n\n\
             [Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapZoom=5\nGravity=120\nTopOpen=1\nBottomOpen=0\nLeftOpen=7\nRightOpen=9\nAutoScanSideOpen=0\n",
        )
        .expect("write save core");
        std::fs::write(
            scenario_dir.join("Game.txt"),
            "[Landscape]\r\nMapSeed=-7\r\nLeftOpen=-1\r\nRightOpen=42\r\nBottomOpen=true\r\nMatModulation=4278255360\r\n",
        )
        .expect("write C++ runtime block");
        std::fs::write(
            scenario_dir.join("Map.bmp"),
            encode_indexed_bmp(&[&[0, 0, 0], &[30, 30, 30]]),
        )
        .expect("write static map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "30=Earth-Smooth\n").expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nColor=1,2,3,4,5,6,7,8,9\nDensity=100\nShape=2\n",
        )
        .expect("write earth");
        write_test_texture(&materials, "Smooth");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("savegame loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("savegame applies");

        let landscape = engine.landscape().expect("landscape restored");
        assert_eq!(landscape.map_seed(), -7);
        assert_eq!(landscape.left_open(), -1);
        assert_eq!(landscape.right_open(), 42);
        assert!(
            !landscape.top_open(),
            "omitted runtime TopOpen defaults false"
        );
        assert!(landscape.bottom_open());
        assert_eq!(landscape.modulation(), 4_278_255_360);
        assert_eq!(
            landscape.mode(),
            LANDSCAPE_MODE_STATIC,
            "omitted runtime Mode is inferred during Init"
        );
        let mut shapes = vec![None; 128];
        shapes[30] = Some(crate::chunky::ChunkShape::Smooth);
        let indices = [0, 0, 0, 30, 30, 30];
        let expected =
            crate::chunky::synthesize_landscape(&indices, 3, 2, 5, -7, &shapes).into_bytes();
        let generated_seed = map_seed_from_random_seed(0);
        let wrong = crate::chunky::synthesize_landscape(&indices, 3, 2, 5, generated_seed, &shapes)
            .into_bytes();
        assert_ne!(expected, wrong, "fixture must distinguish the runtime seed");
        let grid = landscape.pixel_grid().expect("static Surface8 restored");
        for row in 0..10 {
            assert_eq!(
                &grid.bytes()[row * 100..row * 100 + 15],
                &expected[row * 15..row * 15 + 15],
                "runtime MapSeed must affect ChunkOZoom before synthesis"
            );
        }
        assert_eq!(
            engine.physics().gravity_raw(),
            crate::network_game_data::LANDSCAPE_DEFAULT_GRAVITY_RAW,
            "omitted runtime Gravity defaults to raw FIXED100(20)"
        );
        assert_eq!(
            engine.physics().gravity,
            100,
            "GetGravity projection restores too"
        );
        let mut edited_physics = engine.physics();
        edited_physics.gravity = 200;
        assert_eq!(
            edited_physics.gravity_raw(),
            (crate::math::fixed100(200) / 5).val(),
            "a copied value immediately honors its edited public projection"
        );
        assert!(
            !serde_json::to_string(&edited_physics)
                .unwrap()
                .contains("gravity_raw"),
            "a stale hidden override is not serialized"
        );
        engine.set_physics(edited_physics);
        assert_eq!(
            engine.physics().gravity_raw(),
            (crate::math::fixed100(200) / 5).val()
        );

        // Initial network dynamics carry exact Game.txt data without turning
        // Scenario.txt into a savegame. MapSeed/Mode/Modulation are compiled
        // before Init, while ScenarioInit still owns gravity and borders.
        let core = std::fs::read_to_string(scenario_dir.join("Scenario.txt")).unwrap();
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            core.replace("SaveGame=1\n", "SaveGame=0\n"),
        )
        .unwrap();
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("initial dynamic loads");
        let mut initial = Engine::with_seed(0);
        scenario
            .apply(&mut initial)
            .expect("initial dynamic applies");
        let landscape = initial.landscape().unwrap();
        assert_eq!(landscape.map_seed(), -7);
        assert_eq!(landscape.mode(), LANDSCAPE_MODE_STATIC);
        assert_eq!(landscape.modulation(), 4_278_255_360);
        assert_eq!((landscape.left_open(), landscape.right_open()), (7, 9));
        assert!(landscape.top_open());
        assert!(!landscape.bottom_open());
        assert_eq!(initial.physics().gravity, 120);
    }

    #[test]
    fn legacy_scenario_auto_scan_side_open_scans_the_border_columns() {
        // AutoScanSideOpen defaults to true (C4Scenario.cpp:297):
        // ScanSideOpen (C4Landscape.cpp:231-238) replaces LeftOpen /
        // RightOpen with the first non-sky pixel of the border columns.
        let dir = tempdir().expect("tempdir");
        let defs_root = dir.path().join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(good.join("Script.c"), "// fine\n").expect("write script");
        write_test_definition_graphics(&good);

        let scenario_dir = dir.path().join("Scan.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Scan\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Landscape]\nMapZoom=10\n",
        )
        .expect("write scenario core");
        // Column 0 turns solid at map row 2 (world y 20); column 3 is all
        // sky (right side fully open through the 100px minimum height).
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[
                &[0, 30, 30, 0],
                &[0, 30, 30, 0],
                &[30, 30, 30, 0],
                &[30, 30, 30, 0],
            ]),
        )
        .expect("write map");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "# table\n30=Earth-Smooth\n")
            .expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");
        write_test_texture(&materials, "Smooth");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        let landscape = engine.landscape().expect("landscape loaded");
        assert_eq!(landscape.left_open(), 20, "first non-sky pixel in column 0");
        assert_eq!(
            landscape.right_open(),
            100,
            "all-sky border column opens the full height"
        );
    }

    #[test]
    fn get_index_mat_tex_returns_cross_mapped_default_not_first_material_slot() {
        // C4MaterialMap::CrossMapMaterials stores the exact overlay entry in
        // DefaultMatTex (C4Material.cpp:349-370), and GetIndexMatTex returns
        // that field after its explicit-texture attempts (C4Texture.cpp:
        // 346-367). An earlier Earth-Ridge slot must not replace the recorded
        // Earth-Smooth default.
        let mut names = vec![None; 128];
        names[4] = Some("Earth".to_string());
        names[30] = Some("Earth".to_string());
        let mut textures = vec![None; 128];
        textures[4] = Some("Ridge".to_string());
        textures[30] = Some("Smooth".to_string());
        let mut classifier =
            MapPixelClassifier::from_slots([0; 128], names, textures, vec![None; 128]);
        classifier.state.set_default_material_entry("Earth", 30);

        let crossmap_entry = classifier.get_index_mat_tex("Earth", None);
        classifier
            .state
            .material_crossmap_entries
            .push(crossmap_entry);
        assert_eq!(crossmap_entry, 30);
        assert_eq!(classifier.state.material_crossmap_entries, vec![30]);
    }

    #[test]
    fn missing_first_texmap_builds_dynamic_slots_and_stops_resource_chain() {
        // LoadMap failing on the first group leaves an empty table in C++;
        // it does not skip TextureMap.Init or CrossMapMaterials. Because this
        // group loads both materials and textures, the two independent
        // zero-count fallbacks stay closed and the installed group is ignored.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("NoTexMap.c4s");
        let local = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&local).expect("local materials dir");
        for (file, name, density, overlay) in [
            ("A-Wet.c4m", "Wet", 25, "Liquid"),
            ("B-Rock.c4m", "Rock", 70, "Smooth"),
        ] {
            std::fs::write(
                local.join(file),
                format!("[Material]\nName={name}\nDensity={density}\nTextureOverlay={overlay}\n"),
            )
            .expect("write local material");
        }
        write_test_texture(&local, "Liquid");
        write_test_texture(&local, "Smooth");

        let installed_root = dir.path().join("Installed");
        let installed = installed_root.join("Material.c4g");
        std::fs::create_dir_all(&installed).expect("installed materials dir");
        std::fs::write(installed.join("TexMap.txt"), "1=Global-Rough\n")
            .expect("write installed texmap");
        std::fs::write(
            installed.join("Global.c4m"),
            "[Material]\nName=Global\nDensity=100\nTextureOverlay=Rough\n",
        )
        .expect("write installed material");
        write_test_texture(&installed, "Rough");

        let group = Group::open(&scenario_dir).expect("scenario group opens");
        let resolver = FileSystemResolver {
            roots: vec![installed_root],
        };
        let classifier = build_map_pixel_classifier(&group, &resolver)
            .expect("classifier load succeeds")
            .expect("a missing first TexMap still builds a classifier");

        let library = classifier.material_library().expect("local materials load");
        let material_order = library
            .iter()
            .map(|material| (material.name(), material.int("Density").unwrap_or(0)))
            .collect::<Vec<_>>();
        assert_eq!(material_order.len(), 2);
        assert!(material_order.iter().any(|(name, _)| *name == "Wet"));
        assert!(material_order.iter().any(|(name, _)| *name == "Rock"));
        assert!(library.get("Global").is_none());
        assert!(classifier.texture_exists("Liquid"));
        assert!(classifier.texture_exists("Smooth"));
        assert!(!classifier.texture_exists("Rough"));

        assert_eq!(classifier.state.default_material_entry("Global"), None);
        for (offset, (name, density)) in material_order.iter().enumerate() {
            let slot = offset + 1;
            let expected_texture = if *name == "Wet" { "Liquid" } else { "Smooth" };
            assert_eq!(
                classifier.state.default_material_entry(name),
                Some(slot as u8)
            );
            assert_eq!(
                classifier.state.material_names[slot].as_deref(),
                Some(*name)
            );
            assert_eq!(
                classifier.state.match_texture_names[slot].as_deref(),
                Some(expected_texture)
            );
            assert_eq!(classifier.density(slot as u8), *density);
        }
        assert!(classifier.state.material_names[0].is_none());
        assert!(classifier.state.material_names[3].is_none());

        let bitmap = clonk_resources::bitmap::IndexedBitmap {
            width: 3,
            height: 1,
            indices: vec![1, 2, 0],
        };
        let landscape = exact_classified_landscape(&bitmap, &classifier, 0, 2, None)
            .expect("indexed map classifies");
        assert_eq!(
            (0..3)
                .map(|x| landscape.grid_byte_at(x, 0))
                .collect::<Vec<_>>(),
            vec![Some(1), Some(2), Some(0)]
        );
        for (offset, (name, _)) in material_order.iter().enumerate() {
            if *name == "Wet" {
                assert!(landscape.is_liquid_at(offset as i32, 0));
            } else {
                assert!(landscape.is_solid_at(offset as i32, 0));
            }
        }
        assert!(!landscape.is_liquid_at(2, 0));
        assert!(!landscape.is_solid_at(2, 0));
    }

    #[test]
    fn material_enumeration_pairwise_swaps_before_crossmap_like_cpp() {
        // Raw A,B,C plus the prefix enumeration C must become C,B,A: C++
        // swaps the requested entry with slot zero rather than stably moving
        // it. The same order must drive both MaterialIds and dynamic texmap
        // allocation (before CrossMapMaterials).
        let classifier =
            build_material_enumeration_classifier(Some(b"ignored [Enumeration]\r\nC\r\n"))
                .expect("enumerated classifier builds");
        let library = classifier.material_library().expect("materials loaded");
        assert_eq!(
            library
                .iter()
                .map(|material| material.name())
                .collect::<Vec<_>>(),
            vec!["C", "B", "A"]
        );

        let materials = crate::MaterialSet::from_resource_library(library);
        assert_eq!(materials.id_of("C").map(|id| id.index()), Some(0));
        assert_eq!(materials.id_of("B").map(|id| id.index()), Some(1));
        assert_eq!(materials.id_of("A").map(|id| id.index()), Some(2));
        assert_eq!(classifier.state.default_material_entry("C"), Some(1));
        assert_eq!(classifier.state.default_material_entry("B"), Some(2));
        assert_eq!(classifier.state.default_material_entry("A"), Some(3));
    }

    #[test]
    fn material_enumeration_missing_name_fails_scenario_material_load() {
        let error =
            match build_material_enumeration_classifier(Some(b"[Enumeration]\r\nMissing\r\n")) {
                Ok(_) => panic!("missing enumeration material must fail"),
                Err(error) => error,
            };
        assert!(matches!(
            error,
            ScenarioError::MaterialEnumeration(
                clonk_resources::material::MaterialEnumerationError::MissingMaterial(ref name)
            ) if name == "Missing"
        ));
    }

    #[test]
    fn material_enumeration_name_fails_even_without_material_or_texmap_groups() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("MatMap.txt"),
            b"[Enumeration]\r\nMissing\r\n",
        )
        .expect("write MatMap");
        let group = Group::open(dir.path()).expect("scenario group opens");
        let resolver = FileSystemResolver { roots: Vec::new() };
        let error = match build_map_pixel_classifier(&group, &resolver) {
            Ok(_) => panic!("Num=0 cannot satisfy a listed material"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ScenarioError::MaterialEnumeration(
                clonk_resources::material::MaterialEnumerationError::MissingMaterial(ref name)
            ) if name == "Missing"
        ));
    }

    #[test]
    fn missing_material_enumeration_keeps_fresh_scenario_load_order() {
        let classifier =
            build_material_enumeration_classifier(None).expect("fresh classifier builds");
        assert_eq!(
            classifier
                .material_library()
                .expect("materials loaded")
                .iter()
                .map(|material| material.name())
                .collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
    }

    #[test]
    fn material_crossmap_slots_survive_classifier_build_and_serialization() {
        // CrossMapMaterials stores numeric slots in each C4Material. A later
        // SetTextureIndex can create duplicate names, so save/restore must
        // retain the originally resolved number rather than look it up again.
        let dir = tempdir().expect("tempdir");
        let materials = dir.path().join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(
            materials.join("TexMap.txt"),
            "4=Earth-Ridge\n30=Earth-Smooth\n",
        )
        .expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\nTextureOverlay=Smooth\nBlastShiftTo=Earth\n",
        )
        .expect("write material");
        write_test_texture(&materials, "Ridge");
        write_test_texture(&materials, "Smooth");

        let group = Group::open(dir.path()).expect("scenario group opens");
        let resolver = FileSystemResolver { roots: Vec::new() };
        let classifier = build_map_pixel_classifier(&group, &resolver)
            .expect("classifier builds")
            .expect("local texmap exists");
        assert_eq!(classifier.state.default_material_entry("Earth"), Some(30));
        assert_eq!(classifier.state.material_crossmap_entries, vec![30]);

        let encoded = serde_json::to_string(&classifier.state).expect("texmap serializes");
        let restored: RuntimeTexMapState = serde_json::from_str(&encoded).expect("texmap restores");
        assert_eq!(restored, classifier.state);
    }

    #[test]
    fn texmap_init_clears_unresolved_entries_and_frees_their_slots() {
        // C4TextureMap::Init clears both a known material with an unloaded
        // texture and an untrimmed, therefore unknown, material before
        // CrossMapMaterials. Once
        // slots 1..29 are occupied, GetIndex must therefore be able to reuse
        // the cleared slot 30 (C4Texture.cpp:68-104,229-244,319-345).
        let dir = tempdir().expect("tempdir");
        let materials = dir.path().join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(
            materials.join("TexMap.txt"),
            "1=Earth-NoSuchTex\n30=Earth-NoSuchTex\n31= Earth-Rough\n",
        )
        .expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=50\n",
        )
        .expect("write earth");
        std::fs::write(
            materials.join("X.c4m"),
            "[Material]\nName=X\nDensity=75\nShape=2\nTextureOverlay=Rough\n",
        )
        .expect("write allocation material");
        std::fs::write(
            materials.join("Y.c4m"),
            "[Material]\nName=Y\nDensity=80\nShape=3\n",
        )
        .expect("write fresh-pair material");
        write_test_texture(&materials, "Rough");

        let group = Group::open(dir.path()).expect("scenario group opens");
        let resolver = FileSystemResolver { roots: Vec::new() };
        let mut classifier = build_map_pixel_classifier(&group, &resolver)
            .expect("classifier builds")
            .expect("local texmap exists");

        for slot in [30usize, 31] {
            assert_eq!(classifier.state.densities[slot], 0);
            assert!(classifier.state.material_names[slot].is_none());
            assert!(classifier.state.texture_names[slot].is_none());
            assert!(classifier.state.match_texture_names[slot].is_none());
            assert!(classifier.state.shapes[slot].is_none());
        }
        assert!(!classifier.is_solid(30), "the invalid map byte is sky");
        assert_eq!(
            classifier.state.default_material_entry("X"),
            Some(1),
            "CrossMapMaterials reuses the cleared first slot"
        );

        for slot in 1..30 {
            classifier.state.material_names[slot] = Some(format!("Taken{slot}"));
        }
        assert_eq!(classifier.get_index("Y", Some("Rough"), true), 30);
        assert_eq!(classifier.state.material_names[30].as_deref(), Some("Y"));
        assert_eq!(
            classifier.state.match_texture_names[30].as_deref(),
            Some("Rough")
        );
        assert_eq!(classifier.state.densities[30], 80);
    }

    #[test]
    fn texmap_init_validates_liquid_smooth_against_liquid_texture() {
        // C4TexMapEntry::Init substitutes Liquid only for the texture lookup.
        // Smooth being loaded cannot save the static slot when Liquid is not.
        // The separately tracked dynamic-add substitution gap may create a
        // Water-Smooth pair elsewhere; this assertion pins the parsed slot.
        let dir = tempdir().expect("tempdir");
        let materials = dir.path().join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "25=Water-Smooth\n").expect("write texmap");
        std::fs::write(
            materials.join("Water.c4m"),
            "[Material]\nName=Water\nDensity=25\n",
        )
        .expect("write water");
        write_test_texture(&materials, "Smooth");

        let group = Group::open(dir.path()).expect("scenario group opens");
        let resolver = FileSystemResolver { roots: Vec::new() };
        let classifier = build_map_pixel_classifier(&group, &resolver)
            .expect("classifier builds")
            .expect("local texmap exists");

        assert_eq!(classifier.state.densities[25], 0);
        assert!(classifier.state.material_names[25].is_none());
        assert!(classifier.state.texture_names[25].is_none());
        assert!(classifier.state.match_texture_names[25].is_none());
        assert!(classifier.state.shapes[25].is_none());
        assert!(!classifier.is_liquid(25), "the invalid map byte is sky");
    }

    #[test]
    fn texmap_init_preserves_valid_entries_and_raw_liquid_pair_name() {
        // A valid liquid Smooth entry retains Smooth for pair matching while
        // selecting Liquid for rendering. Ordinary valid entries remain
        // byte-for-byte classified at their original slot.
        let dir = tempdir().expect("tempdir");
        let materials = dir.path().join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(
            materials.join("TexMap.txt"),
            "25=Water-Smooth\n30=Earth-Rough\n",
        )
        .expect("write texmap");
        std::fs::write(
            materials.join("Water.c4m"),
            "[Material]\nName=Water\nDensity=25\n",
        )
        .expect("write water");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");
        write_test_texture(&materials, "Liquid");
        write_test_texture(&materials, "Rough");

        let group = Group::open(dir.path()).expect("scenario group opens");
        let resolver = FileSystemResolver { roots: Vec::new() };
        let classifier = build_map_pixel_classifier(&group, &resolver)
            .expect("classifier builds")
            .expect("local texmap exists");

        assert_eq!(classifier.state.densities[25], 25);
        assert_eq!(
            classifier.state.material_names[25].as_deref(),
            Some("Water")
        );
        assert_eq!(
            classifier.state.match_texture_names[25].as_deref(),
            Some("Smooth")
        );
        assert_eq!(
            classifier.state.texture_names[25].as_deref(),
            Some("Liquid")
        );
        assert_eq!(classifier.state.densities[30], 100);
        assert_eq!(
            classifier.state.material_names[30].as_deref(),
            Some("Earth")
        );
        assert_eq!(classifier.state.texture_names[30].as_deref(), Some("Rough"));
    }

    #[test]
    fn get_index_never_allocates_reserved_diff_slot_127() {
        // C4M_MaxTexIndex is 127 and index 127 is reserved for landscape
        // diffs (C4Constants.h:63). C4TextureMap::GetIndex searches and
        // allocates only `byIndex < C4M_MaxTexIndex` (C4Texture.cpp:319-340),
        // so a map whose usable slots 1..=126 are full must return 0.
        let mut names = vec![None; 128];
        for (slot, name) in names.iter_mut().enumerate().take(127).skip(1) {
            *name = Some(format!("Taken{slot}"));
        }
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material]\nName=Earth\nDensity=100\nShape=2\n",
        )
        .expect("material parses");
        let mut classifier = MapPixelClassifier::from_slots_with_library(
            [0; 128],
            names,
            vec![None; 128],
            vec![None; 128],
            library,
            vec!["smooth".to_string()],
        );

        assert_eq!(classifier.get_index("Earth", Some("Smooth"), true), 0);
        assert!(classifier.state.material_names[127].is_none());
    }

    #[test]
    fn public_legacy_loaders_apply_the_explicit_startup_player_count() {
        // InitLocal freezes the admitted startup-player count before
        // C4Landscape::CreateMap reads it for MapPlayerExtend (pristine
        // 9ffa0a5d src/C4Game.cpp:2394-2431;
        // src/C4Landscape.cpp:518-522; src/C4Scenario.cpp:327-334).
        let dir = tempdir().expect("tempdir");
        let definition = dir.path().join("Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&definition).expect("definition dir");
        std::fs::write(
            definition.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        write_test_definition_graphics(&definition);

        let scenario_dir = dir.path().join("Extend.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Extend\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=20,0,1,20\nMapHeight=10,0,1,10\nMapZoom=5\n\
             MapPlayerExtend=1\nMaterial=Earth\n",
        )
        .expect("write scenario core");
        let materials = scenario_dir.join("Material.c4g");
        std::fs::create_dir_all(&materials).expect("materials dir");
        std::fs::write(materials.join("TexMap.txt"), "30=Earth-Smooth\n").expect("write texmap");
        std::fs::write(
            materials.join("Earth.c4m"),
            "[Material]\nName=Earth\nDensity=100\n",
        )
        .expect("write earth");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let group = Group::open(&scenario_dir).expect("scenario group opens");
        let one = Scenario::load_from_group_with_languages_and_seed_and_startup_player_count(
            &group,
            &resolver,
            &["US"],
            0,
            1,
        )
        .expect("one-player scenario loads through group API");
        let three = Scenario::load_from_path_with_languages_and_seed_and_startup_player_count(
            &scenario_dir,
            &resolver,
            &["US"],
            0,
            3,
        )
        .expect("three-player scenario loads through path API");

        assert_eq!(
            (
                one.landscape
                    .as_ref()
                    .expect("one-player landscape")
                    .width(),
                three
                    .landscape
                    .as_ref()
                    .expect("three-player landscape")
                    .width(),
            ),
            (20 * 5, 20 * 3 * 5),
        );
    }

    #[test]
    fn dynamic_landscape_uses_the_explicit_startup_player_count() {
        // InitLocal freezes the admitted startup-player count before
        // C4Landscape::CreateMap reads it for MapPlayerExtend (pristine
        // 9ffa0a5d src/C4Game.cpp:2394-2431;
        // src/C4Landscape.cpp:518-522; src/C4Scenario.cpp:327-334).
        // The fixed MapZoom has Rnd=0 and therefore keeps its configured
        // value even though Evaluate still consumes Random(1).
        let dir = tempdir().expect("tempdir");
        let group = Group::open(dir.path()).expect("scenario group opens");
        let manifest = parse_legacy_scenario_text(
            "[Landscape]\nMapWidth=20,0,1,20\nMapHeight=10,0,1,10\nMapZoom=5\nMapPlayerExtend=1\n",
        )
        .expect("scenario core parses");
        let mut classifier = MapPixelClassifier::from_slots(
            [0; 128],
            vec![None; 128],
            vec![None; 128],
            vec![None; 128],
        );

        let landscape =
            load_legacy_landscape_body_for_test(&group, &manifest, Some(&mut classifier), 0, 3)
                .expect("landscape loads")
                .expect("landscape exists");

        assert_eq!(landscape.width(), 20 * 3 * 5);
        assert_eq!(
            landscape
                .raster_state()
                .expect("dynamic map retains raster state")
                .map_zoom(),
            5
        );
    }

    #[test]
    fn dynamic_landscape_without_materials_still_clamps_to_cpp_minimum() {
        let dir = tempdir().expect("tempdir");
        let group = Group::open(dir.path()).expect("scenario group opens");
        let manifest = parse_legacy_scenario_text(
            "[Landscape]\nMapWidth=8,0,1,8\nMapHeight=5,0,1,5\nMapZoom=5\n",
        )
        .expect("scenario core parses");

        let mut callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut creator = None;
        let landscape = load_legacy_landscape_body(
            &group,
            &manifest,
            None,
            false,
            None,
            0,
            1,
            &HashSet::new(),
            &mut callbacks,
            &mut creator,
        )
        .expect("fallback landscape loads")
        .expect("fallback landscape exists");

        assert_eq!(
            (landscape.width(), landscape.estimated_height()),
            (100, 100)
        );
    }

    #[test]
    fn randomized_map_zoom_uses_post_map_creation_cpp_rng_draw() {
        // FixRandom(7) makes an early MapZoom Evaluate produce 9. C++ first
        // builds this basic map, then draw #530 is Random(5)=0, so
        // 10 + 0 - 2 yields zoom 8 (C4Landscape.cpp:578-635).
        let dir = tempdir().expect("tempdir");
        let group = Group::open(dir.path()).expect("scenario group opens");
        let manifest = parse_legacy_scenario_text(
            "[Landscape]\nMapWidth=20,0,1,20\nMapHeight=10,0,1,10\nMapZoom=10,2,5,15\n",
        )
        .expect("scenario core parses");
        let mut early_rng = legacy_map_creation_rng(7);
        assert_eq!(
            legacy_map_zoom(manifest.sections.get("landscape"), &mut early_rng),
            9,
            "evaluating before map creation uses the wrong ledger position"
        );
        let mut classifier = MapPixelClassifier::from_slots(
            [0; 128],
            vec![None; 128],
            vec![None; 128],
            vec![None; 128],
        );

        let landscape =
            load_legacy_landscape_body_for_test(&group, &manifest, Some(&mut classifier), 7, 1)
                .expect("landscape loads")
                .expect("landscape exists");
        assert_eq!(
            landscape
                .raster_state()
                .expect("dynamic map retains raster state")
                .map_zoom(),
            8
        );
        assert_eq!(landscape.width(), 160);

        let fallback = load_legacy_landscape_body_for_test(&group, &manifest, None, 7, 1)
            .expect("fallback landscape loads")
            .expect("fallback landscape exists");
        assert_eq!(
            fallback.width(),
            160,
            "fallback consumes the map-creation draws before MapZoom"
        );
    }

    #[test]
    fn keep_map_creator_persists_the_evaluated_tree_with_raster_state() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("KeepCreator.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("landscape.txt"),
            "overlay Named { seed = 7; }; map Test { seed = 11; };",
        )
        .expect("write lowercase landscape.txt");
        let group = Group::open(&scenario_dir).expect("scenario group opens");
        let manifest = parse_legacy_scenario_text(
            "[Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=5\nKeepMapCreator=1\n",
        )
        .expect("scenario core parses");
        let mut classifier = MapPixelClassifier::from_slots(
            [0; 128],
            vec![None; 128],
            vec![None; 128],
            vec![None; 128],
        );

        let landscape =
            load_legacy_landscape_body_for_test(&group, &manifest, Some(&mut classifier), 0, 1)
                .expect("landscape loads")
                .expect("landscape exists");
        let raster = landscape.raster_state().expect("raster state retained");
        assert_eq!(raster.map_zoom(), 5);
        assert!(
            raster.map_creator().is_some(),
            "KeepMapCreator retains tree"
        );

        let encoded = serde_json::to_string(&landscape).expect("landscape serializes");
        let restored: Landscape = serde_json::from_str(&encoded).expect("landscape restores");
        assert_eq!(restored, landscape, "creator and texmap survive saves");
    }

    #[test]
    fn shipped_lowercase_landscape_txt_uses_the_s2_creator() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(
            repository.join("content").is_dir(),
            "the official content submodule must be initialized"
        );

        for relative in [
            "content/Worlds.c4f/FoggyCliffs.c4s",
            "content/Fantasy.c4f/Crystalvalley.c4s",
        ] {
            let group = Group::open(repository.join(relative)).expect("shipped scenario group");
            let mut manifest = parse_legacy_scenario_manifest(&group).expect("scenario core");
            // Creator retention is only a diagnostic here: the basic-map
            // fallback cannot populate this state. Keep the render small
            // while exercising the shipped S2 source and loader gate.
            manifest.core.landscape.keep_map_creator = true;
            manifest.core.landscape.map_width = LegacyC4SVal::new(64, 0, 64, 250);
            manifest.core.landscape.map_height = LegacyC4SVal::new(40, 0, 40, 250);
            let mut classifier = MapPixelClassifier::from_slots(
                [0; 128],
                vec![None; 128],
                vec![None; 128],
                vec![None; 128],
            );

            let landscape =
                load_legacy_landscape_body_for_test(&group, &manifest, Some(&mut classifier), 0, 1)
                    .expect("shipped landscape loads")
                    .expect("shipped landscape exists");
            assert!(
                landscape
                    .raster_state()
                    .expect("dynamic landscape retains raster state")
                    .map_creator()
                    .is_some(),
                "{relative} must traverse create_s2_map_with_state"
            );
        }
    }

    #[test]
    fn classified_static_map_builds_the_per_pixel_plane_like_cpp() {
        // MapToLandscape blits each map cell at MapZoom scale into the
        // Surface8 pixel plane (C4Landscape::MapToSurface via
        // ChunkOZoom, C4Landscape.cpp:732-789); GBackSolid then reads
        // Pix2Dens per PIXEL (C4Wrappers.h:174-177), so cave water below
        // the column surface is liquid — never solid — while the earth
        // roof above it stays solid. The column approximation calls
        // everything below the first solid row "solid", which sheds
        // cave-roof objects (GoldRush's stalactites fell and shattered).
        let bitmap = clonk_resources::bitmap::IndexedBitmap {
            width: 4,
            height: 4,
            indices: vec![
                0, 30, 30, 0, //
                0, 20, 20, 0, //
                30, 20, 20, 0, //
                30, 30, 30, 0,
            ],
        };
        let mut densities = [0i32; 128];
        densities[20] = 25; // Water
        densities[30] = 100; // Earth
        let mut names: Vec<Option<String>> = vec![None; 128];
        names[20] = Some("Water".into());
        names[30] = Some("Earth".into());
        // No `Shape` in either material: MapChunkType 0 = Flat, chunks
        // box-fill their blocks (C4Landscape.cpp:285-287).
        let mut shapes: Vec<Option<crate::chunky::ChunkShape>> = vec![None; 128];
        shapes[20] = Some(crate::chunky::ChunkShape::Flat);
        shapes[30] = Some(crate::chunky::ChunkShape::Flat);
        let classifier = MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes);

        let landscape =
            classified_landscape(&bitmap, &classifier, 10, 0).expect("landscape builds");

        let grid = landscape
            .pixel_grid()
            .expect("classified maps build Surface8");
        assert_eq!(
            grid.byte_at(15, 15),
            Some(20),
            "world pixels carry the raw map byte of their zoom block"
        );
        assert_eq!(grid.byte_at(15, 5), Some(30), "roof block is earth");
        assert_eq!(grid.byte_at(35, 35), Some(0), "sky column stays sky");

        // Map column 1 (world x 10..20): earth roof row 0, water rows 1-2,
        // earth bed row 3. Pixel truth: the water interior is NOT solid.
        assert!(
            !landscape.is_solid_at(15, 15),
            "GBackSolid is false in water (density 25 < C4M_Solid)"
        );
        assert!(landscape.is_liquid_at(15, 15), "river interior is liquid");
        assert!(landscape.is_solid_at(15, 5), "earth roof is solid");
        assert!(landscape.is_solid_at(15, 35), "earth bed is solid");
        assert!(
            !landscape.is_solid_at(35, 25),
            "sky below roof level in an open column is not solid"
        );
    }

    #[test]
    fn classified_landscape_clamps_tiny_maps_to_cpp_minimum_dimensions() {
        // C4Landscape::Init allocates max(MapZoom*MapSize, 100) in each
        // dimension, but MapToLandscape still clips drawing to the zoomed
        // map rectangle. The remaining right/bottom pixels stay sky and the
        // closed border begins at the clamped coordinate.
        let bitmap = clonk_resources::bitmap::IndexedBitmap {
            width: 8,
            height: 5,
            indices: vec![30; 8 * 5],
        };
        let mut densities = [0i32; 128];
        densities[30] = 100;
        let mut names = vec![None; 128];
        names[30] = Some("Earth".into());
        let mut shapes = vec![None; 128];
        shapes[30] = Some(crate::chunky::ChunkShape::Flat);
        let classifier = MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes);

        let landscape =
            classified_landscape(&bitmap, &classifier, 10, 0).expect("landscape builds");
        let grid = landscape
            .pixel_grid()
            .expect("classified maps build Surface8");
        assert_eq!(
            (landscape.width(), landscape.estimated_height()),
            (100, 100)
        );
        assert_eq!((grid.width(), grid.height()), (100, 100));
        assert_eq!(grid.byte_at(79, 49), Some(30), "map reaches its own edge");
        for y in 0..50 {
            for x in 80..100 {
                assert_eq!(grid.byte_at(x, y), Some(0), "right padding at ({x},{y})");
            }
        }
        for y in 50..100 {
            for x in 0..100 {
                assert_eq!(grid.byte_at(x, y), Some(0), "bottom padding at ({x},{y})");
            }
        }
        assert!(
            !landscape.is_solid_at(99, 99),
            "last in-bounds pixel is sky"
        );
        assert!(
            landscape.is_solid_at(100, 99),
            "right border starts at x=100"
        );
        assert!(
            landscape.is_solid_at(99, 100),
            "bottom border starts at y=100"
        );
    }

    #[test]
    fn classified_landscape_keeps_large_map_plane_byte_identical() {
        let bitmap = clonk_resources::bitmap::IndexedBitmap {
            width: 11,
            height: 10,
            indices: vec![30; 11 * 10],
        };
        let mut densities = [0i32; 128];
        densities[30] = 100;
        let mut names = vec![None; 128];
        names[30] = Some("Earth".into());
        let mut shapes = vec![None; 128];
        shapes[30] = Some(crate::chunky::ChunkShape::Flat);
        let classifier =
            MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes.clone());
        let expected = crate::chunky::synthesize_landscape(&bitmap.indices, 11, 10, 10, 0, &shapes)
            .into_bytes();

        let landscape =
            classified_landscape(&bitmap, &classifier, 10, 0).expect("landscape builds");
        let grid = landscape
            .pixel_grid()
            .expect("classified maps build Surface8");
        assert_eq!((grid.width(), grid.height()), (110, 100));
        assert_eq!(
            grid.bytes(),
            expected,
            "unclamped planes stay byte-identical"
        );
    }

    #[test]
    fn classified_static_map_synthesizes_chunky_borders_like_chunk_o_zoom() {
        // MapToLandscape zooms map cells through ChunkOZoom: Smooth/Rough
        // materials draw jittered chunk POLYGONS, not blocks
        // (DrawChunk, C4Landscape.cpp:280-313), so material borders
        // bulge past the zoom grid. With MapSeed=0 the cell at map (1,1)
        // (cro=5) reaches one pixel above its block at world columns
        // 5..=7 (hand-stepped in chunky::tests) — cave roofs gain the
        // overhang that keeps stalactites attached in C++.
        let bitmap = clonk_resources::bitmap::IndexedBitmap {
            width: 3,
            height: 2,
            indices: vec![
                0, 0, 0, //
                30, 30, 30,
            ],
        };
        let mut densities = [0i32; 128];
        densities[30] = 100;
        let mut names: Vec<Option<String>> = vec![None; 128];
        names[30] = Some("Earth".into());
        let mut shapes: Vec<Option<crate::chunky::ChunkShape>> = vec![None; 128];
        shapes[30] = Some(crate::chunky::ChunkShape::Smooth);
        let classifier = MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes);

        let landscape = classified_landscape(&bitmap, &classifier, 4, 0).expect("landscape builds");

        assert!(landscape.is_solid_at(6, 3), "chunk bulges above its block");
        assert!(!landscape.is_solid_at(4, 3), "bulge is jitter-shaped");
        assert_eq!(
            landscape.surface_height(6),
            Some(3),
            "surface columns derive from the synthesized plane"
        );
        assert_eq!(landscape.surface_height(4), Some(4));
    }

    #[test]
    fn exact_indexed_landscape_preserves_surface8_bytes_verbatim() {
        // C4Landscape::Load installs GroupReadSurfaceOwnPal8's index plane
        // directly as Surface8 (C4Landscape.cpp:1520-1533). Exact landscapes
        // neither apply MapZoom nor pass through ChunkOZoom: Flat pixels must
        // not bleed into their right/bottom neighbors, and IFT stays bit 0x80.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("Exact.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Landscape]\nExactLandscape=1\nNewStyleLandscape=2\nMapZoom=7\n",
        )
        .expect("write scenario core");
        let expected = vec![
            0, 0, 0, 0, //
            0, 5, 0, 0, // isolated Flat pixel: no inclusive-edge bleed
            0, 0x85, 0, 0, // the same texmap slot with IFT set
        ];
        std::fs::write(
            scenario_dir.join("lAnDsCaPe.BmP"),
            encode_indexed_bmp(&[&[0, 0, 0, 0], &[0, 5, 0, 0], &[0, 0x85, 0, 0]]),
        )
        .expect("write mixed-case exact landscape");

        let group = Group::open(&scenario_dir).expect("scenario group opens");
        let manifest = parse_legacy_scenario_text(
            "[Landscape]\nExactLandscape=1\nNewStyleLandscape=2\nMapZoom=7\n",
        )
        .expect("scenario core parses");
        let mut densities = [0i32; 128];
        densities[5] = 100;
        let mut names = vec![None; 128];
        names[5] = Some("Earth".into());
        let mut shapes = vec![None; 128];
        shapes[5] = Some(crate::chunky::ChunkShape::Flat);
        let mut classifier =
            MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes);

        let landscape =
            load_legacy_landscape_body_for_test(&group, &manifest, Some(&mut classifier), 0, 1)
                .expect("exact landscape loads")
                .expect("exact landscape exists");
        let grid = landscape
            .pixel_grid()
            .expect("exact indexed landscape keeps Surface8");
        assert_eq!(landscape.mode(), LANDSCAPE_MODE_EXACT);
        assert_eq!(
            landscape.raster_state().unwrap().map_zoom(),
            0,
            "initial exact landscapes never evaluate or assign MapZoom"
        );
        assert_eq!(landscape.width(), 4, "bitmap width is not MapZoom-scaled");
        assert_eq!(landscape.estimated_height(), 3, "bitmap height is exact");
        assert_eq!((grid.width(), grid.height()), (4, 3));
        assert_eq!(grid.bytes(), expected, "Surface8 bytes are verbatim");
        assert!(
            landscape
                .raster_state()
                .expect("exact landscape retains the runtime texmap")
                .map()
                .is_none(),
            "exact landscapes retain no Map/ChunkOZoom source surface"
        );
    }

    #[test]
    fn exact_landscape_honors_new_style_version_png_and_invalid_bytes() {
        fn classifier() -> MapPixelClassifier {
            let mut state = RuntimeTexMapState::default();
            state.materials = [("Dup", 60), ("dUp", 61), ("Vehicle", 100), ("Tunnel", 50)]
                .into_iter()
                .map(|(name, density)| RuntimeTexMapMaterial {
                    name: name.to_string(),
                    density,
                    shape: crate::chunky::ChunkShape::Flat,
                })
                .collect();
            state.default_material_entries = vec![
                ("Dup".to_string(), 30),
                ("dUp".to_string(), 31),
                ("Vehicle".to_string(), 7),
                ("Tunnel".to_string(), 10),
            ];
            for (slot, name, density) in [
                (30, "Dup", 60),
                (31, "dUp", 61),
                (7, "Vehicle", 100),
                (10, "Tunnel", 50),
            ] {
                state.material_names[slot] = Some(name.to_string());
                state.densities[slot] = density;
                state.shapes[slot] = Some(crate::chunky::ChunkShape::Flat);
            }
            MapPixelClassifier::from_runtime_state(state)
        }

        fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
            use image::ImageEncoder as _;

            let mut encoded = Vec::new();
            image::codecs::png::PngEncoder::new(&mut encoded)
                .write_image(rgba, width, height, ColorType::Rgba8)
                .expect("encode Landscape.png");
            encoded
        }

        fn load_fixture(
            root: &Path,
            name: &str,
            format: i32,
            rows: &[&[u8]],
            png: Option<&[u8]>,
            classifier: &mut MapPixelClassifier,
        ) -> Result<Landscape, ScenarioError> {
            let scenario_dir = root.join(format!("{name}.c4s"));
            std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
            let source = format!("[Landscape]\nExactLandscape=1\nNewStyleLandscape={format}\n");
            std::fs::write(scenario_dir.join("Scenario.txt"), &source)
                .expect("write scenario core");
            std::fs::write(scenario_dir.join("Landscape.bmp"), encode_indexed_bmp(rows))
                .expect("write Landscape.bmp");
            if let Some(png) = png {
                std::fs::write(scenario_dir.join("lAnDsCaPe.PnG"), png)
                    .expect("write Landscape.png");
            }
            let group = Group::open(&scenario_dir).expect("scenario group opens");
            let manifest = parse_legacy_scenario_text(&source).expect("scenario core parses");
            load_legacy_landscape_body_for_test(&group, &manifest, Some(classifier), 0, 1)?
                .ok_or_else(|| {
                    ScenarioError::InvalidLandscape("exact landscape was not created".to_string())
                })
        }

        let dir = tempdir().expect("tempdir");

        // Format 0: three colors per material and the old 0x40 IFT range
        // become each material's current DefaultMatTex plus current 0x80 IFT.
        // The case-only duplicate material names deliberately own different
        // numeric defaults.
        let mut format0_classifier = classifier();
        let format0 = load_fixture(
            dir.path(),
            "Format0",
            0,
            &[&[128, 131, 134, 137, 192, 195, 198, 201]],
            None,
            &mut format0_classifier,
        )
        .expect("format-0 landscape converts");
        assert_eq!(
            format0.pixel_grid().unwrap().bytes(),
            &[30, 31, 7, 10, 158, 159, 135, 138]
        );

        // Format 1 conversion is independent of PNG presence. It retains the
        // three Vehicle colors, maps out-of-range material indices to sky and
        // preserves the source IFT bit.
        let format1_rgba = [1, 2, 3, 255].repeat(10);
        let format1_png = encode_png(10, 1, &format1_rgba);
        let mut format1_classifier = classifier();
        let format1 = load_fixture(
            dir.path(),
            "Format1",
            1,
            &[&[0, 1, 2, 3, 4, 5, 6, 7, 129, 134]],
            Some(&format1_png),
            &mut format1_classifier,
        )
        .expect("format-1 landscape converts with PNG present");
        assert_eq!(
            format1.pixel_grid().unwrap().bytes(),
            &[0, 30, 31, 7, 7, 7, 10, 0, 158, 138]
        );
        assert_eq!(format1.surface32_pixel_at(0, 0), Some(0x0001_0203));

        // Landscape.png is presentation-only: BMP material bytes stay live,
        // ordinary PNG alpha becomes inverted Clonk transparency, and a fully
        // transparent source pixel is canonical transparent black.
        let png = encode_png(
            3,
            1,
            &[
                0x11, 0x22, 0x33, 255, 0x44, 0x55, 0x66, 128, 0x99, 0x88, 0x77, 0,
            ],
        );
        let mut png_classifier = classifier();
        let png_landscape = load_fixture(
            dir.path(),
            "Png",
            2,
            &[&[30, 31, 7]],
            Some(&png),
            &mut png_classifier,
        )
        .expect("current-format PNG landscape loads");
        assert_eq!(png_landscape.pixel_grid().unwrap().bytes(), &[30, 31, 7]);
        assert_eq!(
            (0..3)
                .map(|x| png_landscape.surface32_pixel_at(x, 0))
                .collect::<Vec<_>>(),
            vec![Some(0x0011_2233), Some(0x7f44_5566), Some(0xff00_0000)]
        );

        // Merely finding the sidecar suppresses format-0 conversion. Decode
        // failure is nonfatal, so this already-live byte remains untouched.
        let mut malformed_png_classifier = classifier();
        let malformed_png = load_fixture(
            dir.path(),
            "MalformedPng",
            0,
            &[&[30]],
            Some(b"not a PNG"),
            &mut malformed_png_classifier,
        )
        .expect("malformed PNG is nonfatal");
        assert_eq!(malformed_png.pixel_grid().unwrap().bytes(), &[30]);
        assert_eq!(malformed_png.surface32_pixel_at(0, 0), None);

        // Current/live format validates before DiffLandscape.bmp is applied
        // and rejects the first unmapped nonzero byte in row-major order.
        let mut invalid_classifier = classifier();
        let error = match load_fixture(
            dir.path(),
            "Invalid",
            2,
            &[&[0, 31], &[7, 42]],
            None,
            &mut invalid_classifier,
        ) {
            Ok(_) => panic!("invalid live byte must reject the landscape"),
            Err(error) => error,
        };
        let ScenarioError::InvalidLandscape(detail) = error else {
            panic!("unexpected invalid-byte error: {error:?}");
        };
        assert!(
            detail.contains("(1/1)"),
            "wrong invalid-byte coordinate: {detail}"
        );
        assert!(detail.contains("42"), "wrong invalid-byte value: {detail}");
    }

    #[test]
    fn legacy_landscape_diff_is_applied_after_initial_snapshot() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("Diff.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Landscape]\nExactLandscape=1\nNewStyleLandscape=2\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[1, 2, 3], &[4, 5, 6]]),
        )
        .expect("write base landscape");
        let expected_diff = vec![
            0xff, 7, 0xff, // preserve, change, preserve
            0, 0xff, 8, // zero is a change, preserve, change
        ];
        std::fs::write(
            scenario_dir.join("dIfFlAnDsCaPe.BmP"),
            encode_indexed_bmp(&[&[0xff, 7, 0xff], &[0, 0xff, 8]]),
        )
        .expect("write mixed-case diff landscape");

        let group = Group::open(&scenario_dir).expect("scenario group opens");
        let manifest =
            parse_legacy_scenario_text("[Landscape]\nExactLandscape=1\nNewStyleLandscape=2\n")
                .expect("scenario core parses");
        let mut densities = [0i32; 128];
        let mut names = vec![None; 128];
        let mut shapes = vec![None; 128];
        for slot in 1..=8 {
            densities[slot] = 100;
            names[slot] = Some("Earth".into());
            shapes[slot] = Some(crate::chunky::ChunkShape::Flat);
        }
        let mut classifier =
            MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes);
        let mut callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut creator = None;

        let landscape = load_legacy_landscape(
            &group,
            &manifest,
            None,
            false,
            Some(&mut classifier),
            0,
            1,
            &HashSet::new(),
            &mut callbacks,
            &mut creator,
        )
        .expect("legacy landscape loads")
        .expect("legacy landscape exists");

        assert_eq!(
            landscape.pixel_grid().expect("loaded Surface8").bytes(),
            &[1, 7, 3, 0, 5, 8],
            "0xff preserves the base while every other differing byte applies"
        );
        let saved = landscape
            .save_diff(false)
            .expect("masked diff rebuilds")
            .expect("loaded changes emit a diff");
        assert_eq!(
            saved.indices, expected_diff,
            "SaveInitial ran before ApplyDiff, preserving the original comparison plane"
        );
    }

    #[test]
    fn legacy_landscape_shade_materials_reaches_runtime_landscape() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("Shade.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[1]]),
        )
        .expect("write exact landscape");
        let source = "[Landscape]\nExactLandscape=1\nNewStyleLandscape=2\nShadeMaterials=0\n";
        let group = Group::open(&scenario_dir).expect("scenario group opens");
        let manifest = parse_legacy_scenario_text(source).expect("scenario core parses");
        let mut densities = [0i32; 128];
        densities[1] = 100;
        let mut names = vec![None; 128];
        names[1] = Some("Earth".into());
        let mut shapes = vec![None; 128];
        shapes[1] = Some(crate::chunky::ChunkShape::Flat);
        let mut classifier =
            MapPixelClassifier::from_slots(densities, names, vec![None; 128], shapes);
        let mut callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut creator = None;

        let landscape = load_legacy_landscape(
            &group,
            &manifest,
            None,
            false,
            Some(&mut classifier),
            0,
            1,
            &HashSet::new(),
            &mut callbacks,
            &mut creator,
        )
        .expect("legacy landscape loads")
        .expect("legacy landscape exists");

        assert!(
            !landscape.shade_materials(),
            "the parsed scenario flag rides the runtime landscape snapshot"
        );
        let restored: Landscape =
            serde_json::from_str(&serde_json::to_string(&landscape).expect("landscape serializes"))
                .expect("landscape restores");
        assert!(!restored.shade_materials());
    }

    #[test]
    fn exact_landscape_requires_landscape_bmp_instead_of_map_fallback() {
        // C4Landscape::Load accesses C4CFN_Landscape directly and fails when
        // it is absent; the static-only Map.bmp fallback is not consulted.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("Exact.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Map.bmp"),
            encode_indexed_bmp(&[&[0, 0], &[5, 5]]),
        )
        .expect("write static map only");
        let group = Group::open(&scenario_dir).expect("scenario group opens");
        let manifest =
            parse_legacy_scenario_text("[Landscape]\nExactLandscape=1\nNewStyleLandscape=2\n")
                .expect("scenario core parses");

        let error = load_legacy_landscape_body_for_test(&group, &manifest, None, 0, 1)
            .expect_err("exact load must require Landscape.bmp");
        assert!(
            matches!(
                &error,
                ScenarioError::Resources(GroupError::EntryNotFound(_))
            ) || matches!(
                &error,
                ScenarioError::Resources(GroupError::Io(io_error))
                    if io_error.kind() == io::ErrorKind::NotFound
            ),
            "unexpected missing-Landscape.bmp error: {error:?}"
        );
    }

    #[test]
    fn exact_landscape_bmp_loads_at_pixel_scale() {
        // ExactLandscape=1: Landscape.bmp IS the landscape — C++ reads it
        // straight into the pixel surface (GroupReadSurface8, no MapZoom).
        // The heightfield model reduces it to the column profile at zoom 1;
        // returning NO landscape here left GBackSolid answering "never
        // solid" and hung content like the grass placement loop
        // (Knights.c4f/Dunkelfels.c4s + Grass.c4d Initialize).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Exact\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=GOOD=1\nPosition=4,2\n\n[Landscape]\nExactLandscape=1\nNewStyleLandscape=2\n",
        )
        .expect("write scenario core");
        let mut bitmap = RgbaImage::from_pixel(8, 6, Rgba([0, 0, 255, 255]));
        for y in 2..6 {
            for x in 0..8 {
                bitmap.put_pixel(x, y, Rgba([128, 64, 32, 255]));
            }
        }
        let raw = bitmap.into_raw();
        let mut encoded = Vec::new();
        {
            let mut encoder = BmpEncoder::new(&mut encoded);
            encoder
                .encode(&raw, 8, 6, ColorType::Rgba8)
                .expect("encode landscape bmp");
        }
        std::fs::write(scenario_dir.join("Landscape.bmp"), encoded).expect("write landscape");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let landscape = engine.landscape().expect("exact landscape loads");
        assert_eq!(landscape.width(), 8, "pixel scale: no MapZoom applied");
        assert_eq!(
            landscape.surface(),
            vec![2; 8].as_slice(),
            "the surface Y coordinate is the first ground row"
        );
    }

    #[test]
    fn legacy_map_bmp_creates_landscape_height_profile() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let crew_core = defs_root.join("Crew.c4d");
        std::fs::create_dir_all(&crew_core).expect("crew definition dir");
        std::fs::write(
            crew_core.join("DefCore.txt"),
            "[DefCore]\nid=CLNK\nName=Clonk\nCategory=0\nCrewMember=1\n",
        )
        .expect("write crew defcore");
        std::fs::write(crew_core.join("Script.c"), "// crew script\n").expect("crew script");
        write_test_definition_graphics(&crew_core);

        let scenario_dir = dir.path().join("LegacyLandscape.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Landscape\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=CLNK=1\nPosition=40,60\n\n[Landscape]\nMapWidth=4\nMapHeight=4\nMapZoom=2\n",
        )
        .expect("write scenario core");

        let mut map = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 255, 255]));
        for y in 1..4 {
            for x in 0..4 {
                map.put_pixel(x, y, Rgba([128, 64, 32, 255]));
            }
        }
        let raw = map.into_raw();
        let mut encoded = Vec::new();
        {
            let mut encoder = BmpEncoder::new(&mut encoded);
            encoder
                .encode(&raw, 4, 4, ColorType::Rgba8)
                .expect("encode map bmp");
        }
        std::fs::write(scenario_dir.join("mAp.BmP"), encoded).expect("write mixed-case map");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let landscape = engine.landscape().expect("landscape present");
        // MapZoom=2 clamps to the C4SVal Min of 5 (C4Scenario.cpp:307,353):
        // the rendered map is 20x20 with ground starting at y=5, while
        // C4Landscape::Init pads both axes to 100. The column-only fallback
        // represents the right-hand padding as all-sky columns.
        assert_eq!(
            (landscape.width(), landscape.estimated_height()),
            (100, 100)
        );
        assert_eq!(&landscape.surface()[..20], vec![5; 20].as_slice());
        assert_eq!(&landscape.surface()[20..], vec![100; 80].as_slice());
    }

    #[test]
    fn legacy_scenario_populates_physics_and_environment() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let crew_core = defs_root.join("Crew.c4d");
        std::fs::create_dir_all(&crew_core).expect("crew definition dir");
        std::fs::write(
            crew_core.join("DefCore.txt"),
            "[DefCore]\nid=CLNK\nName=Clonk\nCategory=0\nCrewMember=1\n",
        )
        .expect("write crew defcore");
        std::fs::write(crew_core.join("Script.c"), "// crew script\n").expect("crew script");
        write_test_definition_graphics(&crew_core);

        let scenario_dir = dir.path().join("LegacyEnvironment.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            r#"
            [Head]
            Title=Legacy Environment

            [Definitions]
            Definition1=Defs.c4d

            [Player1]
            Crew=CLNK=1
            Position=20,40

            [Landscape]
            Gravity=120

            [Weather]
            Wind=10,5,-20,20
            Climate=60
            Rain=35
            Lightning=12
            StartSeason=30,10,0,100
            YearSpeed=45
            NoGamma=0

            [Disasters]
            Meteorite=25
            Volcano=15
            Earthquake=5
            "#,
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");

        let physics = scenario.physics().expect("physics present");
        assert_eq!(
            physics.gravity, 120,
            "expected gravity parsed from Scenario.txt"
        );

        let environment = scenario.environment().expect("environment present");
        assert_eq!(environment.wind, 10, "expected wind base from Scenario.txt");
        assert_eq!(
            environment.wind_variation, 5,
            "expected wind variation from Scenario.txt"
        );
        assert_eq!(
            environment.climate, -10,
            "expected climate transformed value"
        );
        assert_eq!(
            environment.temperature, -10,
            "temperature should match initial climate"
        );
        assert_eq!(environment.season, 30, "StartSeason should map to season");
        assert_eq!(environment.year_speed, 45, "YearSpeed should be retained");
        assert_eq!(
            environment.precipitation, 35,
            "rain should map to precipitation"
        );
        assert_eq!(
            environment.precipitation_strength, 35,
            "rain should map to precipitation strength"
        );
        assert_eq!(
            environment.lightning, 12,
            "lightning level should be parsed"
        );
        assert_eq!(
            environment.meteorite, 25,
            "meteorite level should be parsed"
        );
        assert_eq!(environment.volcano, 15, "volcano level should be parsed");
        assert_eq!(
            environment.earthquake, 5,
            "earthquake level should be parsed"
        );
        assert!(
            !environment.no_gamma,
            "NoGamma=0 should enable gamma correction"
        );

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let configured_physics = engine.physics();
        assert_eq!(
            configured_physics.gravity, 120,
            "engine should receive legacy gravity"
        );

        let configured_environment = engine.environment();
        // The applied wind is C4Weather::Init's Wind.Evaluate draw, not the
        // C4SVal base (C4Weather.cpp:47) — replay the init ledger to the
        // wind draw: Season, YearSpeed, Climate precede it.
        let mut replay = crate::rng::LcgRng::seed_from_u64(0);
        // Landscape.ScenarioInit's Gravity draw precedes the weather
        // evaluates (C4Landscape.cpp:66); this scenario's Gravity=120.
        LegacyC4SVal::new(120, 0, 10, 200).evaluate(&mut replay);
        // No NoInitialize: InitVegetation/InitInEarth ALWAYS evaluate
        // their levels — one draw each even with empty id lists
        // (C4Game.cpp:3069,3084) — between the Gravity draw and
        // Weather.Init's.
        LegacyC4SVal::new(50, 30, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(50, 0, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(30, 10, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(45, 0, 0, 100).evaluate(&mut replay);
        LegacyC4SVal::new(60, 10, 0, 100).evaluate(&mut replay);
        let drawn_wind = LegacyC4SVal::new(10, 5, -20, 20).evaluate(&mut replay);
        assert_eq!(
            configured_environment.wind, drawn_wind,
            "engine wind is the Wind.Evaluate init draw (C4Weather.cpp:47)"
        );
        assert_eq!(
            configured_environment.wind_variation, 5,
            "engine should receive wind variation"
        );
        assert_eq!(
            configured_environment.year_speed, 45,
            "engine should receive year speed"
        );
        assert_eq!(
            configured_environment.lightning, 12,
            "engine should receive lightning level"
        );
        assert_eq!(
            configured_environment.meteorite, 25,
            "engine should receive meteorite level"
        );
        assert_eq!(
            configured_environment.volcano, 15,
            "engine should receive volcano level"
        );
        assert_eq!(
            configured_environment.earthquake, 5,
            "engine should receive earthquake level"
        );
        assert!(
            !configured_environment.no_gamma,
            "engine should reflect gamma enabled flag"
        );
    }

    #[test]
    fn physics_validation_rejects_invalid_limits() {
        let manifest = PhysicsManifest {
            gravity: Some(1),
            max_fall_speed: Some(4),
            max_rise_speed: Some(6),
            max_horizontal_speed: None,
        };

        let error = manifest
            .into_settings()
            .expect_err("invalid physics manifest fails");
        match error {
            ScenarioError::InvalidPhysics(detail) => {
                assert!(detail.contains("max_rise_speed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn physics_validation_rejects_negative_horizontal_speed() {
        let manifest = PhysicsManifest {
            gravity: None,
            max_fall_speed: None,
            max_rise_speed: None,
            max_horizontal_speed: Some(-1),
        };

        let error = manifest
            .into_settings()
            .expect_err("negative horizontal speed fails");
        match error {
            ScenarioError::InvalidPhysics(detail) => {
                assert!(detail.contains("max_horizontal_speed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
