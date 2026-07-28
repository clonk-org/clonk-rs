// Contiguous slice 1 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: host registration, object state, players.

    /// The idle object scope these tests hand to `with_effect_context`: one
    /// object at rest with no effects and full construction. Sites that need
    /// a different channel override it through a record update, so the
    /// shared defaults stay in one place.
    ///
    /// Vertices are a parameter rather than an overridable field because the
    /// constructor derives `shape_vertices` from them. Overriding `owner`
    /// must also set `controller`, which the constructor seeds from it
    /// (C4Object.cpp:162), and `construction` is clamped at zero.
    fn idle_object_context_with_vertices(vertices: &[ObjectVertex]) -> HostObjectContext<'_> {
        HostObjectContext::new(
            ObjectId::new(1),
            None,
            ObjectStatus::Normal,
            100,
            OWNER_NONE,
            Vector2::ZERO,
            Vector2::ZERO,
            &[],
            "Idle",
            0,
            0,
            ActionLibrary::default(),
            Direction::Left,
            CommandDirection::Stop,
            0,
            None,
            None,
            vertices,
            crate::FULL_CON,
        )
    }

    fn idle_object_context() -> HostObjectContext<'static> {
        idle_object_context_with_vertices(&[])
    }

    /// A world object carrying the scope defaults these tests share: alive
    /// and normal, Idle, unowned, full energy and construction, at rest at
    /// the origin with no vertices and no container. Sites state only the
    /// channel they exercise, through the type's own builders.
    fn fixture_world_object(id: ObjectId, definition: impl Into<String>) -> HostWorldObject {
        HostWorldObject::new(
            id,
            definition,
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
    }

    #[test]
    fn cpp_add_func_argument_extraction_canonicalizes_scalar_and_pointer_slots() {
        let raw_bool = Value::from_c4_bool_raw(2);
        let high_word_bool = Value::from_c4_bool_data_raw(1usize.checked_shl(32).unwrap_or(2));

        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::Int, &Value::Nil)
                .expect("nil extracts through Data.Int"),
            Value::Int(0)
        );
        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::Int, &Value::Bool(true))
                .expect("Bool shares Data.Int"),
            Value::Int(1)
        );
        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::Int, &raw_bool)
                .expect("raw Bool shares Data.Int"),
            Value::Int(2)
        );
        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::Bool, &Value::Nil)
                .expect("nil extracts as false"),
            Value::Bool(false)
        );
        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::Bool, &raw_bool)
                .expect("raw Bool extracts from its low word"),
            Value::Bool(true)
        );
        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::Bool, &high_word_bool)
                .expect("Bool extraction reads only Data.Int"),
            Value::Bool(usize::BITS <= 32)
        );

        for pointer in [
            Value::Object(7),
            Value::String(String::new().into()),
            Value::Array(Vec::new()),
            Value::Proplist(ValueMap::new()),
        ] {
            assert_eq!(
                extract_cpp_native_argument("Probe", 0, C4VType::Bool, &pointer)
                    .expect("nonnull pointer tags extract as true"),
                Value::Bool(true)
            );
        }
        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::Bool, &Value::Object(0))
                .expect("null object extracts as false"),
            Value::Bool(false)
        );
        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::C4Id, &Value::C4Id("NONE".into()),)
                .expect("zero C4ID extracts as null"),
            Value::Nil
        );
        assert_eq!(
            extract_cpp_native_argument("Probe", 0, C4VType::C4Object, &Value::Object(0))
                .expect("null object pointer extracts as null"),
            Value::Nil
        );
    }

    #[test]
    fn cpp_add_func_argument_extraction_preserves_optional_nil_and_private_tail() {
        assert_eq!(
            extract_cpp_native_argument("ModulateColor", 0, C4VType::Int, &Value::Nil)
                .expect("optional color remains nullopt"),
            Value::Nil
        );
        assert_eq!(
            extract_cpp_native_argument("CustomMessage", 5, C4VType::Int, &Value::Nil)
                .expect("optional custom-message color remains nullopt"),
            Value::Nil
        );
        assert_eq!(
            extract_cpp_native_argument("CustomMessage", 4, C4VType::Int, &Value::Nil)
                .expect("ordinary nil integer extracts as zero"),
            Value::Int(0)
        );
        assert_eq!(
            extract_cpp_native_arguments(
                "EffectVar",
                &[C4VType::Int, C4VType::C4Object, C4VType::Int],
                &[
                    Value::Bool(true),
                    Value::Nil,
                    Value::from_c4_bool_raw(2),
                    Value::String("private setter".into()),
                ],
            )
            .expect("EffectVar extraction succeeds"),
            vec![
                Value::Int(1),
                Value::Nil,
                Value::Int(2),
                Value::String("private setter".into()),
            ]
        );
    }

    #[test]
    fn cpp_add_func_adapter_runs_after_debugger_without_changing_generic_hosts() {
        const PROBE_TYPES: &[C4VType] = &[
            C4VType::Int,
            C4VType::Int,
            C4VType::Bool,
            C4VType::C4Id,
            C4VType::C4Object,
            C4VType::String,
            C4VType::Array,
            C4VType::Map,
        ];

        fn run_probe(wrapped: bool, values: &[Value]) -> (Value, Vec<Value>) {
            let mut engine = ScriptEngine::new();
            engine.register_host_function("Probe", |args| Ok(Value::Array(args.to_vec())));
            if wrapped {
                wrap_cpp_add_func_host_function(&mut engine, "Probe", PROBE_TYPES);
            }
            assert!(engine.set_host_function_parameter_types("Probe", PROBE_TYPES.iter().copied(),));
            engine
                .load_script(
                    "#strict 3\n\
                     func Run(a, b, c, d, e, f, g, h) {\n\
                         return Probe(a, b, c, d, e, f, g, h);\n\
                     }",
                )
                .expect("probe script compiles");

            let observed = Arc::new(Mutex::new(Vec::new()));
            let observed_by_hook = Arc::clone(&observed);
            engine.set_debugger_hooks(clonk_script::DebuggerHooks::new().with_on_call(
                move |name, args| {
                    if name == "Probe" {
                        *observed_by_hook.lock().expect("debugger capture lock") = args.to_vec();
                    }
                },
            ));
            let result = engine.call("Run", values).expect("probe succeeds");
            let debugger_args = observed.lock().expect("debugger capture lock").clone();
            (result, debugger_args)
        }

        let high_word_bool = Value::from_c4_bool_data_raw(1usize.checked_shl(32).unwrap_or(2));
        let vm_prepared = vec![
            Value::Nil,
            Value::from_c4_bool_raw(2),
            high_word_bool.clone(),
            Value::C4Id("NONE".into()),
            Value::Object(0),
            Value::Nil,
            Value::Nil,
            Value::Nil,
        ];

        // C4AulParSet copies host-supplied values through C4Value::Set before
        // the script frame is entered, so a manually supplied C4ID(0) reaches
        // Run (and then Probe) as canonical nil.
        let script_prepared = vec![
            Value::Nil,
            Value::from_c4_bool_raw(2),
            high_word_bool.clone(),
            Value::Nil,
            Value::Object(0),
            Value::Nil,
            Value::Nil,
            Value::Nil,
        ];

        let (wrapped_result, wrapped_debugger_args) = run_probe(true, &vm_prepared);
        assert_eq!(wrapped_debugger_args, script_prepared);
        assert_eq!(
            wrapped_result,
            Value::Array(vec![
                Value::Int(0),
                Value::Int(2),
                Value::Bool(usize::BITS <= 32),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ])
        );

        let (generic_result, generic_debugger_args) = run_probe(false, &vm_prepared);
        assert_eq!(generic_debugger_args, script_prepared);
        assert_eq!(generic_result, Value::Array(script_prepared));
    }

    #[test]
    fn cpp_add_func_extractors_are_installed_on_production_callbacks() {
        let mut engine = ScriptEngine::new();
        register_host_functions(&mut engine);

        assert_eq!(
            engine
                .call("Abs", &[Value::Bool(true)])
                .expect("Bool extracts through the native Int parameter"),
            Value::Int(1)
        );

        let high_word_bool = Value::from_c4_bool_data_raw(1usize.checked_shl(32).unwrap_or(2));
        assert_eq!(
            engine
                .call("Not", &[high_word_bool])
                .expect("native Bool extraction reads the low Data.Int word"),
            Value::Bool(usize::BITS > 32)
        );
    }

    #[test]
    fn cpp_native_registration_kinds_are_exhaustively_partitioned() {
        let cpp_backed = crate::native_function_parameters::native_function_parameter_entries()
            .filter(|(name, _)| {
                !crate::native_function_parameters::RUST_STANDIN_NATIVE_FUNCTIONS.contains(name)
            })
            .count();
        assert_eq!(
            cpp_backed,
            crate::native_function_parameters::CPP_BACKED_NATIVE_FUNCTION_COUNT
        );

        let add_func = crate::native_function_parameters::native_function_parameter_entries()
            .filter(|(name, _)| {
                !crate::native_function_parameters::RUST_STANDIN_NATIVE_FUNCTIONS.contains(name)
                    && !RAW_CPP_NATIVE_FUNCTIONS.contains(name)
            })
            .count();
        assert_eq!(add_func, 442);
        assert_eq!(add_func - REFERENCE_AWARE_CPP_NATIVE_FUNCTIONS.len(), 434);
    }

    #[test]
    fn native_host_parameter_conversion_respects_caller_strictness_and_rejects_maps_as_objects() {
        let mut legacy = ScriptEngine::new();
        register_host_functions(&mut legacy);
        legacy
            .load_script(
                "#strict 2\n\
                 func MapArgument(value) { return GetX(value); }\n\
                 func ZeroArgument() { return GetX(0); }",
            )
            .expect("legacy native conversion probe compiles");

        let map_error = legacy
            .call(
                "MapArgument",
                &[Value::Proplist(ValueMap::from([("id", Value::Int(1))]))],
            )
            .expect_err("a map never converts to C4Object");
        assert!(map_error
            .to_string()
            .contains("call to \"GetX\" parameter 1: got \"map\", but expected \"object\"!"));
        assert_eq!(
            legacy
                .call("ZeroArgument", &[])
                .expect("legacy zero eagerly becomes a null object"),
            Value::Nil
        );

        let mut strict3 = ScriptEngine::new();
        register_host_functions(&mut strict3);
        strict3
            .load_script(
                "#strict 3\n\
                 func MapArgument() { return GetX({ id = 1 }); }\n\
                 func ZeroArgument() { return GetX(0); }",
            )
            .expect("strict-3 native conversion probe compiles");
        assert!(strict3
            .call("MapArgument", &[])
            .expect_err("strict-3 map remains a map")
            .to_string()
            .contains("got \"map\", but expected \"object\""));
        assert!(strict3
            .call("ZeroArgument", &[])
            .expect_err("strict-3 typed zero remains an int")
            .to_string()
            .contains("got \"int\", but expected \"object\""));
    }

    #[test]
    fn queued_object_order_functions_remain_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<ObjectOrderFunction>();
        assert_send_sync::<ObjectOrderCommand>();
    }

    #[test]
    fn lazy_get_inserts_objects_in_canonical_index_order() {
        unsafe fn object(source: *const (), id: ObjectId) -> Option<(usize, HostWorldObject)> {
            // SAFETY: the test keeps the source vector at a stable address
            // until after its only provider-bearing context is dropped.
            let objects = unsafe { &*source.cast::<Vec<HostWorldObject>>() };
            objects
                .iter()
                .enumerate()
                .find(|(_, object)| object.id == id)
                .map(|(index, object)| (index, object.clone()))
        }

        unsafe fn objects(
            source: *const (),
            excluded: &HashSet<usize>,
        ) -> Vec<(usize, HostWorldObject)> {
            // SAFETY: see `object`; excluded entries are skipped before use.
            let objects = unsafe { &*source.cast::<Vec<HostWorldObject>>() };
            objects
                .iter()
                .enumerate()
                .filter(|(index, _)| !excluded.contains(index))
                .map(|(index, object)| (index, object.clone()))
                .collect()
        }

        unsafe fn landscape(_source: *const ()) -> Option<Landscape> {
            None
        }

        let source = vec![
            scenario_section_world_object(40, ObjectStatus::Normal),
            scenario_section_world_object(10, ObjectStatus::Normal),
            scenario_section_world_object(30, ObjectStatus::Normal),
            scenario_section_world_object(20, ObjectStatus::Normal),
        ];
        // SAFETY: `source` remains at a stable address and outlives `world`.
        let provider = unsafe {
            LazyHostWorldProvider::new(
                std::ptr::from_ref(&source).cast(),
                object,
                objects,
                landscape,
            )
        };
        let mut world = HostWorldContext::default().with_lazy_world_provider(provider);
        world.seed_object(2, source[2].clone());

        assert_eq!(
            world.object_store.borrow().order.as_slice(),
            &[ObjectId::new(30)]
        );
        for (id, expected) in [
            (20, vec![30, 20]),
            (40, vec![40, 30, 20]),
            (10, vec![40, 10, 30, 20]),
        ] {
            let id = ObjectId::new(id);
            assert_eq!(world.get(id).map(|object| object.id), Some(id));
            let expected = expected.into_iter().map(ObjectId::new).collect::<Vec<_>>();
            assert_eq!(
                world.object_store.borrow().order.as_slice(),
                expected.as_slice()
            );
        }
    }

    #[test]
    fn pending_solid_mask_negative_source_clamp_keeps_cpp_oob_pixels_solid() {
        // CheckSolidMaskRect rewrites (-1,-1,3,3) to (0,0,3,3) for this
        // 2x2 bitmap because its width/height clamp still uses the OLD -1
        // coordinates. The third column/row are therefore sampled out of
        // bounds. GetPixDw returns zero there, which C++'s inverted-alpha
        // IsPixTransparent/MaskPixel path classifies as solid. Row-zero's
        // out-of-bounds sample must not wrap into transparent row-one col-0.
        let pixels: Arc<[u8]> = vec![
            0, 0, 0, 0, 0, 0, 0, 255, // source row 0: clear, solid
            0, 0, 0, 0, 0, 0, 0, 0, // source row 1: clear, clear
        ]
        .into();
        let image = HostSolidMaskImage::new(2, 2, pixels);
        let (mask, solid) = image
            .mask_pixels(crate::DefinitionTargetRect::new(-1, -1, 3, 3, 4, 5))
            .expect("the legacy negative-source clamp retains a 3x3 mask");

        assert_eq!(mask, crate::DefinitionTargetRect::new(0, 0, 3, 3, 4, 5));
        assert_eq!(solid.as_slice(), &[0, 1, 1, 0, 0, 1, 1, 1, 1]);
    }

    const EXPECTED_HOST_FUNCTIONS: &[&str] = &[
        "AbortMessageBoard",
        "Abs",
        "ActIdle",
        "ActivateGameGoalMenu",
        "AddCommand",
        "AddEffect",
        "AddEvaluationData",
        "AddMenuItem",
        "AddMessage",
        "AddMsgBoardCmd",
        "AddVertex",
        "AdjustWalkRotation",
        "And",
        "Angle",
        "AnyContainer",
        "AppendCommand",
        "ArcCos",
        "ArcSin",
        "AssignVar",
        "AsyncRandom",
        "BitAnd",
        "BlastFree",
        "BlastObject",
        "BlastObjects",
        "BoundBy",
        "Bubble",
        "Buy",
        "C4Id",
        "Call",
        "CallMessageBoard",
        "CastAny",
        "CastBackParticles",
        "CastBool",
        "CastC4ID",
        "CastInt",
        "CastObjects",
        "CastPXS",
        "CastParticles",
        "ChangeDef",
        "ChangeEffect",
        "CheckEffect",
        "CheckEnergyNeedChain",
        "ClearLastPlrCom",
        "ClearMenuItems",
        "ClearParticles",
        "CloseMenu",
        "Collect",
        "ComponentAll",
        "ComposeContents",
        "Contained",
        "Contents",
        "ContentsCount",
        "Cos",
        "CreateArray",
        "CreateConstruction",
        "CreateContents",
        "CreateMenu",
        "CreateObject",
        "CreateParticle",
        "CreateScriptPlayer",
        "CrewMember",
        "CustomMessage",
        "DeathAnnounce",
        "DebugLog",
        "Dec",
        "DecVar",
        "DefinitionCall",
        "DigFree",
        "DigFreeRect",
        "Distance",
        "Div",
        "DoBreath",
        "DoCon",
        "DoCrewExp",
        "DoDamage",
        "DoEnergy",
        "DoHomebaseMaterial",
        "DoHomebaseProduction",
        "DoMagicEnergy",
        "DoScore",
        "DoScoreboardShow",
        "DrawDefMap",
        "DrawMap",
        "DrawMatChunks",
        "DrawMaterialQuad",
        "DrawVolcanoBranch",
        "EditCursor",
        "EffectCall",
        "EffectVar",
        "EliminatePlayer",
        "EnergyCheck",
        "Enter",
        "Equal",
        "ExecuteCommand",
        "Exit",
        "Explode",
        "Extinguish",
        "ExtractLiquid",
        "ExtractMaterialAmount",
        "FatalError",
        "FightWith",
        "FindBase",
        "FindConstructionSite",
        "FindContents",
        "FindObject",
        "FindObject2",
        "FindObjectOwner",
        "FindObjects",
        "FindOtherContents",
        "Find_AtPoint",
        "Find_Category",
        "Find_ID",
        "FinishCommand",
        "FlameConsumeMaterial",
        "Fling",
        "Format",
        "FrameCounter",
        "FreeRect",
        "FxFireInfo",
        "FxFireStart",
        "FxFireStop",
        "FxFireTimer",
        "GBackLiquid",
        "GBackSemiSolid",
        "GBackSky",
        "GBackSolid",
        "GainMissionAccess",
        "GameCall",
        "GameCallEx",
        "GameOver",
        "GetActMapVal",
        "GetActTime",
        "GetAction",
        "GetActionData",
        "GetActionTarget",
        "GetAlive",
        "GetBase",
        "GetBreath",
        "GetCaptain",
        "GetCategory",
        "GetChar",
        "GetClimate",
        "GetClrModulation",
        "GetColor",
        "GetColorDw",
        "GetComDir",
        "GetCommand",
        "GetComponent",
        "GetCon",
        "GetContact",
        "GetController",
        "GetCrew",
        "GetCrewCount",
        "GetCrewEnabled",
        "GetCrewExtraData",
        "GetCursor",
        "GetDamage",
        "GetDefBottom",
        "GetDefCoreVal",
        "GetDefinition",
        "GetDesc",
        "GetDir",
        "GetEffect",
        "GetEffectCount",
        "GetEnergy",
        "GetEntrance",
        "GetGravity",
        "GetHiRank",
        "GetHomebaseMaterial",
        "GetHomebaseProduction",
        "GetID",
        "GetIndexOf",
        "GetKeys",
        "GetKiller",
        "GetLeague",
        "GetLeagueProgressData",
        "GetLeagueScore",
        "GetLength",
        "GetMagicEnergy",
        "GetMass",
        "GetMatAdjust",
        "GetMaterial",
        "GetMaterialColor",
        "GetMaterialCount",
        "GetMaterialVal",
        "GetMaxPlayer",
        "GetMenu",
        "GetMenuSelection",
        "GetMissionAccess",
        "GetName",
        "GetNeededMatStr",
        "GetOCF",
        "GetObjHeight",
        "GetObjWidth",
        "GetObjectBlitMode",
        "GetObjectInfoCoreVal",
        "GetObjectLayer",
        "GetObjectStatus",
        "GetObjectVal",
        "GetOwner",
        "GetPath",
        "GetPhase",
        "GetPhysical",
        "GetPlayerByIndex",
        "GetPlayerCount",
        "GetPlayerID",
        "GetPlayerInfoCoreVal",
        "GetPlayerName",
        "GetPlayerTeam",
        "GetPlayerType",
        "GetPlayerVal",
        "GetPlrColorDw",
        "GetPlrControlName",
        "GetPlrDownDouble",
        "GetPlrExtraData",
        "GetPlrJumpAndRunControl",
        "GetPlrKnowledge",
        "GetPlrMagic",
        "GetPlrValue",
        "GetPlrValueGain",
        "GetPlrView",
        "GetPlrViewMode",
        "GetPortrait",
        "GetProcedure",
        "GetR",
        "GetRDir",
        "GetRank",
        "GetScenarioVal",
        "GetScore",
        "GetScoreboardData",
        "GetScoreboardString",
        "GetSeason",
        "GetSelectCount",
        "GetSkyAdjust",
        "GetSkyColor",
        "GetSystemTime",
        "GetTaggedPlayerName",
        "GetTeamByIndex",
        "GetTeamColor",
        "GetTeamConfig",
        "GetTeamCount",
        "GetTeamName",
        "GetTemperature",
        "GetTexture",
        "GetTime",
        "GetType",
        "GetUnusedOverlayID",
        "GetValue",
        "GetValues",
        "GetVertex",
        "GetVertexContact",
        "GetVertexNum",
        "GetViewCursor",
        "GetVisibility",
        "GetWealth",
        "GetWind",
        "GetX",
        "GetXDir",
        "GetY",
        "GetYDir",
        "GrabContents",
        "GrabObjectInfo",
        "GreaterThan",
        "HideSettlementScoreInEvaluation",
        "Hostile",
        "InLiquid",
        "Inc",
        "IncVar",
        "Incinerate",
        "IncinerateLandscape",
        "InitScenarioPlayer",
        "InsertMaterial",
        "Inside",
        "IsNetwork",
        "IsNewgfx",
        "IsRef",
        "Jump",
        "Kill",
        "LandscapeHeight",
        "LandscapeWidth",
        "LaunchEarthquake",
        "LaunchLightning",
        "LaunchVolcano",
        "LessThan",
        "LoadScenarioSection",
        "LocateFunc",
        "Log",
        "MakeCrewMember",
        "Material",
        "MaterialName",
        "Max",
        "Message",
        "Min",
        "Mod",
        "ModulateColor",
        "Mul",
        "Music",
        "MusicLevel",
        "NoContainer",
        "Not",
        "Object",
        "ObjectCall",
        "ObjectCount",
        "ObjectCount2",
        "ObjectDistance",
        "ObjectNumber",
        "ObjectSetAction",
        "OnFire",
        "OnMessageBoardAnswer",
        "Or",
        "PathFree",
        "PathFree2",
        "PauseGame",
        "PlaceAnimal",
        "PlaceVegetation",
        "PlayerMessage",
        "PlayerObjectCommand",
        "PlrMessage",
        "Pow",
        "PrivateCall",
        "ProtectedCall",
        "Punch",
        "PushParticles",
        "Random",
        "ReloadDef",
        "ReloadParticle",
        "RemoveEffect",
        "RemoveObject",
        "RemoveUnusedTexMapEntries",
        "RemoveVertex",
        "ResetGamma",
        "ResetPhysical",
        "Resort",
        "ResortObject",
        "ResortObjects",
        "SEqual",
        "ScoreboardCol",
        "ScriptCounter",
        "ScriptGo",
        "ScrollContents",
        "SelectCrew",
        "SelectMenuItem",
        "Sell",
        "Set",
        "SetAction",
        "SetActionData",
        "SetActionTargets",
        "SetAlive",
        "SetBridgeActionData",
        "SetCategory",
        "SetClimate",
        "SetClrModulation",
        "SetColor",
        "SetColorDw",
        "SetComDir",
        "SetCommand",
        "SetComponent",
        "SetContactDensity",
        "SetController",
        "SetCrewEnabled",
        "SetCrewExtraData",
        "SetCrewStatus",
        "SetCursor",
        "SetDir",
        "SetEntrance",
        "SetFilmView",
        "SetFoW",
        "SetGameSpeed",
        "SetGamma",
        "SetGraphics",
        "SetGravity",
        "SetHostility",
        "SetKiller",
        "SetLandscapePixel",
        "SetLeaguePerformance",
        "SetLeagueProgressData",
        "SetLength",
        "SetMass",
        "SetMatAdjust",
        "SetMaterialColor",
        "SetMaxPlayer",
        "SetMenuDecoration",
        "SetMenuSize",
        "SetMenuTextProgress",
        "SetName",
        "SetNextMission",
        "SetObjDrawTransform",
        "SetObjDrawTransform2",
        "SetObjectBlitMode",
        "SetObjectLayer",
        "SetObjectOrder",
        "SetObjectStatus",
        "SetOwner",
        "SetPhase",
        "SetPhysical",
        "SetPicture",
        "SetPlayList",
        "SetPlayerTeam",
        "SetPlrExtraData",
        "SetPlrKnowledge",
        "SetPlrMagic",
        "SetPlrShowCommand",
        "SetPlrShowControl",
        "SetPlrShowControlPos",
        "SetPlrView",
        "SetPlrViewRange",
        "SetPortrait",
        "SetPosition",
        "SetPreSend",
        "SetR",
        "SetRDir",
        "SetRestoreInfos",
        "SetScoreboardData",
        "SetSeason",
        "SetShape",
        "SetSkyAdjust",
        "SetSkyColor",
        "SetSkyFade",
        "SetSkyParallax",
        "SetSolidMask",
        "SetTemperature",
        "SetTextureIndex",
        "SetTransferZone",
        "SetVar",
        "SetVertex",
        "SetViewCursor",
        "SetViewOffset",
        "SetVisibility",
        "SetWealth",
        "SetWind",
        "SetXDir",
        "SetYDir",
        "ShakeFree",
        "ShakeObjects",
        "ShiftContents",
        "ShowInfo",
        "SimFlight",
        "Sin",
        "Smoke",
        "SortScoreboard",
        "Sound",
        "SoundLevel",
        "Split2Components",
        "Sqrt",
        "StartCallTrace",
        "StartScriptProfiler",
        "StopScriptProfiler",
        "Stuck",
        "Sub",
        "Sum",
        "SurrenderPlayer",
        "TestMessageBoard",
        "TrainPhysical",
        "UnselectCrew",
        "Value",
        "WildcardMatch",
        "goto",
    ];

    #[test]
    fn host_function_registration_matches_expected() {
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        let actual = engine.host_function_names();
        let expected: Vec<String> = EXPECTED_HOST_FUNCTIONS
            .iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn every_production_host_registration_carries_its_cpp_arity() {
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);

        let names = engine.host_function_names();
        assert_eq!(names.len(), 457);
        for name in names {
            assert_eq!(
                engine.host_function_parameter_count(&name),
                Some(cpp_native_parameter_count(&name)),
                "native arity drift for {name}"
            );
        }
    }

    #[test]
    fn cached_host_registration_overrides_remain_host_local() {
        let mut overridden = ScriptEngine::new();
        register_host_functions(&mut overridden);
        let mut untouched = ScriptEngine::new();
        register_host_functions(&mut untouched);

        overridden.register_host_function_with_arity("Abs", 1, |_| Ok(Value::Int(99)));
        assert!(overridden.set_host_function_parameter_types("Abs", [C4VType::Int]));
        overridden.register_constant("NO_OWNER", Value::Int(77));
        overridden
            .load_script("func ReadConstant() { return NO_OWNER; }")
            .expect("overridden constant probe compiles");
        untouched
            .load_script("func ReadConstant() { return NO_OWNER; }")
            .expect("cached constant probe compiles");

        assert_eq!(
            overridden
                .call("Abs", &[Value::Bool(true)])
                .expect("host-local override runs"),
            Value::Int(99)
        );
        assert_eq!(
            untouched
                .call("Abs", &[Value::Bool(true)])
                .expect("cached C++ native remains installed"),
            Value::Int(1)
        );
        assert_eq!(
            overridden
                .call("ReadConstant", &[])
                .expect("overridden constant resolves"),
            Value::Int(77)
        );
        assert_eq!(
            untouched
                .call("ReadConstant", &[])
                .expect("cached constant remains unchanged"),
            Value::Int(OWNER_NONE)
        );

        overridden.register_host_function("EmbeddingOnly", |_| Ok(Value::Int(55)));
        register_host_functions(&mut overridden);
        assert_eq!(
            overridden
                .call("Abs", &[Value::Bool(true)])
                .expect("reapplying the cache restores the builtin"),
            Value::Int(1)
        );
        assert_eq!(
            overridden
                .call("ReadConstant", &[])
                .expect("reapplying the cache restores the builtin constant"),
            Value::Int(OWNER_NONE)
        );
        assert_eq!(
            overridden
                .call("EmbeddingOnly", &[])
                .expect("unrelated embedding callback survives cache application"),
            Value::Int(55)
        );
    }

    #[test]
    fn get_system_time_reports_local_hour_and_rejects_sync_or_invalid_fields() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script("#strict 3\nfunc Probe(int field) { return GetSystemTime(field); }")
            .expect("GetSystemTime probe compiles");

        let query = |world, field| {
            let (result, _) = with_effect_context(None, &[], world, 1, || {
                script.call("Probe", &[Value::Int(field)])
            });
            result.expect("GetSystemTime executes")
        };

        let before = Local::now().hour() as i32;
        let local = query(HostWorldContext::default(), 4);
        let after = Local::now().hour() as i32;
        assert!(
            matches!(local, Value::Int(hour) if hour == before || hour == after),
            "expected local hour {before} or {after}, got {local:?}"
        );
        assert_eq!(
            query(HostWorldContext::default().with_control_sync_mode(true), 4,),
            Value::Nil
        );
        assert_eq!(query(HostWorldContext::default(), -1), Value::Nil);
        assert_eq!(query(HostWorldContext::default(), 8), Value::Nil);
    }

    #[test]
    fn fatal_error_aborts_with_cpp_user_error_framing() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func StringError() { FatalError(\"boom\"); return true; }\n\
                 func NilError() { FatalError(nil); return true; }\n\
                 func MissingError() { FatalError(); return true; }",
            )
            .expect("FatalError probes compile");

        for (function, expected) in [
            ("StringError", "runtime error: User error: boom"),
            ("NilError", "runtime error: User error: (no error)"),
            ("MissingError", "runtime error: User error: (no error)"),
        ] {
            assert_eq!(
                script
                    .call(function, &[])
                    .expect_err("FatalError must abort its caller")
                    .to_string(),
                expected
            );
        }
    }

    #[test]
    fn deprecated_get_color_stub_returns_zero_and_local_override_wins() {
        let mut builtin = clonk_script::Engine::new();
        register_host_functions(&mut builtin);
        builtin
            .load_script("#strict 2\nfunc Probe() { return GetColor(); }")
            .expect("builtin GetColor probe compiles");
        assert_eq!(
            builtin.call("Probe", &[]).expect("builtin GetColor runs"),
            Value::Int(0)
        );

        let mut overridden = clonk_script::Engine::new();
        register_host_functions(&mut overridden);
        overridden
            .load_script(
                "#strict 2\nfunc GetColor() { return 37; }\nfunc Probe() { return GetColor(); }",
            )
            .expect("local GetColor override compiles");
        assert_eq!(
            overridden
                .call("Probe", &[])
                .expect("local GetColor override runs"),
            Value::Int(37)
        );
    }

    fn scenario_section_world_object(id: u64, status: ObjectStatus) -> HostWorldObject {
        fixture_world_object(ObjectId::new(id), "TEST")
            .with_status(status)
    }

    #[test]
    fn load_scenario_section_rejects_missing_empty_and_unknown_names_without_commands() {
        assert_eq!(
            load_scenario_section(&[Value::String("Mountains".into())])
                .expect("a missing host context is a clean failure"),
            Value::Int(0)
        );

        let cases = [
            Vec::new(),
            vec![Value::Nil],
            vec![Value::String(String::new().into())],
            vec![Value::String("Unknown".into()), Value::Int(3)],
        ];
        for args in cases {
            let world = HostWorldContext::default().with_scenario_sections(["Main", "Mountains"]);
            let (result, outcome) =
                with_effect_context(None, &[], world, 1, || load_scenario_section(&args));
            assert_eq!(
                result.expect("invalid section is a clean failure"),
                Value::Int(0)
            );
            assert!(outcome.player_commands.is_empty());
        }
    }

    #[test]
    fn load_scenario_section_forwards_flags_and_captures_effective_inactive_ids() {
        let world = HostWorldContext::from_objects([
            scenario_section_world_object(1, ObjectStatus::Normal),
            scenario_section_world_object(2, ObjectStatus::Inactive),
            scenario_section_world_object(3, ObjectStatus::Normal),
        ])
        .with_scenario_sections(["Main", "Mountains"]);
        let (result, outcome) = with_object_host_context_with_world(world, || {
            set_object_status(&[Value::Int(ObjectStatus::Inactive.to_script_value())])?;
            load_scenario_section(&[Value::String("mOuNtAiNs".into()), Value::Int(3)])
        });

        assert_eq!(result.expect("known section is accepted"), Value::Int(1));
        match outcome.player_commands.as_slice() {
            [PlayerCommand::LoadScenarioSection {
                name,
                flags,
                preserve_ids,
            }] => {
                assert_eq!(name, "mOuNtAiNs");
                assert_eq!(*flags, 3);
                assert_eq!(preserve_ids, &[ObjectId::new(1), ObjectId::new(2)]);
            }
            other => panic!("unexpected section commands: {other:?}"),
        }
    }

    #[test]
    fn load_scenario_section_defaults_flags_to_zero() {
        let world = HostWorldContext::default().with_scenario_sections(["Main"]);
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            load_scenario_section(&[Value::String("Main".into())])
        });

        assert_eq!(result.expect("known section is accepted"), Value::Int(1));
        match outcome.player_commands.as_slice() {
            [PlayerCommand::LoadScenarioSection {
                flags,
                preserve_ids,
                ..
            }] => {
                assert_eq!(*flags, 0);
                assert!(preserve_ids.is_empty());
            }
            other => panic!("unexpected section commands: {other:?}"),
        }
    }

    #[test]
    fn create_script_player_maps_the_exact_cpp_player_info_request() {
        assert_eq!(
            script_player_extra_data(Some(&Value::C4Id("ABCDE".to_string())))
                .expect("long C4ID converts"),
            *b"ABCD",
            "C4Id keeps the first four bytes like C++"
        );
        assert_eq!(
            script_player_extra_data(Some(&Value::C4Id(clonk_script::c4_id_from_raw(123))))
                .expect("numeric C4ID converts"),
            *b"0123"
        );
        assert_eq!(
            script_player_extra_data(Some(&Value::C4Id(clonk_script::c4_id_from_raw(12345))))
                .expect("non-definition C4ID is accepted as NONE"),
            *b"NONE"
        );
        let error = script_player_extra_data(Some(&Value::Int(10_000)))
            .expect_err("out-of-range int fails the implicit C4ID conversion");
        assert!(error.message().contains("expected C4ID"));
        let updates = Rc::new(RefCell::new(Vec::new()));
        let world = HostWorldContext::default().with_control_host(true, Rc::clone(&updates));
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            create_script_player(&[
                Value::String("Bot".to_string().into()),
                Value::Int(0xff44_5566_u32 as i32),
                Value::Int(2),
                Value::Int(15),
                Value::C4Id("__AI".to_string()),
            ])
        });

        assert_eq!(result.expect("builtin succeeds"), Value::Bool(true));
        let updates = updates.borrow();
        assert_eq!(updates.len(), 1);
        let request = &updates[0];
        assert_eq!(request.client_id, 0);
        assert_eq!(request.flags, crate::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS);
        assert_eq!(request.players.len(), 1);
        let player = &request.players[0];
        assert_eq!(player.name.as_bytes(), b"Bot");
        assert_eq!(player.id, 0);
        assert_eq!(player.player_type, crate::PLAYER_INFO_TYPE_SCRIPT);
        assert_eq!(
            (player.color, player.original_color),
            (0x0044_5566, 0x0044_5566)
        );
        assert_eq!(player.team, 2);
        assert_eq!(player.extra_data, *b"__AI");
        assert_eq!(
            player.flags,
            crate::PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
                | crate::PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                | crate::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK
                | crate::PLAYER_INFO_FLAG_INVISIBLE
        );
    }

    #[test]
    fn create_script_player_validates_name_before_the_control_host_gate() {
        for name in [Value::Nil, Value::String(String::new().into())] {
            let updates = Rc::new(RefCell::new(Vec::new()));
            let world = HostWorldContext::default().with_control_host(true, Rc::clone(&updates));
            let (result, _) = with_effect_context(None, &[], world, 1, || {
                create_script_player(std::slice::from_ref(&name))
            });
            assert_eq!(result.expect("invalid name is handled"), Value::Bool(false));
            assert!(updates.borrow().is_empty());
        }

        let updates = Rc::new(RefCell::new(Vec::new()));
        let world = HostWorldContext::default().with_control_host(false, Rc::clone(&updates));
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            create_script_player(&[Value::String("Remote Bot".to_string().into())])
        });
        assert_eq!(result.expect("peer no-op succeeds"), Value::Bool(true));
        assert!(updates.borrow().is_empty());
    }

    #[test]
    fn set_pre_send_preserves_cpp_negative_and_offline_results() {
        // Typed conversion precedes FnSetPreSend; its body then rejects
        // negatives and makes every nonnegative local-control call a no-op.
        for (world, argument, expected) in [
            (
                HostWorldContext::default(),
                Value::Int(-1),
                Value::Bool(false),
            ),
            (
                HostWorldContext::default().with_network_control_mode(true),
                Value::Int(-1),
                Value::Bool(false),
            ),
            (
                HostWorldContext::default(),
                Value::Int(0),
                Value::Bool(true),
            ),
            (
                HostWorldContext::default(),
                Value::Int(30),
                Value::Bool(true),
            ),
        ] {
            let (result, _) = with_effect_context(None, &[], world, 1, || {
                set_pre_send(&[argument, Value::Nil])
            });
            assert_eq!(
                result.expect("offline/negative SetPreSend succeeds"),
                expected
            );
        }
    }

    #[test]
    fn network_set_pre_send_enqueues_normalized_local_requests() {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let world = HostWorldContext::default()
            .with_network_control_mode(true)
            .with_network_target_fps_requests(Rc::clone(&requests));
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                set_pre_send(&[Value::Int(0)]).expect("zero target succeeds"),
                Value::Bool(true)
            );
            set_pre_send(&[Value::Int(76), Value::String("Client A?i*".into())])
        });
        assert_eq!(
            result.expect("network SetPreSend succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            requests.borrow().as_slice(),
            [
                crate::NetworkTargetFpsRequest {
                    target_fps: 38,
                    client_pattern: None,
                },
                crate::NetworkTargetFpsRequest {
                    target_fps: 76,
                    client_pattern: Some("Client A?i*".into()),
                },
            ]
        );
    }

    #[test]
    fn find_category_builds_the_cpp_system_criterion() {
        // The C++ engine loads this exact wrapper from
        // System.c4g/FindObject.c:61-63; C4FO_Category is 22
        // (C4FindObject.h:25-40). An omitted typed-int argument is zero.
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script("#strict 2\nfunc Probe() { return [Find_Category(32), Find_Category()]; }")
            .expect("Find_Category probe compiles");

        assert_eq!(
            engine.call("Probe", &[]).expect("Find_Category succeeds"),
            Value::Array(vec![
                Value::Array(vec![Value::Int(22), Value::Int(32)]),
                Value::Array(vec![Value::Int(22), Value::Int(0)]),
            ])
        );
    }

    #[test]
    fn find_id_builds_the_cpp_system_criterion_with_a_typed_c4id() {
        // System.c4g/FindObject.c:33-35 returns [C4FO_ID, idDef], preserving
        // the C4ID-typed definition parameter; C4FO_ID is 20
        // (C4FindObject.h:26-48).
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script("#strict 2\nfunc Probe() { return Find_ID(_AR1); }")
            .expect("Find_ID probe compiles");

        assert_eq!(
            engine.call("Probe", &[]).expect("Find_ID succeeds"),
            Value::Array(vec![Value::Int(20), Value::C4Id("_AR1".into())])
        );
    }

    #[test]
    fn find_id_applies_the_strict_cpp_id_parameter_conversion() {
        // C4Value's strict Int -> C4ID conversion accepts exactly 0..=9999;
        // String -> C4ID is always an error (C4Value.cpp:469-478,502-561).
        // A direct call follows C4AulFunc::Exec's legacy eager-falsy path.
        assert_eq!(
            find_id(&[]).expect("nil id converts"),
            Value::Array(vec![Value::Int(20), Value::Nil])
        );
        for value in [Value::Bool(false), Value::Object(0)] {
            assert_eq!(
                find_id(&[value]).expect("raw-falsy id becomes nil"),
                Value::Array(vec![Value::Int(20), Value::Nil])
            );
        }
        assert_eq!(
            find_id(&[Value::Int(0)]).expect("raw-zero id eagerly becomes nil"),
            Value::Array(vec![Value::Int(20), Value::Nil])
        );
        for (raw, id) in [(1, "0001"), (1337, "1337"), (9999, "9999")] {
            assert_eq!(
                find_id(&[Value::Int(raw)]).expect("small integer id converts"),
                Value::Array(vec![Value::Int(20), Value::C4Id(id.into())])
            );
        }
        for rejected in [
            Value::Int(-1),
            Value::Int(10_000),
            Value::String("FLAG".into()),
        ] {
            let error = find_id(&[rejected]).expect_err("strict C4ID conversion rejects value");
            assert!(error.message().contains("expected C4ID"));
        }
    }

    #[test]
    fn find_at_point_adds_the_calling_object_position() {
        // System.c4g/FindObject.c:41-43 adds GetX()/GetY() to the typed-int
        // offsets. FnGetX/FnGetY use cthr->Obj and return nil without one
        // (C4Script.cpp:1198-1202,1293-1297), so global calls use origin zero.
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script("#strict 2\nfunc Probe(x, y) { return Find_AtPoint(x, y); }")
            .expect("Find_AtPoint probe compiles");
        let object = HostObjectContext {
            position: Vector2::new(320, -50),
            ..idle_object_context()
        };
        let definition_commanded_carrier = object.clone();
        let (local, _) =
            with_effect_context(Some(object), &[], HostWorldContext::default(), 1, || {
                engine.call("Probe", &[Value::Int(45), Value::Int(194)])
            });
        assert_eq!(
            local.expect("object-relative Find_AtPoint succeeds"),
            Value::Array(vec![Value::Int(11), Value::Int(365), Value::Int(144)])
        );

        let (definition_commanded, _) = with_effect_context_with_state_and_definition(
            Some(definition_commanded_carrier),
            Some(DefinitionId::from("PROB")),
            None,
            &[],
            HostWorldContext::default(),
            1,
            false,
            || engine.call("Probe", &[Value::Int(-5), Value::Int(7)]),
        );
        assert_eq!(
            definition_commanded.expect("definition-commanded Find_AtPoint succeeds"),
            Value::Array(vec![Value::Int(11), Value::Int(-5), Value::Int(7)]),
            "the affected carrier is not cthr->Obj (C4Effect.cpp:342-345)"
        );

        let (global, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            engine.call("Probe", &[Value::Int(-5), Value::Int(7)])
        });
        assert_eq!(
            global.expect("global Find_AtPoint succeeds"),
            Value::Array(vec![Value::Int(11), Value::Int(-5), Value::Int(7)])
        );
    }

    #[test]
    fn scoreboard_col_retags_the_c4id_payload_as_an_integer() {
        // C4AulDefCastFunc returns the unchanged C4ID data with an Int tag
        // (C4Script.cpp:6184-6195, registered as ScoreboardCol at :7042).
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script("#strict 2\nfunc Probe() { return ScoreboardCol(RACE); }")
            .expect("scoreboard probe compiles");

        assert_eq!(
            engine.call("Probe", &[]).expect("ScoreboardCol succeeds"),
            Value::Int(i32::from_le_bytes(*b"RACE"))
        );
        assert_eq!(
            scoreboard_col(&[Value::C4Id(clonk_script::c4_id_from_raw(12345))])
                .expect("tagged C4ID payload converts"),
            Value::Int(12345)
        );
    }

    #[test]
    fn scoreboard_getters_return_cpp_missing_and_present_values() {
        // FnGetScoreboardString/Data reverse the script row/column pair and
        // return nil/0 for a missing key (C4Script.cpp:5886-5894;
        // C4Scoreboard.cpp:177-197).
        let scoreboard = Rc::new(RefCell::new(ScoreboardState::default()));
        let world = HostWorldContext::default().with_scoreboard(Rc::clone(&scoreboard));
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                "#strict 2\nfunc Probe() {\n\
                 SetScoreboardData(7, 1234, \"42\", 42);\n\
                 return [GetScoreboardString(7, 1234),\n\
                         GetScoreboardData(7, 1234),\n\
                         GetScoreboardString(8, 1234),\n\
                         GetScoreboardData(7, 5678)];\n}",
            )
            .expect("scoreboard getter probe compiles");

        let (result, _) = with_effect_context(None, &[], world, 1, || engine.call("Probe", &[]));
        assert_eq!(
            result.expect("scoreboard getters succeed"),
            Value::Array(vec![
                Value::String("42".into()),
                Value::Int(42),
                Value::Nil,
                Value::Int(0),
            ])
        );
    }

    #[test]
    fn scoreboard_set_cell_invalidation_is_ordered_and_sort_keeps_cached_geometry() {
        let scoreboard = Rc::new(RefCell::new(ScoreboardState::default()));
        let presentations = Rc::new(RefCell::new(ScoreboardPresentationSink::default()));
        presentations.borrow_mut().begin_runtime_capture();
        let world = HostWorldContext::default()
            .with_scoreboard(Rc::clone(&scoreboard))
            .with_scoreboard_presentations(Rc::clone(&presentations));

        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let args = [
                Value::Int(-1),
                Value::Int(-1),
                Value::String("Scores".into()),
                Value::Int(0),
            ];
            set_scoreboard_data(&args)?;
            set_scoreboard_data(&args)?;
            assert_eq!(
                sort_scoreboard(&[Value::Int(-1), Value::Bool(false)])?,
                Value::Bool(true)
            );
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("scoreboard mutations succeed");

        assert_eq!(
            presentations.borrow().layout_revision(),
            2,
            "each SetCell invalidates, including an unchanged assignment"
        );
    }

    #[test]
    fn do_scoreboard_show_targets_one_based_players_and_updates_the_refcount() {
        // FnDoScoreboardShow looks up iForPlr-1, returns false only for a
        // missing requested player, and otherwise passes iChange to the
        // scoreboard refcount (C4Script.cpp:5896-5908;
        // C4Scoreboard.cpp:234-256).
        let scoreboard = Rc::new(RefCell::new(ScoreboardState::default()));
        let presentations = Rc::new(RefCell::new(ScoreboardPresentationSink::default()));
        presentations.borrow_mut().begin_runtime_capture();
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            [PlayerState {
                id: 0,
                ..PlayerState::default()
            }],
        )
        .with_scoreboard(Rc::clone(&scoreboard))
        .with_scoreboard_presentations(Rc::clone(&presentations));
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script(
                "#strict 2\nfunc Probe() {\n\
                 SetScoreboardData(-1, -1, \"Scores\", -1);\n\
                 return [DoScoreboardShow(2, 1),\n\
                         DoScoreboardShow(1, 2),\n\
                         DoScoreboardShow(-1)];\n}",
            )
            .expect("scoreboard show probe compiles");

        let (result, _) = with_effect_context(None, &[], world, 1, || engine.call("Probe", &[]));
        assert_eq!(
            result.expect("scoreboard show calls succeed"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(true),
            ])
        );
        assert_eq!(scoreboard.borrow().show_count(), 1);
        assert!(scoreboard.borrow().should_be_shown());
        let requests = presentations.borrow_mut().drain();
        assert_eq!(requests.len(), 2, "missing-player calls emit no request");
        assert_eq!(requests[0].show_count, 2);
        assert_eq!(requests[1].show_count, 1);
        assert!(requests
            .iter()
            .all(|request| { request.layout_revision == 1 && request.title_widget_present }));
    }

    #[test]
    fn do_scoreboard_show_remote_player_is_a_sync_safe_no_op() {
        // Existing remote players return true without mutating iDlgShow
        // (C4Script.cpp:5900-5905).
        let scoreboard = Rc::new(RefCell::new(ScoreboardState::default()));
        let presentations = Rc::new(RefCell::new(ScoreboardPresentationSink::default()));
        presentations.borrow_mut().begin_runtime_capture();
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            [PlayerState {
                id: 0,
                ..PlayerState::default()
            }],
        )
        .with_local_players([])
        .with_scoreboard(Rc::clone(&scoreboard))
        .with_scoreboard_presentations(Rc::clone(&presentations));
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            do_scoreboard_show(&[Value::Int(3), Value::Int(1)])
        });

        assert_eq!(
            result.expect("remote show call succeeds"),
            Value::Bool(true)
        );
        assert_eq!(scoreboard.borrow().show_count(), 0);
        assert!(presentations.borrow_mut().drain().is_empty());
    }

    #[test]
    fn add_evaluation_data_validates_info_ids_and_preserves_append_order() {
        let player = PlayerState {
            id: 7,
            player_info_id: 41,
            ..PlayerState::default()
        };
        let world =
            HostWorldContext::from_objects_with_players(Vec::<HostWorldObject>::new(), [player])
                .with_player_info_ids([57]);
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(add_evaluation_data(&[])?, Value::Bool(false));
            assert_eq!(
                add_evaluation_data(&[Value::String(String::new().into()), Value::Int(41)])?,
                Value::Bool(false)
            );
            assert_eq!(
                add_evaluation_data(&[Value::String("unknown".into()), Value::Int(99)])?,
                Value::Bool(false)
            );
            assert_eq!(
                add_evaluation_data(&[Value::String("global".into())])?,
                Value::Bool(true)
            );
            assert_eq!(
                add_evaluation_data(&[Value::String("kills".into()), Value::Int(41)])?,
                Value::Bool(true)
            );
            assert_eq!(
                add_evaluation_data(&[Value::String("kills".into()), Value::Int(41)])?,
                Value::Bool(true)
            );
            assert_eq!(
                add_evaluation_data(&[Value::String("retired".into()), Value::Int(57)])?,
                Value::Bool(true)
            );
            assert_eq!(
                add_evaluation_data(&[Value::String("   ".into()), Value::Int(41)])?,
                Value::Bool(true),
                "whitespace-only strings are nonempty in C++"
            );
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("AddEvaluationData calls succeed");
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [
                PlayerCommand::AddEvaluationData { player_info_id: 0, text },
                PlayerCommand::AddEvaluationData { player_info_id: 41, text: first },
                PlayerCommand::AddEvaluationData { player_info_id: 41, text: duplicate },
                PlayerCommand::AddEvaluationData { player_info_id: 57, text: retired },
                PlayerCommand::AddEvaluationData { player_info_id: 41, text: whitespace },
            ] if text == "global"
                && first == "kills"
                && duplicate == "kills"
                && retired == "retired"
                && whitespace == "   "
        ));
    }

    #[test]
    fn round_results_rows_do_not_resurrect_removed_player_info_ids() {
        let mut engine = crate::Engine::with_seed(0);
        engine
            .round_results
            .players
            .push(crate::RoundResultsPlayerState {
                player_info_id: 57,
                ..Default::default()
            });
        let world = engine.host_world_context();
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            add_evaluation_data(&[Value::String("stale".into()), Value::Int(57)])
        });
        assert_eq!(result.expect("lookup completes"), Value::Bool(false));
        assert!(outcome.player_commands.is_empty());
    }

    #[test]
    fn hide_settlement_score_in_evaluation_returns_nil_and_orders_writes() {
        let script = r#"#strict
public func Apply()
{
    return [
        HideSettlementScoreInEvaluation(0),
        HideSettlementScoreInEvaluation(-1),
        HideSettlementScoreInEvaluation("", 123)
    ];
}

public func UseDefault()
{
    return HideSettlementScoreInEvaluation();
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(crate::SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(caller_index, "Apply", Vec::new())
                .expect("hide sequence runs"),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil]),
            "the void host function returns nil for every call"
        );
        assert!(
            engine.round_results.hide_settlement_score,
            "ordered writes leave the final truthy empty-string conversion visible"
        );
        assert!(
            crate::EngineState::from_json_str(
                &engine
                    .capture_state()
                    .to_json_string()
                    .expect("hidden score state serializes"),
            )
            .expect("hidden score state deserializes")
            .round_results
            .hide_settlement_score,
            "C4RoundResults serializes the flag"
        );

        assert_eq!(
            engine
                .call_object_function(caller_index, "UseDefault", Vec::new())
                .expect("default hide call runs"),
            Value::Nil
        );
        assert!(
            !engine.round_results.hide_settlement_score,
            "an omitted typed bool is nil-filled and converts to false"
        );
    }

    #[test]
    fn set_league_performance_gates_ids_orders_writes_and_matches_cpp_save() {
        let error = set_league_performance(&[Value::String("not an int".into())])
            .expect_err("typed score conversion precedes the league gate");
        assert!(error.message().contains("expected int"));

        let script = r#"#strict
public func Apply(int active, int zero_score, int retired, int runtime_number)
{
    return [
        SetLeaguePerformance(),
        SetLeaguePerformance(5, 0),
        SetLeaguePerformance(-3, 0),
        SetLeaguePerformance(100, active),
        SetLeaguePerformance(0, zero_score),
        SetLeaguePerformance(7, retired),
        SetLeaguePerformance(101, active),
        SetLeaguePerformance(9, 99),
        SetLeaguePerformance(8, -1),
        SetLeaguePerformance(6, runtime_number)
    ];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine
            .register_player(crate::PlayerConfig::new(7, "Active").with_player_info_id(41))
            .expect("active player registers");
        engine
            .register_player(crate::PlayerConfig::new(8, "Zero score").with_player_info_id(43))
            .expect("zero-score player registers");
        engine
            .round_results
            .players
            .push(crate::RoundResultsPlayerState {
                player_info_id: 57,
                custom_evaluation_strings: "retained info".into(),
                ..crate::RoundResultsPlayerState::default()
            });
        engine.replace_player_info_league_progress_data([(41, None), (43, None), (57, None)]);
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(crate::SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let args = vec![
            Value::Int(41),
            Value::Int(43),
            Value::Int(57),
            Value::Int(7),
        ];

        assert_eq!(
            engine
                .call_object_function(caller_index, "Apply", args.clone())
                .expect("non-league probe runs"),
            Value::Array(vec![Value::Bool(false); 10])
        );
        assert_eq!(engine.round_results.league_performance, 0);
        assert_eq!(engine.round_results.players.len(), 1);
        assert_eq!(engine.round_results.players[0].league_performance, 0);

        engine.set_league_game(true);
        assert_eq!(
            engine
                .call_object_function(caller_index, "Apply", args)
                .expect("league probe runs"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(false),
            ])
        );
        assert_eq!(engine.round_results.league_performance, -3);
        assert_eq!(
            engine
                .round_results
                .players
                .iter()
                .map(|player| (player.player_info_id, player.league_performance))
                .collect::<Vec<_>>(),
            vec![(57, 7), (41, 101), (43, 0)],
            "existing rows keep position, new rows append, and zero scores still create"
        );
        engine.round_results.players[0].league_progress_data = Some(Vec::new());

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.round_results.players[0].league_performance, 7);
        assert_eq!(snapshot.round_results.players[1].league_performance, 101);
        assert_eq!(
            snapshot.round_results.players[0].league_progress_data,
            Some(Vec::new()),
            "live round results retain allocated-empty progress"
        );
        let from_snapshot = crate::EngineState::from_snapshot(&snapshot);
        assert_eq!(from_snapshot.round_results.league_performance, -3);
        assert!(from_snapshot
            .round_results
            .players
            .iter()
            .all(|player| player.league_performance == 0));
        assert_eq!(
            from_snapshot.round_results.players[0].league_progress_data, None,
            "C4RoundResults save collapses empty progress to its null default"
        );
        let captured = engine.capture_state();
        assert_eq!(captured.round_results.league_performance, -3);
        assert!(captured
            .round_results
            .players
            .iter()
            .all(|player| player.league_performance == 0));
        let restored = crate::EngineState::from_json_str(
            &captured
                .to_json_string()
                .expect("league performance state serializes"),
        )
        .expect("league performance state deserializes");
        assert_eq!(restored.round_results.league_performance, -3);
        assert!(restored
            .round_results
            .players
            .iter()
            .all(|player| player.league_performance == 0));
    }

    #[test]
    fn get_league_progress_data_matches_typed_gate_id_and_byte_semantics() {
        let error = get_league_progress_data(&[Value::String("not an int".into())])
            .expect_err("typed ID conversion precedes the league gate");
        assert!(error.message().contains("expected int"));
        assert_eq!(
            get_league_progress_data(&[Value::Int(41)]).expect("missing context is non-league"),
            Value::Nil
        );

        let progress_data = BTreeMap::from([
            (1, Some(b"bool-id".to_vec())),
            (41, None),
            (43, Some(Vec::new())),
            (57, Some(vec![b'A', 0xff])),
        ]);
        let non_league = HostWorldContext::default()
            .with_league_progress_data(Rc::new(Vec::new()), Rc::new(progress_data.clone()))
            .with_team_runtime_options(TeamConfiguration::default(), true);
        let (result, outcome) = with_effect_context(None, &[], non_league, 1, || {
            get_league_progress_data(&[Value::Int(57)])
        });
        assert_eq!(result.expect("non-league lookup succeeds"), Value::Nil);
        assert!(outcome.player_commands.is_empty());

        let league = HostWorldContext::default()
            .with_league_progress_data(Rc::new(b"LeagueName".to_vec()), Rc::new(progress_data));
        let (result, outcome) = with_effect_context(None, &[], league, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_league_progress_data(&[])?,
                get_league_progress_data(&[Value::Int(0)])?,
                get_league_progress_data(&[Value::Int(-1)])?,
                get_league_progress_data(&[Value::Int(7)])?,
                get_league_progress_data(&[Value::Int(41)])?,
                get_league_progress_data(&[Value::Int(43), Value::Int(999)])?,
                get_league_progress_data(&[Value::Int(57)])?,
                get_league_progress_data(&[Value::Bool(true)])?,
            ]))
        });
        assert_eq!(
            result.expect("league lookups succeed"),
            Value::Array(vec![
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::String(String::new().into()),
                Value::String(clonk_script::c4_string_from_bytes(&[b'A', 0xff]).into()),
                Value::String("bool-id".into()),
            ])
        );
        assert!(
            outcome.player_commands.is_empty(),
            "getter emits no controls"
        );
    }

    #[test]
    fn get_league_score_matches_typed_id_lookup_and_signed_score_semantics() {
        let error = get_league_score(&[Value::String("not an int".into())])
            .expect_err("typed ID conversion precedes context and lookup gates");
        assert!(error.message().contains("GetLeagueScore"));
        assert!(error.message().contains("expected integer"));
        assert_eq!(
            get_league_score(&[Value::Int(43)]).expect("missing context returns nil"),
            Value::Nil
        );

        let world = HostWorldContext::default().with_league_scores(Rc::new(BTreeMap::from([
            (-1, 700),
            (0, 600),
            (1, 17),
            (41, 0),
            (43, 238),
            (57, -19),
        ])));
        assert!(
            !world.league_name_configured() && !world.league_game,
            "GetLeagueScore must not require either league gate"
        );
        assert_eq!(
            world.player_info_league_scores.as_ref(),
            &BTreeMap::from([(1, 17), (43, 238), (57, -19)]),
            "zero is represented by a known ID without a score override"
        );

        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_league_score(&[])?,
                get_league_score(&[Value::Int(-1)])?,
                get_league_score(&[Value::Int(0)])?,
                get_league_score(&[Value::Int(99)])?,
                get_league_score(&[Value::Int(41)])?,
                get_league_score(&[Value::Int(43)])?,
                get_league_score(&[Value::Int(57)])?,
                get_league_score(&[Value::Bool(true)])?,
            ]))
        });
        assert_eq!(
            result.expect("league-score lookups succeed"),
            Value::Array(vec![
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(0),
                Value::Int(238),
                Value::Int(-19),
                Value::Int(17),
            ])
        );
        assert!(
            outcome.player_commands.is_empty(),
            "getter emits no controls"
        );
    }

    #[test]
    fn get_league_score_uses_engine_projection_and_survives_exact_save_boundaries() {
        let mut engine = crate::Engine::with_seed(0);
        for (number, player_info_id) in [(7, 41), (8, 43)] {
            engine
                .register_player(
                    crate::PlayerConfig::new(number, format!("Player {number}"))
                        .with_player_info_id(player_info_id),
                )
                .expect("player registers");
        }
        engine.replace_player_info_league_progress_data([(41, None), (43, None), (57, None)]);
        engine.replace_player_info_league_scores([(41, 238), (43, 0), (57, -19)]);

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.player_info_league_scores,
            BTreeMap::from([(41, 238), (57, -19)]),
            "score zero remains the sparse serialized default"
        );
        let (live, _) = with_effect_context(None, &[], engine.host_world_context(), 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_league_score(&[Value::Int(41)])?,
                get_league_score(&[Value::Int(43)])?,
                get_league_score(&[Value::Int(57)])?,
            ]))
        });
        assert_eq!(
            live.expect("live engine projection is queryable"),
            Value::Array(vec![Value::Int(238), Value::Int(0), Value::Int(-19)])
        );

        let state = crate::EngineState::from_snapshot(&snapshot);
        assert_eq!(
            state.player_info_league_scores.as_ref(),
            Some(&BTreeMap::from([(41, 238)])),
            "exact saves retain joined infos and discard unjoined retained rows"
        );
        let mut restored = crate::Engine::with_seed(1);
        restored.restore_state(&state).expect("state restores");
        let (after_restore, _) =
            with_effect_context(None, &[], restored.host_world_context(), 1, || {
                Ok::<_, RuntimeError>(Value::Array(vec![
                    get_league_score(&[Value::Int(41)])?,
                    get_league_score(&[Value::Int(43)])?,
                    get_league_score(&[Value::Int(57)])?,
                ]))
            });
        assert_eq!(
            after_restore.expect("restored engine projection is queryable"),
            Value::Array(vec![Value::Int(238), Value::Int(0), Value::Nil])
        );
    }

    #[test]
    fn set_league_progress_data_preserves_native_types_bytes_and_lifecycle() {
        let strict_script = r#"#strict 3
public func RoundTrip(int id, string data)
{
    var before = GetLeagueProgressData(id);
    var set_data = SetLeagueProgressData(data, id);
    var after_data = GetLeagueProgressData(id);
    var set_empty = SetLeagueProgressData("", id);
    var after_empty = GetLeagueProgressData(id);
    var clear = SetLeagueProgressData(nil, id);
    var after_clear = GetLeagueProgressData(id);
    var restore = SetLeagueProgressData(data, id);
    return [before, set_data, after_data, set_empty, after_empty,
            clear, after_clear, restore, GetLeagueProgressData(id)];
}

public func StrictZero(int id)
{
    return SetLeagueProgressData(0, id);
}

public func SetOnce(int id, string data)
{
    return [SetLeagueProgressData(data, id), GetLeagueProgressData(id)];
}
"#;
        let legacy_script = r#"#strict
public func ClearWithZero(int id)
{
    return [SetLeagueProgressData(0, id), GetLeagueProgressData(id)];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine.set_league_name(b"League".to_vec());
        engine
            .register_player(crate::PlayerConfig::new(7, "Player").with_player_info_id(41))
            .expect("player registers");
        engine
            .register_definition(
                crate::Definition::from_script("CALL", "Strict caller", strict_script)
                    .expect("strict caller compiles"),
            )
            .expect("strict caller registers");
        engine
            .register_definition(
                crate::Definition::from_script("OLDC", "Legacy caller", legacy_script)
                    .expect("legacy caller compiles"),
            )
            .expect("legacy caller registers");
        let strict = engine
            .spawn_object(crate::SpawnConfig::new("CALL"))
            .expect("strict caller spawns");
        let legacy = engine
            .spawn_object(crate::SpawnConfig::new("OLDC"))
            .expect("legacy caller spawns");
        let strict_index = engine
            .find_object_index(strict)
            .expect("strict caller exists");
        let raw = clonk_script::c4_string_from_bytes(&[b'A', 0xff]);

        assert_eq!(
            engine
                .call_object_function(
                    strict_index,
                    "RoundTrip",
                    vec![Value::Int(41), Value::String(raw.clone().into())],
                )
                .expect("strict round trip runs"),
            Value::Array(vec![
                Value::Nil,
                Value::Bool(true),
                Value::String(raw.clone().into()),
                Value::Bool(true),
                Value::String(String::new().into()),
                Value::Bool(true),
                Value::Nil,
                Value::Bool(true),
                Value::String(raw.clone().into()),
            ])
        );
        assert_eq!(
            engine.snapshot().player_info_league_progress_data.get(&41),
            Some(&Some(vec![b'A', 0xff]))
        );
        assert_eq!(
            engine.take_player_info_league_progress_updates(),
            vec![
                (41, Some(vec![b'A', 0xff])),
                (41, Some(Vec::new())),
                (41, None),
                (41, Some(vec![b'A', 0xff])),
            ]
        );

        assert_eq!(
            engine
                .call_object_function(
                    strict_index,
                    "SetOnce",
                    vec![Value::Int(41), Value::String("lvl2".into())],
                )
                .expect("known league player accepts progress data"),
            Value::Array(vec![Value::Bool(true), Value::String("lvl2".into())])
        );
        assert_eq!(
            engine.take_player_info_league_progress_updates(),
            vec![(41, Some(b"lvl2".to_vec()))]
        );
        assert_eq!(
            engine
                .call_object_function(
                    strict_index,
                    "SetOnce",
                    vec![Value::Int(999), Value::String("lvl2".into())],
                )
                .expect("unknown player-info ID is a false result"),
            Value::Array(vec![Value::Bool(false), Value::Nil])
        );
        assert!(engine.take_player_info_league_progress_updates().is_empty());

        let _error = engine
            .call_object_function(strict_index, "StrictZero", vec![Value::Int(41)])
            .expect_err("strict-3 typed zero is not a C4String pointer");
        assert!(engine.take_player_info_league_progress_updates().is_empty());

        let legacy_index = engine
            .find_object_index(legacy)
            .expect("legacy caller exists");
        assert_eq!(
            engine
                .call_object_function(legacy_index, "ClearWithZero", vec![Value::Int(41)])
                .expect("pre-strict-3 zero eagerly converts to nil"),
            Value::Array(vec![Value::Bool(true), Value::Nil])
        );
        assert_eq!(
            engine.take_player_info_league_progress_updates(),
            vec![(41, None)]
        );

        engine.set_league_name(Vec::new());
        assert_eq!(
            engine
                .call_object_function(
                    strict_index,
                    "SetOnce",
                    vec![Value::Int(41), Value::String("lvl2".into())],
                )
                .expect("non-league setter is a false result"),
            Value::Array(vec![Value::Bool(false), Value::Nil])
        );
        assert!(engine.take_player_info_league_progress_updates().is_empty());
        assert_eq!(
            engine
                .call_object_function(
                    strict_index,
                    "RoundTrip",
                    vec![Value::Int(41), Value::String(raw.into())],
                )
                .expect("non-league calls remain typed but gated"),
            Value::Array(vec![
                Value::Nil,
                Value::Bool(false),
                Value::Nil,
                Value::Bool(false),
                Value::Nil,
                Value::Bool(false),
                Value::Nil,
                Value::Bool(false),
                Value::Nil,
            ])
        );
        assert!(engine.take_player_info_league_progress_updates().is_empty());
    }

    #[test]
    fn get_league_returns_exact_indexed_parameter_sections() {
        let world = HostWorldContext::default()
            .with_league_progress_data(Rc::new(b"Alpha;;Beta;".to_vec()), Rc::new(BTreeMap::new()));
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_league(&[])?,
                get_league(&[Value::Bool(true)])?,
                get_league(&[Value::Int(2)])?,
                get_league(&[Value::Int(3)])?,
                get_league(&[Value::Int(4)])?,
                get_league(&[Value::Int(-1)])?,
            ]))
        });
        assert_eq!(
            result.expect("league section lookup succeeds"),
            Value::Array(vec![
                Value::String("Alpha".into()),
                Value::Nil,
                Value::String("Beta".into()),
                Value::Nil,
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn non_league_progress_projection_preserves_identity_and_snapshot_restore() {
        let mut engine = crate::Engine::with_seed(0);
        for (number, player_info_id) in [(0, 41), (1, 43), (2, 57)] {
            engine
                .register_player(
                    crate::PlayerConfig::new(number, format!("Player {number}"))
                        .with_player_info_id(player_info_id),
                )
                .expect("snapshot player registers");
        }
        engine.replace_player_info_league_progress_data([
            (41, None),
            (43, Some(Vec::new())),
            (57, Some(b"latent".to_vec())),
        ]);

        assert_eq!(
            engine.snapshot().player_info_league_progress_data,
            BTreeMap::from([
                (41, None),
                (43, Some(Vec::new())),
                (57, Some(b"latent".to_vec())),
            ]),
            "resumable snapshots preserve every retained player-info row"
        );
        let snapshot = engine.snapshot();
        let mut restored = crate::Engine::with_seed(1);
        restored
            .restore_snapshot(&snapshot)
            .expect("snapshot restores");
        let mut expected_after_save = snapshot.player_info_league_progress_data.clone();
        expected_after_save.insert(41, Some(Vec::new()));
        assert_eq!(
            restored.snapshot().player_info_league_progress_data,
            expected_after_save,
            "exact save/load applies C4PlayerInfo's allocated-empty default"
        );

        let world = engine.host_world_context();
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            add_evaluation_data(&[Value::String("retained".into()), Value::Int(41)])
        });
        assert_eq!(
            result.expect("retained player-info lookup completes"),
            Value::Bool(true),
            "non-league retained rows remain live PlayerInfo identities"
        );
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::AddEvaluationData {
                player_info_id: 41,
                text
            }] if text == "retained"
        ));

        engine.set_league_name(b"LeagueName".to_vec());
        assert_eq!(
            engine.snapshot().player_info_league_progress_data,
            BTreeMap::from([
                (41, None),
                (43, Some(Vec::new())),
                (57, Some(b"latent".to_vec())),
            ]),
            "the live retained rows survive enabling the league-name gate"
        );
    }

    #[test]
    fn get_league_progress_data_survives_snapshot_and_engine_state_round_trips() {
        let script = r#"#strict
public func Probe()
{
    var binary = GetLeagueProgressData(59);
    return [
        GetLeagueProgressData(41),
        GetLeagueProgressData(43),
        GetLeagueProgressData(57),
        GetLeagueProgressData(7),
        binary,
        GetLength(binary),
        GetChar(binary, 0),
        GetChar(binary, 1),
        binary[0],
        binary[1],
        binary[-1]
    ];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine.set_league_game(true);
        engine.set_league_name(b"LeagueName\0ignored".to_vec());
        engine.replace_player_info_league_progress_data([
            (41, Some(b"active".to_vec())),
            (43, Some(Vec::new())),
            (57, Some(vec![b'X', 0xfe])),
            (59, Some(vec![b'Y', 0xfd, 0, b'Z'])),
        ]);
        engine
            .register_player(crate::PlayerConfig::new(7, "Active").with_player_info_id(41))
            .expect("active player registers");
        engine
            .register_player(crate::PlayerConfig::new(8, "Empty").with_player_info_id(43))
            .expect("empty-progress player registers");
        engine
            .register_player(crate::PlayerConfig::new(9, "Binary").with_player_info_id(59))
            .expect("binary-progress player registers");

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.league_name, b"LeagueName");
        assert_eq!(
            snapshot.player_info_league_progress_data.get(&57),
            Some(&Some(vec![b'X', 0xfe]))
        );
        let from_snapshot = crate::EngineState::from_snapshot(&snapshot);
        assert_eq!(
            from_snapshot.league_name.as_deref(),
            Some(b"LeagueName".as_slice())
        );
        assert_eq!(
            from_snapshot
                .player_info_league_progress_data
                .as_ref()
                .and_then(|data| data.get(&43)),
            Some(&Some(Vec::new()))
        );
        assert!(
            !from_snapshot
                .player_info_league_progress_data
                .as_ref()
                .expect("snapshot projection is present")
                .contains_key(&57),
            "exact saves discard retained/unjoined player-info rows"
        );
        assert_eq!(
            from_snapshot
                .player_info_league_progress_data
                .as_ref()
                .and_then(|data| data.get(&59)),
            Some(&Some(vec![b'Y', 0xfd])),
            "engine boundaries truncate impossible interior-NUL tails"
        );

        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("league progress state serializes");
        let state = crate::EngineState::from_json_str(&encoded)
            .expect("league progress state deserializes");
        let mut restored = crate::Engine::with_seed(1);
        restored
            .restore_state(&state)
            .expect("league progress state restores");
        restored
            .register_definition(
                crate::Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = restored
            .spawn_object(crate::SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = restored.find_object_index(caller).expect("caller exists");
        assert_eq!(
            restored
                .call_object_function(caller_index, "Probe", Vec::new())
                .expect("restored getter runs"),
            Value::Array(vec![
                Value::String("active".into()),
                Value::String(String::new().into()),
                Value::Nil,
                Value::Nil,
                Value::String(clonk_script::c4_string_from_bytes(&[b'Y', 0xfd]).into()),
                Value::Int(2),
                Value::Int(i32::from(b'Y')),
                Value::Int(0xfd),
                Value::String("Y".into()),
                Value::String(clonk_script::c4_string_from_bytes(&[0xfd]).into()),
                Value::String(clonk_script::c4_string_from_bytes(&[0xfd]).into()),
            ])
        );
    }

    #[test]
    fn get_league_progress_data_from_join_info_is_visible_to_preinitialize_player() {
        let scenario = r#"
global func PreInitializePlayer(int player)
{
    CreateObject(MARK, GetLength(GetLeagueProgressData(41)), GetLeagueScore(41), player);
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        engine.set_landscape(crate::Landscape::flat(64, 48));
        engine.set_league_name(b"LeagueName".to_vec());
        engine
            .register_definition(
                crate::Definition::from_script("MARK", "Marker", "").expect("marker compiles"),
            )
            .expect("marker registers");
        engine
            .install_scenario_script_with_convention("Scenario", scenario, true)
            .expect("scenario installs");

        let info = crate::ControlPlayerInfoEntry {
            name: crate::LegacyCString::from_bytes(b"Player".to_vec()).expect("valid name"),
            id: 41,
            league_score: 123,
            league_progress_data: crate::LegacyCString::from_bytes(b"join-data".to_vec())
                .expect("valid progress data"),
            ..Default::default()
        };
        let config = crate::JoinPlayerConfig {
            name: "Player".into(),
            player_info_id: info.id,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x00ff_ffff,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        };
        let player = engine
            .join_player_with_info(config, &info)
            .expect("player joins")
            .number();

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.player_info_league_progress_data.get(&41),
            Some(&Some(b"join-data".to_vec()))
        );
        assert_eq!(snapshot.player_info_league_scores.get(&41), Some(&123));
        let marker = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "MARK" && object.owner == player)
            .expect("PreInitializePlayer creates the marker");
        assert_eq!(
            marker.position.x, 9,
            "getter is visible before ScenarioInit"
        );
        assert_eq!(
            marker.position.y, 123,
            "league score from the joined PlayerInfo is visible before ScenarioInit"
        );
    }

    #[test]
    fn set_object_order_is_available_to_legacy_scripts() {
        // FnSetObjectOrder is registered by C4Script.cpp:6970. Elevator
        // Initialize calls it before SetAction (Elevator.c4d/Script.c:13),
        // so a missing host aborts the rest of Initialize.
        let mut engine = clonk_script::Engine::new();
        register_host_functions(&mut engine);
        engine
            .load_script("global func Probe(target) { return SetObjectOrder(target); }")
            .expect("SetObjectOrder probe compiles");

        let result = engine.call("Probe", &[Value::Object(2)]);
        assert!(
            result.is_ok(),
            "SetObjectOrder must be registered: {result:?}"
        );
    }

    #[test]
    fn set_object_order_queues_cpp_pairs_and_rejects_invalid_pairs() {
        // FnSetObjectOrder (C4Script.cpp:5090-5111): nil pSortObj defaults
        // to the caller, nil pObjBeforeOrAfter and self-pairs return false,
        // and a valid request records fSortAfter without sorting inline.
        let world = HostWorldContext::from_objects(vec![fixture_world_object(
            ObjectId::new(2),
            "Dummy")],
        );
        let (result, outcome) = with_object_host_context_with_world(world, || {
            assert_eq!(
                set_object_order(&[Value::Object(2), Value::Nil, Value::Bool(true)])?,
                Value::Bool(true)
            );
            assert_eq!(set_object_order(&[Value::Object(1)])?, Value::Bool(false));
            assert_eq!(set_object_order(&[])?, Value::Bool(false));
            assert_eq!(set_object_order(&[Value::Object(999)])?, Value::Bool(false));
            Ok::<_, RuntimeError>(())
        });

        result.expect("SetObjectOrder calls succeed");
        assert_eq!(
            outcome.object_order_commands,
            [ObjectOrderCommand::SetRelative {
                relative_to: ObjectId::new(2),
                object: ObjectId::new(1),
                after: true,
            }]
        );
    }

    fn resort_order_world_object(id: u64, definition: &str) -> HostWorldObject {
        fixture_world_object(ObjectId::new(id), definition)
    }

    #[test]
    fn order_func_resorts_capture_caller_owner_after_change_def_and_cpp_defaults() {
        let definition_a = DefinitionId::from("ADEF");
        let definition_b = DefinitionId::from("BDEF");
        let mut script_a = ScriptEngine::new();
        register_host_functions(&mut script_a);
        script_a
            .load_script(
                r#"
                func NearFirst(object first, object second) { return 0; }
                func FarFirst(object first, object second) { return 0; }
                func Queue(object explicit_target) {
                    ChangeDef(BDEF);
                    ResortObjects("NearFirst");
                    ResortObject("FarFirst");
                    ResortObjects("NearFirst", 8);
                    ResortObject("NearFirst", explicit_target);
                    return true;
                }
                "#,
            )
            .expect("definition A order functions compile");
        let script_a = Arc::new(script_a);

        // B deliberately has same-named but different functions. Resolving
        // through the object's effective definition after ChangeDef would
        // capture this Arc instead of the suspended Queue function's owner.
        let mut script_b = ScriptEngine::new();
        script_b
            .load_script(
                r#"
                func NearFirst(object first, object second) { return 21; }
                func FarFirst(object first, object second) { return 22; }
                "#,
            )
            .expect("definition B order functions compile");
        let script_b = Arc::new(script_b);

        let world = HostWorldContext::from_objects(vec![
            resort_order_world_object(1, &definition_a),
            resort_order_world_object(2, &definition_a),
        ])
        .with_definition_metadata(Rc::new(HashMap::from([
            (definition_a.clone(), DefinitionMetadata::default()),
            (definition_b.clone(), DefinitionMetadata::default()),
        ])))
        .with_definition_scripts(HashMap::from([
            (definition_a.clone(), Arc::clone(&script_a)),
            (definition_b, Arc::clone(&script_b)),
        ]));

        let (result, outcome) = with_object_host_context_with_world(world, || {
            script_a
                .call("Queue", &[Value::Object(2)])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });
        assert_eq!(
            result.expect("definition-local resorts queue after ChangeDef"),
            Value::Bool(true)
        );

        let [ObjectOrderCommand::OrderFuncAll {
            order: first,
            category: default_category,
        }, ObjectOrderCommand::OrderFuncObject {
            order: second,
            object: default_object,
        }, ObjectOrderCommand::OrderFuncAll {
            order: third,
            category: explicit_category,
        }, ObjectOrderCommand::OrderFuncObject {
            order: fourth,
            object: explicit_object,
        }] = outcome.object_order_commands.as_slice()
        else {
            panic!(
                "unexpected order-function queue: {:?}",
                outcome.object_order_commands
            );
        };
        assert_eq!(*default_category, CATEGORY_SORT_LIMIT);
        assert_eq!(*default_object, ObjectId::new(1));
        assert_eq!(*explicit_category, 8);
        assert_eq!(*explicit_object, ObjectId::new(2));
        assert_eq!(first.function, "NearFirst");
        assert_eq!(second.function, "FarFirst");
        assert_eq!(third.function, "NearFirst");
        assert_eq!(fourth.function, "NearFirst");
        for order in [first, second, third, fourth] {
            assert_eq!(order.host_identity, script_a.host_identity());
            assert_ne!(order.host_identity, script_b.host_identity());
            assert_eq!(order.script_name, definition_a);
            assert_eq!(order.definition_context.as_ref(), Some(&definition_a));
        }
        // Outcome vectors retain call chronology; ExecuteResorts consumes this
        // batch in reverse, reproducing ResortProc head insertion.
        assert_eq!(
            outcome
                .object_order_commands
                .iter()
                .rev()
                .map(|command| match command {
                    ObjectOrderCommand::OrderFuncAll { order, .. }
                    | ObjectOrderCommand::OrderFuncObject { order, .. } => order.function.as_str(),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>(),
            ["NearFirst", "NearFirst", "FarFirst", "NearFirst"]
        );
    }

    #[test]
    fn order_func_resorts_use_scenario_scope_without_definition_context() {
        let mut scenario_script = ScriptEngine::new();
        register_host_functions(&mut scenario_script);
        scenario_script
            .load_script(
                r#"
                func ScenarioOrder(object first, object second) { return 0; }
                func QueueScenario(object target) {
                    ResortObjects("ScenarioOrder", -1);
                    ResortObject("ScenarioOrder", target);
                    return true;
                }
                "#,
            )
            .expect("scenario order function compiles");
        let scenario_script = Arc::new(scenario_script);
        let world = HostWorldContext::from_objects(vec![resort_order_world_object(2, "SORT")])
            .with_scenario_script(Some(Arc::clone(&scenario_script)));

        let (result, outcome) = with_effect_context(None, &[], world, 3, || {
            scenario_script
                .call("QueueScenario", &[Value::Object(2)])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });
        assert_eq!(
            result.expect("scenario-local resorts queue"),
            Value::Bool(true)
        );
        assert_eq!(outcome.object_order_commands.len(), 2);

        for command in &outcome.object_order_commands {
            let order = match command {
                ObjectOrderCommand::OrderFuncAll { order, category } => {
                    assert_eq!(*category, -1);
                    order
                }
                ObjectOrderCommand::OrderFuncObject { order, object } => {
                    assert_eq!(*object, ObjectId::new(2));
                    order
                }
                _ => panic!("unexpected command: {command:?}"),
            };
            assert_eq!(order.host_identity, scenario_script.host_identity());
            assert_eq!(order.script_name, "Game.Script");
            assert_eq!(order.definition_context, None);
            assert_eq!(order.function, "ScenarioOrder");
        }
    }

    #[test]
    fn order_func_resorts_match_cpp_validation_and_missing_function_error() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                func Known(object first, object second) { return 0; }
                func MissingAll() { return ResortObjects("Missing"); }
                func MissingObject(object target) { return ResortObject("Missing", target); }
                func EmptyName() { return ResortObjects(""); }
                func MissingObjectWithoutTarget() { return ResortObject("Missing"); }
                func KnownInvalidTarget(object target) { return ResortObject("Known", target); }
                "#,
            )
            .expect("known order function compiles");
        let script = Arc::new(script);
        let world = HostWorldContext::from_objects(vec![resort_order_world_object(2, "SORT")])
            .with_scenario_script(Some(Arc::clone(&script)));

        let (result, outcome) = with_effect_context(None, &[], world, 3, || {
            // A native entry without a suspended script frame has no
            // cthr->Caller, even when a scenario script is attached.
            assert_eq!(resort_objects(&[])?, Value::Bool(false));
            assert_eq!(resort_objects(&[Value::Nil])?, Value::Bool(false));
            assert_eq!(
                resort_objects(&[Value::String("Missing".into())])?,
                Value::Bool(false)
            );
            assert_eq!(
                resort_object(&[Value::String("Known".into()), Value::Object(2)])?,
                Value::Bool(false)
            );

            assert_eq!(
                script
                    .call("MissingObjectWithoutTarget", &[])
                    .map_err(|error| RuntimeError::new(error.to_string()))?,
                Value::Bool(false)
            );
            assert_eq!(
                script
                    .call("KnownInvalidTarget", &[Value::Object(999)])
                    .map_err(|error| RuntimeError::new(error.to_string()))?,
                Value::Bool(false)
            );

            let all_error = script
                .call("MissingAll", &[])
                .expect_err("missing whole-list function must throw");
            assert_eq!(
                all_error.to_string(),
                "runtime error: ResortObjects: Resort function Missing not found"
            );
            let object_error = script
                .call("MissingObject", &[Value::Object(2)])
                .expect_err("missing single-object function must throw");
            assert_eq!(
                object_error.to_string(),
                "runtime error: ResortObjects: Resort function Missing not found"
            );
            let empty_error = script
                .call("EmptyName", &[])
                .expect_err("empty names are looked up rather than treated as nil");
            assert_eq!(
                empty_error.to_string(),
                "runtime error: ResortObjects: Resort function  not found"
            );
            Ok::<_, RuntimeError>(())
        });
        result.expect("validation probes complete");
        assert!(outcome.object_order_commands.is_empty());
    }

    #[test]
    fn order_func_resorts_allow_global_fallback_only_for_global_callers() {
        let mut global_script = ScriptEngine::new();
        global_script
            .load_script("global func GlobalOrder(object first, object second) { return 0; }")
            .expect("engine-global order function compiles");
        let mut globals = global_script.functions().clone();

        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                global func QueueGlobal() { return ResortObjects("GlobalOrder"); }
                global func OwnGlobalOrder(object first, object second) { return 0; }
                func QueueLocal() { return ResortObjects("GlobalOrder"); }
                func QueueLocalOwn() { return ResortObjects("OwnGlobalOrder"); }
                "#,
            )
            .expect("global/local caller probes compile");
        // Global declarations are engine-owned; install QueueGlobal in the
        // shared table as the linker would, rather than calling an own named
        // entry that C++ represents only through an unnamed FnLink.
        globals.extend(
            script
                .global_access_functions()
                .map(|(name, function)| (name.clone(), function.clone())),
        );
        script.set_global_functions(Some(Arc::new(globals)));
        let script = Arc::new(script);
        let world = HostWorldContext::default().with_scenario_script(Some(Arc::clone(&script)));

        let (result, outcome) = with_effect_context(None, &[], world, 3, || {
            assert_eq!(
                script
                    .call("QueueGlobal", &[])
                    .map_err(|error| RuntimeError::new(error.to_string()))?,
                Value::Bool(true)
            );
            let local_error = script
                .call("QueueLocal", &[])
                .expect_err("local caller must not search engine globals");
            assert_eq!(
                local_error.to_string(),
                "runtime error: ResortObjects: Resort function GlobalOrder not found"
            );
            let own_global_error = script
                .call("QueueLocalOwn", &[])
                .expect_err("an unnamed own global FnLink is not a local SFunc");
            assert_eq!(
                own_global_error.to_string(),
                "runtime error: ResortObjects: Resort function OwnGlobalOrder not found"
            );
            Ok::<_, RuntimeError>(())
        });
        result.expect("global/local caller probes complete");
        let [ObjectOrderCommand::OrderFuncAll { order, category }] =
            outcome.object_order_commands.as_slice()
        else {
            panic!(
                "unexpected global order queue: {:?}",
                outcome.object_order_commands
            );
        };
        assert_eq!(order.host_identity, script.host_identity());
        assert_eq!(order.function, "GlobalOrder");
        assert_eq!(*category, CATEGORY_SORT_LIMIT);
    }

    #[test]
    fn order_func_global_fallback_uses_the_declaring_link_host_across_definitions() {
        for destination_has_cmp in [false, true] {
            let definition_a = DefinitionId::from("ADEF");
            let definition_b = DefinitionId::from("BDEF");

            let mut script_a = ScriptEngine::new();
            script_a
                .load_script(
                    r#"
                    global func Queue() { return ResortObjects("Cmp"); }
                    func Cmp(object first, object second) { return -11; }
                    "#,
                )
                .expect("declaring definition compiles");
            let globals = Arc::new(
                script_a
                    .global_access_functions()
                    .map(|(name, function)| (name.clone(), function.clone()))
                    .collect::<HashMap<_, _>>(),
            );
            script_a.set_global_functions(Some(Arc::clone(&globals)));
            let script_a = Arc::new(script_a);

            let mut script_b = ScriptEngine::new();
            register_host_functions(&mut script_b);
            if destination_has_cmp {
                script_b
                    .load_script("func Cmp(object first, object second) { return 22; }")
                    .expect("destination comparator compiles");
            }
            script_b.set_global_functions(Some(globals));
            let script_b = Arc::new(script_b);

            let world =
                HostWorldContext::from_objects(vec![resort_order_world_object(1, &definition_b)])
                    .with_definition_scripts(HashMap::from([
                        (definition_a.clone(), Arc::clone(&script_a)),
                        (definition_b, Arc::clone(&script_b)),
                    ]));
            let (result, outcome) = with_object_host_context_with_world(world, || {
                script_b
                    .call("Queue", &[])
                    .map_err(|error| RuntimeError::new(error.to_string()))
            });
            assert_eq!(
                result.expect("cross-host global caller queues its declaring comparator"),
                Value::Bool(true),
                "destination_has_cmp={destination_has_cmp}"
            );

            let [ObjectOrderCommand::OrderFuncAll { order, category }] =
                outcome.object_order_commands.as_slice()
            else {
                panic!(
                    "unexpected cross-host queue: {:?}",
                    outcome.object_order_commands
                );
            };
            assert_eq!(order.host_identity, script_a.host_identity());
            assert_ne!(order.host_identity, script_b.host_identity());
            assert_eq!(order.script_name, definition_a);
            assert_eq!(order.definition_context.as_ref(), Some(&definition_a));
            assert_eq!(order.function, "Cmp");
            assert_eq!(*category, CATEGORY_SORT_LIMIT);
        }
    }
