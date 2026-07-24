// Contiguous slice 4 of 8 of the `scenario/tests` battery, spliced
// by `include!` from the parent module so every test id is unchanged.

    #[test]
    fn join_player_runs_scenario_init_with_the_cpp_draw_ledger() {
        // C4Player::ScenarioInit (C4Player.cpp:670-777) consumes the synced
        // RNG in this exact order: Wealth.Evaluate (one draw,
        // C4Scenario.cpp:43-46), all-random start x/y (C4Player.cpp:745-746,
        // 16 + Random(GBack - 32) each), then PlaceReadyCrew draws one
        // Random(tx2 - tx1) per crew member (C4Player.cpp:548) with
        // FindSolidGround settling each position. Crew objects are created
        // at JOIN time — never at scenario load (C4Game::InitPlayers queues
        // CID_JoinPlr; nothing spawns crew during load).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Join\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=10\n\n\
             [Player1]\nCrew=GOOD=2\nWealth=20,5,0,250\n",
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(7);
        scenario.apply(&mut engine).expect("scenario applies");
        assert_eq!(
            engine.snapshot().objects.len(),
            0,
            "no crew at load — crew joins with the player like C++"
        );

        let mut replay = engine.rng.clone();
        let landscape = engine.landscape().expect("landscape set").clone();
        let world_width = landscape.width() as i32;
        let world_height = landscape.estimated_height();
        assert_eq!((world_width, world_height), (640, 400));

        // Replay the ledger independently.
        let expected_wealth = LegacyC4SVal::new(20, 5, 0, 250).evaluate(&mut replay);
        let mut ptx = 16 + replay.random(world_width - 32);
        let mut pty = 16 + replay.random(world_height - 32);
        if let Some((nx, ny)) = landscape.find_solid_ground(ptx, pty, 30) {
            ptx = nx;
            pty = ny;
        }
        if let Some((nx, ny)) =
            landscape.find_con_site_spot(ptx, pty, 30, 50, 400, |_, _, _, _| false)
        {
            ptx = nx;
            pty = ny;
        }
        let mut expected_positions = Vec::new();
        for _ in 0..2 {
            let mut ctx = (ptx - 30) + replay.random(60);
            let mut cty = pty;
            if let Some((nx, ny)) = landscape.find_solid_ground(ctx, cty, 0) {
                ctx = nx;
                cty = ny;
            }
            expected_positions.push(Vector2::new(ctx, cty));
        }

        let joined = engine
            .join_player(crate::JoinPlayerConfig {
                name: "Tyler".to_string(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xf40000,
                pref_color: 3,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("join succeeds")
            .initialized()
            .expect("join initializes");
        assert_eq!(joined.number, 0);
        assert_eq!((joined.start_x, joined.start_y), (ptx, pty));
        assert!(joined.first_base.is_none());

        // The engine consumed exactly the replayed ledger: same draw count,
        // same LCG state.
        assert_eq!(engine.rng, replay, "RNG stream stays lockstep");

        let player = engine.player(0).expect("player registered");
        assert_eq!(player.wealth(), expected_wealth);
        assert_eq!(player.color_index(), 3, "free PrefColor is taken as-is");

        let snapshot = engine.snapshot();
        let crew: Vec<_> = snapshot
            .objects
            .iter()
            .filter(|object| object.owner == 0 && object.crew_member)
            .collect();
        assert_eq!(crew.len(), 2, "two ready-crew members placed");
        let positions: Vec<_> = crew.iter().map(|object| object.position).collect();
        assert_eq!(positions, expected_positions);

        // Fresh infos: no roster, no name sources -> "Clonk", numbered by
        // MakeValidName (C4ObjectInfoList.cpp:93-101).
        let names: Vec<_> = crew
            .iter()
            .map(|object| {
                engine
                    .crew_object_info(object.id)
                    .expect("crew info recorded")
                    .name
                    .clone()
            })
            .collect();
        assert_eq!(names, vec!["Clonk".to_string(), "Clonk2".to_string()]);
    }

    #[test]
    fn def_core_blast_incinerate_reaches_the_engine_definition() {
        // BlastIncinerate (C4Def.cpp:315) must survive the resource-core
        // apply so C4Object::Blast can consult it (C4Object.cpp:1421-1423).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/DefCore.txt"),
            "[DefCore]\nid=GOOD\nName=Good\nCategory=0\nCrewMember=0\nBlastIncinerate=50\n",
        )
        .expect("write defcore");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .definitions
                .get("GOOD")
                .expect("definition registered")
                .blast_incinerate(),
            50
        );
    }

    #[test]
    fn real_system_scripts_explode_blasts_bystanders_end_to_end() {
        // The FLNT class end-to-end: Hit -> Explode -> DoExplosion ->
        // BlastObjects -> BlastObject through the REAL planet/System.c4g
        // scripts (Explode.c, FindObject.c, GetXVal.c). The exploding
        // object removes itself (Explode.c:18), bystanders inside the
        // 10x10 direct-hit rect take the full level as Damage
        // (Explode.c:93-94 -> C4Object::Blast, C4Object.cpp:1416), and
        // the run stays script-error free (the former 'script error in
        // Hit of FLNT' harness class).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        let defs_root = dir.path().join("Defs.c4d");
        let flint = defs_root.join("Flint.c4d");
        std::fs::create_dir_all(&flint).expect("flint dir");
        std::fs::write(
            flint.join("DefCore.txt"),
            "[DefCore]\nid=FLNX\nName=Flint\nCategory=16\nWidth=6\nHeight=6\nOffset=-3,-3\n",
        )
        .expect("flint defcore");
        std::fs::write(
            flint.join("Script.c"),
            "#strict\npublic func ExplodeSize() { return(18); }\nprotected func Hit() { Explode(ExplodeSize()); }\n",
        )
        .expect("flint script");
        write_test_definition_graphics(&flint);
        let bystander = defs_root.join("Bystander.c4d");
        std::fs::create_dir_all(&bystander).expect("bystander dir");
        std::fs::write(
            bystander.join("DefCore.txt"),
            "[DefCore]\nid=BYST\nName=Bystander\nCategory=16\nWidth=6\nHeight=6\nOffset=-3,-3\n",
        )
        .expect("bystander defcore");
        std::fs::write(bystander.join("Script.c"), "#strict\n").expect("bystander script");
        write_test_definition_graphics(&bystander);

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        // The live game installs planet/System.c4g before the scenario.
        let planet =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../planet/System.c4g");
        let system_scripts: Vec<(String, String)> = ["FindObject.c", "GetXVal.c", "Explode.c"]
            .iter()
            .map(|name| {
                let bytes = std::fs::read(planet.join(name)).expect("system script reads");
                // ISO-8859-1 comments -> chars (the group loader does the
                // same byte-transparent conversion).
                (
                    (*name).to_string(),
                    bytes.iter().map(|&b| b as char).collect::<String>(),
                )
            })
            .collect();
        engine.install_global_scripts(&system_scripts);

        let flint_id = engine
            .spawn_object(SpawnConfig::new("FLNX").with_position(Vector2::new(100, 100)))
            .expect("flint spawns");
        let bystander_id = engine
            .spawn_object(SpawnConfig::new("BYST").with_position(Vector2::new(102, 100)))
            .expect("bystander spawns");
        let flint_idx = engine.find_object_index(flint_id).expect("flint exists");
        engine
            .call_object_function(flint_idx, "Hit", Vec::new())
            .expect("Hit runs without script errors");
        assert!(
            engine
                .find_object_index(flint_id)
                .map(|idx| engine.objects[idx].state.status == ObjectStatus::Deleted)
                .unwrap_or(true),
            "the exploding object removed itself (Explode.c RemoveObject)"
        );
        let bystander_idx = engine
            .find_object_index(bystander_id)
            .expect("bystander survives");
        assert_eq!(
            engine.objects[bystander_idx].state.damage, 18,
            "direct-hit rect victims take the blast level as Damage"
        );
    }

    #[test]
    fn real_system_scripts_shake_viewport_global_effect_end_to_end() {
        // System.c4g ShakeViewPort (planet/System.c4g/Explode.c:166-183)
        // does a nil-target AddEffect("ShakeEffect", pObj, 200, 1) whose
        // global FxShakeEffect* callbacks must fire: Start synchronously
        // inside AddEffect (C4Effect ctor, C4Effect.cpp:128-129), Timer
        // every frame from pGlobalEffects->Execute(nullptr)
        // (C4Game.cpp:830-831, C4Effect.cpp:342-345), and the Timer's -1
        // return kills the effect with its Stop (C4Effect.cpp:350,
        // Explode.c:198 `return(-1)`).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        let probe = dir.path().join("Defs.c4d/Probe.c4d");
        std::fs::create_dir_all(&probe).expect("probe dir");
        std::fs::write(
            probe.join("DefCore.txt"),
            "[DefCore]\nid=PROB\nName=Probe\nCategory=16\nWidth=6\nHeight=6\nOffset=-3,-3\n",
        )
        .expect("probe defcore");
        std::fs::write(
            probe.join("Script.c"),
            "#strict\npublic func Shake() { return(ShakeViewPort(100, 0, 10, 20)); }\n",
        )
        .expect("probe script");
        write_test_definition_graphics(&probe);

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let planet =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../planet/System.c4g");
        let system_scripts: Vec<(String, String)> = ["FindObject.c", "GetXVal.c", "Explode.c"]
            .iter()
            .map(|name| {
                let bytes = std::fs::read(planet.join(name)).expect("system script reads");
                (
                    (*name).to_string(),
                    bytes.iter().map(|&b| b as char).collect::<String>(),
                )
            })
            .collect();
        engine.install_global_scripts(&system_scripts);

        let probe_id = engine
            .spawn_object(SpawnConfig::new("PROB").with_position(Vector2::new(100, 100)))
            .expect("probe spawns");
        let probe_idx = engine.find_object_index(probe_id).expect("probe exists");
        engine
            .call_object_function(probe_idx, "Shake", Vec::new())
            .expect("Shake runs without script errors");

        // FxShakeEffectStart ran inside AddEffect; ShakeViewPort then
        // seeds EffectVar 0..2 with the level and offsets (Explode.c:
        // 175-178).
        assert_eq!(engine.global_effects().len(), 1, "global effect added");
        let effect = &engine.global_effects()[0];
        assert_eq!(effect.name, "ShakeEffect");
        assert_eq!(effect.priority, 200);
        assert_eq!(effect.interval, 1);
        assert_eq!(effect.var(0), crate::EffectVarValue::Int(100));
        assert_eq!(effect.var(1), crate::EffectVarValue::Int(10));
        assert_eq!(effect.var(2), crate::EffectVarValue::Int(20));

        // FxShakeEffectTimer (Explode.c:188-199) fires every frame; its
        // strength formula iLevel/((3*iTime)/2+3)-iTime**2/400 reaches 0
        // at iTime 29 for level 100 -> return(-1) marks the effect dead.
        // Execute does not unlink its current node until the next pass.
        let mut death_frame = None;
        for frame in 1..=40 {
            engine.tick_without_snapshot().expect("tick runs");
            let active = engine
                .global_effects()
                .iter()
                .find(|effect| effect.priority != 0);
            if active.is_none() {
                death_frame = Some(frame);
                break;
            }
            assert_eq!(
                active.map(|effect| effect.timer),
                Some(frame),
                "iTime advances every frame while the shake lives"
            );
        }
        assert_eq!(
            death_frame,
            Some(29),
            "the timer's C4Fx_Execute_Kill return deactivates the global effect"
        );
    }

    #[test]
    fn appendto_scripts_link_into_their_targets_like_c4aullink() {
        // C4AulScript::ResolveAppends (C4AulLink.cpp:29-64) + AppendTo
        // (:114-141): a definition script with `#appendto GOOD` copies its
        // functions into GOOD's script as OVERRIDES (the original stays
        // reachable via inherited), and System.c4g scripts with #appendto
        // do the same (GoldRush's dialogue and AI scripts rely on both).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\n#appendto GOOD\n\
             public func Probe() { return inherited() * 10 + 4; }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "func Probe() { return 1; }\n",
        )
        .expect("write target script");
        let boost = dir.path().join("Defs.c4d/Boost.c4d");
        std::fs::create_dir_all(&boost).expect("boost dir");
        std::fs::write(
            boost.join("DefCore.txt"),
            "[DefCore]\nid=BOST\nName=Boost\nCategory=0\nCrewMember=0\n",
        )
        .expect("write boost defcore");
        std::fs::write(
            boost.join("Script.c"),
            "#strict\n#appendto GOOD\n\
             public func Probe() { return inherited() * 10 + 2; }\n\
             public func SetAI(szName, iInterval) { return 7; }\n",
        )
        .expect("write boost script");
        write_test_definition_graphics(&boost);
        let pack_system = dir.path().join("Defs.c4d/System.c4g");
        std::fs::create_dir_all(&pack_system).expect("pack system dir");
        std::fs::write(
            pack_system.join("Append.c"),
            "#strict\n#appendto GOOD\n\
             public func Probe() { return inherited() * 10 + 3; }\n",
        )
        .expect("write pack system append");
        let system = scenario_dir.join("System.c4g");
        std::fs::create_dir_all(&system).expect("system dir");
        std::fs::write(
            system.join("Append.c"),
            "#strict\n#appendto GOOD\n\
             static const SYSTEM_APPEND_VALUE = 3;\n\
             public func Probe() { return inherited() * 10 + 5; }\n\
             public func FromSystem() { return SYSTEM_APPEND_VALUE(); }\n",
        )
        .expect("write system append");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let id = engine
            .spawn_object(SpawnConfig::new("GOOD"))
            .expect("target spawns");
        let index = engine.find_object_index(id).expect("object index");
        assert_eq!(
            engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("Probe call succeeds"),
            clonk_script::Value::Int(12345),
            "appends follow definition, pack System, Script.c, scenario System order"
        );
        assert_eq!(
            engine
                .call_object_function(index, "SetAI", Vec::new())
                .expect("SetAI call succeeds"),
            clonk_script::Value::Int(7),
            "appended function exists on the target"
        );
        assert_eq!(
            engine
                .call_object_function(index, "FromSystem", Vec::new())
                .expect("FromSystem call succeeds"),
            clonk_script::Value::Int(3),
            "System.c4g appends see their global constants without nil local shadowing"
        );
        assert!(
            engine
                .global_script_functions
                .as_ref()
                .is_none_or(|functions| !functions.contains_key("FromSystem")),
            "a public append function is local to its System host, not engine-global"
        );
    }

    #[test]
    fn objects_created_mid_call_receive_arrow_calls_like_cpp() {
        // C++ CreateObject fully creates the object DURING the call
        // (Game.CreateObject -> NewObject), so `obj->Method()` on the
        // fresh object resolves immediately (GoldRush's DoInitialize does
        // pObj->SetAI(...) right after CreateObject). The copy-in/copy-out
        // model must give pending spawns a callable scope, and their
        // nested outcomes must fold onto the object once spawned.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Mark();\n\
                 return 0;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal hit;\npublic func Mark() { hit = 7; return hit; }\n",
        )
        .expect("write target script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created during Initialize");
        assert_eq!(
            object.local_vars.get("hit"),
            Some(&clonk_script::Value::Int(7)),
            "the nested Mark() call ran on the fresh object and folded"
        );
    }

    #[test]
    fn scenario_statics_are_visible_to_definition_scripts() {
        // C4Aul `static` variables live in Game.ScriptEngine.GlobalNamed —
        // ONE table for every script host: GoldRush's scenario Script.c
        // declares `static iDifficulty;` and the appended AI script (in a
        // definition host) reads it (Locals.c4d/AI.c4d SetAI ->
        // SetDifficultyPhysicals(iDifficulty)).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static shared;\n\
             global func Initialize(state, random) {\n\
                 shared = 4;\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Remember();\n\
                 return 0;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal seen;\n\
             public func Remember() { seen = shared; shared = shared + 1; return seen; }\n",
        )
        .expect("write target script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created");
        assert_eq!(
            object.local_vars.get("seen"),
            Some(&clonk_script::Value::Int(4)),
            "the definition script read the scenario static"
        );
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("shared")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(5)),
            "the definition script's write went back to the shared table"
        );
    }

    #[test]
    fn definition_global_funcs_register_engine_wide_like_cpp() {
        // `global func` declarations in DEFINITION scripts belong to
        // Game.ScriptEngine (AA_GLOBAL, C4AulParse preparse): Time.c4d
        // declares `global func IsNight()` and every other script calls it
        // plainly (GetFuncRecursive walks up to the engine,
        // C4Aul.cpp:285-291). Includes/appends never copy global funcs
        // (C4AulLink.cpp:127) — they are reachable through the engine.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Remember();\n\
                 return 0;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal seen;\n\
             public func Remember() { seen = NightCheck(); return seen; }\n",
        )
        .expect("write target script");
        let time = dir.path().join("Defs.c4d/Time.c4d");
        std::fs::create_dir_all(&time).expect("time dir");
        std::fs::write(
            time.join("DefCore.txt"),
            "[DefCore]\nid=TIME\nName=Time\nCategory=0\nCrewMember=0\n",
        )
        .expect("write time defcore");
        std::fs::write(
            time.join("Script.c"),
            "#strict\nglobal func NightCheck() { return 8; }\n",
        )
        .expect("write time script");
        write_test_definition_graphics(&time);

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created");
        assert_eq!(
            object.local_vars.get("seen"),
            Some(&clonk_script::Value::Int(8)),
            "another def's script called the definition-declared global func"
        );
    }

    #[test]
    fn cross_object_localn_folds_into_the_target_like_cpp() {
        // The GoldRush WSKI pattern (Goldrush.c4s/Script.c:58-62):
        //   pObj = CreateContents(WSKI, pWagon);
        //   LocalN("iWater", pObj) = 90;
        //   pObj->~UpdateGraphics();
        // FnLocalN returns a reference into the TARGET's named locals
        // (C4Script.cpp:4591-4605): the write lands on the fresh object
        // and the nested call right after it sees the new value.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 LocalN(\"iWater\", obj) = 90;\n\
                 obj->Check();\n\
                 LocalN(\"iWater\", obj) += 10;\n\
                 return 0;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal iWater;\nlocal seen;\n\
             public func Check() { seen = iWater; return seen; }\n",
        )
        .expect("write target script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created");
        assert_eq!(
            object.local_vars.get("seen"),
            Some(&clonk_script::Value::Int(90)),
            "the nested call right after the write saw the new value"
        );
        assert_eq!(
            object.local_vars.get("iWater"),
            Some(&clonk_script::Value::Int(100)),
            "the final cell value (write + compound add) folded onto the object"
        );
    }

    #[test]
    fn find_object_uses_the_cpp_argument_layout_and_caller_context() {
        // FnFindObject (C4Script.cpp:2113-2135): parameters are (id, x, y,
        // wdt, hgt, dwOCF, szAction, pActionTarget, vContainer, pFindNext).
        // Local calls EXCLUDE the caller and adjust x/y by the caller's
        // position; vContainer takes an object or the NO_CONTAINER=124 /
        // ANY_CONTAINER=123 sentinels (C4Object.h:83-84) — any other int is
        // simply no filter (C4Value::getObj() yields nil), never an error.
        // GoldRush's cannon Initialize chain depends on this layout
        // (Cannon.c4d/Script.c:31 passes NoContainer() as 9th argument).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            Some(("BOXD", "// box\n")),
            "global func Initialize() {\n\
                 var a = CreateObject(GOOD, 50, 50, -1);\n\
                 var b = CreateObject(GOOD, 55, 52, -1);\n\
                 var box = CreateObject(BOXD, 90, 90, -1);\n\
                 var c = CreateObject(GOOD, 90, 90, -1);\n\
                 c->Enter(box);\n\
                 a->Probe(b, c);\n\
                 return 1;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\n\
             local iExcluded; local iNoContainer; local iAnyContainer;\n\
             local iFindNext; local iIntTolerant; local iRelative;\n\
             public func Probe(pOther, pContained) {\n\
                 if (FindObject(GOOD) == pOther) iExcluded = 1;\n\
                 if (!FindObject(GOOD, 0,0,0,0, 0, 0, 0, NoContainer(), pOther)) iNoContainer = 1;\n\
                 if (FindObject(GOOD, 0,0,0,0, 0, 0, 0, AnyContainer()) == pContained) iAnyContainer = 1;\n\
                 if (FindObject(GOOD, 0,0,0,0, 0, 0, 0, 0, pOther) == pContained) iFindNext = 1;\n\
                 if (FindObject(GOOD, 0,0,0,0, 0, 0, 0, 7) == pOther) iIntTolerant = 1;\n\
                 if (FindObject(GOOD, -10,-10, 20,20) == pOther) iRelative = 1;\n\
             }\n",
        )
        .expect("write prober script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let prober = snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == "GOOD")
            .min_by_key(|object| object.id)
            .expect("prober created");
        let flag = |name: &str| prober.local_vars.get(name).cloned();
        assert_eq!(
            flag("iExcluded"),
            Some(clonk_script::Value::Int(1)),
            "local calls exclude the caller (C4Script.cpp:2131)"
        );
        assert_eq!(
            flag("iNoContainer"),
            Some(clonk_script::Value::Int(1)),
            "NO_CONTAINER in the 9th slot filters contained objects"
        );
        assert_eq!(
            flag("iAnyContainer"),
            Some(clonk_script::Value::Int(1)),
            "ANY_CONTAINER in the 9th slot requires containment"
        );
        assert_eq!(
            flag("iFindNext"),
            Some(clonk_script::Value::Int(1)),
            "the 10th slot is pFindNext"
        );
        assert_eq!(
            flag("iIntTolerant"),
            Some(clonk_script::Value::Int(1)),
            "a non-sentinel int container is no filter, not an error"
        );
        assert_eq!(
            flag("iRelative"),
            Some(clonk_script::Value::Int(1)),
            "local calls offset the search rect by the caller's position \
             (C4Script.cpp:2115-2119)"
        );
    }

    #[test]
    fn join_broadcasts_initialize_player_to_rule_objects_like_cpp() {
        // C4GameScriptHost::GRBroadcast (C4ScriptHost.cpp:234-249): every
        // live object with a C4D_Goal|C4D_Rule|C4D_Environment category bit
        // is called BEFORE the scenario script. The join path broadcasts
        // PSF_InitializePlayer this way (C4Player.cpp:769-775) — GoldRush's
        // TeamAccount rule creates the per-player ACNT from it
        // (TeamAccount.c4d/Script.c InitializePlayer).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize() {\n\
                 CreateObject(RULZ, 0, 0, -1);\n\
                 return 1;\n\
             }\n",
        );
        let rule = dir.path().join("Defs.c4d/Rule.c4d");
        std::fs::create_dir_all(&rule).expect("rule dir");
        std::fs::write(
            rule.join("DefCore.txt"),
            "[DefCore]\nid=RULZ\nName=Rule\nCategory=524288\nCrewMember=0\n",
        )
        .expect("write rule defcore");
        std::fs::write(
            rule.join("Script.c"),
            "#strict\nlocal iJoined;\n\
             public func InitializePlayer(iPlr) {\n\
                 iJoined = iPlr + 1;\n\
                 CreateObject(GOOD, 60, 60, iPlr);\n\
                 return 1;\n\
             }\n",
        )
        .expect("write rule script");
        write_test_definition_graphics(&rule);

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        join_test_player(&mut engine);
        let snapshot = engine.snapshot();
        let rule_object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "RULZ")
            .expect("rule object created");
        assert_eq!(
            rule_object.local_vars.get("iJoined"),
            Some(&clonk_script::Value::Int(1)),
            "the rule object's InitializePlayer ran for the joining player \
             (GRBroadcast, C4ScriptHost.cpp:234-249)"
        );
        assert!(
            snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "GOOD" && object.owner == 0),
            "the rule's InitializePlayer created its per-player object \
             (the TeamAccount ACNT pattern)"
        );
    }

    #[test]
    fn effect_callbacks_run_in_the_command_targets_object_context_like_cpp() {
        // Every effect callback executes with the effect's command target
        // as object context: pFn->Exec(pCommandTarget, ...)
        // (C4Effect.cpp:129,345,392,456) — `this()` is the command target
        // and its object locals are live. GoldRush's bandit AI depends on
        // both: FxAIBanditNoMoveStart does `this()->~ContextDefend()`,
        // equips via CreateContents, and writes the appended local
        // `iOwner=-2` (Goldrush.c4s/Locals.c4d/AI.c4d/Script.c:96-106).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Boot();\n\
                 return 0;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal iSelf;\n\
             public func Boot() { AddEffect(\"Probe\", this(), 1, 0, this()); return 1; }\n\
             public func Tag() { return 1; }\n\
             func FxProbeStart(pThis, iNumber, fTmp) {\n\
                 if (fTmp) return();\n\
                 this()->~Tag();\n\
                 if (this()) iSelf = 1;\n\
                 CreateContents(GOOD);\n\
             }\n",
        )
        .expect("write target script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD" && object.container.is_none())
            .expect("object created");
        assert_eq!(
            object.local_vars.get("iSelf"),
            Some(&clonk_script::Value::Int(1)),
            "this() inside the Start callback is the command target \
             (C4Effect.cpp:129), and its direct local write persists"
        );
        assert!(
            snapshot
                .objects
                .iter()
                .any(|candidate| candidate.container == Some(object.id)),
            "CreateContents from the Start callback equips the command \
             target (the GoldRush bandit pattern)"
        );
    }

    // FnGetPlrColorDw (C4Script.cpp:3658-3666): the joined player's
    // resolved 24-bit ColorDw; a missing player reads nil. GoldRush's
    // intro movie Talker colors its text with it.
    #[test]
    fn get_plr_color_dw_returns_the_joined_players_color() {
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) { return 0; }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal iColor;\n\
             public func Probe(iPlr) {\n\
                 iColor = GetPlrColorDw(iPlr);\n\
                 return(1);\n\
             }\n",
        )
        .expect("write probe script");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        join_test_player(&mut engine);
        let snapshot = engine.snapshot();
        let object_id = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("crew object exists")
            .id;
        let idx = engine.find_object_index(object_id).expect("object exists");
        engine
            .call_object_function(idx, "Probe", vec![clonk_script::Value::Int(0)])
            .expect("probe runs");
        let idx = engine.find_object_index(object_id).expect("object exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iColor"),
            Some(&clonk_script::Value::Int(0xff0000)),
            "the join color (join_test_player uses color_dw 0xff0000) \
             comes back as C4Player::ColorDw"
        );
    }

    #[test]
    fn goldrush_bandit_order_defend_chain_loads_rifle_like_cpp() {
        // The GoldRush f30 wall: a bandit armed by FxAIBanditNoMoveStart
        // (Goldrush.c4s/Locals.c4d/AI.c4d/Script.c:97-107 — ContextDefend
        // + CreateContents AMBO,AMBO,WINC) must reload at the FIRST
        // OrderDefend timer tick: FxOrderDefendTimer -> ExecuteWatch
        // (Cowboy.c4d/Script.c:641-703) -> WINC::ControlThrow
        // (Winchester.c4d/Script.c:7-31) -> FireRifle (Cowboy:436-456) ->
        // CheckAmmo (Winchester:289-299) -> LoadRifle (Cowboy:499-504);
        // the rifle removes itself and leaves a WCHR crosshair. The
        // fixture distills those scripts with the content call forms.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->Boot();\n\
                 return 0;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/ActMap.txt"),
            "[Action]\nName=Walk\nProcedure=NONE\nDirections=2\nFlipDir=1\nLength=16\nDelay=15\nNextAction=Walk\n\n\
             [Action]\nName=AimRifle\nProcedure=NONE\nDirections=2\nFlipDir=1\nLength=10\nDelay=0\nNextAction=Hold\n\n\
             [Action]\nName=LoadRifle\nProcedure=NONE\nDirections=2\nFlipDir=1\nLength=10\nDelay=3\nEndCall=AimAgain\n",
        )
        .expect("write bandit actmap");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            r#"#strict
