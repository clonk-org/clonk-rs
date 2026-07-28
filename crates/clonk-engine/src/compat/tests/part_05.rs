// Contiguous slice 5 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: players, landscape, effects.

    #[test]
    fn draw_material_quad_resolves_and_queues_global_vertices() {
        // FnDrawMaterialQuad passes its material string, four GLOBAL points,
        // and fSub straight to C4Landscape::DrawQuad (C4Script.cpp:5111-5115).
        // DrawQuad resolves GetIndexMatTex before entering PrepareChange
        // (C4Landscape.cpp:2448-2466), so the host can return true now and
        // defer the same resolved operation to the engine fold.
        let args = [
            Value::String("Water".to_string().into()),
            Value::Int(1),
            Value::Int(2),
            Value::Int(4),
            Value::Int(2),
            Value::Int(4),
            Value::Int(5),
            Value::Int(1),
            Value::Int(5),
            Value::Bool(true),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], draw_material_quad_world(), 1, || {
                draw_material_quad(&args)
            });

        assert_eq!(
            result.expect("DrawMaterialQuad succeeds"),
            Value::Bool(true)
        );
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DrawMaterialQuad {
                material_texture,
                vertices,
                ift,
            } => {
                assert_eq!(material_texture, "Water");
                assert_eq!(
                    *vertices,
                    [
                        Vector2::new(1, 2),
                        Vector2::new(4, 2),
                        Vector2::new(4, 5),
                        Vector2::new(1, 5),
                    ]
                );
                assert!(*ift);
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }
    }

    #[test]
    fn draw_material_quad_returns_false_for_unresolved_material() {
        // After an explicit pair fails, GetIndexMatTex checks the ORIGINAL
        // full string as a material name (C4Texture.cpp:346-369), so
        // `Water-Missing` does not fall back to Water's DefaultMatTex.
        // DrawQuad returns false before PrepareChange (C4Landscape.cpp:
        // 2450-2452), so no deferred write may be queued.
        let args = [
            Value::String("Water-Missing".to_string().into()),
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Bool(false),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], draw_material_quad_world(), 1, || {
                draw_material_quad(&args)
            });

        assert_eq!(
            result.expect("invalid material is not an error"),
            Value::Bool(false)
        );
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn void_host_functions_return_nil_while_preserving_side_effects() {
        // `C4AulEngineFunc<void>::Exec` returns C4VNull after invoking the
        // native (C4Script.cpp:6136-6166). Exercise the registered script
        // boundary so both the exposed nil and its falsiness stay pinned.
        let (result, outcome) = with_object_host_context(|| {
            let mut script = ScriptEngine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    r#"
                    #strict 3
                    func Probe() {
                        var debug = DebugLog("void host probe");
                        var sound = SoundLevel("Wind", 40);
                        var go = ScriptGo(true);
                        var shake = ShakeFree(10, 20, 3);
                        var dig = DigFree(30, 40, 5, true);
                        var rect = DigFreeRect(50, 60, 7, 8, false);
                        if (debug) return 1;
                        if (sound) return 2;
                        if (go) return 3;
                        if (shake) return 4;
                        if (dig) return 5;
                        if (rect) return 6;
                        return [debug, sound, go, shake, dig, rect];
                    }
                    "#,
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("void host probes succeed"),
            Value::Array(vec![Value::Nil; 6])
        );
        assert_eq!(outcome.script_go, Some(true));
        assert_eq!(
            outcome.audio.events,
            vec![AudioCommand::SetSoundVolume {
                name: "Wind".into(),
                target: None,
                volume: 40,
            }]
        );
        assert!(matches!(
            outcome.landscape.as_slice(),
            [
                LandscapeOperation::ShakeCircle {
                    center: Vector2 { x: 10, y: 20 },
                    radius: 3,
                },
                LandscapeOperation::DigCirclePreviewed {
                    center: Vector2 { x: 30, y: 40 },
                    radius: 5,
                },
                LandscapeOperation::DigRectPreviewed {
                    origin: Vector2 { x: 50, y: 60 },
                    width: 7,
                    height: 8,
                },
            ]
        ));

        let (no_op_result, no_op_outcome) = with_object_host_context(|| {
            let mut script = ScriptEngine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    r#"
                    #strict 3
                    func Probe() {
                        var sound = SoundLevel();
                        var go = ScriptGo();
                        var shake = ShakeFree();
                        var dig = DigFree(0, 0, -1);
                        var rect = DigFreeRect(0, 0, 0, 1);
                        if (sound) return 1;
                        if (go) return 2;
                        if (shake) return 3;
                        if (dig) return 4;
                        if (rect) return 5;
                        return [sound, go, shake, dig, rect];
                    }
                    "#,
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            no_op_result.expect("void host no-op probes succeed"),
            Value::Array(vec![Value::Nil; 5])
        );
        assert_eq!(no_op_outcome.script_go, Some(false));
        assert!(no_op_outcome.audio.events.is_empty());
        assert!(no_op_outcome.landscape.is_empty());
    }

    #[test]
    fn terrain_mutators_are_visible_to_same_callback_gback_queries() {
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Earth]\nName=Earth\nDensity=100\nDigFree=1\nBlastFree=1\n",
        )
        .expect("terrain-query material builds");
        let materials = Rc::new(MaterialSet::from_resource_library(&library));
        let earth = materials.id_of("Earth").expect("Earth exists");
        let map_world = || {
            let mut world = draw_map_world(8, 7, 3, true);
            world
                .landscape_mut()
                .expect("landscape exists")
                .resolve_grid_materials(|name| materials.id_of(name));
            world.with_materials(Some(Rc::clone(&materials)))
        };
        let terrain_world = || {
            let mut world = map_world();
            let landscape = world.landscape_mut().expect("landscape exists");
            for y in 0..7 {
                for x in 0..8 {
                    landscape.grid_write_byte(x, y, 1);
                }
            }
            landscape.refresh_all_raster_columns();
            world
        };

        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"#strict 2
func TerrainState(x, y) { return [GBackSolid(x, y), GetMaterial(x, y), GetTexture(x, y)]; }
func ProbeBlast() {
    var before = TerrainState(3, 3);
    var changed = BlastFree(3, 3, 1, 1);
    return [before, changed, TerrainState(3, 3)];
}
func ProbeShake() {
    var before = TerrainState(3, 3);
    var changed = ShakeFree(3, 3, 1);
    return [before, changed, TerrainState(3, 3)];
}
func ProbeDig() {
    var before = TerrainState(3, 3);
    var changed = DigFree(3, 3, 1);
    return [before, changed, TerrainState(3, 3)];
}
func ProbeDigRect() {
    var before = TerrainState(3, 3);
    var changed = DigFreeRect(3, 3, 1, 1);
    return [before, changed, TerrainState(3, 3)];
}
func ProbeDrawMap() {
    var before = TerrainState(0, 1);
    var changed = DrawMap(-2, 1, 7, 5, "map Runtime { seed = 9; Named; };");
    return [before, changed, TerrainState(0, 1)];
}
func ProbeDrawDefMap() {
    var before = TerrainState(0, 1);
    var changed = DrawDefMap(-2, 1, 7, 5, "Requested");
    return [before, changed, TerrainState(0, 1)];
}
"#,
            )
            .expect("terrain visibility probes compile");

        let earth_state = Value::Array(vec![
            Value::Bool(true),
            Value::Int(earth.index() as i32),
            Value::String("Rough".to_string().into()),
        ]);
        let sky_state = Value::Array(vec![
            Value::Bool(false),
            Value::Int(MATERIAL_NONE),
            Value::Nil,
        ]);
        for probe in ["ProbeBlast", "ProbeShake", "ProbeDig", "ProbeDigRect"] {
            let random = enter_random_context(LcgRng::new(101));
            let (result, outcome) =
                with_effect_context(None, &[], terrain_world(), 1, || script.call(probe, &[]));
            let _ = random.finish();
            assert_eq!(
                result.unwrap_or_else(|error| panic!("{probe} failed: {error}")),
                Value::Array(vec![earth_state.clone(), Value::Nil, sky_state.clone()]),
                "{probe} must expose its cleared pixel before returning from the callback"
            );
            assert_eq!(outcome.landscape.len(), 1, "{probe} queues one fold");
            assert!(
                matches!(
                    (probe, &outcome.landscape[0]),
                    (
                        "ProbeBlast",
                        LandscapeOperation::BlastCirclePreviewed { .. }
                    ) | ("ProbeShake", LandscapeOperation::ShakeCircle { .. })
                        | ("ProbeDig", LandscapeOperation::DigCirclePreviewed { .. })
                        | ("ProbeDigRect", LandscapeOperation::DigRectPreviewed { .. })
                ),
                "{probe} queued {:?}",
                outcome.landscape[0]
            );
        }

        for (probe, seed) in [("ProbeDrawMap", 17), ("ProbeDrawDefMap", 37)] {
            let random = enter_random_context(LcgRng::new(seed));
            let (result, outcome) =
                with_effect_context(None, &[], map_world(), 1, || script.call(probe, &[]));
            let _ = random.finish();
            assert_eq!(
                result.unwrap_or_else(|error| panic!("{probe} failed: {error}")),
                Value::Array(vec![sky_state.clone(), Value::Int(1), earth_state.clone(),]),
                "{probe} must expose its painted pixel before returning from the callback"
            );
            assert_eq!(outcome.landscape.len(), 1, "{probe} queues one fold");
            assert!(
                matches!(
                    (probe, &outcome.landscape[0]),
                    ("ProbeDrawMap", LandscapeOperation::DrawMap { .. })
                        | ("ProbeDrawDefMap", LandscapeOperation::DrawDefMap { .. })
                ),
                "{probe} queued {:?}",
                outcome.landscape[0]
            );
        }
    }

    #[test]
    fn dig_free_registers_landscape_operation() {
        let args = [
            Value::Int(42),
            Value::Int(128),
            Value::Int(6),
            Value::Bool(true),
        ];
        let (result, outcome) = with_object_host_context(|| dig_free(&args));
        assert_eq!(result.expect("DigFree succeeds"), Value::Nil);
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DigCirclePreviewed { center, radius } => {
                assert_eq!(*center, Vector2::new(42, 128));
                assert_eq!(*radius, 6);
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn dig_free_rect_requires_positive_dimensions() {
        let args = [Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(4)];
        let (result, outcome) = with_object_host_context(|| dig_free_rect(&args));
        assert_eq!(result.expect("DigFreeRect succeeds"), Value::Nil);
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn dig_free_rect_registers_landscape_operation() {
        let args = [
            Value::Int(10),
            Value::Int(20),
            Value::Int(5),
            Value::Int(7),
            Value::Bool(false),
        ];
        let (result, outcome) = with_object_host_context(|| dig_free_rect(&args));
        assert_eq!(result.expect("DigFreeRect succeeds"), Value::Nil);
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DigRectPreviewed {
                origin,
                width,
                height,
            } => {
                assert_eq!(*origin, Vector2::new(10, 20));
                assert_eq!(*width, 5);
                assert_eq!(*height, 7);
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn blast_free_registers_landscape_operation() {
        let args = [Value::Int(12), Value::Int(34), Value::Int(5), Value::Int(3)];
        let (result, outcome) = with_object_host_context(|| blast_free(&args));
        assert_eq!(result.expect("BlastFree succeeds"), Value::Nil);
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::BlastCircle {
                center,
                radius,
                controller,
                ..
            } => {
                assert_eq!(*center, Vector2::new(12, 34));
                assert_eq!(*radius, 5);
                assert_eq!(*controller, Some(2));
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn blast_free_offsets_coordinates_without_explicit_controller() {
        let object_context = HostObjectContext {
            owner: 4,
            controller: 4,
            position: Vector2::new(5, 10),
            ..idle_object_context()
        }
        .with_controller(9);
        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            1,
            || blast_free(&[Value::Int(3), Value::Int(7), Value::Int(6)]),
        );
        assert_eq!(result.expect("BlastFree succeeds"), Value::Nil);
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::BlastCircle {
                center,
                radius,
                controller,
                ..
            } => {
                assert_eq!(*center, Vector2::new(8, 17));
                assert_eq!(*radius, 6);
                // FnBlastFree defaults to the calling object's Controller,
                // not its Owner (C4Script.cpp:2284-2294).
                assert_eq!(*controller, Some(9));
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn blast_free_registers_zero_radius_like_cpp() {
        // FnBlastFree has no positive-level gate; C4Landscape::BlastFree's
        // inclusive loops visit the center once for radius zero
        // (C4Script.cpp:2284-2294; C4Landscape.cpp:1022-1063).
        let args = [Value::Int(0), Value::Int(0), Value::Int(0)];
        let (result, outcome) = with_object_host_context(|| blast_free(&args));
        assert_eq!(result.expect("BlastFree handles zero level"), Value::Nil);
        assert!(matches!(
            outcome.landscape.as_slice(),
            [LandscapeOperation::BlastCircle {
                center: Vector2 { x: 0, y: 0 },
                radius: 0,
                ..
            }]
        ));
    }

    #[test]
    fn blast_free_negative_cause_is_explicit_and_global() {
        // Only caused_by_plus_one == 0 selects the caller-relative fallback.
        // A negative explicit value remains global and maps to value-1
        // (C4Script.cpp:2284-2294).
        let object_context = HostObjectContext {
            owner: 4,
            controller: 4,
            position: Vector2::new(5, 10),
            ..idle_object_context()
        }
        .with_controller(9);
        let (result, outcome) = with_effect_context(
            Some(object_context),
            &[],
            HostWorldContext::default(),
            1,
            || blast_free(&[Value::Int(3), Value::Int(7), Value::Int(6), Value::Int(-2)]),
        );
        assert_eq!(result.expect("BlastFree succeeds"), Value::Nil);
        assert!(matches!(
            outcome.landscape.as_slice(),
            [LandscapeOperation::BlastCircle {
                center: Vector2 { x: 3, y: 7 },
                radius: 6,
                controller: Some(-3),
                ..
            }]
        ));
    }

    #[test]
    fn blast_free_pads_missing_parameters_and_discards_extras_like_cpp() {
        // Parse_Params pads a known four-parameter engine function with nil
        // and drops extra values before its typed call. FnBlastFree is void,
        // so both forms return nil (C4AulParse.cpp:2264-2345;
        // C4Script.cpp:6121-6181,2284-2295).
        let (missing, missing_outcome) = with_object_host_context(|| blast_free(&[]));
        assert_eq!(
            missing.expect("missing arguments become nil/zero"),
            Value::Nil
        );
        assert!(matches!(
            missing_outcome.landscape.as_slice(),
            [LandscapeOperation::BlastCircle {
                center: Vector2 { x: 0, y: 0 },
                radius: 0,
                controller: Some(OWNER_NONE),
                ..
            }]
        ));

        let args = [
            Value::Int(2),
            Value::Int(4),
            Value::Int(6),
            Value::Bool(true),
            Value::String("discarded".to_string().into()),
        ];
        let (extra, extra_outcome) = with_object_host_context(|| blast_free(&args));
        assert_eq!(extra.expect("the fifth argument is discarded"), Value::Nil);
        assert!(matches!(
            extra_outcome.landscape.as_slice(),
            [LandscapeOperation::BlastCircle {
                center: Vector2 { x: 2, y: 4 },
                radius: 6,
                controller: Some(0),
                ..
            }]
        ));
    }

    #[test]
    fn shake_free_registers_landscape_operation() {
        let args = [Value::Int(30), Value::Int(40), Value::Int(5)];
        let (result, outcome) = with_object_host_context(|| shake_free(&args));
        assert_eq!(result.expect("ShakeFree succeeds"), Value::Nil);
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::ShakeCircle { center, radius } => {
                assert_eq!(*center, Vector2::new(30, 40));
                assert_eq!(*radius, 5);
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn gamma_host_fns_queue_valid_ramp_writes_and_ignore_invalid_indices() {
        // FnSetGamma/FnResetGamma (C4Script.cpp:4998-5006) write one of the
        // nine C4GraphicsSystem ramps; SetGamma silently ignores indices
        // outside 0..C4MaxGammaRamps (C4GraphicsSystem.cpp:772-784).
        let args = [
            Value::Int(0x000000),
            Value::Int(0x646464),
            Value::Int(0xc8c8c8),
        ];
        let (result, outcome) = with_object_host_context(|| set_gamma(&args));
        assert_eq!(result.expect("SetGamma succeeds"), Value::Nil);
        assert!(matches!(
            outcome.landscape.as_slice(),
            [LandscapeOperation::GammaRamp {
                index: 0,
                points: [0x000000, 0x646464, 0xc8c8c8]
            }]
        ));

        let (result, outcome) = with_object_host_context(|| reset_gamma(&[]));
        assert_eq!(result.expect("ResetGamma succeeds"), Value::Nil);
        assert!(matches!(
            outcome.landscape.as_slice(),
            [LandscapeOperation::GammaRamp {
                index: 0,
                points: [0x000000, 0x808080, 0xffffff]
            }]
        ));

        let invalid = [Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(9)];
        let (result, outcome) = with_object_host_context(|| set_gamma(&invalid));
        assert_eq!(result.expect("invalid SetGamma is a no-op"), Value::Nil);
        assert!(outcome.landscape.is_empty());

        let invalid = [Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(-1)];
        let (result, outcome) = with_object_host_context(|| set_gamma(&invalid));
        assert_eq!(result.expect("negative SetGamma is a no-op"), Value::Nil);
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn set_sky_parallax_registers_operation_with_keep_defaults() {
        // FnSetSkyParallax (C4Script.cpp:4955-4970): all seven ints pass
        // through; missing/nil args are 0 at the C4Aul boundary (which
        // ZEROES the scroll slots — only the explicit SkyPar_KEEP magic
        // preserves them).
        let args = [Value::Int(1), Value::Int(20), Value::Int(20)];
        let (result, outcome) = with_object_host_context(|| set_sky_parallax(&args));
        assert_eq!(result.expect("SetSkyParallax succeeds"), Value::Nil);
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::SkyParallax {
                mode,
                par_x,
                par_y,
                xdir,
                ydir,
                x,
                y,
            } => {
                assert_eq!((*mode, *par_x, *par_y), (1, 20, 20));
                assert_eq!((*xdir, *ydir, *x, *y), (0, 0, 0, 0));
            }
            other => panic!("unexpected landscape operation: {:?}", other),
        }
    }

    #[test]
    fn set_sky_fade_converts_ignored_destination_arguments() {
        let error = set_sky_fade(&[
            Value::Int(96),
            Value::Int(64),
            Value::Int(200),
            Value::Int(1),
            Value::Int(2),
            Value::String("ignored but typed".into()),
        ])
        .expect_err("SetSkyFade type-checks its ignored destination color");
        assert!(error.message().contains("expected integer for to blue"));
    }

    #[test]
    fn shake_free_missing_and_non_positive_radius_are_noops() {
        for args in [
            Vec::new(),
            vec![Value::Int(10), Value::Int(20)],
            vec![Value::Int(10), Value::Int(20), Value::Int(0)],
        ] {
            let (result, outcome) = with_object_host_context(|| shake_free(&args));
            assert_eq!(
                result.expect("ShakeFree handles a missing or zero radius"),
                Value::Nil
            );
            assert!(outcome.landscape.is_empty());
        }
    }

    #[test]
    fn g_back_liquid_returns_false_in_height_landscape() {
        let landscape = Landscape::flat(8, 4);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_liquid(&[Value::Int(1), Value::Int(6)])
        });
        let value = result.expect("GBackLiquid succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_liquid_detects_liquid_column() {
        let mut landscape = Landscape::flat(8, 4);
        landscape.set_liquid_column(1, vec![LiquidSegment::new(5, 9)]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_liquid(&[Value::Int(1), Value::Int(6)])
        });
        let value = result.expect("GBackLiquid succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn extract_liquid_is_object_relative_and_stages_one_extraction() {
        // FnExtractLiquid offsets by cthr->Obj, rejects non-liquid pixels,
        // then returns Landscape.ExtractMaterial's material number
        // (C4Script.cpp:2194-2199).
        let library =
            clonk_resources::MaterialLibrary::parse("[Material Water]\nName=Water\nDensity=25\n")
                .expect("water material builds");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let mut landscape = Landscape::flat(16, 20);
        landscape.set_liquid_column(
            5,
            vec![LiquidSegment {
                top: 10,
                bottom: 14,
                material: Some(water),
            }],
        );
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            2,
            false,
        )
        .with_materials(Some(Rc::new(materials)));
        let object = HostObjectContext {
            position: Vector2::new(4, 8),
            ..idle_object_context()
        };
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 2\nfunc Probe() { return [ExtractLiquid(1, 3), ExtractLiquid(2, 3)]; }",
            )
            .expect("ExtractLiquid probe compiles");

        let (result, outcome) =
            with_effect_context(Some(object), &[], world, 2, || script.call("Probe", &[]));
        assert_eq!(
            result.expect("ExtractLiquid succeeds"),
            Value::Array(vec![
                Value::Int(water.index() as i32),
                Value::Int(MATERIAL_NONE),
            ])
        );
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::ExtractLiquid { position } => {
                assert_eq!(*position, Vector2::new(5, 11));
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }
    }

    #[test]
    fn repeated_extract_liquid_calls_see_the_live_landscape() {
        // FnExtractLiquid mutates Game.Landscape before returning
        // (C4Script.cpp:2194-2199). Repeated calls at a submerged point
        // therefore peel FindMatTop pixels until that point is dry, and a
        // later GBackLiquid in the SAME callback observes the cleared plane.
        let library =
            clonk_resources::MaterialLibrary::parse("[Material Water]\nName=Water\nDensity=25\n")
                .expect("water material builds");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let mut landscape = Landscape::new(1, vec![4]).expect("landscape builds");
        landscape.set_world_height(4);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            1,
            4,
            vec![0, 1, 1, 1],
            vec![0, 25],
            vec![None, Some("Water".to_string())],
            vec![None; 2],
        ));
        landscape.resolve_grid_materials(|name| materials.id_of(name));
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        )
        .with_materials(Some(Rc::new(materials)));
        let mut script = clonk_script::Engine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 2\nfunc Probe() { return [\n\
                 ExtractLiquid(0, 3), ExtractLiquid(0, 3),\n\
                 ExtractLiquid(0, 3), ExtractLiquid(0, 3),\n\
                 GBackLiquid(0, 3)\n\
                 ]; }",
            )
            .expect("repeated ExtractLiquid probe compiles");

        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || script.call("Probe", &[]));
        assert_eq!(
            result.expect("repeated ExtractLiquid succeeds"),
            Value::Array(vec![
                Value::Int(water.index() as i32),
                Value::Int(water.index() as i32),
                Value::Int(water.index() as i32),
                Value::Int(MATERIAL_NONE),
                Value::Bool(false),
            ])
        );
        assert_eq!(outcome.landscape.len(), 3);
        assert!(outcome.landscape.iter().all(|operation| matches!(
            operation,
            LandscapeOperation::ExtractLiquid { position }
                if *position == Vector2::new(0, 3)
        )));
    }

    #[test]
    fn insert_material_rejects_mnone_without_staging_an_operation() {
        // C4Landscape::InsertMaterial checks MatValid first and returns false
        // without touching landscape/PXS/reactions (C4Landscape.cpp:1159-1166).
        // Deep Sea passes ExtractLiquid's MNone result straight through here
        // after a pump source dries.
        let library =
            clonk_resources::MaterialLibrary::parse("[Material Water]\nName=Water\nDensity=25\n")
                .expect("water material builds");
        let materials = MaterialSet::from_resource_library(&library);
        let world = HostWorldContext::default().with_materials(Some(Rc::new(materials)));

        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            insert_material(&[
                Value::Int(MATERIAL_NONE),
                Value::Int(4),
                Value::Int(5),
                Value::Int(6),
                Value::Int(7),
            ])
        });

        assert_eq!(
            result.expect("InsertMaterial handles MNone"),
            Value::Bool(false)
        );
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn insert_material_returns_the_cpp_preflight_result() {
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Water]\nName=Water\nDensity=25\n\n\
             [Material Earth]\nName=Earth\nDensity=100\n\n\
             [Material Air]\nName=Air\nDensity=0\n",
        )
        .expect("material library builds");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let air = materials.id_of("Air").expect("air exists");
        let mut densities = vec![0; 3];
        densities[1] = 25;
        densities[2] = 100;
        let names = vec![None, Some("Water".to_string()), Some("Earth".to_string())];
        let mut landscape = Landscape::new(3, vec![3; 3]).expect("landscape builds");
        landscape.set_world_height(3);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            3,
            3,
            vec![0, 0, 0, 0, 2, 0, 2, 2, 2],
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

        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                insert_material(&[
                    Value::Int(water.index() as i32),
                    Value::Int(3),
                    Value::Int(1),
                ])?,
                insert_material(&[
                    Value::Int(water.index() as i32),
                    Value::Int(1),
                    Value::Int(1),
                ])?,
                insert_material(&[
                    Value::Int(air.index() as i32),
                    Value::Int(-999),
                    Value::Int(-999),
                ])?,
                insert_material(&[
                    Value::Int(water.index() as i32),
                    Value::Int(0),
                    Value::Int(0),
                ])?,
            ]))
        });

        assert_eq!(
            result.expect("InsertMaterial probes succeed"),
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
                Value::Bool(true),
            ])
        );
        assert_eq!(outcome.landscape.len(), 1);
        assert!(matches!(
            &outcome.landscape[0],
            LandscapeOperation::InsertMaterial {
                material,
                position,
                velocity,
            } if *material == water.index() as i32
                && *position == Vector2::new(0, 0)
                && *velocity == Vector2::new(0, 0)
        ));
    }

    #[test]
    fn insert_material_push_pull_preflight_routes_an_escape_and_rejects_a_sealed_pocket() {
        let library = clonk_resources::MaterialLibrary::parse(
            "[Material Water]\nName=Water\nDensity=25\nMaxSlide=1\nInstable=1\n\n\
             [Material Earth]\nName=Earth\nDensity=100\n",
        )
        .expect("push-pull materials build");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let world = |width: u32, height: u32, bytes: Vec<u8>, push_pull: bool| {
            let materials = materials.clone();
            let mut landscape = Landscape::new(width, vec![height as i32; width as usize])
                .expect("landscape builds");
            landscape.set_world_height(height as i32);
            landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
                width,
                height,
                bytes,
                vec![0, 25, 100],
                vec![None, Some("Water".into()), Some("Earth".into())],
                vec![None; 3],
            ));
            landscape.set_border_open(0, 0, false, false);
            landscape.resolve_grid_materials(|name| materials.id_of(name));
            world_with(
                Vec::<HostWorldObject>::new(),
                Some(landscape),
                HashMap::new(),
                HashMap::new(),
            )
            .with_scenario_values(Rc::new(
                ScenarioValueStore::with_landscape_push_pull_for_test(push_pull),
            ))
            .with_materials(Some(Rc::new(materials)))
        };
        let probe = |world| {
            with_effect_context(None, &[], world, 1, || {
                insert_material(&[
                    Value::Int(water.index() as i32),
                    Value::Int(2),
                    Value::Int(2),
                ])
            })
        };

        let mut corridor = vec![2; 25];
        corridor[2 * 5] = 0;
        corridor[2 * 5 + 1] = 1;
        corridor[2 * 5 + 2] = 1;
        corridor[2 * 5 + 3] = 1;
        let (result, outcome) = probe(world(5, 5, corridor.clone(), false));
        assert_eq!(
            result.expect("default preflight succeeds"),
            Value::Bool(false)
        );
        assert!(outcome.landscape.is_empty());

        let (result, outcome) = probe(world(5, 5, corridor, true));
        assert_eq!(result.expect("push preflight succeeds"), Value::Bool(true));
        assert!(matches!(
            outcome.landscape.as_slice(),
            [LandscapeOperation::InsertMaterial { material, position, velocity }]
                if *material == water.index() as i32
                    && *position == Vector2::new(2, 2)
                    && *velocity == Vector2::ZERO
        ));

        let (result, outcome) = probe(world(3, 3, vec![2; 9], true));
        assert_eq!(
            result.expect("sealed push preflight succeeds"),
            Value::Bool(false)
        );
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn get_player_count_counts_registered_players() {
        let mut alice = PlayerState::default();
        alice.id = 1;
        alice.name = "Alice".into();
        let mut bob = PlayerState::default();
        bob.id = 2;
        bob.name = "Bob".into();
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![alice, bob],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || get_player_count(&[]));
        assert_eq!(result.expect("GetPlayerCount succeeds"), Value::Int(2));
    }

    #[test]
    fn hostile_observes_declared_and_mutated_player_relations_like_cpp() {
        // FnHostile selects symmetric C4PlayerList::Hostile or the directed
        // HostilityDeclared query (C4Script.cpp:2511-2519;
        // C4PlayerList.cpp:82-104). FnSetHostility rejects missing/self
        // opponents and mutates immediately (C4Script.cpp:2521-2537;
        // C4Player.cpp:981-1003).
        let alice = PlayerState {
            id: 1,
            hostility: vec![2],
            ..PlayerState::default()
        };
        let bob = PlayerState {
            id: 2,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![alice, bob],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    "#strict\nglobal func Probe() { return [Hostile(1, 2), Hostile(2, 1), Hostile(1, 2, true), Hostile(2, 1, true), Hostile(1, 1), Hostile(1, 99), SetHostility(2, 1, true, true, true), Hostile(2, 1, true), SetHostility(2, 2, true, true, true), SetHostility(2, 99, true, true, true)]; }",
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("hostility functions run"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(false),
            ])
        );
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::SetHostility {
                player_id: 2,
                opponent: 1,
                hostile: true,
            }]
        ));
    }

    #[test]
    fn get_player_val_indexed_lists_observe_same_call_mutations() {
        let player = PlayerState {
            id: 4,
            hostility_entries: vec![(3, 1)],
            knowledge_entries: vec![("BRIK".into(), 0), ("BRIK".into(), 9)],
            magic_entries: vec![("FIRE".into(), 0), ("FIRE".into(), 4)],
            home_base_material_entries: vec![("ZINC".into(), 0)],
            home_base_production_entries: vec![("ROCK".into(), 0)],
            ..PlayerState::default()
        };
        let opponent = PlayerState {
            id: 2,
            ..PlayerState::default()
        };
        let definitions = ["BRIK", "FIRE", "ZINC", "ROCK"]
            .into_iter()
            .map(|id| {
                (
                    id.to_string(),
                    DefinitionMetadata {
                        category: if id == "FIRE" {
                            crate::CATEGORY_MAGIC
                        } else {
                            1
                        },
                        ..DefinitionMetadata::default()
                    },
                )
            })
            .collect();
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            None,
            definitions,
            HashMap::from([(4, player), (2, opponent)]),
        );

        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                set_hostility(&[
                    Value::Int(4),
                    Value::Int(2),
                    Value::Bool(false),
                    Value::Bool(true),
                    Value::Bool(true),
                ])?,
                Value::Bool(true)
            );
            assert_eq!(
                set_plr_knowledge(&[Value::Int(4), Value::C4Id("BRIK".into())])?,
                Value::Bool(true)
            );
            assert_eq!(
                set_plr_magic(&[Value::Int(4), Value::C4Id("FIRE".into())])?,
                Value::Int(1)
            );
            assert_eq!(
                do_homebase_material(&[Value::Int(4), Value::C4Id("ZINC".into()), Value::Int(2),])?,
                Value::Bool(true)
            );
            assert_eq!(
                do_homebase_production(&[
                    Value::Int(4),
                    Value::C4Id("ROCK".into()),
                    Value::Int(1),
                ])?,
                Value::Bool(true)
            );

            let read = |entry: &str, index: i32| {
                get_player_val(&[
                    Value::String(entry.into()),
                    Value::String("Player".into()),
                    Value::Int(4),
                    Value::Int(index),
                ])
            };
            Ok::<_, RuntimeError>(Value::Array(vec![
                read("Hostile", 0)?,
                read("Hostile", 1)?,
                read("Knowledge", 0)?,
                read("Knowledge", 1)?,
                read("Knowledge", 2)?,
                read("Knowledge", 3)?,
                read("Magic", 0)?,
                read("Magic", 1)?,
                read("Magic", 2)?,
                read("Magic", 3)?,
                read("HomeBaseMaterial", 0)?,
                read("HomeBaseMaterial", 1)?,
                read("HomeBaseProduction", 0)?,
                read("HomeBaseProduction", 1)?,
                hostile(&[Value::Int(4), Value::Int(2), Value::Bool(true)])?,
            ]))
        });

        assert_eq!(
            result.expect("same-call indexed list mutations succeed"),
            Value::Array(vec![
                Value::C4Id("0003".into()),
                Value::Int(0),
                Value::C4Id("BRIK".into()),
                Value::Int(1),
                Value::C4Id("BRIK".into()),
                Value::Int(9),
                Value::C4Id("FIRE".into()),
                Value::Int(1),
                Value::C4Id("FIRE".into()),
                Value::Int(4),
                Value::C4Id("ZINC".into()),
                Value::Int(2),
                Value::C4Id("ROCK".into()),
                Value::Int(1),
                Value::Bool(false),
            ])
        );
        assert_eq!(outcome.player_commands.len(), 5);
    }

    #[test]
    fn get_player_by_index_returns_player_number() {
        let mut alice = PlayerState::default();
        alice.id = 1;
        alice.name = "Alice".into();
        let mut carol = PlayerState::default();
        carol.id = 3;
        carol.name = "Carol".into();
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![alice, carol],
        );
        let args = [Value::Int(1)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_player_by_index(&args));
        assert_eq!(result.expect("GetPlayerByIndex succeeds"), Value::Int(3));
    }

    #[test]
    fn player_order_override_drives_get_player_by_index() {
        let mut zero = PlayerState::default();
        zero.id = 0;
        let mut one = PlayerState::default();
        one.id = 1;
        let mut two = PlayerState::default();
        two.id = 2;
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![two, zero, one],
        );
        assert_eq!(world.player_ids(), &[0, 1, 2]);

        let world = world.with_player_order([1, 1, 99, 0]);
        assert_eq!(world.player_ids(), &[1, 0, 2]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<Value, RuntimeError>(Value::Array(vec![
                get_player_by_index(&[Value::Int(0)])?,
                get_player_by_index(&[Value::Int(1)])?,
                get_player_by_index(&[Value::Int(2)])?,
            ]))
        });
        assert_eq!(
            result.expect("GetPlayerByIndex follows overridden player order"),
            Value::Array(vec![Value::Int(1), Value::Int(0), Value::Int(2)])
        );
    }

    #[test]
    fn get_player_by_index_type_filter_keeps_order_teams_and_eliminated_players() {
        // C4PlayerList::GetByIndex(index, type) filters only C4PlayerInfo
        // type while walking native player-list order. Team and elimination
        // state do not remove a runtime player from this lookup.
        let script_eliminated = PlayerState {
            id: 40,
            script_player: true,
            status: crate::PlayerStatus::Eliminated,
            team: Some(3),
            ..PlayerState::default()
        };
        let user_eliminated = PlayerState {
            id: 10,
            status: crate::PlayerStatus::Eliminated,
            team: Some(2),
            ..PlayerState::default()
        };
        let user_active = PlayerState {
            id: 30,
            status: crate::PlayerStatus::Active,
            team: Some(1),
            ..PlayerState::default()
        };
        let script_active = PlayerState {
            id: 20,
            script_player: true,
            status: crate::PlayerStatus::Active,
            team: None,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            [
                script_eliminated,
                user_eliminated,
                user_active,
                script_active,
            ],
        )
        .with_player_order([40, 10, 30, 20]);

        let user = i32::from(crate::PLAYER_INFO_TYPE_USER);
        let script = i32::from(crate::PLAYER_INFO_TYPE_SCRIPT);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<Value, RuntimeError>(Value::Array(vec![
                get_player_by_index(&[Value::Int(0), Value::Int(user)])?,
                get_player_by_index(&[Value::Int(1), Value::Int(user)])?,
                get_player_by_index(&[Value::Int(2), Value::Int(user)])?,
                get_player_by_index(&[Value::Int(0), Value::Int(script)])?,
                get_player_by_index(&[Value::Int(1), Value::Int(script)])?,
                get_player_by_index(&[Value::Int(2), Value::Int(script)])?,
            ]))
        });

        assert_eq!(
            result.expect("GetPlayerByIndex preserves filtered native order"),
            Value::Array(vec![
                Value::Int(10),
                Value::Int(30),
                Value::Int(OWNER_NONE),
                Value::Int(40),
                Value::Int(20),
                Value::Int(OWNER_NONE),
            ])
        );
    }

    #[test]
    fn get_player_name_returns_registered_name() {
        let mut player = PlayerState::default();
        player.id = 5;
        player.name = "Delta".into();
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(5)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_player_name(&args));
        assert_eq!(
            result.expect("GetPlayerName succeeds"),
            Value::String("Delta".into())
        );
    }

    #[test]
    fn get_tagged_player_name_colors_the_name_like_cpp() {
        // FnGetTaggedPlayerName wraps a valid player's name in the readable
        // 24-bit player color and returns nil for an invalid player
        // (C4Script.cpp:1084-1091; C4Gui.cpp:71-87).
        let player = PlayerState {
            id: 5,
            name: "Delta".into(),
            color: Some(crate::RgbColor::new(0, 0, 0)),
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    "#strict\nglobal func Probe() { return [GetTaggedPlayerName(5), GetTaggedPlayerName(99)]; }",
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("GetTaggedPlayerName succeeds"),
            Value::Array(vec![
                Value::String("<c 656565>Delta</c>".into()),
                Value::Nil
            ])
        );
    }

    #[test]
    fn get_player_val_reads_the_cpp_view_coordinates() {
        // FnGetPlayerVal reflects C4Player::CompileFunc
        // (C4Script.cpp:4252-4263), whose ViewX/ViewY entries are the
        // current player view coordinates (C4Player.cpp:1576-1577).
        let player = PlayerState {
            id: 0,
            view_center: Some(Vector2::new(306, 271)),
            viewports: vec![PlayerViewport::new(Vector2::new(900, 901))],
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    "#strict\nglobal func Probe() { return [GetPlayerVal(\"ViewX\", 0, 0), GetPlayerVal(\"ViewY\", 0, 0)]; }",
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("GetPlayerVal runs"),
            Value::Array(vec![Value::Int(306), Value::Int(271)])
        );
    }

    #[test]
    fn get_player_val_reflects_cpp_indexed_player_lists() {
        let crew = [
            (42, ObjectStatus::Normal),
            (7, ObjectStatus::Inactive),
            (99, ObjectStatus::Deleted),
        ];
        let objects = crew
            .iter()
            .map(|(id, status)| {
                fixture_world_object(ObjectId::new(*id), "CLNK")
                    .with_status(*status)
                    .with_owner(4)
            })
            .collect::<Vec<_>>();
        let player = PlayerState {
            id: 4,
            wealth: 17,
            hostility_entries: vec![(3, 1), (2, 0)],
            home_base_material_entries: vec![("ZINC".into(), 5), ("BRIK".into(), 0)],
            home_base_production_entries: vec![("ROCK".into(), -2), ("WOOD".into(), 11)],
            knowledge_entries: vec![("PLAN".into(), 0), ("KNOW".into(), 3)],
            magic_entries: vec![("FIRE".into(), 0), ("WIND".into(), 1)],
            crew: crew.iter().map(|(id, _)| ObjectId::new(*id)).collect(),
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(objects, vec![player]);
        let expected = [
            (
                "Hostile",
                vec![
                    Value::C4Id("0003".into()),
                    Value::Int(1),
                    Value::C4Id("0002".into()),
                    Value::Int(0),
                ],
            ),
            (
                "HomeBaseMaterial",
                vec![
                    Value::C4Id("ZINC".into()),
                    Value::Int(5),
                    Value::C4Id("BRIK".into()),
                    Value::Int(0),
                ],
            ),
            (
                "HomeBaseProduction",
                vec![
                    Value::C4Id("ROCK".into()),
                    Value::Int(-2),
                    Value::C4Id("WOOD".into()),
                    Value::Int(11),
                ],
            ),
            (
                "Knowledge",
                vec![
                    Value::C4Id("PLAN".into()),
                    Value::Int(0),
                    Value::C4Id("KNOW".into()),
                    Value::Int(3),
                ],
            ),
            (
                "Magic",
                vec![
                    Value::C4Id("FIRE".into()),
                    Value::Int(0),
                    Value::C4Id("WIND".into()),
                    Value::Int(1),
                ],
            ),
            ("Crew", vec![Value::Int(42), Value::Int(7)]),
        ];

        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let read = |entry: &str, index: i32| {
                get_player_val(&[
                    Value::String(entry.into()),
                    Value::String("Player".into()),
                    Value::Int(4),
                    Value::Int(index),
                ])
            };
            let mut values = Vec::new();
            for (entry, expected_values) in &expected {
                for index in 0..expected_values.len() {
                    values.push(read(entry, index as i32)?);
                }
                values.push(read(entry, -1)?);
                values.push(read(entry, expected_values.len() as i32)?);
                values.push(read(entry, i32::MAX)?);
            }
            values.push(read("Wealth", 0)?);
            values.push(read("Wealth", 1)?);
            values.push(read("Wealth", -1)?);
            Ok::<_, RuntimeError>(Value::Array(values))
        });

        let mut expected_values = Vec::new();
        for (_, stream) in expected {
            expected_values.extend(stream);
            expected_values.extend([Value::Nil, Value::Nil, Value::Nil]);
        }
        expected_values.extend([Value::Int(17), Value::Nil, Value::Nil]);
        assert_eq!(
            result.expect("GetPlayerVal indexed reflection succeeds"),
            Value::Array(expected_values)
        );
    }

    #[test]
    fn get_player_val_view_coordinates_follow_the_cursor() {
        // In C4PVM_Cursor, C4Player::UpdateView copies the current cursor
        // position into ViewX/ViewY (C4Player.cpp:1692-1704). Legacy Rust
        // fixtures may carry neither the independent saved center nor a
        // presentation viewport, so retain the cursor migration fallback.
        let cursor = ObjectId::new(42);
        let player = PlayerState {
            id: 0,
            status: crate::PlayerStatus::Active,
            cursor: Some(cursor),
            ..PlayerState::default()
        };
        let object = fixture_world_object(cursor, "CLNK")
            .with_action_name("Walk")
            .with_owner(0)
            .with_position(Vector2::new(306, 271));
        let world = HostWorldContext::from_objects_with_players(vec![object], vec![player]);
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            get_player_val(&[Value::String("ViewX".into()), Value::Int(0), Value::Int(0)])
        });

        assert_eq!(
            result.expect("GetPlayerVal reads cursor view"),
            Value::Int(306)
        );
    }

    #[test]
    fn get_player_val_reads_join_identity_and_modeled_cpp_fields() {
        // C4Player::CompileFunc exposes the join-time client identity and the
        // old-gfx Color/Position indices as their own fields. Color is not
        // ColorDw, and an unknown client is the integer sentinel -1 rather
        // than nil (C4Player.cpp:1556-1580,1718-1728).
        let mut control = PlayerControlState::default();
        control.auto_context_menu = true;
        let remote = PlayerState {
            id: 4,
            at_client: crate::PlayerAtClient::new(7),
            at_client_name: Some("Remote Client".into()),
            evaluated: true,
            color_index: Some(3),
            position_index: Some(2),
            control,
            ..PlayerState::default()
        };
        let unknown = PlayerState {
            id: 5,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![remote, unknown],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let read = |entry: &str, section: Value, player: i32, index: i32| {
                get_player_val(&[
                    Value::String(entry.into()),
                    section,
                    Value::Int(player),
                    Value::Int(index),
                ])
            };
            Ok::<_, RuntimeError>(Value::Array(vec![
                read("AtClient", Value::Int(0), 4, 0)?,
                read("AtClientName", Value::String(String::new().into()), 4, 0)?,
                read("Color", Value::String("Player".into()), 4, 0)?,
                read("Position", Value::Int(0), 4, 0)?,
                read("Evaluated", Value::Int(0), 4, 0)?,
                read("AutoContextMenu", Value::Int(0), 4, 0)?,
                read("AtClient", Value::Int(0), 5, 0)?,
                read("AtClientName", Value::Int(0), 5, 0)?,
                read("Color", Value::Int(0), 5, 0)?,
                read("Unknown", Value::Int(0), 4, 0)?,
                read("AtClient", Value::Int(0), 4, 1)?,
            ]))
        });

        assert_eq!(
            result.expect("GetPlayerVal modeled field reads run"),
            Value::Array(vec![
                Value::Int(7),
                Value::String("Remote Client".into()),
                Value::Int(3),
                Value::Int(2),
                Value::Bool(true),
                Value::Int(1),
                Value::Int(-1),
                Value::String("Local".into()),
                Value::Int(-1),
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn get_player_val_reads_the_cpp_runtime_scalars() {
        // C4Player::CompileFunc exposes FogOfWar, ShowControl, Wealth,
        // Points, Value, InitialValue, ValueGain and ObjectsOwned under
        // these exact names (C4Player.cpp:1580-1590); FnGetPlayerVal
        // returns their native bool/int values (C4Script.cpp:4252-4263).
        let player = PlayerState {
            id: 4,
            wealth: 87,
            points: 135,
            value: 320,
            initial_value: 275,
            value_gain: 45,
            objects_owned: 6,
            fog_of_war: true,
            show_control: 9,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let read = |entry: &str| {
                get_player_val(&[Value::String(entry.into()), Value::Int(0), Value::Int(4)])
            };
            Ok::<_, RuntimeError>(Value::Array(vec![
                read("FogOfWar")?,
                read("ShowControl")?,
                read("Wealth")?,
                read("Points")?,
                read("Value")?,
                read("InitialValue")?,
                read("ValueGain")?,
                read("ObjectsOwned")?,
            ]))
        });

        assert_eq!(
            result.expect("GetPlayerVal scalar reads run"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(9),
                Value::Int(87),
                Value::Int(135),
                Value::Int(320),
                Value::Int(275),
                Value::Int(45),
                Value::Int(6),
            ])
        );
    }

    #[test]
    fn get_player_val_reads_the_cpp_control_bookkeeping() {
        // The remaining modeled C4Player::CompileFunc scalars retain their
        // exact serialized names and integer types (C4Player.cpp:1560-1605).
        let cursor = ObjectId::new(42);
        let view_cursor = ObjectId::new(43);
        let captain = ObjectId::new(44);
        let mut player = PlayerState {
            id: 7,
            player_info_id: 71,
            status: crate::PlayerStatus::Active,
            surrendered: true,
            cursor: Some(cursor),
            view_cursor: Some(view_cursor),
            captain: Some(captain),
            color: Some(crate::RgbColor::new(0x12, 0x34, 0x56)),
            color_dw_raw: Some(0xff12_3456),
            control_set: 3,
            mouse_control: 2,
            view_wealth: 21,
            view_value: 22,
            show_startup: true,
            select_count: 4,
            message_status: 5,
            message_buf: "legacy message".into(),
            crew_created: 6,
            production_delay: 12,
            production_unit: 3,
            ..PlayerState::default()
        };
        player
            .viewports
            .push(PlayerViewport::new(Vector2::ZERO).with_focus(Some(view_cursor)));
        player.control = crate::PlayerControlState {
            last_com: -2_147_483_630,
            last_com_delay: 7,
            last_com_down_double: 9,
            pressed_coms: 11,
            control_style: true,
            control_style_value: 1,
            auto_context_menu: false,
            auto_context_menu_value: 0,
            cursor_flash: 13,
            select_flash: 15,
            cursor_selection: 17,
            cursor_toggled: 19,
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let read = |entry: &str| {
                get_player_val(&[
                    Value::String(entry.into()),
                    Value::String("Player".into()),
                    Value::Int(7),
                ])
            };
            Ok::<_, RuntimeError>(Value::Array(vec![
                read("Status")?,
                read("Index")?,
                read("ID")?,
                read("Surrendered")?,
                read("ColorDw")?,
                read("Control")?,
                read("MouseControl")?,
                read("AutoStopControl")?,
                read("ViewWealth")?,
                read("ViewValue")?,
                read("ShowStartup")?,
                read("ProductionDelay")?,
                read("ProductionUnit")?,
                read("SelectCount")?,
                read("Cursor")?,
                read("ViewCursor")?,
                read("Captain")?,
                read("LastCom")?,
                read("LastComDel")?,
                read("PressedComs")?,
                read("LastComDownDouble")?,
                read("CursorFlash")?,
                read("SelectFlash")?,
                read("CursorSelection")?,
                read("CursorToggled")?,
                read("MessageStatus")?,
                read("MessageBuf")?,
                read("CrewCreated")?,
            ]))
        });

        assert_eq!(
            result.expect("GetPlayerVal control reads run"),
            Value::Array(vec![
                Value::Int(1),
                Value::Int(7),
                Value::Int(71),
                Value::Int(1),
                Value::Int(0xff12_3456_u32 as i32),
                Value::Int(3),
                Value::Int(2),
                Value::Int(1),
                Value::Int(21),
                Value::Int(22),
                Value::Bool(true),
                Value::Int(12),
                Value::Int(3),
                Value::Int(4),
                Value::Int(42),
                Value::Int(43),
                Value::Int(44),
                Value::Int(-2_147_483_630),
                Value::Int(7),
                Value::Int(11),
                Value::Int(9),
                Value::Int(13),
                Value::Int(15),
                Value::Int(17),
                Value::Int(19),
                Value::Int(5),
                Value::String("legacy message".into()),
                Value::Int(6),
            ])
        );
    }

    #[test]
    fn get_player_val_returns_nil_for_unmatched_cpp_reflection_paths() {
        // ValidPlr rejects missing players before reflection
        // (C4Script.cpp:4257), while C4ValueGetCompiler leaves its result
        // nil when no name/entry occurrence matches (C4Script.cpp:4042-4068).
        let player = PlayerState {
            id: 0,
            viewports: vec![PlayerViewport::new(Vector2::new(1, 2))],
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_player_val(&[Value::String("ViewX".into()), Value::Int(0), Value::Int(99)])?,
                get_player_val(&[
                    Value::String("Unknown".into()),
                    Value::Int(0),
                    Value::Int(0),
                ])?,
                get_player_val(&[
                    Value::String("ViewX".into()),
                    Value::String("Other".into()),
                    Value::Int(0),
                ])?,
                get_player_val(&[
                    Value::String("ViewX".into()),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1),
                ])?,
            ]))
        });

        assert_eq!(
            result.expect("GetPlayerVal misses remain nil"),
            Value::Array(vec![Value::Nil; 4])
        );
    }

    #[test]
    fn get_player_info_core_val_drives_hazard_recharge_control_style() {
        // FnGetPlayerInfoCoreVal reflects C4PlayerInfoCore and rejects invalid
        // players before lookup (C4Script.cpp:4266-4280). The saved
        // [Preferences] AutoStopControl entry is PrefControlStyle
        // (C4InfoCore.cpp:165-171), which GetPlrCoreJumpAndRunControl exposes
        // to Hazard's FxRechargeStop (planet/System.c4g/GetXVal.c:167).
        let mut player = PlayerState {
            id: 7,
            ..PlayerState::default()
        };
        player.control.control_style = true;
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    r#"
                    #strict
                    global func GetPlrCoreJumpAndRunControl(int plr)
                    {
                        return GetPlayerInfoCoreVal("AutoStopControl", "Preferences", plr);
                    }

                    global func FxRechargeStop(int controller)
                    {
                        if (GetPlrCoreJumpAndRunControl(controller)) return 1;
                        return 0;
                    }

                    global func Probe()
                    {
                        return [
                            FxRechargeStop(7),
                            GetPlayerInfoCoreVal("AutoStopControl", "Preferences", 99),
                            GetPlayerInfoCoreVal("AutoStopControl", "Player", 7),
                            GetPlayerInfoCoreVal("AutoStopControl", "Preferences", 7, 1)
                        ];
                    }
                    "#,
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("GetPlayerInfoCoreVal runs"),
            Value::Array(vec![Value::Int(1), Value::Nil, Value::Nil, Value::Nil,])
        );
    }

    #[test]
    fn get_player_team_returns_zero_for_valid_unteamed_player_like_cpp() {
        // FnGetPlayerTeam distinguishes a missing player (nil) from a valid
        // player with no team (integer zero; C4Script.cpp:5716-5728).
        let player = PlayerState {
            id: 7,
            name: "Eta".into(),
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(7)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_player_team(&args));
        assert_eq!(result.expect("GetPlayerTeam succeeds"), Value::Int(0));
    }

    #[test]
    fn get_team_config_returns_all_seven_int_values_and_logs_unknown() {
        // FnGetTeamConfig returns optional<C4ValueInt>: every boolean field
        // is integer 0/1, TeamDist is its raw enum integer, and an unknown
        // selector logs at error level before returning nil.
        let mut engine = crate::Engine::with_seed(0);
        engine.set_team_configuration(crate::TeamConfiguration {
            custom: true,
            active: false,
            allow_hostility_change: true,
            distribution: 4,
            allow_team_switch: true,
            auto_generate_teams: false,
            team_colors: true,
        });
        engine
            .register_definition(
                crate::Definition::from_script(
                    "TEAM",
                    "Team config probe",
                    r#"#strict 2
public func Probe()
{
    return [GetTeamConfig(1), GetTeamConfig(2), GetTeamConfig(3),
            GetTeamConfig(4), GetTeamConfig(5), GetTeamConfig(6),
            GetTeamConfig(7), GetTeamConfig(6, 123), GetTeamConfig(99)];
}
"#,
                )
                .expect("team config probe compiles"),
            )
            .expect("team config probe registers");
        let probe = engine
            .spawn_object(SpawnConfig::new("TEAM"))
            .expect("team config probe spawns");
        let probe_index = engine.find_object_index(probe).expect("probe exists");

        let records = Arc::new(Mutex::new(Vec::new()));
        let layer = RecordingLayer::new(Arc::clone(&records));
        let subscriber = Registry::default().with(layer);
        let value = subscriber::with_default(subscriber, || {
            engine
                .call_object_function(probe_index, "Probe", Vec::new())
                .expect("GetTeamConfig probe runs")
        });

        assert_eq!(
            value,
            Value::Array(vec![
                Value::Int(1),
                Value::Int(0),
                Value::Int(1),
                Value::Int(4),
                Value::Int(1),
                Value::Int(0),
                Value::Int(1),
                Value::Int(0),
                Value::Nil,
            ])
        );
        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level, Level::ERROR);
        assert_eq!(records[0].target, "clonk-script");
        assert_eq!(
            records[0].message,
            "GetTeamConfig: Unknown config value: 99"
        );
    }

    #[test]
    fn team_configuration_survives_engine_state_serialization() {
        let configuration = crate::TeamConfiguration {
            custom: true,
            active: true,
            allow_hostility_change: false,
            distribution: 3,
            allow_team_switch: true,
            auto_generate_teams: true,
            team_colors: true,
        };
        let mut engine = crate::Engine::with_seed(0);
        engine.set_team_configuration(configuration);
        let mut encoded = Vec::new();
        engine
            .capture_state()
            .to_writer(&mut encoded)
            .expect("team configuration serializes");
        let state = crate::EngineState::from_reader(encoded.as_slice())
            .expect("team configuration deserializes");
        let mut restored = crate::Engine::with_seed(1);
        restored
            .restore_state(&state)
            .expect("team configuration restores");

        let world = restored.host_world_context();
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(
                (1..=7)
                    .map(|query| get_team_config(&[Value::Int(query)]))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        });
        assert_eq!(
            result.expect("restored GetTeamConfig queries run"),
            Value::Array(vec![
                Value::Int(1),
                Value::Int(1),
                Value::Int(0),
                Value::Int(3),
                Value::Int(1),
                Value::Int(1),
                Value::Int(1),
            ])
        );
    }

    #[test]
    fn get_plr_jump_and_run_control_returns_control_style() {
        // FnGetPlrJumpAndRunControl (C4Script.cpp:2579-2583): returns
        // plr->ControlStyle (0 classic / 1 Jump'n'Run) for a valid player.
        let mut player = PlayerState {
            id: 9,
            ..PlayerState::default()
        };
        player.control.control_style = true;
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(9)];
        let (result, _) =
            with_effect_context(None, &[], world, 1, || get_plr_jump_and_run_control(&args));
        assert_eq!(
            result.expect("GetPlrJumpAndRunControl succeeds"),
            Value::Int(1)
        );
    }

    #[test]
    fn get_plr_down_double_returns_live_countdown_and_nil_for_missing_player() {
        // FnGetPlrDownDouble exposes LastComDownDouble without converting it
        // to bool (C4Script.cpp:2618-2622).
        let mut player = PlayerState {
            id: 9,
            ..PlayerState::default()
        };
        player.control.last_com_down_double = 7;
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_plr_down_double(&[Value::Int(9)])?,
                get_plr_down_double(&[Value::Int(42)])?,
            ]))
        });
        assert_eq!(
            result.expect("GetPlrDownDouble succeeds"),
            Value::Array(vec![Value::Int(7), Value::Nil])
        );
    }

    #[test]
    fn get_plr_jump_and_run_control_missing_player_is_minus_one() {
        // `plr ? plr->ControlStyle : -1` (C4Script.cpp:2582): an absent
        // player yields -1, never nil — the return type is C4ValueInt.
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            Vec::<PlayerState>::new(),
        );
        let args = [Value::Int(42)];
        let (result, _) =
            with_effect_context(None, &[], world, 1, || get_plr_jump_and_run_control(&args));
        assert_eq!(
            result.expect("GetPlrJumpAndRunControl succeeds"),
            Value::Int(-1)
        );
    }

    #[test]
    fn get_wealth_returns_player_wealth() {
        let player = PlayerState {
            id: 12,
            wealth: 87,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let args = [Value::Int(12)];
        let (result, _) = with_effect_context(None, &[], world, 1, || get_wealth(&args));
        assert_eq!(result.expect("GetWealth succeeds"), Value::Int(87));
    }

    #[test]
    fn set_wealth_clamps_and_records_player_command() {
        // FnSetWealth (C4Script.cpp:2761-2766): clamp-set to 0..=100000,
        // false for invalid players. (DoWealth's 10000 cap applies only to
        // the engine-internal adjust path, C4Player.cpp:905-915.)
        let player = PlayerState {
            id: 12,
            wealth: 87,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                set_wealth(&[Value::Int(12), Value::Int(150_000)])?,
                Value::Bool(true)
            );
            // The same callback observes the clamped value.
            assert_eq!(get_wealth(&[Value::Int(12)])?, Value::Int(100_000));
            assert_eq!(
                get_player_val(&[
                    Value::String("ViewWealth".into()),
                    Value::Int(0),
                    Value::Int(12),
                ])?,
                Value::Int(0),
                "FnSetWealth is a direct write and does not arm ViewWealth"
            );
            // Invalid player (C4Script.cpp:2763).
            assert_eq!(
                set_wealth(&[Value::Int(5), Value::Int(10)])?,
                Value::Bool(false)
            );
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("SetWealth succeeds");
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::SetWealth {
                player_id: 12,
                value: 100_000,
                show_change: false,
            }]
        ));
    }

    #[test]
    fn set_fow_nil_fills_arguments_and_updates_eliminated_players() {
        // FnSetFoW (C4Script.cpp:3671-3678) accepts the ordinary two
        // nil-filled C4Aul parameter slots, validates only that the player
        // exists, and immediately calls C4Player::SetFoW. SetFoW persists
        // both FogOfWar and ForceFogOfWar (C4Player.cpp:815-824,
        // 1580-1581), even for an eliminated player.
        let player = PlayerState {
            id: 0,
            status: crate::PlayerStatus::Eliminated,
            fog_of_war: true,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            // No arguments means SetFoW(nil, nil): disable player zero.
            assert_eq!(set_fow(&[])?, Value::Int(1));
            let disabled = HOST_CONTEXT.with(|cell| {
                let borrow = cell.borrow();
                let player = borrow
                    .as_ref()
                    .and_then(|context| context.player_state(0))
                    .expect("player zero remains visible in the callback");
                (player.fog_of_war, player.force_fog_of_war)
            });
            assert_eq!(disabled, (false, true));

            // A single argument nil-fills the player slot with zero.
            assert_eq!(set_fow(&[Value::Int(1)])?, Value::Int(1));
            let enabled = HOST_CONTEXT.with(|cell| {
                let borrow = cell.borrow();
                let player = borrow
                    .as_ref()
                    .and_then(|context| context.player_state(0))
                    .expect("player zero remains visible in the callback");
                (player.fog_of_war, player.force_fog_of_war)
            });
            assert_eq!(enabled, (true, true));

            // Missing players return integer false and record no write.
            assert_eq!(
                set_fow(&[Value::Bool(true), Value::Int(99)])?,
                Value::Int(0)
            );
            assert!(set_fow(&[Value::Bool(true), Value::Int(0), Value::Nil]).is_err());
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("SetFoW calls succeed");
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [
                PlayerCommand::SetFogOfWar {
                    player_id: 0,
                    enabled: false,
                },
                PlayerCommand::SetFogOfWar {
                    player_id: 0,
                    enabled: true,
                }
            ]
        ));

        assert_eq!(
            set_fow(&[Value::Bool(true), Value::Int(0)])
                .expect("a missing host context is not a script error"),
            Value::Int(0)
        );
    }

    #[test]
    fn set_player_show_control_position_persists_the_validated_player_value() {
        // FnSetPlrShowControlPos validates the player, assigns ShowControlPos,
        // and returns bool success (C4Script.cpp:2561-2566). Every Tutorial
        // uses it to place its command hint strip.
        let player = PlayerState {
            id: 0,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script("global func Probe() { return SetPlrShowControlPos(0, 2); }")
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            script
                .call("Probe", &[])
                .map_err(|error| RuntimeError::new(error.to_string()))
        });

        assert_eq!(
            result.expect("SetPlrShowControlPos runs"),
            Value::Bool(true)
        );
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::SetShowControlPosition {
                player_id: 0,
                position: 2,
            }]
        ));
    }

    #[test]
    fn set_player_show_control_encodes_and_validates_like_cpp() {
        // StringBitEval sets one bit per non-space/non-underscore byte at
        // its original string position (C4Script.cpp:209-216), while
        // FnSetPlrShowControl rejects invalid players and otherwise stores
        // that mask (C4Script.cpp:2546-2551).
        let player = PlayerState {
            id: 0,
            ..PlayerState::default()
        };
        assert_eq!(
            string_bit_eval("x_________x_________x"),
            1 | 1 << 10 | 1 << 20
        );
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            let mut script = clonk_script::Engine::new();
            register_host_functions(&mut script);
            script
                .load_script(
                    "global func Probe(int player, string controls) { return SetPlrShowControl(player, controls); }",
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;

            let valid = script
                .call("Probe", &[Value::Int(0), Value::String("x_ x".into())])
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            let encoded = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.player_state(0))
                    .map(|player| player.show_control)
            });
            let invalid = script
                .call("Probe", &[Value::Int(99), Value::String("xx".into())])
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            let unchanged = HOST_CONTEXT.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .and_then(|context| context.player_state(0))
                    .map(|player| player.show_control)
            });
            Ok::<_, RuntimeError>((valid, encoded, invalid, unchanged))
        });

        assert_eq!(
            result.expect("SetPlrShowControl calls run"),
            (Value::Bool(true), Some(9), Value::Bool(false), Some(9))
        );
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::SetShowControl {
                player_id: 0,
                mask: 9,
            }]
        ));
    }

    #[test]
    fn set_plr_show_command_validates_player_and_keeps_raw_command() {
        // FnSetPlrShowCommand accepts any int command, writes only for an
        // existing player, and returns bool (C4Script.cpp:2553-2559).
        let player = PlayerState {
            id: 0,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>((
                set_plr_show_command(&[Value::Int(0), Value::Int(17)])?,
                set_plr_show_command(&[Value::Int(99), Value::Int(23)])?,
            ))
        });

        assert_eq!(
            result.expect("SetPlrShowCommand calls run"),
            (Value::Bool(true), Value::Bool(false))
        );
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::SetShowCommand {
                player_id: 0,
                command: 17,
            }]
        ));
        assert_eq!(
            set_plr_show_command(&[Value::Int(0), Value::Int(5)])
                .expect("a missing host context is not a script error"),
            Value::Bool(false)
        );
    }

    #[test]
    fn plr_extra_data_round_trips_like_cpp() {
        // FnSetPlrExtraData/FnGetPlrExtraData (C4Script.cpp:4692-4747):
        // named C4Player::ExtraData slots; only nil/int/bool/id values
        // store; invalid names and players yield nil — MagiClonk's
        // Recruitment reads `GetPlrExtraData(iPlayer,
        // MCLK_ComboExtraDataName())` (Script.c:76).
        let player = PlayerState {
            id: 3,
            ..PlayerState::default()
        };
        let world = HostWorldContext::from_objects_with_players(
            Vec::<HostWorldObject>::new(),
            vec![player],
        );
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            // Unset name reads nil (no name list, :4741).
            assert_eq!(
                get_plr_extra_data(&[Value::Int(3), Value::String("MCLK_PrefCombo".into())])?,
                Value::Nil
            );
            // Set returns the stored value (:4731).
            assert_eq!(
                set_plr_extra_data(&[
                    Value::Int(3),
                    Value::String("MCLK_PrefCombo".into()),
                    Value::Int(2)
                ])?,
                Value::Int(2)
            );
            // The same callback reads the write back.
            assert_eq!(
                get_plr_extra_data(&[Value::Int(3), Value::String("MCLK_PrefCombo".into())])?,
                Value::Int(2)
            );
            // Invalid player (:4738), string payloads (:4706-4710) and
            // non-identifier names (:4697-4704) all yield nil.
            assert_eq!(
                get_plr_extra_data(&[Value::Int(9), Value::String("MCLK_PrefCombo".into())])?,
                Value::Nil
            );
            assert_eq!(
                set_plr_extra_data(&[
                    Value::Int(3),
                    Value::String("Slot".into()),
                    Value::String("text".into())
                ])?,
                Value::Nil
            );
            assert_eq!(
                set_plr_extra_data(&[
                    Value::Int(3),
                    Value::String("bad name!".into()),
                    Value::Int(1)
                ])?,
                Value::Nil
            );
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("extra data calls succeed");
        assert!(matches!(
            outcome.player_commands.as_slice(),
            [PlayerCommand::SetExtraData { player_id: 3, .. }]
        ));
    }

    #[test]
    fn crew_extra_data_getter_reads_persistent_values_and_nil_defaults_like_cpp() {
        // FnGetCrewExtraData (C4Script.cpp:4786-4800) defaults a nil crew to
        // the caller, reads exact-case named values from C4ObjectInfo, and
        // returns nil for an unknown name or an object without Info. The
        // getter may return a pre-existing string even though C++'s separate
        // SetCrewExtraData builtin rejects new string writes.
        let script = r#"#strict 2
func Probe(object crew, object info_less)
{
    return [GetCrewExtraData(0, "missing"),
            GetCrewExtraData(0, "number"),
            GetCrewExtraData(0, "text"),
            GetCrewExtraData(crew, "id"),
            GetCrewExtraData(crew, "NUMBER"),
            GetCrewExtraData(info_less, "number")];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        let mut definition = crate::Definition::from_script("CREW", "Crew", script)
            .expect("crew extra-data fixture compiles");
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("crew extra-data fixture registers");

        let mut start = crate::scenario::PlayerStart::default();
        start.ready_crew = vec![("CREW".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        let extra_data = vec![
            ("number".to_string(), Value::Int(17)),
            ("text".to_string(), Value::String("persisted".into())),
            ("id".to_string(), Value::C4Id("ROCK".into())),
        ];
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Extra data owner".to_string(),
                player_info_id: 1,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: vec![crate::player_file::CrewInfo {
                    id: "CREW".to_string(),
                    name: "Ada".to_string(),
                    physical: crate::PhysicalInfo::default(),
                    extra_data: extra_data.clone(),
                    ..Default::default()
                }],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("extra data owner joins");

        let crew = engine.player(0).expect("player exists").crew()[0];
        let info_less = engine
            .spawn_object(crate::SpawnConfig::new("CREW"))
            .expect("info-less comparison object spawns");
        let expected = Value::Array(vec![
            Value::Nil,
            Value::Int(17),
            Value::String("persisted".into()),
            Value::C4Id("ROCK".into()),
            Value::Nil,
            Value::Nil,
        ]);
        let probe = |engine: &mut crate::Engine| {
            let index = engine.find_object_index(crew).expect("crew remains live");
            engine
                .call_object_function(
                    index,
                    "Probe",
                    vec![
                        Value::Object(crew.as_u64()),
                        Value::Object(info_less.as_u64()),
                    ],
                )
                .expect("GetCrewExtraData probe runs")
        };
        assert_eq!(probe(&mut engine), expected);
        assert_eq!(
            engine
                .crew_object_info(crew)
                .expect("crew retains live info")
                .extra_data,
            extra_data
        );

        let json = serde_json::to_string(&engine.capture_state()).expect("state serializes");
        let state: crate::EngineState = serde_json::from_str(&json).expect("state deserializes");
        engine.restore_state(&state).expect("state restores");
        assert_eq!(probe(&mut engine), expected);
    }

    #[test]
    fn crew_extra_data_setter_validates_orders_and_persists_like_cpp() {
        // FnSetCrewExtraData (C4Script.cpp:4743-4784) writes nil/int/bool/ID
        // values into the exact C4ObjectInfo map. Overwrites retain slot
        // order; invalid names and string/object values return nil without
        // mutation; a nil object defaults to the caller.
        let script = r#"#strict 2
func Mutate(object crew, object info_less, id_value)
{
    var unset;
    return [SetCrewExtraData(crew, "visited", 1),
            GetCrewExtraData(crew, "visited"),
            SetCrewExtraData(0, "visited", 2),
            GetCrewExtraData(0, "visited"),
            SetCrewExtraData(crew, "flag", true),
            GetCrewExtraData(crew, "flag"),
            SetCrewExtraData(crew, "kind", id_value),
            GetCrewExtraData(crew, "kind"),
            SetCrewExtraData(crew, "empty", unset),
            SetCrewExtraData(crew, "", 3),
            SetCrewExtraData(crew, "bad name!", 3),
            SetCrewExtraData(crew, "visited", "blocked"),
            GetCrewExtraData(crew, "visited"),
            SetCrewExtraData(crew, "visited", crew),
            GetCrewExtraData(crew, "visited"),
            SetCrewExtraData(info_less, "visited", 9),
            GetCrewExtraData(info_less, "visited")];
}

func Read()
{
    return [GetCrewExtraData(0, "visited"),
            GetCrewExtraData(0, "flag"),
            GetCrewExtraData(0, "kind"),
            GetCrewExtraData(0, "empty")];
}

func Transfer(object donor, object recipient)
{
    return [SetCrewExtraData(donor, "before_transfer", 7),
            GrabObjectInfo(donor, recipient),
            SetCrewExtraData(recipient, "after_transfer", 8),
            GetCrewExtraData(recipient, "before_transfer"),
            GetCrewExtraData(recipient, "after_transfer"),
            GetCrewExtraData(donor, "before_transfer")];
}
"#;
        let mut engine = crate::Engine::with_seed(0);
        let mut definition = crate::Definition::from_script("CREW", "Crew", script)
            .expect("crew extra-data setter fixture compiles");
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("crew extra-data setter fixture registers");

        let mut start = crate::scenario::PlayerStart::default();
        start.ready_crew = vec![("CREW".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(crate::JoinPlayerConfig {
                name: "Extra data owner".to_string(),
                player_info_id: 1,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff0000,
                pref_color: 0,
                pref_position: 0,
                crew: vec![crate::player_file::CrewInfo {
                    id: "CREW".to_string(),
                    name: "Ada".to_string(),
                    physical: crate::PhysicalInfo::default(),
                    ..Default::default()
                }],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("extra data owner joins");

        let crew = engine.player(0).expect("player exists").crew()[0];
        let info_less = engine
            .spawn_object(crate::SpawnConfig::new("CREW"))
            .expect("info-less comparison object spawns");
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));
        let result = subscriber::with_default(subscriber, || {
            engine
                .call_object_function(
                    crew_index,
                    "Mutate",
                    vec![
                        Value::Object(crew.as_u64()),
                        Value::Object(info_less.as_u64()),
                        Value::C4Id("ROCK".into()),
                    ],
                )
                .expect("SetCrewExtraData probe runs")
        });
        assert_eq!(
            result,
            Value::Array(vec![
                Value::Int(1),
                Value::Int(1),
                Value::Int(2),
                Value::Int(2),
                Value::Bool(true),
                Value::Bool(true),
                Value::C4Id("ROCK".into()),
                Value::C4Id("ROCK".into()),
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Int(2),
                Value::Nil,
                Value::Int(2),
                Value::Nil,
                Value::Nil,
            ])
        );
        let records = records.lock().expect("log records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level, Level::ERROR);
        assert_eq!(records[0].target, "clonk-script");
        assert_eq!(
            records[0].message,
            "SetCrewExtraData: Ignoring invalid data name \"bad name!\"! Only alphanumerics, _ and - are allowed."
        );
        drop(records);

        let expected_slots = vec![
            ("visited".to_string(), Value::Int(2)),
            ("flag".to_string(), Value::Bool(true)),
            ("kind".to_string(), Value::C4Id("ROCK".into())),
            ("empty".to_string(), Value::Nil),
        ];
        let read_expected = Value::Array(vec![
            Value::Int(2),
            Value::Bool(true),
            Value::C4Id("ROCK".into()),
            Value::Nil,
        ]);
        let read = |engine: &mut crate::Engine| {
            let index = engine.find_object_index(crew).expect("crew remains live");
            engine
                .call_object_function(index, "Read", Vec::new())
                .expect("GetCrewExtraData readback runs")
        };
        assert_eq!(read(&mut engine), read_expected);
        assert_eq!(
            engine
                .crew_object_info(crew)
                .expect("crew retains live info")
                .extra_data,
            expected_slots
        );
        let state = engine.capture_state();
        let link = state.crew_info_links[&crew];
        assert_eq!(
            state.crew_info_rosters[&link.player_id][link.roster_index].extra_data,
            expected_slots
        );

        let json = serde_json::to_string(&state).expect("state serializes");
        let state: crate::EngineState = serde_json::from_str(&json).expect("state deserializes");
        engine.restore_state(&state).expect("state restores");
        assert_eq!(read(&mut engine), read_expected);
        assert_eq!(
            engine
                .crew_object_info(crew)
                .expect("crew retains restored info")
                .extra_data,
            expected_slots
        );

        let transfer = engine
            .call_object_function(
                engine.find_object_index(crew).expect("donor remains live"),
                "Transfer",
                vec![
                    Value::Object(crew.as_u64()),
                    Value::Object(info_less.as_u64()),
                ],
            )
            .expect("ExtraData follows GrabObjectInfo");
        assert_eq!(
            transfer,
            Value::Array(vec![
                Value::Int(7),
                Value::Bool(true),
                Value::Int(8),
                Value::Int(7),
                Value::Int(8),
                Value::Nil,
            ])
        );
        assert!(engine.crew_object_info(crew).is_none());
        let transferred_slots = engine
            .crew_object_info(info_less)
            .expect("recipient owns the transferred info")
            .extra_data
            .clone();
        assert_eq!(
            transferred_slots,
            expected_slots
                .into_iter()
                .chain([
                    ("before_transfer".to_string(), Value::Int(7)),
                    ("after_transfer".to_string(), Value::Int(8)),
                ])
                .collect::<Vec<_>>()
        );
        let state = engine.capture_state();
        assert_eq!(state.crew_info_links.get(&info_less), Some(&link));
        assert!(!state.crew_info_links.contains_key(&crew));
        assert_eq!(
            state.crew_info_rosters[&link.player_id][link.roster_index].extra_data,
            transferred_slots
        );
    }

    #[test]
    fn find_construction_site_stages_through_the_callers_vars_like_cpp() {
        // FnFindConstructionSite (C4Script.cpp:1958-1981): the start
        // position reads from Caller->NumVars[iVarX/iVarY]; a failing
        // ConstructionCheck runs FindConSiteSpot (hrange 20) and writes
        // the found spot back into the caller's slots; an immediate
        // ConstructionCheck hit returns true WITHOUT touching them.
        let mut landscape = Landscape::flat(200, 100);
        landscape.set_world_height(400);
        let expected = landscape
            .find_con_site_spot(50, 40, 20, 20, 20, |_, _, _, _| false)
            .expect("the flat surface has a site");
        let mut definitions = HashMap::new();
        definitions.insert(
            DefinitionId::from("HUT1"),
            DefinitionMetadata {
                category: 1,
                constructable: true,
                shape: Some(DefinitionRect::new(-10, -20, 20, 20)),
                ..DefinitionMetadata::default()
            },
        );
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            definitions,
            HashMap::new(),
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            let mut script = clonk_script::Engine::new();
            script.register_host_function("FindConstructionSite", find_construction_site);
            script
                .load_script(
                    r#"#strict 2
func Probe(definition) {
  // Start in mid-air (no ground support): the check fails, the
  // probe searches for the surface.
  Var(0) = 50; Var(1) = 40;
  var r = FindConstructionSite(definition, 0, 1);
  return([r, Var(0), Var(1)]);
}
func ProbeValid(definition) {
  // Free ground-level spot: the start-position check accepts and the
  // vars stay untouched (C4Script.cpp:1970-1971).
  Var(0) = 50; Var(1) = 100;
  return([FindConstructionSite(definition, 0, 1), Var(0), Var(1)]);
}
func ProbeBadIndex(definition) {
  // Var indices outside 0..C4AUL_MAX_Par-1 yield nil (:1964).
  return(FindConstructionSite(definition, 10, 1));
}
"#,
                )
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            let probed = script
                .call("Probe", &[Value::C4Id("HUT1".into())])
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            assert_eq!(
                probed,
                Value::Array(vec![
                    Value::Bool(true),
                    Value::Int(expected.0),
                    Value::Int(expected.1)
                ])
            );
            let valid = script
                .call("ProbeValid", &[Value::C4Id("HUT1".into())])
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            assert_eq!(
                valid,
                Value::Array(vec![Value::Bool(true), Value::Int(50), Value::Int(100)])
            );
            let bad_index = script
                .call("ProbeBadIndex", &[Value::C4Id("HUT1".into())])
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            assert_eq!(bad_index, Value::Nil);
            // Unknown definition ids fail like C4Id2Def (:1962).
            let unknown = script
                .call("Probe", &[Value::C4Id("XXXX".into())])
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            assert_eq!(
                unknown,
                Value::Array(vec![Value::Nil, Value::Int(50), Value::Int(40)])
            );
            Ok::<Value, RuntimeError>(Value::Nil)
        });
        result.expect("scripted probes succeed");
    }

