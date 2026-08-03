// Contiguous slice 5 of 8 of the `scenario/tests` battery, spliced
// by `include!` from the parent module so every test id is unchanged.

    #[test]
    fn legacy_numbers_tolerate_trailing_junk_like_cpp() {
        // StdCompilerINIRead reads numbers strtol-style: the leading integer
        // parses, trailing junk is ignored. Real content relies on it —
        // `Position=22,28;` (Missions.c4f/LastWill.c4s/Scenario.txt:21).
        assert_eq!(
            parse_position("Position", "22,28;").expect("position parses"),
            [22, 28]
        );
        assert_eq!(parse_i32("7663;").expect("parses"), 7663);
        assert_eq!(parse_i32("-15x").expect("parses"), -15);
        assert_eq!(parse_i32(" 42 trailing words").expect("parses"), 42);
        assert!(parse_i32("junk").is_err(), "no digits is still an error");
    }

    fn parsed_base_functionality(raw: &str) -> i32 {
        parse_legacy_scenario_text(&format!("[Game]\nBaseFunctionality={raw}\n"))
            .expect("scenario core parses")
            .core
            .game
            .realism
            .base_functionality
    }

    #[test]
    fn base_functionality_parses_each_numeric_or_named_pipe_element() {
        assert_eq!(
            parsed_base_functionality("1|BASEFUNC_Buy"),
            BASEFUNC_AUTO_SELL_CONTENTS | BASEFUNC_BUY
        );
        assert_eq!(
            parsed_base_functionality("BASEFUNC_RegenerateEnergy|8"),
            BASEFUNC_REGENERATE_ENERGY | BASEFUNC_SELL
        );
        assert_eq!(
            parsed_base_functionality(" BASEFUNC_Buy \t| 8 "),
            BASEFUNC_BUY | BASEFUNC_SELL
        );
        assert_eq!(parsed_base_functionality("0x10|4"), 20);
        assert_eq!(parsed_base_functionality("-0x10|4"), 0);
        assert_eq!(parsed_base_functionality("077"), 77);
        assert_eq!(
            parsed_base_functionality("BASEFUNC_Buy|8junk|BASEFUNC_AutoSellContents"),
            BASEFUNC_BUY | BASEFUNC_SELL,
            "strtol consumes the numeric prefix and the junk terminates the list"
        );
        assert_eq!(
            parsed_base_functionality("0xG|BASEFUNC_Buy"),
            0,
            "invalid hexadecimal syntax still consumes the leading zero"
        );
        let narrowed_overflow = if std::mem::size_of::<std::os::raw::c_long>() == 8 {
            0
        } else {
            i32::MAX
        };
        assert_eq!(
            parsed_base_functionality("4294967296|BASEFUNC_Buy"),
            narrowed_overflow | BASEFUNC_BUY,
            "strtol narrows its native-long result to int32 before the OR"
        );
    }

    #[test]
    fn base_functionality_unknown_names_warn_without_dropping_known_elements() {
        let (parsed, warnings) = capture_definition_warnings(|| {
            parsed_base_functionality("BASEFUNC_Buy|BASEFUNC_Bogus|BASEFUNC_AutoSellContents")
        });

        assert_eq!(parsed, BASEFUNC_BUY | BASEFUNC_AUTO_SELL_CONTENTS);
        assert!(warnings.iter().any(|warning| {
            warning.message.as_deref() == Some("unknown BaseFunctionality bit name")
        }));
    }

    #[test]
    fn base_functionality_only_pipe_continues_the_element_loop() {
        assert_eq!(
            parsed_base_functionality("BASEFUNC_Buy,BASEFUNC_Sell"),
            BASEFUNC_BUY
        );
        assert_eq!(
            parsed_base_functionality("BASEFUNC_Buy&BASEFUNC_Sell"),
            BASEFUNC_BUY
        );
        assert_eq!(
            parsed_base_functionality("BASEFUNC_Buy||BASEFUNC_Sell"),
            BASEFUNC_DEFAULT,
            "an empty element invalidates the whole naming"
        );
    }

    #[test]
    fn base_functionality_numeric_tail_round_trips_cpp_decompiler_output() {
        let serialized = format_base_functionality(65).expect("nondefault mask serializes");
        assert_eq!(serialized, "BASEFUNC_AutoSellContents|64");
        assert_eq!(parsed_base_functionality(&serialized), 65);
    }

    #[test]
    fn map_zoom_defaults_clamp_and_rnd_zero_draw_like_cpp() {
        // C4SLandscape::Default: MapZoom = C4SVal(10, 0, 5, 15)
        // (C4Scenario.cpp:307,353); Evaluate stays within [Min, Max] and
        // still advances Random(1) when Rnd is zero.
        fn evaluate(entries: Option<&Vec<(String, String)>>) -> u32 {
            let mut rng = legacy_map_creation_rng(0);
            let count = rng.count;
            let zoom = legacy_map_zoom(entries, &mut rng);
            assert_eq!(rng.count, count + 1, "MapZoom always consumes one draw");
            zoom
        }

        assert_eq!(evaluate(None), 10, "absent key uses the C4S default");
        let entries = vec![("MapZoom".to_string(), "8".to_string())];
        assert_eq!(evaluate(Some(&entries)), 8);
        let entries = vec![("MapZoom".to_string(), "1".to_string())];
        assert_eq!(evaluate(Some(&entries)), 5, "clamped to Min=5");
        let entries = vec![("MapZoom".to_string(), "99".to_string())];
        assert_eq!(evaluate(Some(&entries)), 15, "clamped to Max=15");
    }

    #[test]
    fn objects_dir_values_round_trip_as_raw_ints_like_cpp() {
        // C4Action::CompileFunc reads and writes Dir as a plain int with no
        // validation (C4Action.cpp:45-54). Multi-directional definitions use
        // values beyond DIR_Right; Dragon Rock contains Dir=13 banners.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\nAction=Float\nDir=8\nComDir=5\n\n[Object]\nid=GOOD\nNumber=11\nStatus=1\nCategory=0\nX=20\nY=20\nAction=Float\nDir=13\nComDir=5\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .object_snapshot(ObjectId::new(10))
                .expect("Dir=8 object loaded")
                .direction
                .to_script_value(),
            8
        );
        assert_eq!(
            engine
                .object_snapshot(ObjectId::new(11))
                .expect("Dir=13 object loaded")
                .direction
                .to_script_value(),
            13
        );
    }

    #[test]
    fn objects_comdir_values_round_trip_as_raw_ints_like_cpp() {
        // C4Action::CompileFunc reads and writes ComDir as a plain int with
        // no range validation (C4Action.cpp:45-54). Shipped scenarios use
        // ComDir=200 for persisted WDWB objects.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\nComDir=200\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .object_snapshot(ObjectId::new(10))
                .expect("ComDir=200 object loaded")
                .command_direction
                .to_script_value(),
            200
        );
    }

    #[test]
    fn standard_crew_with_clonks_count_spawns_native_crew_like_cpp() {
        // [PlayerN] `Clonks=` is the C4SVal crew COUNT — `Crew` in
        // C4SPlrStart, default C4SVal(1,0,1,10) (C4Scenario.cpp:261,279) —
        // and `StandardCrew=` names the native crew def (NativeCrew, :278).
        // It is NOT a crew-ID list ('Clonks=5,0,1,10' must not become
        // "unknown definition `5,0,1,10`").
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=NativeCrew\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nStandardCrew=GOOD\nClonks=2,0,1,10\nPosition=120,160\n",
        )
        .expect("write scenario core");
        // Old-spec PlaceReadyCrew (C4Player.cpp:489-526) evaluates the
        // count with a synced draw and places NativeCrew members at JOIN
        // time — nothing spawns at load.
        let (mut engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 0, "no crew at load");

        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tester".to_string(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("join succeeds");
        let snapshot = engine.snapshot();
        let crew: Vec<_> = snapshot
            .objects
            .iter()
            .filter(|object| object.owner == 0 && object.crew_member)
            .collect();
        assert_eq!(crew.len(), 2, "Clonks Std=2 native crew at join");
        for object in &crew {
            assert_eq!(object.definition_id, "GOOD");
        }
    }

    #[test]
    fn definition_collection_truncates_and_skips_invalid_c4ids() {
        fn write_definition(path: &Path, id: &str) {
            std::fs::create_dir_all(path).expect("definition directory");
            std::fs::write(
                path.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("write DefCore");
            std::fs::write(path.join("Script.c"), "// definition\n").expect("write script");
            write_test_definition_graphics(path);
        }

        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Defs.c4d");
        write_definition(&root.join("Lowercase.c4d"), "Clonk");
        std::fs::write(root.join("Lowercase.c4d/ActMap.txt"), "not an action map")
            .expect("write malformed skipped ActMap");
        write_definition(&root.join("Lowercase.c4d/Child.c4d"), "CHLD");
        write_definition(&root.join("Long.c4d"), "CLONKX");
        write_definition(&root.join("Numeric.c4d"), "1337");
        write_definition(&root.join("Zero.c4d"), "0000");
        write_definition(&root.join("Hud.c4d"), "3HUD");

        let group = Group::open(&root).expect("definition root opens");
        let language_packs = LanguagePacks::default();
        let mut sound_effect_groups = Vec::new();
        let mut collected = Vec::new();
        collect_definitions_from_group(
            &group,
            false,
            &HashSet::new(),
            &["US"],
            &language_packs,
            &group,
            None,
            &mut sound_effect_groups,
            &mut collected,
        )
        .expect("invalid IDs skip without aborting the definition tree");
        let mut ids = collected
            .into_iter()
            .filter_map(|item| match item {
                CollectedDefinition::Definition(definition) => Some(definition.id),
                CollectedDefinition::SystemScripts(_) | CollectedDefinition::Particle(_) => None,
            })
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, ["1337", "3HUD", "CHLD", "CLON"]);
    }

    #[test]
    fn definition_sound_groups_follow_native_load_effects_gates_and_preorder() {
        fn write_definition(path: &Path, id: &str, needed_gfx_mode: Option<i32>) {
            std::fs::create_dir_all(path).expect("definition directory");
            let mut core = format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n");
            if let Some(mode) = needed_gfx_mode {
                core.push_str(&format!("NeededGfxMode={mode}\n"));
            }
            std::fs::write(path.join("DefCore.txt"), core).expect("write DefCore");
            std::fs::write(path.join("Script.c"), "// definition\n")
                .expect("write definition script");
            write_test_definition_graphics(path);
        }

        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("Defs.c4d");
        std::fs::create_dir_all(&root).expect("definition root");

        let pure = root.join("Pure.c4d");
        std::fs::create_dir_all(&pure).expect("pure sound group");
        std::fs::write(pure.join("Pure.wav"), b"pure").expect("pure sound");

        let good = root.join("Good.c4d");
        write_definition(&good, "GOOD", None);
        let nested = good.join("Nested.c4d");
        std::fs::create_dir_all(&nested).expect("nested pure sound group");
        std::fs::write(nested.join("Nested.wav"), b"nested").expect("nested sound");

        let particle = root.join("Particle.c4d");
        std::fs::create_dir_all(&particle).expect("particle group");
        std::fs::write(particle.join("Particle.txt"), b"invalid particle data")
            .expect("particle core");

        let invalid = root.join("Invalid.c4d");
        write_definition(&invalid, "0000", None);
        let invalid_oldgfx = root.join("InvalidOldGfx.c4d");
        write_definition(&invalid_oldgfx, "0000", Some(2));
        let rejected_core = root.join("RejectedCore.c4d");
        std::fs::create_dir_all(&rejected_core).expect("rejected core group");
        std::fs::write(
            rejected_core.join("DefCore.txt"),
            "[DefCore]\nName=Missing ID\nCategory=0\n",
        )
        .expect("rejected DefCore");

        let skipped = root.join("Skipped.c4d");
        write_definition(&skipped, "SKIP", None);
        let broken_graphics = root.join("BrokenGraphics.c4d");
        std::fs::create_dir_all(&broken_graphics).expect("broken definition group");
        std::fs::write(
            broken_graphics.join("DefCore.txt"),
            "[DefCore]\nid=BRKN\nName=Broken\nCategory=0\n",
        )
        .expect("broken definition core");
        let broken_act_map = root.join("BrokenActMap.c4d");
        write_definition(&broken_act_map, "BACT", None);
        std::fs::write(broken_act_map.join("ActMap.txt"), "not an action map")
            .expect("broken ActMap");

        let ordinary = root.join("Ordinary");
        std::fs::create_dir_all(&ordinary).expect("ordinary directory");
        std::fs::write(ordinary.join("Hidden.wav"), b"hidden").expect("hidden sound");

        let group = Group::open(&root).expect("definition root opens");
        let mut sound_effect_groups = Vec::new();
        let mut collected = Vec::new();
        collect_definitions_from_group(
            &group,
            false,
            &HashSet::from(["SKIP".to_string()]),
            &["US"],
            &LanguagePacks::default(),
            &group,
            None,
            &mut sound_effect_groups,
            &mut collected,
        )
        .expect("collect definition sound events");

        let roots = sound_effect_groups
            .iter()
            .map(|group| group.root().to_path_buf())
            .collect::<Vec<_>>();
        assert_eq!(roots.first(), Some(&root), "the root loads before children");
        for expected in [&pure, &good, &nested, &particle, &invalid, &rejected_core] {
            assert!(
                roots.contains(expected),
                "missing sound event for {}",
                expected.display()
            );
        }
        let good_index = roots.iter().position(|path| path == &good).unwrap();
        let nested_index = roots.iter().position(|path| path == &nested).unwrap();
        assert!(
            good_index < nested_index,
            "parents load sounds before children"
        );
        for excluded in [
            &invalid_oldgfx,
            &skipped,
            &broken_graphics,
            &broken_act_map,
            &ordinary,
        ] {
            assert!(
                !roots.contains(excluded),
                "unexpected sound event for {}",
                excluded.display()
            );
        }
    }

    #[test]
    fn child_definition_enumeration_preserves_distinct_native_names_in_group_order() {
        const U_CHILD_NAME: &[u8] = b"Gr\xfcn.C4D";
        const O_CHILD_NAME: &[u8] = b"Gr\xf6n.C4D";

        let graphics = encode_indexed_bmp(&[&[0x83]]);
        let definition = |id: &str| {
            let mut child = clonk_resources::MutableGroup::new("definition.bin");
            child
                .add_file(
                    "DefCore.txt",
                    format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n").into_bytes(),
                )
                .expect("add child DefCore");
            child
                .add_file("Script.c", format!("// {id}\n").into_bytes())
                .expect("add child script");
            child
                .add_file("Graphics.bmp", graphics.clone())
                .expect("add child graphics");
            child
        };

        // A .bin writer has no stock C4Group sort list, so this deliberately
        // non-lexical insertion order is the stored native order under test.
        let mut packed = clonk_resources::MutableGroup::new("native-order.bin");
        packed
            .add_child_bytes_with_metadata(U_CHILD_NAME.to_vec(), definition("UDEF"), 1, false)
            .expect("add U child definition");
        packed
            .add_child_bytes_with_metadata(O_CHILD_NAME.to_vec(), definition("ODEF"), 1, false)
            .expect("add O child definition");
        let group = Group::from_raw_memory(
            PathBuf::from("Definitions.c4d"),
            packed.pack_raw().expect("pack colliding child definitions"),
        )
        .expect("open colliding child definitions");
        let entries = group.entries().expect("enumerate child definitions");
        assert_eq!(entries[0].name_bytes, U_CHILD_NAME);
        assert_eq!(entries[1].name_bytes, O_CHILD_NAME);
        assert_ne!(entries[0].name_bytes, entries[1].name_bytes);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;

            assert_eq!(
                entries[0].relative_path.as_os_str().as_bytes(),
                U_CHILD_NAME
            );
            assert_eq!(
                entries[1].relative_path.as_os_str().as_bytes(),
                O_CHILD_NAME
            );
            assert_ne!(entries[0].relative_path, entries[1].relative_path);
        }

        let mut sound_effect_groups = Vec::new();
        let mut collected = Vec::new();
        collect_definitions_from_group(
            &group,
            false,
            &HashSet::new(),
            &["US"],
            &LanguagePacks::default(),
            &group,
            None,
            &mut sound_effect_groups,
            &mut collected,
        )
        .expect("collect exact colliding child definitions");
        assert_eq!(
            collected
                .iter()
                .filter_map(|item| match item {
                    CollectedDefinition::Definition(definition) => Some(definition.id.as_str()),
                    CollectedDefinition::SystemScripts(_) | CollectedDefinition::Particle(_) =>
                        None,
                })
                .collect::<Vec<_>>(),
            ["UDEF", "ODEF"]
        );
    }

    #[test]
    fn scenario_local_definition_children_load_and_override_packs() {
        // C++ loads the scenario group itself as the LAST definition source
        // with fOverload (C4Game::InitDefs): any .c4d child of the .c4s is
        // a definition, and it overrides same-id pack definitions
        // (Drachenfels.c4s carries Chest.c4d/_CST and friends directly).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        // A local def new to the scenario...
        let local = scenario_dir.join("Thing.c4d");
        std::fs::create_dir_all(&local).expect("local def dir");
        std::fs::write(
            local.join("DefCore.txt"),
            "[DefCore]\nid=THNG\nName=Thing\nCategory=0\nCrewMember=0\n",
        )
        .expect("write local defcore");
        std::fs::write(local.join("Script.c"), "func Tag() { return 5; }\n")
            .expect("write local script");
        write_test_definition_graphics(&local);
        // ...and a local override of the pack's GOOD definition.
        let shadow = scenario_dir.join("Good.c4d");
        std::fs::create_dir_all(&shadow).expect("shadow def dir");
        std::fs::write(
            shadow.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=LocalGood\nCategory=0\nCrewMember=0\n",
        )
        .expect("write shadow defcore");
        std::fs::write(shadow.join("Script.c"), "// local override\n")
            .expect("write shadow script");
        write_test_definition_graphics(&shadow);
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=THNG\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine.object_snapshot(ObjectId::new(10)).is_some(),
            "the local-child definition resolved for Objects.txt"
        );
        assert_eq!(
            engine
                .definitions
                .get("GOOD")
                .map(|definition| definition.name().to_string()),
            Some("LocalGood".to_string()),
            "the scenario-local definition overrides the pack's (fOverload)"
        );
    }

    #[test]
    fn folder_local_definitions_resolve_for_scenarios_like_cpp() {
        // C++ loads the parent folder chain as definition sources: a .c4d
        // inside the .c4f serves every scenario in the folder (Hazard.c4f/
        // ScenObjects.c4d provides _DIA to Tutorial.c4s).
        let dir = tempdir().expect("tempdir");
        let folder = dir.path().join("Pack.c4f");
        let shared = folder.join("Shared.c4d");
        std::fs::create_dir_all(&shared).expect("shared def dir");
        std::fs::write(
            shared.join("DefCore.txt"),
            "[DefCore]\nid=SHRD\nName=Shared\nCategory=0\nCrewMember=0\n",
        )
        .expect("write shared defcore");
        std::fs::write(shared.join("Script.c"), "// shared\n").expect("write shared script");
        write_test_definition_graphics(&shared);

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

        let scenario_dir = folder.join("Inner.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=FolderLocal\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=GOOD=1\nPosition=10,10\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=SHRD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        assert!(
            engine.object_snapshot(ObjectId::new(10)).is_some(),
            "the folder-local definition resolved for Objects.txt"
        );
    }

    #[test]
    fn folder_local_scan_checks_all_c4f_prefixes_outer_to_inner() {
        fn write_definition(path: &Path, id: &str) {
            std::fs::create_dir_all(path).expect("definition dir");
            std::fs::write(
                path.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("write defcore");
            std::fs::write(path.join("Script.c"), format!("// {id}\n")).expect("write script");
            write_test_definition_graphics(path);
        }

        let dir = tempdir().expect("tempdir");
        let outer = dir.path().join("Outer.c4f");
        let plain = outer.join("plain");
        let inner = plain.join("Inner.C4F");
        let seed_root = dir.path().join("seed-root");
        write_definition(&seed_root.join("Objects.c4d/Base.c4d"), "OBJS");
        write_definition(&outer.join("Outer.c4d"), "OUTR");
        write_definition(&outer.join("Skipped.c4d"), "SKIP");
        write_definition(&plain.join("NotAnAncestor.c4d"), "LEAK");
        write_definition(&inner.join("Inner.c4d"), "INNR");
        write_definition(&inner.join("nested/NotImmediate.c4d"), "DEEP");

        let scenario_dir = inner.join("Scenario.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Nested folders\n\n[Definitions]\n\
             LocalOnly=1\nDefinition1=MustNotResolve.c4d\nSkipDefs=SKIP\n",
        )
        .expect("write scenario core");

        let scenario = Scenario::load_from_path_with_languages_and_definition_seed(
            &scenario_dir,
            &FileSystemResolver {
                roots: vec![seed_root],
            },
            &["US"],
            &["Objects.c4d"],
        )
        .expect("local-only nested scenario loads");
        assert_eq!(
            scenario
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["OBJS", "OUTR", "INNR"],
            "LocalOnly keeps the startup seed, then immediate .c4f definitions load outer first"
        );
        assert_eq!(
            scenario.definition_resource_paths(),
            [
                dir.path().join("seed-root/Objects.c4d"),
                outer,
                inner,
            ],
            "the retained vector contains the external module and outer-to-inner folder groups, but not the scenario group"
        );
        let lobby_definitions = scenario
            .lobby_metadata()
            .expect("legacy lobby metadata")
            .definitions();
        assert!(lobby_definitions.is_local_only());
        assert_eq!(
            lobby_definitions.configured_modules(),
            ["MustNotResolve.c4d"]
        );
        assert_eq!(lobby_definitions.requested_modules(), ["Objects.c4d"]);
        assert_eq!(
            lobby_definitions.selection_source(),
            ScenarioDefinitionSelectionSource::CallerDefaults
        );
        assert_eq!(
            scenario
                .lobby_metadata()
                .expect("legacy lobby metadata")
                .game_parameter_resolution(),
            ScenarioGameParameterResolution::RequiresRuntimeConfiguration
        );
    }

    #[cfg(unix)]
    #[test]
    fn packed_group_path_reopens_native_byte_child_exactly() {
        const CHILD_NAME: &[u8] = b"Gr\xfcn.c4f";

        let mut child = clonk_resources::MutableGroup::new("native-child.bin");
        child
            .add_file("marker.txt", b"exact child".to_vec())
            .expect("add exact child marker");
        let mut outer = clonk_resources::MutableGroup::new("Outer.c4f");
        outer
            .add_child_bytes_with_metadata(CHILD_NAME.to_vec(), child, 1, false)
            .expect("add native-byte child");

        let directory = tempdir().expect("packed parent directory");
        let outer_path = directory.path().join("Outer.c4f");
        std::fs::write(&outer_path, outer.pack().expect("pack native-byte parent"))
            .expect("write native-byte parent");
        let outer = Group::open(&outer_path).expect("open native-byte parent");
        let entry = outer
            .entries()
            .expect("enumerate native-byte parent")
            .into_iter()
            .find(|entry| entry.name_bytes == CHILD_NAME)
            .expect("native-byte child entry");
        let child = outer
            .open_child_entry_exact(&entry)
            .expect("open native-byte child exactly");
        assert_eq!(
            child
                .root()
                .file_name()
                .expect("child path has a filename")
                .as_encoded_bytes(),
            CHILD_NAME
        );

        let reopened = open_group_path(child.root()).expect("reopen retained native-byte path");
        assert_eq!(reopened.read_file("marker.txt").unwrap(), b"exact child");
    }

    #[test]
    fn packed_nested_folder_locals_load_outer_to_inner_and_retained_paths_reload() {
        fn packed_definition(id: &str) -> Vec<u8> {
            let core = format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n");
            let graphics = encode_indexed_bmp(&[&[0x83]]);
            packed_test_group(&[
                ("DefCore.txt", false, core.as_bytes()),
                ("Script.c", false, b"// packed definition\n"),
                ("Graphics.bmp", false, graphics.as_slice()),
            ])
        }

        let dir = tempdir().expect("tempdir");
        let global = dir.path().join("Global.c4d");
        std::fs::create_dir_all(&global).expect("global definition dir");
        std::fs::write(
            global.join("DefCore.txt"),
            "[DefCore]\nid=GLOB\nName=GLOB\nCategory=0\n",
        )
        .expect("write global defcore");
        std::fs::write(global.join("Script.c"), "// global definition\n")
            .expect("write global script");
        write_test_definition_graphics(&global);

        let scenario_data = packed_test_group(&[(
            "Scenario.txt",
            false,
            b"[Head]\nTitle=Packed folders\n\n[Definitions]\nDefinition1=Global.c4d\n",
        )]);
        let inner_definition = packed_definition("INNR");
        let inner = packed_test_group(&[
            ("Inner.c4d", true, inner_definition.as_slice()),
            ("Payload.c4d", false, b"not a child group"),
            ("Nested.c4s", true, scenario_data.as_slice()),
        ]);
        let outer_definition = packed_definition("OUTR");
        let outer = packed_test_group_file(&[
            ("Outer.c4d", true, outer_definition.as_slice()),
            ("Corrupt.c4d", true, b"not a packed group"),
            ("Inner.c4f", true, inner.as_slice()),
        ]);
        let outer_path = dir.path().join("Outer.c4f");
        std::fs::write(&outer_path, outer).expect("write packed outer folder");

        let outer_group = Group::open(&outer_path).expect("open outer folder");
        let inner_group = outer_group
            .open_child("Inner.c4f")
            .expect("open inner folder");
        let scenario_group = inner_group
            .open_child("Nested.c4s")
            .expect("open packed scenario");
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario = Scenario::load_from_group_with(&scenario_group, &resolver)
            .expect("nested packed folder locals load");
        assert_eq!(
            scenario
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["GLOB", "OUTR", "INNR"]
        );
        let inner_path = outer_path.join("Inner.c4f");
        assert_eq!(
            scenario.definition_resource_paths(),
            [global.clone(), outer_path.clone(), inner_path.clone()]
        );

        let retained_modules = scenario
            .definition_resource_paths()
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let reloaded = Scenario::load_from_group_with_languages_and_seed_and_definition_modules(
            &scenario_group,
            &resolver,
            &["US"],
            0,
            &[],
            Some(&retained_modules),
            None,
        )
        .expect("retained packed paths reload as fixed resources");
        assert_eq!(
            reloaded.definition_resource_paths(),
            [
                global,
                outer_path.clone(),
                inner_path.clone(),
                outer_path,
                inner_path,
            ],
            "fixed restart restores the retained vector, then C++ appends folder locals again"
        );
    }

    #[test]
    fn scenario_local_system_c4g_installs_global_scripts() {
        // C4Game::LoadScenarioScripts (C4Game.cpp:3317-3343) loads every
        // script in the scenario's own System.c4g into the global script
        // engine AFTER definitions for overload priority — GoldRush's 31
        // dialogue/helper scripts and Drachenfels' constants rely on this.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("Local.c4s");
        let system = scenario_dir.join("System.c4g");
        std::fs::create_dir_all(&system).expect("system dir");
        std::fs::write(
            system.join("Helpers.c"),
            "#strict\nstatic const SYSTEM_VALUE = 42;\n\
             static const DERIVED_VALUE = SCENARIO_VALUE;\n\
             global func ScenarioLocalHelper() { return SYSTEM_VALUE(); }\n",
        )
        .expect("write helper script");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "#strict\nstatic const SYSTEM_VALUE = 31;\n\
             static const SCENARIO_VALUE = 40;\n",
        )
        .expect("write scenario script");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=LocalSystem\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        let good = dir.path().join("Defs.c4d").join("Good.c4d");
        std::fs::create_dir_all(&good).expect("definition dir");
        std::fs::write(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        )
        .expect("write defcore");
        std::fs::write(
            good.join("Script.c"),
            "#strict\nstatic const SYSTEM_VALUE = 17;\n\
             func Probe() { return ScenarioLocalHelper(); }\n",
        )
        .expect("write script");
        write_test_definition_graphics(&good);

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        assert!(
            engine
                .global_script_functions
                .as_ref()
                .is_some_and(|table| table.contains_key("ScenarioLocalHelper")),
            "scenario System.c4g functions reach the global script engine"
        );
        let id = engine
            .spawn_object(SpawnConfig::new("GOOD"))
            .expect("target spawns");
        let index = engine.find_object_index(id).expect("target index");
        assert_eq!(
            engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("Probe call succeeds"),
            clonk_script::Value::Int(42),
            "scenario System.c4g constants override definition constants"
        );
        assert_eq!(
            engine
                .script_global_consts
                .borrow()
                .get("DERIVED_VALUE")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(40)),
            "scenario System.c4g constants can reference earlier Script.c constants"
        );
    }

    #[test]
    fn scenario_script_globals_exist_before_environment_initialization() {
        // LoadScenarioScripts runs before LinkScriptEngine and InitGame's
        // InitEnvironment object creation (C4Game.cpp:2615-2622, 2493-2503).
        // Those object callbacks therefore see scenario Script.c globals.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic const SCENARIO_READY = 57;\n",
        );
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=ScenarioOrder\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapZoom=10\n\n[Environment]\nObjects=GOOD=1;\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[0u8, 0][..], &[0u8, 0][..]]),
        )
        .expect("write landscape");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nstatic initialized_from_scenario;\n\
             func Initialize() { initialized_from_scenario = SCENARIO_READY; }\n",
        )
        .expect("write definition script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("initialized_from_scenario")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(57))
        );
    }

    #[test]
    fn initialize_may_remove_its_own_object_like_cpp() {
        // Placer objects legitimately self-remove in Initialize (the
        // Environment Grass distributor calls RemoveObject() after placing,
        // Objects.c4d/Environment.c4d/Grass.c4d). C++ has no restriction;
        // the object simply ends removed.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "func Initialize() { RemoveObject(); return 1; }\n",
        )
        .expect("write self-removing script");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tester".to_string(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("the join itself succeeds");
        let lingering = engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.crew_member && object.status == ObjectStatus::Normal);
        assert!(!lingering, "the object ends removed like C++");
    }

    #[test]
    fn create_object_of_unknown_definition_is_nil_not_fatal() {
        // C++ CreateObject resolves the id with C4Id2Def and returns
        // nullptr when it is unknown — never an engine error.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "func Initialize() { CreateObject(\"XXXX\", 0, 0, -1); return 1; }\n",
        )
        .expect("write spawning script");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let joined = join_test_player(&mut engine);
        assert_eq!(joined.len(), 1, "only the crew member itself spawns");
        assert!(engine.object_snapshot(joined[0]).is_some());
    }

    #[test]
    fn orphan_container_references_spawn_uncontained_like_cpp() {
        // C++ creates all Objects.txt objects first and resolves Contained
        // by number afterwards (denumeration): a missing container leaves
        // the object uncontained — never a load failure. Drachenfels/
        // Hammerfest hit this when a container's definition is skipped.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\nContained=999\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine
            .object_snapshot(ObjectId::new(10))
            .expect("the object spawned");
        assert_eq!(
            snapshot.container, None,
            "missing container resolves to uncontained (nullptr denumeration)"
        );
    }

    #[test]
    fn objects_txt_unknown_definitions_are_skipped_like_cpp() {
        // C++ creates Objects.txt objects via C4Id2Def per entry; an unknown
        // id simply produces no object (logged), the rest of the scenario
        // loads. 19 real scenarios reference defs outside their resolver
        // scope and must not hard-fail.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=MISS\nNumber=9\nStatus=1\nCategory=0\nX=1\nY=2\n\n[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine.object_snapshot(ObjectId::new(10)).is_some(),
            "the known object spawned"
        );
        assert!(
            engine.object_snapshot(ObjectId::new(9)).is_none(),
            "the unknown-definition object was skipped"
        );
    }

    #[test]
    fn objects_txt_tolerates_windows_1252_like_cpp() {
        // C++ reads Objects.txt as raw bytes (the config charset); a
        // Windows-1252 umlaut must not abort the load
        // (Fantasy.c4f/Drachenfels.c4s fails strict UTF-8 today).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let mut objects = Vec::new();
        objects.extend_from_slice(
            b"# M\xe4dchen\n[Object]\nid=GOOD\nNumber=10\nStatus=1\nCategory=0\nX=10\nY=20\n",
        );
        std::fs::write(scenario_dir.join("Objects.txt"), objects).expect("write objects");
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(engine.object_snapshot(ObjectId::new(10)).is_some());
    }

    #[test]
    fn definition_script_parse_errors_are_logged_not_fatal_like_cpp() {
        // C4Def::Load ignores the Script.Load result (C4Def.cpp:632): a
        // definition whose Script.c fails to parse still loads — script-less
        // — and the rest of the scenario is unaffected.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some(("BRKN", "func {{{ not a script\n")),
            "global func Initialize(state, random) { return 0; }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            join_test_player(&mut engine).len(),
            1,
            "the good crew member still spawns at join"
        );
        assert!(
            engine.definitions.contains_key("BRKN"),
            "the broken-script definition is registered script-less (C4Def.cpp:632)"
        );
        assert!(engine.definitions.contains_key("GOOD"));
    }

    #[test]
    fn unknown_defcore_bit_names_do_not_abort_scenario_definition_scan() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let bits = dir.path().join("Defs.c4d/Bits.c4d");
        std::fs::create_dir(&bits).expect("bits definition directory");
        std::fs::write(
            bits.join("DefCore.txt"),
            "[DefCore]\nid=BITS\nCategory=C4D_Bogus|C4D_Object\n\
             LineConnect=Nonsense|C4D_PowerInput\n",
        )
        .expect("write unknown-token DefCore");
        write_test_definition_graphics(&bits);
        let tail = dir.path().join("Defs.c4d/Tail.c4d");
        std::fs::create_dir(&tail).expect("tail definition directory");
        std::fs::write(
            tail.join("DefCore.txt"),
            "[DefCore]\nid=TAIL\nCategory=C4D_Object\n",
        )
        .expect("write trailing DefCore");
        write_test_definition_graphics(&tail);

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let (scenario, warnings) = capture_definition_warnings(|| {
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads")
        });
        let warning_count = |bit_name| {
            warnings
                .iter()
                .filter(|warning| {
                    warning.message.as_deref() == Some("unknown definition bit name")
                        && warning.bit_name.as_deref() == Some(bit_name)
                })
                .count()
        };
        assert_eq!(warning_count("C4D_Bogus"), 1);
        assert_eq!(warning_count("Nonsense"), 1);
        let mut ids = Vec::new();
        scenario.visit_definition_groups(|id, _| ids.push(id.to_string()));
        ids.sort();
        assert_eq!(ids, ["BITS", "GOOD", "TAIL"]);

        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        assert!(engine.definitions.contains_key("BITS"));
        assert!(engine.definitions.contains_key("GOOD"));
        assert!(engine.definitions.contains_key("TAIL"));
        assert_eq!(engine.definition_category("BITS"), Some(1 << 4));
        assert_eq!(
            engine
                .definitions
                .get("BITS")
                .expect("BITS registered")
                .line_connect(),
            1
        );
    }

    #[test]
    fn construction_callback_errors_are_logged_not_fatal_like_cpp() {
        // Engine-initiated lifecycle calls are fail-safe in C++
        // (fPassErrors=false → the error logs and the call yields nil,
        // C4AulExec.cpp:1318-1342); the object still spawns.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        // Replace the good def's script with one whose Construction errors.
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "func Construction() { return NoSuchFunctionAnywhere(); }\n",
        )
        .expect("write erroring script");
        let (mut engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(created.len(), 0, "crew joins with the player, not at load");
        let joined = join_test_player(&mut engine);
        assert_eq!(
            joined.len(),
            1,
            "the crew object spawns despite the Construction error"
        );
        assert!(engine.object_snapshot(joined[0]).is_some());
    }

    #[test]
    fn scenario_initialize_errors_are_logged_not_fatal_like_cpp() {
        // The scenario script's Initialize is a game call (fail-safe): a
        // runtime error logs and the round starts anyway.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) { return BadCall(); }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine.scenario_script.is_some(),
            "the scenario script stays installed after the Initialize error"
        );
        assert_eq!(
            join_test_player(&mut engine).len(),
            1,
            "the round continues: a player can still join"
        );
    }

    #[test]
    fn definition_initialize_runs_before_scenario_initialize() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some((
                "INIT",
                "static init_def_calls;\n\
                 static init_def_section;\n\
                 static init_def_id;\n\
                 static init_def_value;\n\
                 static init_def_action_length;\n\
                 func InitializeDef(section) {\n\
                     init_def_calls = 1;\n\
                     init_def_section = section;\n\
                     init_def_id = GetID();\n\
                     init_def_value = GetDefCoreVal(\"Value\");\n\
                     init_def_action_length = GetActMapVal(\"Length\", \"Probe\");\n\
                     CreateObject(GOOD, 10, 20, -1);\n\
                     return 1;\n\
                 }\n",
            )),
            "static scenario_saw_init_def;\n\
             global func Initialize() {\n\
                 scenario_saw_init_def = init_def_calls;\n\
                 return 1;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/INIT.c4d/DefCore.txt"),
            "[DefCore]\nid=INIT\nName=Initializer\nCategory=0\nValue=23\n",
        )
        .expect("write initializer defcore");
        std::fs::write(
            dir.path().join("Defs.c4d/INIT.c4d/ActMap.txt"),
            "[Action]\nName=Probe\nProcedure=NONE\nLength=7\n",
        )
        .expect("write initializer actmap");

        let (engine, created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            created.len(),
            1,
            "InitializeDef's CreateObject is committed"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_section")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Nil),
            "a full scenario load passes a null section name"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_id")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::C4Id("INIT".to_string())),
            "GetID falls back to the no-object definition context"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_value")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(23))
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_action_length")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(7))
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("scenario_saw_init_def")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(1)),
            "scenario Initialize observes the completed definition phase"
        );
    }

    #[test]
    fn authoritative_team_configuration_precedes_definition_initialize() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some((
                "INIT",
                "#strict\nstatic init_def_team_configuration;\n\
                 func InitializeDef() {\n\
                     init_def_team_configuration = [GetTeamConfig(1), GetTeamConfig(2), GetTeamConfig(3), GetTeamConfig(4), GetTeamConfig(5), GetTeamConfig(6), GetTeamConfig(7)];\n\
                 }\n",
            )),
            "// no scenario script\n",
        );
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario
            .apply_before_network_final_init_with_team_configuration(
                &mut engine,
                crate::TeamConfiguration {
                    custom: true,
                    active: false,
                    allow_hostility_change: true,
                    distribution: 4,
                    allow_team_switch: true,
                    auto_generate_teams: false,
                    team_colors: true,
                },
            )
            .expect("network scenario applies");

        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_team_configuration")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Array(vec![
                clonk_script::Value::Int(1),
                clonk_script::Value::Int(0),
                clonk_script::Value::Int(1),
                clonk_script::Value::Int(4),
                clonk_script::Value::Int(1),
                clonk_script::Value::Int(0),
                clonk_script::Value::Int(1),
            ])),
            "InitializeDef observes synchronized Game.Parameters.Teams"
        );
    }

    #[test]
    fn network_savegame_runtime_state_precedes_initialize_def_and_skips_fresh_weather() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some((
                "PROB",
                "#strict\nstatic restored_named;\n\
                 static observed_saved_weather;\n\
                 func InitializeDef() {\n\
                     observed_saved_weather = [restored_named, GetWind(), GetSeason(), GetTemperature(), GetClimate()];\n\
                 }\n",
            )),
            "// no scenario script\n",
        );
        let core =
            std::fs::read_to_string(scenario_dir.join("Scenario.txt")).expect("read scenario core");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            core.replacen(
                "Title=Resilience",
                "Title=Resilience\nSaveGame=1\nNoInitialize=1",
                1,
            ),
        )
        .expect("mark fixture as a savegame");

        let mut gamma = crate::GammaControlState::default();
        assert!(gamma.set_ramp(0, [0x010203, 0x405060, 0xa0b0c0]));
        let scalar_game_data = InitialNetworkGameData {
            time: 91,
            frame: 37,
            control_tick: 12,
            sync_rate: 73,
            tick2: 1,
            tick3: 1,
            tick5: 2,
            tick10: 7,
            tick35: 2,
            tick255: 37,
            tick500: 37,
            tick1000: 37,
            object_enumeration_index: 4_000,
            rules: 1 | 2 | 4 | 8 | 16,
            play_list: "Saved*.ogg".to_string(),
            current_scenario_section: "Saved".to_string(),
            resort_any_object: true,
            music_enabled: true,
            music_level: 43,
            next_mission: crate::NextMissionState {
                path: "Next.c4s".to_string(),
                text: "Continue".to_string(),
                description: "Saved mission".to_string(),
            },
            message_board_commands: Vec::new(),
            script_go: true,
            script_counter: 9,
            environment: crate::EnvironmentSettings {
                wind: 73,
                wind_target: -21,
                season: 44,
                year_speed: 5,
                season_delay: 17,
                temperature: -8,
                temperature_range: 29,
                climate: 12,
                lightning: 3,
                meteorite: 4,
                volcano: 5,
                earthquake: 6,
                no_gamma: true,
                ..crate::EnvironmentSettings::default()
            },
            gamma,
            landscape: None,
            compiled_sections: Default::default(),
        };
        let mut game_source = crate::serialize_initial_network_game(&scalar_game_data, None)
            .expect("savegame runtime serializes")
            .expect("nondefault runtime emits Game.txt");
        let counter = b"Counter=9\r\n";
        let insertion = game_source
            .windows(counter.len())
            .position(|window| window == counter)
            .map(|position| position + counter.len())
            .expect("serialized Script counter");
        game_source.splice(
            insertion..insertion,
            b"Globals=2;i17,S0\r\nGlobalNamed=1;restored_named=i23\r\n"
                .iter()
                .copied(),
        );
        game_source.extend_from_slice(
            b"\r\n[Sky]\r\nX=65536\r\nY=-65536\r\nXDir=32768\r\nYDir=-32768\r\nModulation=4278255360\r\nParX=12\r\nParY=13\r\nParMode=1\r\nBackClr=-16711936\r\nBackClrEnabled=true\r\n\r\n\
[Effects]\r\nGlobalEffects=Fog(1,100,7,3,0,FOGG)[3;i5,b1,m[1;i7=S0]]\r\n\r\n\
[Scoreboard]\r\nRows=2\r\nCols=2\r\nDlgShow=1\r\nCell0_0String=\"Scores\"\r\nCell0_0Value=-1\r\nCell1_0String=\"Round\"\r\nCell1_0Value=1234\r\nCell0_1String=\"Alice\"\r\nCell0_1Value=7\r\nCell1_1String=\"42\"\r\nCell1_1Value=42\r\n",
        );
        let game_data = crate::parse_initial_network_game_data(&game_source);
        assert!(!game_source
            .windows(b"[Landscape]".len())
            .any(|window| window == b"[Landscape]"));
        std::fs::write(scenario_dir.join("Game.txt"), &game_source)
            .expect("write savegame runtime");
        std::fs::write(scenario_dir.join("Strings.txt"), b"saved text\r\n")
            .expect("write compiled string table");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let mut scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("savegame loads");
        // SaveGame alone selects Weather.Init(false). Keep this explicit
        // zero-runtime-landscape shape as a regression for the old Rust gate.
        scenario.runtime_landscape = None;
        let mut engine = Engine::with_seed(0);
        scenario
            .apply_before_network_final_init_with_game_data(&mut engine, &game_data, None, None)
            .expect("network savegame applies");

        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_saved_weather")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Array(vec![
                clonk_script::Value::Int(23),
                clonk_script::Value::Int(73),
                clonk_script::Value::Int(44),
                clonk_script::Value::Int(-8),
                clonk_script::Value::Int(12),
            ])),
            "InitializeDef observes compiled weather, not fresh Weather.Init values"
        );
        assert_eq!(engine.game_time(), 91);
        assert_eq!(engine.frame(), 37);
        assert_eq!(engine.control_tick, 12);
        assert_eq!(engine.sync_rate, 73);
        assert_eq!(engine.next_object_id, 4_001);
        assert_eq!(engine.environment.wind, 73);
        assert_eq!(engine.environment.wind_target, -21);
        assert_eq!(engine.environment.season, 44);
        assert_eq!(engine.environment.year_speed, 5);
        assert_eq!(engine.environment.season_delay, 17);
        assert_eq!(engine.environment.temperature, -8);
        assert_eq!(engine.environment.temperature_range, 29);
        assert_eq!(engine.environment.climate, 12);
        assert_eq!(engine.environment.lightning, 3);
        assert_eq!(engine.environment.meteorite, 4);
        assert_eq!(engine.environment.volcano, 5);
        assert_eq!(engine.environment.earthquake, 6);
        assert!(engine.environment.no_gamma);
        assert_eq!(engine.gamma_controls(), &gamma);
        assert_eq!(
            engine.debug_rng_clone(),
            crate::rng::LcgRng::seed_from_u64(0),
            "Weather.Init(false) consumes no fresh scenario draws"
        );
        assert!(engine.structures_need_energy);
        assert!(engine.construction_needs_material);
        assert!(engine.flag_removeable);
        assert!(engine.structures_snow_in);
        assert!(engine.team_home_base_rule);
        assert_eq!(engine.music_playlist(), "Saved*.ogg");
        assert_eq!(engine.music_level(), 43);
        assert!(engine.scenario_script_go);
        assert_eq!(engine.scenario_script_counter, 9);
        assert_eq!(
            engine
                .script_global_slots
                .borrow()
                .get(&0)
                .map(|value| value.borrow().clone()),
            Some(clonk_script::Value::Int(17))
        );
        assert_eq!(
            engine
                .script_global_slots
                .borrow()
                .get(&1)
                .map(|value| value.borrow().clone()),
            Some(clonk_script::Value::String("saved text".to_string().into()))
        );
        assert_eq!(engine.global_effects.len(), 1);
        assert_eq!(engine.global_effects[0].name, "Fog");
        assert_eq!(engine.global_effects[0].timer, 7);
        assert_eq!(engine.global_effects[0].interval, 3);
        assert_eq!(engine.global_effects[0].command_id.as_deref(), Some("FOGG"));
        assert_eq!(
            engine.global_effects[0].vars,
            vec![
                EffectVarValue::Int(5),
                EffectVarValue::Bool(true),
                EffectVarValue::Proplist(clonk_script::ValueMap::from([(
                    clonk_script::Value::Int(7),
                    clonk_script::Value::String("saved text".to_string().into()),
                )])),
            ]
        );
        let sky = engine
            .sky
            .as_ref()
            .expect("compiled sky restored")
            .snapshot();
        assert_eq!(sky.fixed, Some([65_536, -65_536, 32_768, -32_768]));
        assert_eq!(sky.settings.parallax_x, 12);
        assert_eq!(sky.settings.parallax_y, 13);
        assert_eq!(sky.settings.parallax_mode, SkyParallaxMode::Wind);
        assert_eq!(sky.settings.modulation, Some(4_278_255_360));
        assert_eq!(sky.settings.back_color_raw, 0xff00_ff00);
        assert_eq!(sky.settings.back_color, Some(0xff00_ff00));
        let scoreboard = engine.scoreboard_snapshot();
        assert_eq!((scoreboard.row_count(), scoreboard.column_count()), (2, 2));
        assert_eq!(scoreboard.show_count(), 1);
        assert_eq!(
            scoreboard.cell(1, 1).and_then(crate::ScoreboardCell::text),
            Some("42")
        );
        assert_eq!(
            scoreboard.cell(1, 1).map(crate::ScoreboardCell::value),
            Some(42)
        );
        assert!(engine.resort_any_object_pending());

        // A missing Game.txt skips C4Game::Compile altogether. Network GO
        // still has to stage InitSystem/C4Weather::Default state; otherwise
        // the Rust setup above would leak scenario-derived weather and rules
        // into a SaveGame whose Weather.Init(false) never replaces them.
        let mut default_engine = Engine::with_seed(0);
        scenario
            .apply_before_network_final_init_with_game_data(
                &mut default_engine,
                &InitialNetworkGameData::default(),
                None,
                None,
            )
            .expect("missing-Game defaults apply");
        assert_eq!(default_engine.environment.wind, 0);
        assert_eq!(default_engine.environment.wind_target, 0);
        assert_eq!(default_engine.environment.season, 0);
        assert_eq!(default_engine.environment.temperature, 0);
        assert!(default_engine.environment.no_gamma);
        assert!(!default_engine.structures_need_energy);
        assert!(!default_engine.construction_needs_material);
        assert!(!default_engine.flag_removeable);
        assert!(!default_engine.structures_snow_in);
        assert!(!default_engine.team_home_base_rule);
    }

    #[test]
    fn definition_initialize_uses_numeric_c4id_order() {
        // C4ID is a little-endian integer: ZAAA sorts before ABBB even
        // though lexical ordering says the opposite (C4DefList::SortByID).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some((
                "ZAAA",
                "static init_def_order;\n\
                 func InitializeDef() {\n\
                     if (!init_def_order) init_def_order = 0;\n\
                     init_def_order = init_def_order * 10 + 1;\n\
                 }\n",
            )),
            "// no scenario script\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/DefCore.txt"),
            "[DefCore]\nid=ABBB\nName=ABBB\nCategory=0\n",
        )
        .expect("write ABBB defcore");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "static init_def_order;\n\
             func InitializeDef() {\n\
                 if (!init_def_order) init_def_order = 0;\n\
                 init_def_order = init_def_order * 10 + 2;\n\
             }\n",
        )
        .expect("write ABBB script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_def_order")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(12))
        );
    }

    #[test]
    fn deferred_apply_synchronizes_before_initialize_and_player_join() {
        // InitPlayers only enqueues startup joins; InitGameFinal calls
        // Script.Initialize before the first control execution performs them
        // (C4Game.cpp:456-483, C4PlayerInfo.cpp:1292-1320). Initialize also
        // sees the RNG re-fixed by Synchronize, not the weather-init ledger.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static observed_players;\n\
             static observed_random;\n\
             global func Initialize() {\n\
                 observed_players = GetPlayerCount();\n\
                 observed_random = Random(100);\n\
                 return 1;\n\
             }\n",
        );
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);

        scenario
            .apply_before_players(&mut engine)
            .expect("pre-player apply succeeds");
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_players")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Nil),
            "Script.Initialize remains pending during the pre-player phase"
        );

        let mut synchronized_rng = engine.rng.clone();
        let expected_random = synchronized_rng.random(100);
        engine
            .initialize_scenario_script()
            .expect("scenario Initialize succeeds");
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_players")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(0)),
            "Initialize runs before queued startup-player joins"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_random")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(expected_random)),
            "Initialize consumes the synchronized RNG stream"
        );
        assert_eq!(engine.rng, synchronized_rng);

        assert_eq!(join_test_player(&mut engine).len(), 1);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("observed_players")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(0)),
            "the later player join does not retroactively rerun Initialize"
        );
    }

    #[test]
    fn scenario_initialize_may_return_an_int_like_real_content() {
        // C++ discards scenario-callback return values (Game.Script calls
        // run as bare statements): real scenarios `return(1)` from
        // Initialize, which must not abort the apply (two sweep scenarios
        // regressed on this once their Initialize ran to completion).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) { return 1; }\n",
        );
        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(engine.scenario_script.is_some());
    }

    #[test]
    fn scenario_script_parse_errors_are_logged_not_fatal_like_cpp() {
        // A scenario Script.c with no valid declarations still installs its
        // recovered (empty) script host after logging the parse error.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "global func {{{ broken\n");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert!(
            engine.scenario_script.is_some(),
            "the recovered scenario script host remains installed"
        );
        assert_eq!(
            join_test_player(&mut engine).len(),
            1,
            "the scenario still runs after the parse error"
        );
    }

    #[test]
    fn legacy_value_overloads_replace_definition_value_before_objects_load_like_cpp() {
        // C4Game::InitGame invokes InitValueOverloads immediately before
        // Objects.Load (C4Game.cpp:2704-2713); InitValueOverloads assigns the
        // configured count to C4Def::Value (C4Game.cpp:3997-4004).
        let dir = tempdir().expect("tempdir");
        let fish = dir.path().join("Defs.c4d/Fish.c4d");
        std::fs::create_dir_all(&fish).expect("definition dir");
        std::fs::write(
            fish.join("DefCore.txt"),
            "[DefCore]\nid=FISH\nName=Fish\nCategory=0\nValue=10\n",
        )
        .expect("write defcore");
        std::fs::write(fish.join("Script.c"), "// fish\n").expect("write definition script");
        write_test_definition_graphics(&fish);

        let scenario_dir = dir.path().join("ValueOverload.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Value overload\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Game]\nValueOverloads=FISH=20\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=FISH\nNumber=1\nStatus=1\nCategory=0\nX=10\nY=20\n",
        )
        .expect("write objects");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        assert_eq!(scenario.value_overloads, vec![("FISH".to_string(), 20)]);
        let mut engine = Engine::with_seed(0);
        let created = scenario.apply(&mut engine).expect("scenario applies");

        assert_eq!(
            created.len(),
            1,
            "the loaded object is created after overloads"
        );
        assert_eq!(engine.definition_value("FISH"), Some(20));
    }

    #[test]
    fn loads_legacy_scenario_with_definitions() {
        let dir = tempdir().expect("tempdir");

        let defs_root = dir.path().join("Defs.c4d");
        let foo_core = defs_root.join("Foo.c4d");
        std::fs::create_dir_all(&foo_core).expect("definition dir");
        std::fs::write(
            foo_core.join("DefCore.txt"),
            "[DefCore]\nid=FOOO\nName=Foo\nCategory=0\nCrewMember=0\nSolidMask=0,0,2,1,0,0\n",
        )
        .expect("write defcore");
        std::fs::write(foo_core.join("Script.c"), "// empty definition script\n")
            .expect("write definition script");
        RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 255]))
            .save(foo_core.join("Graphics.png"))
            .expect("write definition graphics");

        assert!(foo_core.join("DefCore.txt").exists(), "defcore exists");
        assert!(foo_core.join("Script.c").exists(), "script exists");

        let foo_group = Group::open(&foo_core).expect("open foo definition group");
        ResourceDefinitionData::load(&foo_group).expect("load foo definition");

        let scenario_dir = dir.path().join("LegacyScenario.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Test\nDisableMouse=1\nForcedAutoStopControl=1\nForcedAutoContextMenu=1\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Game]\nBaseFunctionality=BASEFUNC_Buy\n\n[Player1]\nCrew=FOOO=2\nPosition=120,160\n",
        )
        .expect("write legacy scenario core");
        std::fs::write(
            scenario_dir.join("Script.c"),
            "global func Initialize(state, random) { return 0; }\n",
        )
        .expect("write legacy scenario script");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario_group = Group::open(&scenario_dir).expect("open scenario group");
        resolver
            .resolve_definition_groups(&scenario_group, "Defs.c4d")
            .expect("resolve definition root");

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("legacy scenario loads");
        assert_eq!(scenario.name(), Some("Legacy Test"));
        // C4SHead::ForcedControlStyle is retained past scenario parsing for
        // C4Player::ApplyForcedControl at join (C4Player.cpp:2369-2389).
        assert_eq!(scenario.forced_control_style(), Some(true));
        // ForcedAutoContextMenu uses the same scenario-over-preference
        // precedence (C4Player.cpp:2369-2375).
        assert_eq!(scenario.forced_auto_context_menu(), Some(true));
        assert!(
            scenario.disables_mouse(),
            "C4SHead::DisableMouse must survive loading for C4Player::InitControl (C4Player.cpp:1907-1912)"
        );

        let mut engine = Engine::with_seed(0);
        let created = scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");
        assert_eq!(created.len(), 0, "crew joins with the player, not at load");
        assert!(
            !engine.base_reject_entrance_enabled,
            "the legacy BaseFunctionality mask disables RejectEntrance"
        );
        assert!(
            !engine.base_extinguish_enabled,
            "the legacy BaseFunctionality mask disables Extinguish"
        );
        // The `[Player1] Crew=FOOO=2` list places at JOIN
        // (C4Player::PlaceReadyCrew new spec, C4Player.cpp:528-570); the
        // exact placement positions are pinned by the draw-ledger test.
        let joined = join_test_player(&mut engine);
        assert_eq!(joined.len(), 2, "two ready-crew members at join");
        assert!(
            engine.player(0).expect("joined player").control_style(),
            "ForcedAutoStopControl=1 overrides the player's classic preference"
        );
        assert!(
            engine
                .player(0)
                .expect("joined player")
                .control
                .auto_context_menu,
            "ForcedAutoContextMenu=1 overrides the player's classic preference"
        );
        for id in &joined {
            let object = engine.object_snapshot(*id).expect("spawned object present");
            assert_eq!(object.definition_id, "FOOO");
            assert_eq!(object.owner, 0);
            assert!(
                object.crew_member,
                "legacy crew should be marked as crew member"
            );
        }
        let snapshot = engine.snapshot();
        assert!(
            snapshot.definition_categories.contains_key("FOOO"),
            "expected legacy definition to be registered"
        );

        let id = engine
            .spawn_object(SpawnConfig::new("FOOO"))
            .expect("spawn legacy definition");
        let object = engine
            .object_snapshot(id)
            .expect("object created from legacy definition");
        assert_eq!(object.definition_id, "FOOO");
        assert_eq!(
            engine.debug_solid_mask_override(id.as_u64()),
            Some(None),
            "C4Def::Load disables an out-of-bitmap base mask before object Init can clamp it"
        );
    }

    #[test]
    fn legacy_definitions_prune_future_versions_and_unmet_requirements_before_objects() {
        fn write_definition(
            root: &Path,
            id: &str,
            version: Option<&str>,
            require_def: Option<&str>,
        ) {
            let path = root.join(format!("{id}.c4d"));
            std::fs::create_dir_all(&path).expect("definition dir");
            let mut core = format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n");
            if let Some(version) = version {
                core.push_str(&format!("Version={version}\n"));
            }
            if let Some(require_def) = require_def {
                core.push_str(&format!("RequireDef={require_def}\n"));
            }
            std::fs::write(path.join("DefCore.txt"), core).expect("write DefCore");
            std::fs::write(path.join("Script.c"), "// definition script\n")
                .expect("write definition script");
            write_test_definition_graphics(&path);
        }

        let dir = tempdir().expect("tempdir");
        let definitions = dir.path().join("Defs.c4d");
        write_definition(&definitions, "GOOD", None, None);
        // CompareVersion ignores a non-positive candidate build.
        write_definition(&definitions, "WILD", Some("4,9,11,0,0"), None);
        write_definition(&definitions, "CURR", Some("4,9,11,0,362"), None);
        write_definition(&definitions, "FBLD", Some("4,9,11,0,363"), None);
        write_definition(&definitions, "FUTR", Some("5,0,0,0,0"), None);
        write_definition(&definitions, "DIRC", None, Some("MISS"));
        write_definition(&definitions, "CHNA", None, Some("CHNB"));
        write_definition(&definitions, "CHNB", None, Some("MISS"));
        write_definition(&definitions, "CASC", None, Some("FUTR"));
        write_definition(&definitions, "CYCA", None, Some("CYCB"));
        write_definition(&definitions, "CYCB", None, Some("CYCA"));

        let scenario_dir = dir.path().join("PrunedDefs.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Pruned definitions\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write Scenario.txt");
        std::fs::write(
            scenario_dir.join("Objects.txt"),
            "[Object]\nid=FUTR\nNumber=1\nStatus=1\nCategory=0\nX=10\nY=10\n\n\
             [Object]\nid=CHNA\nNumber=2\nStatus=1\nCategory=0\nX=15\nY=15\n\n\
             [Object]\nid=GOOD\nNumber=3\nStatus=1\nCategory=0\nX=20\nY=20\n",
        )
        .expect("write Objects.txt");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");

        let mut retained = scenario
            .definitions
            .iter()
            .map(|definition| definition.id.clone())
            .collect::<Vec<_>>();
        retained.sort();
        assert_eq!(retained, ["CURR", "CYCA", "CYCB", "GOOD", "WILD"]);
        assert_eq!(scenario.initial_spawns.len(), 1);
        assert_eq!(scenario.initial_spawns[0].config.definition_id, "GOOD");

        let mut engine = Engine::with_seed(0);
        let created = scenario.apply(&mut engine).expect("scenario applies");
        let mut registered = engine
            .definition_ids()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        registered.sort();
        assert_eq!(registered, ["CURR", "CYCA", "CYCB", "GOOD", "WILD"]);
        assert_eq!(created.len(), 1, "the future-version object is skipped");
        assert_eq!(
            engine
                .object_snapshot(created[0])
                .expect("surviving object exists")
                .definition_id,
            "GOOD"
        );
    }

    #[test]
    fn fixed_definition_modules_replace_preset_and_keep_selected_order() {
        let dir = tempdir().expect("tempdir");
        for (module, id) in [("First.c4d", "FIRS"), ("Second.c4d", "SCND")] {
            let definition = dir.path().join(module).join(format!("{id}.c4d"));
            std::fs::create_dir_all(&definition).expect("definition dir");
            std::fs::write(
                definition.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("write defcore");
            std::fs::write(definition.join("Script.c"), "// fixed module\n").expect("write script");
            write_test_definition_graphics(&definition);
        }

        let scenario_dir = dir.path().join("Fixed.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Fixed\n\n[Definitions]\nDefinition1=MissingPreset.c4d\n",
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario = Scenario::load_from_path_with_languages_and_definition_modules(
            &scenario_dir,
            &resolver,
            &["US"],
            &["Second.c4d", "First.c4d"],
        )
        .expect("fixed definition selection replaces the missing preset");
        assert_eq!(
            scenario
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["SCND", "FIRS"]
        );
        let lobby_definitions = scenario
            .lobby_metadata()
            .expect("legacy lobby metadata")
            .definitions();
        assert_eq!(
            lobby_definitions.configured_modules(),
            ["MissingPreset.c4d"]
        );
        assert_eq!(
            lobby_definitions.requested_modules(),
            ["Second.c4d", "First.c4d"]
        );
        assert_eq!(
            lobby_definitions.selection_source(),
            ScenarioDefinitionSelectionSource::FixedCallerSelection
        );
    }

    #[test]
    fn network_game_resources_replace_local_definition_and_material_discovery() {
        // A client binds every synchronized C4GameRes to its resolved local
        // file, retrieves the complete list, then loads NRT_Definitions in
        // list order and scenario-local + NRT_Material groups in list order
        // (pristine 9ffa0a5d C4GameParameters.cpp:73-79,255-271;
        // C4Game.cpp:80-101,876-952). Folder definitions were already added
        // to that list by the host and must not be rediscovered on the client
        // (C4Game.cpp:209-212).
        let dir = tempdir().expect("tempdir");
        let package = dir.path().join("Installed.c4f");
        let scenario_dir = package.join("Combined7.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Authoritative resources\nNetworkGame=true\n\n\
             [Definitions]\nDefinition1=MissingInstalled.c4d\n\n\
             [Landscape]\nMapZoom=10\nSky=ClientSky\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[0x83]]),
        )
        .expect("write landscape");

        let local_definition = package.join("Local.c4d");
        std::fs::create_dir_all(&local_definition).expect("local definition");
        std::fs::write(
            local_definition.join("DefCore.txt"),
            "[DefCore]\nid=LOCL\nName=Local\nCategory=0\n",
        )
        .expect("write local definition");
        let local_materials = package.join("Material.c4g");
        std::fs::create_dir_all(&local_materials).expect("local materials");
        std::fs::write(local_materials.join("TexMap.txt"), "3=Rock-Smooth\n")
            .expect("write local texmap");
        std::fs::write(
            local_materials.join("Rock.c4m"),
            "[Material]\nName=Rock\nDensity=100\n",
        )
        .expect("write local material");

        let network = dir.path().join("Network");
        let host_definitions = network.join("Objects.c4d");
        let host_definition = host_definitions.join("Host.c4d");
        std::fs::create_dir_all(&host_definition).expect("host definition");
        std::fs::write(
            host_definition.join("DefCore.txt"),
            "[DefCore]\nid=HOST\nName=Host\nCategory=0\n",
        )
        .expect("write host definition");
        write_test_definition_graphics(&host_definition);
        let folder_definitions = network.join("Tutorial.c4f");
        let folder_definition = folder_definitions.join("Folder.c4d");
        std::fs::create_dir_all(&folder_definition).expect("folder definition");
        std::fs::write(
            folder_definition.join("DefCore.txt"),
            "[DefCore]\nid=FOLD\nName=Folder\nCategory=0\n",
        )
        .expect("write folder definition");
        write_test_definition_graphics(&folder_definition);

        let map_materials = network.join("PackageMaterial.c4g");
        std::fs::create_dir_all(&map_materials).expect("map materials");
        std::fs::write(
            map_materials.join("TexMap.txt"),
            "OverloadMaterials\n3=Water-Liquid\n",
        )
        .expect("write authoritative texmap");
        std::fs::write(
            map_materials.join("PackStone.c4m"),
            "[Material]\nName=PackStone\nDensity=100\n",
        )
        .expect("write package material");
        write_test_texture(&map_materials, "Liquid");
        let global_materials = network.join("Material.c4g");
        std::fs::create_dir_all(&global_materials).expect("global materials");
        std::fs::write(global_materials.join("TexMap.txt"), "# global\n")
            .expect("write global texmap");
        std::fs::write(
            global_materials.join("Water.c4m"),
            "[Material]\nName=Water\nDensity=25\n",
        )
        .expect("write water material");

        let graphics = network.join("Graphics.c4g");
        std::fs::create_dir_all(&graphics).expect("graphics group");
        let mut sky_png = Vec::new();
        {
            use image::ImageEncoder as _;
            image::codecs::png::PngEncoder::new(&mut sky_png)
                .write_image(&[11, 22, 33, 255], 1, 1, ColorType::Rgba8.into())
                .expect("encode client sky");
        }
        std::fs::write(graphics.join("ClientSky.png"), sky_png).expect("write client sky");

        let definition_groups = [
            Group::open(&host_definitions).expect("open host definitions"),
            Group::open(&folder_definitions).expect("open folder definitions"),
        ];
        let material_groups = [
            Group::open(&map_materials).expect("open map materials"),
            Group::open(&global_materials).expect("open global materials"),
        ];
        let graphics_groups = [Group::open(&graphics).expect("open graphics")];
        let scenario = Scenario::load_network_from_path_with_languages_and_seed(
            &scenario_dir,
            &definition_groups,
            &material_groups,
            &graphics_groups,
            &["US"],
            0,
        )
        .expect("network scenario loads from authoritative resources");

        assert_eq!(
            scenario.definition_resource_paths(),
            [host_definitions, folder_definitions]
        );
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");
        assert!(engine.definition_ids().any(|id| id == "HOST"));
        assert!(engine.definition_ids().any(|id| id == "FOLD"));
        assert!(!engine.definition_ids().any(|id| id == "LOCL"));
        let sky = scenario.sky().expect("network sky config");
        let surface = sky.surface.as_ref().expect("client sky surface");
        assert_eq!(&surface.pixels()[..4], &[11, 22, 33, 255]);
        assert!(
            engine
                .landscape()
                .expect("landscape loaded")
                .is_liquid_at(5, 5),
            "the first authoritative TexMap admits Water from the second resource"
        );
    }

    #[test]
    fn modern_and_fixed_empty_definition_elements_reach_load_failure() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("EmptyElement.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Empty element\n\n[Definitions]\nDefinitions=\"\"\n",
        )
        .expect("write scenario core");
        let resolver = FileSystemResolver { roots: Vec::new() };

        let modern = Scenario::load_from_path_with_languages_and_definition_seed(
            &scenario_dir,
            &resolver,
            &["US"],
            &["Objects.c4d"],
        );
        assert!(matches!(
            modern,
            Err(ScenarioError::LegacyDefinitionNotFound { path }) if path.is_empty()
        ));

        let fixed = Scenario::load_from_path_with_languages_and_definition_modules(
            &scenario_dir,
            &resolver,
            &["US"],
            &[""],
        );
        assert!(matches!(
            fixed,
            Err(ScenarioError::LegacyDefinitionNotFound { path }) if path.is_empty()
        ));
    }

    #[test]
    fn non_fixed_definition_seed_is_replaced_by_non_local_preset() {
        let dir = tempdir().expect("tempdir");
        for (module, id) in [("Objects.c4d", "OBJS"), ("Preset.c4d", "PRST")] {
            let definition = dir.path().join(module).join(format!("{id}.c4d"));
            std::fs::create_dir_all(&definition).expect("definition dir");
            std::fs::write(
                definition.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("write defcore");
            write_test_definition_graphics(&definition);
        }
        let scenario_dir = dir.path().join("Preset.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Preset\n\n[Definitions]\nDefinition1=Preset.c4d\n",
        )
        .expect("write scenario core");

        let scenario = Scenario::load_from_path_with_languages_and_definition_seed(
            &scenario_dir,
            &FileSystemResolver {
                roots: vec![dir.path().to_path_buf()],
            },
            &["US"],
            &["Objects.c4d"],
        )
        .expect("scenario preset loads");
        assert_eq!(
            scenario
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["PRST"]
        );

        let empty_dir = dir.path().join("EmptyPreset.c4s");
        std::fs::create_dir_all(&empty_dir).expect("empty scenario dir");
        std::fs::write(
            empty_dir.join("Scenario.txt"),
            "[Head]\nTitle=Empty preset\n\n[Definitions]\nAllowUserChange=1\n",
        )
        .expect("write empty scenario core");
        let empty = Scenario::load_from_path_with_languages_and_definition_seed(
            &empty_dir,
            &FileSystemResolver {
                roots: vec![dir.path().to_path_buf()],
            },
            &["US"],
            &["Objects.c4d"],
        )
        .expect("empty preset keeps seed");
        assert_eq!(
            empty
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["OBJS"]
        );
    }

    #[test]
    fn shipped_objects_collision_prefers_the_global_external_group() {
        struct CollisionResolver {
            local: Group,
            global: Group,
        }

        impl LegacyDefinitionResolver for CollisionResolver {
            fn resolve_definition_groups(
                &self,
                _scenario: &Group,
                identifier: &str,
            ) -> Result<Vec<Group>, ScenarioError> {
                assert_eq!(identifier, "Objects.c4d");
                Ok(vec![self.global.clone(), self.local.clone()])
            }
        }

        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let content = repository.join("content");
        assert!(
            content.join("Tutorial.c4f/Tutorial01.c4s").is_dir(),
            "the initialized official content submodule must provide Tutorial01"
        );
        let scenario = Group::open(content.join("Tutorial.c4f/Tutorial01.c4s"))
            .expect("open shipped tutorial");
        let local = Group::open(content.join("Tutorial.c4f/Objects.c4d"))
            .expect("open shipped tutorial-local Objects.c4d");
        let global =
            Group::open(content.join("Objects.c4d")).expect("open shipped global Objects.c4d");
        let resolver = CollisionResolver {
            local,
            global: global.clone(),
        };

        let selected = resolve_one_definition_group(&scenario, &resolver, "Objects.c4d")
            .expect("global collision resolves");
        assert_eq!(selected.root(), global.root());
    }

    #[cfg(unix)]
    #[test]
    fn definition_group_paths_restore_projected_non_ascii_bytes() {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

        let dir = tempdir().expect("tempdir");
        let scenario_path = dir.path().join("Scenario.c4s");
        std::fs::create_dir_all(&scenario_path).expect("scenario directory");
        let scenario = Group::open(&scenario_path).expect("open scenario group");

        let absolute_path = dir.path().join(std::ffi::OsString::from_vec(
            b"Absolute-\xe2\x98\x83.c4d".to_vec(),
        ));
        std::fs::create_dir_all(&absolute_path).expect("absolute definition directory");
        let absolute_spec =
            clonk_script::c4_string_from_bytes(absolute_path.as_os_str().as_bytes());
        let resolved_absolute = resolve_one_definition_group(
            &scenario,
            &FileSystemResolver { roots: Vec::new() },
            &absolute_spec,
        )
        .expect("absolute legacy-byte definition resolves");
        assert_eq!(resolved_absolute.root(), absolute_path);

        let relative_name = std::ffi::OsString::from_vec(b"Rooted-\xe2\x98\x85.c4d".to_vec());
        let rooted_path = dir.path().join(&relative_name);
        std::fs::create_dir_all(&rooted_path).expect("rooted definition directory");
        let relative_spec = clonk_script::c4_string_from_bytes(relative_name.as_bytes());
        let resolved_rooted = resolve_rooted_definition_group(dir.path(), &relative_spec)
            .expect("rooted legacy-byte definition resolves");
        assert_eq!(resolved_rooted.root(), rooted_path);
    }

    #[test]
    fn literal_definition_prefix_preserves_separatorless_and_dot_spellings() {
        fn write_pack(path: &Path, id: &str) {
            std::fs::create_dir_all(path).expect("definition pack");
            std::fs::write(
                path.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("definition core");
            std::fs::write(path.join("Script.c"), "// prefix fixture\n")
                .expect("definition script");
            write_test_definition_graphics(path);
        }

        let dir = tempdir().expect("tempdir");
        let prefix = dir.path().join("Defs");
        std::fs::create_dir_all(&prefix).expect("DefinitionPath activation directory");
        let prefixed_objects = dir.path().join("DefsObjects.c4d");
        let prefixed_preset = dir.path().join("Defs.").join("Preset.c4d");
        let original_objects = dir.path().join("Objects.c4d");
        let original_preset = dir.path().join("Preset.c4d");
        for (path, id) in [
            (&prefixed_objects, "POBJ"),
            (&prefixed_preset, "PSET"),
            (&original_objects, "OOBJ"),
            (&original_preset, "OPST"),
        ] {
            write_pack(path, id);
        }
        let scenario_path = dir.path().join("Prefix.c4s");
        std::fs::create_dir_all(&scenario_path).expect("scenario group");
        std::fs::write(
            scenario_path.join("Scenario.txt"),
            "[Head]\nTitle=Literal prefix\n\n[Definitions]\nDefinition1=Ignored.c4d\n",
        )
        .expect("scenario core");
        let scenario_group = Group::open(&scenario_path).expect("open scenario");
        let modules = vec!["Objects.c4d".to_owned(), "./Preset.c4d".to_owned()];
        let scenario =
            Scenario::load_from_group_with_languages_and_definition_selection_and_prefix(
                &scenario_group,
                &FileSystemResolver {
                    roots: vec![dir.path().to_path_buf()],
                },
                &["US"],
                &[] as &[String],
                Some(modules.as_slice()),
                Some(&prefix),
            )
            .expect("literal prefix vector loads");

        assert_eq!(
            scenario.definition_resource_paths(),
            [
                prefixed_objects,
                prefixed_preset,
                original_objects,
                original_preset,
            ]
        );
        let definitions = scenario.lobby_metadata().unwrap().definitions();
        assert_eq!(
            definitions.requested_modules(),
            ["Objects.c4d", "Preset.c4d"]
        );
        assert_eq!(
            definitions.requested_module_spellings(),
            ["Objects.c4d", "./Preset.c4d"]
        );
    }

    #[test]
    fn non_fixed_module_loads_one_external_then_folder_local_once() {
        fn write_definition(path: &Path, id: &str) {
            std::fs::create_dir_all(path).expect("definition dir");
            std::fs::write(
                path.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("write defcore");
            std::fs::write(path.join("Script.c"), "// definition\n").expect("write script");
            write_test_definition_graphics(path);
        }

        struct CollisionResolver {
            local: PathBuf,
            global: PathBuf,
        }

        impl LegacyDefinitionResolver for CollisionResolver {
            fn resolve_definition_groups(
                &self,
                _scenario: &Group,
                identifier: &str,
            ) -> Result<Vec<Group>, ScenarioError> {
                if identifier != "Objects.c4d" {
                    return Ok(Vec::new());
                }
                Ok(vec![Group::open(&self.global)?, Group::open(&self.local)?])
            }
        }

        let dir = tempdir().expect("tempdir");
        let folder = dir.path().join("Tutorial.c4f");
        let local_module = folder.join("Objects.c4d");
        let global_module = dir.path().join("Objects.c4d");
        write_definition(&local_module.join("Local.c4d"), "LOCL");
        write_definition(&global_module.join("Global.c4d"), "GLOB");
        let scenario_dir = folder.join("Tutorial01.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Collision\n\n[Definitions]\nDefinition1=Objects.c4d\n",
        )
        .expect("write scenario core");

        let scenario = Scenario::load_from_path_with(
            &scenario_dir,
            &CollisionResolver {
                local: local_module,
                global: global_module.clone(),
            },
        )
        .expect("collision scenario loads");
        assert_eq!(
            scenario
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["GLOB", "LOCL"]
        );
        assert_eq!(
            scenario.definition_resource_paths(),
            [global_module, folder],
            "the explicit vector contributes one global group and the folder-local pass contributes its parent once"
        );
    }

    #[test]
    fn fixed_modules_expand_rooted_block_then_original_block() {
        fn write_pack(root: &Path, id: &str) {
            let definition = root.join("Shared.c4d").join(format!("{id}.c4d"));
            std::fs::create_dir_all(&definition).expect("definition dir");
            std::fs::write(
                definition.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("write defcore");
            std::fs::write(definition.join("Script.c"), format!("// {id}\n"))
                .expect("write script");
            write_test_definition_graphics(&definition);
        }

        let dir = tempdir().expect("tempdir");
        let normal_first = dir.path().join("normal-first");
        let normal_second = dir.path().join("normal-second");
        let selector_root = dir.path().join("selector");
        write_pack(&normal_first, "PREF");
        write_pack(&normal_second, "OTHR");
        write_pack(&selector_root, "CSTM");

        let scenario_dir = dir.path().join("FixedOne.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Fixed one\n\n[Definitions]\nDefinition1=Missing.c4d\n",
        )
        .expect("write scenario core");
        let resolver = FileSystemResolver {
            roots: vec![normal_first.clone(), normal_second],
        };

        let normal = Scenario::load_from_path_with_languages_and_definition_modules(
            &scenario_dir,
            &resolver,
            &["US"],
            &["Shared.c4d"],
        )
        .expect("fixed module resolves");
        assert_eq!(
            normal
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["PREF"],
            "a fixed vector element loads only the resolver's first group"
        );

        let rooted = Scenario::load_from_path_with_languages_and_definition_modules_in_root(
            &scenario_dir,
            &resolver,
            &["US"],
            &["Shared.c4d", "Shared.c4d"],
            &selector_root,
        )
        .expect("selector root resolves");
        assert_eq!(
            rooted
                .definitions
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["CSTM", "PREF"],
            "rooted copies load before the retained original copies"
        );
        assert_eq!(
            rooted.definition_load_steps.len(),
            4,
            "each entry in both expanded blocks remains a distinct load event"
        );
        assert!(matches!(
            &rooted.definition_load_steps[0],
            DefinitionLoadStep::Declarations { name, .. } if name == "CSTM"
        ));
        assert!(matches!(
            &rooted.definition_load_steps[1],
            DefinitionLoadStep::Definition(id) if id == "CSTM"
        ));
        assert!(matches!(
            &rooted.definition_load_steps[2],
            DefinitionLoadStep::Declarations { name, .. } if name == "PREF"
        ));
        assert!(matches!(
            &rooted.definition_load_steps[3],
            DefinitionLoadStep::Definition(id) if id == "PREF"
        ));
        assert_eq!(
            rooted.definition_resource_paths(),
            [
                selector_root.join("Shared.c4d"),
                selector_root.join("Shared.c4d"),
                normal_first.join("Shared.c4d"),
                normal_first.join("Shared.c4d"),
            ],
            "the retained paths expose the exact rooted block followed by the original block"
        );
    }

    #[test]
    fn rooted_definition_loading_requires_rooted_and_original_copies() {
        fn write_pack(root: &Path, id: &str) {
            let definition = root.join("Shared.c4d").join(format!("{id}.c4d"));
            std::fs::create_dir_all(&definition).expect("definition dir");
            std::fs::write(
                definition.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\n"),
            )
            .expect("write defcore");
        }

        let dir = tempdir().expect("tempdir");
        let normal_root = dir.path().join("normal");
        write_pack(&normal_root, "NORM");
        let scenario_dir = dir.path().join("RequiredCopies.c4s");
        std::fs::create_dir_all(&scenario_dir).expect("scenario dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Required copies\n\n[Definitions]\nDefinition1=Ignored.c4d\n",
        )
        .expect("write scenario core");

        let missing_root = dir.path().join("missing-root");
        let rooted_missing = Scenario::load_from_path_with_languages_and_definition_modules_in_root(
            &scenario_dir,
            &FileSystemResolver {
                roots: vec![normal_root.clone()],
            },
            &["US"],
            &["Shared.c4d"],
            &missing_root,
        );
        assert!(matches!(
            rooted_missing,
            Err(ScenarioError::LegacyDefinitionNotFound { path })
                if path == missing_root.join("Shared.c4d").display().to_string()
        ));

        let rooted_only = dir.path().join("rooted-only");
        write_pack(&rooted_only, "ROOT");
        let original_missing =
            Scenario::load_from_path_with_languages_and_definition_modules_in_root(
                &scenario_dir,
                &FileSystemResolver { roots: Vec::new() },
                &["US"],
                &["Shared.c4d"],
                &rooted_only,
            );
        assert!(matches!(
            original_missing,
            Err(ScenarioError::LegacyDefinitionNotFound { path }) if path == "Shared.c4d"
        ));
    }
