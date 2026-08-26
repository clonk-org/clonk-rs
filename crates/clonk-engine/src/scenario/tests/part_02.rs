// Contiguous slice 2 of 8 of the `scenario/tests` battery, spliced
// by `include!` from the parent module so every test id is unchanged.

    #[test]
    fn game_save_title_uses_cpp_native_c_string_copy_semantics() {
        let core = LegacyScenarioCore::default();
        let mut capped_bytes = vec![0x80];
        capped_bytes.extend(std::iter::repeat_n(b'A', 510));
        capped_bytes.extend_from_slice(&[0xff, b'Z']);
        let capped_title = clonk_script::c4_string_from_bytes(&capped_bytes);

        for saved in [
            core.initial_network_save(&capped_title, &[], "", "", ""),
            core.initial_record_save(&capped_title, &[], "", "", ""),
            core.runtime_network_save(&capped_title, &[], "", "", ""),
        ] {
            assert_eq!(
                clonk_script::c4_string_bytes(&saved.head.title),
                capped_bytes[..C4_MAX_TITLE],
                "SCopy counts native bytes, including non-UTF-8 bytes"
            );
        }

        let nul_title = clonk_script::c4_string_from_bytes(b"Visible\0Hidden");
        for saved in [
            core.initial_network_save(&nul_title, &[], "", "", ""),
            core.initial_record_save(&nul_title, &[], "", "", ""),
            core.runtime_network_save(&nul_title, &[], "", "", ""),
        ] {
            assert_eq!(
                clonk_script::c4_string_bytes(&saved.head.title),
                b"Visible",
                "SCopy stops at the first native NUL"
            );
        }
    }

    #[test]
    fn game_save_set_modules_strips_cpp_paths_without_normalizing_separators() {
        let core = LegacyScenarioCore::default();
        let modules = vec![
            "/OPT/GAME/Objects.c4d".to_owned(),
            "/opt/game/Definitions/Pack.c4d".to_owned(),
            "definitions\\Already.c4d".to_owned(),
            "Relative\\Keep.c4d".to_owned(),
        ];
        let expected = vec![
            "Objects.c4d".to_owned(),
            "Pack.c4d".to_owned(),
            "definitions\\Already.c4d".to_owned(),
            "Relative\\Keep.c4d".to_owned(),
        ];

        for saved in [
            core.initial_network_save("Title", &modules, "/opt/game/", "Definitions/", ""),
            core.initial_record_save("Title", &modules, "/opt/game/", "Definitions/", ""),
            core.runtime_network_save("Title", &modules, "/opt/game/", "Definitions/", ""),
        ] {
            assert_eq!(saved.definitions.definitions, expected);
        }

        assert_eq!(
            set_legacy_definition_modules(
                &["/CUSTOM/Defs/Outside.c4d".to_owned()],
                "/opt/game/",
                "/custom/defs/",
            ),
            vec!["Outside.c4d".to_owned()],
            "an absolute DefinitionPath is still checked after ExePath"
        );

        let lower_umlaut_prefix = clonk_script::c4_string_from_bytes(b"/g\xe4me/");
        let upper_umlaut_module = clonk_script::c4_string_from_bytes(b"/G\xc4ME/Pack.c4d");
        assert_eq!(
            set_legacy_definition_modules(&[upper_umlaut_module], &lower_umlaut_prefix, "",),
            vec!["Pack.c4d".to_owned()],
            "SEqualNoCase applies the C++ CharCapital umlaut folding"
        );
    }

    #[test]
    fn initial_game_save_serializes_the_effective_modules_not_the_authored_reflection() {
        let core = parsed_scenario("[Definitions]\nDefinitions=Old.c4d\n").core;
        let modules = vec!["Effective.c4d".to_owned()];

        for saved in [
            core.initial_network_save("Title", &modules, "", "", ""),
            core.initial_record_save("Title", &modules, "", "", ""),
        ] {
            let serialized = String::from_utf8(saved.serialize()).test_value();
            assert!(serialized.contains("Definitions=\"Effective.c4d\"\r\n"));
            assert!(!serialized.contains("Old.c4d"));
        }
    }

    #[test]
    fn runtime_scenario_and_savegame_core_adjustments_match_cpp() {
        let core = parsed_scenario(
            "[Head]\nIcon=7\nTitle=Authored\nVersion=1,2,3,4,359\nSaveGame=1\nNoInitialize=0\nMissionAccess=MISS\nNetworkGame=true\nNetworkRuntimeJoin=true\nOrigin=Retained\\Game.c4s\n\n[Definitions]\nDefinitions=Old.c4d\n",
        )
        .core;

        let scenario = String::from_utf8(core.runtime_scenario_save().serialize()).test_value();
        for expected in [
            "Title=Authored\r\n",
            "Version=4,9,11,0,359\r\n",
            "NoInitialize=1\r\n",
            "NetworkRuntimeJoin=true\r\n",
            "Origin=Retained/Game.c4s\r\n",
            "Definitions=\"Old.c4d\"\r\n",
        ] {
            assert!(scenario.contains(expected), "missing {expected:?}");
        }
        for absent in ["SaveGame=1\r\n", "NetworkGame=true\r\n", "MissionAccess="] {
            assert!(!scenario.contains(absent), "retained {absent:?}");
        }

        let savegame = String::from_utf8(
            core.runtime_savegame(
                "Runtime",
                &["Objects.c4d".to_owned()],
                "",
                "",
                "Fallback.c4s",
                4,
            )
            .serialize(),
        )
        .test_value();
        for expected in [
            "Icon=4\r\n",
            "Title=Runtime\r\n",
            "SaveGame=1\r\n",
            "NoInitialize=1\r\n",
            "NetworkRuntimeJoin=true\r\n",
            "Origin=Retained/Game.c4s\r\n",
            "Definitions=\"Objects.c4d\"\r\n",
        ] {
            assert!(savegame.contains(expected), "missing {expected:?}");
        }
        assert!(!savegame.contains("NetworkGame=true\r\n"));
    }

    #[test]
    fn initial_network_scenario_rejects_json_without_faking_a_legacy_core() {
        // C4GameSaveNetwork serializes C4Scenario; a JSON-only Rust fixture
        // has no C++ C4Scenario equivalent to save (C4GameSave.cpp:58-108).
        let scenario = json_scenario_without_legacy_core();

        let error = scenario
            .serialize_initial_network_scenario("JSON", &[], "", "", "JSON.c4s")
            .expect_err("JSON scenarios must not fabricate Scenario.txt");

        assert!(matches!(
            error,
            ScenarioError::InitialNetworkScenarioUnsupported
        ));
    }

    #[test]
    fn initial_record_scenario_matches_cpp_initial_save_flags() {
        // C4GameSave::SaveCore resets NetworkGame, retains the initial
        // SaveGame/NoInitialize and NetworkRuntimeJoin fields, installs the
        // effective definition vector and origin, and updates only the first
        // four version components. C4GameSaveRecord::AdjustCore then sets
        // Replay, Icon and the already-formatted record title
        // (C4GameSave.cpp:58-108,576-584).
        let scenario = scenario_with_retained_legacy_core(
            "[Head]\nIcon=7\nTitle=Old\nVersion=1,2,3,4,359\nSaveGame=1\nNoInitialize=1\nMissionAccess=MISS\nNetworkGame=true\nNetworkRuntimeJoin=true\nOrigin=Retained\\Game.c4s\n\n[Definitions]\nDefinitions=Old.c4d\n\n[Game]\nStructNeedEnergy=0\nLandscapeInsertThrust=0\n",
        );

        let actual = scenario
            .serialize_initial_record_scenario(
                "007 Test [362]",
                &["Objects.c4d".to_owned(), "Folder.c4f".to_owned()],
                "",
                "",
                "Fallback.c4s",
            )
            .test_value();

        assert_eq!(
            actual,
            concat!(
                "[Head]\r\n",
                "Icon=29\r\n",
                "Title=007 Test [362]\r\n",
                "Version=4,9,11,0,359\r\n",
                "SaveGame=1\r\n",
                "Replay=1\r\n",
                "NoInitialize=1\r\n",
                "NetworkRuntimeJoin=true\r\n",
                "ForcedGfxMode=1\r\n",
                "Origin=Retained/Game.c4s\r\n",
                "\r\n",
                "[Definitions]\r\n",
                "Definitions=\"Objects.c4d\",\"Folder.c4f\"\r\n",
                "\r\n",
                "[Game]\r\n",
                "StructNeedEnergy=false\r\n",
                "LandscapeInsertThrust=0\r\n",
                "\r\n",
                "[Landscape]\r\n",
                "ShadeMaterials=false\r\n",
            )
            .as_bytes()
        );
    }

    #[test]
    fn initial_record_scenario_uses_fallback_origin_and_rejects_json() {
        let scenario = scenario_with_retained_legacy_core(
            "[Head]\nNetworkGame=true\nMissionAccess=MISS\n\n[Game]\nStructNeedEnergy=0\n",
        );
        let actual = scenario
            .serialize_initial_record_scenario("Record", &[], "", "", "Folder\\Game.c4s")
            .test_value();
        let actual = String::from_utf8(actual).test_value();

        assert!(actual.contains("Replay=1\r\n"));
        assert!(actual.contains("Icon=29\r\n"));
        assert!(actual.contains("LocalOnly=true\r\n"));
        assert!(actual.contains("Origin=Folder/Game.c4s\r\n"));
        assert!(!actual.contains("NetworkGame="));
        assert!(!actual.contains("MissionAccess="));

        let error = json_scenario_without_legacy_core()
            .serialize_initial_record_scenario("JSON", &[], "", "", "JSON.c4s")
            .expect_err("JSON scenarios must not fabricate record Scenario.txt");
        assert!(matches!(
            error,
            ScenarioError::InitialRecordScenarioUnsupported
        ));
    }

    #[test]
    fn initial_network_metadata_matches_cpp_scenario_defaults_and_conversion_order() {
        // C4Scenario::Load converts the old goal/rule selectors before
        // C4GameParameters::CompileFunc reads its scenario-derived defaults
        // (C4Scenario.cpp:86-97,503-540; C4GameParameters.cpp:555-568).
        let scenario = scenario_with_retained_legacy_core(
            "[Head]\nRandomSeed=12345\nMaxPlayer=7\nForcedNoCrew=1\nDefCrewStrength=42\n\
             \n[Definitions]\nDefinitions=\"First.c4d\",\"Second.c4d\"\n\
             \n[Game]\nMode=1\nCooperativeGoal=3\nValueGain=250\n\
             StructNeedMaterial=1\nStructNeedEnergy=1\nEnableRemoveFlag=1\nElimination=2\n\
             Goals=EXST=4;MELE=7\nRules=CTFL=9;EXST\n",
        );

        let metadata = scenario.initial_network_scenario_metadata().test_value();

        assert_eq!(
            metadata,
            InitialNetworkScenarioMetadata {
                icon: 18,
                definition_modules: vec!["First.c4d".to_owned(), "Second.c4d".to_owned()],
                random_seed: 12_345,
                max_players: 7,
                use_fair_crew: true,
                fair_crew_forced: true,
                fair_crew_strength: 42,
                rules: vec![
                    ScenarioIdListEntry::new("CTFL", 1),
                    ScenarioIdListEntry::new("EXST", 0),
                    ScenarioIdListEntry::new("CNMT", 1),
                    ScenarioIdListEntry::new("ENRG", 1),
                    ScenarioIdListEntry::new("FGRV", 1),
                ],
                goals: vec![
                    ScenarioIdListEntry::new("EXST", 4),
                    ScenarioIdListEntry::new("MELE", 1),
                    // Mode conversion calls ClearOldGoals first, so the
                    // following legacy ValueGain selector sees zero.
                    ScenarioIdListEntry::new("VALG", 1),
                ],
            }
        );
    }

    #[test]
    fn initial_network_metadata_uses_get_modules_and_normal_crew_semantics() {
        // GetModules hides definitions for LocalOnly, while ForcedNoCrew=2
        // means forced normal crew: FairCrewForced=true, UseFairCrew=false
        // (C4Scenario.cpp:456-459; C4Scenario.h:55-60).
        let scenario = scenario_with_retained_legacy_core(
            "[Head]\nForcedNoCrew=2\n\n[Definitions]\nLocalOnly=1\n\
             Definitions=Hidden.c4d\n\n[Game]\nStructNeedEnergy=0\n",
        );

        let metadata = scenario.initial_network_scenario_metadata().test_value();

        assert!(metadata.definition_modules.is_empty());
        assert!(!metadata.use_fair_crew);
        assert!(metadata.fair_crew_forced);
    }

    #[test]
    fn initial_network_metadata_rejects_json_without_legacy_core() {
        let error = json_scenario_without_legacy_core()
            .initial_network_scenario_metadata()
            .expect_err("JSON scenarios have no C++ scenario defaults");

        assert!(matches!(
            error,
            ScenarioError::InitialNetworkScenarioUnsupported
        ));
    }

    #[test]
    fn initial_network_team_metadata_matches_cpp_file_defaults_and_order() {
        // C4TeamList::CompileFunc and C4Team::CompileFunc preserve repeated
        // Team section order, apply their own file defaults, normalize
        // LastTeamID, and fill missing player-array entries with -1
        // (C4Teams.cpp:138-150,556-610).
        let mut scenario = scenario_with_retained_legacy_core("[Game]\nStructNeedEnergy=0\n");
        scenario.legacy_team_metadata = Some(
            parse_legacy_team_metadata_source(
                r#"[Teams]
            Active=0
            AllowTeamSwitch=1
            LastTeamID=1
            TeamDistribution=RandomInv
            TeamColors=1
            MaxScriptPlayers=3
            ScriptPlayerNames="Wolf|Bear"
            RandomTeamCount=2

              [Team]
              id=3
              Name=Third
              PlrStartIndex=2
              PlayerCount=3
              Players=9,4
              IconSpec="Portrait:KNIG::1"
              MaxPlayer=5

              [Team]
              id=1
              Name=First
              Color=1193046
            "#,
            )
            .test_value(),
        );

        assert_eq!(
            scenario
                .initial_network_team_metadata()
                .expect("legacy scenario exposes team metadata"),
            InitialNetworkTeamMetadata {
                active: false,
                custom: true,
                allow_hostility_change: false,
                allow_team_switch: true,
                auto_generate_teams: false,
                last_team_id: 3,
                team_distribution: InitialNetworkTeamDistribution::RandomInvisible,
                team_colors: true,
                max_script_players: 3,
                script_player_names: legacy_cstring(b"Wolf|Bear"),
                random_team_count: 2,
                teams: vec![
                    InitialNetworkTeam {
                        id: 3,
                        name: legacy_cstring(b"Third"),
                        player_start_index: 2,
                        player_ids: vec![9, 4, -1],
                        color: 0x00fc_f41c,
                        icon_spec: legacy_cstring(b"Portrait:KNIG::1"),
                        max_players: 5,
                    },
                    InitialNetworkTeam {
                        id: 1,
                        name: legacy_cstring(b"First"),
                        player_start_index: 0,
                        player_ids: Vec::new(),
                        color: 1_193_046,
                        icon_spec: LegacyCString::default(),
                        max_players: 0,
                    },
                ],
            }
        );
    }

    #[test]
    fn initial_network_team_metadata_applies_empty_file_and_missing_file_defaults() {
        // An empty existing Teams.txt compiles as custom/active and forces
        // AutoGenerateTeams. With no file, Load instead enables autogenerated
        // FFA only for the post-ConvertGoals melee/rivalry lists
        // (C4Teams.cpp:605-651; C4Scenario.cpp:503-540).
        let mut empty_file = scenario_with_retained_legacy_core("[Game]\nStructNeedEnergy=0\n");
        empty_file.legacy_team_metadata =
            Some(parse_legacy_team_metadata_source("[Teams]\n").test_value());
        let empty_file = empty_file.initial_network_team_metadata().test_value();
        assert!(empty_file.active);
        assert!(empty_file.custom);
        assert!(!empty_file.allow_hostility_change);
        assert!(empty_file.auto_generate_teams);
        assert!(empty_file.teams.is_empty());

        let cooperative = scenario_with_retained_legacy_core("[Game]\nStructNeedEnergy=0\n")
            .initial_network_team_metadata()
            .test_value();
        assert!(!cooperative.active);
        assert!(!cooperative.custom);
        assert!(cooperative.allow_hostility_change);
        assert!(!cooperative.auto_generate_teams);

        let melee = scenario_with_retained_legacy_core("[Game]\nMode=1\nStructNeedEnergy=0\n")
            .initial_network_team_metadata()
            .test_value();
        assert!(melee.active);
        assert!(!melee.custom);
        assert!(melee.allow_hostility_change);
        assert!(melee.auto_generate_teams);
        assert!(melee.teams.is_empty());
    }

    #[test]
    fn initial_network_team_metadata_rejects_json_without_legacy_scenario() {
        let error = json_scenario_without_legacy_core()
            .initial_network_team_metadata()
            .expect_err("JSON scenarios have no C++ team-load defaults");

        assert!(matches!(
            error,
            ScenarioError::InitialNetworkTeamMetadataUnsupported
        ));
    }

    #[test]
    fn initial_network_team_metadata_skips_empty_table_and_preserves_cpp_bytes() {
        // C4Group::LoadEntryString and C4LangStringTable::ReplaceStrings work
        // on bytes; C4Team::Name is then read into C4MaxName+1 with a 30-byte
        // limit (C4Group.cpp:2243-2260; C4LangStringTable.cpp:33-144;
        // C4Teams.cpp:138-150). CP1252 bytes must not silently become UTF-8.
        let dir = test_tempdir();
        write_test_file(
            dir.path().join("Teams.txt"),
            b"[Teams]\nScriptPlayerNames=\"$Roster$\"\n\n  [Team]\n  id=1\n  Name=$LocalizedTeam$\n  IconSpec=\"\\334\"\n",
        );
        let mut table = b"LocalizedTeam=".to_vec();
        table.extend(std::iter::repeat_n(0xdc, 35));
        table.extend_from_slice(b"\nRoster=");
        table.extend_from_slice(&[0xc4, b'|', 0xd6, b'\n']);
        write_test_file(dir.path().join("StringTbl.txt"), []);
        write_test_file(dir.path().join("StringTblUS.txt"), table);

        let group = Group::open(dir.path()).test_value();
        let (teams, loaded) =
            load_initial_network_teams(&group, &ComponentGroups::local(&group), &["US"])
                .test_value();
        let metadata = loaded.test_value().metadata;

        assert_eq!(clonk_script::c4_string_bytes(&teams[0].name), [0xdc; 30]);
        assert_eq!(metadata.teams[0].name.as_bytes(), &[0xdc; 30]);
        assert_eq!(metadata.teams[0].icon_spec.as_bytes(), &[0xdc]);
        assert_eq!(metadata.script_player_names.as_bytes(), &[0xc4, b'|', 0xd6]);
    }

    #[test]
    fn legacy_team_strings_preserve_cp1252_bytes_through_load_runtime_and_save() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        write_test_file(
            scenario_dir.join("Teams.txt"),
            b"[Teams]\nScriptPlayerNames=$Roster$\n  [Team]\n  id=1\n  Name=$TeamName$\n  Color=1\n  IconSpec=$TeamIcon$\n",
        );
        write_test_file(
            scenario_dir.join("StringTblUS.txt"),
            b"TeamName=Caf\xe9\nTeamIcon=Cr\xe8st:1\nRoster=Andr\xe9|Ren\xe9\n",
        );
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let scenario =
            Scenario::load_from_path_with_languages(&scenario_dir, &resolver, &["US"]).test_value();
        let expected_name = b"Caf\xe9";
        let expected_icon = b"Cr\xe8st:1";
        let expected_roster = b"Andr\xe9|Ren\xe9";
        assert_eq!(
            clonk_script::c4_string_bytes(&scenario.teams[0].name),
            expected_name
        );
        assert_eq!(
            clonk_script::c4_string_bytes(
                scenario.teams[0]
                    .icon_spec
                    .as_deref()
                    .expect("runtime icon spec"),
            ),
            expected_icon
        );
        let lobby_teams = scenario.lobby_metadata().test_value().teams();
        assert_eq!(
            clonk_script::c4_string_bytes(lobby_teams.teams()[0].name()),
            expected_name
        );
        assert_eq!(
            clonk_script::c4_string_bytes(
                lobby_teams.teams()[0].icon_spec().expect("lobby icon spec"),
            ),
            expected_icon
        );
        assert_eq!(
            clonk_script::c4_string_bytes(lobby_teams.script_player_names()),
            expected_roster
        );

        let engine = applied_test_scenario(&scenario);
        assert_eq!(
            clonk_script::c4_string_bytes(&engine.teams()[0].name),
            expected_name
        );
        assert_eq!(
            clonk_script::c4_string_bytes(
                engine.teams()[0]
                    .icon_spec
                    .as_deref()
                    .expect("engine icon spec"),
            ),
            expected_icon
        );

        let encoded = engine.capture_state().to_json_string().test_value();
        let state = crate::EngineState::from_json_str(&encoded).test_value();
        let mut restored = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut restored);
        restored.restore_state(&state).test_value();
        assert_eq!(
            clonk_script::c4_string_bytes(&restored.teams()[0].name),
            expected_name
        );
        assert_eq!(
            clonk_script::c4_string_bytes(
                restored.teams()[0]
                    .icon_spec
                    .as_deref()
                    .expect("restored icon spec"),
            ),
            expected_icon
        );
    }

    #[test]
    fn loader_head_uses_case_insensitive_scenario_entry_lookup() {
        let directory = test_tempdir();
        write_test_file(
            directory.path().join("sCeNaRiO.TxT"),
            "[Head]\nLoader=LoaderMixed*\n",
        );
        let group = Group::open(directory.path()).test_value();
        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(head.loader().configured_specification(), "LoaderMixed*");
    }

    #[test]
    fn legacy_landscape_entries_match_case_insensitively_in_directory_and_packed_groups() {
        let directory = test_tempdir();
        let entries: [(&str, &[u8]); 3] = [
            ("mAp.BmP", b"map"),
            ("lAnDsCaPe.BmP", b"landscape"),
            ("landscape.txt", b"script"),
        ];
        for (name, bytes) in entries {
            write_test_file(directory.path().join(name), bytes);
        }
        let directory_group = Group::open(directory.path()).test_value();

        let mut mutable = clonk_resources::MutableGroup::new("Case.c4s");
        for (name, bytes) in entries {
            mutable.add_file(name, bytes.to_vec()).test_value();
        }
        let packed_group = Group::from_memory(
            PathBuf::from("Case.c4s"),
            mutable.pack().expect("packed group image"),
        )
        .test_value();

        for (query, expected) in [
            ("Map.bmp", b"map".as_slice()),
            ("Landscape.bmp", b"landscape".as_slice()),
            ("Landscape.txt", b"script".as_slice()),
        ] {
            for group in [&directory_group, &packed_group] {
                assert_eq!(
                    try_read_group_file_case_insensitive(group, query)
                        .expect("entry lookup")
                        .as_deref(),
                    Some(expected),
                    "{query} resolves identically"
                );
            }
        }
        for group in [&directory_group, &packed_group] {
            assert_eq!(
                try_read_group_file_case_insensitive(group, "Missing.map").expect("missing lookup"),
                None
            );
        }
    }

    #[test]
    fn loader_head_retains_cpp_can_open_player_constraints() {
        let directory = scenario_test_root(concat!(
            "[Head]\n",
            "MinPlayer=0\n",
            "MaxPlayer=0\n",
            "SaveGame=1\n",
            "Replay=0\n",
            "MissionAccess=Secret\n",
            "\n",
            "[Game]\n",
            "Mode=1\n",
        ));
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(head.min_players(), 2, "melee derives a two-player floor");
        assert_eq!(head.max_players(), 0);
        assert!(head.is_save_game());
        assert!(!head.is_replay());
        assert_eq!(head.mission_access(), "Secret");
    }

    #[test]
    fn loader_head_retains_native_byte_and_capped_mission_access() {
        let directory = test_tempdir();
        let mut core = b"[Head]\nMissionAccess=Secr\x80t".to_vec();
        core.extend(std::iter::repeat_n(b'A', 520));
        core.extend_from_slice(b"\n");
        write_test_file(directory.path().join("Scenario.txt"), core);
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        let access = clonk_script::c4_string_bytes(head.mission_access());
        assert_eq!(&access[..6], b"Secr\x80t");
        assert_eq!(access.len(), 512, "C4MaxTitle truncates the fixed buffer");
    }

    #[test]
    fn loader_head_applies_subpath_origin_validation_and_separator_normalization() {
        let directory = scenario_test_root("[Head]\nOrigin=\\..\\Bad*?<>;|:A:B.c4s\n");
        let group = Group::open(directory.path()).test_value();
        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        let expected = if cfg!(windows) {
            "___\\Bad_______A_B.c4s"
        } else {
            "___/Bad_______A_B.c4s"
        };
        assert_eq!(head.origin(), Some(expected));

        write_test_file(directory.path().join("Scenario.txt"), "[Head]\nOrigin=\n");
        let empty = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(empty.origin(), Some("empty"));
    }

    #[test]
    fn loader_head_parses_only_raw_scenario_core_prefix_before_nul() {
        let directory = scenario_test_root(
            b"[Head]\nTitle=Visible\0\nLoader=LoaderHidden*\nOrigin=Hidden.c4f/Hidden.c4s\n",
        );
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(head.scenario_title(), "Visible");
        assert_eq!(head.loader().configured_specification(), "");
        assert_eq!(head.origin(), None);
    }

    #[test]
    fn loader_head_title_ignores_rust_manifest_and_uses_classic_precedence() {
        let directory = scenario_test_root("[Head]\nTitle=Legacy fallback\n");
        write_test_file(
            directory.path().join("Scenario.json"),
            r#"{"name":"Rust-only title","definitions":[]}"#,
        );
        write_test_file(
            directory.path().join("tItLeUs.TxT"),
            "US:Localized classic title\n",
        );
        let group = Group::open(directory.path()).test_value();
        let head =
            ScenarioLoaderHead::load_from_group_with_languages(&group, &["US", "DE"]).test_value();
        assert_eq!(head.scenario_title(), "Localized classic title");

        std::fs::remove_file(directory.path().join("tItLeUs.TxT")).test_value();
        let fallback = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(fallback.scenario_title(), "Legacy fallback");
    }

    #[test]
    fn loader_head_title_validation_trims_only_ascii_whitespace() {
        let em_space = '\u{2003}';
        let title = format!(" \t{em_space}Legacy{em_space}\r\n");
        assert_eq!(
            validate_name_ex_no_empty(title).expect("representable title"),
            format!("{em_space}Legacy{em_space}")
        );
    }

    #[test]
    fn initial_network_team_parser_matches_cpp_empty_case_and_raw_name_rules() {
        // A zero-byte entry makes C4Group::LoadEntryString fail and therefore
        // selects C4TeamList's missing-file branch. INI names and enum tokens
        // are case-sensitive; RCT_All skips leading spaces but preserves a
        // Name's trailing bytes (C4Group.cpp:2243-2259;
        // StdCompiler.cpp:498-532,692-714,794-865,897-998).
        let dir = test_tempdir();
        write_test_file(dir.path().join("Teams.txt"), []);
        let group = Group::open(dir.path()).test_value();
        let (_, loaded) =
            load_initial_network_teams(&group, &ComponentGroups::local(&group), &["US"])
                .test_value();
        assert!(loaded.is_none());

        let core = parsed_scenario("[Game]\nStructNeedEnergy=0\n").core;
        let (_, lobby_teams) =
            load_legacy_teams(&group, &ComponentGroups::local(&group), &["US"], &core).test_value();
        assert_eq!(
            lobby_teams.source(),
            ScenarioTeamsSource::DerivedScenarioDefault
        );
        assert!(!lobby_teams.is_active());
        assert!(!lobby_teams.is_custom());
        assert!(lobby_teams.allows_hostility_change());
        assert!(!lobby_teams.auto_generates_teams());

        let mut scenario = scenario_with_retained_legacy_core("[Game]\nStructNeedEnergy=0\n");
        scenario.legacy_team_metadata = loaded;
        assert_eq!(
            scenario.initial_team_configuration(),
            crate::TeamConfiguration {
                custom: false,
                active: false,
                allow_hostility_change: true,
                distribution: 0,
                allow_team_switch: false,
                auto_generate_teams: false,
                team_colors: false,
            }
        );

        let loaded = parse_legacy_team_metadata_source(concat!(
            "[Teams]\n",
            "active=0\n",
            "Active=TRUE\n",
            "Custom=false trailing bytes\n",
            "AllowTeamSwitch=1 trailing bytes\n",
            "TeamDistribution=randominv\n",
            "  [team]\n  id=99\n  Color=1\n",
            "  [Team]\n  ID=7\n  id=2\n  Name=  Tail  \n  Color=1\n",
        ))
        .test_value();
        assert!(loaded.metadata.active);
        assert!(!loaded.metadata.custom);
        assert!(loaded.metadata.allow_team_switch);
        assert_eq!(
            loaded.metadata.team_distribution,
            InitialNetworkTeamDistribution::Free
        );
        assert_eq!(loaded.metadata.teams.len(), 1);
        assert_eq!(loaded.metadata.teams[0].id, 2);
        assert_eq!(loaded.metadata.teams[0].name.as_bytes(), b"Tail  ");
        assert_eq!(parse_team_bool("true trailing"), Some(true));
        assert_eq!(parse_team_bool("0 trailing"), Some(false));
        assert_eq!(
            parse_team_distribution("Free trailing"),
            (Some(InitialNetworkTeamDistribution::Free), None)
        );
    }

    #[test]
    fn initial_network_team_metadata_typed_rejects_unknown_numeric_distribution() {
        let mut scenario = scenario_with_retained_legacy_core("[Game]\nStructNeedEnergy=0\n");
        scenario.legacy_team_metadata =
            Some(parse_legacy_team_metadata_source("[Teams]\nTeamDistribution=9\n").test_value());

        assert!(matches!(
            scenario.initial_network_team_metadata(),
            Err(ScenarioError::InitialNetworkTeamDistributionUnsupported { value: 9 })
        ));
    }

    #[test]
    fn initial_network_scenario_serializes_the_fully_converted_cpp_load_core() {
        // C4Scenario::Load mutates the loaded core through ConvertGoals before
        // C4GameSave::SaveCore later copies it (C4Scenario.cpp:86-97,503-545;
        // C4GameSave.cpp:58-108). Selector fields and obsolete objectives are
        // therefore absent from the initial dynamic's Scenario.txt.
        let scenario = scenario_with_retained_legacy_core(
            "[Game]\nMode=1\nCooperativeGoal=3\nValueGain=250\n\
             CreateObjects=WOOD=3\nClearObjects=ROCK=2\nClearMaterials=Gold=4\n\
             StructNeedMaterial=1\nStructNeedEnergy=1\nEnableRemoveFlag=1\nElimination=2\n\
             Goals=EXST=4;MELE=7\nRules=CTFL=9;EXST\n",
        );

        let serialized = scenario
            .serialize_initial_network_scenario(
                "Converted",
                &["Objects.c4d".to_owned()],
                "",
                "",
                "Converted.c4s",
            )
            .test_value();
        let serialized = String::from_utf8(serialized).test_value();

        assert!(serialized.contains(concat!(
            "[Game]\r\n",
            "StructNeedEnergy=false\r\n",
            "Goals=EXST=4;MELE=1;VALG=1\r\n",
            "Rules=CTFL=1;EXST=0;CNMT=1;ENRG=1;FGRV=1\r\n",
        )));
        for obsolete in [
            "Mode",
            "Elimination",
            "CooperativeGoal",
            "CreateObjects",
            "ClearObjects",
            "ClearMaterials",
            "ValueGain",
            "EnableRemoveFlag",
            "StructNeedMaterial",
        ] {
            assert!(
                !serialized
                    .lines()
                    .any(|line| line.starts_with(&format!("{obsolete}="))),
                "converted Scenario.txt retained obsolete {obsolete}:\n{serialized}"
            );
        }
    }

    #[test]
    fn initial_network_scenario_retains_initial_flags_and_existing_origin() {
        // fInitial skips the NoInitialize/SaveGame rewrite, and SaveCore only
        // populates an empty Origin (C4GameSave.cpp:65-75,93-101). It updates
        // only C4XVer[0..4), retaining the historical fifth component (:63-64).
        let scenario = scenario_with_retained_legacy_core(
            "[Head]\nTitle=Old\nVersion=4,9,10,15,359\nSaveGame=1\nNoInitialize=1\nMissionAccess=MISS\nOrigin=Retained\\Game.c4s\n\n[Game]\nLandscapeInsertThrust=0\n",
        );

        let actual = scenario
            .serialize_initial_network_scenario("New", &[], "", "", "Fallback.c4s")
            .test_value();

        assert_eq!(
            actual,
            concat!(
                "[Head]\r\n",
                "Title=New\r\n",
                "Version=4,9,11,0,359\r\n",
                "SaveGame=1\r\n",
                "NoInitialize=1\r\n",
                "NetworkGame=true\r\n",
                "ForcedGfxMode=1\r\n",
                "Origin=Retained/Game.c4s\r\n",
                "\r\n",
                "[Definitions]\r\n",
                "LocalOnly=true\r\n",
                "\r\n",
                "[Game]\r\n",
                "StructNeedEnergy=false\r\n",
                "LandscapeInsertThrust=0\r\n",
                "Rules=ENRG=1\r\n",
            )
            .as_bytes()
        );
    }

    #[test]
    fn loader_head_title_uses_classic_cr_before_lf_termination() {
        let directory = scenario_test_root("[Head]\nTitle=Fallback\n");
        write_test_file(
            directory.path().join("TitleUS.txt"),
            b"US:one\ntwo\rignored",
        );
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(head.scenario_title(), "one\ntwo");
    }

    #[test]
    fn loader_head_title_skips_zero_size_language_component() {
        let directory = scenario_test_root("[Head]\nTitle=Head fallback\n");
        write_test_file(directory.path().join("TitleUS.txt"), []);
        write_test_file(directory.path().join("Title.txt"), b"US:Plain fallback\n");
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(head.scenario_title(), "Plain fallback");
    }

    #[test]
    fn loader_head_title_ignores_component_data_after_nul() {
        let directory = scenario_test_root("[Head]\nTitle=Head fallback\n");
        write_test_file(
            directory.path().join("TitleUS.txt"),
            b"prefix\0US:Wrong suffix",
        );
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(head.scenario_title(), "Head fallback");
    }

    #[test]
    fn loader_head_title_decodes_legacy_cp1252_for_presentation() {
        let directory = scenario_test_root("[Head]\nTitle=Head fallback\n");
        write_test_file(directory.path().join("TitleUS.txt"), b"US:Caf\xe9\n");
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(head.scenario_title(), "Caf\u{e9}");
        assert_eq!(head.scenario_title_bytes(), b"Caf\xe9");
    }

    #[test]
    fn loader_head_fallback_title_preserves_native_cp1252_bytes() {
        let directory = scenario_test_root(b"[Head]\nTitle=S\xe4uresee\n");
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        assert_eq!(head.scenario_title(), "S\u{e4}uresee");
        assert_eq!(head.scenario_title_bytes(), b"S\xe4uresee");
    }

    #[test]
    fn loader_head_title_uses_classic_nonoverlapping_ssearch() {
        let directory = scenario_test_root("[Head]\nTitle=Head fallback\n");
        write_test_file(directory.path().join("TitleAA.txt"), b"AAA:Wrong");
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group_with_languages(&group, &["AA"]).test_value();
        assert_eq!(head.scenario_title(), "Head fallback");
    }

    #[test]
    fn loader_head_fallback_title_truncates_native_bytes_like_cpp() {
        let directory = test_tempdir();
        let title = format!("{}é", "A".repeat(119));
        write_test_file(
            directory.path().join("Scenario.txt"),
            format!("[Head]\nTitle={title}\n"),
        );
        let group = Group::open(directory.path()).test_value();

        let head = ScenarioLoaderHead::load_from_group(&group).test_value();
        let source = title.as_bytes();
        assert_eq!(head.scenario_title_bytes(), &source[..120]);
    }

    #[test]
    fn resource_registration_head_never_resolves_unrelated_title_data() {
        let directory = test_tempdir();
        let title = format!("{}é", "A".repeat(119));
        write_test_file(
            directory.path().join("Scenario.txt"),
            format!(
                "[Head]\nTitle={title}\nOrigin=Parent.c4s\n\
                 \n[Definitions]\nLocalOnly=1\nDefinition1=Objects.c4d\n"
            ),
        );
        let group = Group::open(directory.path()).test_value();

        let head =
            ScenarioLoaderHead::load_from_group_for_resource_registration(&group).test_value();
        assert_eq!(head.scenario_title(), "");
        assert_eq!(head.origin(), Some("Parent.c4s"));
        assert_eq!(head.configured_definition_modules(), ["Objects.c4d"]);
        assert!(head.local_only());
    }

    /// Builds the raw on-disk image of a tiny C4Group. This is intentionally
    /// local to scenario tests so nested DefinitionPath traversal is exercised
    /// through the real packed-group reader rather than a mock resolver.
    fn packed_test_group(entries: &[(&str, bool, &[u8])]) -> Vec<u8> {
        const HEADER_SIZE: usize = 204;
        const ENTRY_SIZE: usize = 316;
        const GROUP_FILE_ID: &[u8] = b"RedWolf Design GrpFolder";

        fn put_i32(buffer: &mut [u8], offset: usize, value: i32) {
            buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }

        let mut header = [0_u8; HEADER_SIZE];
        header[..GROUP_FILE_ID.len()].copy_from_slice(GROUP_FILE_ID);
        put_i32(&mut header, 28, 1);
        put_i32(&mut header, 32, 2);
        put_i32(&mut header, 36, i32::try_from(entries.len()).test_value());
        for byte in &mut header {
            *byte ^= 237;
        }
        for chunk in header.chunks_exact_mut(3) {
            chunk.swap(0, 2);
        }

        let mut image = header.to_vec();
        let mut data_offset = 0_usize;
        for (name, child, data) in entries {
            let mut entry = [0_u8; ENTRY_SIZE];
            entry[..name.len()].copy_from_slice(name.as_bytes());
            put_i32(&mut entry, 264, i32::from(*child));
            put_i32(&mut entry, 268, i32::try_from(data.len()).test_value());
            put_i32(&mut entry, 276, i32::try_from(data_offset).test_value());
            image.extend_from_slice(&entry);
            data_offset += data.len();
        }
        for (_, _, data) in entries {
            image.extend_from_slice(data);
        }
        image
    }

    /// Wraps a raw packed-group fixture in the standalone-file gzip envelope.
    /// Child entries stay raw because packed mother groups open them in place.
    fn packed_test_group_file(entries: &[(&str, bool, &[u8])]) -> Vec<u8> {
        let mut group = clonk_resources::MutableGroup::new("PackedFixture");
        for (name, child, data) in entries {
            if *child {
                group
                    .add_packed_child_with_metadata((*name).to_owned(), data.to_vec(), 0, 0, false)
                    .test_value();
            } else {
                group
                    .add_file((*name).to_owned(), data.to_vec())
                    .test_value();
            }
        }
        group.pack().test_value()
    }

    #[test]
    fn map_seed_replays_cpp_draw_after_randomize3() {
        // C4Game::FixRandom fills FRndBuf3 with 500 draws, then
        // C4Landscape::Init draws Random(3133700) for MapSeed
        // (C4Game.cpp:3554-3558; C4Landscape.cpp:563-579).
        assert_eq!(map_seed_from_random_seed(0), 59_893);
        assert_eq!(map_seed_from_random_seed(7), 42_711);
    }

    #[test]
    fn legacy_team_sections_keep_cpp_list_order_and_fields() {
        // C4TeamList::CompileFunc reads the repeated Team sections into its
        // array in file order (C4Teams.cpp:556-613).
        let teams = parse_legacy_teams_source(concat!(
            "[Teams]\n",
            "Active=1\n\n",
            "  [Team]\n",
            "  id=2\n",
            "  Name=Right\n",
            "  PlrStartIndex=2\n",
            "  Color=16053492\n",
            "  PlayerCount=3\n",
            "  Players=11,,12\n",
            "  IconSpec=TMS1:3\n\n",
            "  [Team]\n",
            "  id=1\n",
            "  Name=Left\n",
            "  Color=14699548\n",
            "  IconSpec=\"KNIG\"\n",
        ));
        assert_eq!(teams.teams.len(), 2);
        assert_eq!(teams.teams[0].id(), 2);
        assert_eq!(teams.teams[0].name(), "Right");
        assert_eq!(teams.teams[0].player_start_index(), 2);
        assert_eq!(teams.teams[0].player_count(), 3);
        assert_eq!(teams.teams[0].players(), [11, -1, 12]);
        assert_eq!(teams.teams[0].configured_color(), 16_053_492);
        assert_eq!(
            teams.teams[0].color(),
            ScenarioTeamColor::Explicit(16_053_492)
        );
        assert_eq!(teams.teams[0].icon_spec(), Some("TMS1:3"));
        assert_eq!(teams.teams[1].id(), 1);
        assert_eq!(teams.teams[1].name(), "Left");
        assert_eq!(teams.teams[1].configured_color(), 14_699_548);
        assert_eq!(teams.teams[1].icon_spec(), Some("KNIG"));

        let prefix_then_separator_mismatch = parse_legacy_teams_source(concat!(
            "[Teams]\n",
            "  [Team]\n",
            "  PlayerCount=2\n",
            "  Players=11x,12\n",
        ));
        assert_eq!(prefix_then_separator_mismatch.teams[0].players(), [11, -1]);
    }

    #[test]
    fn legacy_team_metadata_defaults_numbers_strtol_cannot_read() {
        // StdCompilerINIRead::ReadNum hands the raw text to strtol/strtoul and
        // picks base 16 only when the number itself starts `0x`
        // (StdCompiler.h:705-723), so an unprefixed hex literal reads no digits
        // at all. The resulting notFound is caught by the naming adaptor's
        // default handler (StdAdaptors.h:44-60, 99-131), leaving the field
        // default instead of failing the load. Clonkparty.c4s ships
        // `Color=fa1010`, and C4Team::CompileFunc defaults every one of these
        // fields to 0 (C4Teams.cpp:139-150, 556-579).
        let parsed = parse_legacy_team_metadata_source(concat!(
            "[Teams]\n",
            "Active=1\n",
            "LastTeamID=none\n",
            "MaxScriptPlayers=none\n",
            "RandomTeamCount=none\n",
            "\t[Team]\n",
            "\tid=1\n",
            "\tName=Spieler\n",
            "\tPlrStartIndex=none\n",
            "\tPlayerCount=none\n",
            "\tColor=fa1010\n",
            "\tMaxPlayer=none\n",
        ))
        .test_value();

        assert_eq!(parsed.metadata.last_team_id, 1, "raised by the team id");
        assert_eq!(parsed.metadata.max_script_players, 0);
        assert_eq!(parsed.metadata.random_team_count, 0);
        let team = &parsed.metadata.teams[0];
        assert_eq!(team.player_start_index, 0);
        assert_eq!(team.player_ids, Vec::<i32>::new());
        assert_eq!(team.max_players, 0);
        // The defaulted 0 then takes RecheckColor's id-indexed team colour
        // (C4Teams.cpp:181-218).
        assert_eq!(team.color, 0x00f4_0000);
        assert_eq!(parsed.random_color_team_id, None);
    }

    #[test]
    fn legacy_team_metadata_reads_colors_like_strtoul() {
        // dwClr is uint32_t, so ReadNum uses strtoul: a literal `0x` prefix
        // selects base 16, a leading sign keeps base 10, and a negative value
        // is negated modulo the unsigned range before the uint32_t store
        // (StdCompiler.cpp:648-653; C4Teams.cpp:147).
        let colors = |source: &str| {
            parse_legacy_team_metadata_source(source)
                .test_value()
                .metadata
                .teams
                .iter()
                .map(|team| team.color)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            colors(concat!(
                "[Teams]\n",
                "\t[Team]\n\tid=1\n\tColor=0xfa1010\n",
                "\t[Team]\n\tid=2\n\tColor=-1\n",
                "\t[Team]\n\tid=3\n\tColor=16053492;\n",
            )),
            [0x00fa_1010, 0xffff_ffff, 16_053_492]
        );
    }

    #[test]
    fn legacy_teams_bom_prefixed_root_header_is_skipped_like_cpp() {
        let plain = parse_legacy_teams_source(
            "[Teams]\nActive=0\nCustom=0\n  [Team]\n  id=7\n  Name=Visible\n",
        );
        assert!(!plain.active);
        assert!(!plain.custom);
        assert_eq!(plain.teams.len(), 1);
        assert_eq!(plain.teams[0].id(), 7);

        let bom = parse_legacy_teams_source(
            "\u{feff}[Teams]\r\nActive=0\r\nCustom=0\r\n  [Team]\r\n  id=7\r\n  Name=Hidden\r\n",
        );
        assert!(bom.active, "the skipped root header keeps file defaults");
        assert!(bom.custom);
        assert!(bom.teams.is_empty());
    }

    #[test]
    fn legacy_team_metadata_accepts_lf_crlf_and_cr_line_endings() {
        let lf = concat!(
            "[Teams]\n",
            "Active=0\n",
            "LastTeamID=7\n",
            "  [Team]\n",
            "  id=3\n",
            "  Name=Third\n",
            "  PlayerCount=2\n",
            "  Players=9,4\n",
        );
        let crlf = lf.replace('\n', "\r\n");
        let cr = lf.replace('\n', "\r");
        let expected = parse_legacy_team_metadata_source(&crlf).test_value();

        for (label, source) in [("LF", lf), ("CR", cr.as_str())] {
            let parsed = parse_legacy_team_metadata_source(source)
                .unwrap_or_else(|error| panic!("{label} exact Teams.txt parses: {error}"));
            assert_eq!(parsed.metadata, expected.metadata, "{label} metadata");
            assert_eq!(parsed.random_color_team_id, expected.random_color_team_id);
            assert_eq!(
                parsed.unsupported_team_distribution,
                expected.unsupported_team_distribution
            );
        }

        // An unreadable number defaults rather than failing, so the diagnostic
        // this pins comes from a Name that has no CP1252 byte to compile to.
        let invalid_lf = "[Teams]\n  [Team]\n  Name=\u{0100}\n";
        for (label, source) in [
            ("LF", invalid_lf.to_string()),
            ("CRLF", invalid_lf.replace('\n', "\r\n")),
            ("CR", invalid_lf.replace('\n', "\r")),
        ] {
            let error = parse_legacy_team_metadata_source(&source)
                .expect_err("invalid team name must fail");
            assert!(
                error.to_string().contains("Teams.txt line 3:"),
                "{label} diagnostics preserve physical line numbers: {error}"
            );
        }
    }

    #[test]
    fn legacy_lobby_projection_keeps_scenario_parameter_and_team_boundaries() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Lobby Probe\nLoader=\nMaxPlayer=7\nMinPlayer=0\n\
             SaveGame=1\nReplay=0\nForcedNoCrew=2\nDefCrewStrength=99\n\
             NetworkGame=1\nNetworkRuntimeJoin=1\n\n[Definitions]\n\
             AllowUserChange=1\nDefinition1=Defs.c4d\n\n[Game]\nMode=1\n",
        );
        write_test_file(
            scenario_dir.join("Parameters.txt"),
            concat!(
                "[Parameters]\n",
                "RandomSeed=44\n",
                "MaxPlayers=5\n",
                "MaxPlayers=broken-but-ignored\n",
                "StartupPlayerCount=3\n",
                "UseFairCrew=1\n",
                "FairCrewForced=0\n",
                "FairCrewStrength=1234\n",
                "AllowDebug=0\n",
                "IsNetworkGame=1\n",
                "ControlRate=4\n",
                "AutoFrameSkip=1\n",
                "Rules=RULE=2\n",
                "Goals=GOAL=3\n",
                "League=\"Cup\\nFinal\"\n\n",
                "  [Client]\n",
                "  ID=7\n",
                "  Activated=1\n",
                "  Observer=0\n",
                "  Name=\"Host\\tName\"\n",
                "  Nick=Host // literal\n",
                "  LobbyReady=1\n",
            ),
        );
        write_test_file(
            scenario_dir.join("Teams.txt"),
            concat!(
                "[Teams]\n",
                "Active=1\n",
                "Custom=1\n",
                "AllowHostilityChange=0\n",
                "AllowTeamSwitch=1\n",
                "AutoGenerateTeams=0\n",
                "LastTeamID=1\n",
                "TeamDistribution=RandomInv\n",
                "TeamColors=1\n",
                "MaxScriptPlayers=3\n",
                "ScriptPlayerNames=Alpha|Beta // literal\n",
                "RandomTeamCount=2\n\n",
                "  [Team]\n",
                "  id=2\n",
                "  Name=\"Second\" // literal\n",
                "  PlrStartIndex=2\n",
                "  PlayerCount=2\n",
                "  Players=11,12\n",
                "  IconSpec=\"TMS1:3\"\n",
                "  MaxPlayer=4\n\n",
                "  [Team]\n",
                "  id=12\n",
                "  Name=Twelfth\n",
            ),
        );
        write_test_file(
            scenario_dir.join("Game.txt"),
            "[DefinitionFiles]\nDefinition1=OldObjects.c4d\nDefinition2=OldPack.c4d\n\n[Game]\n",
        );
        // Named for what `line.substr(1)` asks the loader to open
        // (src/C4Game.cpp:3646).
        for (module, id) in [
            ("efinition1=OldObjects.c4d", "OLDA"),
            ("efinition2=OldPack.c4d", "OLDB"),
        ] {
            let definition = dir.path().join(module).join(format!("{id}.c4d"));
            std::fs::create_dir_all(&definition).test_value();
            write_test_file(
                definition.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\nCrewMember=0\n"),
            );
            write_test_file(definition.join("Script.c"), "// old\n");
            write_test_definition_graphics(&definition);
        }
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let scenario = load_test_scenario(&scenario_dir, &resolver);
        let lobby = scenario.lobby_metadata().test_value();
        let head = lobby.head();
        assert_eq!(head.configured_min_players(), 0);
        assert_eq!(head.min_players(), 2);
        assert_eq!(head.max_players(), 7);
        assert_eq!(head.max_players_league(), 7);
        assert!(head.is_save_game());
        assert!(!head.is_replay());
        assert_eq!(head.loader().configured_specification(), "");
        assert_eq!(head.loader().effective_specification(), "Loader*");
        assert_eq!(
            head.loader().selection(),
            ScenarioLoaderSelection::DeferredResourceSearch
        );
        assert_eq!(head.fair_crew_force(), ScenarioFairCrewForce::NormalCrew);
        assert_eq!(head.fair_crew_force().forced_use_fair_crew(), Some(false));
        assert_eq!(head.fair_crew_strength(), 99);
        assert_eq!(head.random_seed(), 0);
        assert!(head.was_network_game());
        assert!(head.allows_network_runtime_join());
        let parameter_defaults = lobby.game_parameter_defaults();
        assert_eq!(parameter_defaults.max_players(), 7);
        assert_eq!(parameter_defaults.control_rate(), -1);
        assert!(parameter_defaults
            .goals()
            .iter()
            .any(|entry| entry.id() == "MELE" && entry.count() == 1));

        let definitions = lobby.definitions();
        assert!(!definitions.is_local_only());
        assert!(definitions.allows_user_change());
        assert_eq!(definitions.configured_modules(), ["Defs.c4d"]);
        assert_eq!(definitions.requested_modules(), ["Defs.c4d"]);
        assert_eq!(
            definitions.selection_source(),
            ScenarioDefinitionSelectionSource::ScenarioPreset
        );
        assert!(!definitions.definition_root_applied());
        assert_eq!(
            definitions.effective_modules(),
            ["efinition1=OldObjects.c4d", "efinition2=OldPack.c4d"],
            "the override replaced the Defs.c4d preset above"
        );
        assert_eq!(definitions.resolved_load_resources().len(), 2);
        assert_eq!(
            definitions
                .savegame_override()
                .definition_lines()
                .expect("game definition lines"),
            ["Definition1=OldObjects.c4d", "Definition2=OldPack.c4d"]
        );

        let parameters = lobby.embedded_game_parameters().test_value();
        assert_eq!(parameters.random_seed(), Some(44));
        assert_eq!(parameters.max_players(), Some(5));
        assert_eq!(parameters.startup_player_count(), Some(3));
        assert_eq!(parameters.use_fair_crew(), Some(true));
        assert_eq!(parameters.fair_crew_forced(), Some(false));
        assert_eq!(parameters.fair_crew_strength(), Some(1234));
        assert_eq!(parameters.allow_debug(), Some(false));
        assert_eq!(parameters.is_network_game(), Some(true));
        assert_eq!(parameters.control_rate(), Some(4));
        assert_eq!(parameters.auto_frame_skip(), Some(true));
        assert_eq!(parameters.rules().expect("rules")[0].id(), "RULE");
        assert_eq!(parameters.rules().expect("rules")[0].count(), 2);
        assert_eq!(parameters.goals().expect("goals")[0].id(), "GOAL");
        assert_eq!(parameters.league(), Some("Cup\nFinal"));
        assert_eq!(parameters.clients().len(), 1);
        assert_eq!(parameters.clients()[0].id(), 7);
        assert_eq!(parameters.clients()[0].name(), "Host\tName");
        assert_eq!(parameters.clients()[0].nick(), "Host // literal");
        assert!(parameters.clients()[0].is_lobby_ready());
        assert_eq!(
            lobby.game_parameter_resolution(),
            ScenarioGameParameterResolution::EmbeddedFileBeforeRuntimeAdjustments
        );
        let merged = lobby.embedded_game_parameter_values().test_value();
        assert_eq!(merged.control_rate(), 4);
        assert_eq!(merged.max_players(), 5);

        let teams = lobby.teams();
        assert_eq!(teams.source(), ScenarioTeamsSource::TeamsFile);
        assert!(teams.is_active());
        assert!(teams.is_custom());
        assert!(!teams.allows_hostility_change());
        assert!(teams.allows_team_switch());
        assert!(!teams.configured_auto_generate());
        assert!(!teams.auto_generates_teams());
        assert_eq!(teams.configured_last_team_id(), 1);
        assert_eq!(teams.last_team_id(), 12);
        assert_eq!(
            teams.distribution(),
            ScenarioTeamDistribution::RandomInvisible
        );
        assert!(teams.uses_team_colors());
        assert_eq!(teams.max_script_players(), 3);
        assert_eq!(teams.script_player_names(), "Alpha|Beta // literal");
        assert_eq!(teams.random_team_count(), 2);
        assert_eq!(teams.teams().len(), 2);
        assert_eq!(teams.teams()[0].id(), 2);
        assert_eq!(teams.teams()[0].name(), "\"Second\" // literal");
        assert_eq!(teams.teams()[0].player_count(), 2);
        assert_eq!(teams.teams()[0].players(), [11, 12]);
        assert_eq!(teams.teams()[0].configured_max_players(), 4);
        assert_eq!(teams.teams()[0].max_players(), Some(4));
        assert_eq!(
            teams.teams()[0].color(),
            ScenarioTeamColor::DefaultForId(0x00_C8_00)
        );
        assert_eq!(
            teams.teams()[1].color(),
            ScenarioTeamColor::DeferredRuntimeRandom
        );

        let replay_core = std::fs::read_to_string(scenario_dir.join("Scenario.txt"))
            .test_value()
            .replace("SaveGame=1", "SaveGame=0")
            .replace("Replay=0", "Replay=1");
        write_test_file(scenario_dir.join("Scenario.txt"), replay_core);
        let replay = load_test_scenario(&scenario_dir, &resolver);
        let replay_lobby = replay.lobby_metadata().test_value();
        assert!(replay_lobby.head().is_replay());
        assert!(!replay_lobby.head().is_save_game());
        assert_eq!(
            replay_lobby.definitions().savegame_override(),
            &ScenarioSavegameDefinitionOverride::None
        );
        assert_eq!(replay_lobby.definitions().effective_modules(), ["Defs.c4d"]);
    }

    #[test]
    fn legacy_lobby_defaults_distinguish_min_players_and_missing_teams() {
        let cooperative = parsed_scenario(
            "[Head]\nMaxPlayer=5\nMaxPlayer=broken-but-ignored\n\n[Game]\nRules=RVLR=1\n",
        );
        assert_eq!(cooperative.core.head.max_player_league, 5);
        assert_eq!(legacy_effective_min_players(&cooperative.core), 1);
        let rivalry_teams = derive_legacy_teams_default(&cooperative.core);
        assert_eq!(
            rivalry_teams.source(),
            ScenarioTeamsSource::DerivedScenarioDefault
        );
        assert!(rivalry_teams.is_active());
        assert!(rivalry_teams.auto_generates_teams());
        assert!(!rivalry_teams.is_custom());

        let melee = parsed_scenario("[Head]\n\n[Game]\nGoals=MELE=1\n");
        assert_eq!(legacy_effective_min_players(&melee.core), 2);
        assert_eq!(melee.core.head.max_player, 12);
        assert_eq!(melee.core.head.max_player_league, 12);

        let duplicate_melee = parsed_scenario("[Head]\n\n[Game]\nGoals=MELE=0;MELE=1\n");
        assert_eq!(
            legacy_effective_min_players(&duplicate_melee.core),
            1,
            "C4IDList::GetIDCount uses the first duplicate"
        );
        assert!(!ScenarioValueStore::from_runtime_core(&duplicate_melee.core, false).is_melee());
        let duplicate_melee_reversed = parsed_scenario("[Head]\n\n[Game]\nGoals=MELE=1;MELE=0\n");
        assert_eq!(
            legacy_effective_min_players(&duplicate_melee_reversed.core),
            2
        );
        assert!(
            ScenarioValueStore::from_runtime_core(&duplicate_melee_reversed.core, false).is_melee()
        );

        let explicit = parsed_scenario("[Head]\nMinPlayer=-2\n");
        assert_eq!(legacy_effective_min_players(&explicit.core), -2);

        let unknown_force = ScenarioFairCrewForce::from_raw(7);
        assert_eq!(unknown_force, ScenarioFairCrewForce::Unknown(7));
        assert!(unknown_force.is_forced());
        assert_eq!(unknown_force.forced_use_fair_crew(), Some(false));

        let custom_loader = ScenarioLoaderMetadata {
            configured_specification: "RoundLoader*".to_string(),
        };
        assert_eq!(custom_loader.effective_specification(), "RoundLoader*");
    }

    #[test]
    fn evaluation_uses_melee_personal_gain_and_counts_profile_and_crew_rounds() {
        fn crew_info(
            name: &str,
            rounds: i32,
            total_playing_time: i32,
            in_action: bool,
            was_in_action: bool,
            in_action_time: i32,
        ) -> crate::player_file::CrewInfo {
            crate::player_file::CrewInfo {
                id: "CLNK".to_string(),
                name: name.to_string(),
                core: Default::default(),
                rank_name: "Clonk".to_string(),
                rounds,
                physical: Default::default(),
                total_playing_time,
                in_action,
                was_in_action,
                in_action_time,
                portraits: Default::default(),
                ..Default::default()
            }
        }

        fn evaluate(source: &str) -> Engine {
            let parsed = parsed_scenario(source);
            let mut engine = Engine::new();
            engine.set_scenario_values(ScenarioValueStore::from_runtime_core(&parsed.core, false));
            engine.register_test_player(
                crate::PlayerConfig::new(0, "Winner")
                    .with_score(250)
                    .with_rounds(4, 2, 2)
                    .with_initial_value(100),
            );
            engine.register_test_player(
                crate::PlayerConfig::new(1, "Loser")
                    .with_status(crate::PlayerStatus::Eliminated)
                    .with_score(80)
                    .with_rounds(7, 5, 2)
                    .with_initial_value(100),
            );
            engine.player_mut(0).test_value().update_asset_value(165, 0);
            engine.player_mut(1).test_value().update_asset_value(75, 0);
            engine.game_time = 20;
            engine.crew_rosters.insert(
                0,
                vec![
                    crew_info("Active", 3, 10, true, true, 5),
                    crew_info("Retired", 7, 17, false, true, 4),
                    crew_info("Unused", 9, 30, false, false, 0),
                ],
            );
            engine.evaluate_game().test_value();
            engine
        }

        let cooperative = evaluate("[Game]\n");
        let coop_winner = cooperative.player(0).test_value();
        let coop_loser = cooperative.player(1).test_value();
        assert_eq!((coop_winner.score(), coop_loser.score()), (382, 112));
        assert_eq!(
            (
                coop_winner.rounds(),
                coop_winner.rounds_won(),
                coop_winner.rounds_lost()
            ),
            (5, 3, 2)
        );
        assert_eq!(
            (
                coop_loser.rounds(),
                coop_loser.rounds_won(),
                coop_loser.rounds_lost()
            ),
            (8, 5, 3)
        );

        let mut melee = evaluate("[Game]\nMode=1\n");
        assert!(melee.scenario_values.is_melee());
        assert_eq!(melee.player(0).map(crate::Player::score), Some(415));
        assert_eq!(melee.player(1).map(crate::Player::score), Some(80));
        let roster = melee.crew_rosters.get(&0).test_value();
        assert_eq!((roster[0].rounds, roster[0].total_playing_time), (4, 25));
        assert!(!roster[0].in_action, "active crew is retired at evaluation");
        assert_eq!((roster[1].rounds, roster[1].total_playing_time), (8, 17));
        assert_eq!((roster[2].rounds, roster[2].total_playing_time), (9, 30));

        let before = melee.capture_state();
        melee.evaluate_game().test_value();
        assert_eq!(melee.capture_state().players, before.players);
        assert_eq!(
            melee.capture_state().crew_info_rosters,
            before.crew_info_rosters
        );
    }

    #[test]
    fn delayed_player_retirement_uses_the_same_melee_and_crew_evaluation() {
        let parsed = parsed_scenario("[Game]\nGoals=MELE=1\n");
        let mut engine = Engine::new();
        engine.set_scenario_values(ScenarioValueStore::from_runtime_core(&parsed.core, false));
        engine.register_test_player(
            crate::PlayerConfig::new(3, "Retiring")
                .with_status(crate::PlayerStatus::Eliminated)
                .with_score(10)
                .with_rounds(2, 1, 1)
                .with_initial_value(100),
        );
        engine.register_test_player(crate::PlayerConfig::new(4, "Peer").with_initial_value(100));
        engine.player_mut(3).test_value().update_asset_value(140, 0);
        engine.player_mut(4).test_value().update_asset_value(200, 0);
        engine.game_time = 12;
        engine.crew_rosters.insert(
            3,
            vec![crate::player_file::CrewInfo {
                id: "CLNK".to_string(),
                name: "Retiring crew".to_string(),
                rounds: 5,
                total_playing_time: 3,
                in_action: true,
                was_in_action: true,
                in_action_time: 2,
                ..Default::default()
            }],
        );

        let retired = engine.retire_player(3).test_value();
        assert_eq!(
            retired.score(),
            50,
            "melee uses personal gain 40, not average 70"
        );
        assert_eq!(
            (
                retired.rounds(),
                retired.rounds_won(),
                retired.rounds_lost()
            ),
            (3, 1, 2)
        );
        assert!(
            !engine.crew_rosters.contains_key(&3),
            "deleted retired player's C4ObjectInfoList does not persist"
        );
        assert_eq!(
            engine
                .round_results
                .players
                .iter()
                .find(|result| result.player_info_id == retired.player_info_id())
                .and_then(|result| result.score_new),
            Some(50)
        );
    }

    #[test]
    fn malformed_lobby_metadata_uses_cpp_defaults_and_exact_hierarchy() {
        let teams = parse_legacy_teams_source(concat!(
            "[Teams]\n",
            "MaxScriptPlayers=not-a-number\n",
            "maxscriptplayers=9\n",
            "TeamDistribution=7\n",
            "  [Team]\n",
            "  id=1\n",
            "  Name=Nested // retained\n",
            "  IconSpec= \"not escaped\"\n",
            "[Team]\n",
            "id=99\n",
        ));
        assert_eq!(teams.max_script_players(), 0);
        assert_eq!(teams.distribution(), ScenarioTeamDistribution::Numeric(7));
        assert_eq!(teams.teams().len(), 1, "unindented Team is a root sibling");
        assert_eq!(teams.teams()[0].name(), "Nested // retained");
        assert_eq!(teams.teams()[0].icon_spec(), Some("\"not escaped\""));

        let unknown_distribution = parse_legacy_teams_source("[Teams]\nTeamDistribution=random\n");
        assert_eq!(
            unknown_distribution.distribution(),
            ScenarioTeamDistribution::Free
        );

        let core = parsed_scenario("[Head]\nForcedNoCrew=2\n");
        let defaults = game_parameter_defaults(&core.core);
        let parameters = parse_legacy_game_parameter_overrides(
            concat!(
                "[Parameters]\n",
                "UseFairCrew=sometimes\n",
                "ControlRate=broken\n",
                "controlrate=8\n",
                "[Client]\n",
                "ID=99\n",
            ),
            &defaults,
        );
        assert_eq!(parameters.use_fair_crew(), Some(false));
        assert_eq!(parameters.control_rate(), Some(-1));
        assert!(
            parameters.clients().is_empty(),
            "unindented Client is ignored"
        );

        let empty_team_file = parse_legacy_teams_source("[Teams]\nAutoGenerateTeams=0\n");
        assert!(!empty_team_file.configured_auto_generate());
        assert!(empty_team_file.auto_generates_teams());
    }

    #[test]
    fn scenario_core_uses_exact_first_child_ini_semantics() {
        let manifest = parsed_scenario(concat!(
            "[Outer]\n",
            "  [Head]\n",
            "  MaxPlayer=99\n",
            "[Head]\n",
            "MaxPlayer=7\n",
            "maxplayer=88\n",
            "MinPlayer=0\n",
            "NetworkGame= 1\n",
            "Loader=Loader* // literal  \n",
            "Description=Searchable // literal  \n",
            "[Head]\n",
            "SaveGame=1\n",
            "MaxPlayer=10\n",
            "[Game]\n",
            "Goals=MELE=0\n",
            "Goals=MELE=1\n",
            "[Game]\n",
            "Goals=MELE=1\n",
        ));

        assert_eq!(manifest.core.head.max_player, 7);
        assert_eq!(manifest.core.head.min_player, 0);
        assert_eq!(manifest.core.head.save_game, 0);
        assert!(!manifest.core.head.network_game);
        assert_eq!(manifest.core.head.loader, "Loader* // literal  ");
        assert_eq!(
            manifest.description.as_deref(),
            Some("Searchable // literal  ")
        );
        assert_eq!(manifest.core.game.goals.len(), 1);
        assert_eq!(manifest.core.game.goals[0].id, "MELE");
        assert_eq!(manifest.core.game.goals[0].count, Some(0));

        let malformed = parsed_scenario("[Head]\nMaxPlayer=broken\n");
        assert_eq!(malformed.core.head.max_player, 12);
        assert_eq!(malformed.core.head.max_player_league, 12);

        let integer_flag = parsed_scenario("[Head]\nSaveGame=true\n");
        assert_eq!(integer_flag.core.head.save_game, 0);
    }

    #[test]
    fn legacy_scenario_line_endings_match_cpp() {
        let lf = concat!(
            "[Head]\n",
            "Title=Line Endings\n",
            "MaxPlayer=4\n",
            "\n",
            "[Game]\n",
            "Goals=MELE=1\n",
            "\n",
            "[Landscape]\n",
            "MapWidth=80\n",
            "MapHeight=20\n",
            "MapZoom=3\n",
        );
        let crlf = lf.replace('\n', "\r\n");
        let cr = lf.replace('\n', "\r");
        let expected = parsed_scenario(&crlf);

        for (label, source) in [("LF", lf), ("CR", cr.as_str())] {
            let parsed = parse_legacy_scenario_text(source)
                .unwrap_or_else(|error| panic!("{label} Scenario.txt parses: {error}"));
            assert_eq!(parsed.sections, expected.sections, "{label} section tree");
            assert_eq!(parsed.title.as_deref(), Some("Line Endings"));
            assert_eq!(parsed.core.head.max_player, 4);
            assert_eq!(parsed.core.landscape.map_width.std, 80);
            assert_eq!(parsed.ground_height_hint, Some(60));
        }
    }

    #[test]
    fn parameters_recover_containers_validate_clients_and_sort_by_id() {
        let core = parsed_scenario("[Head]\nForcedNoCrew=2\n");
        let defaults = game_parameter_defaults(&core.core);
        let parameters = parse_legacy_game_parameter_overrides(
            concat!(
                "[Parameters]\n",
                "UseFairCrew= 1\n",
                "Rules=RULE=2;=3;NEXT=4\n",
                "Goals=RULE=2;badx=3;NEXT=4\n",
                "  [Client]\n",
                "  ID=9\n",
                "  Name=\n",
                "  Nick=\n",
                "  [Client]\n",
                "  ID=2\n",
            ),
            &defaults,
        );

        assert_eq!(parameters.use_fair_crew(), Some(false));
        assert_eq!(parameters.rules().expect("rules").len(), 1);
        assert_eq!(parameters.rules().expect("rules")[0].id(), "RULE");
        assert_eq!(parameters.rules().expect("rules")[0].count(), 2);
        assert_eq!(parameters.goals().expect("goals").len(), 1);
        assert_eq!(parameters.goals().expect("goals")[0].id(), "RULE");
        assert_eq!(parameters.goals().expect("goals")[0].count(), 2);
        assert_eq!(parameters.clients().len(), 2);
        assert_eq!(parameters.clients()[0].id(), 2);
        assert_eq!(parameters.clients()[0].name(), "");
        assert_eq!(parameters.clients()[0].nick(), "");
        assert_eq!(parameters.clients()[1].id(), 9);
        assert_eq!(parameters.clients()[1].name(), "Unknown");
        assert_eq!(parameters.clients()[1].nick(), "Unknown");
    }

    #[test]
    fn replay_startup_preflight_merges_scenario_and_parameter_map_inputs() {
        let directory = scenario_test_root("[Head]\nReplay=1\nRandomSeed=41\n");
        write_test_file(
            directory.path().join("Parameters.txt"),
            "[Parameters]\nRandomSeed=73\nStartupPlayerCount=4\n",
        );
        write_test_file(
            directory.path().join("PlayerInfos.txt"),
            concat!(
                "[PlayerInfoList]\n",
                "  [Client]\n",
                "    [Player]\n",
                "    Name=First\n",
                "    [Player]\n",
                "    Name=Removed\n",
                "    Flags=Joined|Removed\n",
                "    [Player]\n",
                "    Name=Numeric Removed\n",
                "    Flags=4\n",
                "    [Player]\n",
                "    Name=Second\n",
            ),
        );
        let group = Group::open(directory.path()).test_value();

        assert_eq!(
            Scenario::preflight_replay_startup_from_group(&group).unwrap(),
            Some(ReplayScenarioStartupPreflight {
                random_seed: 73,
                startup_player_count: 2,
            })
        );

        write_test_file(directory.path().join("Game.txt"), "[Game]\nFrame=37\n");
        let group = Group::open(directory.path()).test_value();
        assert_eq!(
            Scenario::preflight_replay_startup_from_group(&group).unwrap(),
            Some(ReplayScenarioStartupPreflight {
                random_seed: 73,
                startup_player_count: 4,
            }),
            "nonzero-frame records retain the serialized parameter"
        );

        std::fs::remove_file(directory.path().join("Game.txt")).test_value();
        std::fs::remove_file(directory.path().join("PlayerInfos.txt")).test_value();
        let group = Group::open(directory.path()).test_value();
        assert_eq!(
            Scenario::preflight_replay_startup_from_group(&group)
                .unwrap()
                .expect("replay preflight")
                .startup_player_count,
            0,
            "a frame-zero replay with no PlayerInfos has an empty startup list"
        );
    }

    #[test]
    fn legacy_parameters_preflight_accepts_cpp_line_endings_and_rejects_bom_header() {
        for source in [
            "[Parameters]\nMaxPlayers=5\n",
            "[Parameters]\r\nMaxPlayers=5\r\n",
            "[Parameters]\rMaxPlayers=5\r",
        ] {
            assert_eq!(
                parse_legacy_parameters_max_players(source.as_bytes(), 12)
                    .expect("Parameters.txt preflight parses"),
                5
            );
        }

        assert_eq!(
            parse_legacy_parameters_max_players(
                "\u{feff}[Parameters]\rMaxPlayers=5\r".as_bytes(),
                12,
            )
            .expect("BOM-prefixed Parameters.txt keeps the scenario default"),
            12
        );
    }

    #[test]
    fn scenario_defaulted_components_keep_prefix_and_cursor_state() {
        let manifest = parsed_scenario(concat!(
            "[Head]\n",
            "Version=4,bad,6\n",
            "[Player1]\n",
            "Position=22,bad\n",
            "[Landscape]\n",
            "SkyFade=1,bad,3\n",
            "MapWidth=101,bad,70,300\n",
        ));

        assert_eq!(manifest.core.head.version, [4, 0, 0, 0, 0]);
        assert_eq!(manifest.core.players[0].position, [22, -1]);
        assert_eq!(manifest.core.landscape.sky_fade, [1, 0, 0, 0, 0, 0]);
        assert_eq!(manifest.core.landscape.map_width, c4s(101, 0, 64, 250));
    }

    #[test]
    fn scenario_name_lists_match_c4namelist_bounds_and_identifier_tokens() {
        let layers = (1..=11)
            .map(|index| format!("Layer{index}=50"))
            .collect::<Vec<_>>()
            .join(";");
        let manifest = parsed_scenario(&format!(
            "[Game]\nClearMaterials={layers}\n[Landscape]\nLayers={layers}\n"
        ));
        assert_eq!(manifest.core.game.clear_materials.len(), 10);
        assert_eq!(manifest.core.landscape.layers.len(), 10);
        assert_eq!(manifest.core.landscape.layers[0].name, "Layer1");
        assert_eq!(manifest.core.landscape.layers[9].name, "Layer10");

        let malformed = parsed_scenario(concat!(
            "[Game]\n",
            "ClearMaterials=ABCDEFGHIJKLMNOPQRSTUVWXYZ12345=2;Gold=3\n",
            "[Landscape]\n",
            "Layers=My Rock=2;Earth=3\n",
        ));
        assert_eq!(
            malformed
                .core
                .game
                .clear_materials
                .iter()
                .map(|entry| (entry.name.as_str(), entry.count.unwrap_or(0)))
                .collect::<Vec<_>>(),
            [("ABCDEFGHIJKLMNOPQRSTUVWXYZ1234", 0)]
        );
        assert_eq!(
            malformed
                .core
                .landscape
                .layers
                .iter()
                .map(|entry| (entry.name.as_str(), entry.count.unwrap_or(0)))
                .collect::<Vec<_>>(),
            [("My", 0)]
        );

        let reentered = parsed_scenario(concat!(
            "[Game]\n",
            "ClearMaterials=A=1=2;B=3\n",
            "[Landscape]\n",
            "Layers=\u{a0}Gold=1\n",
        ));
        assert_eq!(
            reentered
                .core
                .game
                .clear_materials
                .iter()
                .map(|entry| (entry.name.as_str(), entry.count.unwrap_or(0)))
                .collect::<Vec<_>>(),
            [("A", 1), ("B", 3)]
        );
        assert!(reentered.core.landscape.layers.is_empty());
    }

    /// The override replaces the preset before anything is resolved
    /// (src/C4Game.cpp:180-227), so a save whose Scenario.txt names a module
    /// that no longer exists fails — or not — on the *override's* module.
    #[test]
    fn savegame_definition_override_replaces_missing_scenario_modules() {
        let dir = test_tempdir();
        let scenario_dir = dir.path().join("OldSave.c4s");
        std::fs::create_dir(&scenario_dir).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Old Save\nSaveGame=1\n\n[Definitions]\nDefinition1=Missing.c4d\n",
        );
        write_test_file(
            scenario_dir.join("Game.txt"),
            "[DefinitionFiles]\nDefinition1=Historical.c4d\n\n[Game]\n",
        );
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let error = Scenario::load_from_group_with_languages(
            &Group::open(&scenario_dir).test_value(),
            &resolver,
            &["US"],
        )
        .expect_err("the override's own module is missing too");
        assert!(
            matches!(
                &error,
                ScenarioError::LegacyDefinitionNotFound { path }
                    if path == "efinition1=Historical.c4d"
            ),
            "the preset is never consulted once the override replaced it: {error:?}"
        );
    }

    #[test]
    fn shipped_canyon_lobby_metadata_matches_legacy_files() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let scenario_path = repository.join("content/Melees.c4f/Canyon.c4s/Scenario.txt");
        let teams_path = repository.join("content/Melees.c4f/Canyon.c4s/Teams.txt");
        assert!(
            scenario_path.is_file() && teams_path.is_file(),
            "the initialized official content submodule must provide Canyon Scenario.txt and Teams.txt"
        );
        let scenario_source =
            decode_legacy_script_text(&std::fs::read(&scenario_path).test_value());
        let team_source = decode_legacy_script_text(&std::fs::read(&teams_path).test_value());
        let manifest = parsed_scenario(&scenario_source);
        let teams = parse_legacy_teams_source(&team_source);

        assert_eq!(manifest.core.head.max_player, 6);
        assert_eq!(manifest.core.head.max_player_league, 6);
        assert_eq!(legacy_effective_min_players(&manifest.core), 2);
        assert_eq!(manifest.definition_specs, ["Objects.c4d"]);
        assert_eq!(teams.source(), ScenarioTeamsSource::TeamsFile);
        assert_eq!(teams.distribution(), ScenarioTeamDistribution::Free);
        assert!(!teams.auto_generates_teams());
        assert!(teams.uses_team_colors());
        assert_eq!(teams.teams().len(), 2);
        assert_eq!(teams.teams()[0].id(), 1);
        assert_eq!(teams.teams()[0].icon_spec(), Some("TMS1:2"));
        assert_eq!(teams.teams()[1].id(), 2);
        assert_eq!(teams.teams()[1].icon_spec(), Some("TMS1:3"));
    }

    #[test]
    fn json_scenarios_do_not_synthesize_legacy_lobby_metadata() {
        let dir = test_tempdir();
        write_test_file(
            dir.path().join("Scenario.json"),
            r#"{"name":"Fixture","definitions":[{"id":"TEST","script":"Def.c"}]}"#,
        );
        write_test_file(dir.path().join("Def.c"), "// fixture\n");
        let scenario = Scenario::load_from_path(dir.path()).test_value();
        assert!(scenario.lobby_metadata().is_none());
    }

    #[test]
    fn json_manifest_script_paths_preserve_native_bytes() {
        let dir = test_tempdir();
        write_test_file(
            dir.path().join("Scenario.json"),
            r#"{"definitions":[{"id":"TEST","script":"Def.c"}],"script":"Scenario.c"}"#,
        );
        write_test_file(
            dir.path().join("Def.c"),
            [
                b"#strict\nfunc Raw() { return \"".as_slice(),
                &[0xe9, 0xff],
                b"\"; }\n",
            ]
            .concat(),
        );
        write_test_file(
            dir.path().join("Scenario.c"),
            [
                b"#strict\nglobal func Initialize(state, random) { Message(\"".as_slice(),
                &[0xe9, 0xff],
                b"\"); }\n",
            ]
            .concat(),
        );

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        assert!(
            clonk_script::c4_string_bytes(&scenario.definitions[0].script)
                .windows(2)
                .any(|bytes| bytes == [0xe9, 0xff])
        );
        assert!(scenario.script.as_ref().is_some_and(|script| {
            clonk_script::c4_string_bytes(&script.source)
                .windows(2)
                .any(|bytes| bytes == [0xe9, 0xff])
        }));

        let mut engine = Engine::new();
        apply_test_scenario(&scenario, &mut engine);
        assert_eq!(
            clonk_script::c4_string_bytes(&engine.snapshot().hud.messages[0].lines[0]),
            [0xe9, 0xff]
        );
        let object = engine.spawn_test_object(SpawnConfig::new("TEST"));
        let index = engine.test_object_index(object);
        assert_eq!(
            engine
                .call_object_function(index, "Raw", Vec::new())
                .expect("raw definition function runs"),
            script_string(clonk_script::c4_string_from_bytes(&[0xe9, 0xff]).into())
        );
    }

    #[test]
    fn json_scenario_with_missing_definition_include_still_applies() {
        let dir = test_tempdir();
        write_test_file(
            dir.path().join("Scenario.json"),
            r#"{"name":"Missing include","definitions":[{"id":"TEST","script":"Def.c"}]}"#,
        );
        write_test_file(
            dir.path().join("Def.c"),
            "#include MISS\npublic func OwnValue() { return 42; }\n",
        );

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let mut engine = Engine::new();
        apply_test_scenario(&scenario, &mut engine);
        assert!(engine.definition_script_has_function("TEST", "OwnValue"));
    }

    #[test]
    fn all_shipped_team_icon_specs_are_retained_recursively() {
        fn collect_team_files(directory: &Path, output: &mut Vec<PathBuf>) {
            let entries = std::fs::read_dir(directory).unwrap_or_else(|error| {
                panic!(
                    "read shipped content directory {}: {error}",
                    directory.display()
                )
            });
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_team_files(&path, output);
                } else if path
                    .file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Teams.txt"))
                {
                    output.push(path);
                }
            }
        }

        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(
            repository.join("content").is_dir(),
            "the official content submodule must be initialized"
        );
        let mut team_files = Vec::new();
        collect_team_files(&repository.join("content"), &mut team_files);
        collect_team_files(&repository.join("planet"), &mut team_files);
        team_files.sort();

        let mut files_with_icons = 0;
        let mut icon_count = 0;
        let mut unparsed: Vec<String> = Vec::new();
        for path in team_files {
            let bytes = std::fs::read(&path).test_value();
            let source = decode_legacy_script_text(&bytes);
            let teams = parse_legacy_teams_source(&source);
            let icons = teams
                .teams()
                .iter()
                .filter_map(ScenarioLobbyTeam::icon_spec)
                .collect::<Vec<_>>();
            if !icons.is_empty() {
                files_with_icons += 1;
            }
            for icon in icons {
                if crate::text_spec::parse_text_spec(icon).is_none() {
                    let relative = path
                        .strip_prefix(&repository)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    unparsed.push(format!("{relative} :: {icon}"));
                }
                icon_count += 1;
            }
        }
        // Sorted so the assertion pins the set, not the order teams happen to
        // appear in within a file.
        unparsed.sort();
        // Two specs in one third-party scenario use a single colon where the
        // portrait grammar wants two. C++ refuses them as well:
        // `C4Portrait::EvaluatePortraitString` (C4DefGraphics.cpp:578-603) falls
        // into its bare-name `else` branch and hands back the default ID, and
        // `C4Game::DrawTextSpecImage` was passed `C4ID_None` as that default, so
        // it bails at `if (idPortrait == C4ID_None) return false`
        // (C4Game.cpp:4314-4315). The icon draws in neither engine. Pinned by
        // path rather than tolerated by count, so a *new* unparseable spec —
        // which would mean the parser regressed — still fails here.
        assert_eq!(
            unparsed,
            vec![
                "content/Collection.c4f/Settling.c4f/RufDerWipfeRE.c4f/Others.c4f/ModernBattle.c4s/Teams.txt :: Portrait:CLN2:Soldier1",
                "content/Collection.c4f/Settling.c4f/RufDerWipfeRE.c4f/Others.c4f/ModernBattle.c4s/Teams.txt :: Portrait:CLN2:Soldier2",
            ],
            "the set of IconSpecs neither engine can draw changed"
        );
        assert_eq!(files_with_icons, 52, "recursive team-file census changed");
        assert_eq!(icon_count, 105, "recursive IconSpec census changed");
    }

    #[test]
    fn legacy_c4sval_evaluate_draws_via_the_game_rng_like_cpp() {
        // C4SVal::Evaluate (C4Scenario.cpp:43-46):
        //   return BoundBy(Std + Random(2 * Rnd + 1) - Rnd, Min, Max);
        // One Random draw per Evaluate — even Rnd == 0 calls Random(1),
        // which advances the LCG stream (C4Random.h:40-61).
        let mut reference = crate::rng::LcgRng::new(42);
        let expected = (10 + reference.random(2 * 3 + 1) - 3).clamp(0, 250);

        let mut rng = crate::rng::LcgRng::new(42);
        assert_eq!(c4s(10, 3, 0, 250).evaluate(&mut rng), expected);
        assert_eq!(rng, reference);

        // Rnd == 0 still draws: Random(1) returns 0 but advances hold/count.
        let before = rng.clone();
        assert_eq!(c4s(5, 0, 0, 250).evaluate(&mut rng), 5);
        assert_ne!(rng.hold, before.hold);
        assert_eq!(rng.count, before.count + 1);
    }

    #[test]
    fn legacy_fow_color_minus_one_reinterprets_as_u32_max() {
        let manifest = parsed_scenario("[Game]\nFoWColor=-1\n");

        assert_eq!(manifest.core.game.fow_color, u32::MAX);
    }

    #[test]
    fn legacy_fow_color_uses_stdcompiler_signed_prefix_rules() {
        for (raw, expected) in [
            ("-2147483648", 0x8000_0000),
            ("-0x80000000", 0),
            ("+0x80000000", 0),
            ("4294967295", u32::MAX),
        ] {
            let manifest = parsed_scenario(&format!("[Game]\nFoWColor={raw}\n"));

            assert_eq!(manifest.core.game.fow_color, expected, "FoWColor={raw}");
        }
    }

    #[test]
    fn legacy_fow_color_keeps_in_range_decimal_and_hex_values() {
        for (raw, expected) in [("305419896", 0x1234_5678), ("0x89abcdef", 0x89ab_cdef)] {
            let manifest = parsed_scenario(&format!("[Game]\nFoWColor={raw}\n"));

            assert_eq!(manifest.core.game.fow_color, expected, "FoWColor={raw}");
        }
    }

    #[test]
    fn scenario_value_store_reinterprets_fow_color_as_signed_i32() {
        let manifest = parsed_scenario("[Game]\nFoWColor=-1\n");
        let values = ScenarioValueStore::from_runtime_core(&manifest.core, false);

        assert_eq!(
            values.get("FoWColor", Some("Game"), 0),
            Some(&scenario_int(-1))
        );
        assert_eq!(values.fow_color(), u32::MAX);
        assert_eq!(values.fow_resolution(), 64);
    }

    #[test]
    fn scenario_value_store_projects_fow_resolution_for_the_renderer() {
        let manifest = parsed_scenario("[Landscape]\nFoWRes=96\n");
        let values = ScenarioValueStore::from_runtime_core(&manifest.core, false);

        assert_eq!(values.fow_resolution(), 96);
    }

    #[test]
    fn legacy_scenario_core_parses_all_fields() {
        let legacy = r#"
[Head]
Title=Legacy Land
Icon=7
Loader=LoaderGfx
Font=CustomFont
Version=4,9,10,15,359
Difficulty=3
MaxPlayer=6
MaxPlayerLeague=4
MinPlayer=2
SaveGame=1
Replay=0
Film=2
DisableMouse=1
NoInitialize=1
RandomSeed=12345
ForcedAutoContextMenu=0
ForcedAutoStopControl=1
Engine=Legacy
MissionAccess=MISS
NetworkGame=1
NetworkRuntimeJoin=0
ForcedGfxMode=2
ForcedNoCrew=1
DefCrewStrength=42
Origin=Planet\Legacy.c4s

[Definitions]
LocalOnly=0
AllowUserChange=1
Definitions="Defs.c4d","More.c4d"
Definition3=Extra.c4d
SkipDefs=CLNK=2;ROCK

[Game]
Mode=2
Elimination=3
CooperativeGoal=1
CreateObjects=FIRE=3;WOOD=1
ClearObjects=ROCK=2
ClearMaterials=Earth=5;Gold
ValueGain=150
EnableRemoveFlag=1
StructNeedMaterial=1
StructNeedEnergy=0
ValueOverloads=VALU=2
LandscapePushPull=2
LandscapeInsertThrust=3
BaseFunctionality=BASEFUNC_Buy|BASEFUNC_Sell
BaseRegenerateEnergyPrice=12
Goals=GOAL=1
Rules=RULE=1
FoWColor=0x12345678

[Player1]
StandardCrew=CLNK
Clonks=2,1,1,5
Wealth=50,0,0,500
Position=100,200
EnforcePosition=1
Crew=CLNK=2;OCEN
Buildings=HUTS=1
Vehicles=CARR=1
Material=ROCK=3
Knowledge=KNOW
HomeBaseMaterial=WOOD=5
HomeBaseProduction=METL=2
Magic=MAGI=1

[Landscape]
ExactLandscape=1
Vegetation=GRAS;TREE
VegetationLevel=60,20,0,100
InEarth=ROCK;COAL
InEarthLevel=40,0,0,100
Sky=Sky.ocg
SkyFade=1,2,3,4,5,6
NoSky=0
BottomOpen=1
TopOpen=0
LeftOpen=1
RightOpen=2
AutoScanSideOpen=0
MapWidth=120,0,64,250
MapHeight=80,0,40,250
MapZoom=5,0,5,15
Amplitude=10,0,0,100
Phase=25,0,0,100
Period=30,0,0,100
Random=15,0,0,100
Material=Sand
Liquid=Lava
LiquidLevel=5,0,0,100
MapPlayerExtend=1
Layers=Earth=2;Sky=1
Gravity=90,0,10,200
NoScan=1
KeepMapCreator=1
SkyScrollMode=2
NewStyleLandscape=1
FoWRes=128
ShadeMaterials=0

[Weather]
Climate=40,10,0,100
StartSeason=10,20,0,100
YearSpeed=5,0,0,100
Rain=30,0,0,100
Wind=5,10,-50,50
Lightning=20,0,0,100
Precipitation=Oil
NoGamma=0

[Disasters]
Meteorite=10,0,0,100
Volcano=5,0,0,100
Earthquake=3,0,0,100

[Animals]
Animal=WLF_=2
Nest=ANT_=3

[Environment]
Objects=STNE=1;TREE=1
"#;

        let manifest = parsed_scenario(legacy);
        let core = &manifest.core;

        assert_eq!(core.head.title, "Legacy Land");
        assert_eq!(core.head.icon, 7);
        assert_eq!(core.head.loader, "LoaderGfx");
        assert_eq!(core.head.font, "CustomFont");
        assert_eq!(core.head.version, [4, 9, 10, 15, 359]);
        assert_eq!(core.head.difficulty, 3);
        assert_eq!(core.head.max_player, 6);
        assert_eq!(core.head.max_player_league, 4);
        assert_eq!(core.head.min_player, 2);
        assert_eq!(core.head.save_game, 1);
        assert_eq!(core.head.disable_mouse, 1);
        assert_eq!(core.head.no_initialize, 1);
        assert_eq!(core.head.random_seed, 12345);
        assert_eq!(core.head.forced_auto_context_menu, 0);
        assert_eq!(core.head.forced_control_style, 1);
        assert_eq!(core.head.engine, "Legacy");
        assert_eq!(core.head.mission_access, "MISS");
        assert!(core.head.network_game);
        assert!(!core.head.network_runtime_join);
        assert_eq!(core.head.forced_gfx_mode, 2);
        assert_eq!(core.head.forced_fair_crew, 1);
        assert_eq!(core.head.fair_crew_strength, 42);
        let expected_origin = if cfg!(windows) {
            "Planet\\Legacy.c4s"
        } else {
            "Planet/Legacy.c4s"
        };
        assert_eq!(core.head.origin.as_deref(), Some(expected_origin));

        assert!(!core.definitions.local_only);
        assert!(core.definitions.allow_user_change);
        assert_eq!(
            core.definitions.definitions,
            vec!["Defs.c4d".to_string(), "More.c4d".to_string()]
        );
        assert_eq!(core.definitions.skip_defs.len(), 2);
        assert_eq!(core.definitions.skip_defs[0].id, "CLNK");
        assert_eq!(core.definitions.skip_defs[0].count, Some(2));
        assert_eq!(core.definitions.skip_defs[1].id, "ROCK");
        assert_eq!(core.definitions.skip_defs[1].count, None);

        assert_eq!(core.game.mode, 2);
        assert_eq!(core.game.elimination, 3);
        assert_eq!(core.game.cooperative_goal, 1);
        assert_eq!(core.game.create_objects.len(), 2);
        assert_eq!(core.game.clear_objects.len(), 1);
        assert_eq!(core.game.clear_materials.len(), 2);
        assert_eq!(core.game.value_gain, 150);
        assert!(core.game.enable_remove_flag);
        assert!(core.game.realism.construction_needs_material);
        assert!(!core.game.realism.structures_need_energy);
        assert_eq!(core.game.realism.landscape_push_pull, 2);
        assert_eq!(core.game.realism.landscape_insert_thrust, 3);
        assert_eq!(
            core.game.realism.base_functionality,
            BASEFUNC_BUY | BASEFUNC_SELL
        );
        assert_eq!(core.game.realism.base_regenerate_energy_price, 12);
        assert_eq!(core.game.goals.len(), 1);
        assert_eq!(core.game.rules.len(), 1);
        assert_eq!(core.game.fow_color, 0x1234_5678);

        assert_eq!(core.players.len(), 1);
        let player = &core.players[0];
        assert_eq!(player.standard_crew.as_deref(), Some("CLNK"));
        assert_eq!(player.clonks.std, 2);
        assert_eq!(player.clonks.rnd, 1);
        assert_eq!(player.wealth.std, 50);
        assert_eq!(player.position, [100, 200]);
        assert_eq!(player.enforce_position, 1);
        assert_eq!(player.crew.len(), 2);
        assert_eq!(player.buildings.len(), 1);
        assert_eq!(player.vehicles.len(), 1);
        assert_eq!(player.material.len(), 1);
        assert_eq!(player.knowledge.len(), 1);
        assert_eq!(player.home_base_material.len(), 1);
        assert_eq!(player.home_base_production.len(), 1);
        assert_eq!(player.magic.len(), 1);

        let landscape = &core.landscape;
        assert!(landscape.exact_landscape);
        assert_eq!(landscape.vegetation.len(), 2);
        assert_eq!(landscape.in_earth.len(), 2);
        assert_eq!(landscape.sky.as_deref(), Some("Sky.ocg"));
        assert_eq!(landscape.sky_fade, [1, 2, 3, 4, 5, 6]);
        assert!(landscape.bottom_open);
        assert!(!landscape.top_open);
        assert_eq!(landscape.left_open, 1);
        assert_eq!(landscape.right_open, 2);
        assert!(!landscape.auto_scan_side_open);
        assert_eq!(landscape.map_width.std, 120);
        assert_eq!(landscape.map_height.std, 80);
        assert_eq!(landscape.map_zoom.std, 5);
        assert_eq!(landscape.material, "Sand");
        assert_eq!(landscape.liquid, "Lava");
        assert!(landscape.map_player_extend);
        assert_eq!(landscape.layers.len(), 2);
        assert!(landscape.no_scan);
        assert!(landscape.keep_map_creator);
        assert_eq!(landscape.sky_scroll_mode, 2);
        assert_eq!(landscape.new_style_landscape, 1);
        assert_eq!(landscape.fow_resolution, 128);
        assert!(!landscape.shade_materials);

        let weather = &core.weather;
        assert_eq!(weather.climate.std, 40);
        assert_eq!(weather.start_season.std, 10);
        assert_eq!(weather.year_speed.std, 5);
        assert_eq!(weather.rain.std, 30);
        assert_eq!(weather.wind.std, 5);
        assert_eq!(weather.lightning.std, 20);
        assert_eq!(weather.precipitation, "Oil");
        assert!(!weather.no_gamma);

        assert_eq!(core.disasters.meteorite.std, 10);
        assert_eq!(core.disasters.volcano.std, 5);
        assert_eq!(core.disasters.earthquake.std, 3);

        assert_eq!(core.animals.free_life.len(), 1);
        assert_eq!(core.animals.earth_nest.len(), 1);
        assert_eq!(core.environment.objects.len(), 2);
    }

    /// `C4Game::DefinitionFilenamesFromSaveGame` clears the whole vector when
    /// Game.txt carries `[DefinitionFiles]` and appends `line.substr(1)` for
    /// every accepted line (src/C4Game.cpp:3625-3653). `substr(1)` is not a
    /// typo: the precedence bug at 3643 assigns the *result of the comparison*
    /// to `p`, so `p` is 1 and the pushed name is the line minus its first
    /// character, not the text after `=`.
    ///
    /// The caller runs it after the preset, the DefinitionPath expansion and
    /// the folder-local scan (src/C4Game.cpp:180-227), so it replaces all
    /// three.
    #[test]
    fn an_old_save_definition_files_section_replaces_the_definition_vector() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        // Named for what the precedence bug actually asks the loader to open.
        let overridden = dir.path().join("efinition1=Old.c4d");
        let definition = overridden.join("Old.c4d");
        std::fs::create_dir_all(&definition).test_value();
        write_test_file(
            definition.join("DefCore.txt"),
            "[DefCore]\nid=OLDD\nName=Old\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(definition.join("Script.c"), "// old\n");
        write_test_definition_graphics(&definition);
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Old Save\nSaveGame=1\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        );
        write_test_file(
            scenario_dir.join("Game.txt"),
            "[DefinitionFiles]\nDefinition1=Old.c4d\n\n[Game]\n",
        );
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let scenario = load_test_scenario(&scenario_dir, &resolver);
        let lobby = scenario.lobby_metadata().test_value();
        let definitions = lobby.definitions();

        assert_eq!(
            definitions.effective_modules(),
            ["efinition1=Old.c4d"]
        );
        let resolved = definitions.resolved_load_resources();
        assert!(
            resolved.iter().any(|path| path.ends_with("efinition1=Old.c4d")),
            "the replacement drives loading: {resolved:?}"
        );
        assert!(
            !resolved.iter().any(|path| path.ends_with("Defs.c4d")),
            "the preset the override cleared must not survive: {resolved:?}"
        );
    }

    /// The loop that `[DefinitionFiles]` drives (src/C4Game.cpp:3637-3651):
    /// a line is accepted only when `find("Definition") == 0` *and* the line
    /// holds an `=`; the first line that fails after one was accepted breaks
    /// out; and the section header itself never matches, so lines before the
    /// first accepted one are skipped rather than ending the scan.
    #[test]
    fn old_save_definition_lines_follow_the_cpp_accept_and_break_rules() {
        for (label, game_text, expected) in [
            (
                "order is kept and every accepted line loses its first character",
                "[DefinitionFiles]\nDefinition1=A.c4d\nDefinition2=B.c4d\n",
                vec!["efinition1=A.c4d", "efinition2=B.c4d"],
            ),
            (
                "a duplicate line is kept twice, like push_back",
                "[DefinitionFiles]\nDefinition1=A.c4d\nDefinition1=A.c4d\n",
                vec!["efinition1=A.c4d", "efinition1=A.c4d"],
            ),
            (
                "a line without `=` is not accepted",
                "[DefinitionFiles]\nDefinition1\nDefinition2=B.c4d\n",
                vec!["efinition2=B.c4d"],
            ),
            (
                "a line that only contains `Definition` later does not match",
                "[DefinitionFiles]\n Definition1=A.c4d\nDefinition2=B.c4d\n",
                vec!["efinition2=B.c4d"],
            ),
            (
                "the first non-matching line after an accepted one ends the scan",
                "[DefinitionFiles]\nDefinition1=A.c4d\n\n[Game]\nDefinition2=B.c4d\n",
                vec!["efinition1=A.c4d"],
            ),
            (
                "an empty section clears the vector and adds nothing",
                "[DefinitionFiles]\n\n[Game]\nFrame=3\n",
                Vec::new(),
            ),
            (
                "the match is case-sensitive, like std::string::find",
                "[DefinitionFiles]\ndefinition1=A.c4d\nDefinition2=B.c4d\n",
                vec!["efinition2=B.c4d"],
            ),
        ] {
            let dir = test_tempdir();
            write_test_file(
                dir.path().join("Scenario.txt"),
                "[Head]\nTitle=Old Save\nSaveGame=1\n",
            );
            write_test_file(dir.path().join("Game.txt"), game_text);
            let group = Group::open(dir.path()).test_value();

            let override_lines = load_savegame_definition_override(&group, true)
                .expect("the section parses")
                .effective_modules()
                .expect("Game.txt carried a section");

            assert_eq!(override_lines, expected, "{label}");
        }
    }

    /// Without the section the ordinary selection stands — `None`, not an
    /// empty replacement (src/C4Game.cpp:3653).
    #[test]
    fn a_save_without_the_section_keeps_the_ordinary_definition_selection() {
        let dir = test_tempdir();
        write_test_file(
            dir.path().join("Scenario.txt"),
            "[Head]\nTitle=Old Save\nSaveGame=1\n",
        );
        write_test_file(dir.path().join("Game.txt"), "[Game]\nFrame=3\n");
        let group = Group::open(dir.path()).test_value();

        assert!(load_savegame_definition_override(&group, true)
            .expect("Game.txt parses")
            .effective_modules()
            .is_none());
    }

    /// `if (C4S.Head.SaveGame)` gates the whole call (src/C4Game.cpp:227), so
    /// an ordinary scenario carrying the same text is untouched.
    #[test]
    fn a_scenario_that_is_not_a_savegame_never_reads_the_section() {
        let dir = test_tempdir();
        write_test_file(dir.path().join("Scenario.txt"), "[Head]\nTitle=Live\n");
        write_test_file(
            dir.path().join("Game.txt"),
            "[DefinitionFiles]\nDefinition1=A.c4d\n",
        );
        let group = Group::open(dir.path()).test_value();

        assert!(load_savegame_definition_override(&group, false)
            .expect("Game.txt parses")
            .effective_modules()
            .is_none());
    }
