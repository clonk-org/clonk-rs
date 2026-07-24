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
        let core = parse_legacy_scenario_text("[Definitions]\nDefinitions=Old.c4d\n")
            .expect("legacy core parses")
            .core;
        let modules = vec!["Effective.c4d".to_owned()];

        for saved in [
            core.initial_network_save("Title", &modules, "", "", ""),
            core.initial_record_save("Title", &modules, "", "", ""),
        ] {
            let serialized = String::from_utf8(saved.serialize()).expect("Scenario.txt is UTF-8");
            assert!(serialized.contains("Definitions=\"Effective.c4d\"\r\n"));
            assert!(!serialized.contains("Old.c4d"));
        }
    }

    #[test]
    fn runtime_scenario_and_savegame_core_adjustments_match_cpp() {
        let core = parse_legacy_scenario_text(
            "[Head]\nIcon=7\nTitle=Authored\nVersion=1,2,3,4,359\nSaveGame=1\nNoInitialize=0\nMissionAccess=MISS\nNetworkGame=true\nNetworkRuntimeJoin=true\nOrigin=Retained\\Game.c4s\n\n[Definitions]\nDefinitions=Old.c4d\n",
        )
        .expect("legacy core parses")
        .core;

        let scenario = String::from_utf8(core.runtime_scenario_save().serialize())
            .expect("Scenario.txt is UTF-8");
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
        .expect("Scenario.txt is UTF-8");
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
            .expect("legacy initial record scenario serializes");

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
            .expect("legacy initial record scenario serializes");
        let actual = String::from_utf8(actual).expect("Scenario.txt is UTF-8");

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

        let metadata = scenario
            .initial_network_scenario_metadata()
            .expect("legacy scenario exposes initial network metadata");

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

        let metadata = scenario
            .initial_network_scenario_metadata()
            .expect("legacy scenario exposes initial network metadata");

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
            .expect("Teams.txt parses"),
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
            Some(parse_legacy_team_metadata_source("[Teams]\n").expect("empty Teams.txt parses"));
        let empty_file = empty_file
            .initial_network_team_metadata()
            .expect("empty team file has C++ defaults");
        assert!(empty_file.active);
        assert!(empty_file.custom);
        assert!(!empty_file.allow_hostility_change);
        assert!(empty_file.auto_generate_teams);
        assert!(empty_file.teams.is_empty());

        let cooperative = scenario_with_retained_legacy_core("[Game]\nStructNeedEnergy=0\n")
            .initial_network_team_metadata()
            .expect("missing Teams.txt derives cooperative defaults");
        assert!(!cooperative.active);
        assert!(!cooperative.custom);
        assert!(cooperative.allow_hostility_change);
        assert!(!cooperative.auto_generate_teams);

        let melee = scenario_with_retained_legacy_core("[Game]\nMode=1\nStructNeedEnergy=0\n")
            .initial_network_team_metadata()
            .expect("missing Teams.txt derives converted melee defaults");
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
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Teams.txt"),
            b"[Teams]\nScriptPlayerNames=\"$Roster$\"\n\n  [Team]\n  id=1\n  Name=$LocalizedTeam$\n  IconSpec=\"\\334\"\n",
        )
        .expect("write Teams.txt");
        let mut table = b"LocalizedTeam=".to_vec();
        table.extend(std::iter::repeat_n(0xdc, 35));
        table.extend_from_slice(b"\nRoster=");
        table.extend_from_slice(&[0xc4, b'|', 0xd6, b'\n']);
        std::fs::write(dir.path().join("StringTbl.txt"), []).expect("write empty string table");
        std::fs::write(dir.path().join("StringTblUS.txt"), table)
            .expect("write localized string table");

        let group = Group::open(dir.path()).expect("open group");
        let (teams, loaded) =
            load_initial_network_teams(&group, &ComponentGroups::local(&group), &["US"])
                .expect("load Teams.txt");
        let metadata = loaded.expect("Teams.txt metadata").metadata;

        assert_eq!(clonk_script::c4_string_bytes(&teams[0].name), [0xdc; 30]);
        assert_eq!(metadata.teams[0].name.as_bytes(), &[0xdc; 30]);
        assert_eq!(metadata.teams[0].icon_spec.as_bytes(), &[0xdc]);
        assert_eq!(metadata.script_player_names.as_bytes(), &[0xc4, b'|', 0xd6]);
    }

    #[test]
    fn legacy_team_strings_preserve_cp1252_bytes_through_load_runtime_and_save() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        std::fs::write(
            scenario_dir.join("Teams.txt"),
            b"[Teams]\nScriptPlayerNames=$Roster$\n  [Team]\n  id=1\n  Name=$TeamName$\n  Color=1\n  IconSpec=$TeamIcon$\n",
        )
        .expect("write Teams.txt");
        std::fs::write(
            scenario_dir.join("StringTblUS.txt"),
            b"TeamName=Caf\xe9\nTeamIcon=Cr\xe8st:1\nRoster=Andr\xe9|Ren\xe9\n",
        )
        .expect("write byte-native string table");
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario = Scenario::load_from_path_with_languages(&scenario_dir, &resolver, &["US"])
            .expect("legacy scenario loads");
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
        let lobby_teams = scenario
            .lobby_metadata()
            .expect("legacy lobby metadata")
            .teams();
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

        let mut engine = Engine::with_seed(0);
        scenario
            .apply(&mut engine)
            .expect("legacy scenario applies");
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

        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("engine state serializes");
        let state = crate::EngineState::from_json_str(&encoded).expect("engine state deserializes");
        let mut restored = Engine::with_seed(0);
        scenario
            .apply(&mut restored)
            .expect("scenario applies before restore");
        restored
            .restore_state(&state)
            .expect("engine state restores");
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
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("sCeNaRiO.TxT"),
            "[Head]\nLoader=LoaderMixed*\n",
        )
        .expect("mixed-case core");
        let group = Group::open(directory.path()).expect("scenario group");
        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        assert_eq!(head.loader().configured_specification(), "LoaderMixed*");
    }

    #[test]
    fn legacy_landscape_entries_match_case_insensitively_in_directory_and_packed_groups() {
        let directory = tempdir().expect("scenario directory");
        let entries: [(&str, &[u8]); 3] = [
            ("mAp.BmP", b"map"),
            ("lAnDsCaPe.BmP", b"landscape"),
            ("landscape.txt", b"script"),
        ];
        for (name, bytes) in entries {
            std::fs::write(directory.path().join(name), bytes).expect("directory entry");
        }
        let directory_group = Group::open(directory.path()).expect("directory group");

        let mut mutable = clonk_resources::MutableGroup::new("Case.c4s");
        for (name, bytes) in entries {
            mutable
                .add_file(name, bytes.to_vec())
                .expect("packed entry");
        }
        let packed_group = Group::from_memory(
            PathBuf::from("Case.c4s"),
            mutable.pack().expect("packed group image"),
        )
        .expect("packed group");

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
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            concat!(
                "[Head]\n",
                "MinPlayer=0\n",
                "MaxPlayer=0\n",
                "SaveGame=1\n",
                "Replay=0\n",
                "MissionAccess=Secret\n",
                "\n",
                "[Game]\n",
                "Mode=1\n",
            ),
        )
        .expect("scenario core");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        assert_eq!(head.min_players(), 2, "melee derives a two-player floor");
        assert_eq!(head.max_players(), 0);
        assert!(head.is_save_game());
        assert!(!head.is_replay());
        assert_eq!(head.mission_access(), "Secret");
    }

    #[test]
    fn loader_head_retains_native_byte_and_capped_mission_access() {
        let directory = tempdir().expect("scenario directory");
        let mut core = b"[Head]\nMissionAccess=Secr\x80t".to_vec();
        core.extend(std::iter::repeat_n(b'A', 520));
        core.extend_from_slice(b"\n");
        std::fs::write(directory.path().join("Scenario.txt"), core)
            .expect("native-byte scenario core");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        let access = clonk_script::c4_string_bytes(head.mission_access());
        assert_eq!(&access[..6], b"Secr\x80t");
        assert_eq!(access.len(), 512, "C4MaxTitle truncates the fixed buffer");
    }

    #[test]
    fn loader_head_applies_subpath_origin_validation_and_separator_normalization() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\nOrigin=\\..\\Bad*?<>;|:A:B.c4s\n",
        )
        .expect("scenario core");
        let group = Group::open(directory.path()).expect("scenario group");
        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        let expected = if cfg!(windows) {
            "___\\Bad_______A_B.c4s"
        } else {
            "___/Bad_______A_B.c4s"
        };
        assert_eq!(head.origin(), Some(expected));

        std::fs::write(directory.path().join("Scenario.txt"), "[Head]\nOrigin=\n")
            .expect("empty origin");
        let empty = ScenarioLoaderHead::load_from_group(&group).expect("empty origin head");
        assert_eq!(empty.origin(), Some("empty"));
    }

    #[test]
    fn loader_head_parses_only_raw_scenario_core_prefix_before_nul() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            b"[Head]\nTitle=Visible\0\nLoader=LoaderHidden*\nOrigin=Hidden.c4f/Hidden.c4s\n",
        )
        .expect("NUL scenario core");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("visible core prefix");
        assert_eq!(head.scenario_title(), "Visible");
        assert_eq!(head.loader().configured_specification(), "");
        assert_eq!(head.origin(), None);
    }

    #[test]
    fn loader_head_title_ignores_rust_manifest_and_uses_classic_precedence() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\nTitle=Legacy fallback\n",
        )
        .expect("scenario core");
        std::fs::write(
            directory.path().join("Scenario.json"),
            r#"{"name":"Rust-only title","definitions":[]}"#,
        )
        .expect("Rust manifest");
        std::fs::write(
            directory.path().join("tItLeUs.TxT"),
            "US:Localized classic title\n",
        )
        .expect("title component");
        let group = Group::open(directory.path()).expect("scenario group");
        let head = ScenarioLoaderHead::load_from_group_with_languages(&group, &["US", "DE"])
            .expect("loader head");
        assert_eq!(head.scenario_title(), "Localized classic title");

        std::fs::remove_file(directory.path().join("tItLeUs.TxT")).expect("remove title");
        let fallback = ScenarioLoaderHead::load_from_group(&group).expect("fallback head");
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
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Teams.txt"), []).expect("write empty Teams.txt");
        let group = Group::open(dir.path()).expect("open group");
        let (_, loaded) =
            load_initial_network_teams(&group, &ComponentGroups::local(&group), &["US"])
                .expect("load empty Teams.txt");
        assert!(loaded.is_none());

        let core = parse_legacy_scenario_text("[Game]\nStructNeedEnergy=0\n")
            .expect("default cooperative scenario")
            .core;
        let (_, lobby_teams) =
            load_legacy_teams(&group, &ComponentGroups::local(&group), &["US"], &core)
                .expect("load lobby team metadata");
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
        .expect("warnings retain compiler defaults");
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
        scenario.legacy_team_metadata = Some(
            parse_legacy_team_metadata_source("[Teams]\nTeamDistribution=9\n")
                .expect("numeric enum compiles as uint8"),
        );

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
            .expect("legacy initial network scenario serializes");
        let serialized = String::from_utf8(serialized).expect("Scenario.txt is UTF-8");

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
            .expect("legacy initial network scenario serializes");

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
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\nTitle=Fallback\n",
        )
        .expect("scenario core");
        std::fs::write(
            directory.path().join("TitleUS.txt"),
            b"US:one\ntwo\rignored",
        )
        .expect("mixed-newline title component");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        assert_eq!(head.scenario_title(), "one\ntwo");
    }

    #[test]
    fn loader_head_title_skips_zero_size_language_component() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\nTitle=Head fallback\n",
        )
        .expect("scenario core");
        std::fs::write(directory.path().join("TitleUS.txt"), [])
            .expect("empty localized title component");
        std::fs::write(directory.path().join("Title.txt"), b"US:Plain fallback\n")
            .expect("plain title component");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        assert_eq!(head.scenario_title(), "Plain fallback");
    }

    #[test]
    fn loader_head_title_ignores_component_data_after_nul() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\nTitle=Head fallback\n",
        )
        .expect("scenario core");
        std::fs::write(
            directory.path().join("TitleUS.txt"),
            b"prefix\0US:Wrong suffix",
        )
        .expect("NUL title component");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        assert_eq!(head.scenario_title(), "Head fallback");
    }

    #[test]
    fn loader_head_title_decodes_legacy_cp1252_for_presentation() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\nTitle=Head fallback\n",
        )
        .expect("scenario core");
        std::fs::write(directory.path().join("TitleUS.txt"), b"US:Caf\xe9\n")
            .expect("legacy title component");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        assert_eq!(head.scenario_title(), "Caf\u{e9}");
        assert_eq!(head.scenario_title_bytes(), b"Caf\xe9");
    }

    #[test]
    fn loader_head_fallback_title_preserves_native_cp1252_bytes() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            b"[Head]\nTitle=S\xe4uresee\n",
        )
        .expect("legacy scenario core");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        assert_eq!(head.scenario_title(), "S\u{e4}uresee");
        assert_eq!(head.scenario_title_bytes(), b"S\xe4uresee");
    }

    #[test]
    fn loader_head_title_uses_classic_nonoverlapping_ssearch() {
        let directory = tempdir().expect("scenario directory");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\nTitle=Head fallback\n",
        )
        .expect("scenario core");
        std::fs::write(directory.path().join("TitleAA.txt"), b"AAA:Wrong")
            .expect("overlapping-prefix title component");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group_with_languages(&group, &["AA"])
            .expect("loader head");
        assert_eq!(head.scenario_title(), "Head fallback");
    }

    #[test]
    fn loader_head_fallback_title_truncates_native_bytes_like_cpp() {
        let directory = tempdir().expect("scenario directory");
        let title = format!("{}é", "A".repeat(119));
        std::fs::write(
            directory.path().join("Scenario.txt"),
            format!("[Head]\nTitle={title}\n"),
        )
        .expect("scenario core");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group(&group).expect("loader head");
        let source = title.as_bytes();
        assert_eq!(head.scenario_title_bytes(), &source[..120]);
    }

    #[test]
    fn resource_registration_head_never_resolves_unrelated_title_data() {
        let directory = tempdir().expect("scenario directory");
        let title = format!("{}é", "A".repeat(119));
        std::fs::write(
            directory.path().join("Scenario.txt"),
            format!(
                "[Head]\nTitle={title}\nOrigin=Parent.c4s\n\
                 \n[Definitions]\nLocalOnly=1\nDefinition1=Objects.c4d\n"
            ),
        )
        .expect("scenario core");
        let group = Group::open(directory.path()).expect("scenario group");

        let head = ScenarioLoaderHead::load_from_group_for_resource_registration(&group)
            .expect("resource registration head");
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
        put_i32(
            &mut header,
            36,
            i32::try_from(entries.len()).expect("test entry count fits i32"),
        );
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
            put_i32(
                &mut entry,
                268,
                i32::try_from(data.len()).expect("test entry size fits i32"),
            );
            put_i32(
                &mut entry,
                276,
                i32::try_from(data_offset).expect("test offset fits i32"),
            );
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
                    .expect("packed child adds");
            } else {
                group
                    .add_file((*name).to_owned(), data.to_vec())
                    .expect("packed file adds");
            }
        }
        group.pack().expect("standalone packed group compresses")
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
        let expected =
            parse_legacy_team_metadata_source(&crlf).expect("CRLF exact Teams.txt metadata parses");

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

        let invalid_lf = "[Teams]\n  [Team]\n  id=broken\n";
        for (label, source) in [
            ("LF", invalid_lf.to_string()),
            ("CRLF", invalid_lf.replace('\n', "\r\n")),
            ("CR", invalid_lf.replace('\n', "\r")),
        ] {
            let error =
                parse_legacy_team_metadata_source(&source).expect_err("invalid team id must fail");
            assert!(
                error.to_string().contains("Teams.txt line 3:"),
                "{label} diagnostics preserve physical line numbers: {error}"
            );
        }
    }

    #[test]
    fn legacy_lobby_projection_keeps_scenario_parameter_and_team_boundaries() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Lobby Probe\nLoader=\nMaxPlayer=7\nMinPlayer=0\n\
             SaveGame=1\nReplay=0\nForcedNoCrew=2\nDefCrewStrength=99\n\
             NetworkGame=1\nNetworkRuntimeJoin=1\n\n[Definitions]\n\
             AllowUserChange=1\nDefinition1=Defs.c4d\n\n[Game]\nMode=1\n",
        )
        .expect("write scenario");
        std::fs::write(
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
        )
        .expect("write parameters");
        std::fs::write(
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
        )
        .expect("write teams");
        std::fs::write(
            scenario_dir.join("Game.txt"),
            "[DefinitionFiles]\nDefinition1=OldObjects.c4d\nDefinition2=OldPack.c4d\n\n[Game]\n",
        )
        .expect("write game text");
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let lobby = scenario.lobby_metadata().expect("legacy lobby metadata");
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
        assert!(definitions.resolved_load_resources().is_none());
        assert!(definitions.effective_modules().is_none());
        assert_eq!(
            definitions
                .savegame_override()
                .definition_lines()
                .expect("game definition lines"),
            ["Definition1=OldObjects.c4d", "Definition2=OldPack.c4d"]
        );

        let parameters = lobby
            .embedded_game_parameters()
            .expect("Parameters.txt boundary retained");
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
        let merged = lobby
            .embedded_game_parameter_values()
            .expect("defaulted embedded values");
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
            .expect("read scenario")
            .replace("SaveGame=1", "SaveGame=0")
            .replace("Replay=0", "Replay=1");
        std::fs::write(scenario_dir.join("Scenario.txt"), replay_core).expect("write replay core");
        let replay = Scenario::load_from_path_with(&scenario_dir, &resolver).expect("replay loads");
        let replay_lobby = replay.lobby_metadata().expect("replay metadata");
        assert!(replay_lobby.head().is_replay());
        assert!(!replay_lobby.head().is_save_game());
        assert_eq!(
            replay_lobby.definitions().savegame_override(),
            &ScenarioSavegameDefinitionOverride::None
        );
        assert_eq!(
            replay_lobby
                .definitions()
                .effective_modules()
                .expect("replay modules are effective"),
            ["Defs.c4d"]
        );
    }

    #[test]
    fn legacy_lobby_defaults_distinguish_min_players_and_missing_teams() {
        let cooperative = parse_legacy_scenario_text(
            "[Head]\nMaxPlayer=5\nMaxPlayer=broken-but-ignored\n\n[Game]\nRules=RVLR=1\n",
        )
        .expect("cooperative core parses");
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

        let melee = parse_legacy_scenario_text("[Head]\n\n[Game]\nGoals=MELE=1\n")
            .expect("melee core parses");
        assert_eq!(legacy_effective_min_players(&melee.core), 2);
        assert_eq!(melee.core.head.max_player, 12);
        assert_eq!(melee.core.head.max_player_league, 12);

        let duplicate_melee = parse_legacy_scenario_text("[Head]\n\n[Game]\nGoals=MELE=0;MELE=1\n")
            .expect("duplicate goals parse");
        assert_eq!(
            legacy_effective_min_players(&duplicate_melee.core),
            1,
            "C4IDList::GetIDCount uses the first duplicate"
        );
        assert!(!ScenarioValueStore::from_runtime_core(&duplicate_melee.core, false).is_melee());
        let duplicate_melee_reversed =
            parse_legacy_scenario_text("[Head]\n\n[Game]\nGoals=MELE=1;MELE=0\n")
                .expect("reversed duplicate goals parse");
        assert_eq!(
            legacy_effective_min_players(&duplicate_melee_reversed.core),
            2
        );
        assert!(
            ScenarioValueStore::from_runtime_core(&duplicate_melee_reversed.core, false).is_melee()
        );

        let explicit =
            parse_legacy_scenario_text("[Head]\nMinPlayer=-2\n").expect("explicit minimum parses");
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
                death_message: String::new(),
                core: Default::default(),
                rank: 0,
                rank_name: "Clonk".to_string(),
                experience: 0,
                rounds,
                physical: Default::default(),
                death_count: 0,
                total_playing_time,
                birthday: 0,
                age: 0,
                participation: 1,
                in_action,
                was_in_action,
                in_action_time,
                has_died: false,
                extra_data: Vec::new(),
                portraits: Default::default(),
            }
        }

        fn evaluate(source: &str) -> Engine {
            let parsed = parse_legacy_scenario_text(source).expect("scenario core parses");
            let mut engine = Engine::new();
            engine.set_scenario_values(ScenarioValueStore::from_runtime_core(&parsed.core, false));
            engine
                .register_player(
                    crate::PlayerConfig::new(0, "Winner")
                        .with_score(250)
                        .with_rounds(4, 2, 2)
                        .with_initial_value(100),
                )
                .expect("winner registers");
            engine
                .register_player(
                    crate::PlayerConfig::new(1, "Loser")
                        .with_status(crate::PlayerStatus::Eliminated)
                        .with_score(80)
                        .with_rounds(7, 5, 2)
                        .with_initial_value(100),
                )
                .expect("loser registers");
            engine
                .player_mut(0)
                .expect("winner exists")
                .update_asset_value(165, 0);
            engine
                .player_mut(1)
                .expect("loser exists")
                .update_asset_value(75, 0);
            engine.game_time = 20;
            engine.crew_rosters.insert(
                0,
                vec![
                    crew_info("Active", 3, 10, true, true, 5),
                    crew_info("Retired", 7, 17, false, true, 4),
                    crew_info("Unused", 9, 30, false, false, 0),
                ],
            );
            engine.evaluate_game().expect("game evaluates");
            engine
        }

        let cooperative = evaluate("[Game]\n");
        let coop_winner = cooperative.player(0).expect("co-op winner remains");
        let coop_loser = cooperative.player(1).expect("co-op loser remains");
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
        let roster = melee.crew_rosters.get(&0).expect("crew roster remains");
        assert_eq!((roster[0].rounds, roster[0].total_playing_time), (4, 25));
        assert!(!roster[0].in_action, "active crew is retired at evaluation");
        assert_eq!((roster[1].rounds, roster[1].total_playing_time), (8, 17));
        assert_eq!((roster[2].rounds, roster[2].total_playing_time), (9, 30));

        let before = melee.capture_state();
        melee.evaluate_game().expect("second evaluation is ignored");
        assert_eq!(melee.capture_state().players, before.players);
        assert_eq!(
            melee.capture_state().crew_info_rosters,
            before.crew_info_rosters
        );
    }

    #[test]
    fn delayed_player_retirement_uses_the_same_melee_and_crew_evaluation() {
        let parsed =
            parse_legacy_scenario_text("[Game]\nGoals=MELE=1\n").expect("melee core parses");
        let mut engine = Engine::new();
        engine.set_scenario_values(ScenarioValueStore::from_runtime_core(&parsed.core, false));
        engine
            .register_player(
                crate::PlayerConfig::new(3, "Retiring")
                    .with_status(crate::PlayerStatus::Eliminated)
                    .with_score(10)
                    .with_rounds(2, 1, 1)
                    .with_initial_value(100),
            )
            .expect("retiring player registers");
        engine
            .register_player(crate::PlayerConfig::new(4, "Peer").with_initial_value(100))
            .expect("peer registers");
        engine
            .player_mut(3)
            .expect("retiring player exists")
            .update_asset_value(140, 0);
        engine
            .player_mut(4)
            .expect("peer exists")
            .update_asset_value(200, 0);
        engine.game_time = 12;
        engine.crew_rosters.insert(
            3,
            vec![crate::player_file::CrewInfo {
                id: "CLNK".to_string(),
                name: "Retiring crew".to_string(),
                death_message: String::new(),
                core: Default::default(),
                rank: 0,
                rank_name: "Clonk".to_string(),
                experience: 0,
                rounds: 5,
                physical: Default::default(),
                death_count: 0,
                total_playing_time: 3,
                birthday: 0,
                age: 0,
                participation: 1,
                in_action: true,
                was_in_action: true,
                in_action_time: 2,
                has_died: false,
                extra_data: Vec::new(),
                portraits: Default::default(),
            }],
        );

        let retired = engine.retire_player(3).expect("player retires");
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

        let core = parse_legacy_scenario_text("[Head]\nForcedNoCrew=2\n").expect("default core");
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
        let manifest = parse_legacy_scenario_text(concat!(
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
        ))
        .expect("exact scenario INI tree parses");

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

        let malformed = parse_legacy_scenario_text("[Head]\nMaxPlayer=broken\n")
            .expect("malformed scalar uses DefaultAdapt");
        assert_eq!(malformed.core.head.max_player, 12);
        assert_eq!(malformed.core.head.max_player_league, 12);

        let integer_flag = parse_legacy_scenario_text("[Head]\nSaveGame=true\n")
            .expect("integer-backed flag defaults on a boolean token");
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
        let expected = parse_legacy_scenario_text(&crlf).expect("CRLF Scenario.txt parses");

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
        let core = parse_legacy_scenario_text("[Head]\nForcedNoCrew=2\n").expect("default core");
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
        let directory = tempdir().expect("replay group");
        std::fs::write(
            directory.path().join("Scenario.txt"),
            "[Head]\nReplay=1\nRandomSeed=41\n",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("Parameters.txt"),
            "[Parameters]\nRandomSeed=73\nStartupPlayerCount=4\n",
        )
        .unwrap();
        std::fs::write(
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
        )
        .unwrap();
        let group = Group::open(directory.path()).unwrap();

        assert_eq!(
            Scenario::preflight_replay_startup_from_group(&group).unwrap(),
            Some(ReplayScenarioStartupPreflight {
                random_seed: 73,
                startup_player_count: 2,
            })
        );

        std::fs::write(directory.path().join("Game.txt"), "[Game]\nFrame=37\n").unwrap();
        let group = Group::open(directory.path()).unwrap();
        assert_eq!(
            Scenario::preflight_replay_startup_from_group(&group).unwrap(),
            Some(ReplayScenarioStartupPreflight {
                random_seed: 73,
                startup_player_count: 4,
            }),
            "nonzero-frame records retain the serialized parameter"
        );

        std::fs::remove_file(directory.path().join("Game.txt")).unwrap();
        std::fs::remove_file(directory.path().join("PlayerInfos.txt")).unwrap();
        let group = Group::open(directory.path()).unwrap();
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
        let manifest = parse_legacy_scenario_text(concat!(
            "[Head]\n",
            "Version=4,bad,6\n",
            "[Player1]\n",
            "Position=22,bad\n",
            "[Landscape]\n",
            "SkyFade=1,bad,3\n",
            "MapWidth=101,bad,70,300\n",
        ))
        .expect("malformed components use their adaptor defaults");

        assert_eq!(manifest.core.head.version, [4, 0, 0, 0, 0]);
        assert_eq!(manifest.core.players[0].position, [22, -1]);
        assert_eq!(manifest.core.landscape.sky_fade, [1, 0, 0, 0, 0, 0]);
        assert_eq!(
            manifest.core.landscape.map_width,
            LegacyC4SVal::new(101, 0, 64, 250)
        );
    }

    #[test]
    fn scenario_name_lists_match_c4namelist_bounds_and_identifier_tokens() {
        let layers = (1..=11)
            .map(|index| format!("Layer{index}=50"))
            .collect::<Vec<_>>()
            .join(";");
        let manifest = parse_legacy_scenario_text(&format!(
            "[Game]\nClearMaterials={layers}\n[Landscape]\nLayers={layers}\n"
        ))
        .expect("bounded C4NameList fields parse");
        assert_eq!(manifest.core.game.clear_materials.len(), 10);
        assert_eq!(manifest.core.landscape.layers.len(), 10);
        assert_eq!(manifest.core.landscape.layers[0].name, "Layer1");
        assert_eq!(manifest.core.landscape.layers[9].name, "Layer10");

        let malformed = parse_legacy_scenario_text(concat!(
            "[Game]\n",
            "ClearMaterials=ABCDEFGHIJKLMNOPQRSTUVWXYZ12345=2;Gold=3\n",
            "[Landscape]\n",
            "Layers=My Rock=2;Earth=3\n",
        ))
        .expect("truncated name lists use compiler defaults");
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

        let reentered = parse_legacy_scenario_text(concat!(
            "[Game]\n",
            "ClearMaterials=A=1=2;B=3\n",
            "[Landscape]\n",
            "Layers=\u{a0}Gold=1\n",
        ))
        .expect("separator reentry and raw identifier bytes parse");
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

    #[test]
    fn savegame_definition_boundary_precedes_missing_scenario_modules() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = dir.path().join("OldSave.c4s");
        std::fs::create_dir(&scenario_dir).expect("create scenario");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Old Save\nSaveGame=1\n\n[Definitions]\nDefinition1=Missing.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            scenario_dir.join("Game.txt"),
            "[DefinitionFiles]\nDefinition1=Historical.c4d\n\n[Game]\n",
        )
        .expect("write old save definition vector");
        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };

        let scenario = Scenario::load_from_path_with(&scenario_dir, &resolver)
            .expect("unresolved old-save boundary remains inspectable");
        let definitions = scenario
            .lobby_metadata()
            .expect("legacy lobby metadata")
            .definitions();
        assert_eq!(definitions.configured_modules(), ["Missing.c4d"]);
        assert_eq!(definitions.requested_modules(), ["Missing.c4d"]);
        assert_eq!(definitions.effective_modules(), None);
        assert_eq!(
            definitions
                .savegame_override()
                .definition_lines()
                .expect("Game.txt boundary"),
            ["Definition1=Historical.c4d"]
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
        let scenario_source = decode_legacy_script_text(
            &std::fs::read(&scenario_path).expect("read shipped Scenario.txt"),
        );
        let team_source =
            decode_legacy_script_text(&std::fs::read(&teams_path).expect("read shipped Teams.txt"));
        let manifest =
            parse_legacy_scenario_text(&scenario_source).expect("parse Canyon Scenario.txt");
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
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Scenario.json"),
            r#"{"name":"Fixture","definitions":[{"id":"TEST","script":"Def.c"}]}"#,
        )
        .expect("write manifest");
        std::fs::write(dir.path().join("Def.c"), "// fixture\n").expect("write script");
        let scenario = Scenario::load_from_path(dir.path()).expect("JSON scenario loads");
        assert!(scenario.lobby_metadata().is_none());
    }

    #[test]
    fn json_manifest_script_paths_preserve_native_bytes() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Scenario.json"),
            r#"{"definitions":[{"id":"TEST","script":"Def.c"}],"script":"Scenario.c"}"#,
        )
        .expect("write manifest");
        std::fs::write(
            dir.path().join("Def.c"),
            [
                b"#strict\nfunc Raw() { return \"".as_slice(),
                &[0xe9, 0xff],
                b"\"; }\n",
            ]
            .concat(),
        )
        .expect("write raw definition source");
        std::fs::write(
            dir.path().join("Scenario.c"),
            [
                b"#strict\nglobal func Initialize(state, random) { Message(\"".as_slice(),
                &[0xe9, 0xff],
                b"\"); }\n",
            ]
            .concat(),
        )
        .expect("write raw scenario source");

        let scenario = Scenario::load_from_path(dir.path()).expect("JSON scenario loads");
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
        scenario.apply(&mut engine).expect("raw sources compile");
        assert_eq!(
            clonk_script::c4_string_bytes(&engine.snapshot().hud.messages[0].lines[0]),
            [0xe9, 0xff]
        );
        let object = engine
            .spawn_object(SpawnConfig::new("TEST"))
            .expect("TEST spawns");
        let index = engine.find_object_index(object).expect("TEST index");
        assert_eq!(
            engine
                .call_object_function(index, "Raw", Vec::new())
                .expect("raw definition function runs"),
            clonk_script::Value::String(clonk_script::c4_string_from_bytes(&[0xe9, 0xff]).into())
        );
    }

    #[test]
    fn json_scenario_with_missing_definition_include_still_applies() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("Scenario.json"),
            r#"{"name":"Missing include","definitions":[{"id":"TEST","script":"Def.c"}]}"#,
        )
        .expect("write manifest");
        std::fs::write(
            dir.path().join("Def.c"),
            "#include MISS\npublic func OwnValue() { return 42; }\n",
        )
        .expect("write definition script");

        let scenario = Scenario::load_from_path(dir.path()).expect("JSON scenario loads");
        let mut engine = Engine::new();
        scenario
            .apply(&mut engine)
            .expect("missing include does not abort complete scenario apply");
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
        for path in team_files {
            let bytes = std::fs::read(&path).expect("read shipped Teams.txt");
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
                assert!(
                    crate::text_spec::parse_text_spec(icon).is_some(),
                    "{} retained invalid IconSpec `{icon}`",
                    path.display()
                );
                icon_count += 1;
            }
        }
        assert_eq!(files_with_icons, 19, "recursive team-file census changed");
        assert_eq!(icon_count, 39, "recursive IconSpec census changed");
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
        assert_eq!(
            LegacyC4SVal::new(10, 3, 0, 250).evaluate(&mut rng),
            expected
        );
        assert_eq!(rng, reference);

        // Rnd == 0 still draws: Random(1) returns 0 but advances hold/count.
        let before = rng.clone();
        assert_eq!(LegacyC4SVal::new(5, 0, 0, 250).evaluate(&mut rng), 5);
        assert_ne!(rng.hold, before.hold);
        assert_eq!(rng.count, before.count + 1);
    }

    #[test]
    fn legacy_fow_color_minus_one_reinterprets_as_u32_max() {
        let manifest =
            parse_legacy_scenario_text("[Game]\nFoWColor=-1\n").expect("signed FoWColor parses");

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
            let manifest = parse_legacy_scenario_text(&format!("[Game]\nFoWColor={raw}\n"))
                .expect("FoWColor prefix parses");

            assert_eq!(manifest.core.game.fow_color, expected, "FoWColor={raw}");
        }
    }

    #[test]
    fn legacy_fow_color_keeps_in_range_decimal_and_hex_values() {
        for (raw, expected) in [("305419896", 0x1234_5678), ("0x89abcdef", 0x89ab_cdef)] {
            let manifest = parse_legacy_scenario_text(&format!("[Game]\nFoWColor={raw}\n"))
                .expect("in-range FoWColor parses");

            assert_eq!(manifest.core.game.fow_color, expected, "FoWColor={raw}");
        }
    }

    #[test]
    fn scenario_value_store_reinterprets_fow_color_as_signed_i32() {
        let manifest =
            parse_legacy_scenario_text("[Game]\nFoWColor=-1\n").expect("signed FoWColor parses");
        let values = ScenarioValueStore::from_runtime_core(&manifest.core, false);

        assert_eq!(
            values.get("FoWColor", Some("Game"), 0),
            Some(&ScenarioValue::Int(-1))
        );
        assert_eq!(values.fow_color(), u32::MAX);
        assert_eq!(values.fow_resolution(), 64);
    }

    #[test]
    fn scenario_value_store_projects_fow_resolution_for_the_renderer() {
        let manifest =
            parse_legacy_scenario_text("[Landscape]\nFoWRes=96\n").expect("FoWRes parses");
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

        let manifest = parse_legacy_scenario_text(legacy).expect("legacy scenario manifest parses");
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