local iOwner;
local idWeapon;
local iRifleAmmo;
local iAimPhase;
local pOrdrTarget;
local ordrData1, ordrData2;
local iRewrites, iLastAngle;

public func Boot() {
  SetAction("Walk");
  ContextDefend();
  CreateContents(AMBO);
  CreateContents(AMBO);
  CreateContents(WINC);
  iOwner = -2;
  return(1);
}

public func ContextDefend()
{
  if(GetEffect("Order*", this())) RemoveEffect("Order*", this());
  ordrData1 = GetX(); ordrData2 = GetY();
  AddEffect("OrderDefend", this(), 1, 30, this());
  return(1);
}

func FxOrderDefendTimer(pThis, iNumber)
{
  var iDx=Abs(GetX()-ordrData1), iDy=Abs(GetY()-ordrData2);
  if (!pOrdrTarget)
    if (iDx>20 || iDy>50)
      return(SetCommand(this(),"MoveTo",0,ordrData1,ordrData2));
  if (iDx>150 || iDy>150)
    return(1, pOrdrTarget=0);
  pOrdrTarget=FindEnemyUnit();
  if (!pOrdrTarget) return(ExecuteWatch());
  if (ObjectDistance(pOrdrTarget) > 350) return(ExecuteWatch());
  return(1);
}

