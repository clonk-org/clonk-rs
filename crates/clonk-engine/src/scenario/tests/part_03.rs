// Contiguous slice 3 of 8 of the `scenario/tests` battery, spliced
// by `include!` from the parent module so every test id is unchanged.

    #[test]
    fn modern_definition_list_uses_shared_parser_and_first_key() {
        let manifest = parse_legacy_scenario_text(
            "[Definitions]\n\
             Definitions=\"First.c4d\",\"Comma,Pack.c4d\",\"First.c4d\"\n\
             Definitions=\"Ignored.c4d\"\n\
             Definition1=Fallback.c4d\n",
        ).test_value();
        assert_eq!(
            manifest.definition_specs,
            ["First.c4d", "Comma,Pack.c4d", "First.c4d"]
        );
    }

    #[test]
    fn modern_definition_list_preserves_post_equals_whitespace_mode() {
        let manifest = parse_legacy_scenario_text(
            "[Definitions]\nDefinitions= \"First.c4d\",\"Second.c4d\"\n",
        ).test_value();
        assert_eq!(manifest.definition_specs, ["\"First.c4d\",\"Second.c4d\""]);
    }

    #[test]
    fn definition_parser_keeps_first_section_and_first_scalar_values() {
        let manifest = parse_legacy_scenario_text(
            "[Definitions]\n\
             LocalOnly=1\n\
             LocalOnly=not-a-bool\n\
             AllowUserChange=1\n\
             AllowUserChange=0\n\
             SkipDefs=FIRS\n\
             SkipDefs=SCND\n\
             Definition1=First.c4d\n\
             [Definitions]\n\
             LocalOnly=0\n\
             AllowUserChange=0\n\
             SkipDefs=LAST\n\
             Definition1=Second.c4d\n",
        ).test_value();

        assert!(manifest.core.definitions.local_only);
        assert!(manifest.core.definitions.allow_user_change);
        assert_eq!(manifest.core.definitions.skip_defs.len(), 1);
        assert_eq!(manifest.core.definitions.skip_defs[0].id, "FIRS");
        assert_eq!(manifest.definition_specs, ["First.c4d"]);
    }

    #[test]
    fn definition_parser_preserves_modern_empty_but_ignores_numbered_empty() {
        let modern = parse_legacy_scenario_text(
            "[Definitions]\nDefinitions=\"\"\nDefinition1=Fallback.c4d\n",
        ).test_value();
        assert_eq!(modern.definition_specs, [""]);

        let bare =
            parse_legacy_scenario_text("[Definitions]\nDefinitions=\nDefinition1=Fallback.c4d\n").test_value();
        assert_eq!(bare.definition_specs, [""]);

        let numbered =
            parse_legacy_scenario_text("[Definitions]\nDefinition1=\nDefinition2=Second.c4d\n").test_value();
        assert_eq!(numbered.definition_specs, ["Second.c4d"]);
    }

    #[test]
    fn definitions_section_and_key_names_are_cpp_case_sensitive_and_exact() {
        let manifest = parse_legacy_scenario_text(
            "[definitions]\n\
             Definitions=WrongSection.c4d\n\
             [ Definitions ]\n\
             Definitions=WrongWhitespace.c4d\n\
             [Definitions]\n\
             LOCALONLY=0\n\
             LocalOnly=1\n\
             allowuserchange=0\n\
             AllowUserChange=1\n\
             skipdefs=WRNG\n\
             SkipDefs=GOOD\n\
             definitions=WrongKey.c4d\n\
             Definition01=WrongAlias.c4d\n\
             definition1=WrongCase.c4d\n\
             Definition1=Right.c4d\n",
        ).test_value();

        assert!(manifest.core.definitions.local_only);
        assert!(manifest.core.definitions.allow_user_change);
        assert_eq!(manifest.core.definitions.skip_defs.len(), 1);
        assert_eq!(manifest.core.definitions.skip_defs[0].id, "GOOD");
        assert_eq!(manifest.definition_specs, ["Right.c4d"]);
    }

    #[test]
    fn definitions_names_accept_tabs_but_preserve_spaces_like_stdcompiler() {
        let tabbed = parse_legacy_scenario_text(
            "[Definitions\t ]\n\
             LocalOnly\t =1\n\
             [!malformed]\n\
             AllowUserChange\t =1\n\
             SkipDefs\t =GOOD\n\
             Definitions\t =Tabbed.c4d\n",
        ).test_value();

        assert!(tabbed.core.definitions.local_only);
        assert!(tabbed.core.definitions.allow_user_change);
        assert_eq!(tabbed.core.definitions.skip_defs.len(), 1);
        assert_eq!(tabbed.core.definitions.skip_defs[0].id, "GOOD");
        assert_eq!(tabbed.definition_specs, ["Tabbed.c4d"]);

        let spaced = parse_legacy_scenario_text(
            "[Definitions ]\n\
             Definition1=WrongSection.c4d\n\
             [Definitions]\n\
             LocalOnly =1\n\
             AllowUserChange =1\n\
             SkipDefs =WRNG\n\
             Definitions =WrongModern.c4d\n\
             Definition1 =WrongNumbered.c4d\n\
             Definition1=Right.c4d\n",
        ).test_value();

        assert!(!spaced.core.definitions.local_only);
        assert!(!spaced.core.definitions.allow_user_change);
        assert!(spaced.core.definitions.skip_defs.is_empty());
        assert_eq!(spaced.definition_specs, ["Right.c4d"]);
    }

    #[test]
    fn modern_definition_paths_normalize_classic_backslashes() {
        let manifest = parse_legacy_scenario_text(
            r#"[Definitions]
        Definitions="Western.c4f\\Misc.c4d"
        "#,
        ).test_value();
        assert_eq!(manifest.definition_specs, ["Western.c4f/Misc.c4d"]);
    }

    #[test]
    fn loads_flat_landscape_scenario() {
        let dir = test_tempdir();
        // Keep the moving fixture in C4D_Object: C4Object::SyncClearance
        // zeroes C4D_StaticBack velocity before players join
        // (C4Object.cpp:3830-3850; C4Game.cpp:473-475).
        let manifest = r#"
        {
            "name": "Temp Scenario",
            "ticks": 240,
            "landscape": { "kind": "flat", "width": 128, "height": 42 },
            "definitions": [
                { "id": "Mover", "name": "Mover", "script": "scripts/mover.aul", "category": 16 }
            ],
            "initial_objects": [
                { "definition": "Mover", "position": [10, 20], "velocity": [1, -1], "energy": 99 }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        assert_eq!(scenario.name(), Some("Temp Scenario"));
        assert_eq!(scenario.configured_ticks(), Some(240));
        assert_eq!(scenario.ground_height_hint(), Some(42));
        assert!(scenario.has_initial_objects());

        let mut engine = Engine::with_seed(0);
        let created = apply_test_scenario(&scenario, &mut engine);
        assert_eq!(created.len(), 1);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.objects.len(), 1);
        let object = &snapshot.objects[0];
        assert_eq!(object.definition_id, "Mover");
        assert_eq!(object.position, Vector2::new(10, 20));
        assert_eq!(object.velocity, Vector2::new(1, -1));
        assert_eq!(object.energy, 99);

        let landscape = engine.landscape().test_value();
        assert_eq!(landscape.surface_height(0), Some(42));
        assert!(scenario.physics().is_none());
    }

    #[test]
    fn applies_action_configuration_from_manifest() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                {
                    "id": "Mover",
                    "script": "scripts/mover.aul",
                    "default_action": "Walk",
                    "actions": {
                        "Walk": { "length": 2, "delay": 1, "next": "Idle" },
                        "Idle": { "length": 1 }
                    }
                }
            ],
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let mut engine = Engine::with_seed(5);
        let created = apply_test_scenario(&scenario, &mut engine);
        let id = created[0];

        let initial = engine.object_snapshot(id).test_value();
        assert_eq!(initial.action.name, "Walk");
        assert_eq!(initial.action.phase, 0);

        let snapshot = engine.tick().test_value();
        let object = snapshot.object(id).test_value();
        assert_eq!(object.action.name, "Walk");
        assert_eq!(object.action.phase, 1);

        let snapshot = engine.tick().test_value();
        let object = snapshot.object(id).test_value();
        assert_eq!(object.action.name, "Idle");
        assert_eq!(object.action.phase, 0);
    }

    #[test]
    fn seeds_initial_action_and_effects_from_manifest() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                {
                    "id": "Mover",
                    "script": "scripts/mover.aul",
                    "default_action": "Idle",
                    "actions": {
                        "Idle": { "length": 1 },
                        "Walk": { "length": 5, "next": "Idle" }
                    }
                }
            ],
            "initial_objects": [
                {
                    "definition": "Mover",
                    "action": { "name": "Walk", "phase": 3 },
                    "effects": [
                        { "name": "Intoxicated", "priority": 150, "interval": 3, "timer": 5 }
                    ]
                }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let mut engine = Engine::with_seed(0);
        let created = apply_test_scenario(&scenario, &mut engine);
        assert_eq!(created.len(), 1);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.objects.len(), 1);
        let object = &snapshot.objects[0];
        assert_eq!(object.action.name, "Walk");
        assert_eq!(object.action.phase, 3);
        assert_eq!(object.effects.len(), 1);
        let effect = &object.effects[0];
        assert_eq!(effect.name, "Intoxicated");
        assert_eq!(effect.priority, 150);
        assert_eq!(effect.interval, 3);
        // iTime is stored verbatim - C++ never wraps it (C4Effect.cpp:66-67).
        assert_eq!(effect.timer, 5);
    }

    #[test]
    fn seeds_initial_status_from_manifest() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                {
                    "id": "Mover",
                    "script": "scripts/mover.aul",
                    "default_action": "Idle",
                    "actions": { "Idle": { "length": 1 } }
                }
            ],
            "initial_objects": [
                {
                    "definition": "Mover",
                    "status": "inactive"
                }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let mut engine = Engine::with_seed(0);
        let created = apply_test_scenario(&scenario, &mut engine);
        let id = created[0];

        let snapshot = engine.snapshot();
        let object = snapshot.object(id).test_value();
        assert_eq!(object.status, ObjectStatus::Inactive);
        let initial_phase = object.action.phase;

        let ticked = engine.tick().test_value();
        let object = ticked.object(id).test_value();
        assert_eq!(object.status, ObjectStatus::Inactive);
        assert_eq!(object.action.phase, initial_phase);
    }

    #[test]
    fn spawns_contents_with_container_handles() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Chest", "script": "scripts/chest.aul" },
                { "id": "Gem", "script": "scripts/gem.aul" }
            ],
            "initial_objects": [
                { "definition": "Chest", "handle": "store" },
                { "definition": "Gem", "container": "store" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/chest.aul"), TEST_SCRIPT);
        write_test_file(dir.path().join("scripts/gem.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let mut engine = Engine::with_seed(0);
        let created = apply_test_scenario(&scenario, &mut engine);
        assert_eq!(created.len(), 2);

        let snapshot = engine.snapshot();
        let chest = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "Chest").test_value();
        let gem = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "Gem").test_value();
        assert_eq!(gem.container, Some(chest.id));
        assert!(chest.contents.contains(&gem.id));
    }

    #[test]
    fn errors_on_unknown_container_handle() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Chest", "script": "scripts/chest.aul" },
                { "id": "Gem", "script": "scripts/gem.aul" }
            ],
            "initial_objects": [
                { "definition": "Gem", "container": "missing" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/chest.aul"), TEST_SCRIPT);
        write_test_file(dir.path().join("scripts/gem.aul"), TEST_SCRIPT);

        let error = Scenario::load_from_path(dir.path()).expect_err("scenario fails");
        match error {
            ScenarioError::UnknownContainerHandle(handle) => assert_eq!(handle, "missing"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn container_cycles_degrade_to_partial_containment() {
        // C++ resolves Contained by two-phase denumeration, so mutual
        // containment loads without error. The sequential spawn model
        // breaks ONE edge (documented divergence) — both objects must
        // exist, with one containment intact.
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Crate", "script": "scripts/crate.aul" },
                { "id": "Barrel", "script": "scripts/barrel.aul" }
            ],
            "initial_objects": [
                { "definition": "Crate", "handle": "crate", "container": "barrel" },
                { "definition": "Barrel", "handle": "barrel", "container": "crate" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/crate.aul"), TEST_SCRIPT);
        write_test_file(dir.path().join("scripts/barrel.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let mut engine = Engine::with_seed(0);
        let created = apply_test_scenario(&scenario, &mut engine);
        assert_eq!(created.len(), 2, "both cycle members spawn");
        let contained_count = created
            .iter()
            .filter_map(|id| engine.object_snapshot(*id))
            .filter(|snapshot| snapshot.container.is_some())
            .count();
        assert_eq!(contained_count, 1, "one containment edge survives");
    }

    #[test]
    fn errors_on_unknown_definition_reference() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "initial_objects": [
                { "definition": "Missing" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let error = Scenario::load_from_path(dir.path()).expect_err("scenario fails");
        match error {
            ScenarioError::UnknownDefinition(name) => assert_eq!(name, "Missing"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn manifest_missing_returns_error() {
        let dir = test_tempdir();
        let error = Scenario::load_from_path(dir.path()).expect_err("scenario fails");
        assert!(matches!(error, ScenarioError::ManifestMissing));
    }

    #[test]
    fn loads_physics_overrides() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "physics": {
                "gravity": 2,
                "max_fall_speed": 8,
                "max_rise_speed": -10,
                "max_horizontal_speed": 7
            }
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let physics = scenario.physics().test_value();
        assert_eq!(physics.gravity, 2);
        assert_eq!(physics.max_fall_speed, 8);
        assert_eq!(physics.max_rise_speed, -10);
        assert_eq!(physics.max_horizontal_speed, 7);

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let configured = engine.physics();
        assert_eq!(configured.gravity, 2);
        assert_eq!(configured.max_fall_speed, 8);
        assert_eq!(configured.max_rise_speed, -10);
        assert_eq!(configured.max_horizontal_speed, 7);
    }

    #[test]
    fn loads_environment_settings_and_applies_to_engine() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": -3
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let environment = scenario.environment().test_value();
        assert_eq!(environment.wind, -3);
        assert_eq!(environment.wind_variation, 0);
        assert_eq!(environment.wind_period, 0);
        assert_eq!(environment.temperature, 0);
        assert_eq!(environment.precipitation, 0);
        assert!(environment.sky_color.is_none());

        let mut engine = Engine::with_seed(0);
        let created = apply_test_scenario(&scenario, &mut engine);
        assert_eq!(created.len(), 1);

        let configured = engine.environment();
        assert_eq!(configured.wind, -3);
        assert_eq!(configured.wind_variation, 0);
        assert_eq!(configured.wind_period, 0);
        assert_eq!(configured.temperature, 0);
        assert_eq!(configured.precipitation, 0);
        assert!(configured.sky_color.is_none());
    }

    #[test]
    fn loads_environment_variation_and_temperature() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 4,
                "wind_variation": -6,
                "wind_period": 180,
                "temperature": -15
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let environment = scenario.environment().test_value();
        assert_eq!(environment.wind, 4);
        assert_eq!(environment.wind_variation, 6);
        assert_eq!(environment.wind_period, 180);
        assert_eq!(environment.temperature, -15);
        assert_eq!(environment.precipitation, 0);

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let configured = engine.environment();
        assert_eq!(configured.wind, 4);
        assert_eq!(configured.wind_variation, 6);
        assert_eq!(configured.wind_period, 180);
        assert_eq!(configured.temperature, -15);
        assert_eq!(configured.time_of_day, 0);
        assert_eq!(configured.time_speed, 0);
        assert_eq!(configured.precipitation, 0);
    }

    #[test]
    fn loads_environment_climate_and_temperature_cycle() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 1,
                "climate": 8,
                "temperature": -4,
                "temperature_variation": 6,
                "temperature_period": 120,
                "temperature_phase": 30
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let environment = scenario.environment().test_value();
        assert_eq!(environment.climate, 8);
        assert_eq!(environment.temperature, -4);
        assert_eq!(environment.temperature_variation, 6);
        assert_eq!(environment.temperature_period, 120);
        assert_eq!(environment.temperature_phase, 30);
        assert_eq!(environment.precipitation, 0);

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let configured = engine.environment();
        assert_eq!(configured.climate, 8);
        assert_eq!(configured.temperature, -4);
        assert_eq!(configured.temperature_variation, 6);
        assert_eq!(configured.temperature_period, 120);
        assert_eq!(configured.temperature_phase, 30);
        assert_eq!(configured.precipitation, 0);
    }

    #[test]
    fn loads_environment_time_settings() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 1,
                "time_of_day": -45,
                "time_speed": 400
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let environment = scenario.environment().test_value();
        assert_eq!(environment.wind, 1);
        assert_eq!(environment.time_of_day, 2355);
        assert_eq!(environment.time_speed, 120);
        assert_eq!(environment.precipitation, 0);

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let configured = engine.environment();
        assert_eq!(configured.wind, 1);
        assert_eq!(configured.time_of_day, 2355);
        assert_eq!(configured.time_speed, 120);
        assert_eq!(configured.precipitation, 0);
    }

    #[test]
    fn loads_environment_precipitation_with_clamping() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 2,
                "precipitation": 140
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let environment = scenario.environment().test_value();
        assert_eq!(environment.wind, 2);
        assert_eq!(environment.precipitation, 100);

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let configured = engine.environment();
        assert_eq!(configured.wind, 2);
        assert_eq!(configured.precipitation, 100);
    }

    #[test]
    fn loads_environment_sky_color_from_array() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 1,
                "sky_color": [18, 42, 200]
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let environment = scenario.environment().test_value();
        assert_eq!(environment.sky_color, Some(RgbColor::new(18, 42, 200)));

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let configured = engine.environment();
        assert_eq!(configured.sky_color, Some(RgbColor::new(18, 42, 200)));
    }

    #[test]
    fn loads_environment_sky_color_from_hex() {
        let dir = test_tempdir();
        let manifest = r##"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "environment": {
                "wind": 0,
                "sky_color": "#7F9AC3"
            },
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "##;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let environment = scenario.environment().test_value();
        assert_eq!(environment.sky_color, Some(RgbColor::new(0x7F, 0x9A, 0xC3)));

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let configured = engine.environment();
        assert_eq!(configured.sky_color, Some(RgbColor::new(0x7F, 0x9A, 0xC3)));
    }

    #[test]
    fn scenario_without_environment_resets_engine_to_default() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Mover", "script": "scripts/mover.aul" }
            ],
            "initial_objects": [
                { "definition": "Mover" }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/mover.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        assert!(scenario.environment().is_none());

        let mut engine = Engine::with_seed(0);
        engine.set_environment(EnvironmentSettings::new(5));
        assert!(engine.gamma.set_ramp(0, [0, 0x646464, 0xc8c8c8]));
        apply_test_scenario(&scenario, &mut engine);

        let configured = engine.environment();
        assert_eq!(configured, EnvironmentSettings::default());
        assert!(engine.gamma_controls().is_default());
    }

    #[test]
    fn scenario_tracks_crew_member_flags() {
        let dir = test_tempdir();
        let manifest = r#"
        {
            "definitions": [
                { "id": "Crew", "script": "scripts/crew.aul", "crew_member": true }
            ],
            "initial_objects": [
                { "definition": "Crew", "owner": 1 },
                { "definition": "Crew", "owner": 2, "crew_member": false }
            ]
        }
        "#;

        std::fs::create_dir_all(dir.path().join("scripts")).test_value();
        write_test_file(dir.path().join("Scenario.json"), manifest);
        write_test_file(dir.path().join("scripts/crew.aul"), TEST_SCRIPT);

        let scenario = Scenario::load_from_path(dir.path()).test_value();
        let mut engine = Engine::with_seed(0);
        let created = apply_test_scenario(&scenario, &mut engine);

        assert_eq!(created.len(), 2);
        let first = engine
            .object_snapshot(created[0]).test_value();
        assert!(first.crew_member);
        assert_eq!(first.owner, 1);

        let second = engine
            .object_snapshot(created[1]).test_value();
        assert!(!second.crew_member);
        assert_eq!(second.owner, 2);
    }

    #[test]
    fn scenario_script_initialize_spawns_objects() {
        let scenario_script = r#"
#strict 3
global func Initialize(state, random)
{
    return { spawn = [ { definition = "Mover", owner = 42, energy = 77 } ] };
}

global func Step(state, frame, random)
{
    return nil;
}
"#;

        let scenario = Scenario {
            legacy_core: None,
            legacy_team_metadata: None,
            name: Some("Script Test".into()),
            description: None,
            ticks: None,
            ground_height_hint: Some(220),
            material_library: None,
            definitions: vec![ScenarioDefinition {
                id: "Mover".into(),
                name: Some("Mover".into()),
                description: None,
                clonk_names: None,
                script: TEST_SCRIPT.to_string(),
                script_name: None,
                actions: None,
                crew_member: false,
                can_be_base: false,
                movement: MovementProfile::default(),
                movement_manifest: false,
                category: crate::DEFAULT_CATEGORY,
                value: 0,
                mass: 0,
                picture: None,
                picture_image: None,
                picture_color_by_owner_mask: None,
                graphics_image: None,
                color_by_owner_mask: None,
                additional_graphics: HashMap::new(),
                portrait_image: None,
                portrait_graphics_image: None,
                portrait_color_by_owner_mask: None,
                portrait_graphics: Vec::new(),
                rank_symbols_image: None,
                rank_names: None,
                rank_base: None,
                rank_symbol_count: None,
                resource_group: None,
                components: Vec::new(),
                line_connect: 0,
                vertices: Vec::new(),
                shape: None,
                core: None,
            }],
            value_overloads: Vec::new(),
            initial_spawns: vec![ScenarioSpawn {
                handle: None,
                container_handle: None,
                contents_handles: Vec::new(),
                info_name: None,
                config: SpawnConfig::new("Mover"),
            }],
            landscape: None,
            post_init_map_callbacks: crate::map_creator_s2::PostInitMapCallbacks::default(),
            keep_map_creator: false,
            scenario_sections: Vec::new(),
            physics: None,
            legacy_string_table: clonk_script::new_string_registrations(),
            round_results: RoundResultsState::default(),
            gravity: LegacyC4SVal::new(100, 0, 10, 200),
            environment: None,
            weather_init: None,
            sky: None,
            script: Some(ScenarioScriptSource {
                name: "Script.c".into(),
                source: scenario_script.to_string(),
                c4_args: false,
            }),
            objectives: ScenarioObjectives::default(),
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            base_auto_sell_enabled: true,
            base_reject_entrance_enabled: false,
            base_regenerate_energy_enabled: false,
            base_extinguish_enabled: false,
            base_regenerate_energy_price: 12,
            landscape_insert_thrust: false,
            disable_mouse: false,
            forced_auto_context_menu: None,
            forced_control_style: None,
            definition_load_steps: vec![DefinitionLoadStep::Definition("Mover".into())],
            definition_resource_paths: Vec::new(),
            definition_root_groups: Vec::new(),
            sound_effect_groups: Vec::new(),
            scenario_system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
            teams: Vec::new(),
            lobby_metadata: None,
            standard_names: None,
            map_zoom: LegacyC4SVal::new(10, 0, 5, 15),
            init_placement: None,
        };

        let mut engine = Engine::with_seed(11);
        let created = apply_test_scenario(&scenario, &mut engine);
        assert_eq!(created.len(), 2);
        assert!(!engine.base_reject_entrance_enabled);
        assert!(!engine.base_regenerate_energy_enabled);
        assert!(!engine.base_extinguish_enabled);
        assert_eq!(engine.base_regenerate_energy_price, 12);

        let mut energies: Vec<i32> = created
            .iter()
            .map(|id| engine.object_snapshot(*id).test_value().energy)
            .collect();
        energies.sort_unstable();
        assert_eq!(energies, vec![0, 77]);

        let owners: Vec<i32> = created
            .iter()
            .map(|id| engine.object_snapshot(*id).test_value().owner)
            .collect();
        assert!(owners.contains(&42));
    }

    #[test]
    fn scenario_script_step_runs_each_tick() {
        let scenario_script = r#"
#strict 3
global func Initialize(state, random)
{
    return nil;
}

global func Step(state, frame, random)
{
    if (frame == 1)
    {
        return { spawn = [ { definition = "Mover", owner = 99 } ] };
    }
    return nil;
}
"#;

        let scenario = Scenario {
            legacy_core: None,
            legacy_team_metadata: None,
            name: Some("Step Test".into()),
            description: None,
            ticks: None,
            ground_height_hint: Some(220),
            material_library: None,
            definitions: vec![ScenarioDefinition {
                id: "Mover".into(),
                name: Some("Mover".into()),
                description: None,
                clonk_names: None,
                script: TEST_SCRIPT.to_string(),
                script_name: None,
                actions: None,
                crew_member: false,
                can_be_base: false,
                movement: MovementProfile::default(),
                movement_manifest: false,
                category: crate::DEFAULT_CATEGORY,
                value: 0,
                mass: 0,
                picture: None,
                picture_image: None,
                picture_color_by_owner_mask: None,
                graphics_image: None,
                color_by_owner_mask: None,
                additional_graphics: HashMap::new(),
                portrait_image: None,
                portrait_graphics_image: None,
                portrait_color_by_owner_mask: None,
                portrait_graphics: Vec::new(),
                rank_symbols_image: None,
                rank_names: None,
                rank_base: None,
                rank_symbol_count: None,
                resource_group: None,
                components: Vec::new(),
                line_connect: 0,
                vertices: Vec::new(),
                shape: None,
                core: None,
            }],
            value_overloads: Vec::new(),
            initial_spawns: vec![ScenarioSpawn {
                handle: None,
                container_handle: None,
                contents_handles: Vec::new(),
                info_name: None,
                config: SpawnConfig::new("Mover").with_owner(1),
            }],
            landscape: None,
            post_init_map_callbacks: crate::map_creator_s2::PostInitMapCallbacks::default(),
            keep_map_creator: false,
            scenario_sections: Vec::new(),
            physics: None,
            legacy_string_table: clonk_script::new_string_registrations(),
            round_results: RoundResultsState::default(),
            gravity: LegacyC4SVal::new(100, 0, 10, 200),
            environment: None,
            weather_init: None,
            sky: None,
            script: Some(ScenarioScriptSource {
                name: "Script.c".into(),
                source: scenario_script.to_string(),
                c4_args: false,
            }),
            objectives: ScenarioObjectives::default(),
            construction_needs_material: false,
            structures_need_energy: false,
            base_buy_enabled: true,
            base_sell_enabled: true,
            base_auto_sell_enabled: true,
            base_reject_entrance_enabled: true,
            base_regenerate_energy_enabled: true,
            base_extinguish_enabled: true,
            base_regenerate_energy_price: BASE_REGENERATE_ENERGY_PRICE,
            landscape_insert_thrust: false,
            disable_mouse: false,
            forced_auto_context_menu: None,
            forced_control_style: None,
            definition_load_steps: vec![DefinitionLoadStep::Definition("Mover".into())],
            definition_resource_paths: Vec::new(),
            definition_root_groups: Vec::new(),
            sound_effect_groups: Vec::new(),
            scenario_system_scripts: Vec::new(),
            player_starts: PlayerStart::slots_from_legacy(&[]),
            teams: Vec::new(),
            lobby_metadata: None,
            standard_names: None,
            map_zoom: LegacyC4SVal::new(10, 0, 5, 15),
            init_placement: None,
        };

        let mut engine = Engine::with_seed(7);
        apply_test_scenario(&scenario, &mut engine);

        let initial_snapshot = engine.snapshot();
        assert_eq!(initial_snapshot.objects.len(), 1);

        let snapshot = engine.tick().test_value();
        assert_eq!(snapshot.objects.len(), 2);
        assert!(snapshot.objects.iter().any(|object| object.owner == 99));
    }

    struct FileSystemResolver {
        roots: Vec<PathBuf>,
    }

    #[derive(Debug, Clone, Default)]
    struct DefinitionWarning {
        message: Option<String>,
        group: Option<String>,
        error: Option<String>,
        bit_name: Option<String>,
    }

    #[derive(Clone)]
    struct DefinitionWarningLayer {
        warnings: Arc<Mutex<Vec<DefinitionWarning>>>,
    }

    impl<S> Layer<S> for DefinitionWarningLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            if *event.metadata().level() != Level::WARN {
                return;
            }
            let mut warning = DefinitionWarning::default();
            event.record(&mut warning);
            self.warnings.lock().test_value().push(warning);
        }
    }

    impl Visit for DefinitionWarning {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.record_value(field, format!("{value:?}").trim_matches('"'));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.record_value(field, value);
        }
    }

    impl DefinitionWarning {
        fn record_value(&mut self, field: &Field, value: &str) {
            match field.name() {
                "message" => self.message = Some(value.to_string()),
                "group" => self.group = Some(value.to_string()),
                "error" => self.error = Some(value.to_string()),
                "bit_name" => self.bit_name = Some(value.to_string()),
                _ => {}
            }
        }
    }

    fn capture_definition_warnings<T>(run: impl FnOnce() -> T) -> (T, Vec<DefinitionWarning>) {
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(DefinitionWarningLayer {
            warnings: Arc::clone(&warnings),
        });
        let result = subscriber::with_default(subscriber, run);
        let captured = warnings.lock().test_value().clone();
        (result, captured)
    }

    #[test]
    fn definition_warning_remains_before_its_subtree_progress() {
        // C4DefList::Load emits a definition diagnostic at the point that
        // child is visited, before the caller advances past the subtree
        // (src/C4Def.cpp:930-958). Parallel replay must preserve that
        // diagnostic/progress interleaving, not merely each stream's order.
        let dir = test_tempdir();
        let root = dir.path().join("Defs.c4d");
        let child = root.join("Broken.c4d");
        std::fs::create_dir_all(&child).test_value();
        write_test_file(
            child.join("DefCore.txt"),
            "[DefCore]\nid=TOOLONG\nName=Broken\nCategory=0\n",
        );
        let group = Group::open_indexed(&root).test_value();
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(DefinitionWarningLayer {
            warnings: Arc::clone(&warnings),
        });
        let mut warning_visible_at_first_progress = None;
        let mut sound_effect_groups = Vec::new();
        let mut collected = Vec::new();

        subscriber::with_default(subscriber, || {
            collect_definitions_from_group_with_progress(
                &group,
                false,
                &HashSet::new(),
                &["US"],
                &LanguagePacks::default(),
                &group,
                None,
                &mut sound_effect_groups,
                &mut collected,
                0,
                16,
                "complete",
                &mut |_, _| {
                    warning_visible_at_first_progress
                        .get_or_insert_with(|| !warnings.lock().test_value().is_empty());
                },
            )
        })
        .test_value();

        assert_eq!(warning_visible_at_first_progress, Some(true));
    }

    fn load_legacy_landscape_body_for_test(
        group: &Group,
        manifest: &LegacyScenarioManifest,
        classifier: Option<&mut MapPixelClassifier>,
        random_seed: u64,
        startup_player_count: i32,
    ) -> Result<Option<Landscape>, ScenarioError> {
        let mut callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut creator = None;
        load_legacy_landscape_body(
            group,
            manifest,
            None,
            false,
            classifier,
            random_seed,
            startup_player_count,
            &HashSet::new(),
            &mut callbacks,
            &mut creator,
        )
    }

    impl LegacyDefinitionResolver for FileSystemResolver {
        fn resolve_definition_groups(
            &self,
            scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            let mut groups = Vec::new();
            let normalized = identifier.replace('\\', "/");
            let path = Path::new(&normalized);

            if let Ok(child) = scenario.open_child(path) {
                groups.push(child);
            }

            for root in &self.roots {
                let candidate = root.join(path);
                if !candidate.exists() {
                    continue;
                }
                let group = Group::open(&candidate)?;
                if groups
                    .iter()
                    .all(|existing| existing.root() != group.root())
                {
                    groups.push(group);
                }
            }

            if groups.is_empty() {
                Err(ScenarioError::LegacyDefinitionNotFound {
                    path: identifier.to_string(),
                })
            } else {
                Ok(groups)
            }
        }
    }

    struct LanguagePackResolver {
        filesystem: FileSystemResolver,
        language_packs: LanguagePacks,
    }

    impl LegacyDefinitionResolver for LanguagePackResolver {
        fn resolve_definition_groups(
            &self,
            scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            self.filesystem
                .resolve_definition_groups(scenario, identifier)
        }

        fn resolve_language_packs(
            &self,
            _scenario: &Group,
        ) -> Result<LanguagePacks, ScenarioError> {
            Ok(self.language_packs.clone())
        }
    }

    /// Builds a minimal legacy scenario dir with one good definition and an
    /// optional extra definition + scenario script, for resilience tests.
    fn write_resilience_fixture(
        dir: &std::path::Path,
        extra_def: Option<(&str, &str)>,
        scenario_script: &str,
    ) -> std::path::PathBuf {
        let defs_root = dir.join("Defs.c4d");
        let good = defs_root.join("Good.c4d");
        std::fs::create_dir_all(&good).test_value();
        write_test_file(
            good.join("DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(good.join("Script.c"), "// fine\n");
        write_test_definition_graphics(&good);

        if let Some((id, script)) = extra_def {
            let extra = defs_root.join(format!("{id}.c4d"));
            std::fs::create_dir_all(&extra).test_value();
            write_test_file(
                extra.join("DefCore.txt"),
                format!("[DefCore]\nid={id}\nName={id}\nCategory=0\nCrewMember=0\n"),
            );
            write_test_file(extra.join("Script.c"), script);
            write_test_definition_graphics(&extra);
        }

        let scenario_dir = dir.join("Resilience.c4s");
        std::fs::create_dir_all(&scenario_dir).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Resilience\n\n[Definitions]\nDefinition1=Defs.c4d\n\n[Player1]\nCrew=GOOD=1\nPosition=120,160\n",
        );
        write_test_file(scenario_dir.join("Script.c"), scenario_script);
        scenario_dir
    }

    #[test]
    fn combined_runtime_group_restores_live_round_results_component() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        write_test_file(
            scenario_dir.join("RoundResults.txt"),
            concat!(
                "[RoundResults]\r\n",
                "Goals=GOOD=0\r\n",
                "PlayingTime=73\r\n",
                "CustomEvaluationStrings=\"saved\\ntext\"\r\n\r\n",
                "  [PlayerInfos]\r\n\r\n",
                "    [Player]\r\n",
                "    ID=17\r\n",
                "    SettlementScoreOld=40\r\n",
                "    SettlementScoreNew=52\r\n",
                "    LeagueProgressData=\"progress\\200\"\r\n",
                "    Status=Won\r\n",
                "NetResult=\"server detail\"\r\n",
                "NetResult=LeagueOK\r\n",
            ),
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario = load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);

        let restored = engine.capture_state().round_results;
        assert_eq!(restored.goal_counts, vec![("GOOD".to_owned(), 0)]);
        assert_eq!(restored.goals, vec!["GOOD".to_owned()]);
        assert_eq!(restored.playing_time_seconds, 73);
        assert_eq!(restored.custom_evaluation_strings, "saved\ntext");
        assert_eq!(
            restored.network_result,
            Some(crate::RoundResultsNetworkResult::LeagueOk)
        );
        assert_eq!(restored.network_result_message, b"server detail");
        assert_eq!(restored.players.len(), 1);
        assert_eq!(restored.players[0].player_info_id, 17);
        assert_eq!(restored.players[0].score_new, Some(52));
        assert_eq!(
            restored.players[0].league_progress_data.as_deref(),
            Some(&b"progress\x80"[..])
        );
        assert_eq!(
            restored.players[0].status,
            crate::RoundResultsPlayerStatus::Won
        );
    }

    #[test]
    fn missing_round_results_component_uses_cpp_melee_default() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let source =
            std::fs::read_to_string(scenario_dir.join("Scenario.txt")).test_value();
        write_test_file(
            scenario_dir.join("Scenario.txt"),
            format!("{source}\n[Game]\nGoals=MELE=1\n"),
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert!(engine.capture_state().round_results.hide_settlement_score);
    }

    #[test]
    fn malformed_round_results_component_fails_combined_group_load() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        write_test_file(scenario_dir.join("RoundResults.txt"), b"[Wrong]\r\n");
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        assert!(matches!(
            Scenario::load_from_path_with(&scenario_dir, &resolver),
            Err(ScenarioError::LegacyRoundResultsParse(_))
        ));
    }

    #[test]
    fn legacy_group_loading_reports_monotonic_nonempty_decode_status() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// scenario script\n");
        let group = Group::open(&scenario_dir).test_value();
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let mut reported = Vec::new();

        Scenario::load_from_group_with_languages_and_definition_selection_and_progress(
            &group,
            &resolver,
            &["US"],
            &[] as &[String],
            None,
            None,
            |progress, log| reported.push((progress, log)),
        ).test_value();

        let checkpoints = reported
            .iter()
            .map(|(progress, _)| *progress)
            .collect::<Vec<_>>();
        assert_eq!(
            checkpoints,
            [
                4, 8, 11, 35, 40, 56, 57, 58, 60, 70, 80, 87, 88, 89, 90, 91, 92, 93
            ]
        );
        assert!(
            checkpoints.windows(2).all(|pair| pair[0] <= pair[1]),
            "loader progress must never regress: {checkpoints:?}"
        );
        assert!(reported.iter().all(|(progress, log)| {
            !log.trim().is_empty() || matches!(progress, 11 | 35)
        }));
    }

    #[test]
    fn legacy_group_loading_reports_landscape_work_boundaries() {
        // C4Landscape::Init advances through map creation, sky/pixel-map
        // preparation, and map-to-landscape conversion before C4Game reports
        // the finished landscape (src/C4Landscape.cpp:588-707;
        // src/C4Game.cpp:2654-2661).
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// scenario script\n");
        let group = Group::open(&scenario_dir).test_value();
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let mut reported = Vec::new();

        Scenario::load_from_group_with_languages_and_definition_selection_and_progress(
            &group,
            &resolver,
            &["US"],
            &[] as &[String],
            None,
            None,
            |progress, _| reported.push(progress),
        )
        .test_value();

        assert_eq!(
            reported
                .into_iter()
                .filter(|progress| (60..=88).contains(progress))
                .collect::<Vec<_>>(),
            [60, 70, 80, 87, 88]
        );
    }

    #[test]
    fn static_bitmap_scenario_does_not_build_a_scripted_map_callback_linker() {
        // C4Landscape::Init accepts Map.bmp before it attempts CreateMapS2
        // (src/C4Landscape.cpp:590-606), so a scenario without Landscape.txt
        // or section groups cannot invoke scripted map callbacks.
        let directory = test_tempdir();
        let scenario_path = directory.path().join("StaticMap.c4s");
        std::fs::create_dir(&scenario_path).test_value();
        write_test_file(scenario_path.join("Map.bmp"), b"static map marker");
        let group = Group::open_indexed(&scenario_path).test_value();

        assert!(!scenario_may_need_map_callbacks(&group).test_value());
    }

    #[test]
    fn plain_s2_scenario_does_not_build_a_scripted_map_callback_linker() {
        // Only evalFn/drawFn fields resolve through Game.Script.GetSFunc
        // while parsing an S2 map (C4MapCreatorS2.cpp:367-378, 1615-1616).
        // A plain Landscape.txt therefore needs no callback-name linker.
        let directory = test_tempdir();
        let scenario_path = directory.path().join("PlainS2.c4s");
        std::fs::create_dir(&scenario_path).test_value();
        write_test_file(
            scenario_path.join("Landscape.txt"),
            "map Plain { overlay Earth { algo=solid; }; };\n",
        );
        let group = Group::open_indexed(&scenario_path).test_value();

        assert!(!scenario_may_need_map_callbacks(&group).test_value());
    }

    #[test]
    fn s2_callback_field_keeps_the_scripted_map_callback_linker() {
        // C4MCV_ScriptFunc validates evalFn/drawFn through the linked scenario
        // host while parsing (C4MapCreatorS2.cpp:367-378, 1615-1616).
        let directory = test_tempdir();
        let scenario_path = directory.path().join("CallbackS2.c4s");
        std::fs::create_dir(&scenario_path).test_value();
        write_test_file(
            scenario_path.join("Landscape.txt"),
            "map Callback { overlay Earth { evalFn=Probe; }; };\n",
        );
        let group = Group::open_indexed(&scenario_path).test_value();

        assert!(scenario_may_need_map_callbacks(&group).test_value());
    }

    #[test]
    fn corrupt_static_map_precedes_invalid_section_name_validation() {
        // C4Landscape::Init decodes Map.bmp before InitScenarioSections scans
        // section names (src/C4Game.cpp:2643-2694), so the callback-linker
        // preflight must not validate a later scenario-section candidate.
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// scenario script\n");
        write_test_file(scenario_dir.join("Map.bmp"), b"not a bitmap");
        std::fs::create_dir(scenario_dir.join("Sect.c4g")).test_value();
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let error = Scenario::load_from_path_with(&scenario_dir, &resolver).unwrap_err();

        assert!(matches!(error, ScenarioError::LegacyMapDecode { .. }));
    }

    #[test]
    fn explicit_definition_error_precedes_folder_local_scan_failure() {
        // InitDefs resolves the explicit definition vector before its
        // folder-local pass (src/C4Game.cpp:2580-2586; src/C4Def.cpp:1013-1039),
        // so progress pre-counting must not inspect a broken ancestor first.
        let dir = test_tempdir();
        let folder_path = dir.path().join("Broken.c4f");
        write_test_file(&folder_path, b"not a group");
        let mut scenario = clonk_resources::MutableGroup::new("Scenario.c4s");
        scenario
            .add_file(
                "Scenario.txt",
                b"[Head]\nTitle=Ordering\n\n[Definitions]\nDefinition1=Missing.c4d\n".to_vec(),
            )
            .test_value();
        let group = Group::from_raw_memory(
            folder_path.join("Scenario.c4s"),
            scenario.pack_raw().test_value(),
        )
        .test_value();
        let resolver = test_resolver(Vec::new());

        assert!(
            folder_local_definition_groups(&group)
                .test_value()
                .is_empty(),
            "C4Game::FoldersWithLocalsDefs skips an ancestor C4Group that cannot open"
        );

        let error = Scenario::load_from_group_with_languages_and_definition_selection(
            &group,
            &resolver,
            &["US"],
            &[] as &[String],
            None,
            None,
        )
        .unwrap_err();

        assert!(matches!(error, ScenarioError::LegacyDefinitionNotFound { .. }));
    }

    #[test]
    fn failed_landscape_decode_does_not_report_a_prepared_source_map() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// scenario script\n");
        write_test_file(scenario_dir.join("Map.bmp"), b"not a bitmap");
        let group = Group::open(&scenario_dir).test_value();
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let mut reported = Vec::new();

        let result =
            Scenario::load_from_group_with_languages_and_definition_selection_and_progress(
                &group,
                &resolver,
                &["US"],
                &[] as &[String],
                None,
                None,
                |progress, _| reported.push(progress),
            );

        assert!(result.is_err());
        assert!(
            !reported.contains(&70),
            "a failed source-map decode is not prepared: {reported:?}"
        );
    }

    #[test]
    fn invalid_exact_landscape_material_precedes_progress_70_and_80() {
        // C4Landscape::Load validates every exact Surface8 material byte before
        // it reports 70, and Init reports 80 only after Load succeeds
        // (src/C4Landscape.cpp:1520-1608,658-674).
        let dir = test_tempdir();
        let source = "[Landscape]\nExactLandscape=1\nNewStyleLandscape=2\n";
        let scenario_dir = scenario_test_group(dir.path(), "InvalidExact.c4s", source);
        write_test_file(
            scenario_dir.join("Landscape.bmp"),
            encode_indexed_bmp(&[&[42]]),
        );
        let group = Group::open(&scenario_dir).test_value();
        let manifest = parsed_scenario(source);
        let mut classifier = map_classifier(&[(5, "Earth", 100, ChunkShape::Flat)]);
        let mut callbacks = crate::map_creator_s2::PostInitMapCallbacks::default();
        let mut creator = None;
        let mut reported = Vec::new();

        let result = load_legacy_landscape_body_with_progress(
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
            &mut |progress, _| reported.push(progress),
        );

        assert!(matches!(result, Err(ScenarioError::InvalidLandscape(_))));
        assert!(
            reported.iter().all(|progress| !matches!(progress, 70 | 80)),
            "invalid exact material reported prepared phases: {reported:?}"
        );
    }

    #[test]
    fn network_group_loading_reports_the_shared_initgame_milestones() {
        // Network clients and hosts enter the same InitGame first/second-part
        // milestones after their respective 7 checkpoint
        // (src/C4Game.cpp:456-457,2551-2721).
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// scenario script\n");
        let group = Group::open(&scenario_dir).test_value();
        let definitions =
            Group::open(dir.path().join("Defs.c4d")).test_value();
        let mut reported = Vec::new();

        Scenario::load_network_from_group_with_languages_and_seed_and_packs_and_progress(
            &group,
            &[definitions],
            &[],
            &[],
            &["US"],
            0,
            &LanguagePacks::default(),
            |progress, log| reported.push((progress, log)),
        ).test_value();

        assert_eq!(
            reported
                .iter()
                .map(|(progress, _)| *progress)
                .collect::<Vec<_>>(),
            [
                4, 8, 11, 35, 40, 56, 57, 58, 60, 70, 80, 87, 88, 89, 90, 91, 92, 93
            ]
        );
        assert!(reported.iter().all(|(progress, log)| {
            !log.trim().is_empty() || matches!(progress, 11 | 35)
        }));
    }

    #[test]
    fn missing_defcore_id_skips_only_parent_and_still_loads_its_child() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let broken = dir.path().join("Defs.c4d/Broken.c4d");
        let child = broken.join("Child.c4d");
        std::fs::create_dir_all(&child).test_value();
        write_test_file(
            broken.join("DefCore.txt"),
            "[DefCore]\nName=Broken\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(
            child.join("DefCore.txt"),
            "[DefCore]\nid=CHLD\nName=Child\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(child.join("Script.c"), "// child\n");
        write_test_definition_graphics(&child);

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let (loaded, warnings) =
            capture_definition_warnings(|| Scenario::load_from_path_with(&scenario_dir, &resolver));
        let scenario = loaded.test_value();
        let mut ids = scenario
            .definitions
            .iter()
            .map(|definition| definition.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, ["CHLD", "GOOD"]);

        assert!(warnings.iter().any(|warning| {
            warning.message.as_deref() == Some("definition failed to load; skipping")
                && warning.group.as_deref() == Some(broken.to_string_lossy().as_ref())
                && warning
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("id"))
        }));
    }

    #[test]
    fn zero_size_defcore_is_skipped_and_zero_size_actmap_uses_defaults() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");

        let empty_core = dir.path().join("Defs.c4d/EmptyCore.c4d");
        let child = empty_core.join("Child.c4d");
        std::fs::create_dir_all(&child).test_value();
        write_test_file(empty_core.join("DefCore.txt"), []);
        write_test_file(
            child.join("DefCore.txt"),
            "[DefCore]\nid=CHLD\nName=Child\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(child.join("Script.c"), "// child\n");
        write_test_definition_graphics(&child);

        let empty_act = dir.path().join("Defs.c4d/EmptyAct.c4d");
        std::fs::create_dir_all(&empty_act).test_value();
        write_test_file(
            empty_act.join("DefCore.txt"),
            "[DefCore]\nid=EACT\nName=Empty ActMap\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(empty_act.join("Script.c"), "// empty ActMap\n");
        write_test_file(empty_act.join("ActMap.txt"), []);
        write_test_definition_graphics(&empty_act);

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let (loaded, warnings) =
            capture_definition_warnings(|| Scenario::load_from_path_with(&scenario_dir, &resolver));
        let scenario = loaded.test_value();
        let mut ids = scenario
            .definitions
            .iter()
            .map(|definition| definition.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, ["CHLD", "EACT", "GOOD"]);
        assert!(scenario
            .definitions
            .iter()
            .find(|definition| definition.id == "EACT")
            .is_some_and(|definition| definition.actions.is_none()));
        assert!(warnings.iter().all(|warning| {
            !matches!(
                warning.group.as_deref(),
                Some(group)
                    if group == empty_core.to_string_lossy()
                        || group == empty_act.to_string_lossy()
            )
        }));

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
    }

    #[test]
    fn malformed_actmap_skips_only_parent_warns_and_still_loads_its_child() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let broken = dir.path().join("Defs.c4d/BadAct.c4d");
        let tail = broken.join("Tail.c4d");
        std::fs::create_dir_all(&tail).test_value();
        write_test_file(
            broken.join("DefCore.txt"),
            "[DefCore]\nid=BACT\nName=Bad ActMap\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(broken.join("Script.c"), "// parent\n");
        write_test_file(broken.join("ActMap.txt"), "malformed action map\n");
        write_test_definition_graphics(&broken);
        write_test_file(
            tail.join("DefCore.txt"),
            "[DefCore]\nid=TAIL\nName=Tail\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(tail.join("Script.c"), "// tail\n");
        write_test_definition_graphics(&tail);

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let (loaded, warnings) =
            capture_definition_warnings(|| Scenario::load_from_path_with(&scenario_dir, &resolver));
        let scenario = loaded.test_value();
        let mut ids = scenario
            .definitions
            .iter()
            .map(|definition| definition.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, ["GOOD", "TAIL"]);

        assert!(warnings.iter().any(|warning| {
            warning.message.as_deref() == Some("definition failed to load; skipping")
                && warning.group.as_deref() == Some(broken.to_string_lossy().as_ref())
                && warning
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("ActMap"))
        }));
    }

    #[test]
    fn mismatched_owner_overlay_skips_only_parent_and_still_loads_its_child() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let broken = dir.path().join("Defs.c4d/BadGfx.c4d");
        let child = broken.join("Child.c4d");
        std::fs::create_dir_all(&child).test_value();
        write_test_file(
            broken.join("DefCore.txt"),
            "[DefCore]\nid=BGFX\nName=Bad Graphics\nColorByOwner=1\n",
        );
        write_test_file(broken.join("Script.c"), "// parent\n");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]))
            .save(broken.join("Graphics.png")).test_value();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([32, 32, 32, 255]))
            .save(broken.join("Overlay.png")).test_value();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([136, 0, 0, 255]))
            .save(broken.join("GraphicsBad.png")).test_value();
        image::RgbaImage::from_pixel(2, 1, image::Rgba([64, 64, 64, 255]))
            .save(broken.join("OverlayBad.png")).test_value();
        write_test_file(
            child.join("DefCore.txt"),
            "[DefCore]\nid=CHLD\nName=Child\nCategory=0\nCrewMember=0\n",
        );
        write_test_file(child.join("Script.c"), "// child\n");
        write_test_definition_graphics(&child);

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let (loaded, warnings) =
            capture_definition_warnings(|| Scenario::load_from_path_with(&scenario_dir, &resolver));
        let scenario = loaded.test_value();
        let mut ids = scenario
            .definitions
            .iter()
            .map(|definition| definition.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, ["CHLD", "GOOD"]);

        assert!(warnings.iter().any(|warning| {
            warning.message.as_deref() == Some("definition failed to load; skipping")
                && warning.group.as_deref() == Some(broken.to_string_lossy().as_ref())
                && warning
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("OverlayBad.png"))
        }));
    }

    #[test]
    fn definition_load_skip_ladder_matches_cpp() {
        fn write_core(root: &Path, name: &str, source: &str) -> PathBuf {
            let path = root.join(format!("{name}.c4d"));
            std::fs::create_dir_all(&path).test_value();
            write_test_file(path.join("DefCore.txt"), source);
            path
        }

        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let defs = dir.path().join("Defs.c4d");

        let truncated = write_core(&defs, "Truncated", "[DefCore]\nid=TOWER\n");
        write_test_definition_graphics(&truncated);

        let lowercase = write_core(&defs, "Lowercase", "[DefCore]\nid=lowr\n");
        write_test_definition_graphics(&lowercase);

        let old_gfx = write_core(&defs, "OldGfx", "[DefCore]\nid=OLDG\nNeededGfxMode=2\n");
        write_test_definition_graphics(&old_gfx);

        let modern_gfx = write_core(&defs, "ModernGfx", "[DefCore]\nid=MODN\nNeededGfxMode=3\n");
        write_test_definition_graphics(&modern_gfx);

        let missing = write_core(&defs, "Missing", "[DefCore]\nid=MISS\n");

        let variant_only = write_core(&defs, "VariantOnly", "[DefCore]\nid=VARI\n");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([4, 5, 6, 255]))
            .save(variant_only.join("GraphicsAlt.png")).test_value();

        let corrupt_png = write_core(&defs, "CorruptPng", "[DefCore]\nid=CORR\n");
        write_test_file(corrupt_png.join("Graphics.png"), b"not a png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([7, 8, 9, 255]))
            .save(corrupt_png.join("Graphics.bmp")).test_value();

        let mislabeled_png = write_core(&defs, "MislabeledPng", "[DefCore]\nid=MSLB\n");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([13, 14, 15, 255]))
            .save_with_format(mislabeled_png.join("Graphics.png"), image::ImageFormat::Bmp).test_value();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([16, 17, 18, 255]))
            .save(mislabeled_png.join("Graphics.bmp")).test_value();

        let corrupt_additional = write_core(&defs, "CorruptAdditional", "[DefCore]\nid=CADD\n");
        write_test_definition_graphics(&corrupt_additional);
        write_test_file(corrupt_additional.join("GraphicsBad.png"), b"not a png");

        let dual_base = write_core(&defs, "DualBase", "[DefCore]\nid=DUAL\nColorByOwner=1\n");
        write_test_definition_graphics(&dual_base);
        image::RgbaImage::from_pixel(1, 1, image::Rgba([32, 32, 32, 255]))
            .save(dual_base.join("Overlay.png")).test_value();
        image::RgbaImage::from_pixel(2, 1, image::Rgba([19, 20, 21, 255]))
            .save(dual_base.join("Graphics.bmp")).test_value();

        let bad_overlay = write_core(&defs, "BadOverlay", "[DefCore]\nid=OVLY\nColorByOwner=1\n");
        write_test_definition_graphics(&bad_overlay);
        image::RgbaImage::from_pixel(2, 1, image::Rgba([32, 32, 32, 255]))
            .save(bad_overlay.join("Overlay.png")).test_value();

        let particle = write_core(&defs, "Particle", "[DefCore]\nid=PART\n");
        write_test_definition_graphics(&particle);
        write_test_file(
            particle.join("Particle.txt"),
            b"[Particle]\nName=ScenarioParticle\nInitFn=StdInit\nExecFn=StdExec\nDrawFn=Std\nFace=0,0,1,1,0,0\n",
        );
        let particle_child = write_core(&particle, "Child", "[DefCore]\nid=CHLD\n");
        write_test_definition_graphics(&particle_child);

        let bitmap = write_core(&defs, "Bitmap", "[DefCore]\nid=BMAP\n");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 11, 12, 255]))
            .save(bitmap.join("Graphics.bmp")).test_value();

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let (loaded, warnings) =
            capture_definition_warnings(|| Scenario::load_from_path_with(&scenario_dir, &resolver));
        let scenario = loaded.test_value();
        let mut ids = scenario
            .definitions
            .iter()
            .map(|definition| definition.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, ["BMAP", "CHLD", "DUAL", "GOOD", "MODN", "TOWE"]);

        assert!(warnings.iter().any(|warning| {
            warning.message.as_deref() == Some("skipping definition with invalid C4ID")
                && warning.group.as_deref() == Some(lowercase.to_string_lossy().as_ref())
        }));
        assert!(warnings.iter().any(|warning| {
            warning.message.as_deref() == Some("definition failed to load; skipping")
                && warning.group.as_deref() == Some(bad_overlay.to_string_lossy().as_ref())
                && warning
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("Overlay.png"))
        }));
        assert!(
            warnings.iter().any(|warning| {
                warning.message.as_deref() == Some("definition failed to load; skipping")
                    && warning.group.as_deref() == Some(missing.to_string_lossy().as_ref())
                    && warning
                        .error
                        .as_deref()
                        .is_some_and(|error| error.contains("Graphics.png/Graphics.bmp"))
            }),
            "captured warnings: {warnings:#?}"
        );
        for rejected in [&corrupt_png, &mislabeled_png] {
            assert!(warnings.iter().any(|warning| {
                warning.message.as_deref() == Some("definition failed to load; skipping")
                    && warning.group.as_deref() == Some(rejected.to_string_lossy().as_ref())
                    && warning
                        .error
                        .as_deref()
                        .is_some_and(|error| error.contains("Graphics.png"))
            }));
        }
        assert!(warnings.iter().any(|warning| {
            warning.message.as_deref() == Some("definition failed to load; skipping")
                && warning.group.as_deref() == Some(corrupt_additional.to_string_lossy().as_ref())
                && warning
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("GraphicsBad.png"))
        }));
        assert!(warnings.iter().any(|warning| {
            warning.message.as_deref() == Some("definition failed to load; skipping")
                && warning.group.as_deref() == Some(variant_only.to_string_lossy().as_ref())
                && warning
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("Graphics.png/Graphics.bmp"))
        }));

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let particle = engine
            .particle_system()
            .get_def("ScenarioParticle").test_value();
        assert_eq!(particle.length, 1);
        assert!(particle.graphics.is_some());
    }

    fn write_definition_localization_fixture(
        dir: &std::path::Path,
        script: &str,
        string_table: &str,
    ) -> std::path::PathBuf {
        let scenario_dir = write_resilience_fixture(dir, None, "// no script\n");
        let definition_dir = dir.join("Defs.c4d/Good.c4d");
        write_test_file(definition_dir.join("Script.c"), script);
        write_test_file(definition_dir.join("StringTblUS.txt"), string_table);
        scenario_dir
    }

    fn apply_resilience_fixture(
        dir: &tempfile::TempDir,
        scenario_dir: &std::path::Path,
    ) -> (Engine, Vec<ObjectId>) {
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        let created = apply_test_scenario(&scenario, &mut engine);
        (engine, created)
    }

    #[test]
    fn legacy_scenario_script_replaces_localized_strings_before_compile() {
        // C4Game loads ScenarioLangStringTable before Script.c and
        // C4ScriptHost::MakeScript replaces `$key$` before Preparse
        // (C4Game.cpp:229-230,3336-3341; C4ScriptHost.cpp:66-82).
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nglobal func Initialize() { Message(\"$MsgIntro2a$\"); }\n",
        );
        write_test_file(
            scenario_dir.join("StringTblUS.txt"),
            "MsgIntro2a=Come with me, princess!\n",
        );
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let script = &scenario.script.as_ref().test_value().source;

        assert!(script.contains("\"Come with me, princess!\""));
        assert!(!script.contains("$MsgIntro2a$"));

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let snapshot = engine.tick().test_value();
        assert_eq!(snapshot.hud.messages.len(), 1);
        assert_eq!(snapshot.hud.messages[0].lines, ["Come with me, princess!"]);
        assert!(snapshot.audio.iter().all(|command| !matches!(
            command,
            crate::AudioCommand::PlaySound { name, .. } if name == "MsgIntro2a"
        )));
    }

    #[test]
    fn legacy_definition_script_localizes_runtime_strings_without_parse_regressions() {
        // C4Def::Load gives each definition script its local StringTbl and
        // C4ScriptHost::MakeScript replaces `$key$` before Preparse
        // (C4Def.cpp:625-633; C4ScriptHost.cpp:46-82). Localized context text
        // such as `Put/Get` must not make an otherwise valid definition fail.
        let dir = test_tempdir();
        let scenario_dir = write_definition_localization_fixture(
            dir.path(),
            "#strict\n\
             func Label() { return \"$Reloaded$\"; }\n\
             func ContextAction() {\n\
               [$ActionLabel$|Image=GOOD]\n\
               return 1;\n\
             }\n",
            "Reloaded=Reloaded %dx {{%i}}.\nActionLabel=Put/Get\n",
        );
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let id = engine.spawn_test_object(SpawnConfig::new("GOOD"));
        let index = engine.test_object_index(id);

        assert_eq!(
            engine
                .call_object_function(index, "Label", Vec::new())
                .expect("Label runs"),
            clonk_script::Value::String("Reloaded %dx {{%i}}.".to_string().into())
        );
    }

    #[test]
    fn definition_string_table_loads_from_language_pack() {
        let dir = test_tempdir();
        let scenario_dir = write_definition_localization_fixture(
            dir.path(),
            "#strict\nfunc PackLabel() { return \"$PackLabel$\"; }\n",
            "",
        );
        std::fs::remove_file(dir.path().join("Defs.c4d/Good.c4d/StringTblUS.txt")).test_value();

        let language_container = dir.path().join("Language.c4g");
        let pack_definition = language_container.join("Finnish.c4g/Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&pack_definition).test_value();
        write_test_file(
            pack_definition.join("StringTblUS.txt"),
            "PackLabel=Packed value\n",
        );
        let resolver = LanguagePackResolver {
            filesystem: test_resolver(vec![dir.path().to_path_buf()]),
            language_packs: LanguagePacks::discover(
                std::slice::from_ref(&language_container),
                &[dir.path().to_path_buf()],
            ),
        };

        let scenario = load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let id = engine.spawn_test_object(SpawnConfig::new("GOOD"));
        let index = engine.test_object_index(id);

        assert_eq!(
            engine
                .call_object_function(index, "PackLabel", Vec::new())
                .expect("PackLabel runs"),
            clonk_script::Value::String("Packed value".to_string().into())
        );
    }

    #[test]
    fn scenario_script_string_table_loads_from_language_pack() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static scenario_pack_value;\n\
             global func Initialize() { scenario_pack_value = \"$ScenarioPack$\"; }\n",
        );
        let language_container = dir.path().join("Language.c4g");
        let pack_scenario = language_container.join("Finnish.c4g/Resilience.c4s");
        std::fs::create_dir_all(&pack_scenario).test_value();
        write_test_file(
            pack_scenario.join("StringTblUS.txt"),
            "ScenarioPack=localized scenario\n",
        );
        let resolver = LanguagePackResolver {
            filesystem: test_resolver(vec![dir.path().to_path_buf()]),
            language_packs: LanguagePacks::discover(
                std::slice::from_ref(&language_container),
                &[dir.path().to_path_buf()],
            ),
        };

        let scenario = load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("scenario_pack_value")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::String(
                "localized scenario".to_string().into()
            ))
        );
    }

    #[test]
    fn definition_clonk_names_cross_load_only_after_local_marker() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no script\n");
        let definition_dir = dir.path().join("Defs.c4d/Good.c4d");
        let language_container = dir.path().join("Language.c4g");
        let pack_definition = language_container.join("Finnish.c4g/Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&pack_definition).test_value();
        write_test_file(
            pack_definition.join("ClonkNamesUS.txt"),
            "Pack One\nPack Two\n",
        );
        write_test_file(
            pack_definition.join("ClonkNamesDE.txt"),
            b"Pack J\xfcrgen\n",
        );
        let packs = LanguagePacks::discover(
            std::slice::from_ref(&language_container),
            &[dir.path().to_path_buf()],
        );
        let load = |languages: &[&str]| {
            Scenario::load_from_path_with_languages(
                &scenario_dir,
                &LanguagePackResolver {
                    filesystem: test_resolver(vec![dir.path().to_path_buf()]),
                    language_packs: packs.clone(),
                },
                languages,
            ).test_value()
        };

        let mut without_marker = Engine::with_seed(0);
        load(&["DE", "US"])
            .apply(&mut without_marker).test_value();
        assert_eq!(
            without_marker
                .definitions
                .get(&DefinitionId::from("GOOD"))
                .and_then(|definition| definition.clonk_names()),
            None,
            "C4Def's local ClonkNames*.txt marker gate rejects a pack-only list"
        );

        write_test_file(
            definition_dir.join("ClonkNamesFI.txt"),
            "local marker for an unselected language\n",
        );
        let mut us_first = Engine::with_seed(0);
        load(&["US", "DE"])
            .apply(&mut us_first).test_value();
        assert_eq!(
            us_first
                .definitions
                .get(&DefinitionId::from("GOOD"))
                .and_then(|definition| definition.clonk_names()),
            Some("Pack One\nPack Two\n")
        );

        let mut de_first = Engine::with_seed(0);
        load(&["DE", "US"])
            .apply(&mut de_first).test_value();
        assert_eq!(
            de_first
                .definitions
                .get(&DefinitionId::from("GOOD"))
                .and_then(|definition| definition.clonk_names()),
            Some("Pack Jürgen\n")
        );
    }

    #[test]
    fn system_script_enumeration_preserves_non_utf8_group_entry_name_bytes() {
        const NATIVE_SCRIPT_NAME: &[u8] = b"Gr\xfcn.C";

        let mut packed = clonk_resources::MutableGroup::new("native-order.bin");
        for (name, source) in [
            (b"Zulu.c".as_slice(), b"// zulu\n".as_slice()),
            (b"Ignore.txt".as_slice(), b"not a script\n".as_slice()),
            (NATIVE_SCRIPT_NAME, b"// native\n".as_slice()),
        ] {
            packed
                .add_file_bytes_with_metadata(name.to_vec(), source.to_vec(), 1, false).test_value();
        }
        packed
            .add_packed_child_bytes_with_metadata(
                b"Child.c".to_vec(),
                b"// child-marked\n".to_vec(),
                0,
                1,
                false,
            ).test_value();
        packed
            .add_file_bytes_with_metadata(b"Alpha.c".to_vec(), b"// alpha\n".to_vec(), 1, false).test_value();
        let group = Group::from_raw_memory(
            PathBuf::from("System.c4g"),
            packed.pack_raw().expect("pack native-order System group"),
        ).test_value();
        let entries = group.entries().test_value();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name_bytes.as_slice())
                .collect::<Vec<_>>(),
            [
                b"Zulu.c".as_slice(),
                b"Ignore.txt".as_slice(),
                NATIVE_SCRIPT_NAME,
                b"Child.c".as_slice(),
                b"Alpha.c".as_slice(),
            ]
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;

            assert_eq!(
                entries[2].relative_path.as_os_str().as_bytes(),
                NATIVE_SCRIPT_NAME,
                "packed names retain their native bytes in Unix path labels"
            );
        }

        let scripts = load_system_scripts(&group).test_value();
        assert_eq!(
            scripts
                .iter()
                .map(|(name, _)| clonk_script::c4_string_bytes(name))
                .collect::<Vec<_>>(),
            [
                b"Zulu.c".to_vec(),
                NATIVE_SCRIPT_NAME.to_vec(),
                b"Child.c".to_vec(),
                b"Alpha.c".to_vec(),
            ]
        );
        assert_eq!(
            scripts
                .iter()
                .map(|(_, source)| source.as_str())
                .collect::<Vec<_>>(),
            [
                "// zulu\n",
                "// native\n",
                "// child-marked\n",
                "// alpha\n",
            ]
        );
    }

    #[test]
    fn system_script_enumeration_keeps_failed_host_and_continues() {
        let directory = test_tempdir();
        std::fs::create_dir(directory.path().join("Bad.c")).test_value();
        write_test_file(directory.path().join("Good.c"), "// good\n");
        let group = Group::open(directory.path()).test_value();

        let scripts = load_system_scripts(&group).test_value();
        assert_eq!(
            scripts
                .iter()
                .find(|(name, _)| name == "Bad.c")
                .map(|(_, source)| source.as_str()),
            Some("")
        );
        assert_eq!(
            scripts
                .iter()
                .find(|(name, _)| name == "Good.c")
                .map(|(_, source)| source.as_str()),
            Some("// good\n")
        );
    }

    #[test]
    fn system_script_string_table_keeps_candidate_major_pack_priority() {
        let dir = test_tempdir();
        let install = dir.path().join("install");
        let system_path = install.join("System.c4g");
        std::fs::create_dir_all(&system_path).test_value();
        write_test_file(
            system_path.join("Probe.c"),
            "global func SystemPackLabel() { return \"$Label$\"; }\n",
        );
        write_test_file(system_path.join("StringTblUS.txt"), "Label=Local later\n");

        let language_container = install.join("Language.c4g");
        let pack_system = language_container.join("Finnish.c4g/System.c4g");
        std::fs::create_dir_all(&pack_system).test_value();
        write_test_file(pack_system.join("StringTbl.txt"), "Label=Pack first\n");

        let system = Group::open(&system_path).test_value();
        let packs = LanguagePacks::discover(
            std::slice::from_ref(&language_container),
            std::slice::from_ref(&install),
        );
        let components = packs.component_groups(&system, None, None);
        let scripts = load_system_scripts_with_components(&system, &components, &["US"]).test_value();
        assert_eq!(scripts.len(), 1);
        assert!(scripts[0].1.contains("return \"Pack first\""));
        assert!(!scripts[0].1.contains("Local later"));
        assert!(!scripts[0].1.contains("$Label$"));
    }

    #[test]
    fn scenario_and_definition_system_scripts_cross_load_pack_tables() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static def_system_value, scenario_system_value;\n\
             global func Initialize() {\n\
               def_system_value = DefSystemValue();\n\
               scenario_system_value = ScenarioSystemValue();\n\
             }\n",
        );
        let definition_system = dir.path().join("Defs.c4d/System.c4g");
        std::fs::create_dir_all(&definition_system).test_value();
        write_test_file(
            definition_system.join("DefSystem.c"),
            "global func DefSystemValue() { return \"$DefValue$\"; }\n",
        );
        let scenario_system = scenario_dir.join("System.c4g");
        std::fs::create_dir_all(&scenario_system).test_value();
        write_test_file(
            scenario_system.join("ScenarioSystem.c"),
            "global func ScenarioSystemValue() { return \"$ScenarioValue$\"; }\n",
        );

        let language_container = dir.path().join("Language.c4g");
        let pack_definition_system = language_container.join("Finnish.c4g/Defs.c4d/System.c4g");
        std::fs::create_dir_all(&pack_definition_system).test_value();
        write_test_file(
            pack_definition_system.join("StringTblUS.txt"),
            "DefValue=definition pack\n",
        );
        let pack_scenario_system = language_container.join("Finnish.c4g/Resilience.c4s/System.c4g");
        std::fs::create_dir_all(&pack_scenario_system).test_value();
        write_test_file(
            pack_scenario_system.join("StringTblUS.txt"),
            "ScenarioValue=scenario pack\n",
        );

        let resolver = LanguagePackResolver {
            filesystem: test_resolver(vec![dir.path().to_path_buf()]),
            language_packs: LanguagePacks::discover(
                std::slice::from_ref(&language_container),
                &[dir.path().to_path_buf()],
            ),
        };
        let scenario = load_test_scenario(&scenario_dir, &resolver);
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("def_system_value")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::String(
                "definition pack".to_string().into()
            ))
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("scenario_system_value")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::String(
                "scenario pack".to_string().into()
            ))
        );
    }

    #[test]
    fn network_loader_threads_language_packs_to_authoritative_definitions() {
        let dir = test_tempdir();
        let scenario_dir = write_definition_localization_fixture(
            dir.path(),
            "#strict\nfunc NetworkPackLabel() { return \"$NetworkLabel$\"; }\n",
            "",
        );
        std::fs::remove_file(dir.path().join("Defs.c4d/Good.c4d/StringTblUS.txt")).test_value();
        let language_container = dir.path().join("Language.c4g");
        let pack_definition = language_container.join("Finnish.c4g/Defs.c4d/Good.c4d");
        std::fs::create_dir_all(&pack_definition).test_value();
        write_test_file(
            pack_definition.join("StringTblUS.txt"),
            "NetworkLabel=network pack\n",
        );
        let packs = LanguagePacks::discover(
            std::slice::from_ref(&language_container),
            &[dir.path().to_path_buf()],
        );
        let definitions =
            [Group::open(dir.path().join("Defs.c4d")).test_value()];

        let scenario = Scenario::load_network_from_path_with_languages_and_seed_and_packs(
            &scenario_dir,
            &definitions,
            &[],
            &[],
            &["US"],
            0,
            &packs,
        ).test_value();
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let id = engine.spawn_test_object(SpawnConfig::new("GOOD"));
        let index = engine.test_object_index(id);
        assert_eq!(
            engine
                .call_object_function(index, "NetworkPackLabel", Vec::new())
                .expect("NetworkPackLabel runs"),
            clonk_script::Value::String("network pack".to_string().into())
        );
    }

    #[test]
    fn legacy_scenario_and_definition_scripts_preserve_native_bytes() {
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// replaced below\n");
        write_test_file(
            scenario_dir.join("Script.c"),
            [
                b"#strict\nglobal func Initialize() { Message(\"".as_slice(),
                &[0xe9, 0xff],
                b"\"); }\n",
            ]
            .concat(),
        );

        let definition_dir = dir.path().join("Defs.c4d/Good.c4d");
        write_test_file(
            definition_dir.join("Script.c"),
            b"#strict\nfunc RawLabel() { return \"$RawLabel$\"; }\n",
        );
        write_test_file(
            definition_dir.join("StringTblUS.txt"),
            [b"RawLabel=".as_slice(), &[0xe9, 0xff], b"\n"].concat(),
        );

        let resolver = test_resolver(vec![dir.path().to_path_buf()]);
        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        assert!(scenario
            .script
            .as_ref()
            .is_some_and(|script| clonk_script::c4_string_bytes(&script.source).contains(&0xff)));

        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let snapshot = engine.snapshot();
        assert_eq!(
            clonk_script::c4_string_bytes(&snapshot.hud.messages[0].lines[0]),
            [0xe9, 0xff]
        );

        let object = engine.spawn_test_object(SpawnConfig::new("GOOD"));
        let index = engine.test_object_index(object);
        let value = engine
            .call_object_function(index, "RawLabel", Vec::new()).test_value();
        assert_eq!(
            value,
            clonk_script::Value::String(clonk_script::c4_string_from_bytes(&[0xe9, 0xff]).into())
        );
    }

    #[test]
    fn legacy_definition_script_localizes_unquoted_array_values() {
        // C4Def::Load gives each definition script its local StringTbl and
        // C4ScriptHost::MakeScript replaces `$key$` across the whole source
        // before Preparse (C4Def.cpp:625-633; C4ScriptHost.cpp:46-82).
        // Hazard's HHKS definition relies on an unquoted key expanding to a
        // C4Script array literal (Killstats.c4d/StringTblUS.txt:1-6).
        let dir = test_tempdir();
        let scenario_dir = write_definition_localization_fixture(
            dir.path(),
            "#strict\nfunc Messages() { return $Messages$; }\n",
            "Messages=[\"first\",\"second\"]\n",
        );
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let scenario =
            load_test_scenario(&scenario_dir, &resolver);
        let definition = scenario
            .definitions
            .iter()
            .find(|definition| definition.id == "GOOD").test_value();

        assert!(definition.script.contains("return [\"first\",\"second\"]"));
        assert!(!definition.script.contains("$Messages$"));
        let mut engine = Engine::with_seed(0);
        apply_test_scenario(&scenario, &mut engine);
        let id = engine.spawn_test_object(SpawnConfig::new("GOOD"));
        let index = engine.test_object_index(id);

        assert_eq!(
            engine
                .call_object_function(index, "Messages", Vec::new())
                .expect("Messages runs"),
            clonk_script::Value::Array(vec![
                clonk_script::Value::String("first".to_string().into()),
                clonk_script::Value::String("second".to_string().into()),
            ])
        );
    }

    #[test]
    fn legacy_scenario_script_respects_language_order() {
        // C4ComponentHost tries each LanguageEx code in order after the
        // unsuffixed StringTbl.txt candidate (C4ComponentHost.cpp:65-89;
        // C4Components.h:56).
        let dir = test_tempdir();
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nglobal func Localized() { return \"$MsgIntro2a$\"; }\n",
        );
        write_test_file(
            scenario_dir.join("StringTblUS.txt"),
            "MsgIntro2a=Come with me, princess!\n",
        );
        write_test_file(
            scenario_dir.join("StringTblDE.txt"),
            "MsgIntro2a=Komm mit mir, Prinzessin!\n",
        );
        let resolver = test_resolver(vec![dir.path().to_path_buf()]);

        let scenario =
            Scenario::load_from_path_with_languages(&scenario_dir, &resolver, &["DE", "US"]).test_value();
        let script = &scenario.script.as_ref().test_value().source;

        assert!(script.contains("\"Komm mit mir, Prinzessin!\""));
        assert!(!script.contains("\"Come with me, princess!\""));
    }

    #[test]
    fn scenario_script_assembly_uses_all_c4cfn_script_segments_in_language_order() {
        fn loaded_source(
            root: &std::path::Path,
            scenario_dir: &std::path::Path,
            languages: &[&str],
        ) -> String {
            let resolver = test_resolver(vec![root.to_path_buf()]);
            Scenario::load_from_path_with_languages(scenario_dir, &resolver, languages)
                .expect("scenario components load")
                .script.test_value()
                .source
        }

        // A scenario may consist only of a localized Script{}.c component;
        // each LanguageEx segment is narrowed to two native bytes.
        let localized = test_tempdir();
        let localized_scenario = write_resilience_fixture(localized.path(), None, "// remove me\n");
        std::fs::remove_file(localized_scenario.join("Script.c")).test_value();
        write_test_file(
            localized_scenario.join("ScriptDE.c"),
            b"// $Assembly$\0ignored localized tail",
        );
        write_test_file(
            localized_scenario.join("StringTblDE.txt"),
            b"Assembly=localized only\n",
        );
        write_test_file(
            localized_scenario.join("ScriptOld.c"),
            b"// must stay excluded",
        );
        let source = loaded_source(localized.path(), &localized_scenario, &["DE-extra", "US"]);
        assert_eq!(
            clonk_script::c4_string_bytes(&source),
            b"\n// localized only"
        );

        // The preferred ScriptDE.c exists but is not readable as a file, so
        // its segment falls through to US. C4Script restarts at the empty DE
        // file, which still contributes one LF and suppresses US. Every
        // successful component gets its own leading LF and its own NUL bound.
        let assembled = test_tempdir();
        let assembled_scenario =
            write_resilience_fixture(assembled.path(), None, "// base\0hidden base");
        std::fs::create_dir(assembled_scenario.join("ScriptDE.c")).test_value();
        write_test_file(
            assembled_scenario.join("ScriptUS.c"),
            b"// localized US\0hidden localized tail",
        );
        write_test_file(assembled_scenario.join("C4ScriptDE.c"), b"");
        write_test_file(
            assembled_scenario.join("C4ScriptUS.c"),
            b"// losing legacy component",
        );
        write_test_file(
            assembled_scenario.join("ScriptOld.c"),
            b"// must stay excluded",
        );
        let source = loaded_source(assembled.path(), &assembled_scenario, &["DE", "US"]);
        assert_eq!(
            clonk_script::c4_string_bytes(&source),
            b"\n// base\n// localized US\n"
        );

        // SCopySegment over an empty LanguageEx still yields one empty code:
        // Script.c is selected by both the first and second template segment.
        let empty = test_tempdir();
        let empty_scenario = write_resilience_fixture(empty.path(), None, "// base");
        write_test_file(empty_scenario.join("C4Script.c"), b"// legacy");
        let source = loaded_source(empty.path(), &empty_scenario, &[]);
        assert_eq!(
            clonk_script::c4_string_bytes(&source),
            b"\n// base\n// base\n// legacy"
        );
    }

    /// Joins a default test player: the fixture's `[Player1] Crew=GOOD=1`
    /// places its crew at JOIN like C++ (C4Player::PlaceReadyCrew,
    /// C4Player.cpp:481-570). Returns the objects created by the join.
    fn join_test_player(engine: &mut Engine) -> Vec<ObjectId> {
        let before: std::collections::HashSet<ObjectId> = engine
            .snapshot()
            .objects
            .iter()
            .map(|object| object.id)
            .collect();
        engine
            .join_player(scenario_join_player_config("Tester")).test_value();
        engine
            .snapshot()
            .objects
            .iter()
            .map(|object| object.id)
            .filter(|id| !before.contains(id))
            .collect()
    }

    // Fair crew (the LegacyClonk default config; live-oracle probe read
    // UseFairCrew=1, strength 1000): crew physicals come from the def
    // promoted to RankByExperience(1000)=1 (C4Def.cpp:860-874,
    // C4RankSystem.cpp:226-237) — Energy = max(def, 55 * C4MaxPhysical/
    // 100) = 55000 (PromotionUpdate, C4InfoCore.cpp:207-213). GoldRush
    // live oracle: crew read 55000; bandits keep script-set temporary
    // physicals (25000).
    #[test]
    fn crew_infos_promote_physicals_by_rank_like_cpp() {
        let mut engine = Engine::with_seed(3);
        let mut clonk =
            Definition::from_script("CLNK", "Clonk", "#strict\n").test_value();
        clonk.set_crew_member(true);
        clonk.set_category(crate::CATEGORY_OBJECT | crate::CATEGORY_LIVING);
        clonk.set_physical(crate::PhysicalInfo {
            energy: 50_000,
            scale: 30_000,
            hangle: 30_000,
            swim: 60_000,
            fight: 50_000,
            ..crate::PhysicalInfo::default()
        });
        engine.register_test_definition(clonk);

        engine
            .join_player(crate::JoinPlayerConfig {
                crew: vec![crate::player_file::CrewInfo {
                    id: "CLNK".to_string(),
                    name: "Henry".to_string(),
                    death_message: String::new(),
                    core: Default::default(),
                    rank: 1,
                    rank_name: "Ensign".to_string(),
                    experience: 120,
                    rounds: 0,
                    physical: crate::PhysicalInfo {
                        energy: 55_000,
                        scale: 30_000,
                        hangle: 30_000,
                        swim: 60_000,
                        fight: 50_000,
                        can_scale: 1,
                        can_hangle: 1,
                        can_dig: 1,
                        can_construct: 1,
                        can_chop: 1,
                        ..crate::PhysicalInfo::default()
                    },
                    death_count: 0,
                    total_playing_time: 0,
                    birthday: 0,
                    age: 0,
                    participation: 1,
                    in_action: false,
                    was_in_action: false,
                    in_action_time: 0,
                    has_died: false,
                    extra_data: Vec::new(),
                    portraits: Default::default(),
                }],
                ..scenario_join_player_config("Tester")
            }).test_value();

        let crew = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "CLNK").test_value();
        assert_eq!(
            crew.state.energy, 55_000,
            "rank-1 promotion: max(50000, 55*1000) (C4InfoCore.cpp:212)"
        );
        assert_eq!(
            crew.state.info_physical,
            Some(crate::PhysicalInfo {
                energy: 55_000,
                scale: 30_000,
                hangle: 30_000,
                swim: 60_000,
                fight: 50_000,
                can_scale: 1,
                can_hangle: 1,
                can_dig: 1,
                can_construct: 1,
                can_chop: 1,
                ..crate::PhysicalInfo::default()
            }),
            "persistent info physicals remain promoted by the info rank"
        );
        let crew_index = engine.test_object_index(crew.id);
        assert_eq!(
            engine.object_physical(crew_index),
            crate::PhysicalInfo {
                energy: 55_000,
                scale: 33_500,
                hangle: 33_500,
                swim: 62_000,
                fight: 52_500,
                can_scale: 1,
                can_hangle: 1,
                can_dig: 1,
                can_construct: 1,
                can_chop: 1,
                ..crate::PhysicalInfo::default()
            },
            "FairCrewStrength=1000 is selected live without overwriting Info"
        );
    }
