// Contiguous slice 4 of 11 of the `compat::tests` battery, spliced by
// `include!` from compat.rs so every test id stays `compat::tests::*`.
// Mostly: landscape, world, misc.

    #[test]
    fn reload_def_is_registered_and_returns_false_while_unsupported() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Explicit() { return ReloadDef(TEST); }\n\
                 func Local() { return ReloadDef(); }",
            )
            .expect("ReloadDef probes compile");

        assert_eq!(
            script
                .call("Explicit", &[])
                .expect("explicit reload executes"),
            Value::Int(0)
        );
        assert_eq!(
            script.call("Local", &[]).expect("local reload executes"),
            Value::Int(0)
        );
    }

    #[test]
    fn reload_particle_returns_false_without_a_reloadable_definition() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Named() { return ReloadParticle(\"Smoke\"); }\n\
                 func Unnamed() { return ReloadParticle(); }",
            )
            .expect("ReloadParticle probes compile");

        assert_eq!(
            script.call("Named", &[]).expect("named reload executes"),
            Value::Int(0)
        );
        assert_eq!(
            script
                .call("Unnamed", &[])
                .expect("unnamed reload executes"),
            Value::Int(0)
        );
    }

    // C4Script.cpp:5161-5165 -> C4Game::ReloadParticle (C4Game.cpp:2369-2394).
    //
    // The builtin returns its answer *synchronously*, which the staged-command
    // channel cannot do — so it answers from state seeded before the call (the
    // definitions carrying a Filename, and whether this is a network game),
    // the same shape `CreateObject` uses to return a reference to an object the
    // engine has not created yet. The engine then does the work.
    #[test]
    fn reload_particle_answers_synchronously_and_the_engine_applies_it() {
        let dir = tempfile::tempdir().expect("temp particle root");
        let group = dir.path().join("Smoke.c4d");
        std::fs::create_dir_all(&group).expect("create particle group");

        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Known() { return ReloadParticle(\"Smoke\"); }\n\
                 func Unknown() { return ReloadParticle(\"NoSuchParticle\"); }\n\
                 func Pathless() { return ReloadParticle(\"Sparks\"); }",
            )
            .expect("ReloadParticle probes compile");

        let mut engine = crate::Engine::new();
        // One definition backed by a real group, one with no Filename at all —
        // `C4ParticleDef::Reload` refuses the latter (C4Particles.cpp:197).
        let core = |name: &str| crate::particles::ParticleDefCore {
            name: name.to_string(),
            init_fn: "StdInit".to_string(),
            exec_fn: "StdExec".to_string(),
            draw_fn: "Std".to_string(),
            ..Default::default()
        };
        engine
            .particle_system
            .register_def(core("Smoke"), 4, 1.0)
            .expect("register a particle def");
        assert!(engine
            .particle_system
            .set_def_source_path("Smoke", Some(group.clone())));
        engine
            .particle_system
            .register_def(core("Sparks"), 4, 1.0)
            .expect("register a simulation-only particle def");

        let world = engine.host_world_context();
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            // A definition with a group answers true before the work happens.
            assert_eq!(
                script.call("Known", &[]).expect("known probe executes"),
                Value::Int(1)
            );
            // Every C++ false case still answers false.
            assert_eq!(
                script.call("Unknown", &[]).expect("unknown probe executes"),
                Value::Int(0)
            );
            assert_eq!(
                script.call("Pathless", &[]).expect("pathless probe executes"),
                Value::Int(0),
                "a def with no Filename can never reload"
            );
            Ok::<_, RuntimeError>(())
        });
        result.expect("live ReloadParticle probes execute");

        // Only the accepted name is staged, and the engine does the work once.
        assert_eq!(engine.apply_particle_reload_requests(), 0);
        assert!(
            engine.particle_system.get_def("Smoke").is_none(),
            "the group has no Particle.txt, so the reload failed and removed it"
        );
        assert_eq!(
            engine.apply_particle_reload_requests(),
            0,
            "the request is drained once, not replayed"
        );
    }

    // C4Script.cpp:5143-5159 -> C4Game::ReloadDef (C4Game.cpp:2322-2367).
    #[test]
    fn reload_def_answers_synchronously_and_defaults_to_the_callers_definition() {
        let dir = tempfile::tempdir().expect("temp group root");
        let group = dir.path().join("Rock.c4d");
        std::fs::create_dir_all(&group).expect("create definition group");
        std::fs::write(
            group.join("DefCore.txt"),
            "[DefCore]\nid=ROCK\nVersion=4,9,8\nName=Rock\n",
        )
        .expect("write DefCore");

        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Known() { return ReloadDef(ROCK); }\n\
                 func Pathless() { return ReloadDef(STON); }\n\
                 func Own() { return ReloadDef(); }",
            )
            .expect("ReloadDef probes compile");

        let mut engine = crate::Engine::new();
        for (id, path) in [("ROCK", Some(group.clone())), ("STON", None)] {
            let mut definition =
                crate::Definition::from_script(id.to_string(), id.to_string(), "")
                    .expect("script definition compiles");
            definition.set_source_path(path);
            engine
                .register_definition(definition)
                .expect("register definition");
        }

        let world = engine.host_world_context();
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            // A definition with a group answers true before the work happens.
            assert_eq!(
                script.call("Known", &[]).expect("known probe executes"),
                Value::Int(1)
            );
            // One with no Filename can never reload.
            assert_eq!(
                script.call("Pathless", &[]).expect("pathless probe executes"),
                Value::Int(0)
            );
            // `ReloadDef()` with no id and no calling object is a plain false,
            // not an error (`C4Script.cpp:5146-5151`).
            assert_eq!(
                script.call("Own", &[]).expect("own probe executes"),
                Value::Int(0)
            );
            Ok::<_, RuntimeError>(())
        });
        result.expect("live ReloadDef probes execute");

        // Only the accepted id was staged, and the engine does the work once.
        assert_eq!(engine.apply_definition_reload_requests(), 1);
        assert_eq!(
            engine.definition("ROCK").map(crate::Definition::name),
            Some("Rock"),
            "the reload rebuilt the definition from DefCore.txt on disk"
        );
        assert_eq!(
            engine.apply_definition_reload_requests(),
            0,
            "the request is drained once, not replayed"
        );
    }

    #[test]
    fn pause_game_requests_halt_and_toggle_but_is_a_replay_noop() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Halt() { PauseGame(); return 7; }\n\
                 func Toggle() { PauseGame(true); return 8; }\n\
                 func ReturnValue() { return PauseGame(false); }",
            )
            .expect("PauseGame probes compile");

        let mut engine = crate::Engine::new();
        let world = engine.host_world_context();
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            assert_eq!(
                script.call("Halt", &[]).expect("halt probe executes"),
                Value::Int(7)
            );
            assert_eq!(
                script.call("Toggle", &[]).expect("toggle probe executes"),
                Value::Int(8)
            );
            assert_eq!(
                script
                    .call("ReturnValue", &[])
                    .expect("return-value probe executes"),
                Value::Nil
            );
            Ok::<_, RuntimeError>(())
        });
        result.expect("live PauseGame probes execute");
        assert_eq!(
            engine.take_pause_game_requests(),
            vec![
                PauseGameRequest::Halt,
                PauseGameRequest::Toggle,
                PauseGameRequest::Halt,
            ]
        );
        assert!(engine.take_pause_game_requests().is_empty());

        engine.set_replay_control(true);
        let replay_world = engine.host_world_context();
        let (result, _) = with_effect_context(None, &[], replay_world, 1, || {
            assert_eq!(
                script
                    .call("Halt", &[])
                    .expect("replay halt probe executes"),
                Value::Int(7)
            );
            assert_eq!(
                script
                    .call("Toggle", &[])
                    .expect("replay toggle probe executes"),
                Value::Int(8)
            );
            Ok::<_, RuntimeError>(())
        });
        result.expect("replay PauseGame probes execute");
        assert!(engine.take_pause_game_requests().is_empty());

        let sync_requests = Rc::new(RefCell::new(Vec::new()));
        let sync_world = HostWorldContext::default()
            .with_control_sync_mode(true)
            .with_pause_game_requests(false, Rc::clone(&sync_requests));
        let (result, _) = with_effect_context(None, &[], sync_world, 1, || {
            script
                .call("Halt", &[])
                .expect("synchronized halt probe executes");
            Ok::<_, RuntimeError>(())
        });
        result.expect("non-replay synchronized PauseGame executes");
        assert_eq!(*sync_requests.borrow(), vec![PauseGameRequest::Halt]);
    }

    #[test]
    fn set_film_view_validates_in_live_games_and_requests_replay_retargets() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Probe() { return [SetFilmView(), SetFilmView(42), SetFilmView(-1)]; }",
            )
            .expect("SetFilmView probe compiles");

        let mut engine = crate::Engine::new();
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("film-view player registers");
        let call = |engine: &crate::Engine| {
            let (result, _) =
                with_effect_context(None, &[], engine.host_world_context(), 1, || {
                    script.call("Probe", &[])
                });
            result.expect("SetFilmView probe executes")
        };

        assert_eq!(
            call(&engine),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(true),
            ]),
            "validation precedes the live no-op and nil defaults to player zero"
        );
        assert!(engine.take_viewport_presentation_requests().is_empty());

        engine.set_replay_control(true);
        assert_eq!(
            call(&engine),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(true),
            ]),
            "replay Initialize still runs before physical viewport creation"
        );
        assert!(engine.take_viewport_presentation_requests().is_empty());

        engine.set_film_viewport_available(true);
        assert_eq!(
            call(&engine),
            Value::Array(vec![
                Value::Bool(true),
                Value::Bool(false),
                Value::Bool(true),
            ])
        );
        assert_eq!(
            engine.take_viewport_presentation_requests(),
            vec![
                crate::ViewportPresentationRequest::SetFilmView { player: 0 },
                crate::ViewportPresentationRequest::SetFilmView { player: OWNER_NONE },
            ],
            "invalid players never reach the app-owned viewport channel"
        );
        assert!(engine.take_viewport_presentation_requests().is_empty());

        let mut without_viewport = crate::Engine::new();
        without_viewport.set_replay_control(true);
        assert_eq!(
            call(&without_viewport),
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
            ]),
            "NO_OWNER remains valid when the replay has no viewport"
        );
        assert!(
            without_viewport
                .take_viewport_presentation_requests()
                .is_empty(),
            "an empty viewport list is a successful no-op sampled at call time"
        );
        without_viewport.set_film_viewport_available(true);
        assert_eq!(
            call(&without_viewport),
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
            ])
        );
        assert_eq!(
            without_viewport.take_viewport_presentation_requests(),
            vec![crate::ViewportPresentationRequest::SetFilmView { player: OWNER_NONE }],
            "an explicit observer viewport exists independently of local players"
        );
    }

    #[test]
    fn viewport_presentation_requests_preserve_cross_native_script_order() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\nfunc Probe() { SetViewOffset(0, 1, 2); SetFilmView(0); SetViewOffset(0, 3, 4); }",
            )
            .expect("viewport request probe compiles");

        let mut engine = crate::Engine::new();
        engine
            .register_player(crate::PlayerConfig::new(0, "Player"))
            .expect("viewport player registers");
        engine.set_replay_control(true);
        engine.set_film_viewport_available(true);
        let (result, _) = with_effect_context(None, &[], engine.host_world_context(), 1, || {
            script.call("Probe", &[])
        });
        result.expect("viewport request probe executes");
        assert_eq!(
            engine.take_viewport_presentation_requests(),
            vec![
                crate::ViewportPresentationRequest::SetViewOffset {
                    player: 0,
                    offset: Vector2::new(1, 2),
                },
                crate::ViewportPresentationRequest::SetFilmView { player: 0 },
                crate::ViewportPresentationRequest::SetViewOffset {
                    player: 0,
                    offset: Vector2::new(3, 4),
                },
            ]
        );
    }

    #[test]
    fn set_film_view_updates_player_targeted_sound_routing_within_the_call() {
        // SetFilmView mutates the first physical viewport synchronously
        // (C4Script.cpp:5134-5148), so a later Sound in the same callback sees
        // the new target through GraphicsSystem.GetViewport
        // (C4Script.cpp:2297-2309).
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Probe() { SetFilmView(1); Sound(\"NewView\", true, nil, 100, 2); Sound(\"OldView\", true, nil, 100, 1); }",
            )
            .expect("same-call film-view sound probe compiles");

        let mut engine = crate::Engine::new();
        engine
            .register_player(crate::PlayerConfig::new(0, "Old target"))
            .expect("old film-view target registers");
        engine
            .register_player(crate::PlayerConfig::new(1, "New target"))
            .expect("new film-view target registers");
        engine.set_local_players([]);
        engine.set_replay_control(true);
        engine.set_film_viewport_available(true);
        engine.set_physical_viewport_players([0]);

        let (result, outcome) =
            with_effect_context(None, &[], engine.host_world_context(), 1, || {
                script.call("Probe", &[])
            });
        result.expect("same-call film-view sound probe executes");
        let sounds = outcome
            .audio
            .events
            .iter()
            .filter_map(|event| match event {
                AudioCommand::PlaySound { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(sounds, vec!["NewView"]);
        assert_eq!(
            engine.take_viewport_presentation_requests(),
            vec![crate::ViewportPresentationRequest::SetFilmView { player: 1 }]
        );
    }

    #[test]
    fn set_film_view_sound_routing_survives_new_host_contexts() {
        // C++ mutates the physical viewport itself, so the new target remains
        // visible to callbacks later in the same tick before the app returns
        // to its presentation loop (C4Script.cpp:5134-5148,2297-2309).
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "#strict 3\n\
                 func Retarget() { SetFilmView(1); }\n\
                 func Probe() { Sound(\"NewContextView\", true, nil, 100, 2); Sound(\"OldContextView\", true, nil, 100, 1); }",
            )
            .expect("cross-context film-view sound probe compiles");

        let mut engine = crate::Engine::new();
        engine
            .register_player(crate::PlayerConfig::new(0, "Old target"))
            .expect("old film-view target registers");
        engine
            .register_player(crate::PlayerConfig::new(1, "New target"))
            .expect("new film-view target registers");
        engine.set_local_players([]);
        engine.set_replay_control(true);
        engine.set_film_viewport_available(true);
        engine.set_physical_viewport_players([0]);

        let (result, _) = with_effect_context(None, &[], engine.host_world_context(), 1, || {
            script.call("Retarget", &[])
        });
        result.expect("film view retarget executes");
        let (result, outcome) =
            with_effect_context(None, &[], engine.host_world_context(), 1, || {
                script.call("Probe", &[])
            });
        result.expect("later-context sound probe executes");
        let sounds = outcome
            .audio
            .events
            .iter()
            .filter_map(|event| match event {
                AudioCommand::PlaySound { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(sounds, vec!["NewContextView"]);
    }

    #[test]
    fn debug_builtin_script_profiler_emits_one_sorted_report_across_vm_calls() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script.register_host_function("ProfilerDelay", |_| {
            std::thread::sleep(std::time::Duration::from_millis(3));
            Ok(Value::Nil)
        });
        script
            .load_script(
                "#strict 3\n\
                 func BeginProfile() { return StartScriptProfiler(); }\n\
                 func AOuter() { return BInner(); }\n\
                 func BInner() { ProfilerDelay(); return 1; }\n\
                 func EndProfile() { return StopScriptProfiler(); }",
            )
            .expect("script-profiler report probes compile");
        let script = Arc::new(script);
        let world = HostWorldContext::default().with_scenario_script(Some(Arc::clone(&script)));
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));

        let (result, _) = subscriber::with_default(subscriber, || {
            with_effect_context(None, &[], world, 1, || {
                let call = |name: &str| {
                    script
                        .call(name, &[])
                        .map_err(|error| RuntimeError::new(error.to_string()))
                };
                assert_eq!(call("BeginProfile")?, Value::Bool(true));
                assert_eq!(call("AOuter")?, Value::Int(1));
                assert_eq!(call("EndProfile")?, Value::Nil);
                assert_eq!(call("EndProfile")?, Value::Nil);
                Ok::<_, RuntimeError>(())
            })
        });
        result.expect("script-profiler report probes execute");

        let records = records.lock().unwrap();
        let profiler = records
            .iter()
            .filter(|record| record.target == "clonk-script-profiler")
            .collect::<Vec<_>>();
        assert!(profiler.iter().all(|record| record.level == Level::INFO));
        assert_eq!(
            profiler
                .iter()
                .filter(|record| record.message == "Profiler statistics:")
                .count(),
            1,
            "a repeated inactive stop emits no second report"
        );
        assert_eq!(
            profiler
                .iter()
                .filter(|record| record.message == "==============================")
                .count(),
            2
        );

        let rows = profiler
            .iter()
            .filter(|record| record.message.contains("ms\t"))
            .collect::<Vec<_>>();
        let outer = rows
            .iter()
            .position(|record| record.message.ends_with("game AOuter"))
            .expect("outer function is profiled");
        let inner = rows
            .iter()
            .position(|record| record.message.ends_with("game BInner"))
            .expect("inner function is profiled");
        assert!(
            outer < inner,
            "inclusive outer time sorts before inner time"
        );
        let elapsed = rows
            .iter()
            .map(|record| {
                record.message[..record.message.find("ms\t").unwrap()]
                    .parse::<u128>()
                    .expect("profile row starts with milliseconds")
            })
            .collect::<Vec<_>>();
        assert!(
            elapsed.windows(2).all(|pair| pair[0] >= pair[1]),
            "profile rows are sorted descending"
        );
    }

    #[test]
    fn locate_func_lists_explicit_object_overload_sources_and_lines() {
        let mut appended = ScriptEngine::new();
        appended
            .load_script("#appendto BASE\n\npublic func Initialize() { return 2; }")
            .expect("append source compiles");

        let mut base = ScriptEngine::new();
        register_host_functions(&mut base);
        base.load_script(
            "public func Initialize() { return 1; }\n\
             func Probe(object target) { return LocateFunc(\"Initialize\", target); }\n\
             func ProbeDefinition() { return LocateFunc(\"Initialize\", 0, BASE); }\n\
             func Log() { return true; }\n\
             func ProbeNative(object target) { return LocateFunc(\"Log\", target); }",
        )
        .expect("base LocateFunc probe compiles");
        base.append_overrides_from(&appended);

        let appended = Arc::new(appended);
        let base = Arc::new(base);
        let world = HostWorldContext::from_objects(vec![resort_order_world_object(1, "BASE")])
            .with_definition_scripts(HashMap::from([("BASE".into(), Arc::clone(&base))]))
            .with_linked_script_hosts(vec![("APND".into(), Arc::clone(&appended))]);
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));

        let (result, _) = subscriber::with_default(subscriber, || {
            with_effect_context(None, &[], world, 2, || {
                let call = |name: &str, args: &[Value]| {
                    base.call(name, args)
                        .map_err(|error| RuntimeError::new(error.to_string()))
                };
                assert_eq!(call("Probe", &[Value::Object(1)])?, Value::Bool(true));
                assert_eq!(call("ProbeDefinition", &[])?, Value::Bool(true));
                call("ProbeNative", &[Value::Object(1)])
            })
        });
        assert_eq!(
            result.expect("LocateFunc probe succeeds"),
            Value::Bool(true)
        );

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 6);
        assert!(records.iter().all(|record| record.level == Level::INFO));
        assert!(records.iter().all(|record| record.target == "clonk-script"));
        assert_eq!(records[0].message, "Initialize (APND:2)");
        assert_eq!(records[1].message, "overloads Initialize (BASE:0)");
        assert_eq!(records[2].message, "Initialize (APND:2)");
        assert_eq!(records[3].message, "overloads Initialize (BASE:0)");
        assert_eq!(records[4].message, "Log (BASE:3)");
        assert_eq!(records[5].message, "overloads Log (engine)");
    }

    #[test]
    fn locate_func_global_caller_skips_declaring_host_local_chain_nodes() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                "func Pick() { return 1; }\n\
                 global func Pick() { return 2; }\n\
                 global func Probe() { return LocateFunc(\"Pick\"); }",
            )
            .expect("mixed local/global LocateFunc probe compiles");
        let globals = script
            .global_access_functions()
            .map(|(name, function)| (name.clone(), function.clone()))
            .collect::<rustc_hash::FxHashMap<_, _>>();
        script.set_global_functions(Some(Arc::new(globals)));
        let script = Arc::new(script);
        let world = HostWorldContext::default().with_scenario_script(Some(Arc::clone(&script)));
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));

        let (result, _) = subscriber::with_default(subscriber, || {
            with_effect_context(None, &[], world, 1, || {
                script
                    .call("Probe", &[])
                    .map_err(|error| RuntimeError::new(error.to_string()))
            })
        });
        assert_eq!(
            result.expect("global LocateFunc caller succeeds"),
            Value::Bool(true)
        );

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level, Level::INFO);
        assert_eq!(records[0].target, "clonk-script");
        assert_eq!(records[0].message, "Pick (Game.Script:1)");
    }

    #[test]
    fn locate_func_matches_name_context_missing_and_engine_diagnostics() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                #strict 3
                func MissingName() { return LocateFunc(); }
                func EmptyName(object target) { return LocateFunc("", target); }
                func MissingFunction(object target) { return LocateFunc("Missing", target); }
                func EngineFunction(object target) { return LocateFunc("Log", target); }
                func InvalidDefinition() { return LocateFunc("Anything", nil, BADD); }
                func WrongName(object target) { return LocateFunc(false, target); }
                func WrongObject() { return LocateFunc("Log", 0); }
                "#,
            )
            .expect("LocateFunc validation probes compile");
        let script = Arc::new(script);
        let world = HostWorldContext::from_objects(vec![resort_order_world_object(1, "BASE")])
            .with_definition_scripts(HashMap::from([("BASE".into(), Arc::clone(&script))]));
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = Registry::default().with(RecordingLayer::new(Arc::clone(&records)));

        let (result, _) = subscriber::with_default(subscriber, || {
            with_effect_context(None, &[], world, 2, || {
                let call = |name: &str, args: &[Value]| {
                    script
                        .call(name, args)
                        .map_err(|error| RuntimeError::new(error.to_string()))
                };
                assert_eq!(call("MissingName", &[])?, Value::Bool(false));
                // An allocated empty C4String is not a missing C4String*.
                assert_eq!(call("EmptyName", &[Value::Object(1)])?, Value::Bool(true));
                assert_eq!(
                    call("MissingFunction", &[Value::Object(1)])?,
                    Value::Bool(true)
                );
                assert_eq!(
                    call("EngineFunction", &[Value::Object(1)])?,
                    Value::Bool(true)
                );
                assert_eq!(call("InvalidDefinition", &[])?, Value::Bool(false));
                assert!(call("WrongName", &[Value::Object(1)])
                    .expect_err("strict-3 bool must not convert to string")
                    .to_string()
                    .contains("got \"bool\", but expected \"string\""));
                assert!(call("WrongObject", &[])
                    .expect_err("strict-3 integer zero must not convert to object")
                    .to_string()
                    .contains("got \"int\", but expected \"object\""));
                assert_eq!(
                    locate_func(&[Value::String("Anything".into())])?,
                    Value::Bool(false)
                );
                Ok::<_, RuntimeError>(())
            })
        });
        result.expect("LocateFunc validation probes succeed");

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 6);
        let diagnostics = records
            .iter()
            .map(|record| {
                (
                    record.level,
                    record.target.as_str(),
                    record.message.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            diagnostics,
            [
                (Level::ERROR, "clonk-script", "No func name"),
                (Level::ERROR, "clonk-script", "Func  not found"),
                (Level::ERROR, "clonk-script", "Func Missing not found"),
                (Level::INFO, "clonk-script", "Log (engine)"),
                (Level::ERROR, "clonk-script", "Invalid or unloaded def"),
                (Level::ERROR, "clonk-script", "No valid script context"),
            ]
        );
    }

    #[test]
    fn game_over_returns_true_only_once_per_context() {
        let (result, outcome) = with_effect_context_with_state(
            None,
            &[],
            HostWorldContext::default(),
            1,
            false,
            || {
                let first = game_over(&[])?;
                assert_eq!(first, Value::Bool(true));
                game_over(&[])
            },
        );
        let second = result.expect("GameOver second call succeeds");
        assert_eq!(second, Value::Bool(false));
        assert!(outcome.trigger_game_over);
    }

    #[test]
    fn effect_context_is_cleared_and_dropped_when_callback_panics() {
        let world = HostWorldContext::default();
        let world_lifetime = Rc::downgrade(
            world
                .master_order
                .get()
                .expect("fixture master order is initialized"),
        );
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = with_effect_context(None, &[], world, 1, || -> Result<(), RuntimeError> {
                panic!("effect callback panic probe")
            });
        }));

        assert!(panic.is_err());
        HOST_CONTEXT.with(|cell| {
            assert!(
                cell.borrow().is_none(),
                "panicking callback must clear its TLS host context"
            );
        });
        assert!(
            world_lifetime.upgrade().is_none(),
            "clearing TLS must drop the callback's world context"
        );

        let (result, _) =
            with_effect_context(None, &[], HostWorldContext::default(), 1, || game_over(&[]));
        assert_eq!(
            result.expect("a callback after panic cleanup succeeds"),
            Value::Bool(true)
        );
    }

    #[test]
    fn game_over_respects_existing_state() {
        let (result, outcome) =
            with_effect_context_with_state(None, &[], HostWorldContext::default(), 1, true, || {
                game_over(&[])
            });
        let value = result.expect("GameOver call succeeds");
        assert_eq!(value, Value::Bool(false));
        assert!(!outcome.trigger_game_over);
    }

    #[test]
    fn g_back_solid_returns_false_without_landscape() {
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            g_back_solid(&[Value::Int(0), Value::Int(0)])
        });
        let value = result.expect("GBackSolid without landscape succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_semi_solid_counts_liquids_like_cpp() {
        // GBackSemiSolid = DensitySemiSolid(GBackDensity) = density >=
        // C4M_SemiSolid(25) (C4Wrappers.h:73-76, C4Material.h:202): water
        // is semi-solid but NOT solid.
        let mut landscape = Landscape::flat(32, 20);
        landscape.set_liquid_column(
            5,
            vec![crate::landscape::LiquidSegment {
                top: 10,
                bottom: 19,
                material: None,
            }],
        );
        let world = || {
            world_with(
                Vec::<HostWorldObject>::new(),
                Some(landscape.clone()),
                HashMap::new(),
                HashMap::new(),
            )
        };
        let (semi, _) = with_effect_context(None, &[], world(), 1, || {
            g_back_semi_solid(&[Value::Int(5), Value::Int(15)])
        });
        assert_eq!(
            semi.expect("GBackSemiSolid succeeds"),
            Value::Bool(true),
            "water is semi-solid"
        );
        let (solid, _) = with_effect_context(None, &[], world(), 1, || {
            g_back_solid(&[Value::Int(5), Value::Int(15)])
        });
        assert_eq!(
            solid.expect("GBackSolid succeeds"),
            Value::Bool(false),
            "water is not solid"
        );
    }

    #[test]
    fn get_scenario_val_reflects_defaults_types_and_index_coercion() {
        // C4ValueGetCompiler sees fully defaulted fields, preserves their
        // primitive type, and flattens C4SVal as Std/Rnd/Min/Max.
        let query = |entry: &str, section: Value, entry_nr: Value| {
            let (result, _) =
                with_effect_context(None, &[], HostWorldContext::default(), 1, || {
                    get_scenario_val(&[Value::String(entry.into()), section, entry_nr])
                });
            result.expect("GetScenarioVal succeeds")
        };
        let landscape = || Value::String("Landscape".into());
        assert_eq!(query("MapZoom", landscape(), Value::Int(0)), Value::Int(10));
        assert_eq!(
            query("MapZoom", landscape(), Value::Bool(true)),
            Value::Int(0),
            "a bool entry_nr converts through the C4ValueInt parameter"
        );
        assert_eq!(query("MapZoom", landscape(), Value::Int(2)), Value::Int(5));
        assert_eq!(query("MapZoom", landscape(), Value::Int(3)), Value::Int(15));
        assert_eq!(query("MapZoom", landscape(), Value::Int(4)), Value::Nil);
        assert_eq!(query("MapZoom", landscape(), Value::Int(-1)), Value::Nil);

        assert_eq!(
            query("BottomOpen", landscape(), Value::Int(0)),
            Value::Bool(false)
        );
        assert_eq!(
            query("TopOpen", landscape(), Value::Int(0)),
            Value::Bool(true)
        );
        assert_eq!(query("LeftOpen", landscape(), Value::Int(0)), Value::Int(0));
        assert_eq!(
            query("RightOpen", landscape(), Value::Int(0)),
            Value::Int(0)
        );

        assert_eq!(
            query("Title", Value::String("Head".into()), Value::Nil),
            Value::String("Default Title".into())
        );
        assert_eq!(
            query("SaveGame", Value::String("Head".into()), Value::Nil),
            Value::Int(0)
        );
        assert_eq!(
            query("NetworkGame", Value::String("Head".into()), Value::Nil),
            Value::Bool(false)
        );
        assert_eq!(
            query("MissionAccess", Value::String("Head".into()), Value::Nil),
            Value::String(String::new().into())
        );
        assert_eq!(
            query("StandardCrew", Value::String("Player1".into()), Value::Nil),
            Value::Nil,
            "C4ID_None is C4V_Any"
        );
        assert_eq!(
            query("MapZoom", Value::String(String::new().into()), Value::Nil),
            Value::Int(10),
            "an empty section is the no-section form"
        );
        assert_eq!(
            query("mapzoom", landscape(), Value::Nil),
            Value::Nil,
            "runtime compiler names are case-sensitive"
        );
        assert_eq!(
            query(
                "StartupPlayerCount",
                Value::String("Head".into()),
                Value::Nil,
            ),
            Value::Nil,
            "StartupPlayerCount belongs to Game.Parameters, not C4Scenario"
        );
        assert_eq!(
            query("Volcano", Value::String("Weather".into()), Value::Nil,),
            Value::Nil
        );
        assert_eq!(
            query("Volcano", Value::String("Disasters".into()), Value::Nil,),
            Value::Int(0)
        );
    }

    #[test]
    fn g_back_solid_detects_surface_in_landscape() {
        let landscape = Landscape::flat(32, 10);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_solid(&[Value::Int(5), Value::Int(12)])
        });
        let value = result.expect("GBackSolid with landscape succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn g_back_solid_respects_surface_height() {
        let landscape = Landscape::flat(16, 20);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            g_back_solid(&[Value::Int(3), Value::Int(15)])
        });
        let value = result.expect("GBackSolid above surface succeeds");
        assert_eq!(value, Value::Bool(false));
    }

    #[test]
    fn g_back_solid_applies_object_relative_coordinates() {
        let object_id = ObjectId::new(7);
        let landscape = Landscape::flat(32, 12);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            8,
            false,
        );
        let object_context = HostObjectContext {
            id: object_id,
            position: Vector2::new(4, 6),
            ..idle_object_context()
        };
        let (result, _) = with_effect_context(Some(object_context), &[], world, 9, || {
            g_back_solid(&[Value::Int(0), Value::Int(7)])
        });
        let value = result.expect("GBackSolid with object context succeeds");
        assert_eq!(value, Value::Bool(true));
    }

    #[test]
    fn g_back_sky_reports_inverse_of_ift() {
        // C++ oracle: FnGBackSky returns !GBackIFT, not !GBackSolid
        // (src/C4Script.cpp:2252-2256). Ordinary solid earth is therefore
        // "sky" to this legacy query, while an IFT-marked tunnel is not.
        let mut landscape = Landscape::flat(20, 5);
        landscape.set_height(2, 2);
        landscape.set_tunnel_column(3, vec![(0, 4)]);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let (ordinary_solid, _) = with_effect_context(None, &[], world.clone(), 1, || {
            g_back_sky(&[Value::Int(2), Value::Int(2)])
        });
        let (tunnel, _) = with_effect_context(None, &[], world, 1, || {
            g_back_sky(&[Value::Int(3), Value::Int(2)])
        });
        assert_eq!(
            ordinary_solid.expect("GBackSky on ordinary solid succeeds"),
            Value::Bool(true)
        );
        assert_eq!(
            tunnel.expect("GBackSky in tunnel succeeds"),
            Value::Bool(false)
        );
    }

    #[test]
    fn get_material_returns_mnone_without_context() {
        let (result, _) = with_effect_context(None, &[], HostWorldContext::default(), 1, || {
            get_material(&[Value::Int(0), Value::Int(0)])
        });
        assert_eq!(
            result.expect("GetMaterial without context succeeds"),
            Value::Int(MATERIAL_NONE)
        );
    }

    #[test]
    fn get_material_reports_solid_material_from_landscape() {
        let material = crate::MaterialId::new(3).expect("material id");
        let mut landscape = Landscape::flat_with_material(32, 10, Some(material));
        landscape.set_world_height(20);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let (result, _) = with_effect_context(None, &[], world, 1, || {
            get_material(&[Value::Int(5), Value::Int(12)])
        });
        let expected = Value::Int(material.index() as i32);
        assert_eq!(
            result.expect("GetMaterial with landscape succeeds"),
            expected
        );
    }

    #[test]
    fn get_material_applies_object_relative_coordinates() {
        let material = crate::MaterialId::new(2).expect("material id");
        let mut landscape = Landscape::flat_with_material(24, 12, Some(material));
        landscape.set_world_height(20);
        let world = HostWorldContext::with_landscape(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            5,
            false,
        );
        let object_id = ObjectId::new(11);
        let object_context = HostObjectContext {
            id: object_id,
            position: Vector2::new(3, 4),
            direction: Direction::Right,
            ..idle_object_context()
        };
        let (result, _) = with_effect_context(Some(object_context), &[], world, 6, || {
            get_material(&[Value::Int(0), Value::Int(8)])
        });
        assert_eq!(
            result.expect("GetMaterial with object succeeds"),
            Value::Int(material.index() as i32)
        );
    }

    #[test]
    fn get_material_honours_vehicle_and_sky_borders_like_cpp() {
        // C++ oracle: FnGetMaterial -> GBackMat -> Landscape.GetMat, whose
        // GetPix maps closed borders to MCVehic and open borders to sky
        // (src/C4Script.cpp:2216-2220; src/C4Wrappers.h:164-167;
        // src/C4Landscape.h:144-175).
        let earth = crate::MaterialId::new(2).expect("earth id");
        let vehicle = crate::MaterialId::new(5).expect("vehicle id");
        let mut landscape = Landscape::flat_with_material(10, 5, Some(earth));
        landscape.set_world_height(20);
        landscape.set_vehicle_material(Some(vehicle));
        landscape.set_border_open(8, 0, false, false);
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );
        let query = |x, y| {
            let (result, _) = with_effect_context(None, &[], world.clone(), 1, || {
                get_material(&[Value::Int(x), Value::Int(y)])
            });
            result.expect("GetMaterial border query succeeds")
        };

        assert_eq!(query(4, 20), Value::Int(vehicle.index() as i32));
        assert_eq!(query(4, -1), Value::Int(vehicle.index() as i32));
        assert_eq!(query(-1, 8), Value::Int(vehicle.index() as i32));
        assert_eq!(query(-1, 7), Value::Int(MATERIAL_NONE));
    }

    fn draw_material_quad_world() -> HostWorldContext {
        let mut densities = vec![0; 128];
        densities[1] = 100;
        densities[2] = 100;
        densities[3] = 25;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        material_names[2] = Some("Vehicle".to_string());
        material_names[3] = Some("Water".to_string());
        let texture_names = vec![None; 128];
        let mut texmap = crate::landscape::RuntimeTexMapState {
            densities: densities.clone(),
            material_names: material_names.clone(),
            texture_names: texture_names.clone(),
            match_texture_names: texture_names.clone(),
            shapes: vec![None; 128],
            materials: vec![crate::landscape::RuntimeTexMapMaterial {
                name: "Water".to_string(),
                density: 25,
                shape: crate::chunky::ChunkShape::Flat,
            }],
            texture_inventory: Vec::new(),
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            ..Default::default()
        };
        texmap.set_default_material_entry("Water", 3);
        let mut landscape = Landscape::new(6, vec![6; 6]).expect("landscape builds");
        landscape.set_world_height(6);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            6,
            6,
            vec![0; 36],
            densities,
            material_names,
            texture_names,
        ));
        landscape.set_raster_state(crate::landscape::LandscapeRasterState::new(1, 0, texmap));
        landscape.refresh_all_raster_columns();
        world_with(Vec::<HostWorldObject>::new(), Some(landscape), HashMap::new(), HashMap::new())
    }

    fn draw_mat_chunks_world() -> HostWorldContext {
        let mut densities = vec![0; 128];
        densities[2] = 100;
        let mut material_names = vec![None; 128];
        material_names[2] = Some("Vehicle".to_string());
        let mut texture_names = vec![None; 128];
        texture_names[2] = Some("Smooth".to_string());
        let mut shapes = vec![None; 128];
        shapes[2] = Some(crate::chunky::ChunkShape::Flat);
        let mut texmap = crate::landscape::RuntimeTexMapState {
            densities: densities.clone(),
            material_names: material_names.clone(),
            texture_names: texture_names.clone(),
            match_texture_names: texture_names.clone(),
            shapes,
            materials: vec![crate::landscape::RuntimeTexMapMaterial {
                name: "Earth".to_string(),
                density: 100,
                shape: crate::chunky::ChunkShape::Rough,
            }],
            texture_inventory: vec!["Rough".to_string()],
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            ..Default::default()
        };
        texmap.set_default_material_entry("Vehicle", 2);
        let mut landscape = Landscape::new(16, vec![12; 16]).expect("landscape builds");
        landscape.set_world_height(12);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            16,
            12,
            vec![0; 16 * 12],
            densities,
            material_names,
            texture_names,
        ));
        landscape.set_raster_state(crate::landscape::LandscapeRasterState::new(1, 9, texmap));
        landscape.refresh_all_raster_columns();
        world_with(Vec::<HostWorldObject>::new(), Some(landscape), HashMap::new(), HashMap::new())
    }

    fn draw_mat_chunks_masked_world() -> HostWorldContext {
        let mut world = draw_mat_chunks_world();
        let landscape = world.landscape_mut().expect("landscape exists");
        for y in 4..6 {
            for x in 5..7 {
                landscape.grid_write_byte(x, y, 2);
            }
        }
        world.with_solid_mask_bakes(vec![(
            ObjectId::new(90),
            crate::SolidMaskBake {
                instance_sequence: 1,
                x: 5,
                y: 4,
                width: 2,
                height: 2,
                tx: 0,
                ty: 0,
                mask_width: 2,
                pixels: None,
                buffer: vec![0; 4],
                rotated: None,
            },
        )])
    }

    fn draw_volcano_branch_world() -> HostWorldContext {
        let mut densities = vec![0; 128];
        densities[1] = 100;
        densities[5] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        material_names[5] = Some("Earth".to_string());
        let mut texture_names = vec![None; 128];
        texture_names[1] = Some("Rough".to_string());
        texture_names[5] = Some("Smooth".to_string());
        let mut texmap = crate::landscape::RuntimeTexMapState {
            densities: densities.clone(),
            material_names: material_names.clone(),
            texture_names: texture_names.clone(),
            match_texture_names: texture_names.clone(),
            shapes: vec![None; 128],
            materials: vec![crate::landscape::RuntimeTexMapMaterial {
                name: "Earth".to_string(),
                density: 100,
                shape: crate::chunky::ChunkShape::Flat,
            }],
            texture_inventory: vec!["Rough".to_string(), "Smooth".to_string()],
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            ..Default::default()
        };
        // Deliberately differ from the first Earth slot: Mat2PixColDefault
        // must use DefaultMatTex, not a material-name scan.
        texmap.set_default_material_entry("Earth", 5);
        let bytes = (0..10)
            .flat_map(|y| (0..12).map(move |x| if (x + y) % 2 == 0 { 0x80 } else { 0 }))
            .collect::<Vec<_>>();
        let mut landscape = Landscape::new(12, vec![10; 12]).expect("landscape builds");
        landscape.set_world_height(10);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            12,
            10,
            bytes,
            densities,
            material_names,
            texture_names,
        ));
        landscape.set_raster_state(crate::landscape::LandscapeRasterState::new(1, 0, texmap));
        landscape.refresh_all_raster_columns();
        world_with(Vec::<HostWorldObject>::new(), Some(landscape), HashMap::new(), HashMap::new())
    }

    fn draw_map_world(
        width: u32,
        height: u32,
        zoom: i32,
        retain_creator: bool,
    ) -> HostWorldContext {
        let mut densities = vec![0; 128];
        densities[1] = 100;
        let mut material_names = vec![None; 128];
        material_names[1] = Some("Earth".to_string());
        let mut texture_names = vec![None; 128];
        texture_names[1] = Some("Rough".to_string());
        let mut shapes = vec![None; 128];
        shapes[1] = Some(crate::chunky::ChunkShape::Flat);
        let mut texmap = crate::landscape::RuntimeTexMapState {
            densities: densities.clone(),
            material_names: material_names.clone(),
            texture_names: texture_names.clone(),
            match_texture_names: texture_names.clone(),
            shapes,
            materials: vec![crate::landscape::RuntimeTexMapMaterial {
                name: "Earth".to_string(),
                density: 100,
                shape: crate::chunky::ChunkShape::Flat,
            }],
            texture_inventory: vec!["Rough".to_string(), "Ridge".to_string()],
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            ..Default::default()
        };
        texmap.set_default_material_entry("Earth", 1);

        let mut classifier = crate::scenario::MapPixelClassifier::from_runtime_state(texmap);
        let mut setup_rng = LcgRng::new(1);
        let retained = crate::map_creator_s2::create_s2_map_with_state(
            "overlay Named { mat = Earth; tex = Rough; wdt = 50; seed = 7; }; \
             map Original { seed = 5; Named; }; \
             overlay Half { mat = Earth; tex = Rough; wdt = 50; }; \
             map Requested { Half; }; \
             map Decoy { seed = 41; \
                 overlay { mat = Earth; tex = Rough; seed = 43; }; \
             };",
            &mut classifier,
            crate::scenario::LegacyC4SVal::new(8, 0, 8, 8),
            crate::scenario::LegacyC4SVal::new(4, 0, 4, 4),
            false,
            1,
            &mut setup_rng,
        )
        .creator;
        let texmap = classifier.into_runtime_state();
        let mut landscape =
            Landscape::new(width, vec![height as i32; width as usize]).expect("landscape builds");
        landscape.set_world_height(height as i32);
        let mut bytes = vec![0; width as usize * height as usize];
        if width > 2 && height > 2 {
            bytes[2 * width as usize + 1] = 1 | 0x80;
            bytes[2 * width as usize + 2] = 2;
        }
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            width,
            height,
            bytes,
            densities,
            material_names,
            texture_names,
        ));
        let mut raster = crate::landscape::LandscapeRasterState::new(zoom, 0, texmap);
        raster.set_map_creator(retain_creator.then_some(retained));
        landscape.set_raster_state(raster);
        landscape.refresh_all_raster_columns();
        world_with(Vec::<HostWorldObject>::new(), Some(landscape), HashMap::new(), HashMap::new())
    }

    fn draw_map_masked_world() -> HostWorldContext {
        let mut world = draw_map_world(8, 7, 3, true);
        let landscape = world.landscape_mut().expect("landscape exists");
        let mut texmap = landscape
            .raster_state()
            .expect("raster state exists")
            .texmap()
            .clone();
        texmap.densities[3] = 100;
        texmap.material_names[3] = Some("Vehicle".to_string());
        texmap.texture_names[3] = Some("Smooth".to_string());
        texmap.match_texture_names[3] = Some("Smooth".to_string());
        texmap.shapes[3] = Some(crate::chunky::ChunkShape::Flat);
        texmap
            .materials
            .push(crate::landscape::RuntimeTexMapMaterial {
                name: "Vehicle".to_string(),
                density: 100,
                shape: crate::chunky::ChunkShape::Flat,
            });
        texmap.texture_inventory.push("Smooth".to_string());
        texmap.set_default_material_entry("Vehicle", 3);
        assert!(landscape.replace_runtime_texmap_state(texmap));
        landscape.grid_write_byte(0, 1, 3);
        landscape.refresh_all_raster_columns();
        world.with_solid_mask_bakes(vec![(
            ObjectId::new(91),
            crate::SolidMaskBake {
                instance_sequence: 1,
                x: 0,
                y: 1,
                width: 1,
                height: 1,
                tx: 0,
                ty: 0,
                mask_width: 1,
                pixels: None,
                buffer: vec![0],
                rotated: None,
            },
        )])
    }

    fn remove_unused_texmap_world() -> HostWorldContext {
        let mut densities = vec![0; 128];
        let mut material_names = vec![None; 128];
        let mut texture_names = vec![None; 128];
        let mut shapes = vec![None; 128];
        for (slot, material, texture) in [
            (1, "Earth", "Rough"),
            (2, "Rock", "Rough"),
            (3, "Earth", "Spots"),
            (4, "Earth", "Ridge"),
            (5, "Earth", "Smooth"),
            (6, "Earth", "Cracked"),
        ] {
            densities[slot] = 100;
            material_names[slot] = Some(material.to_string());
            texture_names[slot] = Some(texture.to_string());
            shapes[slot] = Some(crate::chunky::ChunkShape::Flat);
        }
        let mut texmap = crate::landscape::RuntimeTexMapState {
            densities: densities.clone(),
            material_names: material_names.clone(),
            texture_names: texture_names.clone(),
            match_texture_names: texture_names.clone(),
            shapes,
            materials: Vec::new(),
            texture_inventory: Vec::new(),
            default_material_entries: Vec::new(),
            // Slot 3 models one of BlastShiftTo/BelowTempConvertTo/
            // AboveTempConvertTo; it has no Surface8 pixel of its own.
            material_crossmap_entries: vec![3],
            ..Default::default()
        };
        texmap.set_default_material_entry("Earth", 1);
        texmap.set_default_material_entry("Rock", 2);

        // Surface8 protects slots 1 and 5. The latter carries IFT, which the
        // native masks away while building its usage table.
        let bytes = vec![1, 5 | 0x80, 0, 1, 5, 0x80];
        let mut landscape = Landscape::new(3, vec![2; 3]).expect("landscape builds");
        landscape.set_world_height(2);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            3,
            2,
            bytes,
            densities,
            material_names,
            texture_names,
        ));
        landscape.set_raster_state(crate::landscape::LandscapeRasterState::new(1, 0, texmap));
        landscape.refresh_all_raster_columns();
        world_with(Vec::<HostWorldObject>::new(), Some(landscape), HashMap::new(), HashMap::new())
    }

    #[test]
    fn get_texture_uses_global_coordinates_masks_ift_and_nil_for_missing_entries() {
        // FnGetTexture reads GLOBAL GBackPix coordinates even in an object
        // context, strips the IFT bit through PixCol2Tex, and returns null for
        // sky or an unmapped TextureMap slot (C4Script.cpp:2222-2232).
        let object_context = HostObjectContext {
            id: ObjectId::new(11),
            position: Vector2::new(5, 4),
            direction: Direction::Right,
            ..idle_object_context()
        };
        let (result, _) = with_effect_context(
            Some(object_context),
            &[],
            draw_map_world(8, 7, 3, true),
            1,
            || {
                Ok::<_, RuntimeError>(Value::Array(vec![
                    get_texture(&[Value::Int(1), Value::Int(2)])?,
                    get_texture(&[Value::Int(0), Value::Int(0)])?,
                    get_texture(&[Value::Int(2), Value::Int(2)])?,
                ]))
            },
        );

        assert_eq!(
            result.expect("GetTexture succeeds"),
            Value::Array(vec![
                Value::String("Rough".to_string().into()),
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn get_texture_returns_raw_entry_name_without_changing_render_remap() {
        // C4TexMapEntry::Init remaps liquid Smooth to the Liquid SURFACE but
        // retains Texture="Smooth" for FnGetTexture (C4Texture.cpp:78-82;
        // C4Script.cpp:2222-2232). An explicitly declared Liquid entry keeps
        // that raw name, while sky and unmapped slots still return nil.
        let mut densities = vec![0; 128];
        densities[3] = 25;
        densities[25] = 25;
        let mut material_names = vec![None; 128];
        material_names[3] = Some("Water".to_string());
        material_names[25] = Some("Water".to_string());
        let mut render_texture_names = vec![None; 128];
        render_texture_names[3] = Some("Liquid".to_string());
        render_texture_names[25] = Some("Liquid".to_string());
        let mut raw_texture_names = vec![None; 128];
        raw_texture_names[3] = Some("Liquid".to_string());
        raw_texture_names[25] = Some("Smooth".to_string());
        let texmap = crate::landscape::RuntimeTexMapState {
            densities: densities.clone(),
            material_names: material_names.clone(),
            texture_names: render_texture_names.clone(),
            match_texture_names: raw_texture_names,
            shapes: vec![None; 128],
            materials: Vec::new(),
            texture_inventory: vec!["Liquid".to_string()],
            default_material_entries: Vec::new(),
            material_crossmap_entries: Vec::new(),
            ..Default::default()
        };
        let mut landscape = Landscape::new(4, vec![1; 4]).expect("landscape builds");
        landscape.set_world_height(1);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            4,
            1,
            vec![25, 3, 0, 7],
            densities,
            material_names,
            render_texture_names,
        ));
        assert_eq!(
            landscape.pixel_grid().expect("pixel grid").texture_names()[25].as_deref(),
            Some("Liquid"),
            "the frontend still sees the remapped Liquid render surface"
        );
        landscape.set_raster_state(crate::landscape::LandscapeRasterState::new(1, 0, texmap));
        let world = world_with(
            Vec::<HostWorldObject>::new(),
            Some(landscape),
            HashMap::new(),
            HashMap::new(),
        );

        let (result, _) = with_effect_context(None, &[], world, 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_texture(&[Value::Int(0), Value::Int(0)])?,
                get_texture(&[Value::Int(1), Value::Int(0)])?,
                get_texture(&[Value::Int(2), Value::Int(0)])?,
                get_texture(&[Value::Int(3), Value::Int(0)])?,
            ]))
        });
        assert_eq!(
            result.expect("GetTexture succeeds"),
            Value::Array(vec![
                Value::String("Smooth".to_string().into()),
                Value::String("Liquid".to_string().into()),
                Value::Nil,
                Value::Nil,
            ])
        );
    }

    #[test]
    fn set_texture_index_rejects_range_and_literal_insert_bug_paths() {
        // FnSetTextureIndex rejects values outside the wrapper's 0..=255
        // range before uint8 conversion. C4Landscape then rejects 128, and
        // its insertion room scan always rejects ordinary 1..=125 slots:
        // GetEntry returns a non-null pointer even for an empty entry
        // (C4Script.cpp:5071-5075; C4Landscape.cpp:2733-2755;
        // C4Texture.h:83).
        let (result, outcome) =
            with_effect_context(None, &[], draw_map_world(8, 7, 3, false), 1, || {
                Ok::<_, RuntimeError>(Value::Array(vec![
                    set_texture_index(&[Value::Nil, Value::Int(2), Value::Bool(false)])?,
                    set_texture_index(&[
                        Value::String("Earth-Rough".to_string().into()),
                        Value::Int(0),
                        Value::Bool(false),
                    ])?,
                    set_texture_index(&[
                        Value::String("Earth-Rough".to_string().into()),
                        Value::Int(127),
                        Value::Bool(false),
                    ])?,
                    set_texture_index(&[
                        Value::String("Earth-Rough".to_string().into()),
                        Value::Int(300),
                        Value::Bool(false),
                    ])?,
                    set_texture_index(&[
                        Value::String("Earth-Rough".to_string().into()),
                        Value::Int(128),
                        Value::Bool(false),
                    ])?,
                    set_texture_index(&[
                        Value::String("Earth-Rough".to_string().into()),
                        Value::Int(1),
                        Value::Bool(false),
                    ])?,
                    set_texture_index(&[
                        Value::String("Missing-Rough".to_string().into()),
                        Value::Int(2),
                        Value::Bool(false),
                    ])?,
                    set_texture_index(&[
                        Value::String("Earth-Ridge".to_string().into()),
                        Value::Int(2),
                        Value::Bool(true),
                    ])?,
                    // This is the one defined index-127 insertion arm: an
                    // empty mat/tex is a successful no-op. Nonempty 126/127
                    // paths reach native one-past-array writes and are
                    // deliberately contained.
                    set_texture_index(&[Value::Nil, Value::Int(127), Value::Bool(true)])?,
                    get_texture(&[Value::Int(2), Value::Int(2)])?,
                ]))
            });

        assert_eq!(
            result.expect("SetTextureIndex rejection paths succeed"),
            Value::Array(vec![
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
                Value::Nil,
            ])
        );
        assert!(outcome.landscape.is_empty());
    }

    #[test]
    fn set_texture_index_exact_landscape_without_retained_map_moves_entry_only() {
        let mut world = draw_map_world(8, 7, 1, false);
        let landscape = world.landscape_mut().expect("landscape exists");
        assert!(landscape.set_mode(crate::landscape::LANDSCAPE_MODE_EXACT));
        assert!(
            landscape
                .raster_state()
                .and_then(|state| state.map())
                .is_none(),
            "exact landscape has no retained C4Landscape::Map"
        );
        let initial_landscape = landscape.clone();
        let initial_surface = landscape
            .pixel_grid()
            .expect("pixel grid exists")
            .bytes()
            .to_vec();

        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            set_texture_index(&[
                Value::String("Earth-Rough".to_string().into()),
                Value::Int(2),
                Value::Bool(false),
            ])
        });
        assert_eq!(result.expect("SetTextureIndex succeeds"), Value::Int(1));
        assert_eq!(outcome.landscape.len(), 1);

        let mut engine = crate::Engine::new();
        engine.set_landscape(initial_landscape);
        engine.apply_landscape_operations(outcome.landscape);

        let landscape = engine.landscape().expect("folded landscape exists");
        assert_eq!(
            landscape.pixel_grid().expect("pixel grid").bytes(),
            initial_surface
        );
        let raster = landscape.raster_state().expect("raster state exists");
        assert!(raster.map().is_none());
        assert_eq!(raster.texmap().material_names[1].as_deref(), Some("Earth"));
        assert_eq!(raster.texmap().material_names[2].as_deref(), Some("Earth"));
        assert_eq!(raster.texmap().texture_names[2].as_deref(), Some("Rough"));
    }

    #[test]
    fn set_texture_index_remap_is_live_copied_and_threaded_between_callbacks() {
        // Slot 2 is deliberately absent from the texmap but present in
        // Surface8 at (2,2). A successful ReplaceMapColor + MoveIndex makes
        // GetTexture observe Earth-Rough there immediately and rewrites only
        // the retained editor map; Surface8, Pix2* caches, and the source slot
        // stay unchanged until a static redraw (C4Landscape.cpp:2710-2731,
        // 2787-2808; C4Texture.cpp:313-317).
        let mut replay_world = draw_map_world(8, 7, 1, false);
        let mut retained_indices = vec![0; 8 * 7];
        retained_indices[..6].copy_from_slice(&[1, 0x81, 3, 0x83, 0, 0x80]);
        let retained_map = clonk_resources::bitmap::IndexedBitmap {
            width: 8,
            height: 7,
            indices: retained_indices,
        };
        let replay_landscape = replay_world.landscape_mut().expect("landscape exists");
        assert!(replay_landscape.set_mode(crate::landscape::LANDSCAPE_MODE_STATIC));
        replay_landscape
            .raster_state_mut()
            .expect("raster state exists")
            .set_map(&retained_map);
        let initial_landscape = replay_world
            .landscape_ref()
            .expect("landscape exists")
            .clone();
        let initial_grid = initial_landscape.pixel_grid().expect("pixel grid exists");
        let initial_surface = initial_grid.bytes().to_vec();
        let initial_revision = initial_grid.revision();
        let initial_material_names = initial_grid.material_names().to_vec();
        let initial_texture_names = initial_grid.texture_names().to_vec();
        let (result, outcome) = with_effect_context(None, &[], replay_world.clone(), 1, || {
            Ok::<_, RuntimeError>(Value::Array(vec![
                get_texture(&[Value::Int(2), Value::Int(2)])?,
                set_texture_index(&[
                    Value::String("Earth-Rough".to_string().into()),
                    Value::Int(2),
                    Value::Bool(false),
                ])?,
                get_texture(&[Value::Int(2), Value::Int(2)])?,
                get_texture(&[Value::Int(1), Value::Int(2)])?,
                set_texture_index(&[
                    Value::String("Earth-Rough".to_string().into()),
                    Value::Int(2),
                    Value::Bool(false),
                ])?,
            ]))
        });

        assert_eq!(
            result.expect("SetTextureIndex and GetTexture succeed"),
            Value::Array(vec![
                Value::Nil,
                Value::Int(1),
                Value::String("Rough".to_string().into()),
                Value::String("Rough".to_string().into()),
                Value::Int(0),
            ])
        );
        assert_eq!(outcome.landscape.len(), 1);
        let LandscapeOperation::SetTextureIndex {
            texmap,
            old_index,
            new_index,
        } = &outcome.landscape[0]
        else {
            panic!("unexpected landscape operation: {:?}", outcome.landscape[0]);
        };
        assert_eq!((*old_index, *new_index), (1, 2));
        for slot in [1, 2] {
            assert_eq!(texmap.material_names[slot].as_deref(), Some("Earth"));
            assert_eq!(texmap.texture_names[slot].as_deref(), Some("Rough"));
            assert_eq!(texmap.match_texture_names[slot].as_deref(), Some("Rough"));
            assert_eq!(texmap.densities[slot], 100);
            assert_eq!(texmap.shapes[slot], Some(crate::chunky::ChunkShape::Flat));
        }
        assert_eq!(texmap.default_material_entry("Earth"), Some(1));

        // Effect callbacks are separate host contexts. Threading the first
        // callback's captured entry state and map-color delta must make slot
        // 2 visible to the next one before the authoritative Engine fold.
        replay_world.preview_runtime_landscape_operation(&outcome.landscape[0]);
        let preview_landscape = replay_world
            .landscape_ref()
            .expect("preview landscape exists");
        assert_eq!(
            preview_landscape
                .raster_state()
                .and_then(|state| state.map())
                .expect("preview retained map")
                .indices[..6],
            [2, 0x82, 3, 0x83, 0, 0x80]
        );
        let preview_grid = preview_landscape.pixel_grid().expect("preview pixel grid");
        assert_eq!(preview_grid.bytes(), initial_surface);
        assert_eq!(preview_grid.revision(), initial_revision);
        assert_eq!(preview_grid.material_names(), initial_material_names);
        assert_eq!(preview_grid.texture_names(), initial_texture_names);
        let (replayed, replayed_outcome) = with_effect_context(None, &[], replay_world, 1, || {
            Ok::<_, RuntimeError>((
                get_texture(&[Value::Int(2), Value::Int(2)])?,
                set_texture_index(&[
                    Value::String("Earth-Rough".to_string().into()),
                    Value::Int(2),
                    Value::Bool(false),
                ])?,
            ))
        });
        assert_eq!(
            replayed.expect("replayed callback sees texmap"),
            (Value::String("Rough".to_string().into()), Value::Int(0))
        );
        assert!(replayed_outcome.landscape.is_empty());

        let mut engine = crate::Engine::new();
        engine.set_landscape(initial_landscape);
        engine.apply_landscape_operations(outcome.landscape);
        {
            let landscape = engine.landscape().expect("folded landscape exists");
            assert_eq!(landscape.grid_byte_at(1, 2), Some(1 | 0x80));
            assert_eq!(landscape.grid_byte_at(2, 2), Some(2));
            let grid = landscape.pixel_grid().expect("folded pixel grid");
            assert_eq!(grid.bytes(), initial_surface);
            assert_eq!(grid.revision(), initial_revision);
            assert_eq!(grid.material_names(), initial_material_names);
            assert_eq!(grid.texture_names(), initial_texture_names);
            let raster = landscape.raster_state().expect("raster state");
            assert_eq!(
                raster.map().expect("folded retained map").indices[..6],
                [2, 0x82, 3, 0x83, 0, 0x80]
            );
            let folded = raster.texmap();
            assert_eq!(folded.material_names[1].as_deref(), Some("Earth"));
            assert_eq!(folded.material_names[2].as_deref(), Some("Earth"));
            assert_eq!(folded.default_material_entry("Earth"), Some(1));
        }

        engine.set_editor_landscape_mode(crate::landscape::LANDSCAPE_MODE_EXACT);
        engine.set_editor_landscape_mode(crate::landscape::LANDSCAPE_MODE_STATIC);
        let redrawn = engine.landscape().expect("redrawn landscape exists");
        assert_eq!(redrawn.grid_byte_at(0, 0), Some(2));
        assert_eq!(redrawn.grid_byte_at(1, 0), Some(0x82));
    }

    #[test]
    fn remove_unused_texmap_entries_prunes_live_entries_and_preserves_cpp_caches() {
        // C4Landscape::RemoveUnusedTexMapEntries scans Surface8, strips IFT,
        // protects numeric material references, then clears every other slot
        // from 1 through 126 without HandleTexMapUpdate
        // (C4Landscape.cpp:2983-3007).
        let mut replay_world = remove_unused_texmap_world();
        let initial_landscape = replay_world
            .landscape_ref()
            .expect("landscape exists")
            .clone();
        let initial_bytes = initial_landscape
            .pixel_grid()
            .expect("pixel grid")
            .bytes()
            .to_vec();
        let initial_revision = initial_landscape
            .pixel_grid()
            .expect("pixel grid")
            .revision();

        let (result, outcome) = with_effect_context(None, &[], replay_world.clone(), 1, || {
            Ok::<_, RuntimeError>((
                remove_unused_texmap_entries(&[])?,
                // Slot 4 was Earth-Ridge but had neither a pixel nor a
                // material reference. Its removal is immediately live.
                set_texture_index(&[
                    Value::String("Earth-Ridge".to_string().into()),
                    Value::Int(7),
                    Value::Bool(false),
                ])?,
                get_texture(&[Value::Int(1), Value::Int(0)])?,
            ))
        });
        assert_eq!(
            result.expect("RemoveUnusedTexMapEntries succeeds"),
            (
                Value::Nil,
                Value::Int(0),
                Value::String("Smooth".to_string().into()),
            )
        );
        assert_eq!(outcome.landscape.len(), 1);
        let LandscapeOperation::RemoveUnusedTexMapEntries { cleared_slots } = &outcome.landscape[0]
        else {
            panic!("unexpected landscape operation: {:?}", outcome.landscape[0]);
        };
        assert_eq!(cleared_slots.len(), 122);
        assert_eq!(cleared_slots.first(), Some(&4));
        assert_eq!(cleared_slots.get(1), Some(&6));
        assert_eq!(cleared_slots.last(), Some(&126));
        assert!(!cleared_slots.contains(&5));
        assert!(!cleared_slots.contains(&127));

        // Separate effect callbacks clone the threaded HostWorldContext.
        replay_world.preview_runtime_landscape_operation(&outcome.landscape[0]);
        let replayed_landscape = replay_world.landscape_ref().expect("landscape exists");
        let replayed_texmap = replayed_landscape
            .raster_state()
            .expect("raster state")
            .texmap();
        for slot in [1, 2, 3, 5] {
            assert!(
                replayed_texmap.material_names[slot].is_some(),
                "slot {slot}"
            );
        }
        for slot in [4, 6] {
            assert!(
                replayed_texmap.material_names[slot].is_none(),
                "slot {slot}"
            );
            assert!(replayed_texmap.texture_names[slot].is_none(), "slot {slot}");
            assert!(
                replayed_texmap.match_texture_names[slot].is_none(),
                "slot {slot}"
            );
            assert!(replayed_texmap.shapes[slot].is_none(), "slot {slot}");
            assert_eq!(replayed_texmap.densities[slot], 0, "slot {slot}");
        }
        assert_eq!(replayed_texmap.default_material_entry("Rock"), Some(2));
        assert_eq!(replayed_texmap.material_crossmap_entries, vec![3]);

        let (replayed, replayed_outcome) = with_effect_context(None, &[], replay_world, 1, || {
            set_texture_index(&[
                Value::String("Earth-Ridge".to_string().into()),
                Value::Int(7),
                Value::Bool(false),
            ])
        });
        assert_eq!(replayed.expect("replayed removal is live"), Value::Int(0));
        assert!(replayed_outcome.landscape.is_empty());

        let mut engine = crate::Engine::new();
        engine.set_landscape(initial_landscape);
        engine.apply_landscape_operations(outcome.landscape);
        let landscape = engine.landscape().expect("folded landscape exists");
        let folded = landscape.raster_state().expect("raster state").texmap();
        assert!(folded.material_names[4].is_none());
        assert!(folded.material_names[6].is_none());
        let grid = landscape.pixel_grid().expect("pixel grid");
        assert_eq!(grid.bytes(), initial_bytes);
        assert_eq!(grid.revision(), initial_revision);
        assert_eq!(
            grid.material_names()[4].as_deref(),
            Some("Earth"),
            "RemoveEntry does not refresh C++ Pix2Mat-style caches"
        );
        assert_eq!(grid.texture_names()[4].as_deref(), Some("Ridge"));

        // DrawMaterialQuad mutates Surface8 synchronously. RemoveUnused must
        // therefore see the new slot-4 pixel in this same callback and keep
        // that texture entry live.
        let ordered_world = remove_unused_texmap_world();
        let ordered_initial = ordered_world
            .landscape_ref()
            .expect("landscape exists")
            .clone();
        let draw_args = [
            Value::String("Earth-Ridge".to_string().into()),
            Value::Int(1),
            Value::Int(0),
            Value::Int(2),
            Value::Int(0),
            Value::Int(2),
            Value::Int(1),
            Value::Int(1),
            Value::Int(1),
            Value::Bool(false),
        ];
        let (ordered_result, ordered_outcome) =
            with_effect_context(None, &[], ordered_world, 1, || {
                Ok::<_, RuntimeError>((
                    draw_material_quad(&draw_args)?,
                    remove_unused_texmap_entries(&[])?,
                ))
            });
        assert_eq!(
            ordered_result.expect("ordered operations succeed"),
            (Value::Bool(true), Value::Nil)
        );
        assert_eq!(ordered_outcome.landscape.len(), 2);
        let LandscapeOperation::RemoveUnusedTexMapEntries { cleared_slots } =
            &ordered_outcome.landscape[1]
        else {
            panic!(
                "unexpected ordered operation: {:?}",
                ordered_outcome.landscape[1]
            );
        };
        assert!(
            !cleared_slots.contains(&4),
            "callback preview exposes the earlier quad"
        );

        let mut ordered_engine = crate::Engine::new();
        ordered_engine.set_landscape(ordered_initial);
        ordered_engine.apply_landscape_operations(ordered_outcome.landscape);
        let ordered_landscape = ordered_engine
            .landscape()
            .expect("ordered landscape remains");
        assert_eq!(ordered_landscape.grid_byte_at(2, 0), Some(4));
        assert_eq!(
            ordered_landscape
                .raster_state()
                .expect("ordered raster state")
                .texmap()
                .material_names[4]
                .as_deref(),
            Some("Earth")
        );
    }

    #[test]
    fn remove_unused_texmap_dirty_flag_survives_later_full_state_operation() {
        // Effect callbacks thread a COW preview before their operations fold
        // into the authoritative engine. RemoveUnused's fEntriesAdded bit
        // must survive a later operation that carries the whole texmap.
        let mut replay_world = draw_volcano_branch_world();
        let initial_landscape = replay_world
            .landscape_ref()
            .expect("landscape exists")
            .clone();
        assert!(!initial_landscape.texture_map_entries_added());

        let (removed, mut removed_outcome) =
            with_effect_context(None, &[], replay_world.clone(), 1, || {
                remove_unused_texmap_entries(&[])
            });
        assert_eq!(removed.expect("RemoveUnused succeeds"), Value::Nil);
        assert_eq!(removed_outcome.landscape.len(), 1);
        replay_world.preview_runtime_landscape_operation(&removed_outcome.landscape[0]);

        // Slot 5 already contains Earth-Smooth and is protected as Earth's
        // default. Zero chunk count therefore allocates nothing and draws no
        // pixels, but DrawMatChunks still captures the complete texmap.
        let chunk_args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(8),
            Value::Int(6),
            Value::Int(0),
            Value::Int(0),
            Value::String("Earth".to_string().into()),
            Value::String("Smooth".to_string().into()),
            Value::Bool(false),
        ];
        let (drew, chunks_outcome) =
            with_effect_context(None, &[], replay_world, 1, || draw_mat_chunks(&chunk_args));
        assert_eq!(drew.expect("DrawMatChunks succeeds"), Value::Int(1));
        assert_eq!(chunks_outcome.landscape.len(), 1);

        removed_outcome.landscape.extend(chunks_outcome.landscape);
        let mut engine = crate::Engine::new();
        engine.set_landscape(initial_landscape);
        engine.apply_landscape_operations(removed_outcome.landscape);
        assert!(
            engine
                .landscape()
                .expect("folded landscape exists")
                .texture_map_entries_added(),
            "the later full-state fold must not clear RemoveUnused's sticky flag"
        );
    }

    #[test]
    fn draw_map_clips_before_rounding_and_resolves_retained_template() {
        // FnDrawMap passes GLOBAL coordinates through unchanged
        // (C4Script.cpp:4851-4855). DrawMap first ClipRects the requested
        // pixel rect, then builds a temporary map of ceil(w/MapZoom) by
        // ceil(h/MapZoom) and clones pMapCreator so named templates resolve
        // (C4Landscape.cpp:2636-2663,2698-2707).
        let args = [
            Value::Int(-2),
            Value::Int(1),
            Value::Int(7),
            Value::Int(5),
            Value::String("map Runtime { seed = 9; Named; };".to_string().into()),
        ];
        let guard = enter_random_context(LcgRng::new(17));
        let (result, outcome) =
            with_effect_context(None, &[], draw_map_world(8, 7, 3, true), 1, || {
                draw_map(&args)
            });
        let final_rng = guard.finish();

        assert_eq!(result.expect("DrawMap succeeds"), Value::Int(1));
        assert_eq!(final_rng.count, 2, "exact FakeLS size draws occur once");
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DrawMap {
                origin,
                bitmap,
                map_width,
                map_height,
                texmap,
                ..
            } => {
                assert_eq!(*origin, Vector2::new(0, 1));
                assert_eq!((*map_width, *map_height), (2, 2));
                assert_eq!((bitmap.width, bitmap.height), (2, 2));
                // The clipped request is 5x5, but MapToLandscape paints the
                // full 6x6 zoomed map-cell extent (C4Landscape.cpp:496-506).
                assert_eq!(bitmap.indices, vec![1 | 0x80, 0, 1 | 0x80, 0]);
                assert_eq!(texmap.default_material_entry("Earth"), Some(1));
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }
    }

    #[test]
    fn draw_map_carries_requested_segment_when_rendered_map_is_larger() {
        // An empty source on a cloned retained creator renders its last
        // scenario map at the original dimensions, but DrawMap still passes
        // the new requested iMapWdt/iMapHgt segment to MapToLandscape
        // (C4Landscape.cpp:2642-2666,482-506; C4MapCreatorS2.cpp:773-815).
        let args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::String(String::new().into()),
        ];
        let guard = enter_random_context(LcgRng::new(19));
        let (result, outcome) =
            with_effect_context(None, &[], draw_map_world(8, 7, 3, true), 1, || {
                draw_map(&args)
            });
        let _ = guard.finish();

        assert_eq!(result.expect("DrawMap succeeds"), Value::Int(1));
        match &outcome.landscape[0] {
            LandscapeOperation::DrawMap {
                bitmap,
                map_width,
                map_height,
                ..
            } => {
                assert_eq!((bitmap.width, bitmap.height), (8, 4));
                assert_eq!((*map_width, *map_height), (1, 1));
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }
    }

    #[test]
    fn draw_map_consumes_all_map_draws_before_later_script_random() {
        // C4Landscape::DrawMap constructs and renders synchronously before
        // FnDrawMap returns (C4Landscape.cpp:2642-2668), so its two exact
        // FakeLS size draws precede a later Random in the same script call.
        let seed = 23;
        let mut expected_rng = LcgRng::new(seed);
        let _ = expected_rng.random(1);
        let _ = expected_rng.random(1);
        let expected_random = expected_rng.random(1_000);
        let guard = enter_random_context(LcgRng::new(seed));
        let args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(3),
            Value::Int(3),
            Value::String("map Runtime { seed = 9; Named; };".to_string().into()),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], draw_map_world(8, 7, 3, true), 1, || {
                let drew = draw_map(&args)?;
                let after = random(&[Value::Int(1_000)])?;
                Ok::<_, RuntimeError>((drew, after))
            });
        let final_rng = guard.finish();

        assert_eq!(
            result.expect("DrawMap and Random succeed"),
            (Value::Int(1), Value::Int(expected_random))
        );
        assert_eq!(final_rng, expected_rng);
        assert_eq!(outcome.landscape.len(), 1, "render queues exact bytes once");
    }

    #[test]
    fn draw_map_script_algorithm_reenters_scenario_and_threads_live_rng() {
        let mut script = ScriptEngine::new();
        register_host_functions(&mut script);
        script
            .load_script(
                r#"
                func ScriptAlgoRuntime(x, y, a, b) {
                    Random(1000);
                    return x == 0 && y == 0 && a == 7 && b == 13;
                }
                func Probe() {
                    DrawMap(0, 0, 3, 3,
                        "map RuntimeMap { seed=9; wdt=1px; hgt=1px; overlay Runtime { algo=script; seed=11; a=7; b=13; mat=Earth; tex=Rough; sub=0; }; };");
                    return Random(1000);
                }
                "#,
            )
            .expect("runtime ScriptAlgo probe compiles");
        let script = Arc::new(script);
        let world = draw_map_world(8, 7, 3, true).with_scenario_script(Some(Arc::clone(&script)));

        let seed = 53;
        let mut expected_rng = LcgRng::new(seed);
        let _ = expected_rng.random(1);
        let _ = expected_rng.random(1);
        let _ = expected_rng.random(1_000);
        let expected_after = expected_rng.random(1_000);

        let guard = enter_random_context(LcgRng::new(seed));
        let (result, outcome) =
            with_effect_context(None, &[], world, 1, || script.call("Probe", &[]));
        let final_rng = guard.finish();

        assert_eq!(
            result.expect("DrawMap reenters the scenario script"),
            Value::Int(expected_after)
        );
        assert_eq!(
            final_rng, expected_rng,
            "the callback Random draw stays between map setup and Probe's later draw"
        );
        let [LandscapeOperation::DrawMap { bitmap, .. }] = outcome.landscape.as_slice() else {
            panic!("unexpected runtime map operations: {:?}", outcome.landscape);
        };
        assert_eq!(bitmap.indices, vec![1]);
    }

    #[test]
    fn draw_map_hosts_repair_active_masks_during_synchronous_preview() {
        let draw_map_args = [
            Value::Int(-2),
            Value::Int(1),
            Value::Int(7),
            Value::Int(5),
            Value::String("map Runtime { seed = 9; Named; };".to_string().into()),
        ];
        let draw_def_map_args = [
            Value::Int(-2),
            Value::Int(1),
            Value::Int(7),
            Value::Int(5),
            Value::String("Requested".to_string().into()),
        ];

        for (name, seed, draw_def) in [("DrawMap", 17, false), ("DrawDefMap", 37, true)] {
            let world = draw_map_masked_world();
            let guard = enter_random_context(LcgRng::new(seed));
            let (result, outcome) = with_effect_context(None, &[], world.clone(), 1, || {
                let drew = if draw_def {
                    draw_def_map(&draw_def_map_args)?
                } else {
                    draw_map(&draw_map_args)?
                };
                let visible_mask = get_texture(&[Value::Int(0), Value::Int(1)])?;
                Ok::<_, RuntimeError>((drew, visible_mask))
            });
            let _ = guard.finish();

            assert_eq!(
                result.unwrap_or_else(|error| panic!("{name} failed: {error}")),
                (Value::Int(1), Value::String("Smooth".to_string().into())),
                "{name} re-puts the Vehicle mask before returning"
            );
            assert_eq!(outcome.landscape.len(), 1);

            let mut replay_world = world;
            replay_world.preview_runtime_landscape_operation(&outcome.landscape[0]);
            assert_eq!(
                replay_world
                    .landscape_ref()
                    .expect("landscape exists")
                    .grid_byte_at(0, 1),
                Some(3),
                "{name} leaves the live mask pixel in place"
            );
            assert_eq!(
                replay_world.solid_mask_bakes[0].1.buffer,
                vec![1 | 0x80],
                "{name} stores the newly painted Earth beneath the mask"
            );
        }
    }

    #[test]
    fn draw_map_texmap_allocation_is_immediately_visible_to_get_texture() {
        // ReadScript evaluates a complete top-level overlay immediately, so
        // GetIndexMatTex may allocate a TextureMap slot before Render finds
        // no map. The allocation survives even though DrawMap returns false
        // (C4MapCreatorS2.cpp:1201-1203,773-815; C4Landscape.cpp:2659-2663;
        // C4Texture.cpp:319-369). A subsequent GetTexture in the same
        // callback resolves through that live mapping even though this
        // particular source contains no renderable map and changes no pixel.
        let args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(3),
            Value::Int(3),
            Value::String(
                "overlay Added { mat = Earth; tex = Ridge; seed = 7; };"
                    .to_string()
                    .into(),
            ),
        ];
        let guard = enter_random_context(LcgRng::new(31));
        let (result, outcome) =
            with_effect_context(None, &[], draw_map_world(8, 7, 3, false), 1, || {
                let drew = draw_map(&args)?;
                let texture = get_texture(&[Value::Int(2), Value::Int(2)])?;
                Ok::<_, RuntimeError>((drew, texture))
            });
        let _ = guard.finish();

        assert_eq!(
            result.expect("DrawMap and GetTexture succeed"),
            (Value::Int(0), Value::String("Ridge".to_string().into()))
        );
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::SyncRuntimeTexMap { texmap } => {
                assert_eq!(texmap.material_names[2].as_deref(), Some("Earth"));
                assert_eq!(texmap.match_texture_names[2].as_deref(), Some("Ridge"));
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }
    }

    #[test]
    fn draw_map_false_paths_do_not_render_or_queue() {
        // DrawMap returns false before constructing C4MapCreatorS2 for a
        // null definition or an empty ClipRect (C4Landscape.cpp:2636-2641,
        // 2698-2707), so neither path may consume synced random values.
        for args in [
            vec![
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
                Value::Int(1),
                Value::Nil,
            ],
            vec![
                Value::Int(8),
                Value::Int(0),
                Value::Int(1),
                Value::Int(1),
                Value::String("map Runtime { Named; };".to_string().into()),
            ],
        ] {
            let initial_rng = LcgRng::new(29);
            let guard = enter_random_context(initial_rng.clone());
            let (result, outcome) =
                with_effect_context(None, &[], draw_map_world(8, 7, 3, true), 1, || {
                    draw_map(&args)
                });
            let final_rng = guard.finish();
            assert_eq!(result.expect("DrawMap false path succeeds"), Value::Int(0));
            assert!(outcome.landscape.is_empty());
            assert_eq!(final_rng, initial_rng);
        }
    }

    #[test]
    fn draw_def_map_clips_renders_the_named_map_and_consumes_full_tree_rng() {
        // DrawDefMap mutates the retained creator rather than cloning a
        // FakeLS creator. Requested is deliberately not the last map, and
        // only its left-half template should reach the queued 2x2 bitmap.
        // ReEvaluate visits three seedless overlays, for six exact draws.
        let seed = 37;
        let mut expected_rng = LcgRng::new(seed);
        for _ in 0..3 {
            let _ = expected_rng.random(32768);
            let _ = expected_rng.random(65536);
        }
        let expected_random = expected_rng.random(1_000);
        let guard = enter_random_context(LcgRng::new(seed));
        let args = [
            Value::Int(-2),
            Value::Int(1),
            Value::Int(7),
            Value::Int(5),
            Value::String("Requested".to_string().into()),
        ];
        let world = draw_map_world(8, 7, 3, true);
        let initial_landscape = world.landscape_ref().expect("landscape exists").clone();
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            let drew = draw_def_map(&args)?;
            let after = random(&[Value::Int(1_000)])?;
            Ok::<_, RuntimeError>((drew, after))
        });
        let final_rng = guard.finish();

        assert_eq!(
            result.expect("DrawDefMap and Random succeed"),
            (Value::Int(1), Value::Int(expected_random))
        );
        assert_eq!(final_rng, expected_rng);
        assert_eq!(outcome.landscape.len(), 1);
        let expected_creator = match &outcome.landscape[0] {
            LandscapeOperation::DrawDefMap {
                origin,
                bitmap,
                map_width,
                map_height,
                texmap,
                map_creator,
            } => {
                assert_eq!(*origin, Vector2::new(0, 1));
                assert_eq!((*map_width, *map_height), (2, 2));
                assert_eq!((bitmap.width, bitmap.height), (2, 2));
                assert_eq!(bitmap.indices, vec![1 | 0x80, 0, 1 | 0x80, 0]);
                assert_eq!(texmap.default_material_entry("Earth"), Some(1));
                map_creator.0.clone()
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        };

        let mut engine = crate::Engine::new();
        engine.set_landscape(initial_landscape);
        engine.apply_landscape_operations(outcome.landscape.clone());
        let landscape = engine.landscape().expect("folded landscape exists");
        assert_eq!(landscape.grid_byte_at(0, 1), Some(1 | 0x80));
        assert_eq!(
            landscape
                .raster_state()
                .and_then(|state| state.map_creator()),
            Some(&expected_creator),
            "authoritative fold persists the mutated retained creator"
        );
    }

    #[test]
    fn draw_def_map_creator_mutation_is_live_to_later_draw_map() {
        // pMapCreator is mutated before DrawDefMap returns. Resize the last
        // scenario map to 1x1, then let DrawMap clone that live creator and
        // render its last map from an empty runtime source. A stale creator
        // snapshot would incorrectly produce Decoy's original 8x4 bitmap.
        let guard = enter_random_context(LcgRng::new(41));
        let def_args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::String("Decoy".to_string().into()),
        ];
        let map_args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::String(String::new().into()),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], draw_map_world(8, 7, 3, true), 1, || {
                Ok::<_, RuntimeError>((draw_def_map(&def_args)?, draw_map(&map_args)?))
            });
        let final_rng = guard.finish();

        assert_eq!(
            result.expect("DrawDefMap then DrawMap succeed"),
            (Value::Int(1), Value::Int(1))
        );
        assert_eq!(
            final_rng.count, 8,
            "six ReEvaluate seed draws plus two DrawMap FakeLS size draws"
        );
        assert_eq!(outcome.landscape.len(), 2);
        match &outcome.landscape[1] {
            LandscapeOperation::DrawMap { bitmap, .. } => {
                assert_eq!((bitmap.width, bitmap.height), (1, 1));
                assert_eq!(bitmap.indices, vec![1 | 0x80]);
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }
    }

    #[test]
    fn draw_def_map_creator_mutation_threads_between_host_contexts() {
        // Effect event batches invoke separate host contexts against a
        // threaded HostWorldContext before their operations reach Engine.
        // Replay the first callback's state-bearing operation and prove the
        // second context clones Decoy at its resized 1x1 dimensions.
        let guard = enter_random_context(LcgRng::new(47));
        let mut world = draw_map_world(8, 7, 3, true);
        let def_args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::String("Decoy".to_string().into()),
        ];
        let (first_result, first_outcome) =
            with_effect_context(None, &[], world.clone(), 1, || draw_def_map(&def_args));
        assert_eq!(first_result.expect("DrawDefMap succeeds"), Value::Int(1));
        for operation in &first_outcome.landscape {
            world.preview_runtime_landscape_operation(operation);
        }

        let map_args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::String(String::new().into()),
        ];
        let (second_result, second_outcome) =
            with_effect_context(None, &[], world, 1, || draw_map(&map_args));
        let final_rng = guard.finish();

        assert_eq!(second_result.expect("DrawMap succeeds"), Value::Int(1));
        assert_eq!(final_rng.count, 8);
        match &second_outcome.landscape[0] {
            LandscapeOperation::DrawMap { bitmap, .. } => {
                assert_eq!((bitmap.width, bitmap.height), (1, 1));
                assert_eq!(bitmap.indices, vec![1 | 0x80]);
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }
    }

    #[test]
    fn draw_def_map_false_paths_leave_rng_creator_and_landscape_untouched() {
        // GetMap runs after ClipRect but before SetSize/ReEvaluate. Missing
        // or non-map names, a clipped-away rect, nil, and absent retained
        // state therefore all return zero without queuing or drawing RNG.
        for (retain_creator, args) in [
            (
                true,
                vec![
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1),
                    Value::Int(1),
                    Value::String("Missing".to_string().into()),
                ],
            ),
            (
                true,
                vec![
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1),
                    Value::Int(1),
                    Value::String("Half".to_string().into()),
                ],
            ),
            (
                true,
                vec![
                    Value::Int(8),
                    Value::Int(0),
                    Value::Int(1),
                    Value::Int(1),
                    Value::String("Requested".to_string().into()),
                ],
            ),
            (
                true,
                vec![
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1),
                    Value::Int(1),
                    Value::Nil,
                ],
            ),
            (
                false,
                vec![
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(1),
                    Value::Int(1),
                    Value::String("Requested".to_string().into()),
                ],
            ),
        ] {
            let initial_rng = LcgRng::new(43);
            let guard = enter_random_context(initial_rng.clone());
            let (result, outcome) = with_effect_context(
                None,
                &[],
                draw_map_world(8, 7, 3, retain_creator),
                1,
                || draw_def_map(&args),
            );
            let final_rng = guard.finish();

            assert_eq!(
                result.expect("DrawDefMap false path succeeds"),
                Value::Int(0)
            );
            assert!(outcome.landscape.is_empty());
            assert_eq!(final_rng, initial_rng);
        }
    }

    #[test]
    fn draw_mat_chunks_matches_chunk_pixels_and_consumes_rng_in_cpp_order() {
        // DrawChunks enters x first, then y, and samples Random(1000) before
        // each DrawChunk call (C4Landscape.cpp:2434-2438). The Rough shape
        // then combines that sampled offset with the retained MapSeed.
        let seed = 59;
        let mut expected_rng = LcgRng::new(seed);
        let expected_offsets = (0..4)
            .map(|_| expected_rng.random(1_000))
            .collect::<Vec<_>>();
        let expected_after = expected_rng.random(1_000);
        let args = [
            Value::Int(4),
            Value::Int(3),
            Value::Int(8),
            Value::Int(6),
            Value::Int(2),
            Value::Int(2),
            Value::String("Earth".to_string().into()),
            Value::String("Rough".to_string().into()),
            Value::Bool(true),
        ];
        let mut replay_world = draw_mat_chunks_world();
        let initial_landscape = replay_world
            .landscape_ref()
            .expect("landscape exists")
            .clone();
        let guard = enter_random_context(LcgRng::new(seed));
        let (result, outcome) = with_effect_context(None, &[], replay_world.clone(), 1, || {
            let drew = draw_mat_chunks(&args)?;
            let immediate_texture = get_texture(&[Value::Int(6), Value::Int(5)])?;
            let after = random(&[Value::Int(1_000)])?;
            Ok::<_, RuntimeError>((drew, immediate_texture, after))
        });
        let final_rng = guard.finish();

        assert_eq!(
            result.expect("DrawMatChunks and Random succeed"),
            (
                Value::Int(1),
                Value::String("Rough".to_string().into()),
                Value::Int(expected_after)
            )
        );
        assert_eq!(final_rng, expected_rng);
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DrawMatChunks {
                origin,
                width,
                height,
                count_x,
                count_y,
                material,
                byte,
                map_seed,
                random_offsets,
                texmap,
            } => {
                assert_eq!(*origin, Vector2::new(4, 3));
                assert_eq!((*width, *height, *count_x, *count_y), (8, 6, 2, 2));
                assert_eq!(material, "Earth");
                assert_eq!(*byte, 1 | 0x80);
                assert_eq!(*map_seed, 9);
                assert_eq!(random_offsets, &expected_offsets);
                assert_eq!(texmap.material_names[1].as_deref(), Some("Earth"));
                assert_eq!(texmap.match_texture_names[1].as_deref(), Some("Rough"));
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }

        // State-bearing replay makes the newly allocated TextureMap slot
        // live to a later effect callback before the authoritative fold.
        replay_world.preview_runtime_landscape_operation(&outcome.landscape[0]);
        assert_eq!(
            replay_world
                .landscape_ref()
                .and_then(Landscape::raster_state)
                .and_then(|state| state.texmap().material_names[1].as_deref()),
            Some("Earth")
        );

        let mut engine = crate::Engine::new();
        engine.set_landscape(initial_landscape);
        engine.apply_landscape_operations(outcome.landscape);
        let landscape = engine.landscape().expect("folded landscape exists");
        let actual = (0..12)
            .flat_map(|y| {
                (0..16).map(move |x| landscape.grid_byte_at(x, y).expect("pixel is in bounds"))
            })
            .collect::<Vec<_>>();

        // Golden Surface8 rows produced by the C++ DrawChunk and Allegro
        // polygon oracle for offsets 231,283,416,148 and MapSeed 9.
        let golden_rows = [
            "................",
            "................",
            ".........###....",
            "....##########..",
            "...############.",
            "...###########..",
            "....##########..",
            "...############.",
            "...###########..",
            "....##########..",
            "........#####...",
            ".........###....",
        ];
        let golden = golden_rows
            .into_iter()
            .flat_map(str::bytes)
            .map(|pixel| if pixel == b'#' { 1 | 0x80 } else { 0 })
            .collect::<Vec<_>>();
        assert_eq!(actual, golden);
        assert_eq!(
            actual.iter().filter(|&&byte| byte == (1 | 0x80)).count(),
            87
        );
    }

    #[test]
    fn draw_mat_chunks_offscreen_clip_collapses_to_the_cpp_boundary_pixel() {
        // The raw clip begins at x=16, wholly right of this 16-pixel plane.
        // CSurface8::Clip independently clamps both endpoints to x=15, and
        // the Rough polygon still reaches that one column. A normal rectangle
        // intersection would incorrectly discard all seven C++ pixels.
        let args = [
            Value::Int(21),
            Value::Int(3),
            Value::Int(20),
            Value::Int(6),
            Value::Int(1),
            Value::Int(1),
            Value::String("Earth".to_string().into()),
            Value::String("Rough".to_string().into()),
            Value::Bool(false),
        ];
        let mut world = draw_mat_chunks_world();
        assert!(world
            .landscape_mut()
            .is_some_and(|landscape| { landscape.set_surface32_pixel(15, 5, 0x0011_2233) }));
        let initial_landscape = world.landscape_ref().expect("landscape exists").clone();
        let guard = enter_random_context(LcgRng::new(59));
        let (result, outcome) = with_effect_context(None, &[], world, 1, || {
            let drew = draw_mat_chunks(&args)?;
            let immediate = get_texture(&[Value::Int(15), Value::Int(5)])?;
            Ok::<_, RuntimeError>((drew, immediate))
        });
        let final_rng = guard.finish();

        assert_eq!(
            result.expect("offscreen DrawMatChunks succeeds"),
            (Value::Int(1), Value::String("Rough".to_string().into()))
        );
        assert_eq!(final_rng.count, 1);
        let mut engine = crate::Engine::new();
        engine.set_landscape(initial_landscape);
        engine.apply_landscape_operations(outcome.landscape);
        let landscape = engine.landscape().expect("folded landscape exists");
        assert_eq!(
            landscape.surface32_pixel_at(15, 5),
            None,
            "Relight expands the raw offscreen prepare box before clipping"
        );
        for y in 0..12 {
            for x in 0..16 {
                let expected = u8::from(x == 15 && (3..=9).contains(&y));
                assert_eq!(
                    landscape.grid_byte_at(x, y),
                    Some(expected),
                    "C++ edge clip pixel ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn draw_mat_chunks_preview_repairs_solid_masks_and_updates_their_background() {
        let args = [
            Value::Int(4),
            Value::Int(3),
            Value::Int(8),
            Value::Int(6),
            Value::Int(2),
            Value::Int(2),
            Value::String("Earth".to_string().into()),
            Value::String("Rough".to_string().into()),
            Value::Bool(true),
        ];
        let guard = enter_random_context(LcgRng::new(59));
        let (result, outcome) =
            with_effect_context(None, &[], draw_mat_chunks_masked_world(), 1, || {
                let drew = draw_mat_chunks(&args)?;
                let masked_texture = get_texture(&[Value::Int(5), Value::Int(4)])?;
                Ok::<_, RuntimeError>((drew, masked_texture))
            });
        let _ = guard.finish();

        assert_eq!(
            result.expect("masked DrawMatChunks succeeds"),
            (Value::Int(1), Value::String("Smooth".to_string().into())),
            "FinishChange re-puts the Vehicle mask before the host returns"
        );

        let mut replay_world = draw_mat_chunks_masked_world();
        replay_world.preview_runtime_landscape_operation(&outcome.landscape[0]);
        let landscape = replay_world.landscape_ref().expect("landscape exists");
        for y in 4..6 {
            for x in 5..7 {
                assert_eq!(landscape.grid_byte_at(x, y), Some(2));
            }
        }
        assert_eq!(replay_world.solid_mask_bakes.len(), 1);
        assert_eq!(
            replay_world.solid_mask_bakes[0].1.buffer,
            vec![1 | 0x80; 4],
            "Repair stores the newly drawn terrain beneath the mask"
        );
    }

    #[test]
    fn draw_mat_chunks_failure_and_zero_count_paths_do_not_draw_rng() {
        for (material, texture) in [("Missing", "Rough"), ("Earth", "Missing")] {
            let args = [
                Value::Int(0),
                Value::Int(0),
                Value::Int(8),
                Value::Int(6),
                Value::Int(2),
                Value::Int(2),
                Value::String(material.to_string().into()),
                Value::String(texture.to_string().into()),
                Value::Bool(false),
            ];
            let initial_rng = LcgRng::new(61);
            let guard = enter_random_context(initial_rng.clone());
            let (result, outcome) =
                with_effect_context(None, &[], draw_mat_chunks_world(), 1, || {
                    draw_mat_chunks(&args)
                });
            let final_rng = guard.finish();

            assert_eq!(
                result.expect("unresolved DrawMatChunks is not an error"),
                Value::Int(0)
            );
            assert!(outcome.landscape.is_empty());
            assert_eq!(final_rng, initial_rng);
        }

        let zero_count_args = [
            Value::Int(0),
            Value::Int(0),
            Value::Int(8),
            Value::Int(6),
            Value::Int(0),
            Value::Int(2),
            Value::String("Earth".to_string().into()),
            Value::String("Rough".to_string().into()),
            Value::Bool(false),
        ];
        let initial_rng = LcgRng::new(67);
        let guard = enter_random_context(initial_rng.clone());
        let (result, outcome) = with_effect_context(None, &[], draw_mat_chunks_world(), 1, || {
            draw_mat_chunks(&zero_count_args)
        });
        let final_rng = guard.finish();

        assert_eq!(result.expect("zero-count call succeeds"), Value::Int(1));
        assert_eq!(final_rng, initial_rng);
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DrawMatChunks {
                random_offsets,
                texmap,
                ..
            } => {
                assert!(random_offsets.is_empty());
                assert_eq!(texmap.material_names[1].as_deref(), Some("Earth"));
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }
    }

    #[test]
    fn draw_volcano_branch_matches_cpp_pixels_ift_and_rng_order() {
        // The C++ loop starts at (tx,ty), excludes fy, truncates interpolation
        // toward zero, and draws [cx-size/2,cx+size/2). With these endpoints
        // the row centers are exactly 8,6,5,4,3. The default Earth byte is 5
        // even though slot 1 also carries Earth, and each destination's IFT
        // bit must survive independently.
        let args = [
            Value::Int(0),
            Value::Int(2),
            Value::Int(7),
            Value::Int(8),
            Value::Int(2),
            Value::Int(5),
        ];
        let mut world = draw_volcano_branch_world();
        assert!(world
            .landscape_mut()
            .is_some_and(|landscape| { landscape.set_surface32_pixel(6, 2, 0x0011_2233) }));
        let initial_landscape = world.landscape_ref().expect("landscape exists").clone();
        let object_context = HostObjectContext {
            id: ObjectId::new(91),
            position: Vector2::new(30, 40),
            direction: Direction::Right,
            ..idle_object_context()
        };
        let seed = 73;
        let mut expected_rng = LcgRng::new(seed);
        let expected_after = expected_rng.random(1_000);
        let guard = enter_random_context(LcgRng::new(seed));
        let (result, outcome) = with_effect_context(Some(object_context), &[], world, 92, || {
            let drew = draw_volcano_branch(&args)?;
            let texture = get_texture(&[Value::Int(6), Value::Int(2)])?;
            // GBackSky is caller-relative, unlike both natives above.
            let ift_pixel = g_back_sky(&[Value::Int(6 - 30), Value::Int(2 - 40)])?;
            let plain_pixel = g_back_sky(&[Value::Int(7 - 30), Value::Int(2 - 40)])?;
            let after = random(&[Value::Int(1_000)])?;
            Ok::<_, RuntimeError>((drew, texture, ift_pixel, plain_pixel, after))
        });
        let final_rng = guard.finish();

        assert_eq!(
            result.expect("DrawVolcanoBranch and queries succeed"),
            (
                Value::Nil,
                Value::String("Smooth".to_string().into()),
                Value::Bool(false),
                Value::Bool(true),
                Value::Int(expected_after),
            )
        );
        assert_eq!(final_rng, expected_rng, "the native consumes no RNG");
        assert_eq!(outcome.landscape.len(), 1);
        match &outcome.landscape[0] {
            LandscapeOperation::DrawVolcanoBranch {
                from,
                to,
                size,
                material_byte,
            } => {
                assert_eq!(*from, Vector2::new(2, 7));
                assert_eq!(*to, Vector2::new(8, 2));
                assert_eq!(*size, 5);
                assert_eq!(*material_byte, 5);
            }
            other => panic!("unexpected landscape operation: {other:?}"),
        }

        // Effect batches replay state-bearing operations between callbacks.
        let mut replay_world = draw_volcano_branch_world();
        replay_world.preview_runtime_landscape_operation(&outcome.landscape[0]);
        let (replayed, _) = with_effect_context(None, &[], replay_world, 1, || {
            Ok::<_, RuntimeError>((
                get_texture(&[Value::Int(6), Value::Int(2)])?,
                g_back_sky(&[Value::Int(6), Value::Int(2)])?,
            ))
        });
        assert_eq!(
            replayed.expect("replayed callback reads landscape"),
            (
                Value::String("Smooth".to_string().into()),
                Value::Bool(false)
            )
        );

        let mut engine = crate::Engine::new();
        engine.set_landscape(initial_landscape);
        engine.apply_landscape_operations(outcome.landscape);
        let landscape = engine.landscape().expect("folded landscape exists");
        let centers = [(2, 8), (3, 6), (4, 5), (5, 4), (6, 3)];
        for y in 0..10 {
            for x in 0..12 {
                let initial = if (x + y) % 2 == 0 { 0x80 } else { 0 };
                let drawn = centers
                    .iter()
                    .any(|&(row, center)| row == y && x >= center - 2 && x < center + 2);
                let expected = if drawn { 5 | (initial & 0x80) } else { initial };
                assert_eq!(
                    landscape.grid_byte_at(x, y),
                    Some(expected),
                    "C++ branch pixel ({x},{y})"
                );
            }
        }
        assert_eq!(
            landscape.surface32_pixel_at(6, 2),
            None,
            "raw SetPix queues a relight over the old Surface32 override"
        );
    }

    #[test]
    fn draw_volcano_branch_empty_ranges_and_invalid_material_are_safe_noops() {
        // C++ enters no inner iteration for these shapes. In particular,
        // ty==fy must not divide by zero, and odd size 1 paints zero pixels.
        for (from_y, to_y, size) in [(4, 4, 6), (3, 7, 6), (7, 2, 1), (7, 2, -3)] {
            let world = draw_volcano_branch_world();
            let initial = world.landscape_ref().expect("landscape exists").clone();
            let args = [
                Value::Int(0),
                Value::Int(2),
                Value::Int(from_y),
                Value::Int(8),
                Value::Int(to_y),
                Value::Int(size),
            ];
            let (result, outcome) =
                with_effect_context(None, &[], world, 1, || draw_volcano_branch(&args));
            assert_eq!(result.expect("empty branch succeeds"), Value::Nil);
            assert_eq!(outcome.landscape.len(), 1);
            let mut engine = crate::Engine::new();
            engine.set_landscape(initial.clone());
            engine.apply_landscape_operations(outcome.landscape);
            let actual = engine.landscape().expect("landscape exists");
            for y in 0..10 {
                for x in 0..12 {
                    assert_eq!(actual.grid_byte_at(x, y), initial.grid_byte_at(x, y));
                }
            }
        }

        let args = [
            Value::Int(99),
            Value::Int(2),
            Value::Int(7),
            Value::Int(8),
            Value::Int(2),
            Value::Int(5),
        ];
        let (result, outcome) =
            with_effect_context(None, &[], draw_volcano_branch_world(), 1, || {
                draw_volcano_branch(&args)
            });
        assert_eq!(result.expect("invalid material is contained"), Value::Nil);
        assert!(outcome.landscape.is_empty());
    }
