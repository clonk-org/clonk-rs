// Contiguous slice 6 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: players, objects, object state.

    #[test]
    fn find_construction_site_without_a_script_caller_yields_nil() {
        // `if (!cthr->Caller) return {}` (C4Script.cpp:1966).
        let (result, _) = with_object_host_context(|| {
            find_construction_site(&[Value::C4Id("HUT1".into()), Value::Int(0), Value::Int(1)])
        });
        assert_eq!(result.expect("direct host call runs"), Value::Nil);
    }

    #[test]
    fn get_score_returns_player_points() {
        let player = PlayerState {
            id: 4,
            points: 135,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(4)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_score(&args));
        assert_eq!(result.expect("GetScore succeeds"), Value::Int(135));
    }

    #[test]
    fn do_score_compounds_clamps_and_returns_integer_success() {
        let player = PlayerState {
            id: 4,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(do_score(&[Value::Int(4), Value::Int(5)])?, Value::Int(1));
            assert_eq!(
                get_player_val(&[
                    Value::String("ViewValue".into()),
                    Value::Int(0),
                    Value::Int(4),
                ])?,
                Value::Int(100),
                "DoPoints arms ViewValue before the same script call continues"
            );
            assert_eq!(
                do_score(&[Value::Int(4), Value::Int(5)])?,
                Value::Int(1),
                "DoPoints returns success, not the running total"
            );
            assert_eq!(get_score(&[Value::Int(4)])?, Value::Int(10));

            assert_eq!(
                do_score(&[Value::Int(4), Value::Int(-200_000)])?,
                Value::Int(1)
            );
            assert_eq!(get_score(&[Value::Int(4)])?, Value::Int(-100_000));

            // C4Aul warns about and discards arguments beyond the native
            // function's declared arity; they do not abort the call.
            assert_eq!(
                do_score(&[Value::Int(4), Value::Int(0), Value::Int(999)])?,
                Value::Int(1)
            );
            assert_eq!(get_score(&[Value::Int(4)])?, Value::Int(-100_000));

            assert_eq!(do_score(&[Value::Int(99), Value::Int(7)])?, Value::Int(0));
            assert_eq!(get_score(&[Value::Int(4)])?, Value::Int(-100_000));
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("DoScore calls succeed");
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [
                PlayerCommand::AdjustPoints {
                    player_id: 4,
                    delta: 5,
                },
                PlayerCommand::AdjustPoints {
                    player_id: 4,
                    delta: 5,
                },
                PlayerCommand::AdjustPoints {
                    player_id: 4,
                    delta: -200_000,
                },
                PlayerCommand::AdjustPoints {
                    player_id: 4,
                    delta: 0,
                },
            ]
        ));
    }

    #[test]
    fn do_crew_exp_without_an_object_returns_false() {
        let (result, outcome) =
            with_effect_context(None, &[], HostWorldContext::default(), 1, || {
                do_crew_exp(&[Value::Int(1)])
            });
        assert_eq!(result.expect("DoCrewExp call succeeds"), Value::Bool(false));
        assert!(outcome.player_commands.is_empty());
    }

    #[test]
    fn get_plr_value_returns_total_value() {
        let player = PlayerState {
            id: 9,
            value: 320,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(9)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_value(&args));
        assert_eq!(result.expect("GetPlrValue succeeds"), Value::Int(320));
    }

    #[test]
    fn get_plr_value_gain_returns_gain() {
        let player = PlayerState {
            id: 9,
            value_gain: 45,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(9)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_value_gain(&args));
        assert_eq!(result.expect("GetPlrValueGain succeeds"), Value::Int(45));
    }

    #[test]
    fn get_plr_knowledge_reports_known_definition() {
        let mut player = PlayerState::default();
        player.id = 5;
        player.knowledge = vec!["BRIK".to_string()];
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                category: 0x1,
                ..Default::default()
            },
        )]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(5, player)]),
        );
        let args = [Value::Int(5), Value::C4Id("BRIK".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_knowledge(&args));

        assert_eq!(result.expect("GetPlrKnowledge succeeds"), Value::Bool(true));
    }

    #[test]
    fn get_plr_knowledge_returns_definition_by_index() {
        let mut player = PlayerState::default();
        player.id = 6;
        player.knowledge = vec!["BRIK".to_string(), "STON".to_string()];
        let definitions = HashMap::from([
            (
                "BRIK".to_string(),
                DefinitionMetadata {
                    category: 0x1,
                    ..Default::default()
                },
            ),
            (
                "STON".to_string(),
                DefinitionMetadata {
                    category: 0x2,
                    ..Default::default()
                },
            ),
        ]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(6, player)]),
        );
        let args = [Value::Int(6), Value::Nil, Value::Int(0), Value::Int(0x2)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_knowledge(&args));

        assert_eq!(
            result.expect("GetPlrKnowledge succeeds"),
            Value::C4Id("STON".into())
        );
    }

    #[test]
    fn get_plr_knowledge_indexed_zero_and_all_categories_match_c4idlist() {
        let player = PlayerState {
            id: 26,
            knowledge: vec!["ZERO".to_string(), "BRIK".to_string()],
            ..PlayerState::default()
        };
        let definitions = HashMap::from([
            (DefinitionId::from("ZERO"), DefinitionMetadata::default()),
            (
                DefinitionId::from("BRIK"),
                DefinitionMetadata {
                    category: 0x1,
                    ..DefinitionMetadata::default()
                },
            ),
        ]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(26, player)]),
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<Value, RuntimeError>(Value::Array(vec![
                get_plr_knowledge(&[Value::Int(26), Value::Nil, Value::Int(0)])?,
                get_plr_knowledge(&[Value::Int(26), Value::Nil, Value::Int(0), Value::Int(0)])?,
                get_plr_knowledge(&[Value::Int(26), Value::Nil, Value::Int(0), Value::Int(-1)])?,
                get_plr_knowledge(&[Value::Int(26), Value::Nil, Value::Int(1), Value::Int(-1)])?,
            ]))
        });

        assert_eq!(
            result.expect("GetPlrKnowledge category edge queries succeed"),
            Value::Array(vec![
                Value::Nil,
                Value::Nil,
                Value::C4Id("ZERO".into()),
                Value::C4Id("BRIK".into()),
            ])
        );
    }

    #[test]
    fn set_plr_knowledge_defaults_omitted_remove_to_false() {
        let mut player = PlayerState::default();
        player.id = 7;
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                category: 0x1,
                ..Default::default()
            },
        )]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(7, player)]),
        );
        // Parse_Params pads omitted parameters with nil, whose bool
        // conversion is false (C4AulParse.cpp:2342-2344; C4Value.h:325-330).
        // Dragon Rock's InitializePlayer relies on this two-argument grant.
        let args = [Value::Int(7), Value::C4Id("BRIK".into())];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || set_plr_knowledge(&args));

        assert_eq!(result.expect("SetPlrKnowledge succeeds"), Value::Bool(true));
        assert_eq!(outcome.player_commands.len(), 1);
        match &outcome.player_commands[0] {
            PlayerCommand::GrantKnowledge {
                player_id,
                definition_id,
            } => {
                assert_eq!(*player_id, 7);
                assert_eq!(definition_id, "BRIK");
            }
            other => panic!("unexpected player command: {other:?}"),
        }
    }

    #[test]
    fn set_plr_knowledge_accepts_integer_true_to_revoke() {
        let mut player = PlayerState::default();
        player.id = 8;
        player.knowledge = vec!["BRIK".to_string()];
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                category: 0x1,
                ..Default::default()
            },
        )]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(8, player)]),
        );
        let args = [Value::Int(8), Value::C4Id("BRIK".into()), Value::Int(1)];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || set_plr_knowledge(&args));

        assert_eq!(result.expect("SetPlrKnowledge succeeds"), Value::Bool(true));
        assert_eq!(outcome.player_commands.len(), 1);
        match &outcome.player_commands[0] {
            PlayerCommand::RevokeKnowledge {
                player_id,
                definition_id,
            } => {
                assert_eq!(*player_id, 8);
                assert_eq!(definition_id, "BRIK");
            }
            other => panic!("unexpected player command: {other:?}"),
        }
    }

    #[test]
    fn native_c4id_arguments_apply_the_cpp_conversion_table() {
        // FnFindObject and the other C4ID-typed native slots receive a value
        // only after CheckConvertFunctionParameters. Definition constants
        // keep their C4ID tag, small integers use FnCnvInt2Id, and a direct
        // engine call has legacy eager-falsy conversion.
        for (value, expected) in [
            (None, None),
            (Some(Value::Nil), None),
            (Some(Value::Int(0)), None),
            (Some(Value::Bool(false)), None),
            (Some(Value::Object(0)), None),
            (Some(Value::C4Id("NONE".into())), None),
            (Some(Value::C4Id("NOPC".into())), Some("NOPC")),
            (Some(Value::Int(1)), Some("0001")),
            (Some(Value::Int(42)), Some("0042")),
            (Some(Value::Int(9999)), Some("9999")),
        ] {
            let parsed = parse_native_c4id_argument(value.as_ref(), "FindObject")
                .expect("valid C4ID conversion");
            assert_eq!(parsed.as_deref(), expected);
        }

        for value in [
            Value::Int(-1),
            Value::Int(10_000),
            Value::Bool(true),
            Value::Object(1),
            Value::String("NOPC".into()),
            Value::Array(Vec::new()),
            Value::Proplist(ValueMap::new()),
        ] {
            let error = parse_native_c4id_argument(Some(&value), "FindObject")
                .expect_err("invalid C4ID conversion");
            assert!(error.message().contains("expected C4ID"));
        }
    }

    #[test]
    fn native_c4id_arguments_honor_the_strict_nil_boundary() {
        // Pre-strict-3 native calls first replace every raw-falsy argument
        // with nil. Strict 3 keeps the original false/null-object type, so
        // the subsequent C4ID conversion rejects it. Nil and integer zero
        // remain valid at either strictness.
        for directive in ["", "#strict 2\n"] {
            let mut script = ScriptEngine::new();
            register_host_functions(&mut script);
            script
                .load_script(&format!(
                    r#"{directive}
                    func FalseID() {{ return ScoreboardCol(false); }}
                    func PassedID(value) {{ return ScoreboardCol(value); }}
                    "#
                ))
                .expect("legacy C4ID conversion probe compiles");

            assert_eq!(
                script
                    .call("FalseID", &[])
                    .expect("false eagerly becomes nil"),
                Value::Int(0)
            );
            assert_eq!(
                script
                    .call("PassedID", &[Value::Object(0)])
                    .expect("null object eagerly becomes nil"),
                Value::Int(0)
            );
        }

        let mut strict3 = ScriptEngine::new();
        register_host_functions(&mut strict3);
        strict3
            .load_script(
                r#"
                #strict 3
                func NilID() { return ScoreboardCol(nil); }
                func ZeroID() { return ScoreboardCol(0); }
                func FalseID() { return ScoreboardCol(false); }
                func PassedID(value) { return ScoreboardCol(value); }
                "#,
            )
            .expect("strict-3 C4ID conversion probe compiles");

        for function in ["NilID", "ZeroID"] {
            assert_eq!(
                strict3.call(function, &[]).expect("zero ID converts"),
                Value::Int(0)
            );
        }
        assert!(strict3
            .call("FalseID", &[])
            .expect_err("strict 3 retains bool false")
            .to_string()
            .contains("expected \"id\""));
        assert!(strict3
            .call("PassedID", &[Value::Object(0)])
            .expect_err("strict 3 retains a null object")
            .to_string()
            .contains("expected \"id\""));
    }

    #[test]
    fn native_c4id_slots_convert_before_host_body_early_returns() {
        // Native parameter conversion covers every declared slot before the
        // C++ function body runs. A nil name, invalid player, or priority
        // zero therefore cannot hide a String -> C4ID conversion failure.
        let failures = [
            (
                "CreateScriptPlayer",
                create_script_player(&[
                    Value::String(String::new().into()),
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    Value::String("__AI".into()),
                ]),
            ),
            (
                "SetPortrait",
                set_portrait(&[
                    Value::String(String::new().into()),
                    Value::Nil,
                    Value::String("CLNK".into()),
                ]),
            ),
            (
                "GetPhysical",
                get_physical(&[
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    Value::String("CLNK".into()),
                ]),
            ),
            (
                "GetDefCoreVal",
                get_def_core_val(&[Value::Nil, Value::Nil, Value::String("CLNK".into())]),
            ),
            (
                "GetActMapVal",
                get_act_map_val(&[Value::Nil, Value::Nil, Value::String("CLNK".into())]),
            ),
            (
                "FindObjectOwner",
                find_object_owner(&[Value::String("CLNK".into()), Value::Int(999)]),
            ),
            ("CreateMenu", create_menu(&[Value::String("CLNK".into())])),
            (
                "AddMenuItem",
                add_menu_item(&[Value::Nil, Value::Nil, Value::String("CLNK".into())]),
            ),
            (
                "SetMenuDecoration",
                set_menu_decoration(&[Value::String("CLNK".into())]),
            ),
            ("ChangeDef", change_def(&[Value::String("CLNK".into())])),
            (
                "DefinitionCall",
                definition_call(&[Value::String("CLNK".into()), Value::String("Probe".into())]),
            ),
            (
                "AddEffect",
                add_effect(&[
                    Value::Nil,
                    Value::Nil,
                    Value::Int(0),
                    Value::Nil,
                    Value::Nil,
                    Value::String("CLNK".into()),
                ]),
            ),
            (
                "CustomMessage",
                custom_message(&[
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    Value::String("CLNK".into()),
                ]),
            ),
            (
                "ScoreboardCol",
                scoreboard_col(&[Value::String("CLNK".into())]),
            ),
        ];

        for (function, result) in failures {
            let error = result.expect_err("string must not enter a native C4ID slot");
            assert!(
                error.message().contains("expected C4ID"),
                "{function}: {error}"
            );
        }
    }

    #[test]
    fn get_component_answers_def_counts_and_indexed_ids() {
        // FnGetComponent (C4Script.cpp:2685-2709): with idDef the def's
        // component list answers; idComponent selects the count form,
        // otherwise the index form returns the id (C4VID).
        let mut metadata = DefinitionMetadata {
            ..Default::default()
        };
        metadata.components = vec![("WOOD".to_string(), 3), ("METL".to_string(), 1)];
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::from([(DefinitionId::from("HUTT"), metadata)]),
            HashMap::new(),
        );
        let (result, _) = with_effect_context(None, &[], world.clone(), 1, || {
            let count = get_component(&[
                Value::C4Id("WOOD".into()),
                Value::Int(0),
                Value::Nil,
                Value::C4Id("HUTT".into()),
            ])?;
            assert_eq!(count, Value::Int(3), "count form");
            get_component(&[
                Value::Nil,
                Value::Int(1),
                Value::Nil,
                Value::C4Id("HUTT".into()),
            ])
        });
        assert_eq!(
            result.expect("GetComponent succeeds"),
            Value::C4Id("METL".into()),
            "index form returns the id"
        );
    }

    #[test]
    fn sawmill_component_all_accepts_only_pure_wood_components() {
        // FnComponentAll (C4Script.cpp:1873-1883) rejects an object when
        // any component OTHER than the requested id has a positive count.
        // The real sawmill applies that predicate at Script.c:166-197,
        // especially the fSawable expression on line 176.
        let object = |id, definition: &str, components: &[(&str, i32)]| {
            let mut state = crate::preview_spawn_state(
                Vector2::ZERO,
                OWNER_NONE,
                OWNER_NONE,
                DEFAULT_CATEGORY,
                crate::FULL_CON,
                crate::CONTACT_DENSITY_SOLID,
                Vec::new(),
            );
            state.components = components
                .iter()
                .map(|(id, count)| (DefinitionId::from(*id), *count))
                .collect();
            state.component_order = components
                .iter()
                .map(|(id, _)| DefinitionId::from(*id))
                .collect();
            fixture_world_object(ObjectId::new(id), definition)
            .with_full_state(Rc::new(state))
        };
        let world = HostWorldContext::from_objects([
            object(2, "TRE2", &[("WOOD", 5), ("METL", 0)]),
            object(3, "MIXD", &[("WOOD", 1), ("METL", 1)]),
            object(4, "WOOD", &[]),
        ]);
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                r#"#strict 2
func Sawable(obj) {
  return GetID(obj) != WOOD && GetComponent(WOOD, 0, obj)
         && ComponentAll(obj, WOOD);
}
func Missing() { return ComponentAll(0, WOOD); }
"#,
            )
            .expect("sawmill predicate compiles");

        let (result, _) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                engine.call("Sawable", &[Value::Object(2)])?,
                Value::Bool(true),
                "positive WOOD plus zero-count foreign entries is pure"
            );
            assert_eq!(
                engine.call("Sawable", &[Value::Object(3)])?,
                Value::Bool(false),
                "a positive foreign component makes the object impure"
            );
            assert_eq!(
                engine.call("Sawable", &[Value::Object(4)])?,
                Value::Bool(false),
                "the sawmill excludes loose WOOD before ComponentAll"
            );
            engine.call("Missing", &[])
        });
        assert_eq!(
            result.expect("sawmill predicate runs"),
            Value::Nil,
            "a missing ComponentAll target returns nil"
        );
    }

    #[test]
    fn material_resolves_names_to_numbers_like_cpp() {
        // FnMaterial (C4Script.cpp:2488-2491): Game.Material.Get — the
        // material number, -1 for unknown names.
        let library =
            clonk_resources::MaterialLibrary::parse("[Material Earth]\nName=Earth\nDensity=50\n")
                .expect("library builds");
        let materials = MaterialSet::from_resource_library(&library);
        let expected = materials.get("Earth").expect("earth exists").id().index() as i32;
        let world = world_with(Vec::<HostWorldObject>::new(), None, HashMap::new(), HashMap::new())
        .with_materials(Some(Rc::new(materials)));
        let (result, _) = with_effect_context(None, &[], world.clone(), 1, || {
            let known = material(&[Value::String("Earth".into())])?;
            assert_eq!(known, Value::Int(expected));
            material(&[Value::String("Unobtainium".into())])
        });
        assert_eq!(
            result.expect("Material succeeds"),
            Value::Int(MATERIAL_NONE)
        );
    }

    #[test]
    fn material_name_returns_exact_loaded_name_or_nil_for_invalid_index() {
        // FnMaterialName indexes Game.Material.Map directly: MatValid false
        // returns null; a valid index returns the material's exact Name
        // (C4Script.cpp:4475-4482).
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Glow]\nName=GlowingRock\nDensity=50\n\n\
             [Material Water]\nName=Water\nDensity=25\n",
        )
        .expect("material library builds");
        let materials = MaterialSet::from_resource_library(&library);
        let glowing = materials
            .get("GlowingRock")
            .expect("glowing rock exists")
            .id()
            .index() as i32;
        let world = HostWorldContext::default().with_materials(Some(Rc::new(materials)));
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                material_name(&[Value::Int(glowing)])?,
                material_name(&[Value::Int(-1)])?,
                material_name(&[Value::Int(99)])?,
            ]))
        });

        assert_eq!(
            result.expect("MaterialName succeeds"),
            Value::Array(vec![
                Value::String("GlowingRock".to_string().into()),
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn get_material_count_matches_cpp_real_and_effective_counts() {
        // FnGetMaterialCount returns -1 for an invalid material, otherwise
        // MatCount when fReal is true (or MinHeightCount is zero) and
        // EffectiveMatCount when it is false (C4Script.cpp:2207-2213).
        // C4Landscape::UpdateMatCnt counts only vertical runs reaching
        // MinHeightCount in the effective total (C4Landscape.cpp:2904-2967).
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Oil]\nName=Oil\nDensity=60\nMinHeightCount=4\n\n\
             [Material Gold]\nName=Gold\nDensity=50\n",
        )
        .expect("oil library builds");
        let materials = MaterialSet::from_resource_library(&library);
        let oil = materials.id_of("Oil").expect("oil exists");
        assert_eq!(
            oil.index(),
            0,
            "missing C4ValueInt material becomes index 0"
        );

        // Runs by column: [4], [3, 2], [5]. Raw=14; effective=4+0+5=9.
        let bytes = vec![1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 1, 0, 1, 1];
        let mut densities = vec![0; 3];
        densities[1] = 60;
        densities[2] = 50;
        let names = vec![None, Some("Oil".to_string()), Some("Gold".to_string())];
        let mut landscape = Landscape::new(3, vec![0; 3]).expect("landscape builds");
        landscape.set_world_height(6);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            3,
            6,
            bytes,
            densities,
            names,
            vec![None; 3],
        ));
        landscape.resolve_grid_materials(|name| materials.id_of(name));

        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        )
        .with_materials(Some(Rc::new(materials)));
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                "#strict 2\nfunc Probe() { return [\n\
                 GetMaterialCount(),\n\
                 GetMaterialCount(Material(\"Oil\")),\n\
                 GetMaterialCount(Material(\"Oil\"), true),\n\
                 GetMaterialCount(Material(\"Gold\")),\n\
                 GetMaterialCount(-1),\n\
                 GetMaterialCount(2),\n\
                 GetMaterialCount(Material(\"Oil\"), false, 99)\n\
                 ]; }",
            )
            .expect("material-count probe compiles");

        let (result, _) = with_effect_context(None, &[], world, 1, || engine.call("Probe", &[]));
        assert_eq!(
            result.expect("GetMaterialCount succeeds"),
            Value::Array(vec![
                Value::Int(9),
                Value::Int(9),
                Value::Int(14),
                Value::Int(1),
                Value::Int(-1),
                Value::Int(-1),
                Value::Int(9),
            ])
        );
    }

    #[test]
    fn get_hi_rank_skips_disabled_and_retains_first_equal_rank() {
        // FnGetHiRank (C4Script.cpp:2792-2796) ->
        // C4Player::GetHiRankActiveCrew(false) (C4Player.cpp:1003-1020):
        // skip CrewDisabled, then replace only for a STRICTLY higher rank,
        // so the first of equal ranks wins. No eligible crew returns nil.
        let crew_ids = [11_u64, 22_u64, 33_u64];
        let call = |disabled: &[u64]| {
            let objects: Vec<HostWorldObject> = crew_ids
                .iter()
                .map(|&id| {
                    fixture_world_object(ObjectId::new(id), "Clonk")
                        .with_owner(1)
                    .with_crew_disabled(disabled.contains(&id))
                })
                .collect();
            let mut player = PlayerState::default();
            player.id = 1;
            player.crew = crew_ids.iter().map(|&id| ObjectId::new(id)).collect();
            let world = world_with(objects, None, HashMap::new(), HashMap::from([(1, player)]))
            .with_crew_ranks(std::rc::Rc::new(HashMap::from([
                (11_u64, 5),
                (22_u64, 3),
                (33_u64, 3),
            ])));
            let (result, _) =
                with_effect_context(None, &[], world, 1, || get_hi_rank(&[Value::Int(1)]));
            result.expect("GetHiRank succeeds")
        };

        assert_eq!(
            call(&[11]),
            object_reference_value(ObjectId::new(22)),
            "the disabled rank-5 member is skipped and the first rank-3 member wins the tie"
        );
        assert_eq!(call(&crew_ids), Value::Nil, "all disabled returns nil");
    }

    #[test]
    fn get_rank_defaults_to_caller_and_requires_linked_crew_info() {
        // FnGetRank defaults a null pObj to cthr->Obj, then reads exactly
        // pObj->Info->Rank; an absent caller or absent Info is nil
        // (C4Script.cpp:1378-1383). Rank zero remains integer zero and
        // surplus parameters are evaluated by the VM but ignored by C++.
        let world =
            HostWorldContext::default().with_crew_ranks(Rc::new(HashMap::from([(1, 5), (3, 0)])));

        let (object_result, _) = with_object_host_context_with_world(world.clone(), || {
            Ok(Value::Array(vec![
                get_rank(&[])?,
                get_rank(&[Value::Nil])?,
                get_rank(&[object_reference_value(ObjectId::new(2))])?,
                get_rank(&[object_reference_value(ObjectId::new(3))])?,
                get_rank(&[
                    object_reference_value(ObjectId::new(1)),
                    Value::String("ignored".to_owned().into()),
                ])?,
            ]))
        });
        assert_eq!(
            object_result.expect("object-context GetRank succeeds"),
            Value::Array(vec![
                Value::Int(5),
                Value::Int(5),
                Value::Nil,
                Value::Int(0),
                Value::Int(5),
            ])
        );

        let (global_result, _) = with_effect_context(None, &[], world, 4, || {
            Ok::<_, RuntimeError>(Value::Array(vec![get_rank(&[])?, get_rank(&[Value::Nil])?]))
        });
        assert_eq!(
            global_result.expect("global GetRank succeeds"),
            Value::Array(vec![Value::Nil, Value::Nil])
        );
        assert!(get_rank(&[Value::String("not an object".to_owned().into())]).is_err());
    }

    #[test]
    fn get_crew_returns_nth_crew_member() {
        let crew_ids = [101_u64, 202_u64];
        let objects = vec![
            fixture_world_object(ObjectId::new(crew_ids[0]), "Clonk")
                .with_owner(1),
            fixture_world_object(ObjectId::new(crew_ids[1]), "Clonk")
                .with_owner(1),
        ];
        let mut player = PlayerState::default();
        player.id = 1;
        player.crew = vec![ObjectId::new(crew_ids[0]), ObjectId::new(crew_ids[1])];

        let world = world_with(objects, None, HashMap::new(), HashMap::from([(1, player)]));
        let args = [Value::Int(1), Value::Int(1)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_crew(&args));

        assert_eq!(
            result.expect("GetCrew succeeds"),
            object_reference_value(ObjectId::new(crew_ids[1]))
        );
    }

    #[test]
    fn get_crew_returns_nil_for_out_of_range_index() {
        let crew_ids = [700_u64];
        let objects = vec![fixture_world_object(ObjectId::new(crew_ids[0]), "Clonk")
            .with_owner(3)];
        let mut player = PlayerState::default();
        player.id = 3;
        player.crew = vec![ObjectId::new(crew_ids[0])];

        let world = world_with(objects, None, HashMap::new(), HashMap::from([(3, player)]));
        let args = [Value::Int(3), Value::Int(5)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_crew(&args));

        assert_eq!(result.expect("GetCrew succeeds"), Value::Nil);
    }

    #[test]
    fn get_crew_count_reports_total_crew() {
        let crew_ids = [303_u64, 404_u64, 505_u64];
        let objects = crew_ids
            .iter()
            .map(|id| {
                fixture_world_object(ObjectId::new(*id), "Clonk")
                    .with_owner(2)
            })
            .collect::<Vec<_>>();
        let mut player = PlayerState::default();
        player.id = 2;
        player.crew = crew_ids
            .iter()
            .map(|id| ObjectId::new(*id))
            .collect::<Vec<_>>();

        let world = world_with(objects, None, HashMap::new(), HashMap::from([(2, player)]));
        let args = [Value::Int(2)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_crew_count(&args));

        assert_eq!(result.expect("GetCrewCount succeeds"), Value::Int(3));
    }

    #[test]
    fn get_cursor_defaults_to_current_cursor() {
        let cursor = ObjectId::new(900);
        let mut player = PlayerState::default();
        player.id = 12;
        player.cursor = Some(cursor);
        player.crew = vec![cursor];
        let selection = CrewSelectionState {
            selected: vec![cursor],
            cursor: Some(cursor),
        };

        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(12, player)]),
            HashMap::from([(12, selection)]),
            1,
            false,
        );
        let args = [Value::Int(12)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_cursor_host(&args));

        assert_eq!(
            result.expect("GetCursor succeeds"),
            object_reference_value(cursor)
        );
    }

    #[test]
    fn edit_cursor_returns_local_target_and_hides_it_in_sync_mode() {
        let target = ObjectId::new(906);
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script("#strict 3\nfunc Probe() { return EditCursor(); }")
            .expect("EditCursor probe compiles");

        let query = |world| {
            let (result, _) = with_effect_context(None, &[], world, 1, || {
                Ok::<_, RuntimeError>(
                    script
                        .call("Probe", &[])
                        .expect("EditCursor probe executes"),
                )
            });
            result.expect("EditCursor host context succeeds")
        };

        assert_eq!(
            script
                .call("Probe", &[])
                .expect("missing-console probe executes"),
            Value::Nil
        );
        let local = HostWorldContext::from_objects([scenario_section_world_object(
            target.as_u64(),
            ObjectStatus::Normal,
        )])
        .with_edit_cursor_target(Some(target));
        assert_eq!(query(local.clone()), object_reference_value(target));
        assert_eq!(
            query(local.with_control_sync_mode(true)),
            Value::Nil,
            "network, replay, and recording modes hide the local editor target"
        );
        assert_eq!(
            query(HostWorldContext::default().with_edit_cursor_target(Some(target))),
            Value::Nil,
            "a stale or unknown target behaves like C++ ClearPointers"
        );

        let mut engine = crate::Engine::new();
        engine.set_edit_cursor_target(Some(target));
        assert_eq!(engine.host_world_context().edit_cursor_target, Some(target));
        engine.set_edit_cursor_target(None);
        assert_eq!(engine.host_world_context().edit_cursor_target, None);
    }

    #[test]
    fn get_cursor_omitted_player_defaults_to_player_zero_like_cpp() {
        // Native calls always provide C4AUL_MAX_Par slots and convert an
        // omitted integer slot from nil to zero (C4AulExec.cpp:1364-1396),
        // so FnGetCursor's iPlr is 0 when script calls GetCursor() with no
        // arguments (C4Script.cpp:2905-2925).
        let cursor = ObjectId::new(905);
        let mut player = PlayerState::default();
        player.id = 0;
        player.cursor = Some(cursor);
        player.crew = vec![cursor];

        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            HashMap::from([(0, player)]),
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || get_cursor_host(&[]));

        assert_eq!(
            result.expect("omitted iPlr converts to player zero"),
            object_reference_value(cursor)
        );
    }

    #[test]
    fn get_cursor_returns_selected_member_by_index() {
        let cursor = ObjectId::new(910);
        let other = ObjectId::new(920);
        let mut player = PlayerState::default();
        player.id = 13;
        player.cursor = Some(cursor);
        player.crew = vec![cursor, other];
        let selection = CrewSelectionState {
            selected: vec![cursor, other],
            cursor: Some(cursor),
        };

        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(13, player)]),
            HashMap::from([(13, selection)]),
            1,
            false,
        );
        let args = [Value::Int(13), Value::Int(1)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_cursor_host(&args));

        assert_eq!(
            result.expect("GetCursor succeeds"),
            object_reference_value(other)
        );
    }

    #[test]
    fn get_select_count_reads_the_cached_player_execute_count() {
        let cursor = ObjectId::new(930);
        let other = ObjectId::new(940);
        let mut player = PlayerState::default();
        player.id = 14;
        player.cursor = Some(cursor);
        player.crew = vec![cursor, other];
        player.select_count = 2;
        let selection = CrewSelectionState {
            selected: vec![cursor, other],
            cursor: Some(cursor),
        };

        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            Vec::new(),
            HashMap::from([(14, player)]),
            HashMap::from([(14, selection)]),
            1,
            false,
        );
        let args = [Value::Int(14)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_select_count(&args));

        assert_eq!(result.expect("GetSelectCount succeeds"), Value::Int(2));
    }

    #[test]
    fn get_view_cursor_returns_the_independent_cpp_view_cursor() {
        let view_cursor = ObjectId::new(950);
        let view_target = ObjectId::new(951);
        let mut player = PlayerState {
            id: 15,
            view_cursor: Some(view_cursor),
            ..PlayerState::default()
        };
        player
            .viewports
            .push(PlayerViewport::new(Vector2::ZERO).with_focus(Some(view_target)));
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            HashMap::new(),
            HashMap::from([(15, player)]),
        );
        let args = [Value::Int(15)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_view_cursor(&args));

        assert_eq!(
            result.expect("GetViewCursor succeeds"),
            object_reference_value(view_cursor)
        );
    }

    #[test]
    fn set_view_cursor_replaces_and_clears_the_cpp_view_cursor_pointer() {
        // FnSetViewCursor validates only the player, then assigns ViewCursor;
        // a nil object clears it (C4Script.cpp:2954-2963). The following
        // GetViewCursor in the same script call observes that live assignment
        // (C4Script.cpp:2934-2941).
        let old_focus = ObjectId::new(951);
        let new_focus = ObjectId::new(952);
        let mut player = PlayerState {
            id: 15,
            view_cursor: Some(old_focus),
            ..PlayerState::default()
        };
        player
            .viewports
            .push(PlayerViewport::new(Vector2::ZERO).with_focus(Some(old_focus)));
        let world = HostWorldContext::from_objects_with_players(
            vec![
                find_world_object(old_focus.as_u64(), "OLD_", 0, 0, 15),
                find_world_object(new_focus.as_u64(), "NEW_", 0, 0, 15),
            ],
            vec![player],
        );
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 2\nfunc Probe(object next) {\n\
                 return [GetViewCursor(15), SetViewCursor(15, next),\n\
                         GetViewCursor(15), SetViewCursor(15),\n\
                         GetViewCursor(15), SetViewCursor(99, next)];\n}",
            )
            .expect("SetViewCursor probe compiles");

        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            script.call("Probe", &[Value::Object(new_focus.as_u64())])
        });

        assert_eq!(
            result.expect("SetViewCursor calls succeed"),
            Value::Array(vec![
                object_reference_value(old_focus),
                Value::Bool(true),
                object_reference_value(new_focus),
                Value::Bool(true),
                Value::Nil,
                Value::Bool(false),
            ])
        );
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [
                PlayerCommand::SetViewCursor {
                    player_id: 15,
                    object: Some(object),
                },
                PlayerCommand::SetViewCursor {
                    player_id: 15,
                    object: None,
                },
            ] if *object == new_focus
        ));
    }

    #[test]
    fn get_plr_view_mode_exposes_scrolling_only_outside_control_sync_mode() {
        let player = PlayerState {
            id: 15,
            view_mode: crate::PLAYER_VIEW_MODE_SCROLLING,
            ..PlayerState::default()
        };
        let local = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let query = |world: HostWorldContext, player_id| {
            let args = [Value::Int(player_id)];
            let (result, _) = with_effect_context(None, &[], world, 1, || get_plr_view_mode(&args));
            result.expect("GetPlrViewMode succeeds")
        };

        assert_eq!(
            query(local.clone(), 15),
            Value::Int(crate::PLAYER_VIEW_MODE_SCROLLING)
        );
        assert_eq!(query(local.clone(), 99), Value::Int(-1));
        assert_eq!(
            query(local.with_control_sync_mode(true), 15),
            Value::Int(-1),
            "network, replay, and recording modes hide local view state"
        );
    }

    #[test]
    fn get_plr_view_mode_sync_projection_covers_network_recording_and_replay() {
        let mut engine = crate::Engine::new();
        assert!(!engine.host_world_context().control_sync_mode);

        engine.set_network_game(true);
        engine.set_network_control_mode(true);
        assert!(engine.host_world_context().control_sync_mode);

        engine.set_network_control_mode(false);
        assert!(engine.host_world_context().network_game());
        assert!(!engine.host_world_context().control_sync_mode);

        engine.set_recording_active(true);
        assert!(engine.host_world_context().control_sync_mode);

        engine.set_recording_active(false);
        assert!(!engine.host_world_context().control_sync_mode);

        engine.set_network_game(false);
        engine.set_control_host(false);
        engine.set_replay_control(true);
        assert!(engine.host_world_context().control_sync_mode);
    }

    #[test]
    fn get_time_returns_process_milliseconds_only_outside_control_sync_mode() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script("#strict 3\nfunc Probe() { return GetTime(); }")
            .expect("GetTime probe compiles");

        assert_eq!(
            script.call("Probe", &[]).expect("CM_None probe executes"),
            Value::Nil,
            "a missing game context matches C4GameControl::CM_None"
        );

        let query = |engine: &crate::Engine| {
            let (result, _) =
                with_effect_context(None, &[], engine.host_world_context(), 1, || {
                    Ok::<_, RuntimeError>(
                        script.call("Probe", &[]).expect("GetTime probe executes"),
                    )
                });
            result.expect("GetTime host context succeeds")
        };

        let mut engine = crate::Engine::new();
        let Value::Int(first) = query(&engine) else {
            panic!("local GetTime must return an integer");
        };
        std::thread::sleep(std::time::Duration::from_millis(2));
        let Value::Int(second) = query(&engine) else {
            panic!("local GetTime must keep returning an integer");
        };
        assert!(
            second >= first,
            "adjacent local millisecond samples may be equal but not decrease"
        );

        engine.set_network_control_mode(true);
        assert_eq!(query(&engine), Value::Nil, "network control is synced");
        engine.set_network_control_mode(false);
        engine.set_recording_active(true);
        assert_eq!(query(&engine), Value::Nil, "recording is synced");
        engine.set_recording_active(false);
        engine.set_replay_control(true);
        assert_eq!(query(&engine), Value::Nil, "replay control is synced");
    }

    #[test]
    fn set_plr_view_targets_the_viewport_without_changing_view_cursor() {
        // FnSetPlrView switches to C4PVM_Target/ViewTarget, which is distinct
        // from the saved ViewCursor pointer queried by GetViewCursor
        // (C4Script.cpp:2545-2550,2931-2937; C4Player.cpp:917-920).
        let view_cursor = ObjectId::new(953);
        let view_target = ObjectId::new(954);
        let next_view_cursor = ObjectId::new(955);
        let mut player = PlayerState {
            id: 15,
            view_cursor: Some(view_cursor),
            ..PlayerState::default()
        };
        player
            .viewports
            .push(PlayerViewport::new(Vector2::ZERO).with_focus(Some(view_cursor)));
        let world = HostWorldContext::from_objects_with_players(
            vec![
                find_world_object(view_cursor.as_u64(), "CURS", 0, 0, 15),
                find_world_object(view_target.as_u64(), "TRGT", 0, 0, 15),
                find_world_object(next_view_cursor.as_u64(), "NEXT", 0, 0, 15),
            ],
            vec![player],
        );
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 2\nfunc Probe(object target, object next) {\n\
                 return [GetViewCursor(15), GetPlrView(15),\n\
                         SetPlrView(15), GetPlrView(15),\n\
                         SetPlrView(15, target), GetPlrView(15),\n\
                         SetViewCursor(15, next), GetViewCursor(15),\n\
                         GetPlrView(15), GetPlrViewMode(15),\n\
                         SetPlrView(99, target), GetPlrView(99)];\n}",
            )
            .expect("SetPlrView probe compiles");

        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            let result = script.call(
                "Probe",
                &[
                    Value::Object(view_target.as_u64()),
                    Value::Object(next_view_cursor.as_u64()),
                ],
            );
            let state = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.player_state(15))
                    .cloned()
                    .expect("player remains in host context")
            });
            result.map(|result| (result, state))
        });

        let (result, state) = result.expect("SetPlrView calls succeed");
        assert_eq!(
            result,
            Value::Array(vec![
                object_reference_value(view_cursor),
                Value::Nil,
                Value::Bool(true),
                Value::Nil,
                Value::Bool(true),
                object_reference_value(view_target),
                Value::Bool(true),
                object_reference_value(next_view_cursor),
                object_reference_value(view_target),
                Value::Int(crate::PLAYER_VIEW_MODE_TARGET),
                Value::Bool(false),
                Value::Nil,
            ])
        );
        assert_eq!(
            state.view_mode,
            crate::PLAYER_VIEW_MODE_TARGET,
            "SetViewCursor must not leave target mode"
        );
        assert_eq!(state.view_cursor, Some(next_view_cursor));
        assert_eq!(
            state.viewports.first().and_then(|viewport| viewport.focus),
            Some(next_view_cursor),
            "viewport UI focus follows ViewCursor, not ViewTarget"
        );
        assert_eq!(
            state.viewports.first().map(|viewport| viewport.center),
            Some(Vector2::ZERO),
            "SetPlrView only changes mode/target; C4Player::UpdateView updates center later"
        );
        assert_eq!(
            state.view_target,
            Some(view_target),
            "SetViewCursor changes logical ViewCursor without overriding active ViewTarget"
        );
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [
                PlayerCommand::SetPlrView {
                    player_id: 15,
                    object: None,
                },
                PlayerCommand::SetPlrView {
                    player_id: 15,
                    object: Some(object),
                },
                PlayerCommand::SetViewCursor {
                    player_id: 15,
                    object: Some(next),
                },
            ] if *object == view_target && *next == next_view_cursor
        ));
    }

    #[test]
    fn remove_object_clears_preexisting_and_same_call_player_view_pointers() {
        // C4Object removal calls C4Player::ClearPointers synchronously, so
        // untouched saved ViewCursor pointers and a ViewTarget installed
        // earlier in this VM call both disappear before the next getter
        // (oracle-src-pinned src/C4Player.cpp:57-77;
        // src/C4Script.cpp:456-460).
        let cursor = ObjectId::new(956);
        let removed = ObjectId::new(957);
        let player = PlayerState {
            id: 15,
            cursor: Some(cursor),
            view_cursor: Some(removed),
            viewports: vec![PlayerViewport::new(Vector2::new(12, 34)).with_focus(Some(removed))],
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            vec![
                find_world_object(cursor.as_u64(), "CURS", 0, 0, 15),
                find_world_object(removed.as_u64(), "GONE", 0, 0, 15),
            ],
            vec![player],
        )
        .with_player_fow_view_objects([(15, [removed])]);
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 2\nfunc Probe(object target) {\n\
                 var old_cursor = GetViewCursor(15);\n\
                 SetPlrView(15, target);\n\
                 var old_target = GetPlrView(15);\n\
                 RemoveObject();\n\
                 return [old_cursor, GetViewCursor(15),\n\
                         old_target, GetPlrView(15)];\n}",
            )
            .expect("pointer-clear probe compiles");
        let object = HostObjectContext {
            id: removed,
            owner: 15,
            controller: 15,
            position: Vector2::new(20, 30),
            ..idle_object_context()
        };

        let (result, outcome) = with_effect_context(Some(object), &[], world, 1, || {
            let result = script.call("Probe", &[Value::Object(removed.as_u64())]);
            let (state, retained_fow_link) = HOST_CONTEXT.with(|cell| {
                let borrow = cell.borrow();
                let context = borrow.as_ref().expect("host context remains");
                (
                    context
                        .player_state(15)
                        .cloned()
                        .expect("player remains in host context"),
                    context.world.player_has_fow_view_object(15, removed),
                )
            });
            result.map(|result| (result, state, retained_fow_link))
        });
        let (result, state, retained_fow_link) = result.expect("pointer-clear probe succeeds");

        assert_eq!(
            result,
            Value::Array(vec![
                object_reference_value(removed),
                Value::Nil,
                object_reference_value(removed),
                Value::Nil,
            ])
        );
        assert!(outcome.destroy_object);
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [
                PlayerCommand::SetPlrView {
                    player_id: 15,
                    object: Some(target),
                },
                PlayerCommand::ClearPlayerObjectPointersBeforeAdjust {
                    player_id: 15,
                    object: before,
                },
                PlayerCommand::ClearPlayerObjectPointersAfterAdjust {
                    player_id: 15,
                    object: after,
                },
            ] if *target == removed && *before == removed && *after == removed
        ));
        assert_eq!(state.cursor, Some(cursor));
        assert_eq!(state.view_cursor, None);
        assert_eq!(state.view_target, None);
        assert_eq!(state.view_mode, crate::PLAYER_VIEW_MODE_TARGET);
        assert!(
            !retained_fow_link,
            "non-death ClearPointers removes the player's FoW link synchronously"
        );
        assert_eq!(state.viewports[0].focus, Some(cursor));
        assert_eq!(state.viewports[0].center, Vector2::new(12, 34));
    }

    #[test]
    fn remove_object_copy_out_clears_authoritative_fow_membership() {
        // AssignRemoval clears Status and then Game.ClearPointers reaches the
        // owner's non-death FoW removal before RemoveObject returns
        // (oracle-src-pinned src/C4Object.cpp:240-320;
        // src/C4Player.cpp:57-77; src/C4Script.cpp:456-460).
        let definition = crate::Definition::from_script(
            "RFOW",
            "Removed FoW object",
            r#"#strict
public func Trigger()
{
    return RemoveObject();
}
"#,
        )
        .expect("removed-FoW script compiles");
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("removed-FoW player registers");
        engine
            .register_definition(definition)
            .expect("removed-FoW definition registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("RFOW")
                    .with_owner(0)
                    .with_plr_view_range(500),
            )
            .expect("removed-FoW object spawns");
        let index = engine
            .find_object_index(target)
            .expect("removed-FoW object exists");
        assert!(engine
            .player(0)
            .is_some_and(|player| player.has_fow_view_object(target)));

        assert_eq!(
            engine
                .call_object_function(index, "Trigger", Vec::new())
                .expect("removed-FoW trigger succeeds"),
            Value::Bool(true)
        );
        assert!(engine.objects[index].destroyed);
        assert!(
            !engine
                .player(0)
                .is_some_and(|player| player.has_fow_view_object(target)),
            "AssignRemoval's status-zero copy-out removes the authoritative FoW link"
        );
    }

    #[test]
    fn set_view_offset_requests_the_first_physical_match_without_sync_state() {
        // FnSetViewOffset writes C4Viewport::ViewOffsX/Y when the requested
        // player has a physical viewport. A valid player with no matching
        // viewport still returns true; the app resolves that process-local
        // lookup after preserving script-call order (C4Script.cpp:5676-5687).
        let player = PlayerState {
            id: 15,
            ..PlayerState::default()
        };
        let local_requests = Rc::new(RefCell::new(Vec::new()));
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player.clone()],
        )
        .with_local_players([15])
        .with_viewport_presentation_requests(false, Rc::clone(&local_requests))
        .with_film_viewport_available(true);
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                set_view_offset(&[Value::Int(15), Value::Int(7), Value::Int(-4)])?,
                Value::Bool(true)
            );
            assert_eq!(
                set_view_offset(&[Value::Int(99), Value::Int(1), Value::Int(2)])?,
                Value::Bool(false)
            );
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("local SetViewOffset calls succeed");
        assert!(outcome.player_commands.is_empty());
        assert_eq!(
            *local_requests.borrow(),
            vec![crate::ViewportPresentationRequest::SetViewOffset {
                player: 15,
                offset: Vector2::new(7, -4),
            }]
        );

        let remote_requests = Rc::new(RefCell::new(Vec::new()));
        let remote_world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        )
        .with_local_players([])
        .with_viewport_presentation_requests(false, Rc::clone(&remote_requests))
        .with_film_viewport_available(true);
        let (remote_result, remote_outcome) =
            with_effect_context(None, &[], remote_world, 1, || {
                set_view_offset(&[Value::Int(15), Value::Int(9), Value::Int(3)])
            });
        assert_eq!(
            remote_result.expect("remote SetViewOffset is sync-safe"),
            Value::Bool(true)
        );
        assert!(remote_outcome.player_commands.is_empty());
        assert_eq!(
            *remote_requests.borrow(),
            vec![crate::ViewportPresentationRequest::SetViewOffset {
                player: 15,
                offset: Vector2::new(9, 3),
            }],
            "remote/replay displayed players are resolved against the physical list"
        );

        let absent_requests = Rc::new(RefCell::new(Vec::new()));
        let absent_world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![PlayerState {
                id: 15,
                ..PlayerState::default()
            }],
        )
        .with_viewport_presentation_requests(false, Rc::clone(&absent_requests));
        let (absent_result, _) = with_effect_context(None, &[], absent_world, 1, || {
            set_view_offset(&[Value::Int(15), Value::Int(5), Value::Int(6)])
        });
        assert_eq!(
            absent_result.expect("headless call succeeds"),
            Value::Bool(true)
        );
        assert!(
            absent_requests.borrow().is_empty(),
            "GetViewport returned null at call time"
        );
    }

    #[test]
    fn clear_last_plr_com_clears_only_the_two_cpp_command_latches() {
        // FnClearLastPlrCom clears LastCom and LastComDownDouble, but not
        // LastComDelay or PressedComs (C4Script.cpp:2624-2635).
        let player = PlayerState {
            id: 16,
            control: PlayerControlState {
                last_com: 7,
                last_com_delay: 11,
                last_com_down_double: 4,
                pressed_coms: 19,
                ..PlayerControlState::default()
            },
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 2\nfunc P(string key) { return GetPlayerVal(key, \"Player\", 16); }\n\
                 func Probe() {\n\
                   var ok = ClearLastPlrCom(16);\n\
                   return [ok, P(\"LastCom\"), P(\"LastComDel\"),\n\
                           P(\"LastComDownDouble\"), P(\"PressedComs\"),\n\
                           ClearLastPlrCom(99)];\n}",
            )
            .expect("ClearLastPlrCom probe compiles");

        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || script.call("Probe", &[]));

        assert_eq!(
            result.expect("ClearLastPlrCom calls succeed"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(0),
                Value::Int(11),
                Value::Int(0),
                Value::Int(19),
                Value::Bool(false),
            ])
        );
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::ClearLastPlrCom { player_id: 16 }]
        ));
    }

    #[test]
    fn get_homebase_material_returns_count_for_definition() {
        let mut player = PlayerState::default();
        player.id = 1;
        player.home_base_material.insert("BRIK".to_string(), 3_u32);
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                category: 1,
                ..Default::default()
            },
        )]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(1, player)]),
        );
        let args = [Value::Int(1), Value::C4Id("BRIK".into())];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_homebase_material(&args));

        assert_eq!(result.expect("GetHomebaseMaterial succeeds"), Value::Int(3));
    }

    #[test]
    fn homebase_queries_follow_ordered_c4id_list_semantics() {
        let player = PlayerState {
            id: 1,
            home_base_material_entries: vec![
                ("ZMAT".into(), 0),
                ("MISS".into(), -7),
                ("AMAT".into(), 5),
            ],
            home_base_production_entries: vec![
                ("ZPRD".into(), 0),
                ("MIPR".into(), -9),
                ("APRD".into(), 6),
            ],
            ..PlayerState::default()
        };
        let definition = |category| DefinitionMetadata {
            category,
            ..DefinitionMetadata::default()
        };
        let definitions = HashMap::from([
            ("ZMAT".into(), definition(1)),
            ("AMAT".into(), definition(2)),
            ("ZPRD".into(), definition(4)),
            ("APRD".into(), definition(8)),
        ]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(1, player)]),
        );

        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let indexed = |query: fn(&[Value]) -> Result<Value, RuntimeError>,
                           index: Value,
                           category: Option<i32>| {
                let mut args = vec![Value::Int(1), Value::Nil, index];
                if let Some(category) = category {
                    args.push(Value::Int(category));
                }
                query(&args)
            };
            Ok::<_, RuntimeError>(Value::Array(vec![
                indexed(get_homebase_material, Value::Int(0), Some(-1))?,
                indexed(get_homebase_material, Value::Int(1), Some(-1))?,
                indexed(get_homebase_material, Value::Int(2), Some(-1))?,
                indexed(get_homebase_material, Value::Nil, Some(-1))?,
                indexed(get_homebase_material, Value::Int(0), None)?,
                indexed(get_homebase_material, Value::Int(0), Some(0))?,
                indexed(get_homebase_material, Value::Int(0), Some(2))?,
                get_homebase_material(&[Value::Int(1), Value::C4Id("MISS".into())])?,
                indexed(get_homebase_production, Value::Int(0), Some(-1))?,
                indexed(get_homebase_production, Value::Int(1), Some(-1))?,
                indexed(get_homebase_production, Value::Int(2), Some(-1))?,
                indexed(get_homebase_production, Value::Nil, Some(-1))?,
                indexed(get_homebase_production, Value::Int(0), None)?,
                indexed(get_homebase_production, Value::Int(0), Some(0))?,
                indexed(get_homebase_production, Value::Int(0), Some(8))?,
                get_homebase_production(&[Value::Int(1), Value::C4Id("MIPR".into())])?,
            ]))
        });

        assert_eq!(
            result.expect("home-base queries succeed"),
            Value::Array(vec![
                Value::C4Id("ZMAT".into()),
                Value::C4Id("AMAT".into()),
                Value::Nil,
                Value::C4Id("ZMAT".into()),
                Value::Nil,
                Value::Nil,
                Value::C4Id("AMAT".into()),
                Value::Int(-7),
                Value::C4Id("ZPRD".into()),
                Value::C4Id("APRD".into()),
                Value::Nil,
                Value::C4Id("ZPRD".into()),
                Value::Nil,
                Value::Nil,
                Value::C4Id("APRD".into()),
                Value::Int(-9),
            ])
        );
    }

    #[test]
    fn do_homebase_material_records_player_command() {
        let mut player = PlayerState::default();
        player.id = 1;
        player.home_base_material.insert("BRIK".to_string(), 1_u32);
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                category: 1,
                ..Default::default()
            },
        )]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(1, player)]),
        );
        let args = [Value::Int(1), Value::C4Id("BRIK".into()), Value::Int(2)];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || do_homebase_material(&args));

        assert_eq!(
            result.expect("DoHomebaseMaterial succeeds"),
            Value::Bool(true)
        );
        assert_eq!(outcome.player_commands.len(), 1);
        match &outcome.player_commands[0] {
            PlayerCommand::AdjustHomeBaseMaterial {
                player_id,
                definition_id,
                delta,
            } => {
                assert_eq!(*player_id, 1);
                assert_eq!(definition_id, "BRIK");
                assert_eq!(*delta, 2);
            }
            other => panic!("unexpected player command: {other:?}"),
        }
    }

    #[test]
    fn do_homebase_production_records_player_command() {
        let mut player = PlayerState::default();
        player.id = 1;
        let definitions = HashMap::from([(
            "BRIK".to_string(),
            DefinitionMetadata {
                category: 1,
                ..Default::default()
            },
        )]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(1, player)]),
        );
        let args = [Value::Int(1), Value::C4Id("BRIK".into()), Value::Int(1)];
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || do_homebase_production(&args));

        assert_eq!(
            result.expect("DoHomebaseProduction succeeds"),
            Value::Bool(true)
        );
        assert_eq!(outcome.player_commands.len(), 1);
        match &outcome.player_commands[0] {
            PlayerCommand::AdjustHomeBaseProduction {
                player_id,
                definition_id,
                delta,
            } => {
                assert_eq!(*player_id, 1);
                assert_eq!(definition_id, "BRIK");
                assert_eq!(*delta, 1);
            }
            other => panic!("unexpected player command: {other:?}"),
        }
    }

    #[test]
    fn do_homebase_hosts_keep_signed_uncapped_and_zero_entries() {
        let player = PlayerState {
            id: 1,
            home_base_material_entries: vec![("MNEG".into(), 5)],
            home_base_production_entries: vec![("PNEG".into(), 5)],
            ..PlayerState::default()
        };
        let definitions = ["MNEG", "MPLS", "MZER", "PNEG", "PPLS", "PZER"]
            .into_iter()
            .map(|id| (id.into(), DefinitionMetadata::default()))
            .collect();
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(1, player)]),
        );

        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            let adjust = |host: fn(&[Value]) -> Result<Value, RuntimeError>,
                          definition: &str,
                          change: Option<i32>| {
                let mut args = vec![Value::Int(1), Value::C4Id(definition.into())];
                if let Some(change) = change {
                    args.push(Value::Int(change));
                }
                host(&args)
            };
            assert_eq!(
                adjust(do_homebase_material, "MPLS", Some(100))?,
                Value::Bool(true)
            );
            assert_eq!(
                adjust(do_homebase_material, "MNEG", Some(-10))?,
                Value::Bool(true)
            );
            assert_eq!(
                adjust(do_homebase_material, "MZER", None)?,
                Value::Bool(true)
            );
            assert_eq!(
                adjust(do_homebase_production, "PPLS", Some(100))?,
                Value::Bool(true)
            );
            assert_eq!(
                adjust(do_homebase_production, "PNEG", Some(-10))?,
                Value::Bool(true)
            );
            assert_eq!(
                adjust(do_homebase_production, "PZER", None)?,
                Value::Bool(true)
            );

            let indexed = |query: fn(&[Value]) -> Result<Value, RuntimeError>, index: i32| {
                query(&[Value::Int(1), Value::Nil, Value::Int(index), Value::Int(-1)])
            };
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_homebase_material(&[Value::Int(1), Value::C4Id("MPLS".into())])?,
                get_homebase_material(&[Value::Int(1), Value::C4Id("MNEG".into())])?,
                indexed(get_homebase_material, 0)?,
                indexed(get_homebase_material, 2)?,
                get_homebase_production(&[Value::Int(1), Value::C4Id("PPLS".into())])?,
                get_homebase_production(&[Value::Int(1), Value::C4Id("PNEG".into())])?,
                indexed(get_homebase_production, 0)?,
                indexed(get_homebase_production, 2)?,
            ]))
        });

        assert_eq!(
            result.expect("home-base adjustments succeed"),
            Value::Array(vec![
                Value::Int(100),
                Value::Int(-5),
                Value::C4Id("MNEG".into()),
                Value::C4Id("MZER".into()),
                Value::Int(100),
                Value::Int(-5),
                Value::C4Id("PNEG".into()),
                Value::C4Id("PZER".into()),
            ])
        );
        let adjustments = outcome
            .player_commands
            .iter()
            .filter_map(|command| match command {
                PlayerCommand::AdjustHomeBaseMaterial {
                    definition_id,
                    delta,
                    ..
                } => Some(("material", definition_id.as_str(), *delta)),
                PlayerCommand::AdjustHomeBaseProduction {
                    definition_id,
                    delta,
                    ..
                } => Some(("production", definition_id.as_str(), *delta)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            adjustments,
            vec![
                ("material", "MPLS", 100),
                ("material", "MNEG", -10),
                ("material", "MZER", 0),
                ("production", "PPLS", 100),
                ("production", "PNEG", -10),
                ("production", "PZER", 0),
            ]
        );
    }

    #[test]
    fn set_transfer_zone_registers_command_for_active_object() {
        let args = [Value::Int(2), Value::Int(3), Value::Int(5), Value::Int(7)];
        let world = world_with(
            vec![fixture_world_object(ObjectId::new(1), "ZoneTester")],
            None,
            HashMap::new(),
            HashMap::new(),
        );
        let (result, outcome) =
            with_object_host_context_with_world(world, || set_transfer_zone(&args));
        assert_eq!(result.expect("SetTransferZone succeeds"), Value::Bool(true));
        assert_eq!(outcome.transfer_zones.len(), 1);
        match &outcome.transfer_zones[0] {
            TransferZoneCommand::Set { owner, rect } => {
                assert_eq!(*owner, ObjectId::new(1));
                assert_eq!(rect.x, 2);
                assert_eq!(rect.y, 3);
                assert_eq!(rect.width, 5);
                assert_eq!(rect.height, 7);
            }
            other => panic!("expected set command, got {:?}", other),
        }
    }

    #[test]
    fn set_transfer_zone_omitted_dimensions_clear_the_active_object_zone() {
        // Parse_Params pads FnSetTransferZone's five slots with nil
        // (C4AulParse.cpp:2342-2344), and the engine call converts its four
        // integer slots to zero (C4AulExec.cpp:1364-1396). FnSetTransferZone
        // then uses the active object (C4Script.cpp:3145-3149), so `()` clears
        // that object's existing zone (C4TransferZone.cpp:78-83).
        let (result, outcome) = with_object_host_context(|| set_transfer_zone(&[]));

        assert_eq!(result.expect("SetTransferZone succeeds"), Value::Bool(true));
        assert_eq!(outcome.transfer_zones.len(), 1);
        match outcome.transfer_zones.first() {
            Some(TransferZoneCommand::Clear { owner }) => {
                assert_eq!(*owner, ObjectId::new(1));
            }
            other => panic!("expected clear command, got {:?}", other),
        }
    }

    #[test]
    fn set_transfer_zone_with_zero_size_clears_existing() {
        let world = world_with(
            vec![fixture_world_object(ObjectId::new(1), "ZoneTester")],
            None,
            HashMap::new(),
            HashMap::new(),
        );
        let (result, outcome) = with_object_host_context_with_world(world, || {
            set_transfer_zone(&[Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(10)])
        });
        assert_eq!(result.expect("SetTransferZone succeeds"), Value::Bool(true));
        assert_eq!(outcome.transfer_zones.len(), 1);
        match outcome.transfer_zones.first() {
            Some(TransferZoneCommand::Clear { owner }) => {
                assert_eq!(*owner, ObjectId::new(1));
            }
            other => panic!("expected clear command, got {:?}", other),
        }
    }

    #[test]
    fn negative_transfer_zone_command_stays_set() {
        let (result, outcome) = with_object_host_context(|| {
            set_transfer_zone(&[Value::Int(0), Value::Int(0), Value::Int(-1), Value::Int(10)])
        });

        assert_eq!(result.expect("SetTransferZone succeeds"), Value::Bool(true));
        match outcome.transfer_zones.first() {
            Some(TransferZoneCommand::Set { owner, rect }) => {
                assert_eq!(*owner, ObjectId::new(1));
                assert_eq!((rect.x, rect.y, rect.width, rect.height), (0, 0, -1, 10));
            }
            other => panic!("expected set command, got {:?}", other),
        }
    }

    #[test]
    fn set_transfer_zone_resolves_the_in_flight_object_like_cpp() {
        // FnSetTransferZone reads pObj->x/y off the LIVE object
        // (C4Script.cpp:3151-3156). The C4Object exists before its own
        // Initialize fires (C4Object::Init calls the script AFTER
        // construction, C4Object.cpp:215+), so a `SetTransferZone` from
        // Initialize works even when the world snapshot predates the
        // object — WZKP's UpdateTransferZone via the player-join homebase
        // placement. The empty default world reproduces the race: the
        // executing scope is the only knowledge of object 1.
        let (result, outcome) = with_object_host_context(|| {
            set_transfer_zone(&[Value::Int(2), Value::Int(3), Value::Int(5), Value::Int(7)])
        });
        assert_eq!(result.expect("SetTransferZone succeeds"), Value::Bool(true));
        assert_eq!(outcome.transfer_zones.len(), 1);
        match outcome.transfer_zones.first() {
            Some(TransferZoneCommand::Set { owner, rect }) => {
                assert_eq!(*owner, ObjectId::new(1));
                assert_eq!((rect.x, rect.y, rect.width, rect.height), (2, 3, 5, 7));
            }
            other => panic!("expected set command, got {:?}", other),
        }
    }

    /// An active-object context carrying magic state — the FnDoMagicEnergy/
    /// FnGetMagicEnergy fixtures (C4Script.cpp:517-550).
    fn with_magic_object_context<F, T>(
        magic_energy: i32,
        physical_magic: i32,
        func: F,
    ) -> (Result<T, RuntimeError>, EffectContextOutcome)
    where
        F: FnOnce() -> Result<T, RuntimeError>,
    {
        with_effect_context(
            Some(
                idle_object_context()
                .with_physicals(
                    None,
                    None,
                    Vec::new(),
                    PhysicalInfo {
                        magic: physical_magic,
                        ..PhysicalInfo::default()
                    },
                )
                .with_magic_energy(magic_energy),
            ),
            &[],
            HostWorldContext::default(),
            1,
            func,
        )
    }

    #[test]
    fn do_magic_energy_scales_by_the_physical_factor_like_cpp() {
        // FnDoMagicEnergy (C4Script.cpp:517-544): iChange *=
        // MagicPhysicalFactor (1000, C4Object.h:81), then BoundBy into
        // 0..GetPhysical()->Magic — WizardTower RefillMagic's
        // DoMagicEnergy(+1).
        let (result, outcome) =
            with_magic_object_context(1_500, 200_000, || do_magic_energy(&[Value::Int(1)]));
        assert_eq!(result.expect("DoMagicEnergy succeeds"), Value::Bool(true));
        assert_eq!(
            outcome.object_update.and_then(|update| update.magic_energy),
            Some(2_500)
        );
    }

    #[test]
    fn do_magic_energy_full_overload_fails_without_partial_like_cpp() {
        // `if (pObj->MagicEnergy + iChange > pObj->GetPhysical()->Magic)`
        // without fAllowPartial returns false and writes nothing
        // (C4Script.cpp:523-526).
        let (result, outcome) =
            with_magic_object_context(199_500, 200_000, || do_magic_energy(&[Value::Int(1)]));
        assert_eq!(result.expect("DoMagicEnergy runs"), Value::Bool(false));
        assert_eq!(
            outcome.object_update.and_then(|update| update.magic_energy),
            None,
            "a refused change leaves MagicEnergy untouched"
        );
    }

    #[test]
    fn do_magic_energy_partial_overload_clamps_to_the_cap_like_cpp() {
        // fAllowPartial clamps the gain to the remaining headroom
        // (C4Script.cpp:527-529); a zero remainder still fails (:528).
        let (result, outcome) = with_magic_object_context(199_500, 200_000, || {
            do_magic_energy(&[Value::Int(1), Value::Nil, Value::Bool(true)])
        });
        assert_eq!(result.expect("DoMagicEnergy runs"), Value::Bool(true));
        assert_eq!(
            outcome.object_update.and_then(|update| update.magic_energy),
            Some(200_000)
        );

        let (result, outcome) = with_magic_object_context(200_000, 200_000, || {
            do_magic_energy(&[Value::Int(1), Value::Nil, Value::Bool(true)])
        });
        assert_eq!(
            result.expect("DoMagicEnergy runs"),
            Value::Bool(false),
            "zero headroom fails even with fAllowPartial"
        );
        assert_eq!(
            outcome.object_update.and_then(|update| update.magic_energy),
            None
        );
    }

    #[test]
    fn do_magic_energy_underload_mirrors_the_cpp_partial_rules() {
        // `if (pObj->MagicEnergy + iChange < 0)` (C4Script.cpp:532-538):
        // refused outright without fAllowPartial, clamped to -MagicEnergy
        // with it, and a zero clamp still fails.
        let (result, _) =
            with_magic_object_context(1_500, 200_000, || do_magic_energy(&[Value::Int(-2)]));
        assert_eq!(result.expect("DoMagicEnergy runs"), Value::Bool(false));

        let (result, outcome) = with_magic_object_context(1_500, 200_000, || {
            do_magic_energy(&[Value::Int(-2), Value::Nil, Value::Bool(true)])
        });
        assert_eq!(result.expect("DoMagicEnergy runs"), Value::Bool(true));
        assert_eq!(
            outcome.object_update.and_then(|update| update.magic_energy),
            Some(0)
        );

        let (result, _) = with_magic_object_context(0, 200_000, || {
            do_magic_energy(&[Value::Int(-2), Value::Nil, Value::Bool(true)])
        });
        assert_eq!(
            result.expect("DoMagicEnergy runs"),
            Value::Bool(false),
            "an already-empty store fails the drain"
        );
    }

    #[test]
    fn get_magic_energy_reads_in_physical_factor_units_like_cpp() {
        // FnGetMagicEnergy: MagicEnergy / MagicPhysicalFactor
        // (C4Script.cpp:546-550) — SkiesOfFire's InitializePlayer refill
        // reads it back through NoMagicEnergy's global override.
        let (result, _) = with_magic_object_context(2_500, 200_000, || get_magic_energy(&[]));
        assert_eq!(result.expect("GetMagicEnergy runs"), Value::Int(2));
    }

    #[test]
    fn add_effect_registers_command_and_updates_view() {
        let state = empty_state();
        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(150),
                Value::Int(3),
            ])
        });
        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add { effect, .. } => {
                assert_eq!(effect.name, "Glow");
                assert_eq!(effect.priority, 150);
                assert_eq!(effect.interval, 3);
                assert_eq!(effect.command_target, None);
                assert!(effect.command_id.is_none());
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn add_effect_records_command_target_metadata() {
        let state = empty_state();
        let mut target_map = ValueMap::new();
        target_map.insert("id".into(), Value::Int(42));
        let target = Value::Proplist(target_map.into_iter().collect());

        let (result, outcome) = with_object_host_context(|| {
            add_effect(&[
                Value::String("Glow".into()),
                state.clone(),
                Value::Int(120),
                Value::Int(2),
                target.clone(),
                Value::C4Id("FOOB".into()),
            ])
        });

        let value = result.expect("AddEffect succeeds");
        assert_eq!(value, Value::Int(1));
        assert_eq!(outcome.object.len(), 1);
        match &outcome.object[0] {
            EffectCommand::Add { effect, .. } => {
                assert_eq!(effect.command_target, Some(42));
                assert_eq!(effect.command_id.as_deref(), Some("FOOB"));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn set_gravity_records_physics_update() {
        let (result, delta) =
            with_physics_context(PhysicsSettings::default(), || set_gravity(&[Value::Int(5)]));
        let value = result.expect("SetGravity succeeds");
        assert_eq!(value, Value::Nil);
        assert_eq!(delta.gravity, Some(5));
    }

    #[test]
    fn set_gravity_clamps_bounds() {
        let (_, delta) = with_physics_context(PhysicsSettings::default(), || {
            set_gravity(&[Value::Int(400)])
        });
        assert_eq!(delta.gravity, Some(300));
        let (_, delta) = with_physics_context(PhysicsSettings::default(), || {
            set_gravity(&[Value::Int(-500)])
        });
        assert_eq!(delta.gravity, Some(-300));
    }

    #[test]
    fn get_gravity_returns_current_value() {
        let settings = PhysicsSettings::new(6, 20, -30);
        let (result, _) = with_physics_context(settings, || get_gravity(&[]));
        let value = result.expect("GetGravity succeeds");
        assert_eq!(value, Value::Int(6));
    }

    #[test]
    fn set_wind_records_environment_update() {
        let mut initial = EnvironmentSettings::new(5).with_wind_variation(4, 2_000);
        initial.wind_target = -20;
        let (result, delta) = with_environment_context(initial, 0, || {
            set_wind(&[Value::Int(75)])?;
            ENVIRONMENT_CONTEXT.with(|cell| {
                let context = cell.borrow();
                let settings = context
                    .as_ref()
                    .expect("environment context exists")
                    .settings
                    .borrow();
                assert_eq!((settings.wind, settings.wind_target), (75, 75));
                assert_eq!(settings.base_wind, 5, "scenario Wind.Std is unchanged");
            });
            get_wind(&[])
        });

        let value = result.expect("SetWind/GetWind succeeds");
        assert_eq!(value, Value::Int(75));
        assert_eq!(delta.wind, Some(75));

        let mut applied = initial;
        delta.apply(&mut applied);
        assert_eq!((applied.wind, applied.wind_target), (75, 75));
        assert_eq!(applied.base_wind, 5, "delta keeps scenario Wind.Std");
    }

    #[test]
    fn get_wind_positional_reads_tunnel_background() {
        // FnGetWind (C4Script.cpp:3001-3008): the global form returns
        // Weather.Wind; the positional form reads GBackWind — zero inside
        // tunnel-background (IFT) pixels (C4Wrappers.h:189-192).
        let mut landscape = Landscape::flat(32, 100);
        landscape.set_tunnel_column(5, vec![(0, 20)]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let (result, _) = with_environment_context(EnvironmentSettings::new(60), 0, || {
            let (inner, _) = with_effect_context(None, &[], world, 1, || {
                assert_eq!(get_wind(&[Value::Int(5), Value::Int(10)])?, Value::Int(0));
                assert_eq!(get_wind(&[Value::Int(6), Value::Int(10)])?, Value::Int(60));
                assert_eq!(
                    get_wind(&[Value::Nil, Value::Nil, Value::Bool(true)])?,
                    Value::Int(60)
                );
                Ok(Value::Nil)
            });
            inner
        });
        result.expect("GetWind positional succeeds");
    }