private func FindEnemyUnit()
{
  // The real Hostile/GetOwner overrides live in the Goldrush AI.c4d
  // appendto; the fixture pins the no-enemy watch path.
  return(0);
}

private func ExecuteWatch()
{
  var pWeapon;
  if(GetCartridgeCount())
  {
    if(pWeapon = FindContents(WINC)) pWeapon->ControlThrow(this());
    if(idWeapon == WINC && !iRifleAmmo) return(LoadRifle());
  }
  if (Random(3)) return(1);
  if (GetAction() eq "Walk" || GetAction() eq "AimRifle")
  {
    SetDir(Random(2));
    var obj = FindObject(WCHR, 0, 0, 0, 0, 0, "Crosshair", this());
    if(obj) { iRewrites++; iLastAngle = Local(0,obj); SetVertexXY(0,-Sin(Local(0,obj),40)*(GetDir()*2-1),Cos(Local(0,obj),40),obj); }
  }
  return(1);
}

private func GetCartridgeCount() { return(GetSpecialCount("IsBullet")); }

private func GetObjectCount(idObj)
{
  var idUnpackedObj;
  if (idUnpackedObj = idObj->~UnpackTo())
    return(GetObjectCount(idUnpackedObj) * idObj->PackCount());
  return(1);
}

private func GetSpecialCount(szTest)
{
  var iCnt, pObj;
  for(var i = 0; pObj = Contents(i); i++)
    if(ObjectCall(pObj, szTest))
      iCnt++;
  for(var i = 0; pObj = Contents(i); i++)
    if(pObj->~UnpackTo())
      if(DefinitionCall(pObj->~UnpackTo(), szTest))
        iCnt += GetObjectCount(pObj);
  return(iCnt);
}

public func FireRifle()
{
  if (!Contents()->~IsRifle()) return(0);
  if (GetAction() eq "Walk")
    if (SetAction("AimRifle"))
      return(1, SetPhase(6));
  return(0);
}

