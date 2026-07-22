    // The GoldRush intro Talker blesses every living object with a pure    // The GoldRush intro Talker blesses every living object with a pure    // The GoldRush intro Talker blesses every living object with a pure
    // MARKER effect: `AddEffect("Divinity", o, 200, 1)` on FOREIGN
    // targets found via the FindObject find-next iteration
    // (Talker.c4d/Script.c:137-138). C4Effect::New creates the effect
    // even when no Fx* callbacks exist anywhere (C4Effect.cpp:64-118 —
    // no function lookup gates creation).
    #[test]
    fn foreign_add_effect_creates_marker_effect_like_cpp() {
        let talker_script = r#"#strict
func Bless() {
    var o;
    while (o = FindObject(0, 0,0,0,0, OCF_Alive, 0,0, 0, o))
        AddEffect("Divinity", o, 200, 1);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let talker =
            Definition::from_script("TALK", "Talker", talker_script).expect("talker compiles");
        engine
            .register_definition(talker)
            .expect("talker registers");
        let mut animal_def = simple_definition("ANML");
        // Alive targets are livings: OCF_Alive needs Category & C4D_Living
        // (SetOCF, C4Object.cpp:600-605).
        animal_def.set_category(CATEGORY_LIVING);
        engine
            .register_definition(animal_def)
            .expect("animal registers");

        let talker_id = engine
            .spawn_object(SpawnConfig::new("TALK").with_category(CATEGORY_OBJECT))
            .expect("talker spawns");
        let animal_a = engine
            .spawn_object(
                SpawnConfig::new("ANML")
                    .with_position(Vector2::new(40, 40))
                    .with_alive(true),
            )
            .expect("animal a spawns");
        let animal_b = engine
            .spawn_object(
                SpawnConfig::new("ANML")
                    .with_position(Vector2::new(90, 40))
                    .with_alive(true),
            )
            .expect("animal b spawns");

        let idx = engine.find_object_index(talker_id).expect("talker exists");
        engine
            .call_object_function(idx, "Bless", Vec::new())
            .expect("Bless runs");

        for id in [animal_a, animal_b] {
            let idx = engine.find_object_index(id).expect("animal exists");
            let effects = engine.objects[idx].state.effects.clone();
            assert!(
                effects.iter().any(|effect| {
                    effect.name == "Divinity" && effect.priority == 200 && effect.interval == 1
                }),
                "marker effect lands on foreign target {id:?}: {effects:?}"
            );
        }
    }

    // C4Game::NewObject runs PSF_Construction and (via the initial    // C4Game::NewObject runs PSF_Construction and (via the initial
    // DoCon's completion) PSF_Initialize INSIDE FnCreateObject
    // (C4Game.cpp:1117-1127): the new object's script side effects exist
    // the moment CreateObject returns. GoldRush: every clonk's appended
    // Initialize adds the "Life" effect (NeedFood Append.c4d) and SetAI
    // removes it right after creating the bandit (AI.c4d:18) — deferring
    // Initialize to materialization left the removal a no-op.
    #[test]
    fn create_object_runs_initialize_synchronously_like_cpp() {
        let bandit_script = r#"#strict
protected func Initialize() {
    AddEffect("Life", this(), 1, 35, this());
    return(1);
}
"#;
        let caller_script = r#"#strict
local iHadLife;
func Trigger() {
    var pObj = CreateObject(BNDT, 0, 0, -1);
    iHadLife = GetEffectCount("Life", pObj);
    RemoveEffect("Life", pObj);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let bandit =
            Definition::from_script("BNDT", "Bandit", bandit_script).expect("bandit compiles");
        engine
            .register_definition(bandit)
            .expect("bandit registers");
        let caller =
            Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");

        let id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");

        let idx = engine.find_object_index(id).expect("caller exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iHadLife"),
            Some(&Value::Int(1)),
            "Initialize ran inside CreateObject: Life visible immediately \
             (C4Game.cpp:1123-1127)"
        );
        let bandit = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "BNDT")
            .expect("bandit exists");
        assert!(
            bandit
                .state
                .effects
                .iter()
                .any(|effect| effect.name == "Life" && effect.priority == 0),
            "RemoveEffect right after CreateObject leaves Life linked dead"
        );
        engine.tick_without_snapshot().expect("next Execute cleans the dead Life node");
        let bandit = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "BNDT")
            .expect("bandit exists");
        assert!(
            bandit.state.effects.iter().all(|effect| effect.name != "Life"),
            "materialization must not re-run Initialize after cleanup"
        );
    }

    #[test]
    fn cast_pxs_matches_cpp_relative_position_rng_order_and_void_result() {
        // FnCastPXS offsets by the caller before C4PXSSystem::Cast; Cast
        // draws r2 (ydir) before r1 (xdir) for each particle and returns
        // void (C4Script.cpp:2470-2474; C4PXS.cpp:309-321).
        let script = r#"#strict
local cast_result;
func Burst() {
    cast_result = CastPXS("Water", 2, 20, 3, -4);
    return(Random(100));
}
"#;
        let library = MaterialLibrary::parse("[Material]\nName=Water\nDensity=50\n")
            .expect("water material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water material exists");
        let mut engine = Engine::with_seed(17);
        engine.set_materials(materials);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 200)),
            )
            .expect("caller spawns");

        let mut mirror = engine.rng.clone();
        let expected = (0..2)
            .map(|_| {
                let r2 = mirror.random(21);
                let r1 = mirror.random(21);
                (
                    C4Fixed::from_raw(math::itofix(r1 - 10).val() / 10),
                    C4Fixed::from_raw(math::itofix(r2 - 20).val() / 10),
                )
            })
            .collect::<Vec<_>>();
        let expected_return = mirror.random(100);

        let index = engine.find_object_index(caller).expect("caller exists");
        let result = engine
            .call_object_function(index, "Burst", Vec::new())
            .expect("cast runs");

        assert_eq!(result, Value::Int(expected_return));
        assert_eq!(engine.rng, mirror);
        assert_eq!(
            engine.object_snapshot(caller).expect("caller remains").local_vars["cast_result"],
            Value::Nil,
            "void FnCastPXS returns nil"
        );
        let particles = engine.pxs_system.iter().collect::<Vec<_>>();
        assert_eq!(particles.len(), 2);
        for (particle, (xdir, ydir)) in particles.into_iter().zip(expected) {
            assert_eq!(particle.mat, water);
            assert_eq!(particle.x, math::itofix(103));
            assert_eq!(particle.y, math::itofix(196));
            assert_eq!(particle.xdir, xdir);
            assert_eq!(particle.ydir, ydir);
        }
    }

    #[test]
    fn cast_pxs_failed_material_and_nonpositive_amount_match_cpp_draws() {
        // Material lookup happens before Cast, but invalid MNone attempts
        // still run Cast's r2/r1 loop; zero/negative amounts do not enter
        // it, and level zero uses Random(1) twice (C4Script.cpp:2470-2474;
        // C4PXS.cpp:207-216,309-321).
        let script = r#"#strict
func Burst() {
    CastPXS("Missing", 2, 0);
    CastPXS("Water", 0, 20);
    CastPXS("Water", -3, 20);
    CastPXS("Water", 1, 0);
    return(1);
}
"#;
        let library = MaterialLibrary::parse("[Material]\nName=Water\nDensity=50\n")
            .expect("water material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water material exists");
        let mut engine = Engine::with_seed(29);
        engine.set_materials(materials);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 5)),
            )
            .expect("caller spawns");

        let mut mirror = engine.rng.clone();
        for _ in 0..3 {
            let _ = mirror.random(1);
            let _ = mirror.random(1);
        }

        let index = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(index, "Burst", Vec::new())
            .expect("casts run");

        assert_eq!(engine.rng, mirror);
        let particles = engine.pxs_system.iter().collect::<Vec<_>>();
        assert_eq!(particles.len(), 1, "only the final valid cast creates PXS");
        assert_eq!(particles[0].mat, water);
        assert_eq!(particles[0].x, math::itofix(4));
        assert_eq!(particles[0].y, math::itofix(5));
        assert_eq!(particles[0].xdir, C4Fixed::ZERO);
        assert_eq!(particles[0].ydir, C4Fixed::ZERO);
    }

    #[test]
    fn presentation_snapshot_preserves_pxs_chunk_slot_identity() {
        // C4PXSSystem::Draw derives each graphical PXS phase and size from
        // `cnt2`, the live pixel's slot within its 500-entry chunk
        // (C4PXS.cpp:285-304). A presentation snapshot therefore cannot
        // compact live pixels the way `iter()` does.
        let library = MaterialLibrary::parse("[Material]\nName=Snow\nDensity=25\n")
            .expect("snow material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let snow = materials.id_of("Snow").expect("snow material exists");
        let mut engine = Engine::new();
        engine.set_materials(materials);
        assert!(engine.pxs_system.create_at(
            0,
            417,
            clonk_engine::pxs::Pxs {
                mat: snow,
                x: math::itofix(12),
                y: math::itofix(34),
                xdir: C4Fixed::ZERO,
                ydir: C4Fixed::ZERO,
            },
        ));

        let particle = engine
            .snapshot()
            .particles
            .into_iter()
            .find(|particle| particle.definition_id == "material/pxs/snow")
            .expect("PXS appears in presentation snapshot");
        assert_eq!(particle.pxs_slot, Some(417));
    }

    #[test]
    fn cast_objects_matches_cpp_rng_spawn_state_and_completion_order() {
        // FnCastObjects -> C4Game::CastObjects (C4Script.cpp:2476-2480,
        // C4Game.cpp:1727-1739): each object draws rdir, ydir, xdir, then
        // rotation before CreateObject runs its synchronous callbacks.
        let caller_script = r#"#strict
local result;
func Burst() {
    result = CastObjects(SPRK, 2, 7, 3, -4);
    return(1);
}
"#;
        let spark_script = r#"#strict
local iConstructed, iCompleted, pConstructedBy;
local iConstructionCon, iConstructionY, bConstructionAlive;
local iCompletionCon, iCompletionY, iCompletionR, iCompletionRDir;
local iInitialized;
protected func Construction(object pCreator) {
    iConstructed++;
    pConstructedBy = pCreator;
    iConstructionCon = GetCon();
    iConstructionY = GetY();
    bConstructionAlive = GetAlive();
    return(1);
}
protected func Completion() {
    var no_object;
    iCompletionCon = GetCon();
    iCompletionY = GetY();
    iCompletionR = GetR();
    iCompletionRDir = GetRDir(no_object, 100);
    iCompleted = Random(100);
    SetAction("Sparkle");
    return(1);
}
protected func Initialize() {
    iInitialized = 1;
    return(1);
}
"#;
        let mut engine = Engine::with_seed(17);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        let mut spark =
            Definition::from_script("SPRK", "Spark", spark_script).expect("spark compiles");
        spark.set_shape_rect(Some(DefinitionRect::new(-2, -2, 5, 5)));
        spark.set_stretch_growth(true);
        spark.configure_actions(
            None,
            HashMap::from([(
                "Sparkle".to_string(),
                ActionSpec::default().with_delay(1),
            )]),
        );
        engine.register_definition(spark).expect("spark registers");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 200))
                    .with_owner(2)
                    .with_controller(3),
            )
            .expect("caller spawns");

        let mut mirror = engine.rng.clone();
        let mut expected = Vec::new();
        for _ in 0..2 {
            let _sampled_rdir = math::itofix(mirror.random(3) + 1);
            let ydir = math::fixed10(mirror.random(15) - 7);
            let xdir = math::fixed10(mirror.random(15) - 7);
            let _sampled_rotation = mirror.random(360);
            let completion_random = mirror.random(100);
            expected.push((xdir, ydir, completion_random));
        }

        let index = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(index, "Burst", Vec::new())
            .expect("cast runs");

        assert_eq!(engine.rng, mirror, "callbacks interleave with cast draws");
        assert_eq!(
            engine.object_snapshot(caller).expect("caller remains").local_vars["result"],
            Value::Nil,
            "void FnCastObjects returns nil"
        );
        let mut sparks: Vec<&Object> = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "SPRK")
            .collect();
        sparks.sort_by_key(|object| object.id);
        assert_eq!(sparks.len(), 2);
        for (spark, (xdir, ydir, completion_random)) in sparks.into_iter().zip(expected) {
            assert_eq!(spark.state.position, Vector2::new(103, 193));
            assert_eq!(
                spark.fixed_position,
                FixedVec2::from_ints(103, 193),
                "Completion's SetAction resyncs fix_y to the adjusted integer y"
            );
            assert_eq!(spark.state.owner, 2);
            assert_eq!(spark.state.controller, 3);
            assert_eq!(spark.state.rotation, 0, "non-rotateable Init clears r");
            assert_eq!(spark.fixed_velocity, FixedVec2::new(xdir, ydir));
            assert_eq!(
                spark.rotation_velocity,
                C4Fixed::ZERO,
                "non-rotateable Init clears rdir"
            );
            assert_eq!(spark.state.action.name, "Sparkle");
            assert_eq!(
                spark.state.local_vars.get("iConstructed"),
                Some(&Value::Int(1)),
                "Construction runs once, synchronously"
            );
            assert_eq!(
                spark.state.local_vars.get("pConstructedBy"),
                Some(&Value::Object(caller.as_u64())),
                "Construction receives the creator"
            );
            assert_eq!(
                spark.state.local_vars.get("iConstructionCon"),
                Some(&Value::Int(0)),
                "Construction runs before initial DoCon"
            );
            assert_eq!(
                spark.state.local_vars.get("iConstructionY"),
                Some(&Value::Int(196)),
                "Construction sees the raw Init center"
            );
            assert_eq!(
                spark.state.local_vars.get("bConstructionAlive"),
                Some(&Value::Bool(false)),
                "Init only marks living definitions alive"
            );
            assert_eq!(
                spark.state.local_vars.get("iCompletionCon"),
                Some(&Value::Int(100)),
                "Completion follows initial DoCon"
            );
            assert_eq!(
                spark.state.local_vars.get("iCompletionY"),
                Some(&Value::Int(193)),
                "Completion sees the bottom-adjusted center"
            );
            assert_eq!(
                spark.state.local_vars.get("iCompletionR"),
                Some(&Value::Int(0)),
                "Completion observes the post-Init rotation"
            );
            assert_eq!(
                spark.state.local_vars.get("iCompletionRDir"),
                Some(&Value::Int(0)),
                "Completion observes the post-Init rdir"
            );
            assert_eq!(
                spark.state.local_vars.get("iCompleted"),
                Some(&Value::Int(completion_random))
            );
            assert_eq!(
                spark.state.local_vars.get("iInitialized"),
                Some(&Value::Int(1)),
                "Initialize follows Completion exactly once"
            );
        }
    }

    #[test]
    fn cast_objects_missing_definition_consumes_draws_but_no_object_number() {
        // C4Game::CastObjects draws before CreateObject performs C4Id2Def
        // (C4Game.cpp:1727-1739; C4Game.cpp:1142-1148). Zero amount is an
        // empty loop; failed attempts consume RNG but no enumeration id.
        let script = r#"#strict
func Burst() {
    CastObjects(MISS, 0, 5);
    CastObjects(MISS, 2, 5);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(29);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let next_object_id = engine.next_object_id;
        let mut mirror = engine.rng.clone();
        for _ in 0..2 {
            let _ = mirror.random(3);
            let _ = mirror.random(11);
            let _ = mirror.random(11);
            let _ = mirror.random(360);
        }

        let index = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(index, "Burst", Vec::new())
            .expect("failed casts remain silent");

        assert_eq!(engine.rng, mirror);
        assert_eq!(engine.next_object_id, next_object_id);
        assert_eq!(engine.objects.len(), 1);
    }

    #[test]
    fn cast_objects_rotateable_definition_uses_sampled_rotation_and_rdir() {
        // C4Game::CastObjects passes all four sampled values into CreateObject;
        // C4Object::Init preserves r/rdir when Rotateable is set
        // (C4Game.cpp:1727-1739; C4Object.cpp:169-174).
        let script = r#"#strict
func Burst() {
    CastObjects(ROTA, 1, 4, 0, 0);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(41);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let mut cast = Definition::from_script("ROTA", "Rotating", "").expect("cast compiles");
        cast.set_rotateable(1);
        engine.register_definition(cast).expect("cast registers");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 20)),
            )
            .expect("caller spawns");
        let mut mirror = engine.rng.clone();
        let rdir = math::itofix(mirror.random(3) + 1);
        let ydir = math::fixed10(mirror.random(9) - 4);
        let xdir = math::fixed10(mirror.random(9) - 4);
        let rotation = mirror.random(360);

        let index = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(index, "Burst", Vec::new())
            .expect("cast runs");

        let object = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "ROTA")
            .expect("cast object exists");
        assert_eq!(engine.rng, mirror);
        assert_eq!(object.state.position, Vector2::new(10, 20));
        assert_eq!(object.fixed_velocity, FixedVec2::new(xdir, ydir));
        assert_eq!(object.state.rotation, rotation);
        assert_eq!(object.rotation_velocity, rdir);
    }

    #[test]
    fn cast_objects_initial_docon_preserves_fixed_position_without_resync() {
        // Initial DoCon changes integer y after bottom growth but not fix_y
        // (C4Object.cpp:1489-1495). No callback here performs a later
        // SetAction/ForcePosition resync.
        let script = r#"#strict
func Burst() {
    CastObjects(GROW, 1, 0, 3, -4);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(43);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let mut grow = Definition::from_script("GROW", "Growing", "").expect("grow compiles");
        grow.set_shape_rect(Some(DefinitionRect::new(-2, -2, 5, 5)));
        grow.set_stretch_growth(true);
        engine.register_definition(grow).expect("grow registers");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 200)),
            )
            .expect("caller spawns");

        let index = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(index, "Burst", Vec::new())
            .expect("cast runs");

        let object = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "GROW")
            .expect("cast object exists");
        assert_eq!(object.state.position, Vector2::new(103, 193));
        assert_eq!(object.fixed_position, FixedVec2::from_ints(103, 196));
    }

    #[test]
    fn cast_objects_completion_removal_suppresses_initialize() {
        // Completion may remove the new object; the following Initialize
        // dispatch then fails the C4Object::Call Status guard and consumes no
        // RNG (C4Object.cpp:1506-1511, 2224-2227).
        let caller_script = r#"#strict
func Burst() {
    CastObjects(GONE, 1, 0, 0, 0);
    return(1);
}
"#;
        let removed_script = r#"#strict
protected func Completion() {
    RemoveObject();
    return(1);
}
protected func Initialize() {
    Random(100);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(47);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("GONE", "Removed", removed_script)
                    .expect("removed compiles"),
            )
            .expect("removed registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let next_object_id = engine.next_object_id;
        let mut mirror = engine.rng.clone();
        let _ = mirror.random(3);
        let _ = mirror.random(1);
        let _ = mirror.random(1);
        let _ = mirror.random(360);

        let index = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(index, "Burst", Vec::new())
            .expect("cast runs");

        assert_eq!(engine.rng, mirror, "Initialize did not draw");
        assert_eq!(engine.next_object_id, next_object_id + 1);
        assert_eq!(engine.objects.len(), 1, "removed spawn never materializes");
    }

    // FnGetController (C4Script.cpp:1316-1320) reads C4Object::Controller,
    // which C4Object::Init seeds from the owner when no explicit
    // controller is handed in (C4Object.cpp:162).
    #[test]
    fn get_controller_defaults_to_owner_like_init() {
        let script = r#"#strict
local iCtrl;
func Trigger() {
    iCtrl = GetController();
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let def = Definition::from_script("CALL", "Caller", script).expect("caller compiles");
        engine.register_definition(def).expect("caller registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(2),
            )
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");

        let idx = engine.find_object_index(id).expect("caller exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iCtrl"),
            Some(&Value::Int(2)),
            "Controller = Owner at Init (C4Object.cpp:162)"
        );
    }

    // FnCreateObject copies the creating object's controller onto the
    // spawn so cause-effect chains trace back to the causing player
    // (C4Script.cpp:1905-1906) - even when the new object is ownerless.
    #[test]
    fn create_object_inherits_creator_controller() {
        let bandit_script = "#strict\n";
        let caller_script = r#"#strict
local iCtrl;
func Trigger() {
    var pObj = CreateObject(BNDT, 0, 0, -1);
    iCtrl = GetController(pObj);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let bandit =
            Definition::from_script("BNDT", "Bandit", bandit_script).expect("bandit compiles");
        engine
            .register_definition(bandit)
            .expect("bandit registers");
        let caller =
            Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(2),
            )
            .expect("caller spawns");
        let idx = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");

        let idx = engine.find_object_index(id).expect("caller exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iCtrl"),
            Some(&Value::Int(2)),
            "spawn controller = creator controller (C4Script.cpp:1905-1906)"
        );
        let bandit = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "BNDT")
            .expect("bandit exists");
        assert_eq!(bandit.state.owner, OWNER_NONE, "owner stays NO_OWNER");
        assert_eq!(bandit.state.controller, 2, "controller traces the cause");
    }

    // FnSetController (C4Script.cpp:1322-1331): NO_OWNER always passes,
    // any other value must be a valid player, and foreign targets are
    // written directly (the BlastObjects shockwave marks flung
    // projectiles with the causing player, Explode.c:116).
    #[test]
    fn set_controller_validates_player_and_writes_foreign_targets() {
        let caller_script = r#"#strict
local iSelf, iInvalid, iForeign, iCleared;
func Trigger(object pOther) {
    iInvalid = SetController(9);
    iSelf = SetController(1);
    iForeign = SetController(1, pOther);
    iCleared = SetController(-1);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(1, "P1"))
            .expect("player registers");
        let caller =
            Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");
        let other = Definition::from_script("OTHR", "Other", "#strict\n").expect("other compiles");
        engine.register_definition(other).expect("other registers");

        let caller_id = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(0),
            )
            .expect("caller spawns");
        let other_id = engine
            .spawn_object(
                SpawnConfig::new("OTHR")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(0),
            )
            .expect("other spawns");
        let idx = engine.find_object_index(caller_id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", vec![Value::Object(other_id.as_u64())])
            .expect("trigger runs");

        let idx = engine.find_object_index(caller_id).expect("caller exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(
            locals.get("iInvalid"),
            Some(&Value::Bool(false)),
            "invalid player rejected (ValidPlr gate)"
        );
        assert_eq!(locals.get("iSelf"), Some(&Value::Bool(true)));
        assert_eq!(locals.get("iForeign"), Some(&Value::Bool(true)));
        assert_eq!(
            locals.get("iCleared"),
            Some(&Value::Bool(true)),
            "NO_OWNER bypasses the player check"
        );
        assert_eq!(
            engine.objects[idx].state.controller, OWNER_NONE,
            "self ends cleared"
        );
        let other_idx = engine.find_object_index(other_id).expect("other exists");
        assert_eq!(
            engine.objects[other_idx].state.controller, 1,
            "foreign target updated"
        );
    }

    // FnGetDefCoreVal (C4Script.cpp:4177) must resolve the blast-chain
    // entries that BlastObjectsShockwaveCheck and DoExplosion read through
    // the System.c4g GetXVal wrappers (GetDefGrab/GetDefHorizontalFix/
    // GetDefContainBlast, GetXVal.c): Grab, HorizontalFix, ContainBlast,
    // and the fire thresholds.
    #[test]
    fn get_def_core_val_resolves_the_blast_chain_entries() {
        let caller_script = r#"#strict
local iGrab, iFix, iShield, iBlast, iContact;
func Probe() {
    iGrab = GetDefCoreVal("Grab", "DefCore", HUTX);
    iFix = GetDefCoreVal("HorizontalFix", "DefCore", HUTX);
    iShield = GetDefCoreVal("ContainBlast", "DefCore", HUTX);
    iBlast = GetDefCoreVal("BlastIncinerate", "DefCore", HUTX);
    iContact = GetDefCoreVal("ContactIncinerate", "DefCore", HUTX);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        let mut hut = simple_definition("HUTX");
        hut.set_grab(2);
        hut.set_no_horizontal_move(1);
        hut.set_contain_blast(1);
        hut.set_blast_incinerate(50);
        hut.set_fire_properties(10, false, false);
        engine.register_definition(hut).expect("hut registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(caller_idx, "Probe", Vec::new())
            .expect("probe runs");
        let locals = &engine.objects[caller_idx].state.local_vars;
        assert_eq!(locals.get("iGrab"), Some(&Value::Int(2)));
        assert_eq!(locals.get("iFix"), Some(&Value::Int(1)));
        assert_eq!(locals.get("iShield"), Some(&Value::Int(1)));
        assert_eq!(locals.get("iBlast"), Some(&Value::Int(50)));
        assert_eq!(locals.get("iContact"), Some(&Value::Int(10)));
    }

    #[test]
    fn get_def_core_val_reflects_fire_top_from_def_core() -> Result<(), EngineError> {
        // FnGetDefCoreVal reflects the named C4Def compiler entry
        // (C4Script.cpp:4171-4182); C4Shape::CompileFunc exposes FireTop in
        // DefCore with default zero (C4Shape.cpp:496-510).
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Wampfruit.c4d");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=WMPF\nName=Wampfruit\nCategory=C4D_Object\nFireTop=10\n",
        )
        .expect("write def core");
        std::fs::write(
            def_dir.join("Script.c"),
            br#"#strict
local iFireTop;
func Probe() {
    iFireTop = GetDefCoreVal("FireTop", "DefCore", WMPF);
    return(1);
}
"#,
        )
        .expect("write script");

        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load definition");
        let definition = Definition::from_resource(&resource)?;
        let mut engine = Engine::with_seed(0);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(SpawnConfig::new("WMPF"))?;
        let index = engine.find_object_index(id).expect("object exists");
        engine.call_object_function(index, "Probe", Vec::new())?;

        assert_eq!(
            engine.objects[index].state.local_vars.get("iFireTop"),
            Some(&Value::Int(10))
        );
        Ok(())
    }

    // C4SortObject::CompareGetValue returns int32_t (C4FindObject.h:430):
    // C4SortObjectDistance computes dx*dx+dy*dy in i32 and WRAPS on big
    // coordinates (C4FindObject.cpp:908-911); C4SortObjectSpeed's C4Fixed
    // sum reaches int32_t through the IMPLICIT `operator bool`
    // (Fixed.h:117 — the only conversion C4Fixed offers), so the key is
    // 0/1 "moving at all"; C4SortObjectMass reads the LIVE UpdateMass
    // field ((Def->Mass+OwnMass)*Con/FullCon max 1, C4Object.cpp:497-500).
    #[test]
    fn sort_object_keys_use_the_cpp_i32_semantics() {
        let caller_script = r#"#strict
local iFarFirst, iFastFirst, iHalfMass;
func Probe() {
    var aDist = FindObjects([C4FO_Category, 16], [C4SO_Distance, 0, 0]);
    iFarFirst = GetX(aDist[0]);
    var aSpeed = FindObjects([C4FO_Category, 16], [C4SO_Speed]);
    iFastFirst = GetX(aSpeed[0]);
    var aMass = FindObjects([C4FO_Category, 16], [C4SO_Mass]);
    iHalfMass = GetX(aMass[0]);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        let mut heavy = simple_definition("HEVY");
        heavy.set_mass(100);
        engine.register_definition(heavy).expect("heavy registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(9000, 0)))
            .expect("caller spawns");

        // Distance wrap: dx=50000 → 2.5e9 wraps negative in i32 and sorts
        // FIRST; dx=1000 stays 1e6.
        let far = engine
            .spawn_object(
                SpawnConfig::new("HEVY")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50_000, 0)),
            )
            .expect("far spawns");
        engine
            .spawn_object(
                SpawnConfig::new("HEVY")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(1_000, 0)),
            )
            .expect("near spawns");
        // Speed keys are 0/1: both movers key 1, so the STABLE sort keeps
        // the faster-but-first object ahead.
        let fast_idx = engine.find_object_index(far).expect("far exists");
        engine.objects[fast_idx].fixed_velocity = FixedVec2::from_ints(5, 0);
        let near_idx = engine
            .find_object_index(engine.objects[fast_idx + 1].id)
            .expect("near exists");
        engine.objects[near_idx].fixed_velocity = FixedVec2::from_ints(1, 0);
        // Mass: HALF-Con object keys 50 vs 100 and sorts first even though
        // it spawned second.
        engine.objects[near_idx].state.construction = FULL_CON / 2;

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(caller_idx, "Probe", Vec::new())
            .expect("probe runs");
        let locals = &engine.objects[caller_idx].state.local_vars;
        assert_eq!(
            locals.get("iFarFirst"),
            Some(&Value::Int(50_000)),
            "the wrapped-negative distance key sorts first (i32 overflow)"
        );
        assert_eq!(
            locals.get("iFastFirst"),
            Some(&Value::Int(50_000)),
            "0/1 speed keys tie; stable sort keeps collection order"
        );
        assert_eq!(
            locals.get("iHalfMass"),
            Some(&Value::Int(1_000)),
            "live con-scaled mass sorts the half-built object first"
        );
    }

    // C4FindObjectLayer::Check is `pObj->pLayer == pLayer`
    // (C4FindObject.cpp:671-674): Find_Layer(nil) matches every UNLAYERED
    // object and Find_Layer(pLayer) exactly the layer's members —
    // System.c4g BlastObjects (Explode.c:93-97) relies on the nil form to
    // find explosion victims.
    #[test]
    fn find_layer_criterion_compares_the_object_layer_like_cpp() {
        let caller_script = r#"#strict
local iBare, iLayered;
func Probe(pLayer) {
    iBare = GetLength(FindObjects([C4FO_Layer]));
    iLayered = GetLength(FindObjects([C4FO_Layer, pLayer]));
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(simple_definition("OTHR"))
            .expect("other registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let layer = engine
            .spawn_object(SpawnConfig::new("OTHR").with_category(CATEGORY_OBJECT))
            .expect("layer spawns");
        engine
            .spawn_object(
                SpawnConfig::new("OTHR")
                    .with_category(CATEGORY_OBJECT)
                    .with_layer(layer),
            )
            .expect("layered spawns");

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        engine
            .call_object_function(caller_idx, "Probe", vec![Value::Object(layer.as_u64())])
            .expect("probe runs");
        let locals = &engine.objects[caller_idx].state.local_vars;
        assert_eq!(
            locals.get("iBare"),
            Some(&Value::Int(2)),
            "nil layer matches the two unlayered objects"
        );
        assert_eq!(
            locals.get("iLayered"),
            Some(&Value::Int(1)),
            "the layer's one member matches"
        );
    }

    // FnGetObjectLayer (C4Script.cpp:5160-5166): the object's pLayer —
    // nil for the (default) unlayered world, the layer object when set.
    // System.c4g Explode.c reads it before removing the exploding object.
    #[test]
    fn get_object_layer_returns_nil_or_the_layer_object() {
        let caller_script = r#"#strict
local aBare, aLayered;
func Trigger(object pLayered) {
    aBare = GetObjectLayer();
    aLayered = GetObjectLayer(pLayered);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let caller =
            Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");
        let other = Definition::from_script("OTHR", "Other", "#strict\n").expect("other compiles");
        engine.register_definition(other).expect("other registers");

        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let layered_id = engine
            .spawn_object(
                SpawnConfig::new("OTHR")
                    .with_category(CATEGORY_OBJECT)
                    .with_layer(caller_id),
            )
            .expect("layered spawns");
        let idx = engine.find_object_index(caller_id).expect("caller exists");
        engine
            .call_object_function(idx, "Trigger", vec![Value::Object(layered_id.as_u64())])
            .expect("trigger runs");

        let idx = engine.find_object_index(caller_id).expect("caller exists");
        let locals = &engine.objects[idx].state.local_vars;
        assert_eq!(locals.get("aBare"), Some(&Value::Nil), "no layer -> nil");
        assert_eq!(
            locals.get("aLayered"),
            Some(&Value::Object(caller_id.as_u64())),
            "layer object returned"
        );
    }

    #[test]
    fn removing_layer_clears_members_before_same_frame_cross_check() -> Result<(), EngineError> {
        // AssignRemoval calls Game.ClearPointers before RemoveObject returns.
        // Former layer members therefore expose nil immediately and join the
        // nil-layer CrossCheck group later in this same frame.
        let remover_script = r#"#strict
local doomed, member, cleared_immediately, nil_layer_count;
func Cull()
{
    RemoveObject(doomed);
    cleared_immediately = !GetObjectLayer(member);
    nil_layer_count = GetLength(FindObjects([C4FO_Layer]));
    return(1);
}
"#;
        let mut remover = Definition::from_script("RMVR", "Remover", remover_script)?;
        remover.set_timer(3);
        remover.set_timer_call(Some("Cull".to_string()));
        let mut collector = simple_definition("COLL");
        collector.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
        collector.set_collection_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
        let mut item = simple_definition("ITEM");
        item.set_category(CATEGORY_OBJECT);
        item.set_collectible(true);

        let mut engine = Engine::with_seed(0);
        engine.register_definition(simple_definition("LAYR"))?;
        engine.register_definition(remover)?;
        engine.register_definition(collector)?;
        engine.register_definition(item)?;

        let layer = engine.spawn_object(
            SpawnConfig::new("LAYR").with_position(Vector2::new(5, 5)),
        )?;
        let collector = engine.spawn_object(
            SpawnConfig::new("COLL")
                .with_category(CATEGORY_LIVING)
                .with_alive(true)
                .with_position(Vector2::new(50, 60))
                .with_layer(layer),
        )?;
        let item = engine.spawn_object(
            SpawnConfig::new("ITEM").with_position(Vector2::new(50, 50)),
        )?;
        let remover = engine.spawn_object(
            SpawnConfig::new("RMVR")
                .with_position(Vector2::new(5, 5))
                .with_local_vars(HashMap::from([
                    ("doomed".to_string(), Value::Object(layer.as_u64())),
                    ("member".to_string(), Value::Object(collector.as_u64())),
                ])),
        )?;

        engine.cross_check(3)?;
        assert_eq!(
            engine.object_snapshot(item).expect("item remains").container,
            None,
            "the layer mismatch blocks a collection-eligible CrossCheck"
        );
        for _ in 0..2 {
            engine.tick_without_snapshot()?;
            assert_eq!(
                engine.object_snapshot(item).expect("item remains").container,
                None,
                "different layers cannot collect before removal"
            );
        }
        assert_eq!(
            engine
                .object_snapshot(collector)
                .expect("collector remains")
                .layer,
            Some(layer)
        );

        engine.tick_without_snapshot()?;

        assert!(engine.object_snapshot(layer).is_none(), "layer was removed");
        let remover_index = engine.find_object_index(remover).expect("remover remains");
        assert_eq!(
            engine.objects[remover_index]
                .state
                .local_vars
                .get("cleared_immediately"),
            Some(&Value::Bool(true)),
            "GetObjectLayer is nil before RemoveObject returns to the script"
        );
        assert_eq!(
            engine.objects[remover_index]
                .state
                .local_vars
                .get("nil_layer_count"),
            Some(&Value::Int(3)),
            "same-call Find_Layer(nil) includes the former layer member"
        );
        assert_eq!(
            engine
                .object_snapshot(collector)
                .expect("collector remains")
                .layer,
            None,
            "the live object snapshot cannot retain the dead layer id"
        );
        assert_eq!(
            engine
                .snapshot()
                .object(collector)
                .expect("collector is serialized")
                .layer,
            None,
            "the simulation snapshot serializes the cleared layer without a reload"
        );
        assert_eq!(
            engine.object_snapshot(item).expect("item remains").container,
            Some(collector),
            "the frame-3 CrossCheck pairs both now-unlayered objects"
        );
        Ok(())
    }

    #[test]
    fn queued_layer_destruction_clears_members_before_cross_check() -> Result<(), EngineError> {
        // Engine-owned removals do not enter the compat RemoveObject host.
        // Their common pre-CrossCheck fallback must perform the same
        // C4Game::ClearPointers pLayer sweep in the destruction frame.
        let mut collector = simple_definition("COLL");
        collector.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
        collector.set_collection_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
        let mut item = simple_definition("ITEM");
        item.set_category(CATEGORY_OBJECT);
        item.set_collectible(true);
        let observer_script = r#"#strict
local member, cleared_before_callback;
func Probe() { cleared_before_callback = !GetObjectLayer(member); }
"#;
        let mut observer = Definition::from_script("OBSV", "Observer", observer_script)?;
        observer.set_category(CATEGORY_OBJECT);
        observer.set_timer(3);
        observer.set_timer_call(Some("Probe".to_string()));

        let mut engine = Engine::with_seed(0);
        let mut layer_definition = simple_definition("LAYR");
        layer_definition.set_category(CATEGORY_OBJECT);
        engine.register_definition(layer_definition)?;
        engine.register_definition(collector)?;
        engine.register_definition(item)?;
        engine.register_definition(observer)?;

        let layer = engine.spawn_object(
            SpawnConfig::new("LAYR").with_position(Vector2::new(5, 5)),
        )?;
        let collector = engine.spawn_object(
            SpawnConfig::new("COLL")
                .with_category(CATEGORY_LIVING)
                .with_alive(true)
                .with_position(Vector2::new(50, 60))
                .with_layer(layer),
        )?;
        let item = engine.spawn_object(
            SpawnConfig::new("ITEM").with_position(Vector2::new(50, 50)),
        )?;
        let observer = engine.spawn_object(
            SpawnConfig::new("OBSV")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(5, 5))
                .with_local_vars(HashMap::from([(
                    "member".to_string(),
                    Value::Object(collector.as_u64()),
                )])),
        )?;
        let order = engine.debug_exec_order();
        let layer_position = order
            .iter()
            .position(|id| *id == layer)
            .expect("layer is executable");
        let observer_position = order
            .iter()
            .position(|id| *id == observer)
            .expect("observer is executable");
        assert!(
            layer_position < observer_position,
            "the native removal executes before the observer callback"
        );

        for _ in 0..2 {
            engine.tick_without_snapshot()?;
            assert_eq!(
                engine.object_snapshot(item).expect("item remains").container,
                None
            );
        }
        engine.queue_object_command(
            layer,
            QueuedCommand::immediate(ObjectUpdate::new()).with_destroy(true),
        )?;

        engine.tick_without_snapshot()?;

        let observer_index = engine.find_object_index(observer).expect("observer remains");
        assert_eq!(
            engine.objects[observer_index]
                .state
                .local_vars
                .get("cleared_before_callback"),
            Some(&Value::Bool(true)),
            "native ClearPointers is visible to later object callbacks, not just CrossCheck"
        );
        assert_eq!(
            engine
                .object_snapshot(collector)
                .expect("collector remains")
                .layer,
            None
        );
        assert_eq!(
            engine.object_snapshot(item).expect("item remains").container,
            Some(collector),
            "native destruction clears the layer before frame-3 CrossCheck"
        );
        Ok(())
    }

    #[test]
    fn set_object_layer_self_foreign_and_clear_are_live_and_persisted() {
        // FnSetObjectLayer writes pLayer immediately, defaults its target to
        // the caller and accepts nil/0 to clear (C4Script.cpp:5168-5180).
        // Dragon Rock self-layers the endboss and princess via arrow calls.
        let caller_script = r#"#strict
local bSelf, pSelfNow, bForeign, pForeignNow, bClear, pCleared;
func Trigger(object pOther) {
    bSelf = SetObjectLayer(this());
    pSelfNow = GetObjectLayer();
    bForeign = SetObjectLayer(this(), pOther);
    pForeignNow = GetObjectLayer(pOther);
    bClear = SetObjectLayer(0, pOther);
    pCleared = GetObjectLayer(pOther);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("OTHR", "Other", "#strict\n").expect("other compiles"),
            )
            .expect("other registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let other_id = engine
            .spawn_object(SpawnConfig::new("OTHR").with_category(CATEGORY_OBJECT))
            .expect("other spawns");
        let caller_index = engine.find_object_index(caller_id).expect("caller exists");

        engine
            .call_object_function(
                caller_index,
                "Trigger",
                vec![Value::Object(other_id.as_u64())],
            )
            .expect("layer trigger runs");

        let caller = engine.object_snapshot(caller_id).expect("caller remains");
        assert_eq!(caller.local_vars.get("bSelf"), Some(&Value::Bool(true)));
        assert_eq!(
            caller.local_vars.get("pSelfNow"),
            Some(&Value::Object(caller_id.as_u64()))
        );
        assert_eq!(caller.local_vars.get("bForeign"), Some(&Value::Bool(true)));
        assert_eq!(
            caller.local_vars.get("pForeignNow"),
            Some(&Value::Object(caller_id.as_u64()))
        );
        assert_eq!(caller.local_vars.get("bClear"), Some(&Value::Bool(true)));
        assert_eq!(caller.local_vars.get("pCleared"), Some(&Value::Nil));
        assert_eq!(caller.layer, Some(caller_id));
        assert_eq!(
            engine
                .object_snapshot(other_id)
                .expect("other remains")
                .layer,
            None
        );
    }

    #[test]
    fn set_object_layer_propagates_to_direct_present_contents_only() {
        // FnSetObjectLayer walks exactly the target's direct Contents and
        // accepts both Status 1 and 2; it does not recurse into grandchildren
        // (C4Script.cpp:5174-5178).
        let caller_script = r#"#strict
local bSet, pDirect, pInactive, pGrandchild;
func Trigger(object direct, object inactive, object grandchild) {
    bSet = SetObjectLayer(this());
    pDirect = GetObjectLayer(direct);
    pInactive = GetObjectLayer(inactive);
    pGrandchild = GetObjectLayer(grandchild);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("ITEM", "Item", "#strict\n").expect("item compiles"),
            )
            .expect("item registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let direct_id = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(caller_id),
            )
            .expect("direct content spawns");
        let inactive_id = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_status(ObjectStatus::Inactive)
                    .with_container(caller_id),
            )
            .expect("inactive content spawns");
        let grandchild_id = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(direct_id),
            )
            .expect("grandchild spawns");
        let caller_index = engine.find_object_index(caller_id).expect("caller exists");

        engine
            .call_object_function(
                caller_index,
                "Trigger",
                vec![
                    Value::Object(direct_id.as_u64()),
                    Value::Object(inactive_id.as_u64()),
                    Value::Object(grandchild_id.as_u64()),
                ],
            )
            .expect("layer trigger runs");

        let caller = engine.object_snapshot(caller_id).expect("caller remains");
        assert_eq!(caller.local_vars.get("bSet"), Some(&Value::Bool(true)));
        assert_eq!(
            caller.local_vars.get("pDirect"),
            Some(&Value::Object(caller_id.as_u64()))
        );
        assert_eq!(
            caller.local_vars.get("pInactive"),
            Some(&Value::Object(caller_id.as_u64()))
        );
        assert_eq!(caller.local_vars.get("pGrandchild"), Some(&Value::Nil));
        assert_eq!(
            engine
                .object_snapshot(direct_id)
                .expect("direct content remains")
                .layer,
            Some(caller_id)
        );
        assert_eq!(
            engine
                .object_snapshot(inactive_id)
                .expect("inactive content remains")
                .layer,
            Some(caller_id)
        );
        assert_eq!(
            engine
                .object_snapshot(grandchild_id)
                .expect("grandchild remains")
                .layer,
            None
        );
    }

    #[test]
    fn create_paths_inherit_the_creator_layer_immediately() {
        // C4Object::Init copies pCreator->pLayer (C4Object.cpp:153-170).
        // CreateObject uses the caller as creator; CreateContents uses its
        // container (C4Script.cpp:1886-1902, C4Object.cpp:1866-1871).
        let caller_script = r#"#strict
local pObject, pConstruction, pContents;
local pObjectLayer, pConstructionLayer, pContentsLayer;
local iObjectLayerCache, iConstructionLayerCache, iContentsLayerCache;
local iObjectLayerCacheAfter, iConstructionLayerCacheAfter, iContentsLayerCacheAfter;
func Trigger(object pContainer) {
    SetObjectLayer(this());
    SetObjectLayer(pContainer, pContainer);
    pObject = CreateObject(ITEM, 0, 0, -1);
    pObjectLayer = GetObjectLayer(pObject);
    iObjectLayerCache = GetObjectVal("Layer", "Object", pObject, 0);
    pConstruction = CreateConstruction(ITEM, 0, 0, -1, 100);
    pConstructionLayer = GetObjectLayer(pConstruction);
    iConstructionLayerCache = GetObjectVal("Layer", "Object", pConstruction, 0);
    pContents = CreateContents(ITEM, pContainer);
    pContentsLayer = GetObjectLayer(pContents);
    iContentsLayerCache = GetObjectVal("Layer", "Object", pContents, 0);
    return(1);
}
func ProbeMaterializedCaches() {
    iObjectLayerCacheAfter = GetObjectVal("Layer", "Object", pObject, 0);
    iConstructionLayerCacheAfter = GetObjectVal("Layer", "Object", pConstruction, 0);
    iContentsLayerCacheAfter = GetObjectVal("Layer", "Object", pContents, 0);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("ITEM", "Item", "#strict\n").expect("item compiles"),
            )
            .expect("item registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let container_id = engine
            .spawn_object(SpawnConfig::new("ITEM").with_category(CATEGORY_OBJECT))
            .expect("foreign container spawns");
        engine
            .apply_object_update(
                caller_id,
                ObjectUpdate {
                    compiler_cache: Some(ObjectCompilerCache {
                        layer: 321,
                        ..ObjectCompilerCache::default()
                    }),
                    ..ObjectUpdate::default()
                },
            )
            .expect("caller raw layer cache seeds");
        engine
            .apply_object_update(
                container_id,
                ObjectUpdate {
                    compiler_cache: Some(ObjectCompilerCache {
                        layer: 654,
                        ..ObjectCompilerCache::default()
                    }),
                    ..ObjectUpdate::default()
                },
            )
            .expect("container raw layer cache seeds");
        let caller_index = engine.find_object_index(caller_id).expect("caller exists");

        engine
            .call_object_function(
                caller_index,
                "Trigger",
                vec![Value::Object(container_id.as_u64())],
            )
            .expect("creation trigger runs");
        engine
            .call_object_function(caller_index, "ProbeMaterializedCaches", Vec::new())
            .expect("materialized cache probe runs");

        let caller = engine.object_snapshot(caller_id).expect("caller remains");
        let object_id = match caller.local_vars.get("pObject") {
            Some(Value::Object(id)) => ObjectId::new(*id),
            other => panic!("CreateObject result should be an object, got {other:?}"),
        };
        let contents_id = match caller.local_vars.get("pContents") {
            Some(Value::Object(id)) => ObjectId::new(*id),
            other => panic!("CreateContents result should be an object, got {other:?}"),
        };
        let construction_id = match caller.local_vars.get("pConstruction") {
            Some(Value::Object(id)) => ObjectId::new(*id),
            other => panic!("CreateConstruction result should be an object, got {other:?}"),
        };
        assert_eq!(
            caller.local_vars.get("pObjectLayer"),
            Some(&Value::Object(caller_id.as_u64())),
            "CreateObject exposes the inherited layer in the same call"
        );
        assert_eq!(
            caller.local_vars.get("pConstructionLayer"),
            Some(&Value::Object(caller_id.as_u64())),
            "CreateConstruction exposes the inherited layer in the same call"
        );
        assert_eq!(
            caller.local_vars.get("pContentsLayer"),
            Some(&Value::Object(container_id.as_u64())),
            "CreateContents uses the selected container as creator"
        );
        for name in [
            "iObjectLayerCache",
            "iConstructionLayerCache",
            "iObjectLayerCacheAfter",
            "iConstructionLayerCacheAfter",
        ] {
            assert_eq!(
                caller.local_vars.get(name),
                Some(&Value::Int(321)),
                "{name} copies the caller's raw layer cache",
            );
        }
        for name in ["iContentsLayerCache", "iContentsLayerCacheAfter"] {
            assert_eq!(
                caller.local_vars.get(name),
                Some(&Value::Int(654)),
                "{name} copies the container's raw layer cache",
            );
        }
        assert_eq!(
            engine
                .object_snapshot(object_id)
                .expect("created object remains")
                .layer,
            Some(caller_id)
        );
        assert_eq!(
            engine
                .object_snapshot(construction_id)
                .expect("construction remains")
                .layer,
            Some(caller_id)
        );
        assert_eq!(
            engine
                .object_snapshot(contents_id)
                .expect("created content remains")
                .layer,
            Some(container_id)
        );
        assert_eq!(
            engine
                .object_snapshot(contents_id)
                .expect("created content remains")
                .container,
            Some(container_id)
        );
    }

    #[test]
    fn scenario_set_object_layer_sees_and_updates_snapshot_contents() {
        // Scenario callbacks run against host_world_context_from_snapshot.
        // Its object list must carry Contents so FnSetObjectLayer can perform
        // the same direct-only propagation as an object-script callback.
        let scenario_script = r#"#strict
global func ApplyLayer(object pLayer, object pTarget) {
    return(SetObjectLayer(pLayer, pTarget));
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("ITEM", "Item", "#strict\n").expect("item compiles"),
            )
            .expect("item registers");
        engine
            .install_scenario_script_with_convention("Scenario", scenario_script, true)
            .expect("scenario installs");
        let layer_id = engine
            .spawn_object(SpawnConfig::new("ITEM").with_category(CATEGORY_OBJECT))
            .expect("layer spawns");
        let target_id = engine
            .spawn_object(SpawnConfig::new("ITEM").with_category(CATEGORY_OBJECT))
            .expect("target spawns");
        let direct_id = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(target_id),
            )
            .expect("direct content spawns");
        let grandchild_id = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(direct_id),
            )
            .expect("grandchild spawns");

        engine
            .call_scenario_script_function(
                "ApplyLayer",
                vec![
                    Value::Object(layer_id.as_u64()),
                    Value::Object(target_id.as_u64()),
                ],
            )
            .expect("scenario layer call runs");

        assert_eq!(
            engine.object_snapshot(target_id).expect("target remains").layer,
            Some(layer_id)
        );
        assert_eq!(
            engine.object_snapshot(direct_id).expect("content remains").layer,
            Some(layer_id),
            "scenario host snapshot exposes the target's direct contents"
        );
        assert_eq!(
            engine
                .object_snapshot(grandchild_id)
                .expect("grandchild remains")
                .layer,
            None,
            "layer propagation remains direct-only"
        );
    }

    #[test]
    fn object_blit_mode_base_get_set_reset_and_foreign_target_match_cpp() {
        // FnSetObjectBlitMode returns the previous raw base mode, marks a
        // nonzero mode CUSTOM (128), and resets zero to the target def mode.
        let caller_script = r#"#strict
local iSelfInitial, iForeignInitial, iSetPrevious, iSelfCustom;
local iResetPrevious, iSelfReset, iForeignPrevious, iForeignCustom;
func Trigger(object pOther) {
    iSelfInitial = GetObjectBlitMode();
    iForeignInitial = GetObjectBlitMode(pOther);
    iSetPrevious = SetObjectBlitMode(1);
    iSelfCustom = GetObjectBlitMode();
    iResetPrevious = SetObjectBlitMode();
    iSelfReset = GetObjectBlitMode();
    iForeignPrevious = SetObjectBlitMode(2, pOther);
    iForeignCustom = GetObjectBlitMode(pOther);
    return(1);
}
"#;
        let mut caller =
            Definition::from_script("CALL", "Caller", caller_script).expect("caller compiles");
        caller.set_blit_mode(2);
        let mut other =
            Definition::from_script("OTHR", "Other", "#strict\n").expect("other compiles");
        other.set_blit_mode(4);
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(other)
            .expect("other registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let other_id = engine
            .spawn_object(SpawnConfig::new("OTHR"))
            .expect("other spawns");

        let caller_index = engine.find_object_index(caller_id).expect("caller exists");
        engine
            .call_object_function(
                caller_index,
                "Trigger",
                vec![Value::Object(other_id.as_u64())],
            )
            .expect("blit mode trigger runs");

        let caller = engine.object_snapshot(caller_id).expect("caller remains");
        let locals = &caller.local_vars;
        assert_eq!(locals.get("iSelfInitial"), Some(&Value::Int(2)));
        assert_eq!(locals.get("iForeignInitial"), Some(&Value::Int(4)));
        assert_eq!(locals.get("iSetPrevious"), Some(&Value::Int(2)));
        assert_eq!(locals.get("iSelfCustom"), Some(&Value::Int(129)));
        assert_eq!(locals.get("iResetPrevious"), Some(&Value::Int(129)));
        assert_eq!(locals.get("iSelfReset"), Some(&Value::Int(2)));
        assert_eq!(locals.get("iForeignPrevious"), Some(&Value::Int(4)));
        assert_eq!(locals.get("iForeignCustom"), Some(&Value::Int(130)));
        assert_eq!(caller.blit_mode, 2);
        assert_eq!(
            engine
                .object_snapshot(other_id)
                .expect("other remains")
                .blit_mode,
            130
        );
    }

    #[test]
    fn object_blit_mode_overlay_updates_existing_only_and_returns_true() {
        let script = r#"#strict
local iSetExisting, iGetExisting, iSetMissing, iGetMissing;
func Trigger() {
    iSetExisting = SetObjectBlitMode(2, 0, 7);
    iGetExisting = GetObjectBlitMode(0, 7);
    iSetMissing = SetObjectBlitMode(4, 0, 8);
    iGetMissing = GetObjectBlitMode(0, 8);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let id = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        engine
            .apply_object_update(
                id,
                ObjectUpdate {
                    graphics_overlays: Some(vec![ObjectGraphicsOverlay::new(
                        7,
                        GraphicsOverlayMode::Base,
                    )
                    .with_blit_mode(1)]),
                    ..ObjectUpdate::default()
                },
            )
            .expect("overlay seeds");

        let index = engine.find_object_index(id).expect("caller exists");
        engine
            .call_object_function(index, "Trigger", Vec::new())
            .expect("overlay trigger runs");

        let snapshot = engine.object_snapshot(id).expect("caller remains");
        assert_eq!(snapshot.local_vars.get("iSetExisting"), Some(&Value::Int(1)));
        assert_eq!(snapshot.local_vars.get("iGetExisting"), Some(&Value::Int(2)));
        assert_eq!(snapshot.local_vars.get("iSetMissing"), Some(&Value::Nil));
        assert_eq!(snapshot.local_vars.get("iGetMissing"), Some(&Value::Nil));
        assert_eq!(snapshot.graphics_overlays.len(), 1);
        assert_eq!(snapshot.graphics_overlays[0].id, 7);
        assert_eq!(snapshot.graphics_overlays[0].blit_mode, 2);
    }

    #[test]
    fn create_paths_expose_definition_blit_mode_before_materialization() {
        let script = r#"#strict
local iObject, iConstruction, iContents;
func Trigger() {
    var pObject = CreateObject(ITEM, 0, 0, -1);
    iObject = GetObjectBlitMode(pObject);
    var pConstruction = CreateConstruction(ITEM, 0, 0, -1, 100);
    iConstruction = GetObjectBlitMode(pConstruction);
    var pContents = CreateContents(ITEM);
    iContents = GetObjectBlitMode(pContents);
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", script).expect("caller compiles"),
            )
            .expect("caller registers");
        let mut item =
            Definition::from_script("ITEM", "Item", "#strict\n").expect("item compiles");
        item.set_blit_mode(2);
        engine.register_definition(item).expect("item registers");
        let id = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let index = engine.find_object_index(id).expect("caller exists");

        engine
            .call_object_function(index, "Trigger", Vec::new())
            .expect("creation trigger runs");

        let snapshot = engine.object_snapshot(id).expect("caller remains");
        assert_eq!(snapshot.local_vars.get("iObject"), Some(&Value::Int(2)));
        assert_eq!(
            snapshot.local_vars.get("iConstruction"),
            Some(&Value::Int(2))
        );
        assert_eq!(snapshot.local_vars.get("iContents"), Some(&Value::Int(2)));
        assert_eq!(
            engine
                .snapshot()
                .objects
                .iter()
                .filter(|object| object.definition_id == "ITEM")
                .map(|object| object.blit_mode)
                .collect::<Vec<_>>(),
            vec![2, 2, 2]
        );
    }

    #[test]
    fn change_def_updates_default_blit_mode_but_preserves_custom_mode() {
        // C4Object::ChangeDef follows the new definition only when the old
        // mode lacks C4GFXBLIT_CUSTOM (C4Object.cpp:1231-1233).
        let script = r#"#strict
local bChanged, iAfter;
func Switch() {
    bChanged = ChangeDef(NEWD);
    iAfter = GetObjectBlitMode();
    return(1);
}
"#;
        let mut old =
            Definition::from_script("OLDD", "Old", script).expect("old definition compiles");
        old.set_blit_mode(2);
        let mut new = Definition::from_script("NEWD", "New", "#strict\n")
            .expect("new definition compiles");
        new.set_blit_mode(4);
        let mut engine = Engine::with_seed(0);
        engine.register_definition(old).expect("old registers");
        engine.register_definition(new).expect("new registers");
        let default_id = engine
            .spawn_object(SpawnConfig::new("OLDD"))
            .expect("default object spawns");
        let custom_id = engine
            .spawn_object(SpawnConfig::new("OLDD").with_blit_mode(129))
            .expect("custom object spawns");

        for id in [default_id, custom_id] {
            let index = engine.find_object_index(id).expect("object exists");
            engine
                .call_object_function(index, "Switch", Vec::new())
                .expect("definition switch runs");
        }

        let default = engine.object_snapshot(default_id).expect("default remains");
        assert_eq!(default.local_vars.get("bChanged"), Some(&Value::Bool(true)));
        assert_eq!(default.definition_id, "NEWD");
        assert_eq!(default.local_vars.get("iAfter"), Some(&Value::Int(4)));
        assert_eq!(default.blit_mode, 4);
        let custom = engine.object_snapshot(custom_id).expect("custom remains");
        assert_eq!(custom.definition_id, "NEWD");
        assert_eq!(custom.local_vars.get("iAfter"), Some(&Value::Int(129)));
        assert_eq!(custom.blit_mode, 129);
    }

    #[test]
    fn contained_change_def_silently_reenters_at_the_unsorted_contents_tail_like_cpp(
    ) -> Result<(), EngineError> {
        // C4Object::ChangeDef first performs Exit(..., false), marks the
        // object Unsorted, and then Enter(old_container, false)
        // (C4Object.cpp:1207-1254). The second false suppresses the normal
        // exit/entry callbacks, but Enter still queries the NEW definition's
        // RejectEntrance. Because Unsorted is already set, stContents Add
        // appends the object instead of category/id sorting it.
        let container_script = r#"#strict
local ejection_count, departure_count, collection_count, entrance_count;
protected func Ejection(pObject) { ejection_count += 1; return(1); }
protected func Collection2(pObject) { collection_count += 1; return(1); }
public func NoteDeparture() { departure_count += 1; return(1); }
public func NoteEntrance() { entrance_count += 1; return(1); }
"#;
        let old_script = r#"#strict
public func Swap() { return(ChangeDef(NEWD)); }
protected func Departure(pContainer) { pContainer->NoteDeparture(); return(1); }
"#;
        let new_script = r#"#strict
local reject_count;
protected func RejectEntrance(pContainer) { reject_count += 1; return(0); }
protected func Entrance(pContainer) { pContainer->NoteEntrance(); return(1); }
"#;

        let mut container = Definition::from_script("CONT", "Container", container_script)?;
        container.set_c4_callback_convention(true);
        let mut old = Definition::from_script("OLDD", "Old", old_script)?;
        old.set_category(CATEGORY_OBJECT);
        old.set_c4_callback_convention(true);
        let mut new = Definition::from_script("NEWD", "New", new_script)?;
        new.set_category(CATEGORY_STRUCTURE);
        new.set_c4_callback_convention(true);
        let mut peer = simple_definition("PEER");
        peer.set_category(CATEGORY_VEHICLE);

        let mut engine = Engine::with_seed(3);
        engine.register_definition(container)?;
        engine.register_definition(old)?;
        engine.register_definition(new)?;
        engine.register_definition(peer)?;

        let container_fix = FixedVec2::new(
            C4Fixed::from_raw(itofix(37).val() + 123),
            C4Fixed::from_raw(itofix(49).val() + 456),
        );
        let container_velocity = FixedVec2::new(itofix(3), itofix(-4));
        let container_rdir = C4Fixed::from_raw(777);
        let container = engine.spawn_object(
            SpawnConfig::new("CONT")
                .with_category(CATEGORY_STRUCTURE)
                .with_position(Vector2::new(37, 49))
                .with_fixed_position(container_fix)
                .with_fixed_velocity(container_velocity)
                .with_rotation_velocity(container_rdir)
                .with_mobile(true)
                .with_local_vars(HashMap::from([
                    ("ejection_count".to_string(), Value::Int(0)),
                    ("departure_count".to_string(), Value::Int(0)),
                    ("collection_count".to_string(), Value::Int(0)),
                    ("entrance_count".to_string(), Value::Int(0)),
                ])),
        )?;
        let peer_a = engine.spawn_object(
            SpawnConfig::new("PEER")
                .with_category(CATEGORY_VEHICLE)
                .with_container(container),
        )?;
        let peer_b = engine.spawn_object(
            SpawnConfig::new("PEER")
                .with_category(CATEGORY_VEHICLE)
                .with_container(container),
        )?;
        let changed = engine.spawn_object(
            SpawnConfig::new("OLDD")
                .with_category(CATEGORY_VEHICLE)
                .with_container(container),
        )?;

        let container_idx = engine
            .find_object_index(container)
            .expect("container exists");
        assert_eq!(
            engine.objects[container_idx].state.contents,
            vec![changed, peer_b, peer_a],
            "the fixture starts with the changed object at the sorted front"
        );

        let changed_idx = engine.find_object_index(changed).expect("object exists");
        engine.objects[changed_idx].fixed_velocity = FixedVec2::new(itofix(8), itofix(9));
        engine.objects[changed_idx].rotation_velocity = itofix(6);
        engine.objects[changed_idx].state.mobile = false;
        engine.objects[changed_idx].state.in_liquid = true;
        assert_eq!(
            engine.call_object_function(changed_idx, "Swap", Vec::new())?,
            Value::Bool(true)
        );

        let changed_idx = engine.find_object_index(changed).expect("object remains");
        let changed_object = &engine.objects[changed_idx];
        assert_eq!(changed_object.definition_id, "NEWD");
        assert_eq!(
            changed_object.state.category, CATEGORY_VEHICLE,
            "ChangeDef preserves the live C4Object::Category"
        );
        assert_eq!(changed_object.state.container, Some(container));
        assert_eq!(changed_object.state.position, Vector2::new(37, 49));
        assert_eq!(
            changed_object.fixed_position,
            FixedVec2::new(itofix(37), itofix(49)),
            "Enter CopyMotion snaps FixX/FixY to the container's integer position"
        );
        assert_eq!(changed_object.fixed_velocity, container_velocity);
        assert_eq!(
            changed_object.rotation_velocity,
            C4Fixed::ZERO,
            "CopyMotion copies xdir/ydir only; the silent Exit's zero rdir survives"
        );
        assert!(
            changed_object.state.mobile,
            "the silent Exit still mobilizes"
        );
        assert!(
            !changed_object.state.in_liquid,
            "the silent Exit clears InLiquid"
        );
        assert_eq!(
            changed_object.state.local_vars.get("reject_count"),
            Some(&Value::Int(1)),
            "the NEW definition's RejectEntrance runs synchronously"
        );

        let container_idx = engine
            .find_object_index(container)
            .expect("container remains");
        let container_state = &engine.objects[container_idx].state;
        assert_eq!(
            container_state.contents,
            vec![peer_b, peer_a, changed],
            "Unsorted bypasses stContents sorting and appends at the forward tail"
        );
        for callback in [
            "ejection_count",
            "departure_count",
            "collection_count",
            "entrance_count",
        ] {
            assert_eq!(
                container_state.local_vars.get(callback),
                Some(&Value::Int(0)),
                "ChangeDef's silent exit/re-entry suppresses {callback}"
            );
        }
        Ok(())
    }

    #[test]
    fn contained_change_def_reject_entrance_leaves_the_object_at_the_silent_exit_state(
    ) -> Result<(), EngineError> {
        let container_script = r#"#strict
local ejection_count, departure_count, collection_count, entrance_count;
protected func Ejection(pObject) { ejection_count += 1; return(1); }
protected func Collection2(pObject) { collection_count += 1; return(1); }
public func NoteDeparture() { departure_count += 1; return(1); }
public func NoteEntrance() { entrance_count += 1; return(1); }
"#;
        let old_script = r#"#strict
public func Swap() { return(ChangeDef(NEWD)); }
protected func Departure(pContainer) { pContainer->NoteDeparture(); return(1); }
"#;
        let new_script = r#"#strict
local reject_count;
protected func RejectEntrance(pContainer) { reject_count += 1; return(1); }
protected func Entrance(pContainer) { pContainer->NoteEntrance(); return(1); }
"#;

        let mut container = Definition::from_script("CONT", "Container", container_script)?;
        container.set_c4_callback_convention(true);
        let mut old = Definition::from_script("OLDD", "Old", old_script)?;
        old.set_c4_callback_convention(true);
        let mut new = Definition::from_script("NEWD", "New", new_script)?;
        new.set_rotateable(1);
        new.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(4);
        engine.register_definition(container)?;
        engine.register_definition(old)?;
        engine.register_definition(new)?;
        let container = engine.spawn_object(
            SpawnConfig::new("CONT")
                .with_position(Vector2::new(50, 60))
                .with_fixed_velocity(FixedVec2::new(itofix(2), itofix(-3)))
                .with_rotation_velocity(itofix(4))
                .with_local_vars(HashMap::from([
                    ("ejection_count".to_string(), Value::Int(0)),
                    ("departure_count".to_string(), Value::Int(0)),
                    ("collection_count".to_string(), Value::Int(0)),
                    ("entrance_count".to_string(), Value::Int(0)),
                ])),
        )?;
        let changed = engine.spawn_object(
            SpawnConfig::new("OLDD")
                .with_category(CATEGORY_OBJECT)
                .with_container(container),
        )?;
        let changed_idx = engine.find_object_index(changed).expect("object exists");
        engine.objects[changed_idx].state.position = Vector2::new(91, 92);
        engine.objects[changed_idx].fixed_position = FixedVec2::new(itofix(91), itofix(92));
        engine.objects[changed_idx].fixed_velocity = FixedVec2::new(itofix(8), itofix(-9));
        engine.objects[changed_idx].state.rotation = 27;
        engine.objects[changed_idx].fixed_rotation = itofix(27);
        engine.objects[changed_idx].rotation_velocity = itofix(5);
        engine.objects[changed_idx].state.mobile = false;
        engine.objects[changed_idx].state.in_liquid = true;

        assert_eq!(
            engine.call_object_function(changed_idx, "Swap", Vec::new())?,
            Value::Bool(true),
            "ChangeDef succeeds even when its attempted re-entry is vetoed"
        );

        let changed_idx = engine.find_object_index(changed).expect("object remains");
        let changed_object = &engine.objects[changed_idx];
        assert_eq!(changed_object.definition_id, "NEWD");
        assert_eq!(changed_object.state.container, None);
        assert_eq!(changed_object.state.position, Vector2::ZERO);
        assert_eq!(changed_object.fixed_position, FixedVec2::ZERO);
        assert_eq!(changed_object.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(changed_object.state.rotation, 0);
        assert_eq!(changed_object.fixed_rotation, C4Fixed::ZERO);
        assert_eq!(changed_object.rotation_velocity, C4Fixed::ZERO);
        assert!(changed_object.state.mobile);
        assert!(!changed_object.state.in_liquid);
        assert_eq!(
            changed_object.state.local_vars.get("reject_count"),
            Some(&Value::Int(1)),
            "the veto came from the NEW definition"
        );

        let container_idx = engine
            .find_object_index(container)
            .expect("container remains");
        let container_state = &engine.objects[container_idx].state;
        assert!(container_state.contents.is_empty());
        for callback in [
            "ejection_count",
            "departure_count",
            "collection_count",
            "entrance_count",
        ] {
            assert_eq!(
                container_state.local_vars.get(callback),
                Some(&Value::Int(0)),
                "a vetoed ChangeDef transfer still suppresses {callback}"
            );
        }
        Ok(())
    }

    #[test]
    fn contained_change_def_silent_exit_runs_old_bounds_contacts_before_action_and_swap(
    ) -> Result<(), EngineError> {
        // ChangeDef's Exit(0,0,...,false) still calls BoundsCheck. With the
        // OLD definition's side/top BorderBound and negative shape origin,
        // it clamps the target to (4,3). TargetBounds invokes ContactLeft and
        // ContactTop even though fCalls=false: containment is already cleared,
        // position is still the pre-exit value, and each velocity component is
        // zeroed immediately before its corresponding callback. Only after
        // both contacts does Exit assign the clamped position; then the old
        // action AbortCall and new-definition RejectEntrance run.
        const SHARED_LOCALS: &str = r#"
local order;
local left_x, left_y, left_xdir, left_ydir, left_contained;
local top_x, top_y, top_xdir, top_ydir, top_contained;
local abort_x, abort_y, abort_contained;
local reject_x, reject_y, reject_contained;
local departure_calls, entrance_calls;
"#;
        let old_script = format!(
            r#"#strict
{SHARED_LOCALS}
protected func ContactLeft()
{{
    order = order * 10 + 1;
    left_x = GetX(); left_y = GetY();
    left_xdir = GetXDir(); left_ydir = GetYDir();
    left_contained = !!Contained();
    return(0);
}}
protected func ContactTop()
{{
    order = order * 10 + 2;
    top_x = GetX(); top_y = GetY();
    top_xdir = GetXDir(); top_ydir = GetYDir();
    top_contained = !!Contained();
    return(0);
}}
protected func OldAbort(int old_phase)
{{
    var no_value;
    order = order * 10 + 3;
    abort_x = GetX(); abort_y = GetY(); abort_contained = !!Contained();
    return(1);
}}
protected func Departure(pContainer) {{ departure_calls += 1; return(1); }}
public func Swap() {{ return(ChangeDef(NEWD)); }}
"#
        );
        let new_script = format!(
            r#"#strict
{SHARED_LOCALS}
protected func RejectEntrance(pContainer)
{{
    order = order * 10 + 4;
    reject_x = GetX(); reject_y = GetY(); reject_contained = !!Contained();
    return(1);
}}
protected func Entrance(pContainer) {{ entrance_calls += 1; return(1); }}
"#
        );
        let mut old = Definition::from_script("OLDD", "Old", &old_script)?;
        old.set_c4_callback_convention(true);
        old.set_shape_rect(Some(DefinitionRect::new(-4, -3, 8, 6)));
        old.set_border_bound(C4D_BORDER_SIDES | C4D_BORDER_TOP);
        old.set_contact_function_calls(true);
        old.configure_actions(
            None,
            HashMap::from([(
                "Work".to_string(),
                ActionSpec::default().with_abort_call("OldAbort"),
            )]),
        );
        let mut new = Definition::from_script("NEWD", "New", &new_script)?;
        new.set_c4_callback_convention(true);
        let mut container = Definition::from_script(
            "CONT",
            "Container",
            "#strict\nlocal ejection_calls; protected func Ejection(pObject) { ejection_calls += 1; return(1); }\n",
        )?;
        container.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(44);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.register_definition(container)?;
        engine.register_definition(old)?;
        engine.register_definition(new)?;
        let container = engine.spawn_object(
            SpawnConfig::new("CONT")
                .with_position(Vector2::new(30, 40))
                .with_local_vars(HashMap::from([(
                    "ejection_calls".to_string(),
                    Value::Int(0),
                )])),
        )?;
        let changed = engine.spawn_object(
            SpawnConfig::new("OLDD")
                .with_container(container)
                .with_action(ActionState::new("Work"))
                .with_local_vars(HashMap::from([
                    ("departure_calls".to_string(), Value::Int(0)),
                    ("entrance_calls".to_string(), Value::Int(0)),
                ])),
        )?;
        let changed_index = engine.find_object_index(changed).expect("object exists");
        engine.objects[changed_index].set_fixed_velocity(FixedVec2::new(itofix(8), itofix(9)));

        assert_eq!(
            engine.call_object_function(changed_index, "Swap", Vec::new())?,
            Value::Bool(true)
        );

        let changed_index = engine.find_object_index(changed).expect("object remains");
        let changed_object = &engine.objects[changed_index];
        assert_eq!(changed_object.definition_id, "NEWD");
        assert_eq!(changed_object.state.container, None);
        assert_eq!(changed_object.state.position, Vector2::new(4, 3));
        assert_eq!(
            changed_object.fixed_position,
            FixedVec2::new(itofix(4), itofix(3))
        );
        assert_eq!(changed_object.fixed_velocity, FixedVec2::ZERO);
        let locals = &changed_object.state.local_vars;
        for (name, expected) in [
            ("order", 1234),
            ("left_x", 30),
            ("left_y", 40),
            ("left_xdir", 0),
            ("left_ydir", 90),
            ("top_x", 30),
            ("top_y", 40),
            ("top_xdir", 0),
            ("top_ydir", 0),
            ("abort_x", 4),
            ("abort_y", 3),
            ("reject_x", 4),
            ("reject_y", 3),
            ("departure_calls", 0),
            ("entrance_calls", 0),
        ] {
            assert_eq!(locals.get(name), Some(&Value::Int(expected)), "{name}");
        }
        for name in [
            "left_contained",
            "top_contained",
            "abort_contained",
            "reject_contained",
        ] {
            assert_eq!(locals.get(name), Some(&Value::Bool(false)), "{name}");
        }
        let container_index = engine
            .find_object_index(container)
            .expect("container remains");
        assert!(engine.objects[container_index].state.contents.is_empty());
        assert_eq!(
            engine.objects[container_index]
                .state
                .local_vars
                .get("ejection_calls"),
            Some(&Value::Int(0)),
            "fCalls=false suppresses Ejection while BoundsCheck contacts still run"
        );
        Ok(())
    }

    #[test]
    fn contained_burn_turn_to_runs_old_bounds_contacts_before_definition_swap(
    ) -> Result<(), EngineError> {
        // Native FnFxFireStart reaches the same C4Object::ChangeDef method as
        // the script host. Its silent Exit must use OLDD's Shape/BorderBound,
        // call Left then Top while the object is already uncontained but still
        // at its pre-exit position, and only then install the clamped target,
        // reset the old action, swap Def, and query NEWD::RejectEntrance.
        const LOCALS: &str = r#"
local order;
local left_x, left_y, left_xdir, left_ydir, left_contained;
local top_x, top_y, top_xdir, top_ydir, top_contained;
local abort_x, abort_y, reject_x, reject_y;
local departure_calls, entrance_calls;
"#;
        let old_script = format!(
            r#"#strict
{LOCALS}
protected func ContactLeft()
{{
    order = order * 10 + 1;
    left_x = GetX(); left_y = GetY();
    left_xdir = GetXDir(); left_ydir = GetYDir();
    left_contained = !!Contained();
    return(0);
}}
protected func ContactTop()
{{
    order = order * 10 + 2;
    top_x = GetX(); top_y = GetY();
    top_xdir = GetXDir(); top_ydir = GetYDir();
    top_contained = !!Contained();
    return(0);
}}
protected func OldAbort(int old_phase)
{{
    order = order * 10 + 3;
    abort_x = GetX(); abort_y = GetY();
    return(1);
}}
protected func Departure(pContainer) {{ departure_calls += 1; return(1); }}
"#
        );
        let new_script = format!(
            r#"#strict
{LOCALS}
protected func RejectEntrance(pContainer)
{{
    order = order * 10 + 4;
    reject_x = GetX(); reject_y = GetY();
    return(1);
}}
protected func Entrance(pContainer) {{ entrance_calls += 1; return(1); }}
"#
        );
        let mut old = Definition::from_script("OLDD", "Old", &old_script)?;
        old.set_c4_callback_convention(true);
        old.set_shape_rect(Some(DefinitionRect::new(-5, -2, 10, 4)));
        old.set_border_bound(C4D_BORDER_SIDES | C4D_BORDER_TOP);
        old.set_contact_function_calls(true);
        old.set_burn_turn_to(Some("NEWD".to_string()));
        old.configure_actions(
            None,
            HashMap::from([(
                "Work".to_string(),
                ActionSpec::default().with_abort_call("OldAbort"),
            )]),
        );
        let mut new = Definition::from_script("NEWD", "New", &new_script)?;
        new.set_c4_callback_convention(true);
        new.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        let mut container = Definition::from_script(
            "CONT",
            "Container",
            "#strict\nlocal ejection_calls; protected func Ejection(pObject) { ejection_calls += 1; return(1); }\n",
        )?;
        container.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(45);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.register_definition(container)?;
        engine.register_definition(old)?;
        engine.register_definition(new)?;
        let container = engine.spawn_object(
            SpawnConfig::new("CONT")
                .with_position(Vector2::new(25, 35))
                .with_local_vars(HashMap::from([(
                    "ejection_calls".to_string(),
                    Value::Int(0),
                )])),
        )?;
        let burner = engine.spawn_object(
            SpawnConfig::new("OLDD")
                .with_container(container)
                .with_action(ActionState::new("Work"))
                .with_local_vars(HashMap::from([
                    ("departure_calls".to_string(), Value::Int(0)),
                    ("entrance_calls".to_string(), Value::Int(0)),
                ])),
        )?;
        let burner_index = engine.find_object_index(burner).expect("burner exists");
        engine.objects[burner_index].set_fixed_velocity(FixedVec2::new(itofix(6), itofix(7)));

        assert!(engine.incinerate_object(burner_index, 1, false, None)?);

        let burner_index = engine.find_object_index(burner).expect("burner remains");
        let burner_object = &engine.objects[burner_index];
        assert_eq!(burner_object.definition_id, "NEWD");
        assert!(burner_object.state.on_fire);
        assert_eq!(burner_object.state.container, None);
        assert_eq!(burner_object.state.position, Vector2::new(5, 2));
        assert_eq!(
            burner_object.fixed_position,
            FixedVec2::new(itofix(5), itofix(2))
        );
        assert_eq!(burner_object.fixed_velocity, FixedVec2::ZERO);
        let locals = &burner_object.state.local_vars;
        for (name, expected) in [
            ("order", 1234),
            ("left_x", 25),
            ("left_y", 35),
            ("left_xdir", 0),
            ("left_ydir", 70),
            ("top_x", 25),
            ("top_y", 35),
            ("top_xdir", 0),
            ("top_ydir", 0),
            ("abort_x", 5),
            ("abort_y", 2),
            ("reject_x", 5),
            ("reject_y", 2),
            ("departure_calls", 0),
            ("entrance_calls", 0),
        ] {
            assert_eq!(locals.get(name), Some(&Value::Int(expected)), "{name}");
        }
        for name in ["left_contained", "top_contained"] {
            assert_eq!(locals.get(name), Some(&Value::Bool(false)), "{name}");
        }
        let container_index = engine
            .find_object_index(container)
            .expect("container remains");
        assert!(engine.objects[container_index].state.contents.is_empty());
        assert_eq!(
            engine.objects[container_index]
                .state
                .local_vars
                .get("ejection_calls"),
            Some(&Value::Int(0))
        );
        Ok(())
    }

    #[test]
    fn contained_change_def_bounds_use_live_shape_then_updateface_restores_definition_shape_on_both_paths(
    ) -> Result<(), EngineError> {
        // BoundsCheck consumes the live C4Object::Shape, including a
        // preceding SetShape, rather than reconstructing Def->Shape. Exit's
        // later UpdateFace(true) then restores the current definition shape
        // before ChangeDef performs SetAction(ActIdle); the definition swap
        // performs another UpdateFace before NEWD::RejectEntrance. Exercise
        // both the same-call script host path and native BurnTurnTo path.
        for via_burn_turn_to in [false, true] {
            let path = if via_burn_turn_to {
                "native BurnTurnTo"
            } else {
                "script ChangeDef"
            };
            const SHAPE_LOCALS: &str = r#"
local abort_width, abort_height, abort_x, abort_y;
local reject_width, reject_height, reject_x, reject_y;
"#;
            let old_script = format!(
                r#"#strict
{SHAPE_LOCALS}
public func Prime(bool do_swap)
{{
    SetShape(-7, -8, 14, 16);
    if (do_swap) return(ChangeDef(NEWD));
    return(1);
}}
protected func OldAbort(int old_phase)
{{
    var no_value;
    abort_width = GetObjectVal("Width");
    abort_height = GetObjectVal("Height");
    abort_x = GetObjectVal("Offset", no_value, no_value, 0);
    abort_y = GetObjectVal("Offset", no_value, no_value, 1);
    return(1);
}}
"#
            );
            let new_script = format!(
                r#"#strict
{SHAPE_LOCALS}
protected func RejectEntrance(pContainer)
{{
    var no_value;
    reject_width = GetObjectVal("Width");
    reject_height = GetObjectVal("Height");
    reject_x = GetObjectVal("Offset", no_value, no_value, 0);
    reject_y = GetObjectVal("Offset", no_value, no_value, 1);
    return(1);
}}
"#
            );

            let mut old = Definition::from_script("OLDD", "Old", &old_script)?;
            old.set_c4_callback_convention(true);
            old.set_shape_rect(Some(DefinitionRect::new(-2, -3, 4, 6)));
            old.set_border_bound(C4D_BORDER_SIDES | C4D_BORDER_TOP);
            old.set_burn_turn_to(Some("NEWD".to_string()));
            old.configure_actions(
                None,
                HashMap::from([(
                    "Work".to_string(),
                    ActionSpec::default().with_abort_call("OldAbort"),
                )]),
            );
            let mut new = Definition::from_script("NEWD", "New", &new_script)?;
            new.set_c4_callback_convention(true);
            new.set_shape_rect(Some(DefinitionRect::new(-1, -4, 2, 8)));
            let container = simple_definition("CONT");

            let mut engine = Engine::with_seed(52);
            engine.set_landscape(Landscape::flat(100, 100));
            engine.register_definition(container)?;
            engine.register_definition(old)?;
            engine.register_definition(new)?;
            let container = engine
                .spawn_object(SpawnConfig::new("CONT").with_position(Vector2::new(30, 40)))?;
            let changed = engine.spawn_object(
                SpawnConfig::new("OLDD")
                    .with_container(container)
                    .with_action(ActionState::new("Work")),
            )?;
            let changed_index = engine.find_object_index(changed).expect("object exists");
            if via_burn_turn_to {
                assert_eq!(
                    engine.call_object_function(
                        changed_index,
                        "Prime",
                        vec![Value::Bool(false)],
                    )?,
                    Value::Int(1),
                    "{path}: SetShape primes the live object shape"
                );
                let changed_index = engine.find_object_index(changed).expect("object remains");
                assert!(engine.incinerate_object(changed_index, 1, false, None)?);
            } else {
                assert_eq!(
                    engine.call_object_function(changed_index, "Prime", vec![Value::Bool(true)],)?,
                    Value::Bool(true),
                    "{path}: same-call SetShape + ChangeDef succeeds"
                );
            }

            let changed_index = engine.find_object_index(changed).expect("object remains");
            let object = &engine.objects[changed_index];
            assert_eq!(object.definition_id, "NEWD", "{path}: definition changes");
            assert_eq!(object.state.container, None, "{path}: re-entry is vetoed");
            assert_eq!(
                object.state.position,
                Vector2::new(7, 8),
                "{path}: BoundsCheck clamps with live SetShape offsets"
            );
            assert_eq!(
                object.state.shape_override, None,
                "{path}: non-line UpdateFace(true) discards SetShape geometry"
            );
            for (name, expected) in [
                ("abort_width", 4),
                ("abort_height", 6),
                ("abort_x", -2),
                ("abort_y", -3),
                ("reject_width", 2),
                ("reject_height", 8),
                ("reject_x", -1),
                ("reject_y", -4),
            ] {
                assert_eq!(
                    object.state.local_vars.get(name),
                    Some(&Value::Int(expected)),
                    "{path}: {name}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn contained_change_def_bounds_preserve_then_refresh_ocf_at_cpp_boundaries_on_both_paths(
    ) -> Result<(), EngineError> {
        // The old container receives a full SetOCF after its contents link is
        // removed and before BoundsCheck. The moving target deliberately does
        // not: ContactLeft observes its cached contained/high-speed mask even
        // though xdir is already zero. Exit's tail then installs the clamped
        // surface position and zero dirs and performs a full target SetOCF,
        // so OldAbort observes the recomputed terrain/availability mask.
        for via_burn_turn_to in [false, true] {
            let path = if via_burn_turn_to {
                "native BurnTurnTo"
            } else {
                "script ChangeDef"
            };
            const OCF_LOCALS: &str = r#"
local home;
local contact_ocf, contact_xdir, parent_ocf;
local abort_ocf, reject_ocf;
"#;
            let old_script = format!(
                r#"#strict
{OCF_LOCALS}
public func Swap() {{ return(ChangeDef(NEWD)); }}
protected func ContactLeft()
{{
    contact_ocf = GetOCF();
    contact_xdir = GetXDir();
    parent_ocf = GetOCF(home);
    return(0);
}}
protected func ContactTop() {{ return(0); }}
protected func OldAbort(int old_phase)
{{
    abort_ocf = GetOCF();
    return(1);
}}
"#
            );
            let new_script = format!(
                r#"#strict
{OCF_LOCALS}
protected func RejectEntrance(pContainer)
{{
    reject_ocf = GetOCF();
    return(1);
}}
"#
            );

            let mut container = simple_definition("CONT");
            // Put-only and no Entrance: the contained target starts without
            // OCF_Available, making the stale-vs-post-Exit boundary explicit.
            container.set_grab_put_get(1);
            let mut old = Definition::from_script("OLDD", "Old", &old_script)?;
            old.set_c4_callback_convention(true);
            old.set_shape_rect(Some(DefinitionRect::new(-4, -60, 8, 120)));
            old.set_border_bound(C4D_BORDER_SIDES | C4D_BORDER_TOP);
            old.set_contact_function_calls(true);
            old.set_burn_turn_to(Some("NEWD".to_string()));
            old.configure_actions(
                None,
                HashMap::from([(
                    "Work".to_string(),
                    ActionSpec::default().with_abort_call("OldAbort"),
                )]),
            );
            let mut new = Definition::from_script("NEWD", "New", &new_script)?;
            new.set_c4_callback_convention(true);

            let mut engine = Engine::with_seed(53);
            // At y=60 the center is solid while y-1 is free: SetOCF must set
            // both InSolid and InFree, and the free-above clause sets Available.
            engine.set_landscape(Landscape::flat(100, 60));
            engine.register_definition(container)?;
            engine.register_definition(old)?;
            engine.register_definition(new)?;
            let container = engine
                .spawn_object(SpawnConfig::new("CONT").with_position(Vector2::new(30, 40)))?;
            let changed = engine.spawn_object(
                SpawnConfig::new("OLDD")
                    .with_container(container)
                    .with_action(ActionState::new("Work"))
                    .with_local_vars(HashMap::from([(
                        "home".to_string(),
                        compat::object_reference_value(container),
                    )])),
            )?;

            let container_index = engine
                .find_object_index(container)
                .expect("container exists");
            assert_eq!(
                engine.object_ocf_at_index(container_index) & ocf::HIT_SPEED4,
                0,
                "{path}: parent OCF starts without HitSpeed4"
            );
            engine.objects[container_index]
                .set_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(9)));
            assert_eq!(
                engine.object_ocf_at_index(container_index) & ocf::HIT_SPEED4,
                0,
                "{path}: changing velocity alone leaves the parent cache stale"
            );

            let changed_index = engine.find_object_index(changed).expect("object exists");
            engine.objects[changed_index]
                .set_fixed_velocity(FixedVec2::new(itofix(9), C4Fixed::ZERO));
            engine.refresh_object_ocf(changed_index);
            let cached_before = engine.object_ocf_at_index(changed_index);
            assert_ne!(cached_before & ocf::HIT_SPEED4, 0);
            assert_eq!(
                cached_before
                    & (ocf::NOT_CONTAINED | ocf::IN_SOLID | ocf::IN_FREE | ocf::AVAILABLE),
                0,
                "{path}: put-only containment hides all outside-only bits"
            );

            if via_burn_turn_to {
                assert!(engine.incinerate_object(changed_index, 1, false, None)?);
            } else {
                assert_eq!(
                    engine.call_object_function(changed_index, "Swap", Vec::new())?,
                    Value::Bool(true)
                );
            }

            let changed_index = engine.find_object_index(changed).expect("object remains");
            let object = &engine.objects[changed_index];
            assert_eq!(object.state.position, Vector2::new(4, 60), "{path}: clamp");
            let local_mask = |name: &str| -> u32 {
                match object.state.local_vars.get(name) {
                    Some(Value::Int(value)) => *value as u32,
                    other => panic!("{path}: expected integer {name}, got {other:?}"),
                }
            };
            let contact_ocf = local_mask("contact_ocf");
            assert_ne!(
                contact_ocf & ocf::HIT_SPEED4,
                0,
                "{path}: Contact sees the target's stale cached HitSpeed4"
            );
            assert_eq!(
                contact_ocf & (ocf::NOT_CONTAINED | ocf::IN_SOLID | ocf::IN_FREE | ocf::AVAILABLE),
                0,
                "{path}: Contact still sees the cached contained OCF"
            );
            assert_eq!(
                object.state.local_vars.get("contact_xdir"),
                Some(&Value::Int(0)),
                "{path}: TargetBounds zeroes xdir before Contact"
            );
            assert_ne!(
                local_mask("parent_ocf") & ocf::HIT_SPEED4,
                0,
                "{path}: the old parent's pre-Contact SetOCF is a full refresh"
            );

            for callback in ["abort_ocf", "reject_ocf"] {
                let refreshed = local_mask(callback);
                assert_eq!(
                    refreshed & ocf::HIT_SPEED4,
                    0,
                    "{path}: {callback} sees zero-dir HitSpeed refresh"
                );
                assert_eq!(
                    refreshed
                        & (ocf::NOT_CONTAINED | ocf::IN_SOLID | ocf::IN_FREE | ocf::AVAILABLE),
                    ocf::NOT_CONTAINED | ocf::IN_SOLID | ocf::IN_FREE | ocf::AVAILABLE,
                    "{path}: {callback} sees the full post-Exit position/container refresh"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn change_def_waits_for_a_later_global_unsorted_sweep_then_uses_the_new_def_cluster(
    ) -> Result<(), EngineError> {
        // ChangeDef sets C4Object::Unsorted but (unlike C4Object::Resort)
        // does not set Game.fResortAnyObject. Its master-list slot therefore
        // survives until some unrelated Resort requests the global
        // ResortUnsorted scan; the eventual re-add uses the NEW definition
        // while preserving the object's independently stored Category.
        let mut old = Definition::from_script(
            "OLDD",
            "Old",
            "#strict\npublic func Swap() { return(ChangeDef(NEWD)); }\n",
        )?;
        old.set_category(CATEGORY_OBJECT);
        let mut new = simple_definition("NEWD");
        new.set_category(CATEGORY_STRUCTURE);
        let trigger = Definition::from_script(
            "TRIG",
            "Trigger",
            "#strict\npublic func Wake() { Resort(); return(1); }\n",
        )?;

        let mut engine = Engine::with_seed(5);
        engine.register_definition(old)?;
        engine.register_definition(new)?;
        engine.register_definition(simple_definition("SEPR"))?;
        engine.register_definition(trigger)?;

        let changed =
            engine.spawn_object(SpawnConfig::new("OLDD").with_category(CATEGORY_OBJECT))?;
        let separator =
            engine.spawn_object(SpawnConfig::new("SEPR").with_category(CATEGORY_OBJECT))?;
        let new_peer =
            engine.spawn_object(SpawnConfig::new("NEWD").with_category(CATEGORY_OBJECT))?;
        let trigger =
            engine.spawn_object(SpawnConfig::new("TRIG").with_category(CATEGORY_STATIC_BACK))?;
        let before = vec![trigger, changed, separator, new_peer];
        assert_eq!(engine.debug_exec_order(), before);

        let changed_idx = engine
            .find_object_index(changed)
            .expect("old object exists");
        assert_eq!(
            engine.call_object_function(changed_idx, "Swap", Vec::new())?,
            Value::Bool(true)
        );
        let changed_idx = engine
            .find_object_index(changed)
            .expect("changed object remains");
        assert_eq!(engine.objects[changed_idx].definition_id, "NEWD");
        assert_eq!(
            engine.objects[changed_idx].state.category, CATEGORY_OBJECT,
            "ChangeDef does not copy the new definition's Category"
        );
        assert_eq!(
            engine.debug_exec_order(),
            before,
            "ChangeDef marks Unsorted without immediately moving the master-list link"
        );
        engine.execute_object_order_commands();
        assert_eq!(
            engine.debug_exec_order(),
            before,
            "ChangeDef alone does not request the global ResortUnsorted scan"
        );

        let trigger_idx = engine.find_object_index(trigger).expect("trigger exists");
        engine.call_object_function(trigger_idx, "Wake", Vec::new())?;
        assert_eq!(engine.debug_exec_order(), before, "Resort is deferred");
        engine.execute_object_order_commands();
        assert_eq!(
            engine.debug_exec_order(),
            vec![trigger, separator, new_peer, changed],
            "the unrelated Resort sweeps every Unsorted object and ChangeDef's re-add joins NEWD"
        );
        Ok(())
    }

    #[test]
    fn change_def_unsorted_link_survives_a_frame_until_an_explicit_resort(
    ) -> Result<(), EngineError> {
        // ChangeDef sets Unsorted without setting fResortAnyObject. In
        // particular, the per-frame execution setup must not run the
        // post-load FixObjectOrder pass over that link. Use a legitimate raw
        // multi-bit sort category so an accidental FixObjectOrder call is
        // observable both as a category rewrite and as a link move.
        let mut old = Definition::from_script(
            "OLDD",
            "Old",
            "#strict\npublic func Swap() { return(ChangeDef(NEWD)); }\n",
        )?;
        old.set_category(CATEGORY_LIVING | CATEGORY_OBJECT);
        let new = simple_definition("NEWD");
        let trigger = Definition::from_script(
            "TRIG",
            "Trigger",
            "#strict\npublic func Wake() { Resort(); return(1); }\n",
        )?;

        let mut engine = Engine::with_seed(51);
        engine.set_landscape(Landscape::flat(100, 100));
        engine.register_definition(old)?;
        engine.register_definition(new)?;
        engine.register_definition(simple_definition("ANCH"))?;
        engine.register_definition(trigger)?;

        let trigger =
            engine.spawn_object(SpawnConfig::new("TRIG").with_category(CATEGORY_STATIC_BACK))?;
        let anchor =
            engine.spawn_object(SpawnConfig::new("ANCH").with_category(CATEGORY_OBJECT))?;
        let raw_category = CATEGORY_LIVING | CATEGORY_OBJECT;
        let changed = engine.spawn_object(SpawnConfig::new("OLDD").with_category(raw_category))?;
        let fixed_link = vec![trigger, anchor, changed];
        assert_eq!(engine.debug_exec_order(), fixed_link);

        let changed_index = engine
            .find_object_index(changed)
            .expect("changed object exists");
        assert_eq!(
            engine.call_object_function(changed_index, "Swap", Vec::new())?,
            Value::Bool(true)
        );
        assert!(engine.objects[changed_index].unsorted);

        engine.tick_without_snapshot()?;
        let changed_index = engine
            .find_object_index(changed)
            .expect("changed object remains");
        assert_eq!(
            engine.debug_exec_order(),
            fixed_link,
            "a frame boundary does not move a ChangeDef-only Unsorted link"
        );
        assert_eq!(
            engine.objects[changed_index].state.category, raw_category,
            "runtime categories are not normalized by the post-load repair"
        );
        assert!(
            engine.objects[changed_index].unsorted,
            "ChangeDef alone does not arm the global Unsorted sweep"
        );

        let trigger_index = engine.find_object_index(trigger).expect("trigger remains");
        engine.call_object_function(trigger_index, "Wake", Vec::new())?;
        engine.execute_object_order_commands();
        let changed_index = engine
            .find_object_index(changed)
            .expect("changed object remains");
        assert!(
            !engine.objects[changed_index].unsorted,
            "an explicit Resort finally consumes the ChangeDef flag"
        );
        assert_eq!(engine.debug_exec_order(), fixed_link);
        Ok(())
    }

    #[test]
    fn construction_change_def_keeps_the_original_spawn_link_until_a_later_resort(
    ) -> Result<(), EngineError> {
        // C4Game::NewObject adds the fresh object to Game.Objects using its
        // original definition/category before Construction runs. A ChangeDef
        // in Construction only marks that existing link Unsorted; it must not
        // be re-added at the forward tail until some later Resort sets the
        // global fResortAnyObject trigger.
        let mut old = Definition::from_script(
            "OLDD",
            "Old",
            "#strict\nprotected func Construction() { ChangeDef(NEWD); }\n",
        )?;
        old.set_c4_callback_convention(true);
        old.set_category(CATEGORY_OBJECT);
        let mut new = simple_definition("NEWD");
        new.set_category(CATEGORY_STRUCTURE);
        let trigger = Definition::from_script(
            "TRIG",
            "Trigger",
            "#strict\npublic func Wake() { Resort(); return(1); }\n",
        )?;

        let mut engine = Engine::with_seed(6);
        engine.register_definition(old)?;
        engine.register_definition(new)?;
        engine.register_definition(simple_definition("SEPR"))?;
        engine.register_definition(trigger)?;

        let trigger =
            engine.spawn_object(SpawnConfig::new("TRIG").with_category(CATEGORY_STATIC_BACK))?;
        let new_peer =
            engine.spawn_object(SpawnConfig::new("NEWD").with_category(CATEGORY_OBJECT))?;
        let separator =
            engine.spawn_object(SpawnConfig::new("SEPR").with_category(CATEGORY_OBJECT))?;
        let changed =
            engine.spawn_object(SpawnConfig::new("OLDD").with_category(CATEGORY_OBJECT))?;

        let before_sweep = vec![trigger, new_peer, separator, changed];
        let changed_index = engine.find_object_index(changed).expect("object exists");
        assert_eq!(engine.objects[changed_index].definition_id, "NEWD");
        assert_eq!(
            engine.objects[changed_index].state.category,
            CATEGORY_OBJECT
        );
        assert!(engine.objects[changed_index].unsorted);
        assert_eq!(
            engine.debug_exec_order(),
            before_sweep,
            "Construction ChangeDef keeps the link inserted for OLDD"
        );
        engine.execute_object_order_commands();
        assert_eq!(
            engine.debug_exec_order(),
            before_sweep,
            "ChangeDef alone does not arm the global unsorted sweep"
        );

        let trigger_index = engine.find_object_index(trigger).expect("trigger exists");
        engine.call_object_function(trigger_index, "Wake", Vec::new())?;
        assert_eq!(
            engine.debug_exec_order(),
            before_sweep,
            "Resort is deferred"
        );
        engine.execute_object_order_commands();
        assert_eq!(
            engine.debug_exec_order(),
            vec![trigger, new_peer, changed, separator],
            "the later sweep re-adds the object into NEWD's cluster"
        );
        Ok(())
    }

    #[test]
    fn change_def_self_arrow_resolves_the_new_definition_inline() {
        // AB_CALLFS reads pDestObj->Def when the instruction executes. The
        // old callback remains on the stack after ChangeDef, but an explicit
        // `this()->~Probe()` must resolve on NEWD even though OLDD also owns a
        // Probe with the same name (C4AulExec.cpp:1216-1305).
        let old = Definition::from_script(
            "OLDD",
            "Old",
            r#"#strict
func Probe() { return(1); }
func Swap() { ChangeDef(NEWD); return(this()->~Probe()); }
"#,
        )
        .expect("old definition compiles");
        let new = Definition::from_script(
            "NEWD",
            "New",
            "#strict\nfunc Probe() { return(2); }\n",
        )
        .expect("new definition compiles");
        let mut engine = Engine::with_seed(0);
        engine.register_definition(old).expect("old registers");
        engine.register_definition(new).expect("new registers");
        let id = engine
            .spawn_object(SpawnConfig::new("OLDD"))
            .expect("old object spawns");
        let index = engine.find_object_index(id).expect("object exists");

        let result = engine
            .call_object_function(index, "Swap", Vec::new())
            .expect("definition switch and arrow call complete");

        assert_eq!(result, Value::Int(2));
        assert_eq!(
            engine.object_snapshot(id).expect("object remains").definition_id,
            "NEWD"
        );
    }

    #[test]
    fn self_arrow_world_dispatch_keeps_object_locals_live() {
        // Object-arrow dispatch always resolves through the target's live
        // Def in C++. The same-object round trip must still share the one
        // C4Object local table with its suspended caller.
        let definition = Definition::from_script(
            "LIVE",
            "Live locals",
            r#"#strict
local iValue;
func Inner() { iValue += 2; return(iValue); }
func Outer() { iValue = 5; var iResult = this()->Inner(); return(iResult * 100 + iValue); }
"#,
        )
        .expect("definition compiles");
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("LIVE"))
            .expect("object spawns");
        let index = engine.find_object_index(id).expect("object exists");

        let result = engine
            .call_object_function(index, "Outer", Vec::new())
            .expect("same-object arrow call completes");

        assert_eq!(result, Value::Int(707));
        assert_eq!(
            engine
                .object_snapshot(id)
                .expect("object remains")
                .local_vars
                .get("iValue"),
            Some(&Value::Int(7))
        );
    }

    #[test]
    fn change_def_self_arrow_reference_resolves_the_new_definition_inline() {
        // AB_CALL's reference-return form uses the same live pDestObj->Def
        // lookup as an ordinary value call. Assignment must therefore write
        // NEWD's local, not the identically named old-definition function.
        let old = Definition::from_script(
            "OLDD",
            "Old",
            r#"#strict
local iOld;
func &Slot() { return(iOld); }
func Swap() { ChangeDef(NEWD); this()->Slot() = 9; return(LocalN("iNew")); }
"#,
        )
        .expect("old definition compiles");
        let new = Definition::from_script(
            "NEWD",
            "New",
            r#"#strict
local iNew;
func &Slot() { return(iNew); }
"#,
        )
        .expect("new definition compiles");
        let mut engine = Engine::with_seed(0);
        engine.register_definition(old).expect("old registers");
        engine.register_definition(new).expect("new registers");
        let id = engine
            .spawn_object(SpawnConfig::new("OLDD"))
            .expect("old object spawns");
        let index = engine.find_object_index(id).expect("object exists");

        let result = engine
            .call_object_function(index, "Swap", Vec::new())
            .expect("definition switch and reference call complete");

        assert_eq!(result, Value::Int(9));
        let snapshot = engine.object_snapshot(id).expect("object remains");
        assert_eq!(snapshot.local_vars.get("iNew"), Some(&Value::Int(9)));
        assert_ne!(snapshot.local_vars.get("iOld"), Some(&Value::Int(9)));
    }

    // FnChangeDef -> C4Object::ChangeDef (C4Object.cpp:1180-1231): the
    // object swaps to the new definition in place - number/position/owner
    // survive, the action resets to ActIdle, dir 0, rotation clears for
    // non-rotateable defs, the solid mask resets to the NEW def default
    // and the shape/vertices rebuild from the new def (WGTW/CTWR tower
    // handlers rely on it during the game-start UpdateTransferZone
    // broadcast).
    // The horse-death pattern: Death() runs `ChangeDef(ID_Dead())` then
    // `SetAction("Dead")` (Horse.c4d Script.c). C++ ChangeDef swaps the
    // definition INLINE at the call site (C4Object.cpp:1205-1231, incl.
    // the SetAction(ActIdle) pre-reset :1214), so the following
    // SetAction("Dead") resolves against the NEW def's ActMap
    // (SetActionByName -> Def->ActMap). Deferring the swap past the
    // staged action apply validated "Dead" against the OLD def (no such
    // action -> default fallback) — the f147 wall: cpp horse action
    // "Dead" vs rust "Idle" (def DHRS in both).
    #[test]
    fn change_def_then_set_action_resolves_against_the_new_def_like_cpp() {
        let mut engine = Engine::with_seed(0);
        let live_script = r#"#strict
public func Death()
{
  ChangeDef(CRPS);
  SetAction("Dead");
  return(1);
}
"#;
        let mut live = Definition::from_script("HRSX", "Horse", live_script).expect("compiles");
        live.set_c4_callback_convention(true);
        live.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        live.configure_actions(
            None,
            HashMap::from([("Gallop".to_string(), ActionSpec::default().with_delay(1))]),
        );
        engine.register_definition(live).expect("registers");
        let mut corpse = Definition::from_script("CRPS", "Corpse", "#strict\n").expect("compiles");
        corpse.configure_actions(
            None,
            HashMap::from([("Dead".to_string(), ActionSpec::default().with_delay(3000))]),
        );
        engine.register_definition(corpse).expect("registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("HRSX")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 50))
                    .with_alive(true)
                    .with_action(ActionState::new("Gallop")),
            )
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");
        engine.objects[idx].state.energy = 30;

        // The kill: energy to zero -> AssignDeath -> Death() script
        // (C4Object.cpp:1363, :1173).
        engine
            .change_object_energy(idx, -30, 0, OWNER_NONE)
            .expect("energy change succeeds");
        engine.assign_death(idx, false).expect("death runs");

        let idx = engine.find_object_index(id).expect("exists");
        let object = &engine.objects[idx];
        assert_eq!(
            object.definition_id.as_str(),
            "CRPS",
            "ChangeDef swapped the definition"
        );
        assert_eq!(
            object.state.action.name, "Dead",
            "SetAction after ChangeDef resolves against the NEW def's ActMap \
             (C4Object.cpp:1205-1231 swaps inline; SetActionByName then finds \
             \"Dead\")"
        );
        assert!(!object.state.alive);
    }

    // FnFling requires an explicit target: `if (!pObj) return false;`
    // (C4Script.cpp:347-349) — NO fallback to the calling object (unlike
    // FnJump directly below it, :358-360). The horse's Tumbling()
    // StartCall runs `Fling(GetRider(), Random(5)-2, -3)` — with no
    // rider the call is a NO-OP in C++; self-targeting it launched the
    // riderless GoldRush horse up-right at its death frame (the f147
    // residual: rust (1,-3) dir Right vs cpp (-0.5,0) dir Left).
    #[test]
    fn fling_with_nil_target_is_a_no_op_like_cpp() {
        let script = r#"#strict
protected func Activity() { return(Fling(GetActionTarget(), 1, -3)); }
"#;
        let mut horse = Definition::from_script("HRSX", "Horse", script).expect("compiles");
        horse.set_c4_callback_convention(true);
        horse.configure_actions(
            None,
            HashMap::from([
                (
                    "Gallop".to_string(),
                    ActionSpec::default().with_delay(1).with_length(20).with_next("Gallop"),
                ),
                ("Tumble".to_string(), ActionSpec::default().with_delay(1)),
            ]),
        );
        horse.set_timer(1);
        horse.set_timer_call(Some("Activity".to_string()));
        let mut engine = Engine::with_seed(0);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(horse).expect("registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("HRSX")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 50))
                    .with_action(ActionState::new("Gallop")),
            )
            .expect("spawns");

        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("exists");
        let object = &engine.objects[idx];
        assert_eq!(
            object.fixed_velocity,
            FixedVec2::ZERO,
            "Fling(nil, 1, -3) must not launch the CALLER (C4Script.cpp:349)"
        );
        assert_eq!(
            object.state.action.name, "Gallop",
            "no Tumble transition from a nil-target Fling"
        );
    }

    #[test]
    fn shake_objects_flings_attached_living_object_with_cpp_rng_order() {
        // FnShakeObjects forwards the caller controller, then C4Game walks
        // master order, draws Random(3), draws Rnd3 only for an attached
        // non-MVehic living object, and calls Fling(Rnd3, 0, false, cause)
        // (C4Script.cpp:3104-3106; C4Game.cpp:1300-1314).
        let mut caller = Definition::from_script(
            "QUAK",
            "Quake",
            "#strict\npublic func Shake() { ShakeObjects(10, 10, 20); }\n",
        )
        .expect("caller script compiles");
        caller.set_category(CATEGORY_OBJECT);

        let mut target =
            Definition::from_script("CLNK", "Clonk", "#strict\n").expect("target script compiles");
        target.set_category(CATEGORY_LIVING | CATEGORY_OBJECT);
        target.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(2);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        let caller_id = engine
            .spawn_object(
                SpawnConfig::new("QUAK")
                    .with_position(Vector2::new(10, 10))
                    .with_owner(7),
            )
            .expect("caller spawns");
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_position(Vector2::new(12, 10))
                    .with_action(ActionState::new("Walk"))
                    .with_category(CATEGORY_LIVING | CATEGORY_OBJECT)
                    .with_alive(true),
            )
            .expect("target spawns");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        let initial_attach = CNAT_BOTTOM | CNAT_LEFT | CNAT_TOP;
        engine.objects[target_idx].state.mobile = false;
        engine.objects[target_idx].state.t_attach = initial_attach;
        engine.objects[target_idx].frame_t_attach = initial_attach;
        engine.objects[target_idx].state.shape_attach = ShapeAttachRecord {
            mat_valid: true,
            mat_vehicle: false,
            x: 12,
            y: 11,
            vtx: 0,
        };

        let mut expected_rng = engine.rng.clone();
        assert_eq!(expected_rng.random(3), 0, "fixture passes the shake gate");
        let expected_xdir = expected_rng.rnd3();
        let caller_idx = engine.find_object_index(caller_id).expect("caller exists");
        engine
            .call_object_function(caller_idx, "Shake", Vec::new())
            .expect("ShakeObjects executes");

        let target_idx = engine.find_object_index(target_id).expect("target remains");
        assert_eq!(engine.objects[target_idx].state.action.name, "Tumble");
        assert_eq!(
            engine.objects[target_idx].fixed_velocity,
            FixedVec2::new(itofix(expected_xdir), C4Fixed::ZERO)
        );
        assert_eq!(
            engine.objects[target_idx].state.direction,
            if expected_xdir < 0 {
                Direction::Right
            } else {
                Direction::Left
            },
            "C4Object::Fling passes the raw `(txdir < 0)` bool to SetDir"
        );
        assert!(
            !engine.objects[target_idx].state.mobile,
            "ObjectActionTumble does not change C4Object::Mobile"
        );
        assert_eq!(
            engine.objects[target_idx].state.t_attach, initial_attach,
            "ShakeObjects calls C4Object::Fling directly, so Tumble preserves t_attach"
        );
        assert_eq!(
            engine.objects[target_idx].frame_t_attach, initial_attach,
            "the current-frame attachment latch is preserved too"
        );
        assert_eq!(engine.objects[target_idx].last_energy_loss_cause, 7);
        assert_eq!(engine.rng, expected_rng);
    }

    #[test]
    fn fling_foreign_target_bypasses_override_and_clears_all_attach_bits() {
        // FnFling calls C4Object::Fling directly and only afterward clears
        // the complete Action.t_attach mask (C4Script.cpp:347-356). A target
        // definition's same-name script function must not intercept that
        // native object-method call.
        let mut caller = Definition::from_script(
            "FLCL",
            "Fling caller",
            "#strict\npublic func Throw(pTarget) { return Fling(pTarget, -2, -3); }\n",
        )
        .expect("caller script compiles");
        caller.set_category(CATEGORY_OBJECT);

        let target_script = r#"#strict
local override_hit;
public func Fling(pObj, iX, iY, iPrec, fAddSpeed) {
    override_hit = 1;
    return 0;
}
"#;
        let mut target = Definition::from_script("FLTG", "Fling target", target_script)
            .expect("target script compiles");
        target.set_category(CATEGORY_OBJECT);
        target.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_directions(2),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_directions(2),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        let caller_id = engine
            .spawn_object(
                SpawnConfig::new("FLCL")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(4),
            )
            .expect("caller spawns");
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("FLTG")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("target spawns");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        let initial_attach = CNAT_LEFT | CNAT_TOP | CNAT_BOTTOM;
        engine.objects[target_idx].state.mobile = false;
        engine.objects[target_idx].state.t_attach = initial_attach;
        engine.objects[target_idx].frame_t_attach = initial_attach;

        let caller_idx = engine.find_object_index(caller_id).expect("caller exists");
        let result = engine
            .call_object_function(
                caller_idx,
                "Throw",
                vec![compat::object_reference_value(target_id)],
            )
            .expect("native Fling executes");
        assert_eq!(result, Value::Bool(true));

        let target_idx = engine.find_object_index(target_id).expect("target remains");
        let object = &engine.objects[target_idx];
        assert_ne!(
            object.state.local_vars.get("override_hit"),
            Some(&Value::Int(1)),
            "the target's script-level Fling override must not run"
        );
        assert_eq!(object.state.action.name, "Tumble");
        assert_eq!(
            object.fixed_velocity,
            FixedVec2::new(itofix(-2), itofix(-3))
        );
        assert_eq!(object.state.direction, Direction::Right);
        assert!(
            !object.state.mobile,
            "the native Tumble branch preserves Mobile"
        );
        assert_eq!(object.state.t_attach, 0, "FnFling clears every attach bit");
        assert_eq!(
            object.frame_t_attach, 0,
            "FnFling also clears the current-frame latch"
        );
    }

    #[test]
    fn fling_add_speed_uses_cpp_bool_coercion() {
        // FnFling's fAddSpeed parameter is a native bool, converted through
        // C4Value::getBool. Int(2) must therefore take the same half-current-
        // speed arm as true (C4Script.cpp:348-356; C4Value.h:161,325-331).
        let mut caller = Definition::from_script(
            "FLBC",
            "Fling bool coercion caller",
            r#"#strict
public func Throw(pTarget, fAddSpeed) {
    return Fling(pTarget, 10, -10, 1, fAddSpeed);
}
"#,
        )
        .expect("caller script compiles");
        caller.set_category(CATEGORY_OBJECT);

        let mut target = Definition::from_script("FLBT", "Fling bool target", "#strict\n")
            .expect("target script compiles");
        target.set_category(CATEGORY_OBJECT);

        let mut engine = Engine::with_seed(19);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("FLBC").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let target_id = engine
            .spawn_object(SpawnConfig::new("FLBT").with_category(CATEGORY_OBJECT))
            .expect("fling target spawns");
        let initial_velocity = FixedVec2::new(
            C4Fixed::from_raw(147_456),
            C4Fixed::from_raw(-81_920),
        );
        let expected_velocity = FixedVec2::new(
            C4Fixed::from_raw(729_088),
            C4Fixed::from_raw(-696_320),
        );
        for flag in [Value::Int(2), Value::Bool(true)] {
            let target_index = engine
                .find_object_index(target_id)
                .expect("fling target exists");
            engine.objects[target_index].set_fixed_velocity(initial_velocity);
            let caller_index = engine
                .find_object_index(caller_id)
                .expect("caller exists");
            assert_eq!(
                engine
                    .call_object_function(
                        caller_index,
                        "Throw",
                        vec![compat::object_reference_value(target_id), flag],
                    )
                    .expect("native Fling executes"),
                Value::Bool(true)
            );
            let target_index = engine
                .find_object_index(target_id)
                .expect("fling target remains");
            assert_eq!(
                engine.objects[target_index].fixed_velocity, expected_velocity,
                "nonzero integer and true must both add half the current speed"
            );
        }
    }

    #[test]
    fn set_x_dir_and_tumble_fling_preserve_live_order_and_mobile() {
        // FnSetXDir writes xdir and Mobile=1 synchronously; ObjectActionTumble
        // then overwrites both velocity components but preserves Mobile
        // (C4Script.cpp:697-705; C4ObjectCom.cpp:74-80). Reversing the calls
        // leaves SetXDir's x component last while still keeping Mobile set.
        let caller_script = r#"#strict
public func SetThenFling(pTarget) {
    SetXDir(50, pTarget);
    return Fling(pTarget, -2, -3);
}

public func FlingThenSet(pTarget) {
    Fling(pTarget, -2, -3);
    return SetXDir(50, pTarget);
}
"#;
        let mut caller = Definition::from_script("FLOR", "Fling order", caller_script)
            .expect("caller script compiles");
        caller.set_category(CATEGORY_OBJECT);

        let mut target = Definition::from_script("FLOT", "Fling order target", "#strict\n")
            .expect("target script compiles");
        target.set_category(CATEGORY_OBJECT);
        target.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_directions(2),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_directions(2),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(17);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("FLOR").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let set_then_fling = engine
            .spawn_object(
                SpawnConfig::new("FLOT")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("set-then-fling target spawns");
        let fling_then_set = engine
            .spawn_object(
                SpawnConfig::new("FLOT")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("fling-then-set target spawns");
        for id in [set_then_fling, fling_then_set] {
            let index = engine.find_object_index(id).expect("target exists");
            engine.objects[index].state.mobile = false;
        }

        let caller_idx = engine.find_object_index(caller_id).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_idx,
                    "SetThenFling",
                    vec![compat::object_reference_value(set_then_fling)],
                )
                .expect("SetXDir then Fling executes"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(
                    caller_idx,
                    "FlingThenSet",
                    vec![compat::object_reference_value(fling_then_set)],
                )
                .expect("Fling then SetXDir executes"),
            Value::Bool(true)
        );

        let first = &engine.objects[engine
            .find_object_index(set_then_fling)
            .expect("first target remains")];
        assert_eq!(first.state.action.name, "Tumble");
        assert_eq!(
            first.fixed_velocity,
            FixedVec2::new(itofix(-2), itofix(-3)),
            "the later Tumble velocity replaces the earlier SetXDir component"
        );
        assert!(
            first.state.mobile,
            "Tumble preserves the Mobile=1 written by SetXDir"
        );

        let second = &engine.objects[engine
            .find_object_index(fling_then_set)
            .expect("second target remains")];
        assert_eq!(second.state.action.name, "Tumble");
        assert_eq!(
            second.fixed_velocity,
            FixedVec2::new(itofix(5), itofix(-3)),
            "the later SetXDir component replaces only Tumble's x velocity"
        );
        assert!(
            second.state.mobile,
            "SetXDir sets Mobile=1 after Tumble preserved the false entry value"
        );
    }

    #[test]
    fn fling_incomplete_target_coerces_tumble_to_idle_without_start_call() {
        // C4Object::SetAction accepts the requested action but coerces it to
        // ActIdle when Con<FullCon and Def->IncompleteActivity is false
        // (C4Object.cpp:4127-4146). ObjectActionTumble therefore returns true
        // and still assigns xdir/ydir, but no Tumble StartCall runs
        // (C4ObjectCom.cpp:74-79; C4ActionCallbacks.h:29).
        let mut caller = Definition::from_script(
            "FIIC",
            "Incomplete fling caller",
            "#strict\npublic func Throw(pTarget) { return Fling(pTarget, 2, -3); }\n",
        )
        .expect("caller script compiles");
        caller.set_category(CATEGORY_OBJECT);

        let target_script = r#"#strict
local tumble_started;
protected func TumbleStart()
{
    tumble_started = 1;
    return 1;
}
"#;
        let mut target = Definition::from_script("FIIT", "Incomplete fling target", target_script)
            .expect("target script compiles");
        target.set_category(CATEGORY_OBJECT);
        target.set_incomplete_activity(false);
        target.set_c4_callback_convention(true);
        target.configure_actions(
            None,
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_start_call("TumbleStart")
                        .with_directions(2),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(19);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("FIIC").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("FIIT")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Walk"))
                    .with_construction(FULL_CON / 2)
                    .with_loaded(true),
            )
            .expect("incomplete target loads");

        let caller_idx = engine.find_object_index(caller_id).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_idx,
                    "Throw",
                    vec![compat::object_reference_value(target_id)],
                )
                .expect("native Fling executes"),
            Value::Bool(true)
        );

        let target_idx = engine.find_object_index(target_id).expect("target remains");
        let object = &engine.objects[target_idx];
        assert_eq!(
            object.state.action.name, "Idle",
            "SetAction must coerce Tumble to ActIdle on an incomplete target"
        );
        assert_ne!(
            object.state.local_vars.get("tumble_started"),
            Some(&Value::Int(1)),
            "the coerced idle action has no Tumble StartCall"
        );
        assert_eq!(
            object.fixed_velocity,
            FixedVec2::new(itofix(2), itofix(-3)),
            "ObjectActionTumble still assigns velocity after SetAction returns true"
        );
    }

    #[test]
    fn fling_tumble_start_change_def_stops_old_abort_and_keeps_new_def_idle() {
        // SetAction dispatches the new StartCall before the old AbortCall,
        // then stops the sequence if StartCall changed Def
        // (C4Object.cpp:4171-4198; C4ActionCallbacks.h:29-31). ChangeDef
        // itself enforces ActIdle before installing the new definition
        // (C4Object.cpp:1217-1225), so the outer Tumble request must not be
        // replayed against the new definition during the Rust outcome fold.
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let calls = Arc::clone(&calls);
            hooks.set_on_call(move |name, _args| {
                if matches!(name, "TumbleStart" | "OldAbort") {
                    calls.lock().unwrap().push(name.to_string());
                }
            });
        }

        let mut caller = Definition::from_script(
            "FCDC",
            "ChangeDef fling caller",
            "#strict\npublic func Throw(pTarget) { return Fling(pTarget, -2, -3); }\n",
        )
        .expect("caller script compiles");
        caller.set_category(CATEGORY_OBJECT);

        let old_script = r#"#strict
protected func TumbleStart()
{
    ChangeDef(FCDN);
    return 1;
}

protected func OldAbort(int old_phase)
{
    return 1;
}
"#;
        let mut old = Definition::from_script("FCDO", "Old fling definition", old_script)
            .expect("old target script compiles");
        old.set_category(CATEGORY_OBJECT);
        old.set_c4_callback_convention(true);
        old.set_debugger_hooks(hooks);
        old.configure_actions(
            None,
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_abort_call("OldAbort"),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_start_call("TumbleStart")
                        .with_directions(2),
                ),
            ]),
        );

        let mut new = Definition::from_script("FCDN", "New fling definition", "#strict\n")
            .expect("new target script compiles");
        new.set_category(CATEGORY_OBJECT);
        // Keep a Tumble entry on the new definition so an incorrectly
        // replayed pending action is visible instead of reconciling away.
        new.configure_actions(
            None,
            HashMap::from([(
                "Tumble".to_string(),
                ActionSpec::default()
                    .with_procedure("FLIGHT")
                    .with_directions(2),
            )]),
        );

        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(new)
            .expect("new definition registers");
        engine
            .register_definition(old)
            .expect("old definition registers");
        engine
            .register_definition(caller)
            .expect("caller registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("FCDC").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("FCDO")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Walk"))
                    .with_loaded(true),
            )
            .expect("target loads");

        let caller_idx = engine.find_object_index(caller_id).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_idx,
                    "Throw",
                    vec![compat::object_reference_value(target_id)],
                )
                .expect("native Fling executes"),
            Value::Bool(true)
        );

        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["TumbleStart"],
            "ChangeDef in StartCall must suppress the old action's AbortCall"
        );
        let target_idx = engine.find_object_index(target_id).expect("target remains");
        let object = &engine.objects[target_idx];
        assert_eq!(object.definition_id, "FCDN");
        assert_eq!(
            object.state.action.name, "Idle",
            "ChangeDef's enforced ActIdle must win over the outer Tumble request"
        );
    }

    #[test]
    fn fling_disabled_tumble_refreshes_ocf_before_start_and_after_foreign_fold() {
        // SetAction calls SetOCF after changing Action and before dispatching
        // StartCall (C4Object.cpp:4142-4169). A Disabled action therefore
        // clears OCF_FightReady and OCF_Collection before TumbleStart reads
        // GetOCF (C4Object.cpp:593-610). The cached value must remain updated
        // after the foreign-target outcome folds as well.
        let mut caller = Definition::from_script(
            "FOCC",
            "OCF fling caller",
            "#strict\npublic func Throw(pTarget) { return Fling(pTarget, 2, -3); }\n",
        )
        .expect("caller script compiles");
        caller.set_category(CATEGORY_OBJECT);

        let target_script = r#"#strict
local ocf_seen_in_tumble_start;
protected func TumbleStart()
{
    ocf_seen_in_tumble_start = GetOCF();
    return 1;
}
"#;
        let mut target = Definition::from_script("FOCT", "OCF fling target", target_script)
            .expect("target script compiles");
        target.set_category(CATEGORY_LIVING | CATEGORY_OBJECT);
        target.set_collection_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        target.set_c4_callback_convention(true);
        target.configure_actions(
            None,
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_start_call("TumbleStart")
                        .with_disabled(true)
                        .with_directions(2),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(29);
        engine
            .register_definition(target)
            .expect("target registers");
        engine
            .register_definition(caller)
            .expect("caller registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("FOCC").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("FOCT")
                    .with_category(CATEGORY_LIVING | CATEGORY_OBJECT)
                    .with_alive(true)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("target spawns");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        let gated_bits = ocf::FIGHT_READY | ocf::COLLECTION;
        assert_eq!(
            engine.objects[target_idx].state.ocf & gated_bits,
            gated_bits,
            "the enabled Walk action starts fight-ready and collectible"
        );

        let caller_idx = engine.find_object_index(caller_id).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(
                    caller_idx,
                    "Throw",
                    vec![compat::object_reference_value(target_id)],
                )
                .expect("native Fling executes"),
            Value::Bool(true)
        );

        let target_idx = engine.find_object_index(target_id).expect("target remains");
        let object = &engine.objects[target_idx];
        let seen = object
            .state
            .local_vars
            .get("ocf_seen_in_tumble_start")
            .and_then(|value| match value {
                Value::Int(value) => Some(*value as u32),
                _ => None,
            })
            .expect("TumbleStart records GetOCF");
        assert_eq!(
            seen & gated_bits,
            0,
            "StartCall must observe SetOCF for the disabled Tumble action"
        );
        assert_eq!(object.state.action.name, "Tumble");
        assert_eq!(
            object.state.ocf & gated_bits,
            0,
            "the foreign target's cached OCF must stay refreshed after folding"
        );
    }

    #[test]
    fn shake_objects_matches_cpp_master_order_gate_ledger() {
        // Frozen C++ oracle fixture (C4Game.cpp:1300-1314), seed 2 after
        // Randomize3. Inactive objects live outside Game.Objects and consume
        // no draw; eligible Random(3) draws are 0,0,2,0, so only the final
        // attached MNone object reaches Rnd3.
        let mut caller = Definition::from_script(
            "SHKO",
            "Shake oracle",
            "#strict\npublic func Shake() { return ShakeObjects(10, 10, 20); }\n",
        )
        .expect("oracle caller compiles");
        caller.set_category(CATEGORY_OBJECT);
        let mut target = Definition::from_script("SHKT", "Shake target", "#strict\n")
            .expect("oracle target compiles");
        target.set_category(CATEGORY_LIVING | CATEGORY_OBJECT);

        let mut engine = Engine::with_seed(2);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        let caller_id = engine
            .spawn_object(
                SpawnConfig::new("SHKO")
                    .with_custom_name("caller")
                    .with_position(Vector2::new(10, 10))
                    .with_owner(7)
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("caller spawns");

        struct Row {
            name: &'static str,
            status: ObjectStatus,
            position: Vector2,
            contained: bool,
            t_attach: u32,
            mat_valid: bool,
            mat_vehicle: bool,
        }
        let rows = [
            Row {
                name: "deleted",
                status: ObjectStatus::Deleted,
                position: Vector2::new(10, 10),
                contained: false,
                t_attach: CNAT_BOTTOM,
                mat_valid: true,
                mat_vehicle: false,
            },
            Row {
                name: "boundary_unattached",
                status: ObjectStatus::Normal,
                position: Vector2::new(-10, 30),
                contained: false,
                t_attach: 0,
                mat_valid: true,
                mat_vehicle: false,
            },
            Row {
                name: "contained",
                status: ObjectStatus::Normal,
                position: Vector2::new(10, 10),
                contained: true,
                t_attach: CNAT_BOTTOM,
                mat_valid: true,
                mat_vehicle: false,
            },
            Row {
                name: "vehicle",
                status: ObjectStatus::Normal,
                position: Vector2::new(10, 10),
                contained: false,
                t_attach: CNAT_BOTTOM,
                mat_valid: true,
                mat_vehicle: true,
            },
            Row {
                name: "inactive_attached",
                status: ObjectStatus::Inactive,
                position: Vector2::new(10, 10),
                contained: false,
                t_attach: CNAT_BOTTOM,
                mat_valid: true,
                mat_vehicle: false,
            },
            Row {
                name: "out_of_range",
                status: ObjectStatus::Normal,
                position: Vector2::new(31, 10),
                contained: false,
                t_attach: CNAT_BOTTOM,
                mat_valid: true,
                mat_vehicle: false,
            },
            Row {
                name: "attached_gate_rejected",
                status: ObjectStatus::Normal,
                position: Vector2::new(10, 10),
                contained: false,
                t_attach: CNAT_BOTTOM,
                mat_valid: true,
                mat_vehicle: false,
            },
            Row {
                name: "attached_mnone",
                status: ObjectStatus::Normal,
                position: Vector2::new(10, 10),
                contained: false,
                t_attach: CNAT_BOTTOM,
                mat_valid: false,
                mat_vehicle: false,
            },
        ];

        let mut row_ids = HashMap::new();
        for (row_index, row) in rows.iter().enumerate() {
            let raw_x = 1_000 + row_index as i32;
            let raw_y = -(2_000 + row_index as i32);
            let id = engine
                .spawn_object(
                    SpawnConfig::new("SHKT")
                        .with_custom_name(row.name)
                        .with_position(row.position)
                        .with_fixed_velocity(FixedVec2::new(
                            C4Fixed::from_raw(raw_x),
                            C4Fixed::from_raw(raw_y),
                        ))
                        .with_category(CATEGORY_LIVING | CATEGORY_OBJECT)
                        .with_alive(false),
                )
                .expect("oracle row spawns");
            let index = engine.find_object_index(id).expect("oracle row exists");
            engine.objects[index].state.status = row.status;
            engine.objects[index].state.container = row.contained.then_some(caller_id);
            engine.objects[index].state.t_attach = row.t_attach;
            engine.objects[index].frame_t_attach = row.t_attach;
            engine.objects[index].state.shape_attach = ShapeAttachRecord {
                mat_valid: row.mat_valid,
                mat_vehicle: row.mat_vehicle,
                x: row.position.x,
                y: row.position.y,
                vtx: 0,
            };
            row_ids.insert(row.name, id);
        }

        let master_order = [
            row_ids["deleted"],
            row_ids["boundary_unattached"],
            row_ids["contained"],
            row_ids["vehicle"],
            caller_id,
            row_ids["inactive_attached"],
            row_ids["out_of_range"],
            row_ids["attached_gate_rejected"],
            row_ids["attached_mnone"],
        ];
        engine.exec_list = master_order.iter().rev().copied().collect();
        assert_eq!(
            (engine.rng.count, engine.rng.hold, engine.rng.rnd3_ptr()),
            (500, 3_424_448_854, 0)
        );

        let before = rows
            .iter()
            .map(|row| {
                let id = row_ids[row.name];
                let index = engine.find_object_index(id).expect("row exists");
                (row.name, engine.objects[index].fixed_velocity)
            })
            .collect::<HashMap<_, _>>();
        let caller_idx = engine.find_object_index(caller_id).expect("caller exists");
        let result = engine
            .call_object_function(caller_idx, "Shake", Vec::new())
            .expect("ShakeObjects executes");
        assert_eq!(result, Value::Nil);
        assert_eq!(
            (engine.rng.count, engine.rng.hold, engine.rng.rnd3_ptr()),
            (504, 1_287_806_202, 1)
        );

        for row in &rows[..rows.len() - 1] {
            let index = engine
                .find_object_index(row_ids[row.name])
                .expect("unchanged row remains");
            assert_eq!(
                engine.objects[index].fixed_velocity, before[row.name],
                "{} must not reach Fling",
                row.name
            );
        }
        let survivor = engine
            .find_object_index(row_ids["attached_mnone"])
            .expect("survivor remains");
        assert_eq!(
            engine.objects[survivor].fixed_velocity,
            FixedVec2::new(C4Fixed::from_raw(65_536), C4Fixed::ZERO)
        );
        assert!(engine.objects[survivor].state.mobile);
        assert_eq!(engine.objects[survivor].state.t_attach, 0);
        assert_eq!(engine.objects[survivor].frame_t_attach, 0);
        assert_eq!(engine.objects[survivor].state.controller, 7);
    }

    #[test]
    fn change_def_swaps_definition_in_place_like_cpp() {
        let mut engine = Engine::with_seed(0);
        let mut old_def = simple_definition("OLDD");
        old_def.set_shape_rect(Some(DefinitionRect::new(-4, -4, 8, 8)));
        old_def.configure_actions(
            None,
            HashMap::from([("Spin".to_string(), ActionSpec::default().with_delay(1))]),
        );
        engine.register_definition(old_def).expect("old registers");
        let mut new_def = simple_definition("NEWD");
        new_def.set_shape_rect(Some(DefinitionRect::new(-8, -2, 16, 4)));
        new_def.set_shape_vertices(vec![ObjectVertex {
            x: 0,
            y: 3,
            cnat: 0,
            friction: 77,
        }]);
        engine.register_definition(new_def).expect("new registers");
        let caller = Definition::from_script(
            "CALL",
            "Caller",
            "#strict\nfunc Swap(pObj) { return(ChangeDef(NEWD, pObj)); }\n",
        )
        .expect("caller compiles");
        engine
            .register_definition(caller)
            .expect("caller registers");

        let target = engine
            .spawn_object(
                SpawnConfig::new("OLDD")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 50))
                    .with_owner(3)
                    .with_action(ActionState::new("Spin")),
            )
            .expect("target spawns");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let idx = engine.find_object_index(caller_id).expect("caller exists");
        let target_value = compat::object_reference_value(target);
        engine
            .call_object_function(idx, "Swap", vec![target_value])
            .expect("swap runs");

        let idx = engine.find_object_index(target).expect("target survives");
        let object = &engine.objects[idx];
        assert_eq!(object.definition_id.as_str(), "NEWD", "definition swapped");
        // Spawn bottom-growth put the center at 50-(8-4)=46; ChangeDef
        // must not move it (C4Object::ChangeDef never touches x/y).
        assert_eq!(object.state.position, Vector2::new(50, 46), "position kept");
        assert_eq!(object.state.owner, 3, "owner kept");
        assert_eq!(
            object.state.action.name,
            clonk_engine::action::DEFAULT_ACTION_NAME,
            "SetAction(ActIdle) at def change (C4Object.cpp:1190)"
        );
        assert_eq!(
            object.state.vertices.first().map(|v| v.friction),
            Some(77),
            "shape/vertices rebuilt from the NEW def (UpdateFace)"
        );
    }

    #[test]
    fn foreign_target_change_def_cannot_be_shadowed_by_target_script() {
        // The caller has already resolved the engine FnChangeDef. C++ then
        // invokes pObj->ChangeDef directly; it does not re-resolve a function
        // named ChangeDef on the explicit target object.
        let caller = Definition::from_script(
            "CALL",
            "Caller",
            "#strict\nfunc Swap(pObj) { return(ChangeDef(NEWD, pObj)); }\n",
        )
        .expect("caller compiles");
        let old = Definition::from_script(
            "OLDD",
            "Old",
            r#"#strict
local shadow_calls;
func ChangeDef(unused_definition) { shadow_calls += 1; return(0); }
"#,
        )
        .expect("old definition compiles");
        let new = simple_definition("NEWD");

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine.register_definition(old).expect("old registers");
        engine.register_definition(new).expect("new registers");

        let target = engine
            .spawn_object(SpawnConfig::new("OLDD"))
            .expect("target spawns");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let result = engine
            .call_object_function(
                caller_index,
                "Swap",
                vec![compat::object_reference_value(target)],
            )
            .expect("foreign ChangeDef runs");

        assert_eq!(result, Value::Bool(true));
        let target = engine.object_snapshot(target).expect("target remains");
        assert_eq!(target.definition_id, "NEWD");
        assert_ne!(target.local_vars.get("shadow_calls"), Some(&Value::Int(1)));
    }

    #[test]
    fn change_def_forced_idle_preserves_abort_callback_action_payload_on_both_paths(
    ) -> Result<(), EngineError> {
        // ChangeDef first performs an ordinary SetAction(ActIdle), including
        // Old's AbortCall. The callback may select a different action and
        // write its payload, but ChangeDef then overwrites Action.Act alone:
        // the final action is Idle while Time/Data/Phase survive untouched.
        for via_burn_turn_to in [false, true] {
            let path = if via_burn_turn_to {
                "native BurnTurnTo"
            } else {
                "script host"
            };
            let mut old = Definition::from_script(
                "OLDD",
                "Old",
                r#"#strict
local abort_calls;
protected func OldAbort(int old_phase)
{
    abort_calls += 1;
    SetAction("Other");
    SetActionData(73);
    SetPhase(4);
    return(1);
}
public func Swap() { return(ChangeDef(NEWD)); }
"#,
            )?;
            old.set_c4_callback_convention(true);
            old.set_burn_turn_to(Some("NEWD".to_string()));
            old.configure_actions(
                None,
                HashMap::from([
                    (
                        "Old".to_string(),
                        ActionSpec::default()
                            .with_length(10)
                            .with_abort_call("OldAbort"),
                    ),
                    ("Other".to_string(), ActionSpec::default().with_length(10)),
                ]),
            );

            let mut engine = Engine::with_seed(41);
            engine.register_definition(old)?;
            engine.register_definition(simple_definition("NEWD"))?;

            let mut action = ActionState::new("Old");
            action.time = 99;
            action.data = 11;
            action.phase = 7;
            let object_id = engine.spawn_object(
                SpawnConfig::new("OLDD")
                    .with_action(action)
                    .with_local_vars(HashMap::from([("abort_calls".to_string(), Value::Int(0))])),
            )?;

            let object_index = engine.find_object_index(object_id).expect("object exists");
            if via_burn_turn_to {
                assert!(engine.incinerate_object(object_index, 1, false, None)?);
            } else {
                assert_eq!(
                    engine.call_object_function(object_index, "Swap", Vec::new())?,
                    Value::Bool(true)
                );
            }

            let object_index = engine.find_object_index(object_id).expect("object remains");
            let object = &engine.objects[object_index];
            assert_eq!(object.definition_id, "NEWD", "{path}: definition swaps");
            assert_eq!(
                object.state.local_vars.get("abort_calls"),
                Some(&Value::Int(1)),
                "{path}: the old action AbortCall runs"
            );
            assert_eq!(
                object.state.action.name,
                clonk_engine::action::DEFAULT_ACTION_NAME,
                "{path}: unconditional Action.Act=ActIdle wins"
            );
            assert_eq!(
                object.state.action.time, 0,
                "{path}: the AbortCall's SetAction time reset survives"
            );
            assert_eq!(
                object.state.action.data, 73,
                "{path}: callback-written Action.Data survives"
            );
            assert_eq!(
                object.state.action.phase, 4,
                "{path}: callback-written Action.Phase survives"
            );
        }
        Ok(())
    }

    #[test]
    fn change_def_from_idle_preserves_time_but_resets_phase_on_both_paths(
    ) -> Result<(), EngineError> {
        // SetAction(ActIdle) only resets Action.Time when the action slot
        // changes. Starting from built-in Idle therefore preserves nonzero
        // Time (and Data), while SetAction's unconditional Phase/PhaseDelay
        // reset still runs before ChangeDef's raw Action.Act=ActIdle write.
        for via_burn_turn_to in [false, true] {
            let path = if via_burn_turn_to {
                "native BurnTurnTo"
            } else {
                "script host"
            };
            let mut old = Definition::from_script(
                "OLDD",
                "Old",
                "#strict\npublic func Swap() { return(ChangeDef(NEWD)); }\n",
            )?;
            old.set_burn_turn_to(Some("NEWD".to_string()));

            let mut engine = Engine::with_seed(43);
            engine.register_definition(old)?;
            engine.register_definition(simple_definition("NEWD"))?;
            let object_id = engine.spawn_object(SpawnConfig::new("OLDD"))?;
            let object_index = engine.find_object_index(object_id).expect("object exists");
            {
                let action = &mut engine.objects[object_index].state.action;
                assert_eq!(action.name, clonk_engine::action::DEFAULT_ACTION_NAME);
                action.time = 77;
                action.data = 19;
                action.phase = 5;
                action.ticks = 3;
            }

            if via_burn_turn_to {
                assert!(engine.incinerate_object(object_index, 1, false, None)?);
            } else {
                assert_eq!(
                    engine.call_object_function(object_index, "Swap", Vec::new())?,
                    Value::Bool(true)
                );
            }

            let object_index = engine.find_object_index(object_id).expect("object remains");
            let object = &engine.objects[object_index];
            assert_eq!(object.definition_id, "NEWD", "{path}: definition swaps");
            assert_eq!(
                object.state.action.name,
                clonk_engine::action::DEFAULT_ACTION_NAME,
                "{path}: action remains built-in Idle"
            );
            assert_eq!(
                object.state.action.time, 77,
                "{path}: same-slot SetAction preserves Action.Time"
            );
            assert_eq!(
                object.state.action.data, 19,
                "{path}: same-procedure SetAction preserves Action.Data"
            );
            assert_eq!(
                object.state.action.phase, 0,
                "{path}: SetAction still clears Action.Phase"
            );
            assert_eq!(
                object.state.action.ticks, 0,
                "{path}: SetAction still clears Action.PhaseDelay"
            );
        }
        Ok(())
    }

    #[test]
    fn change_def_veto_after_abort_enter_keeps_the_pre_swap_contents_link(
    ) -> Result<(), EngineError> {
        // ChangeDef saves HOME, silently exits it, and runs Old's AbortCall.
        // That callback enters STAY while the old definition is still live,
        // so stContents places the object in OLDD's sorted cluster. The new
        // definition then vetoes the attempted return to HOME before Enter
        // can exit STAY. No post-fold ChangeDef marker may reappend the object.
        let mut old = Definition::from_script(
            "OLDD",
            "Old",
            r#"#strict
local stay;
protected func OldAbort(int old_phase) { return(Enter(stay)); }
public func Swap() { return(ChangeDef(NEWD)); }
"#,
        )?;
        old.set_c4_callback_convention(true);
        old.configure_actions(
            None,
            HashMap::from([(
                "Old".to_string(),
                ActionSpec::default().with_abort_call("OldAbort"),
            )]),
        );
        let mut new = Definition::from_script(
            "NEWD",
            "New",
            "#strict\nprotected func RejectEntrance(pContainer) { return(1); }\n",
        )?;
        new.set_c4_callback_convention(true);

        let mut engine = Engine::with_seed(42);
        engine.register_definition(simple_definition("HOME"))?;
        engine.register_definition(simple_definition("STAY"))?;
        engine.register_definition(old)?;
        engine.register_definition(new)?;

        let home = engine.spawn_object(SpawnConfig::new("HOME"))?;
        let stay = engine.spawn_object(SpawnConfig::new("STAY"))?;
        let peer = engine.spawn_object(
            SpawnConfig::new("OLDD")
                .with_category(CATEGORY_OBJECT)
                .with_container(stay),
        )?;
        let changed = engine.spawn_object(
            SpawnConfig::new("OLDD")
                .with_category(CATEGORY_OBJECT)
                .with_container(home)
                .with_action(ActionState::new("Old"))
                .with_local_vars(HashMap::from([(
                    "stay".to_string(),
                    object_reference_value(stay),
                )])),
        )?;

        let changed_index = engine.find_object_index(changed).expect("object exists");
        assert_eq!(
            engine.call_object_function(changed_index, "Swap", Vec::new())?,
            Value::Bool(true)
        );

        let changed_index = engine.find_object_index(changed).expect("object remains");
        assert_eq!(engine.objects[changed_index].definition_id, "NEWD");
        assert_eq!(engine.objects[changed_index].state.container, Some(stay));
        let home_index = engine.find_object_index(home).expect("home remains");
        assert!(engine.objects[home_index].state.contents.is_empty());
        let stay_index = engine.find_object_index(stay).expect("stay remains");
        assert_eq!(
            engine.objects[stay_index].state.contents,
            vec![changed, peer],
            "the old-definition Enter link stays in place after the saved-container veto"
        );
        Ok(())
    }

    // DFA_CONNECT (C4Object.cpp:5341-5420): a Line object's first vertex
    // tracks Action.Target and its last vertex tracks Action.Target2 —
    // C4D_Line_Vertex connects to the target's vertex (index from the
    // numbered Local[2]/Local[3], default 0), LineIntersect=1 assigns the
    // ABSOLUTE point directly. Broken targets fire LineBreak + removal.
    #[test]
    fn connect_lines_track_their_targets_like_cpp() {
        let mut engine = Engine::with_seed(0);
        let mut beam = Definition::from_script(
            "BEAM",
            "Beam",
            "#strict\npublic func RemoveEndpoint(object endpoint) { return RemoveObject(endpoint); }\n",
        )
        .expect("compiles");
        beam.set_line(8); // C4D_Line_Vertex
        beam.set_line_intersect(1);
        beam.set_shape_vertices(vec![
            ObjectVertex {
                x: 0,
                y: 0,
                cnat: 0,
                friction: 0,
            },
            ObjectVertex {
                x: 0,
                y: 0,
                cnat: 0,
                friction: 0,
            },
        ]);
        beam.configure_actions(
            None,
            HashMap::from([(
                "Connect".to_string(),
                ActionSpec::default()
                    .with_procedure("CONNECT")
                    .with_delay(10)
                    .with_length(1)
                    .with_next("Connect"),
            )]),
        );
        engine.register_definition(beam).expect("registers");
        let mut anchor_def =
            Definition::from_script("ANCR", "Anchor", "#strict\n").expect("compiles");
        anchor_def.set_shape_vertices(vec![
            ObjectVertex::new(-3, -5).with_friction(50),
            ObjectVertex::new(6, -7),
            ObjectVertex::new(9, 4),
        ]);
        engine.register_definition(anchor_def).expect("registers");

        let horse = engine
            .spawn_object(
                SpawnConfig::new("ANCR")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(77, 250)),
            )
            .expect("spawns");
        let wagon = engine
            .spawn_object(
                SpawnConfig::new("ANCR")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(28, 250)),
            )
            .expect("spawns");
        let beam_id = engine
            .spawn_object(
                SpawnConfig::new("BEAM")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(28, 250))
                    .with_action(ActionState::new("Connect"))
                    .with_local_vars(HashMap::from([
                        ("__local_2".to_string(), Value::Int(1)),
                        ("__local_3".to_string(), Value::Int(2)),
                    ])),
            )
            .expect("beam spawns");

        // SetAction("Connect", horse, wagon) like CHBM::Connect.
        engine
            .apply_object_update(
                beam_id,
                ObjectUpdate {
                    action: Some(
                        ActionUpdate::default()
                            .with_name("Connect")
                            .with_force(true)
                            .with_target(Some(horse))
                            .with_target2(Some(wagon)),
                    ),
                    ..Default::default()
                },
            )
            .expect("action set");

        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(beam_id).expect("beam exists");
        let vertices = &engine.objects[idx].state.vertices;
        assert_eq!(
            (vertices[0].x, vertices[0].y),
            (83, 243),
            "first vertex = Target position + its Local[2]-selected vertex"
        );
        assert_eq!(
            (vertices[1].x, vertices[1].y),
            (37, 254),
            "last vertex = Target2 position + its Local[3]-selected vertex"
        );

        // GetVertexX/Y return zero for negative and too-large indices
        // (C4Shape.cpp:409-419), so invalid locals attach at object origin.
        engine.objects[idx]
            .state
            .local_vars
            .insert("__local_2".to_string(), Value::Int(-1));
        engine.objects[idx]
            .state
            .local_vars
            .insert("__local_3".to_string(), Value::Int(99));
        engine.tick_without_snapshot().expect("invalid vertex indices tick");
        let idx = engine.find_object_index(beam_id).expect("beam exists");
        let vertices = &engine.objects[idx].state.vertices;
        assert_eq!((vertices[0].x, vertices[0].y), (77, 250));
        assert_eq!((vertices[1].x, vertices[1].y), (28, 250));

        engine
            .apply_object_update(
                horse,
                ObjectUpdate::new().with_status(ObjectStatus::Inactive),
            )
            .expect("deactivate first retained endpoint");
        engine
            .apply_object_update(
                wagon,
                ObjectUpdate::new().with_status(ObjectStatus::Inactive),
            )
            .expect("deactivate second retained endpoint");
        engine
            .tick_without_snapshot()
            .expect("inactive endpoints remain connected");
        assert!(
            engine.find_object_index(beam_id).is_some(),
            "C4OS_INACTIVE action targets retain their native pointers"
        );

        // Real AssignRemoval reaches Game.ClearPointers synchronously. Do
        // not use the low-level mark_destroyed test seam here: it deliberately
        // creates a status-zero tombstone before ClearPointers, during which
        // native raw Action.Target references remain usable.
        let beam_idx = engine.find_object_index(beam_id).expect("beam exists");
        assert_eq!(
            engine
                .call_object_function(
                    beam_idx,
                    "RemoveEndpoint",
                    vec![object_reference_value(horse)],
                )
                .expect("endpoint removal succeeds"),
            Value::Bool(true)
        );
        engine.tick_without_snapshot().expect("tick");
        assert!(
            engine.find_object_index(beam_id).is_none(),
            "broken line fires LineBreak and removes itself (C4Object.cpp:5347-5354)"
        );
    }

    #[test]
    fn resource_definition_preserves_line_core_fields() {
        // C4Def::CompileFunc reads both fields directly from DefCore
        // (src/C4Def.cpp:319-333, 410). Runtime definitions must not drop
        // them between resource parsing and C4Object execution/rendering.
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let group = clonk_resources::Group::open(repository.join(
            "content/Objects.c4d/Structures.c4d/Lines.c4d/PowerLine.c4d",
        ))
        .expect("open shipped PWRL definition");
        let resource =
            ResourceDefinitionData::load(&group).expect("load shipped PWRL definition");
        let definition =
            Definition::from_resource(&resource).expect("compile shipped PWRL definition");

        assert_eq!(definition.line(), resource.core.line);
        assert_eq!(definition.line_intersect(), resource.core.line_intersect);
        assert_ne!(definition.line(), 0, "PWRL is a typed line definition");

        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("register shipped PWRL definition");
        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.definition_lines.get("PWRL"),
            Some(&DefinitionLineMetadata {
                line: resource.core.line,
                line_intersect: resource.core.line_intersect,
            }),
            "frontend snapshot exposes the C4Def line metadata"
        );

        let mut legacy_json = serde_json::to_value(&snapshot).expect("snapshot serializes");
        legacy_json
            .as_object_mut()
            .expect("snapshot is an object")
            .remove("definition_lines");
        let legacy: SimulationSnapshot =
            serde_json::from_value(legacy_json).expect("pre-line-metadata snapshot decodes");
        assert!(legacy.definition_lines.is_empty());
    }

    #[test]
    fn shipped_power_line_inserts_first_cpp_bend_around_solid_pixel() {
        // The shipped PWRL definition is a two-vertex, LineIntersect=0
        // CONNECT line. C4Shape::LineConnect probes the moved endpoint to
        // its neighbour, takes the first blocked pixel, then tests the
        // 4-pixel candidate square in x-major/y-major order
        // (src/C4Shape.cpp:273-326). For the single blocker at (5,5), the
        // first viable C++ bend is therefore (3,3).
        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let group = clonk_resources::Group::open(repository.join(
            "content/Objects.c4d/Structures.c4d/Lines.c4d/PowerLine.c4d",
        ))
        .expect("open shipped PWRL definition");
        let resource =
            ResourceDefinitionData::load(&group).expect("load shipped PWRL definition");

        let mut engine = Engine::with_seed(0);
        let mut power_line =
            Definition::from_resource(&resource).expect("compile shipped PWRL definition");
        power_line.set_line(resource.core.line);
        power_line.set_line_intersect(resource.core.line_intersect);
        engine
            .register_definition(power_line)
            .expect("register shipped PWRL definition");
        engine
            .register_definition(
                Definition::from_script("ANCR", "Line anchor", "#strict\n")
                    .expect("compile anchor definition"),
            )
            .expect("register anchor definition");

        let mut pixels = vec![0_u8; 12 * 10];
        pixels[5 * 12 + 5] = 1;
        let grid = landscape::PixelGrid::new(
            12,
            10,
            pixels,
            vec![0, 100],
            vec![None; 2],
            vec![None; 2],
        );
        let mut landscape = Landscape::new(12, vec![10; 12]).expect("landscape builds");
        landscape.set_pixel_grid(grid);
        landscape.set_world_height(10);
        engine.set_landscape(landscape);

        let first = engine
            .spawn_object(
                SpawnConfig::new("ANCR")
                    .with_category(CATEGORY_STATIC_BACK)
                    .with_position(Vector2::new(0, 5)),
            )
            .expect("first anchor spawns");
        let second = engine
            .spawn_object(
                SpawnConfig::new("ANCR")
                    .with_category(CATEGORY_STATIC_BACK)
                    .with_position(Vector2::new(10, 5)),
            )
            .expect("second anchor spawns");
        let mut action = ActionState::new("Connect");
        action.target = Some(first);
        action.target2 = Some(second);
        let line = engine
            .spawn_object(
                SpawnConfig::new("PWRL")
                    // Keep the shipped line/action definition, but execute it
                    // as a synthetic active object so this unit oracle does
                    // not depend on loaded StaticBack scheduling.
                    .with_category(CATEGORY_OBJECT)
                    .with_loaded(true)
                    .with_action(action)
                    .with_local_vars(HashMap::from([
                        ("__local_2".to_string(), Value::Int(99)),
                        ("__local_3".to_string(), Value::Int(-1)),
                    ]))
                    .with_vertices(vec![ObjectVertex::new(2, 5), ObjectVertex::new(10, 5)]),
            )
            .expect("shipped PWRL spawns");

        engine.tick_without_snapshot().expect("CONNECT line executes");

        let index = engine.find_object_index(line).expect("PWRL survives");
        assert_eq!(
            engine.objects[index]
                .state
                .vertices
                .iter()
                .map(|vertex| (vertex.x, vertex.y))
                .collect::<Vec<_>>(),
            vec![(0, 5), (3, 3), (10, 5)]
        );
    }

    #[test]
    fn shipped_power_line_old_endpoint_fallback_uses_cpp_material_index_quirk() {
        // C4Shape::LineConnect falls back to the OLD endpoint only after all
        // 4/8/12-pixel bend candidates fail, and tests both fallback legs via
        // PathFreeIgnoreVehicle (src/C4Shape.cpp:303-313). C++ accidentally
        // applies DensitySolid to the material index, so every material in
        // this one-entry set is passable despite its solid density
        // (src/C4Landscape.cpp:2044-2052).
        fn run_case(
            resource: &ResourceDefinitionData,
            material: &str,
            move_first_endpoint: bool,
        ) -> Option<Vec<(i32, i32)>> {
            let mut engine = Engine::with_seed(0);
            let library = MaterialLibrary::parse(&format!(
                "[Material {material}]\nName={material}\nDensity=100\n"
            ))
            .expect("single solid material parses");
            engine.set_materials(MaterialSet::from_resource_library(&library));
            let mut power_line =
                Definition::from_resource(resource).expect("compile shipped PWRL definition");
            power_line.set_line(resource.core.line);
            power_line.set_line_intersect(resource.core.line_intersect);
            engine
                .register_definition(power_line)
                .expect("register shipped PWRL definition");
            engine
                .register_definition(
                    Definition::from_script("ANCR", "Line anchor", "#strict\n")
                        .expect("compile anchor definition"),
                )
                .expect("register anchor definition");

            let grid = landscape::PixelGrid::new(
                20,
                20,
                vec![1; 20 * 20],
                vec![0, 100],
                vec![None, Some(material.to_owned())],
                vec![None; 2],
            );
            let mut landscape = Landscape::new(20, vec![20; 20]).expect("landscape builds");
            landscape.set_pixel_grid(grid);
            landscape.set_world_height(20);
            engine.set_landscape(landscape);

            let first = engine
                .spawn_object(
                    SpawnConfig::new("ANCR")
                        .with_category(CATEGORY_STATIC_BACK)
                        .with_position(Vector2::new(2, 10)),
                )
                .expect("first anchor spawns");
            let second = engine
                .spawn_object(
                    SpawnConfig::new("ANCR")
                        .with_category(CATEGORY_STATIC_BACK)
                        .with_position(Vector2::new(18, 10)),
                )
                .expect("second anchor spawns");
            let mut action = ActionState::new("Connect");
            action.target = Some(first);
            action.target2 = Some(second);
            let vertices = if move_first_endpoint {
                vec![ObjectVertex::new(5, 10), ObjectVertex::new(18, 10)]
            } else {
                vec![ObjectVertex::new(2, 10), ObjectVertex::new(15, 10)]
            };
            let line = engine
                .spawn_object(
                    SpawnConfig::new("PWRL")
                        .with_category(CATEGORY_OBJECT)
                        .with_loaded(true)
                        .with_action(action)
                        .with_vertices(vertices),
                )
                .expect("shipped PWRL spawns");

            engine.tick_without_snapshot().expect("CONNECT line executes");
            engine.find_object_index(line).map(|index| {
                engine.objects[index]
                    .state
                    .vertices
                    .iter()
                    .map(|vertex| (vertex.x, vertex.y))
                    .collect()
            })
        }

        let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let group = clonk_resources::Group::open(repository.join(
            "content/Objects.c4d/Structures.c4d/Lines.c4d/PowerLine.c4d",
        ))
        .expect("open shipped PWRL definition");
        let resource =
            ResourceDefinitionData::load(&group).expect("load shipped PWRL definition");

        assert_eq!(
            run_case(&resource, "Vehicle", true),
            Some(vec![(2, 10), (5, 10), (18, 10)]),
            "first endpoint keeps the old point as a bend through Vehicle"
        );
        assert_eq!(
            run_case(&resource, "Vehicle", false),
            Some(vec![(2, 10), (15, 10), (18, 10)]),
            "last endpoint keeps the old point as a bend through Vehicle"
        );
        assert_eq!(
            run_case(&resource, "Granite", true),
            Some(vec![(2, 10), (5, 10), (18, 10)]),
            "low-index Granite keeps the old point as a first-endpoint bend"
        );
        assert_eq!(
            run_case(&resource, "Granite", false),
            Some(vec![(2, 10), (15, 10), (18, 10)]),
            "low-index Granite keeps the old point as a last-endpoint bend"
        );
    }

    #[test]
    fn connect_lines_reduce_redundant_bends_on_tick35_like_cpp() {
        // C++ oracle: CONNECT calls ReduceLineSegments on !Tick35
        // (src/C4Object.cpp:5443-5445). On frame 35 Tick2 is set, so the
        // non-alternate pass removes the middle vertex when endpoint-to-
        // endpoint PathFree succeeds (src/C4Object.cpp:4683-4694).
        let mut engine = Engine::with_seed(0);
        engine.set_landscape(Landscape::flat(100, 50));

        let mut line = Definition::from_script("LINE", "Line", "#strict\n").expect("compiles");
        line.set_line(8); // C4D_Line_Vertex
        line.set_line_intersect(1);
        line.set_shape_vertices(vec![
            ObjectVertex::new(10, 10),
            ObjectVertex::new(20, 20),
            ObjectVertex::new(30, 10),
        ]);
        line.configure_actions(
            None,
            HashMap::from([(
                "Connect".to_string(),
                ActionSpec::default()
                    .with_procedure("CONNECT")
                    .with_delay(10)
                    .with_length(1)
                    .with_next("Connect"),
            )]),
        );
        engine.register_definition(line).expect("line registers");

        let mut anchor =
            Definition::from_script("ANCR", "Anchor", "#strict\n").expect("compiles");
        anchor.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        engine.register_definition(anchor).expect("anchor registers");

        let first = engine
            .spawn_object(
                SpawnConfig::new("ANCR")
                    .with_category(CATEGORY_STATIC_BACK)
                    .with_position(Vector2::new(10, 10)),
            )
            .expect("first anchor spawns");
        let second = engine
            .spawn_object(
                SpawnConfig::new("ANCR")
                    .with_category(CATEGORY_STATIC_BACK)
                    .with_position(Vector2::new(30, 10)),
            )
            .expect("second anchor spawns");
        let line_id = engine
            .spawn_object(
                SpawnConfig::new("LINE")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Connect")),
            )
            .expect("line spawns");
        engine
            .apply_object_update(
                line_id,
                ObjectUpdate {
                    action: Some(
                        ActionUpdate::default()
                            .with_name("Connect")
                            .with_force(true)
                            .with_target(Some(first))
                            .with_target2(Some(second)),
                    ),
                    ..Default::default()
                },
            )
            .expect("connect action sets");

        for _ in 0..34 {
            engine.tick_without_snapshot().expect("pre-reduction tick succeeds");
        }
        let idx = engine.find_object_index(line_id).expect("line exists");
        assert_eq!(
            engine.objects[idx].state.vertices.len(),
            3,
            "line is unchanged before !Tick35"
        );

        engine.tick_without_snapshot().expect("Tick35 reduction succeeds");
        let idx = engine.find_object_index(line_id).expect("line survives");
        assert_eq!(
            engine.objects[idx]
                .state
                .vertices
                .iter()
                .map(|vertex| (vertex.x, vertex.y))
                .collect::<Vec<_>>(),
            vec![(10, 10), (30, 10)]
        );
    }

    // DFA_WALK arms Action.t_attach |= CNAT_Bottom every exec
    // (C4Object.cpp:4790-4792): a loaded walker standing one pixel INTO
    // the ground snaps up via Shape.Attach in the same frame (the INDI
    // 479-vs-478 resting-height class; C4Shape.cpp:165).
    #[test]
    fn walkers_snap_to_one_pixel_above_ground_like_cpp() {
        let mut engine = Engine::with_seed(0);
        engine.set_landscape(Landscape::flat(200, 100)); // ground from y=100
        let mut walker = Definition::from_script("WLKR", "Walker", "#strict\n").expect("compiles");
        walker.set_shape_rect(Some(DefinitionRect::new(-3, -8, 6, 16)));
        walker.set_shape_vertices(vec![ObjectVertex {
            x: 0,
            y: 8,
            cnat: CNAT_BOTTOM,
            friction: 50,
        }]);
        walker.configure_actions(
            None,
            HashMap::from([(
                "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_delay(1)
                    .with_length(8),
            )]),
        );
        engine.register_definition(walker).expect("registers");

        // Bottom vertex at 92+8 = 100 = INSIDE the ground row: the C++
        // attach shifts the position up one pixel (vertex to 99).
        let id = engine
            .spawn_object(
                SpawnConfig::new("WLKR")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 92))
                    .with_action(ActionState::new("Walk"))
                    .with_loaded(true),
            )
            .expect("spawns");
        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(
            engine.objects[idx].state.position.y, 91,
            "walk attach snaps the stander up one pixel (C4Shape::Attach)"
        );
    }