public func LoadRifle()
{
  if(GetAction() eq "AimRifle") { Sound("RifleLoad"); return(SetAction("LoadRifle")); }
}

// Cowboy.c4d/Script.c:266-282: reload the crosshair from the ammo packs
// and resume aiming; Clonk.c4d/Script.c:396-405 GetCartridge.
protected func AimAgain() {
  if(idWeapon == WINC)
    {
    var obj;
    while(iRifleAmmo < 6 && GetCartridgeCount())
      {
      obj = GetCartridge();
      Enter(WINC->GetCrosshair(this()), obj);
      iRifleAmmo++;
      Sound("RifleLoad2");
      }
    SetAction("AimRifle");
    }
  SetPhase(iAimPhase);
  Sound("RifleLoad2");
}

public func GetCartridge()
  {
  var pObj;
  for(var i = 0; pObj = Contents(i); i++)
    if(pObj->~IsCartridgePack())
      return(pObj->~GetItem());
  return(0);
  }

// The post-reload watch idle (ExecuteWatch's Random-gated branch,
// Cowboy.c4d/Script.c:696-701): re-aims the crosshair vertex from the
// stored angle — the f60 live wall read Local(0,obj) here.
public func DoWatchRewrite() {
  SetDir(1);
  var obj = FindObject(WCHR, 0, 0, 0, 0, 0, "Crosshair", this());
  if(obj) SetVertexXY(0,-Sin(Local(0,obj),40)*(GetDir()*2-1),Cos(Local(0,obj),40),obj);
  return(Local(0,obj));
}
"#,
        )
        .expect("write bandit script");
        let defs_root = dir.path().join("Defs.c4d");
        for (id, defcore, script) in [
            (
                "Ammo.c4d",
                "[DefCore]\nid=AMBO\nName=AmmoBox\nCategory=0\nCrewMember=0\n",
                r#"#strict
local iUsedItems;
public func UnpackTo() { return(CSHO); }
public func IsCartridgePack() { return(1); }
public func MaxPackCount() { return(20); }
public func PackCount() { return(MaxPackCount()-LocalN("iUsedItems")); }
public func DoPackCount(iChange)
{
  iUsedItems-=iChange;
  if(PackCount()<=0) return(RemoveObject());
}
public func GetItem()
{
  var obj = CreateContents(UnpackTo(), Contained());
  DoPackCount(-1);
  return(obj);
}
"#,
            ),
            (
                "Shot.c4d",
                "[DefCore]\nid=CSHO\nName=Shot\nCategory=0\nCrewMember=0\n",
                "#strict\npublic func IsBullet() { return(1); }\n",
            ),
            (
                "Crosshair.c4d",
                // Vertices=1 like the shipped crosshair
                // (Western.c4d/Items.c4d/Weapons.c4d/Winchester.c4d/
                // Crosshair.c4d/DefCore.txt): FnSetVertex writes a C4Shape
                // slot but never grows VtxNum, so a vertex-less fixture would
                // leave SetVertexXY writing a dormant slot
                // (src/C4Script.cpp:1310-1323).
                "[DefCore]\nid=WCHR\nName=Crosshair\nCategory=0\nCrewMember=0\nVertices=1\n",
                "#strict\n",
            ),
            (
                "Winchester.c4d",
                "[DefCore]\nid=WINC\nName=Winchester\nCategory=0\nCrewMember=0\nCollectible=1\n",
                r#"#strict
public func IsRifle() { return(1); }
public func ControlThrow(pClonk)
{
  if(!(pClonk->~FireRifle()))
  {
    if(!GetPlrDownDouble(pClonk->GetOwner()))
      return(1);
    else
      return(0);
  }
  SetPhase(6, pClonk);
  var pCross = CreateObject(WCHR, 0, 0, GetOwner(pClonk)); pCross->SetAction("Crosshair", pClonk);
  Local(0,GetCrosshair(pClonk)) = 84;
  WINC->ActualizePhase(pClonk);
  LocalN("iRifleAmmo", pClonk) = ContentsCount();
  while(Contents()) Enter(pCross, Contents());
  LocalN("idWeapon",pClonk)=GetID();
  if(!LocalN("iRifleAmmo", pClonk)) DefinitionCall(GetID(), "CheckAmmo", pClonk);
  RemoveObject();
  return(1);
}
protected func CheckAmmo(pClonk)
{
  if((GetAction(pClonk) ne "AimRifle") && (GetAction(pClonk) ne "RideAimRifle")) return(0);
  if(!pClonk->~GetCartridgeCount()) return(Sound("RevolverNoAmmo", 0, pClonk));
  LocalN("iAimPhase", pClonk)=GetPhase(pClonk);
  pClonk->~LoadRifle();
}
public func GetCrosshair(pClonk)
{
  return(FindObject(WCHR, 0, 0, 0, 0, 0, "Crosshair", pClonk));
}
public func ActualizePhase(pClonk)
{
  var iDir = GetDir(pClonk)*2-1;
  var iAngle = Local(0,GetCrosshair(pClonk));
  var pObj = FindObject(WCHR,0,0,0,0,0,"Crosshair",pClonk);
  SetVertexXY(0,-Sin(iAngle,40)*iDir,Cos(iAngle,40),pObj);
  if(!WildcardMatch(GetAction(pClonk),"*Aim*")) return(1);
  if(iAngle< 90) SetPhase(6,pClonk);
  return(1);
}
"#,
            ),
        ] {
            let def_dir = defs_root.join(id);
            std::fs::create_dir_all(&def_dir).expect("def dir");
            std::fs::write(def_dir.join("DefCore.txt"), defcore).expect("defcore");
            std::fs::write(def_dir.join("Script.c"), script).expect("script");
            write_test_definition_graphics(&def_dir);
        }
        // WCHR needs its Crosshair action for SetAction (content ActMap).
        std::fs::write(
            defs_root.join("Crosshair.c4d/ActMap.txt"),
            "[Action]\nName=Crosshair\nProcedure=ATTACH\nLength=1\nDelay=1\nNextAction=Crosshair\n",
        )
        .expect("crosshair actmap");

        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        // The live game installs planet/System.c4g before the scenario;
        // WINC::ActualizePhase needs its SetVertexXY helper
        // (planet/System.c4g/Commits.c:68-76).
        engine.install_global_scripts(&[(
            "Commits.c".to_string(),
            "#strict\n\
             global func SetVertexXY(int index, int x, int y, object obj) {\n\
               if (!obj && !this()) return(0);\n\
               if (!SetVertex(index, 0, x, obj)) return(0);\n\
               if (!SetVertex(index, 1, y, obj)) return(0);\n\
               return(1);\n\
             }\n"
            .to_string(),
        )]);
        // The OrderDefend effect (interval 30) first fires 30 execs after
        // the scenario-apply Boot (C4Effect::Execute iTime % iIntervall,
        // C4Effect.cpp:340-345).
        for _ in 0..31 {
            engine.tick_without_snapshot().expect("tick");
        }
        let snapshot = engine.snapshot();
        let bandit = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("bandit exists");
        assert_eq!(
            bandit.action.name, "LoadRifle",
            "the first OrderDefend tick walks the whole load chain \
             (cpp GoldRush f30: bandits enter LoadRifle)"
        );
        assert_eq!(
            bandit.local_vars.get("idWeapon"),
            Some(&clonk_script::Value::C4Id("WINC".into())),
            "ControlThrow recorded the weapon id on the clonk \
             (Winchester.c4d/Script.c:25)"
        );
        assert_eq!(
            bandit.local_vars.get("iRifleAmmo"),
            Some(&clonk_script::Value::Int(0)),
            "the empty rifle left iRifleAmmo=0 (Winchester:22)"
        );
        assert_eq!(
            bandit.local_vars.get("iAimPhase"),
            Some(&clonk_script::Value::Int(6)),
            "CheckAmmo stored the aim phase before reloading \
             (Winchester:297, FireRifle set phase 6)"
        );
        assert_eq!(
            snapshot
                .objects
                .iter()
                .filter(|object| object.definition_id == "WCHR")
                .count(),
            1,
            "ControlThrow created the crosshair (Winchester:18)"
        );
        assert!(
            !snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "WINC"),
            "the rifle removed itself at the end of ControlThrow \
             (Winchester:29 - cpp removes the bandits' rifles at f30)"
        );

        // The f60 class, effect-scope form: keep ticking until the
        // Random-gated watch idle (Cowboy.c4d/Script.c:694-702) fires at
        // least once INSIDE FxOrderDefendTimer; the re-read of the stored
        // angle must be 84, never nil (a nil read flattens the vertex to
        // Sin(0,40)=0 — the f60 live wall's crosshairs at owner+0).
        let bandit_id = bandit.id;
        for _ in 0..200 {
            engine.tick_without_snapshot().expect("tick");
        }
        let bandit_idx = engine.find_object_index(bandit_id).expect("bandit exists");
        let rewrites = engine.objects[bandit_idx]
            .state
            .local_vars
            .get("iRewrites")
            .cloned()
            .unwrap_or(clonk_script::Value::Nil);
        assert!(
            matches!(rewrites, clonk_script::Value::Int(n) if n >= 1),
            "the watch idle rewrite fired at least once in 200 ticks \
             (got {rewrites:?})"
        );
        assert_eq!(
            engine.objects[bandit_idx]
                .state
                .local_vars
                .get("iLastAngle"),
            Some(&clonk_script::Value::Int(84)),
            "the effect-scope Local(0,obj) re-read sees the stored angle"
        );
        let snapshot = engine.snapshot();
        let cross_state = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "WCHR")
            .expect("crosshair exists");
        let vertex = cross_state.vertices.first().expect("crosshair vertex");
        assert_eq!(
            vertex.x.abs(),
            40,
            "the rewritten vertex keeps the Sin(84,40) magnitude \
             (got {:?})",
            (vertex.x, vertex.y)
        );

        let bandit_idx = engine.find_object_index(bandit_id).expect("bandit exists");
        let angle = engine
            .call_object_function(bandit_idx, "DoWatchRewrite", Vec::new())
            .expect("watch rewrite runs");
        assert_eq!(
            angle,
            clonk_script::Value::Int(84),
            "Local(0, FindObject(WCHR, ..., \"Crosshair\", this())) reads \
             the angle stored at load time (Winchester.c4d/Script.c:19)"
        );
        let snapshot = engine.snapshot();
        let cross = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "WCHR")
            .expect("crosshair exists");
        let vertex = cross.vertices.first().expect("crosshair vertex");
        assert_eq!(
            (vertex.x, vertex.y),
            (-40, 4),
            "the rewrite recomputes the vertex from the stored 84 and the \
             Right facing (-Sin(84,40)*1, Cos(84,40))"
        );
    }

    #[test]
    fn namespaced_object_calls_reresolve_on_the_target_definition() {
        // `obj->ID::Func(...)` validates ID::Func at parse time, but C++
        // ignores AB_CALLNS at execution and the paired AB_CALL resolves
        // Func on the arrow target (C4AulExec.cpp:1212-1267). GOOD's Tag
        // therefore overrides HLPR's parse-time function.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "global func Initialize(state, random) {\n\
                 var obj = CreateObject(GOOD, 50, 50, -1);\n\
                 obj->HLPR::Tag();\n\
                 return 0;\n\
             }\n",
        );
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nlocal seen;\npublic func Tag() { seen = 1; return seen; }\n",
        )
        .expect("write target script");
        let helper = dir.path().join("Defs.c4d/Helper.c4d");
        std::fs::create_dir_all(&helper).expect("helper dir");
        std::fs::write(
            helper.join("DefCore.txt"),
            "[DefCore]\nid=HLPR\nName=Helper\nCategory=0\nCrewMember=0\n",
        )
        .expect("write helper defcore");
        std::fs::write(
            helper.join("Script.c"),
            "#strict\nlocal seen;\npublic func Tag() { seen = 5; return seen; }\n",
        )
        .expect("write helper script");
        write_test_definition_graphics(&helper);

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let snapshot = engine.snapshot();
        let object = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GOOD")
            .expect("object created");
        assert_eq!(
            object.local_vars.get("seen"),
            Some(&clonk_script::Value::Int(1)),
            "GOOD's same-name function wins when AB_CALL re-resolves"
        );
    }

    #[test]
    fn legacy_scenario_callbacks_use_the_cpp_argument_convention() {
        // C++ scenario calls pass NO synthetic state argument:
        // Game.Script.Call(PSF_Initialize) has no parameters and
        // GRBroadcast(PSF_InitializePlayer, {plr, x, y, base, team, extra})
        // starts with the PLAYER NUMBER (C4Player.cpp:769-775). The
        // state-proplist convention stays a JSON-fixture convenience —
        // legacy content had been receiving shifted arguments
        // (GoldRush's GetCrew(iPlr, ...) got the state map as iPlr).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static joined_player;\nstatic init_arg;\n\
             global func Initialize(first) { init_arg = first; return 0; }\n\
             global func InitializePlayer(plr) { joined_player = plr; return 0; }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_arg")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Nil),
            "Initialize runs with NO arguments for legacy content"
        );
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
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("joined_player")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Nil),
            "the player NUMBER is 0, which pre-strict-3 engine calls normalize to nil"
        );
    }

    #[test]
    fn network_join_retains_the_authoritative_runtime_client_owner() {
        // C4ControlJoinPlayer passes iAtClient through Game::JoinPlayer into
        // C4Player::Init before any player callbacks run; C4Player stores it
        // as AtClient (pristine 9ffa0a5d src/C4Control.cpp:691-768;
        // src/C4Game.cpp:3505-3514; src/C4Player.cpp:246-265).
        let mut engine = crate::Engine::new();
        let joined = engine
            .join_player_at_client(
                crate::JoinPlayerConfig {
                    name: "Remote".to_string(),
                    player_info_id: 41,
                    score: 0,
                    rounds: 0,
                    rounds_won: 0,
                    rounds_lost: 0,
                    total_playing_time: 0,
                    team: None,
                    color_dw: 0x00ff_0000,
                    pref_color: 0,
                    pref_position: 0,
                    crew: Vec::new(),
                    startup_player_count: 1,
                    control_style: false,
                    auto_context_menu: false,
                },
                crate::PlayerAtClient::new(7),
            )
            .expect("remote player joins");

        assert_eq!(
            engine
                .player(joined.number())
                .expect("joined player exists")
                .at_client(),
            crate::PlayerAtClient::new(7)
        );
    }

    #[test]
    fn surrender_control_requires_the_runtime_player_owner() {
        // C4ControlInternalPlayerScriptBase::Allowed accepts a player control
        // only when C4Player::AtClient equals its inherited iByClient; the
        // accepted C4ControlSurrenderPlayer then calls SurrenderPlayer
        // (pristine 9ffa0a5d src/C4Control.cpp:1546-1578;
        // src/C4Control.h:589-594; src/C4Script.cpp:2849-2855).
        let mut engine = crate::Engine::new();
        let info = crate::ControlPlayerInfoEntry {
            flags: crate::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK,
            ..crate::ControlPlayerInfoEntry::default()
        };
        let joined = engine
            .join_player_at_client_with_info(
                crate::JoinPlayerConfig {
                    name: "Remote".to_string(),
                    player_info_id: 42,
                    score: 0,
                    rounds: 0,
                    rounds_won: 0,
                    rounds_lost: 0,
                    total_playing_time: 0,
                    team: None,
                    color_dw: 0x0000_ff00,
                    pref_color: 0,
                    pref_position: 0,
                    crew: Vec::new(),
                    startup_player_count: 1,
                    control_style: false,
                    auto_context_menu: false,
                },
                crate::PlayerAtClient::new(3),
                &info,
            )
            .expect("remote player joins");
        let player = joined.number();

        assert!(
            !engine.execute_surrender_player_control(crate::SurrenderPlayerControlData {
                player,
                by_client: 7,
            })
        );
        assert!(!engine.player(player).expect("player exists").surrendered());

        assert!(
            engine.execute_surrender_player_control(crate::SurrenderPlayerControlData {
                player,
                by_client: 3,
            })
        );
        assert!(engine.player(player).expect("player exists").surrendered());
        for _ in 0..59 {
            engine
                .tick_player_systems()
                .expect("surrender retirement advances");
        }
        assert!(engine.player(player).is_some());
        engine
            .tick_player_systems()
            .expect("sixtieth execute retires surrendered player");
        assert!(engine.player(player).is_none());
    }

    #[test]
    fn team_choice_join_stops_after_preinitialize_like_cpp() {
        // C4Player::Init postpones ScenarioInit while a teamless user is in
        // PS_TeamSelection: it registers the player, runs
        // PreInitializePlayer exactly once, and does not consume the
        // ScenarioInit RNG ledger, place ready crew, or call
        // InitializePlayer (C4Player.cpp:299-320, 344-349).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static preinit_count, init_count;\n\
             global func Initialize() { preinit_count = 0; init_count = 0; }\n\
             global func PreInitializePlayer(plr) { preinit_count = preinit_count + 1; }\n\
             global func InitializePlayer(plr) { init_count = init_count + 1; }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let rng_before = engine.rng.clone();
        let objects_before = engine.snapshot().objects;

        let number = engine
            .join_player_for_team_selection(crate::JoinPlayerConfig {
                name: "Chooser".to_string(),
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
            .expect("team-choice join succeeds");

        assert_eq!(number, 0);
        assert_eq!(
            engine.player(number).map(crate::Player::status),
            Some(crate::PlayerStatus::TeamSelection)
        );
        assert_eq!(engine.rng, rng_before, "ScenarioInit consumed no RNG");
        assert_eq!(
            engine.snapshot().objects,
            objects_before,
            "ScenarioInit placed no ready objects"
        );
        let global = |name: &str| {
            engine
                .script_globals
                .borrow()
                .get(name)
                .map(|cell| cell.borrow().clone())
        };
        assert_eq!(global("preinit_count"), Some(clonk_script::Value::Int(1)));
        assert_eq!(global("init_count"), Some(clonk_script::Value::Nil));
    }

    #[test]
    fn custom_active_teams_automatically_defer_teamless_user_join() {
        // C4TeamList::IsRuntimeJoinTeamChoice is exactly IsCustom &&
        // IsMultiTeams. A non-script player whose team does not resolve is
        // therefore registered in PS_TeamSelection before PreInitialize,
        // and ordinary C4Player::Init skips ScenarioInit
        // (C4Teams.h:186; C4Player.cpp:299-320, 344-349).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static init_count;\n\
             global func Initialize() { init_count = 0; }\n\
             global func InitializePlayer() { init_count = init_count + 1; }\n",
        );
        std::fs::write(
            scenario_dir.join("Teams.txt"),
            "[Teams]\n\
             AllowHostilityChange=0\n\
             AllowTeamSwitch=0\n\
             \t[Team]\n\
             \tid=1\n\
             \tName=Left\n\
             \t[Team]\n\
             \tid=2\n\
             \tName=Right\n",
        )
        .expect("write custom teams");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let rng_before = engine.rng.clone();

        let outcome = engine
            .join_player(crate::JoinPlayerConfig {
                name: "Chooser".to_string(),
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
            .expect("join registers");

        assert_eq!(
            outcome,
            crate::JoinPlayerOutcome::AwaitingTeamSelection { number: 0 }
        );
        assert_eq!(
            engine.player(0).map(crate::Player::status),
            Some(crate::PlayerStatus::TeamSelection)
        );
        assert_eq!(engine.rng, rng_before);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_count")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Nil)
        );
    }

    #[test]
    fn synchronized_team_choice_resumes_scenario_init_like_cpp() {
        // DoTeamSelection first changes PS_TeamSelection to
        // PS_TeamSelectionPending. When CID_InitScenarioPlayer executes,
        // ScenarioAndTeamInit assigns the team and resumes ScenarioInit plus
        // FinalInit without repeating PreInitializePlayer
        // (C4Player.cpp:111-151, 1774-1780).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static preinit_count, init_count, initialized_team;\n\
             global func Initialize() { preinit_count = 0; init_count = 0; initialized_team = 0; }\n\
             global func PreInitializePlayer(plr) { preinit_count = preinit_count + 1; }\n\
             global func InitializePlayer(plr, x, y, base, team) { init_count = init_count + 1; initialized_team = team; }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        engine.set_teams(vec![
            crate::TeamInfo::new(1, "Left", 0x00f4_0000),
            crate::TeamInfo::new(2, "Right", 0x0000_c800),
        ]);
        let number = engine
            .join_player_for_team_selection(crate::JoinPlayerConfig {
                name: "Chooser".to_string(),
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
            .expect("team-choice join succeeds");
        let rng_before = engine.rng.clone();
        let object_count_before = engine.snapshot().objects.len();

        engine
            .mark_team_selection_pending(number)
            .expect("selection request is accepted");
        assert_eq!(
            engine.player(number).map(crate::Player::status),
            Some(crate::PlayerStatus::TeamSelectionPending)
        );
        let joined = engine
            .initialize_scenario_player(number, 1)
            .expect("selection control executes")
            .expect("team is accepted");

        assert_eq!(joined.number, number);
        let player = engine.player(number).expect("player remains registered");
        assert_eq!(player.status(), crate::PlayerStatus::Active);
        assert_eq!(player.team(), Some(1));
        assert_ne!(
            engine.rng, rng_before,
            "ScenarioInit consumed its RNG ledger"
        );
        assert!(
            engine.snapshot().objects.len() > object_count_before,
            "ready crew was placed"
        );
        let global = |name: &str| {
            engine
                .script_globals
                .borrow()
                .get(name)
                .map(|cell| cell.borrow().clone())
        };
        assert_eq!(global("preinit_count"), Some(clonk_script::Value::Int(1)));
        assert_eq!(global("init_count"), Some(clonk_script::Value::Int(1)));
        assert_eq!(
            global("initialized_team"),
            Some(clonk_script::Value::Int(1))
        );
    }

    #[test]
    fn runtime_new_team_choice_generates_next_cpp_team_and_resumes_join() {
        // C4Player::ScenarioAndTeamInit resolves TEAMID_New through
        // GetGenerateTeamByID. CreateTeam uses the next ID, the localized
        // default name, zero start/max/icon metadata, and RecheckColor's
        // fixed palette before ScenarioInit resumes (C4Player.cpp:111-151;
        // C4Teams.cpp:181-218,375-418). Fixed palette colors consume no
        // SafeRandom or lockstep Random draws.
        let config = crate::JoinPlayerConfig {
            name: "Chooser".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x0011_2233,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            startup_player_count: 1,
            control_style: false,
            auto_context_menu: false,
        };
        let first_team = crate::TeamInfo::new(1, "Existing", 0x00f4_0000);
        let second_team = crate::TeamInfo::new(2, "Team 2", 0x0000_c800);

        let mut generated = Engine::new();
        generated.set_teams(vec![first_team.clone()]);
        generated.set_team_colors(true);
        generated.set_auto_generate_teams(true);
        let number = generated
            .join_player_for_team_selection(config.clone())
            .expect("generated-team chooser registers");
        generated
            .mark_team_selection_pending(number)
            .expect("generated-team choice is pending");

        let mut reference = Engine::new();
        reference.set_teams(vec![first_team.clone(), second_team.clone()]);
        reference.set_team_colors(true);
        let reference_number = reference
            .join_player_for_team_selection(config)
            .expect("existing-team chooser registers");
        reference
            .mark_team_selection_pending(reference_number)
            .expect("existing-team choice is pending");

        let joined = generated
            .initialize_scenario_player(number, -1)
            .expect("TEAMID_New control executes")
            .expect("generated team is accepted");
        let reference_joined = reference
            .initialize_scenario_player(reference_number, 2)
            .expect("existing-team control executes")
            .expect("existing team is accepted");

        assert_eq!(generated.teams(), reference.teams());
        assert_eq!(
            generated.teams()[1].player_ids,
            vec![generated
                .player(number)
                .expect("chooser remains joined")
                .player_info_id()]
        );
        assert_eq!(joined, reference_joined);
        assert_eq!(generated.rng, reference.rng, "no lockstep RNG drift");
        let player = generated.player(number).expect("chooser remains joined");
        assert_eq!(
            player.status(),
            crate::PlayerStatus::Eliminated,
            "an empty C++ scenario eliminates the crewless player during frame-zero FinalInit"
        );
        assert_eq!(player.team(), Some(2));
        assert_eq!(player.color(), Some(crate::RgbColor::new(0x00, 0xc8, 0x00)));
    }

    #[test]
    fn runtime_new_team_choice_is_rejected_when_auto_generation_is_disabled() {
        // ScenarioAndTeamInit rejects TEAMID_New unless
        // IsAutoGenerateTeams is true, calls OnTeamSelectionFailed, and
        // leaves ScenarioInit untouched so the player may retry
        // (C4Player.cpp:111-143,2256-2261).
        let mut engine = Engine::new();
        engine.set_teams(vec![crate::TeamInfo::new(1, "Existing", 0x00f4_0000)]);
        let number = engine
            .join_player_for_team_selection(crate::JoinPlayerConfig {
                name: "Chooser".to_string(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0x0011_2233,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("team chooser registers");
        engine
            .mark_team_selection_pending(number)
            .expect("new-team choice is pending");
        let teams_before = engine.teams().to_vec();
        let rng_before = engine.rng.clone();

        assert!(
            engine
                .initialize_scenario_player(number, -1)
                .expect("TEAMID_New control executes")
                .is_none(),
            "disabled auto-generation rejects TEAMID_New"
        );

        assert_eq!(engine.teams(), teams_before);
        assert_eq!(engine.rng, rng_before, "ScenarioInit did not run");
        let player = engine.player(number).expect("chooser remains registered");
        assert_eq!(player.status(), crate::PlayerStatus::TeamSelection);
        assert_eq!(player.team(), None);
    }

    #[test]
    fn runtime_generated_team_keeps_process_random_color_explicitly_unresolved() {
        // RecheckColor uses the fixed table only for the first team IDs;
        // later IDs call process-global SafeRandom until a non-conflicting
        // color is found (C4Teams.cpp:181-218;
        // C4PlayerInfoConflicts.cpp:36-41). That stream is not the lockstep
        // Random ledger and cannot be derived from scenario state. Until the
        // host-selected color is transported, zero is the explicit unresolved
        // team-color marker and must not turn the joined player black.
        let config = crate::JoinPlayerConfig {
            name: "Chooser".to_string(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x0011_2233,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            startup_player_count: 1,
            control_style: false,
            auto_context_menu: false,
        };
        let existing = crate::TeamInfo::new(11, "Existing", 0x0055_6677);
        let unresolved = crate::TeamInfo::new(12, "Team 12", 0);

        let mut generated = Engine::new();
        generated.set_teams(vec![existing.clone()]);
        generated.set_team_colors(true);
        generated.set_auto_generate_teams(true);
        let number = generated
            .join_player_for_team_selection(config.clone())
            .expect("generated-team chooser registers");
        generated
            .mark_team_selection_pending(number)
            .expect("generated-team choice is pending");

        let mut reference = Engine::new();
        reference.set_teams(vec![existing.clone(), unresolved.clone()]);
        reference.set_team_colors(true);
        let reference_number = reference
            .join_player_for_team_selection(config)
            .expect("existing-team chooser registers");
        reference
            .mark_team_selection_pending(reference_number)
            .expect("existing-team choice is pending");

        generated
            .initialize_scenario_player(number, -1)
            .expect("TEAMID_New control executes")
            .expect("generated team is accepted");
        reference
            .initialize_scenario_player(reference_number, 12)
            .expect("existing-team control executes")
            .expect("existing team is accepted");

        assert_eq!(generated.teams(), reference.teams());
        assert_eq!(
            generated.teams()[1].player_ids,
            vec![generated
                .player(number)
                .expect("chooser remains joined")
                .player_info_id()]
        );
        assert_eq!(generated.rng, reference.rng, "no lockstep RNG drift");
        assert_eq!(
            generated.player(number).and_then(crate::Player::color),
            Some(crate::RgbColor::new(0x11, 0x22, 0x33)),
            "an unresolved process-random color is not applied as black"
        );
    }

    #[test]
    fn runtime_team_choice_applies_enabled_team_colors_before_initialize_player() {
        // C4Team::AddPlayer changes both C4PlayerInfo::Color and the joined
        // C4Player::ColorDw before ScenarioAndTeamInit calls ScenarioInit;
        // ScenarioInit reloads ColorDw before InitializePlayer observes it
        // (C4Teams.cpp:53-81; C4Player.cpp:111-151, 670-693, 769-775).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static initialized_name;\n\
             global func Initialize() { initialized_name = \"\"; }\n\
             global func InitializePlayer(plr) { initialized_name = GetTaggedPlayerName(plr); }\n",
        );
        std::fs::write(
            scenario_dir.join("Teams.txt"),
            "[Teams]\n\
             TeamColors=1\n\
             \t[Team]\n\
             \tid=1\n\
             \tName=Orange\n\
             \tColor=16746496\n\
             \t[Team]\n\
             \tid=2\n\
             \tName=Green\n\
             \tColor=4513160\n",
        )
        .expect("write custom teams");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);

        for (name, info_color, team, team_color, tagged_name) in [
            (
                "Alice",
                0x0011_2233,
                1,
                crate::RgbColor::new(0xff, 0x88, 0x00),
                "<c ff8800>Alice</c>",
            ),
            (
                "Bob",
                0x00aa_33cc,
                2,
                crate::RgbColor::new(0x44, 0xdd, 0x88),
                "<c 44dd88>Bob</c>",
            ),
        ] {
            let outcome = engine
                .join_player(crate::JoinPlayerConfig {
                    name: name.to_string(),
                    player_info_id: 0,
                    score: 0,
                    rounds: 0,
                    rounds_won: 0,
                    rounds_lost: 0,
                    total_playing_time: 0,
                    team: None,
                    color_dw: info_color,
                    pref_color: 0,
                    pref_position: 0,
                    crew: Vec::new(),
                    startup_player_count: 2,
                    control_style: false,
                    auto_context_menu: false,
                })
                .expect("team-choice join registers");
            let number = outcome.number();
            assert_eq!(
                outcome,
                crate::JoinPlayerOutcome::AwaitingTeamSelection { number }
            );
            engine
                .mark_team_selection_pending(number)
                .expect("selection request is accepted");
            engine
                .initialize_scenario_player(number, team)
                .expect("selection control executes")
                .expect("team is accepted");

            let player = engine
                .snapshot()
                .players
                .into_iter()
                .find(|player| player.id == number)
                .expect("selected player is in the runtime snapshot");
            assert_eq!(player.color, Some(team_color));
            assert_eq!(
                engine
                    .script_globals
                    .borrow()
                    .get("initialized_name")
                    .map(|cell| cell.borrow().clone()),
                Some(clonk_script::Value::String(tagged_name.to_string().into())),
                "InitializePlayer observes the selected team's color"
            );
        }
    }

    #[test]
    fn runtime_team_choice_preserves_player_info_color_when_team_colors_are_disabled() {
        // C4Team::AddPlayer always assigns the team, but gates both player
        // info and runtime color changes on C4TeamList::IsTeamColors
        // (C4Teams.cpp:68-80).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static initialized_name;\n\
             global func Initialize() { initialized_name = \"\"; }\n\
             global func InitializePlayer(plr) { initialized_name = GetTaggedPlayerName(plr); }\n",
        );
        std::fs::write(
            scenario_dir.join("Teams.txt"),
            "[Teams]\n\
             TeamColors=0\n\
             \t[Team]\n\
             \tid=1\n\
             \tName=Orange\n\
             \tColor=16746496\n\
             \t[Team]\n\
             \tid=2\n\
             \tName=Green\n\
             \tColor=4513160\n",
        )
        .expect("write custom teams");
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        let outcome = engine
            .join_player(crate::JoinPlayerConfig {
                name: "Solo".to_string(),
                player_info_id: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0x0055_cc88,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("team-choice join registers");
        let number = outcome.number();
        engine
            .mark_team_selection_pending(number)
            .expect("selection request is accepted");
        engine
            .initialize_scenario_player(number, 2)
            .expect("selection control executes")
            .expect("team is accepted");

        let player = engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == number)
            .expect("selected player is in the runtime snapshot");
        assert_eq!(player.team, Some(2));
        assert_eq!(player.color, Some(crate::RgbColor::new(0x55, 0xcc, 0x88)));
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("initialized_name")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::String(
                "<c 55cc88>Solo</c>".to_string().into()
            )),
            "InitializePlayer retains the player-info color"
        );
    }

    #[test]
    fn full_team_rejects_synchronized_choice_and_reopens_selection() {
        // ScenarioAndTeamInit asks C4TeamList::IsJoin2TeamAllowed before it
        // mutates the player. A full team rejects the control and
        // OnTeamSelectionFailed changes only PS_TeamSelectionPending back
        // to PS_TeamSelection (C4Player.cpp:130-143, 2256-2261;
        // C4Teams.cpp:545-560).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static init_count;\n\
             global func Initialize() { init_count = 0; }\n\
             global func InitializePlayer() { init_count = init_count + 1; }\n",
        );
        let (mut engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        engine.set_teams(vec![
            crate::TeamInfo::new(1, "Full", 0x00f4_0000).with_max_players(1),
            crate::TeamInfo::new(2, "Open", 0x0000_c800),
        ]);
        engine
            .register_player(crate::PlayerConfig::new(99, "Occupant").with_team(Some(1)))
            .expect("occupant registers");
        let number = engine
            .join_player_for_team_selection(crate::JoinPlayerConfig {
                name: "Chooser".to_string(),
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
            .expect("team-choice join succeeds");
        engine
            .mark_team_selection_pending(number)
            .expect("selection request is accepted");
        let rng_before = engine.rng.clone();
        let objects_before = engine.snapshot().objects;
        let init_count_before = engine
            .script_globals
            .borrow()
            .get("init_count")
            .map(|cell| cell.borrow().clone());

        assert!(
            engine
                .initialize_scenario_player(number, 1)
                .expect("selection control executes")
                .is_none(),
            "full team rejects the join"
        );
        let player = engine.player(number).expect("chooser remains registered");
        assert_eq!(player.status(), crate::PlayerStatus::TeamSelection);
        assert_eq!(player.team(), None);
        assert_eq!(engine.rng, rng_before);
        assert_eq!(engine.snapshot().objects, objects_before);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("init_count")
                .map(|cell| cell.borrow().clone()),
            init_count_before
        );
    }

    #[test]
    fn forced_team_selection_ignores_a_full_alternative() {
        // GetForcedTeamSelection skips every full team unless it is already
        // the player's current team. With one remaining non-full team, that
        // team's ID is forced (C4Teams.cpp:876-914).
        let mut engine = Engine::new();
        engine.set_teams(vec![
            crate::TeamInfo::new(1, "Full", 0x00f4_0000).with_max_players(1),
            crate::TeamInfo::new(2, "Open", 0x0000_c800),
        ]);
        engine
            .register_player(crate::PlayerConfig::new(0, "Chooser"))
            .expect("chooser registers");
        engine
            .register_player(crate::PlayerConfig::new(1, "Occupant").with_team(Some(1)))
            .expect("occupant registers");

        assert_eq!(engine.forced_team_selection(0), Some(2));
    }

    #[test]
    fn forced_team_selection_rejects_multiple_or_generated_alternatives() {
        // A second non-full team makes the choice ambiguous. Likewise,
        // AutoGenerateTeams always leaves a possible new-team choice, even
        // when only one existing team is joinable (C4Teams.cpp:887-906).
        let mut engine = Engine::new();
        engine.set_teams(vec![
            crate::TeamInfo::new(1, "Left", 0x00f4_0000),
            crate::TeamInfo::new(2, "Right", 0x0000_c800),
        ]);
        engine
            .register_player(crate::PlayerConfig::new(0, "Chooser"))
            .expect("chooser registers");
        assert_eq!(engine.forced_team_selection(0), None);

        engine.set_teams(vec![crate::TeamInfo::new(1, "Solo", 0x00f4_0000)]);
        engine.set_auto_generate_teams(true);
        assert_eq!(engine.forced_team_selection(0), None);
    }

    #[test]
    fn definition_pack_system_groups_load_into_the_global_engine() {
        // C4DefList::Load opens C4CFN_System inside every definition
        // group and registers its scripts with Game.ScriptEngine
        // (C4Def.cpp:956-977) — Western.c4d/System.c4g carries Find_Clan
        // and friends. They must be callable like any global script.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "static probed;\n\
             global func Initialize() { probed = PackHelper() + PACK_ORDER; return 0; }\n",
        );
        let system = dir.path().join("Defs.c4d/System.c4g");
        std::fs::create_dir_all(&system).expect("system dir");
        std::fs::write(
            system.join("Helpers.c"),
            "#strict\nstatic const PACK_ORDER = 20;\n\
             global func PackHelper() { return 6; }\n",
        )
        .expect("write pack script");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/Script.c"),
            "#strict\nstatic const PACK_ORDER = 10;\n",
        )
        .expect("write definition script");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("probed")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(26)),
            "the pack's System.c4g loads after its child definitions"
        );
    }

    #[test]
    fn nested_scenario_definition_pack_reenables_system_loading() {
        // InitDefs disables System loading only for the scenario root call.
        // Recursive *.c4d loads use fLoadSysGroups=true again
        // (C4Def.cpp:903-907,939-968).
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(
            dir.path(),
            None,
            "#strict\nstatic nested_result;\n\
             global func Initialize() { nested_result = NestedHelper(); }\n",
        );
        let nested_system = scenario_dir.join("Helpers.c4d/System.c4g");
        std::fs::create_dir_all(&nested_system).expect("nested system dir");
        std::fs::write(
            nested_system.join("Helpers.c"),
            "#strict\nglobal func NestedHelper() { return 73; }\n",
        )
        .expect("write nested helper");

        let (engine, _created) = apply_resilience_fixture(&dir, &scenario_dir);
        assert_eq!(
            engine
                .script_globals
                .borrow()
                .get("nested_result")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(73))
        );
    }

    #[test]
    fn overloaded_definition_keeps_declarations_but_loses_functions_and_appends() {
        // C4Def::Load preparses Script.c before C4DefList::Add replaces an
        // existing ID. Destroying the old host unregisters its functions and
        // appends, but GlobalNamed/GlobalConsts are not rolled back
        // (C4Def.cpp:625-633,927-933,1059-1091; C4Aul.cpp:473-481).
        let dir = tempdir().expect("tempdir");
        let defs = dir.path().join("Defs.c4d");
        let old = defs.join("Old.c4d");
        let target = defs.join("Target.c4d");
        std::fs::create_dir_all(&old).expect("old def dir");
        std::fs::create_dir_all(&target).expect("target def dir");
        std::fs::write(
            old.join("DefCore.txt"),
            "[DefCore]\nid=DUPS\nName=Old\nCategory=0\n",
        )
        .expect("write old core");
        std::fs::write(
            old.join("Script.c"),
            "#strict\nstatic const SURVIVES_REPLACEMENT = 7;\n\
             global func Clash() { return 1; }\n\
             #appendto TARG\npublic func Hook() { return inherited() * 10 + 1; }\n",
        )
        .expect("write old script");
        write_test_definition_graphics(&old);
        std::fs::write(
            target.join("DefCore.txt"),
            "[DefCore]\nid=TARG\nName=Target\nCategory=0\n",
        )
        .expect("write target core");
        std::fs::write(
            target.join("Script.c"),
            "#strict\npublic func Hook() { return 0; }\n",
        )
        .expect("write target script");
        write_test_definition_graphics(&target);
        let pack_system = defs.join("System.c4g");
        std::fs::create_dir_all(&pack_system).expect("pack system dir");
        std::fs::write(
            pack_system.join("Globals.c"),
            "#strict\nglobal func Clash() { return 15; }\n\
             public func PackPrivate() { return 9; }\n",
        )
        .expect("write pack globals");

        let scenario_dir = dir.path().join("Replacement.c4s");
        let replacement = scenario_dir.join("Replacement.c4d");
        std::fs::create_dir_all(&replacement).expect("replacement def dir");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Replacement\n\n[Definitions]\nDefinition1=Defs.c4d\n",
        )
        .expect("write scenario core");
        std::fs::write(
            replacement.join("DefCore.txt"),
            "[DefCore]\nid=DUPS\nName=New\nCategory=0\n",
        )
        .expect("write replacement core");
        std::fs::write(
            replacement.join("Script.c"),
            "#strict\nglobal func Clash() { return 2; }\n\
             #appendto TARG\npublic func Hook() { return inherited() * 10 + 2; }\n",
        )
        .expect("write replacement script");
        write_test_definition_graphics(&replacement);

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        assert_eq!(
            engine
                .script_global_consts
                .borrow()
                .get("SURVIVES_REPLACEMENT")
                .map(|cell| cell.borrow().clone()),
            Some(clonk_script::Value::Int(7)),
            "the removed host's preparsed constant remains registered"
        );
        let globals = engine
            .global_script_functions
            .as_ref()
            .expect("global table exists");
        let clash = globals.get("Clash").expect("replacement Clash exists");
        assert!(clash.overloaded.is_some(), "pack System is inherited");
        assert!(
            clash
                .overloaded
                .as_ref()
                .is_some_and(|parent| parent.overloaded.is_none()),
            "the removed definition function is absent from the overload chain"
        );
        assert!(!globals.contains_key("PackPrivate"));
        let id = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");
        let index = engine.find_object_index(id).expect("target index");
        assert_eq!(
            engine
                .call_object_function(index, "Hook", Vec::new())
                .expect("Hook runs"),
            clonk_script::Value::Int(2),
            "only the live replacement append participates"
        );
    }

    #[test]
    fn join_name_sources_and_map_zoom_follow_cpp() {
        // New crew infos draw their name from the def's ClonkNames list
        // when it has one (C4ObjectInfoList.cpp:160-164, C4Def.cpp:645-652),
        // else from Game.Names — which a scenario Names.txt overrides
        // (C4Game.cpp:3288-3289). A configured [PlayerN] Position
        // multiplies a MapZoom.Evaluate per coordinate
        // (C4Player.cpp:713-714) — one synced draw each.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            dir.path().join("Defs.c4d/Good.c4d/ClonkNames.txt"),
            "Jim\nBob\nJoe\n",
        )
        .expect("write clonk names");
        std::fs::write(dir.path().join("Defs.c4d/Good.c4d/ClonkNamesUS.txt"), [])
            .expect("write empty localized clonk names");
        let plain = dir.path().join("Defs.c4d/Plain.c4d");
        std::fs::create_dir_all(&plain).expect("plain def dir");
        std::fs::write(
            plain.join("DefCore.txt"),
            "[DefCore]\nid=PLAI\nName=Plain\nCategory=0\nCrewMember=1\n",
        )
        .expect("write plain defcore");
        std::fs::write(plain.join("Script.c"), "// plain\n").expect("write plain script");
        write_test_definition_graphics(&plain);
        std::fs::write(scenario_dir.join("Names.txt"), "Alpha\nBeta\n")
            .expect("write scenario names");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Names\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Landscape]\nMapWidth=64\nMapHeight=40\nMapZoom=10\n\n\
             [Player1]\nCrew=GOOD=1;PLAI=1\nPosition=20,30\n",
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(99);
        scenario.apply(&mut engine).expect("scenario applies");

        let mut replay = engine.rng.clone();
        let landscape = engine.landscape().expect("landscape set").clone();

        // Wealth (default C4SVal(0,0,0,250)) — one draw.
        LegacyC4SVal::new(0, 0, 0, 250).evaluate(&mut replay);
        // Position 20,30 with MapZoom (10,0,5,15) — one draw per axis.
        let mut ptx = (20 * LegacyC4SVal::new(10, 0, 5, 15).evaluate(&mut replay)).clamp(0, 639);
        let mut pty = (30 * LegacyC4SVal::new(10, 0, 5, 15).evaluate(&mut replay)).clamp(0, 399);
        if let Some((nx, ny)) = landscape.find_solid_ground(ptx, pty, 30) {
            ptx = nx;
            pty = ny;
        }
        if let Some((nx, ny)) =
            landscape.find_con_site_spot(ptx, pty, 30, 50, 400, |_, _, _, _| false)
        {
            ptx = nx;
            pty = ny;
        }
        let _ = (ptx, pty);
        // Crew member 1 (GOOD): name from ClonkNames — Random over the
        // newline count (3) — then the placement draw.
        let good_names = ["Jim", "Bob", "Joe"];
        let expected_good_name = good_names[replay.random(3) as usize];
        replay.random(60);
        // Crew member 2 (PLAI): no ClonkNames — name from the scenario
        // Names.txt ("Alpha\nBeta\n" has 2 newlines) — then placement.
        let scenario_names = ["Alpha", "Beta"];
        let expected_plain_name = scenario_names[replay.random(2) as usize];
        replay.random(60);

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
        assert_eq!(engine.rng, replay, "draw ledger matches");

        let snapshot = engine.snapshot();
        let names: Vec<(String, String)> = snapshot
            .objects
            .iter()
            .filter(|object| object.crew_member)
            .map(|object| {
                (
                    object.definition_id.clone(),
                    engine
                        .crew_object_info(object.id)
                        .expect("crew info recorded")
                        .name
                        .clone(),
                )
            })
            .collect();
        assert_eq!(
            names,
            vec![
                ("GOOD".to_string(), expected_good_name.to_string()),
                ("PLAI".to_string(), expected_plain_name.to_string()),
            ]
        );
    }

    #[test]
    fn player_start_indexed_lists_preserve_cpp_id_list_entry_streams() {
        let mut player = LegacyPlayer::default();
        player
            .apply_entries(&[
                (
                    "Knowledge".to_string(),
                    "GOOD=7;PLAI=-2;GOOD;PLAI=0".to_string(),
                ),
                (
                    "HomeBaseMaterial".to_string(),
                    "PLAI=-8;GOOD=3;PLAI;GOOD=0".to_string(),
                ),
                (
                    "HomeBaseProduction".to_string(),
                    "GOOD=0;PLAI=12;GOOD=-4;PLAI".to_string(),
                ),
                (
                    "Magic".to_string(),
                    "PLAI;GOOD=-6;PLAI=9;GOOD=0".to_string(),
                ),
            ])
            .expect("player fields compile");

        let start = PlayerStart::from_legacy(&player);
        assert_eq!(
            start.build_knowledge,
            vec![
                ("GOOD".to_string(), 7),
                ("PLAI".to_string(), -2),
                ("GOOD".to_string(), 0),
                ("PLAI".to_string(), 0),
            ]
        );
        assert_eq!(
            start.home_base_material,
            vec![
                ("PLAI".to_string(), -8),
                ("GOOD".to_string(), 3),
                ("PLAI".to_string(), 0),
                ("GOOD".to_string(), 0),
            ]
        );
        assert_eq!(
            start.home_base_production,
            vec![
                ("GOOD".to_string(), 0),
                ("PLAI".to_string(), 12),
                ("GOOD".to_string(), -4),
                ("PLAI".to_string(), 0),
            ]
        );
        assert_eq!(
            start.magic,
            vec![
                ("PLAI".to_string(), 0),
                ("GOOD".to_string(), -6),
                ("PLAI".to_string(), 9),
                ("GOOD".to_string(), 0),
            ]
        );
    }

    #[test]
    fn legacy_player_starts_are_retained_for_the_join_pipeline() {
        // C4SPlrStart (compiled at C4Scenario.cpp:276-291) feeds
        // C4Player::ScenarioInit at join time (C4Player.cpp:670-777):
        // after apply the engine must still know all four start slots —
        // wealth/crew/position/ready lists — for joining players.
        let dir = tempdir().expect("tempdir");
        let scenario_dir = write_resilience_fixture(dir.path(), None, "// no scenario script\n");
        std::fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Starts\n\n[Definitions]\nDefinition1=Defs.c4d\n\n\
             [Player1]\nWealth=50,10,0,250\nCrew=GOOD=2\nBuildings=GOOD=1\n\
             Vehicles=GOOD=1\nMaterial=GOOD=2\nKnowledge=GOOD=1\n\
             HomeBaseMaterial=GOOD=3\nHomeBaseProduction=GOOD=2\nMagic=GOOD=0\n\
             Position=120,160\nEnforcePosition=1\nStandardCrew=GOOD\nClonks=2,0,1,10\n",
        )
        .expect("write scenario core");

        let resolver = FileSystemResolver {
            roots: vec![dir.path().to_path_buf()],
        };
        let scenario =
            Scenario::load_from_path_with(&scenario_dir, &resolver).expect("scenario loads");
        let mut engine = Engine::with_seed(0);
        scenario.apply(&mut engine).expect("scenario applies");

        let start = engine.player_start(0).expect("start slot 0 exists");
        assert_eq!(start.wealth, LegacyC4SVal::new(50, 10, 0, 250));
        assert_eq!(start.crew_count, LegacyC4SVal::new(2, 0, 1, 10));
        assert_eq!(start.native_crew.as_deref(), Some("GOOD"));
        assert_eq!(start.position, [120, 160]);
        assert!(start.enforce_position);
        assert_eq!(start.ready_crew, vec![("GOOD".to_string(), 2)]);
        assert_eq!(start.ready_base, vec![("GOOD".to_string(), 1)]);
        assert_eq!(start.ready_vehic, vec![("GOOD".to_string(), 1)]);
        assert_eq!(start.ready_material, vec![("GOOD".to_string(), 2)]);
        assert_eq!(start.build_knowledge, vec![("GOOD".to_string(), 1)]);
        assert_eq!(start.home_base_material, vec![("GOOD".to_string(), 3)]);
        assert_eq!(start.home_base_production, vec![("GOOD".to_string(), 2)]);
        // A zero count stays zero (GoldRush pins `Magic=EXTG=0;`).
        assert_eq!(start.magic, vec![("GOOD".to_string(), 0)]);

        // Unconfigured slots carry the C4SPlrStart defaults
        // (C4Scenario.cpp:294-300 Default()): Wealth (0,0,0,250),
        // Clonks (1,0,1,10), Position (-1,-1).
        let other = engine.player_start(3).expect("start slot 3 exists");
        assert_eq!(other.wealth, LegacyC4SVal::new(0, 0, 0, 250));
        assert_eq!(other.crew_count, LegacyC4SVal::new(1, 0, 1, 10));
        assert_eq!(other.position, [-1, -1]);
        assert!(other.ready_crew.is_empty());
        assert!(engine.player_start(4).is_none(), "only four start slots");
    }

    #[test]
    fn joining_team_uses_its_one_based_player_start_index() {
        // C4Player::ScenarioInit starts from Number % C4S_MaxPlayer, then a
        // nonzero C4Team::GetPlrStartIndex overrides it with index - 1
        // (pristine 9ffa0a5d src/C4Player.cpp:670-677).
        let mut engine = Engine::new();
        engine.set_landscape(Landscape::flat(256, 180));
        engine.set_map_zoom(LegacyC4SVal::new(1, 0, 1, 1));
        let mut starts = vec![PlayerStart::default(); MAX_PLAYER_STARTS];
        starts[0].position = [20, 30];
        starts[0].enforce_position = true;
        starts[1].position = [120, 130];
        starts[1].enforce_position = true;
        engine.set_player_starts(starts);
        engine.set_teams(vec![
            TeamInfo::new(7, "Indexed", 0).with_player_start_index(2)
        ]);

        let joined = engine
            .join_player(crate::JoinPlayerConfig {
                name: "Team player".to_string(),
                player_info_id: 1,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                team: Some(7),
                color_dw: 0,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("team player joins")
            .initialized()
            .expect("team player initializes");

        assert_eq!((joined.start_x, joined.start_y), (120, 130));
    }

