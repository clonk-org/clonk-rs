    // Attached movement's zero-step iteration: when Shape.Attach pulls the
    // step candidate BACK to the current position (ctx==x, cty attach-
    // corrected to y), C++ still runs the ctco override bookkeeping —
    // `if (at_yovr) { ctcoy = y; ydir = Fix0; fix_y = itofix(y); }`
    // (C4Movement.cpp:351/:367-368; the do-loop has NO candidate==position
    // early exit) — the carried ydir is zeroed and fix_y resyncs to the
    // pixel. The GoldRush snake pinned this: freshly turned (xdir -0.5,
    // fix_x on the .5 boundary so no x step), its Turn-carried ydir 52428
    // must die in the first Walk frame — cpp (fix_y 383.0, ydir 0) vs
    // rust falling ballistic (fix_y 383.8, ydir 52428) at the f57 wall.
    #[test]
    fn attach_pullback_to_current_position_zeroes_ydir_and_resyncs_fix_y() {
        let mut engine = Engine::with_seed(0);
        engine.set_landscape(Landscape::flat(200, 15)); // ground from y=15
        let mut snake = Definition::from_script("SNKE", "Snake", "#strict\n").expect("compiles");
        snake.set_shape_rect(Some(DefinitionRect::new(-14, -5, 28, 10)));
        snake.set_shape_vertices(vec![
            ObjectVertex {
                x: -4,
                y: -3,
                cnat: CNAT_LEFT,
                friction: 100,
            },
            ObjectVertex {
                x: 0,
                y: 4,
                cnat: CNAT_BOTTOM,
                friction: 100,
            },
            ObjectVertex {
                x: 4,
                y: -3,
                cnat: CNAT_RIGHT,
                friction: 100,
            },
        ]);
        snake.configure_actions(
            None,
            HashMap::from([(
                "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_delay(1)
                    .with_length(8),
            )]),
        );
        engine.register_definition(snake).expect("registers");

        // Resting crawl position: bottom vertex (0,+4) at 14, ground at 15.
        let id = engine
            .spawn_object(
                SpawnConfig::new("SNKE")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 10))
                    .with_action(ActionState::new("Walk"))
                    .with_loaded(true),
            )
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");
        // The post-Turn state: no horizontal step this frame, a carried
        // vertical dir from the Turn's gravity accumulation.
        engine.objects[idx].state.command_direction = CommandDirection::Stop;
        engine.objects[idx].fixed_velocity = FixedVec2::new(C4Fixed::ZERO, C4Fixed::from_raw(52428));
        engine.objects[idx].state.mobile = true;

        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("exists");
        let object = &engine.objects[idx];
        assert_eq!(object.state.position, Vector2::new(100, 10), "no net motion");
        assert_eq!(
            object.fixed_velocity.y,
            C4Fixed::ZERO,
            "at_yovr zeroes ydir even when the attach-corrected step equals \
             the current position (C4Movement.cpp:367-368)"
        );
        assert_eq!(
            object.fixed_position.y,
            itofix(10),
            "fix_y resyncs to the pixel on the attachment override"
        );
    }

    // DFA_FLOAT clamps BOTH axes to lLimit = FIXED100(Physical.Float)
    // every exec (C4Object.cpp:5284-5285): a loaded bird with saved
    // XDir=-3 slows to -2.0 on its first frame (BIRD [Physical]
    // Float=200; the live class rust (-3,0) vs cpp (-2,0)).
    #[test]
    fn float_procedure_clamps_loaded_velocity_to_the_physical_limit() {
        let mut engine = Engine::with_seed(0);
        let mut bird = Definition::from_script("BIRD", "Bird", "#strict\n").expect("compiles");
        bird.configure_actions(
            None,
            HashMap::from([(
                "Fly".to_string(),
                ActionSpec::default()
                    .with_procedure("FLOAT")
                    .with_delay(1)
                    .with_length(20),
            )]),
        );
        let mut physical = PhysicalInfo::default();
        physical.float = 200;
        bird.set_physical(physical);
        engine.register_definition(bird).expect("bird registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("BIRD")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 20))
                    .with_action(ActionState::new("Fly"))
                    .with_loaded(true)
                    .with_fixed_velocity(FixedVec2 {
                        x: itofix(-3),
                        y: C4Fixed::ZERO,
                    }),
            )
            .expect("bird spawns");
        let idx = engine.find_object_index(id).expect("bird exists");
        engine.objects[idx].state.mobile = true;

        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("bird exists");
        assert_eq!(
            engine.objects[idx].fixed_velocity.x.val(),
            -math::fixed100(200).val(),
            "xdir clamps to -lLimit on the first DFA_FLOAT exec (C4Object.cpp:5285)"
        );
    }

    // DoGravity's float branch (C4Object.cpp:4644-4661): objects with
    // InLiquid && Def->Float RISE — ydir -= FloatAccel(0.10) clamped to
    // -1.0 (FloatAccel*-10), xdir decays toward 0 by FloatFriction(0.02),
    // and once the float line (y - 1 + Float*Con/FullCon - 1) leaves the
    // liquid, negative ydir zeroes (equilibrium at the surface). Free-fall
    // gravity is the ELSE branch — floats never sink under it.
    #[test]
    fn floating_objects_rise_to_the_float_line_like_cpp() {
        let mut engine = Engine::with_seed(0);
        let mut barrel = simple_definition("BARL");
        barrel.set_shape_rect(Some(DefinitionRect::new(-3, -3, 6, 6)));
        barrel.set_float_line(4);
        engine
            .register_definition(barrel)
            .expect("barrel registers");
        let mut landscape = Landscape::flat(40, 60);
        for x in 0..40 {
            landscape.set_liquid_column(x, vec![LiquidSegment::new(20, 50)]);
        }
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("BARL")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 40)),
            )
            .expect("barrel spawns");
        let idx = engine.find_object_index(id).expect("barrel exists");
        engine.objects[idx].state.in_liquid = true;
        engine.objects[idx].state.mobile = true;
        engine.objects[idx].fixed_velocity.x = itofix(1);

        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("barrel exists");
        assert_eq!(
            engine.objects[idx].fixed_velocity.y.val(),
            -math::FLOAT_ACCEL.val(),
            "one FloatAccel step of rise (C4Object.cpp:4649)"
        );
        assert_eq!(
            engine.objects[idx].fixed_velocity.x.val(),
            itofix(1).val() - math::FLOAT_FRICTION.val(),
            "xdir decays by FloatFriction toward zero (C4Object.cpp:4653)"
        );

        for _ in 0..15 {
            engine.tick_without_snapshot().expect("tick");
        }
        let idx = engine.find_object_index(id).expect("barrel exists");
        assert!(
            engine.objects[idx].fixed_velocity.y.val() >= -65536,
            "rise clamps at FloatAccel*-10 = -1.0 (C4Object.cpp:4650), got {}",
            engine.objects[idx].fixed_velocity.y.val()
        );
    }

    // FnObjectCount passes cthr->Obj as pExclude - LOCAL CALLS EXCLUDE
    // THE CALLER (C4Script.cpp FnObjectCount -> Game.ObjectCount, same as
    // FindObjectOwner). The AmmoHud pair depends on it: AHUD#1's
    // Initialize runs HudCount() = ObjectCount(GetID(),...) and must see
    // 0 (itself excluded) to create its partner (AmmoHud.c4d:17-18,22;
    // C++ NEWOBJ 1423 AHUD creator=AHUD(1422)).
    #[test]
    fn object_count_excludes_the_calling_object_like_cpp() {
        let script = r#"#strict
local iSeen;
func Probe() {
    iSeen = ObjectCount(GetID());
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let hud = Definition::from_script("AHUD", "Hud", script).expect("compiles");
        engine.register_definition(hud).expect("hud registers");
        let id = engine
            .spawn_object(SpawnConfig::new("AHUD").with_category(CATEGORY_OBJECT))
            .expect("hud spawns");
        engine
            .spawn_object(SpawnConfig::new("AHUD").with_category(CATEGORY_OBJECT))
            .expect("second hud spawns");
        let idx = engine.find_object_index(id).expect("hud exists");
        engine
            .call_object_function(idx, "Probe", Vec::new())
            .expect("probe runs");
        let idx = engine.find_object_index(id).expect("hud exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iSeen"),
            Some(&Value::Int(1)),
            "the caller is excluded: 2 huds minus self (FnObjectCount pExclude)"
        );
    }

    // C4Aul include linking sets OwnerOverloaded across #include boundaries
    // (C4AulLink.cpp), so `_inherited()` in COWB::ControlDownDouble reaches
    // CLNK::ControlDownDouble when TRPR includes COWB includes CLNK
    // (Trapper.c4d/Cowboy.c4d/Clonk.c4d) — the GoldRush dismount chain.
    #[test]
    fn sibling_includes_give_the_last_declaration_priority() {
        // C4AulParse pushes includes to the front and ResolveIncludes walks
        // that reversed list (C4AulParse.cpp:1456; C4AulLink.cpp:66-111).
        let mut engine = Engine::with_seed(0);
        engine
            .register_script_definition("INCA", "A", "public func Foo() { return(1); }")
            .expect("A registers");
        engine
            .register_script_definition("INCB", "B", "public func Foo() { return(2); }")
            .expect("B registers");
        engine
            .register_definition(
                Definition::from_script(
                    "CHLD",
                    "Child",
                    "#include INCA\n#include INCB\n",
                )
                .expect("child compiles"),
            )
            .expect("child registers");
        engine.resolve_includes().expect("includes resolve");
        engine.resolve_includes().expect("repeat resolve is stable");
        let id = engine
            .spawn_object(SpawnConfig::new("CHLD").with_category(CATEGORY_OBJECT))
            .expect("child spawns");
        let index = engine.find_object_index(id).expect("child exists");

        assert_eq!(
            engine
                .call_object_function(index, "Foo", Vec::new())
                .expect("Foo runs"),
            Value::Int(2)
        );
    }

    #[test]
    fn last_sibling_include_inherits_to_the_first() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_script_definition("INCA", "A", "public func Foo() { return(1); }")
            .expect("A registers");
        engine
            .register_definition(
                Definition::from_script(
                    "INCB",
                    "B",
                    "#strict\npublic func Foo() { return(_inherited()+10); }",
                )
                .expect("B compiles"),
            )
            .expect("B registers");
        engine
            .register_definition(
                Definition::from_script(
                    "CHLD",
                    "Child",
                    "#include INCA\n#include INCB\n",
                )
                .expect("child compiles"),
            )
            .expect("child registers");
        engine.resolve_includes().expect("includes resolve");
        let id = engine
            .spawn_object(SpawnConfig::new("CHLD").with_category(CATEGORY_OBJECT))
            .expect("child spawns");
        let index = engine.find_object_index(id).expect("child exists");

        assert_eq!(
            engine
                .call_object_function(index, "Foo", Vec::new())
                .expect("Foo runs"),
            Value::Int(11)
        );
    }

    #[test]
    fn own_function_inherits_through_siblings_in_cpp_order() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(
                Definition::from_script(
                    "INCA",
                    "A",
                    "local order; public func Foo() { order=order*10+1; return(order); }",
                )
                .expect("A compiles"),
            )
            .expect("A registers");
        engine
            .register_definition(
                Definition::from_script(
                    "INCB",
                    "B",
                    "#strict\npublic func Foo() { order=order*10+2; return(_inherited()); }",
                )
                .expect("B compiles"),
            )
            .expect("B registers");
        engine
            .register_definition(
                Definition::from_script(
                    "CHLD",
                    "Child",
                    "#strict\n#include INCA\n#include INCB\npublic func Foo() { order=order*10+3; return(inherited()); }",
                )
                .expect("child compiles"),
            )
            .expect("child registers");
        engine.resolve_includes().expect("includes resolve");
        engine.resolve_includes().expect("repeat resolve is stable");
        let id = engine
            .spawn_object(SpawnConfig::new("CHLD").with_category(CATEGORY_OBJECT))
            .expect("child spawns");
        let index = engine.find_object_index(id).expect("child exists");

        assert_eq!(
            engine
                .call_object_function(index, "Foo", Vec::new())
                .expect("Foo runs"),
            Value::Int(321),
            "own -> last include B -> first include A"
        );
    }

    #[test]
    fn byte_identical_sibling_functions_remain_distinct_in_inherited_chain() {
        // C++ copies one C4AulScriptFunc per include owner even when their
        // bodies are byte-identical; structural equality must not collapse
        // the two declarations.
        let identical =
            "#strict\npublic func Foo() { hits=hits+1; _inherited(); return(hits); }";
        let mut engine = Engine::with_seed(0);
        for (id, name) in [("INCA", "A"), ("INCB", "B")] {
            engine.register_script_definition(id, name, identical).expect("include registers");
        }
        engine
            .register_definition(
                Definition::from_script(
                    "CHLD",
                    "Child",
                    "#strict\n#include INCA\n#include INCB\nlocal hits; public func Foo() { return(_inherited()); } public func Hits() { return(hits); }",
                )
                .expect("child compiles"),
            )
            .expect("child registers");
        engine.resolve_includes().expect("includes resolve");
        let id = engine
            .spawn_object(SpawnConfig::new("CHLD").with_category(CATEGORY_OBJECT))
            .expect("child spawns");
        let index = engine.find_object_index(id).expect("child exists");
        engine
            .call_object_function(index, "Foo", Vec::new())
            .expect("identical include chain runs");

        assert_eq!(
            engine
                .call_object_function(index, "Hits", Vec::new())
                .expect("Hits runs"),
            Value::Int(2)
        );
    }

    #[test]
    fn magic_workshop_uses_bas8_basement_functions() {
        // Shipped MagicWorkshop declares WTWR, WRKS, BAS8 in that order;
        // C++ resolves the last sibling's BasementID/BasementWidth copies.
        let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
        let scripts = [
            ("BAS7", "Objects.c4d/Structures.c4d/Basements.c4d/Basement72.c4d/Script.c"),
            ("BAS2", "Objects.c4d/Structures.c4d/Basements.c4d/Basement42.c4d/Script.c"),
            ("BAS8", "Objects.c4d/Structures.c4d/Basements.c4d/Basement80.c4d/Script.c"),
            ("WTWR", "Objects.c4d/Structures.c4d/WizardTower.c4d/Script.c"),
            ("WRKS", "Objects.c4d/Structures.c4d/Workshop.c4d/Script.c"),
            ("MWKS", "Fantasy.c4d/Structures.c4d/MagicWorkshop.c4d/Script.c"),
        ];
        let mut engine = Engine::with_seed(0);
        for id in ["DOOR", "CXEC"] {
            engine.register_script_definition(id, id, "#strict\n").expect("stub registers");
        }
        for (id, relative) in scripts {
            let source = std::fs::read(content.join(relative)).expect("shipped script reads");
            let source = String::from_utf8_lossy(&source);
            engine
                .register_script_definition(id, id, &source)
                .expect("shipped definition registers");
        }
        engine.resolve_includes().expect("shipped includes resolve");
        let id = engine
            .spawn_object(SpawnConfig::new("MWKS").with_category(CATEGORY_OBJECT))
            .expect("MagicWorkshop spawns");
        let index = engine.find_object_index(id).expect("workshop exists");

        assert_eq!(
            engine
                .call_object_function(index, "BasementID", Vec::new())
                .expect("BasementID runs"),
            Value::C4Id("BAS8".to_string())
        );
        assert_eq!(
            engine
                .call_object_function(index, "BasementWidth", Vec::new())
                .expect("BasementWidth runs"),
            Value::Int(80)
        );
    }

    #[test]
    fn underscore_inherited_chains_through_a_two_level_include() {
        let base = r#"#strict
local hits;
protected func Poke() { hits = hits + 1; return(7); }
"#;
        let mid = r#"#strict
#include BASE
public func Poke() { return(_inherited()); }
"#;
        let top = r#"#strict
#include MIDD
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(Definition::from_script("BASE", "Base", base).expect("base"))
            .expect("base registers");
        engine
            .register_definition(Definition::from_script("MIDD", "Mid", mid).expect("mid"))
            .expect("mid registers");
        engine
            .register_definition(Definition::from_script("TOPP", "Top", top).expect("top"))
            .expect("top registers");
        engine.resolve_includes().expect("includes resolve");

        let id = engine
            .spawn_object(SpawnConfig::new("TOPP").with_category(CATEGORY_OBJECT))
            .expect("top spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        let result = engine
            .call_object_function(idx, "Poke", Vec::new())
            .expect("Poke runs without an inherited error");
        assert_eq!(result, Value::Int(7), "the BASE implementation answered");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("hits"),
            Some(&Value::Int(1)),
            "the BASE body executed once"
        );
    }

    // Cowboy.c4d Recruitment creates the AHUD with the recruit's owner
    // (`CreateObject(AHUD,0,0,GetOwner())`, Cowboy.c4d/Script.c:13) — the
    // GoldRush AHUD must belong to player 0 or FindObjectOwner misses it
    // and every later recruit spawns another one (id skew).
    #[test]
    fn recruitment_created_objects_inherit_the_get_owner_argument() {
        let script = r#"#strict
#include BASE
func Recruitment(iPlr) {
    if(!FindObjectOwner(CHLD, GetOwner())) CreateObject(CHLD, 0, 0, GetOwner());
    return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        let base =
            Definition::from_script("BASE", "Base", "#strict\nfunc IsBase() { return(1); }\n")
                .expect("base compiles");
        engine.register_definition(base).expect("base registers");
        let mut crew = Definition::from_script("CREW", "Crew", script).expect("compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        engine.resolve_includes().expect("includes resolve");
        engine
            .register_definition(simple_definition("CHLD"))
            .expect("child registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(5)
                    .with_crew_member(true),
            )
            .expect("crew spawns");
        let idx = engine.find_object_index(id).expect("crew exists");
        engine
            .call_object_function(idx, "Recruitment", vec![Value::Int(5)])
            .expect("recruitment runs");

        let child = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "CHLD")
            .expect("child created");
        assert_eq!(
            child.state.owner, 5,
            "CreateObject's owner argument (GetOwner()) sticks"
        );
    }

    #[test]
    fn get_rank_tracks_same_call_make_crew_and_info_transfer() {
        let script = r#"#strict 2
func Probe() {
    var donor = CreateObject(DONR, 0, 0, 0);
    var before = GetRank(donor);
    var made = MakeCrewMember(donor, 0);
    var made_rank = GetRank(donor);
    var grabbed = GrabObjectInfo(donor);
    return [before, made, made_rank, grabbed, GetRank(), GetRank(donor)];
}
func ReadRank() { return GetRank(); }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Rank owner"))
            .expect("rank owner registers");
        let receiver = Definition::from_script("RCVR", "Receiver", script)
            .expect("GetRank transfer probe compiles");
        engine
            .register_definition(receiver)
            .expect("receiver registers");
        let mut donor = simple_definition("DONR");
        donor.set_crew_member(true);
        engine
            .register_definition(donor)
            .expect("crew donor registers");
        let receiver = engine
            .spawn_object(SpawnConfig::new("RCVR").with_owner(0))
            .expect("receiver spawns");
        let receiver_index = engine
            .find_object_index(receiver)
            .expect("receiver index");

        assert_eq!(
            engine
                .call_object_function(receiver_index, "Probe", Vec::new())
                .expect("same-call rank probe runs"),
            Value::Array(vec![
                Value::Nil,
                Value::Bool(true),
                Value::Int(0),
                Value::Bool(true),
                Value::Int(0),
                Value::Nil,
            ])
        );
        assert_eq!(
            engine
                .call_object_function(receiver_index, "ReadRank", Vec::new())
                .expect("transferred rank persists after the callback"),
            Value::Int(0)
        );
    }

    #[test]
    fn make_crew_member_requires_an_explicit_object_and_preserves_owner() {
        let script = r#"#strict 2
func DirectNil() { var no_object; return MakeCrewMember(no_object, 1); }
func ArrowZero(object target) { return target->MakeCrewMember(0); }
func Explicit(object target) {
    return [MakeCrewMember(target, 1), GetOwner(target), GetController(target)];
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Original owner"))
            .expect("player zero registers");
        engine
            .register_player(PlayerConfig::new(1, "Recruiting player"))
            .expect("player one registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("MakeCrewMember probe compiles");
        crew.set_crew_member(true);
        crew.set_value(17);
        engine.register_definition(crew).expect("crew registers");

        let caller = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("caller spawns outside every crew");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("target spawns outside every crew");
        let caller_index = engine.find_object_index(caller).expect("caller index");

        assert_eq!(
            engine
                .call_object_function(caller_index, "DirectNil", Vec::new())
                .expect("direct nil call completes"),
            Value::Bool(false)
        );
        assert!(engine.player(1).expect("player one").crew().is_empty());
        assert_eq!(engine.object_controller(caller), Some(0));

        // AB_CALL changes cthr->Obj but forwards native arguments unchanged:
        // the zero is a nil pObj and must not recruit the arrow receiver.
        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "ArrowZero",
                    vec![Value::Object(target.as_u64())],
                )
                .expect("arrow nil call completes"),
            Value::Bool(false)
        );
        assert!(engine.player(0).expect("player zero").crew().is_empty());
        assert_eq!(engine.object_controller(target), Some(0));

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Explicit",
                    vec![Value::Object(target.as_u64())],
                )
                .expect("explicit recruitment completes"),
            Value::Array(vec![Value::Bool(true), Value::Int(0), Value::Int(1)])
        );
        assert_eq!(engine.player(1).expect("player one").crew(), &[target]);
        assert_eq!(engine.object_controller(target), Some(1));

        engine
            .update_player_asset_values()
            .expect("cross-player crew values update");
        let original_owner = engine.player(0).expect("original owner remains");
        assert_eq!(original_owner.objects_owned(), 2);
        assert_eq!(original_owner.value(), 34);
        let recruiting_player = engine.player(1).expect("recruiting player remains");
        assert_eq!(recruiting_player.objects_owned(), 0);
        assert_eq!(recruiting_player.value(), 0);
    }

    #[test]
    fn set_crew_status_keeps_cpp_rosters_owner_info_and_callback_order() {
        let script = r#"#strict 2
local recruitments;
local unselections;
local removing_player;
local callback_rank;
local callback_crew_count;

func Recruitment(int player) {
    recruitments = recruitments + 1;
    return 1;
}

func CrewSelection(bool unselect, bool cursor) {
    if (unselect) {
        unselections = unselections + 1;
        callback_rank = GetRank();
        callback_crew_count = GetCrewCount(removing_player);
    }
    return 1;
}

func Setup() {
    var no_object;
    var made = MakeCrewMember(this(), 0);
    var added = SetCrewStatus(1, true, no_object);
    return [made, added, GetOwner(), GetController(), GetRank(),
            GetCrewCount(0), GetCrewCount(1), GetCrew(1), recruitments,
            GetPlayerVal("Crew", "Player", 0, 0),
            GetPlayerVal("Crew", "Player", 1, 0)];
}

func RemoveFrom(int player) {
    var no_object;
    removing_player = player;
    SetCursor(player, this(), true, true, true);
    SelectCrew(player, this(), true, true);
    var removed = SetCrewStatus(player, false, no_object);
    return [removed, GetCrewCount(player), GetRank(), unselections,
            callback_rank, callback_crew_count, GetOwner(), GetController(),
            GetCursor(player) == this(),
            GetPlayerVal("Crew", "Player", player, 0)];
}

func RemoveAgain(int player) {
    var no_object;
    return [SetCrewStatus(player, false, no_object), unselections];
}

func TryAdd(int player, object target) {
    return SetCrewStatus(player, true, target);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Info owner"))
            .expect("player zero registers");
        engine
            .register_player(PlayerConfig::new(1, "Second crew"))
            .expect("player one registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("SetCrewStatus probe compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        engine
            .register_definition(simple_definition("ROCK"))
            .expect("non-crew target registers");

        let target = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("target spawns outside every crew");
        let target_index = engine.find_object_index(target).expect("target index");
        assert_eq!(
            engine
                .call_object_function(target_index, "Setup", Vec::new())
                .expect("both crew additions complete"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(1),
                Value::Int(0),
                Value::Int(1),
                Value::Int(0),
                Value::Int(1),
                Value::Int(1),
                Value::Object(target.as_u64()),
                Value::Int(2),
                Value::Int(target.as_u64() as i32),
                Value::Int(target.as_u64() as i32),
            ])
        );
        assert_eq!(engine.crew_members(0), vec![target]);
        assert_eq!(engine.crew_members(1), vec![target]);
        assert_eq!(
            engine.object_snapshot(target).expect("target exists").owner,
            0,
            "adding to player one changes Controller but never Owner"
        );
        assert!(
            engine
                .object_snapshot(target)
                .expect("target exists")
                .info_physical
                .is_some()
        );

        assert_eq!(
            engine
                .call_object_function(target_index, "RemoveFrom", vec![Value::Int(1)])
                .expect("foreign-info crew removal completes"),
            Value::Array(vec![
                Value::Int(1),
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
                Value::Bool(true),
                Value::Nil,
            ])
        );
        assert_eq!(engine.crew_members(0), vec![target]);
        assert!(engine.crew_members(1).is_empty());
        assert!(
            engine
                .object_snapshot(target)
                .expect("target exists")
                .info_physical
                .is_some(),
            "removing from a foreign crew preserves player zero's Info"
        );

        assert_eq!(
            engine
                .call_object_function(target_index, "RemoveFrom", vec![Value::Int(0)])
                .expect("own-info crew removal completes"),
            Value::Array(vec![
                Value::Int(1),
                Value::Int(0),
                Value::Nil,
                Value::Int(2),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
                Value::Bool(true),
                Value::Nil,
            ])
        );
        assert!(engine.crew_members(0).is_empty());
        assert!(
            engine
                .object_snapshot(target)
                .expect("target exists")
                .info_physical
                .is_none()
        );
        assert!(!engine.object_snapshot(target).expect("target exists").selected);
        assert_eq!(
            engine
                .call_object_function(target_index, "RemoveAgain", vec![Value::Int(0)])
                .expect("idempotent removal completes"),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            "already-absent removal has no callback side effects"
        );

        let newer = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("second crew-capable object spawns outside every crew");
        for object in [target, newer] {
            assert_eq!(
                engine
                    .call_object_function(
                        target_index,
                        "TryAdd",
                        vec![Value::Int(1), Value::Object(object.as_u64())],
                    )
                    .expect("SetCrewStatus add succeeds"),
                Value::Int(1)
            );
        }
        assert_eq!(
            engine.crew_members(1),
            vec![newer, target],
            "stMain inserts a new equal-category/id member before its existing group"
        );
        assert_eq!(
            engine
                .call_object_function(
                    target_index,
                    "TryAdd",
                    vec![Value::Int(1), Value::Object(target.as_u64())],
                )
                .expect("idempotent add succeeds"),
            Value::Int(1)
        );
        assert_eq!(
            engine.crew_members(1),
            vec![newer, target],
            "an already-present member is not reinserted or reordered"
        );

        let rock = engine
            .spawn_object(SpawnConfig::new("ROCK").with_owner(0))
            .expect("rock spawns");
        assert_eq!(
            engine
                .call_object_function(
                    target_index,
                    "TryAdd",
                    vec![Value::Int(1), Value::Object(rock.as_u64())],
                )
                .expect("non-crew add returns normally"),
            Value::Int(0)
        );
        assert_eq!(
            engine
                .call_object_function(
                    target_index,
                    "TryAdd",
                    vec![Value::Int(99), Value::Object(target.as_u64())],
                )
                .expect("invalid-player add returns normally"),
            Value::Int(0)
        );
    }

    #[test]
    fn set_crew_status_shared_member_death_only_clears_the_owner_roster() {
        let script = r#"#strict 2
func SetupShared() {
    var made = MakeCrewMember(this(), 0);
    var shared = SetCrewStatus(1, true);
    SetCursor(0, this(), true, true, true);
    SetCursor(1, this(), true, true, true);
    return [made, shared];
}
func Die() { DoEnergy(-100); return 1; }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Owner"))
            .expect("owner registers");
        engine
            .register_player(PlayerConfig::new(1, "Shared roster"))
            .expect("second player registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("shared-death probe compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_alive(true)
                    .with_energy(100_000)
                    .with_crew_member(false),
            )
            .expect("shared crew target spawns");
        let target_index = engine.find_object_index(target).expect("target index");

        assert_eq!(
            engine
                .call_object_function(target_index, "SetupShared", Vec::new())
                .expect("shared membership installs"),
            Value::Array(vec![Value::Bool(true), Value::Int(1)])
        );
        assert_eq!(engine.player(0).expect("owner").crew(), &[target]);
        assert_eq!(engine.player(1).expect("second player").crew(), &[target]);

        engine
            .call_object_function(target_index, "Die", Vec::new())
            .expect("crew death completes");

        let dead = engine.object_snapshot(target).expect("dead crew remains");
        assert!(!dead.alive);
        assert!(
            dead.crew_member,
            "the legacy bit remains set while any player's Crew contains the object"
        );
        assert!(engine.player(0).expect("owner").crew().is_empty());
        assert_eq!(
            engine.player(1).expect("second player").crew(),
            &[target],
            "AssignDeath calls ClearPointers only for Object::Owner"
        );
        assert_eq!(engine.crew_cursor(0), None);
        assert_eq!(
            engine.crew_cursor(1),
            Some(target),
            "the foreign player's cursor is not an owner pointer"
        );

        for _ in 0..35 {
            engine.tick_without_snapshot().expect("Tick35 window advances");
        }
        assert!(engine.is_owner_eliminated(0));
        assert!(
            !engine.is_owner_eliminated(1),
            "CrewCnt counts roster links regardless of the member's Alive flag"
        );
    }

    #[test]
    fn inactive_crew_preserves_all_rosters_and_pointers_until_reactivated() {
        let script = r#"#strict 2
func Setup() {
    MakeCrewMember(this(), 0);
    SetCrewStatus(1, true);
    SetCursor(0, this(), true, true, true);
    SetCursor(1, this(), true, true, true);
    SelectCrew(0, this(), true, true);
    return 1;
}
func Deactivate() {
    var no_object;
    var changed = SetObjectStatus(2, no_object, false);
    var added_inactive = SetCrewStatus(2, true);
    return [changed, added_inactive, GetObjectStatus(),
            GetCrewCount(0), GetCrewCount(1), GetCrewCount(2),
            GetCrew(0) == this(), GetCrew(1) == this(),
            GetCursor(0) == this(), GetCursor(1) == this()];
}
func Reactivate() {
    var no_object;
    var changed = SetObjectStatus(1, no_object, false);
    return [changed, GetObjectStatus(),
            GetCrewCount(0), GetCrewCount(1), GetCrewCount(2),
            GetCrew(0) == this(), GetCrew(1) == this(),
            GetCursor(0) == this(), GetCursor(1) == this()];
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Owner"))
            .expect("owner registers");
        engine
            .register_player(PlayerConfig::new(1, "Foreign roster"))
            .expect("foreign player registers");
        engine
            .register_player(PlayerConfig::new(2, "Inactive add"))
            .expect("inactive-add player registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("inactive-crew probe compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("crew target spawns");
        let target_index = engine.find_object_index(target).expect("target index");
        engine
            .call_object_function(target_index, "Setup", Vec::new())
            .expect("shared membership and pointers install");

        let preserved = Value::Array(vec![
            Value::Bool(true),
            Value::Int(1),
            Value::Int(2),
            Value::Int(1),
            Value::Int(1),
            Value::Int(1),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
        ]);
        assert_eq!(
            engine
                .call_object_function(target_index, "Deactivate", Vec::new())
                .expect("StatusDeactivate(false) completes"),
            preserved
        );
        assert_eq!(
            engine.object_snapshot(target).expect("target remains").status,
            ObjectStatus::Inactive
        );
        assert_eq!(engine.player(0).expect("owner").crew(), &[target]);
        assert_eq!(engine.player(1).expect("foreign player").crew(), &[target]);
        assert_eq!(engine.player(2).expect("inactive add").crew(), &[target]);
        assert_eq!(engine.crew_cursor(0), Some(target));
        assert_eq!(engine.crew_cursor(1), Some(target));
        assert!(
            engine.object_snapshot(target).expect("target remains").selected,
            "StatusDeactivate(false) preserves the shared C4Object::Select bit"
        );

        assert_eq!(
            engine
                .call_object_function(target_index, "Reactivate", Vec::new())
                .expect("StatusActivate completes"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(1),
                Value::Int(1),
                Value::Int(1),
                Value::Int(1),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
                Value::Bool(true),
            ])
        );
        assert_eq!(engine.player(0).expect("owner").crew(), &[target]);
        assert_eq!(engine.player(1).expect("foreign player").crew(), &[target]);
        assert_eq!(engine.player(2).expect("inactive add").crew(), &[target]);
    }

    #[test]
    fn set_crew_status_retires_the_exact_linked_duplicate_info_entry() {
        let script = r#"#strict 2
func RemoveAndRecruit(object target) {
    var removed = SetCrewStatus(0, false);
    var recruited = MakeCrewMember(target, 0);
    return [removed, recruited];
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("duplicate-info probe compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CREW".to_string(), 2)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(JoinPlayerConfig {
                name: "Duplicate info owner".to_string(),
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
                crew: vec![
                    player_file::CrewInfo {
                        id: "CREW".to_string(),
                        name: "First pointer".to_string(),
                        rank: 4,
                        rank_name: "Major".to_string(),
                        experience: 900,
                        ..Default::default()
                    },
                    player_file::CrewInfo {
                        id: "CREW".to_string(),
                        name: "Second pointer".to_string(),
                        rank: 4,
                        rank_name: "Major".to_string(),
                        experience: 900,
                        ..Default::default()
                    },
                ],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("player and both duplicate-field infos join");

        let replacement = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("replacement object spawns without info");

        let second = engine
            .player(0)
            .expect("player")
            .crew()
            .iter()
            .copied()
            .find(|id| {
                engine
                    .crew_object_info(*id)
                    .is_some_and(|info| info.name == "Second pointer")
            })
            .expect("second exact info is linked");
        let second_index = engine.find_object_index(second).expect("second index");
        assert_eq!(
            engine
                .call_object_function(
                    second_index,
                    "RemoveAndRecruit",
                    vec![Value::Object(replacement.as_u64())],
                )
                .expect("second exact info retires and is reused synchronously"),
            Value::Array(vec![Value::Int(1), Value::Bool(true)])
        );
        assert_eq!(
            engine
                .crew_object_info(replacement)
                .expect("replacement gets info")
                .name,
            "Second pointer",
            "equal rank/experience must not retire the first matching roster entry"
        );
    }

    #[test]
    fn get_and_set_max_player_use_the_live_cpp_integer_parameter() {
        // FnGetMaxPlayer reads the exact live round parameter. FnSetMaxPlayer
        // returns integer zero/one and mutates that parameter synchronously,
        // so a Get later in the same VM call observes successful writes while
        // a rejected negative write leaves the old value visible
        // (C4Script.cpp:3693-3706,6918-6919).
        let mut engine = Engine::with_seed(0);
        engine.set_max_players(7);
        engine
            .register_definition(
                Definition::from_script(
                    "MPLR",
                    "Max-player probe",
                    r#"#strict 2
                    func ReadLimit() { return GetMaxPlayer(); }
                    func SetAndRead(int limit) {
                        return [SetMaxPlayer(limit), GetMaxPlayer()];
                    }
                    "#,
                )
                .expect("max-player probe compiles"),
            )
            .expect("max-player probe registers");
        let probe = engine
            .spawn_object(SpawnConfig::new("MPLR"))
            .expect("max-player probe spawns");
        let probe_index = engine
            .find_object_index(probe)
            .expect("max-player probe remains live");

        assert_eq!(
            engine
                .call_object_function(probe_index, "ReadLimit", Vec::new())
                .expect("GetMaxPlayer call completes"),
            Value::Int(7)
        );
        assert_eq!(
            engine
                .call_object_function(probe_index, "SetAndRead", vec![Value::Int(-1)])
                .expect("negative SetMaxPlayer/GetMaxPlayer call completes"),
            Value::Array(vec![Value::Int(0), Value::Int(7)])
        );
        assert_eq!(
            engine.max_players(),
            Some(7),
            "a rejected negative limit must not mutate the live parameter"
        );

        assert_eq!(
            engine
                .call_object_function(probe_index, "SetAndRead", vec![Value::Int(0)])
                .expect("zero SetMaxPlayer/GetMaxPlayer call completes"),
            Value::Array(vec![Value::Int(1), Value::Int(0)])
        );
        assert_eq!(
            engine.max_players(),
            Some(0),
            "zero is a valid player limit"
        );

        assert_eq!(
            engine
                .call_object_function(probe_index, "SetAndRead", vec![Value::Int(2)])
                .expect("positive SetMaxPlayer/GetMaxPlayer call completes"),
            Value::Array(vec![Value::Int(1), Value::Int(2)])
        );
        assert_eq!(engine.max_players(), Some(2));
    }

    #[test]
    fn shipped_hazard_tutorial_script1_raises_limit_before_creating_drones() {
        // Hazard starts at MaxPlayer=1. Its exact Script1 raises that live
        // parameter to two immediately before CreateScriptPlayer; an absent
        // SetMaxPlayer host aborts the call before the script-player request.
        // As with the Script65 regression below, load the shipped script but
        // skip its very large Initialize function.
        let mut engine = Engine::with_seed(0);
        engine.set_max_players(1);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/Colors.c".to_string(),
                "global func RGB(int red, int green, int blue) { return red * 65536 + green * 256 + blue; }"
                    .to_string(),
            )]),
            1,
            "the app normally supplies System.c4g's RGB helper"
        );
        let scenario_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/Hazard.c4f/Tutorial.c4s");
        let scenario = clonk_resources::Group::open(&scenario_path)
            .expect("shipped Hazard tutorial group opens");
        let source = scenario
            .read_file("Script.c")
            .unwrap_or_else(|error| panic!("shipped Hazard tutorial script reads: {error}"));
        let source = String::from_utf8_lossy(&source);
        let source = clonk_resources::localize_script_source(&scenario, &source, &["US"])
            .expect("shipped Hazard tutorial script localizes");
        engine
            .load_scenario_script_with_convention("Hazard Tutorial", &source, true)
            .expect("shipped Hazard tutorial script loads without Initialize");

        engine
            .call_scenario_script_function("Script1", Vec::new())
            .expect("shipped Hazard Script1 reaches CreateScriptPlayer");

        assert_eq!(engine.max_players(), Some(2));
        let updates = engine.take_script_player_info_updates();
        let [request] = updates.as_slice() else {
            panic!("expected one Hazard drone PlayerInfo request, got {updates:?}");
        };
        assert_eq!(request.client_id, 0);
        assert_eq!(request.flags, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS);
        let [drone] = request.players.as_slice() else {
            panic!("expected one Hazard drone player, got {:?}", request.players);
        };
        assert_eq!(drone.name.as_bytes(), b"Drones");
        assert_eq!(drone.player_type, PLAYER_INFO_TYPE_SCRIPT);
        assert_eq!((drone.color, drone.original_color), (0x0001_0101, 0x0001_0101));
        assert_eq!(drone.team, 2);
        assert_eq!(
            drone.flags,
            PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
                | PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                | PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK
                | PLAYER_INFO_FLAG_INVISIBLE
        );
    }

    #[test]
    fn shipped_hazard_tutorial_script65_awards_crew_experience() {
        // Exercise the exact shipped call without applying Hazard's very large
        // Initialize function. Script65 ends the tutorial with
        // DoCrewExp(100, GetCrew()); 100 is below the first C4RankSystem
        // promotion threshold (Experience(1) = 1000), so only experience
        // changes here.
        let mut engine = Engine::with_seed(0);
        let mut crew = Definition::from_script("HZCK", "Hazard Clonk", "#strict 2\n")
            .expect("synthetic Hazard Clonk compiles");
        crew.set_crew_member(true);
        engine
            .register_definition(crew)
            .expect("synthetic Hazard Clonk registers");

        let mut start = PlayerStart::default();
        start.ready_crew = vec![("HZCK".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(JoinPlayerConfig {
                name: "Hazard trainee".to_string(),
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
                crew: vec![player_file::CrewInfo {
                    id: "HZCK".to_string(),
                    name: "Rookie".to_string(),
                    ..Default::default()
                }],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("Hazard trainee joins");

        let crew_id = *engine
            .player(0)
            .expect("player zero exists")
            .crew()
            .first()
            .expect("ready HZCK crew exists");
        let before = engine.capture_state();
        let link = *before
            .crew_info_links
            .get(&crew_id)
            .expect("HZCK carries its persistent roster link");

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/Hazard.c4f/Tutorial.c4s/Script.c");
        let source = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("shipped Hazard tutorial script reads: {error}"));
        let source = String::from_utf8_lossy(&source);
        engine
            .load_scenario_script_with_convention("Hazard Tutorial", &source, true)
            .expect("shipped Hazard tutorial script loads without Initialize");
        engine
            .call_scenario_script_function("Script65", Vec::new())
            .expect("shipped Hazard Script65 completes");

        let info = engine
            .crew_object_info(crew_id)
            .expect("HZCK keeps its live crew info");
        assert_eq!(info.experience, 100);
        assert_eq!(info.rank, 0, "100 experience is below rank one's 1000");

        let after = engine.capture_state();
        let persisted = &after.crew_info_rosters[&link.player_id][link.roster_index];
        assert_eq!(persisted.experience, 100);
        assert_eq!(persisted.rank, 0);
    }

    fn shipped_clonk_rank_names() -> Vec<String> {
        let base = [
            "Clonk",
            "Ensign",
            "Lieutenant",
            "Captain",
            "Major",
            "Lieutenant Colonel",
            "Colonel",
            "Brigade General",
            "Major General",
            "Lieutenant General",
            "General",
            "Midshipman",
            "Commander",
            "Commodore",
            "Rear-Admiral",
            "Vice-Admiral",
            "Admiral",
            "Fleet Admiral",
            "Counsellor of State",
            "Secretary of State",
            "Chancellor",
            "Vice President",
            "President",
            "Premier",
        ];
        let mut expanded = base.iter().map(|name| (*name).to_string()).collect::<Vec<_>>();
        for extension in [
            "%s First Class",
            "%s Second Degree",
            "%s Without Equal",
            "Sublime %s",
            "Exalted %s",
        ] {
            expanded.extend(base.iter().map(|name| extension.replace("%s", name)));
        }
        expanded
    }

    fn rank_join_config(crew: Vec<player_file::CrewInfo>) -> JoinPlayerConfig {
        JoinPlayerConfig {
            name: "Rank owner".to_string(),
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
            crew,
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        }
    }

    fn crew_info_with_extra_data(
        extra_data: Vec<(String, Value)>,
    ) -> player_file::CrewInfo {
        player_file::CrewInfo {
            id: "CLNK".to_string(),
            name: "Extra data crew".to_string(),
            rank_name: "Clonk".to_string(),
            extra_data,
            portraits: Default::default(),
            ..Default::default()
        }
    }

    const DEFAULT_CREW_RANKS: [&str; 11] = [
        "Clonk",
        "Ensign",
        "Lieutenant",
        "Captain",
        "Major",
        "Lieutenant Colonel",
        "Colonel",
        "Brigade General",
        "Major General",
        "Lieutenant General",
        "General",
    ];

    #[test]
    fn next_rank_info_nonzero_stored_values_bypass_the_default_ranks() {
        for (name, experience, promotion_possible) in [
            ("Custom", 2_500, true),
            ("", 2_500, true),
            ("Custom", -1, false),
            ("Odd sentinel", -2, true),
        ] {
            let core = CrewInfoCoreFields {
                next_rank_name: name.to_string(),
                next_rank_exp: experience,
                ..CrewInfoCoreFields::default()
            };
            let before = core.clone();

            let next = core.next_rank_info(0, &DEFAULT_CREW_RANKS, 1_000);

            assert_eq!(next.name, Some(name));
            assert_eq!(next.experience, experience);
            assert_eq!(next.promotion_possible(), promotion_possible);
            assert_eq!(core, before, "the query must not rewrite persisted fields");
        }
    }

    #[test]
    fn next_rank_info_zero_falls_back_to_default_rank_plus_one() {
        let core = CrewInfoCoreFields {
            next_rank_name: "persisted zero-tag name".to_string(),
            next_rank_exp: 0,
            ..CrewInfoCoreFields::default()
        };
        let before = core.clone();

        let rank_zero = core.next_rank_info(0, &DEFAULT_CREW_RANKS, 1_000);
        assert_eq!(rank_zero.name, Some("Ensign"));
        assert_eq!(rank_zero.experience, 1_000);
        assert!(rank_zero.promotion_possible());

        let negative_rank = core.next_rank_info(-1, &DEFAULT_CREW_RANKS, 1_000);
        assert_eq!(negative_rank.name, Some("Clonk"));
        assert_eq!(negative_rank.experience, 0);
        assert!(
            negative_rank.promotion_possible(),
            "zero experience is still a successful rank-zero fallback"
        );

        let found_empty = core.next_rank_info(-1, &[""], 1_000);
        assert_eq!(found_empty.name, Some(""));
        assert_eq!(found_empty.experience, 0);
        assert!(found_empty.promotion_possible());
        assert_eq!(core, before, "fallback lookup must remain output-only");
    }

    #[test]
    fn next_rank_info_exhausted_default_reports_no_promotion_without_mutation() {
        let core = CrewInfoCoreFields {
            next_rank_name: "untouched output-only source".to_string(),
            next_rank_exp: 0,
            ..CrewInfoCoreFields::default()
        };
        let before = core.clone();

        let next = core.next_rank_info(10, &DEFAULT_CREW_RANKS, 1_000);

        assert_eq!(next.name, None);
        assert_eq!(next.experience, -1);
        assert!(!next.promotion_possible());

        let overflow = core.next_rank_info(i32::MAX, &DEFAULT_CREW_RANKS, 1_000);
        assert_eq!(overflow.name, None);
        assert_eq!(overflow.experience, -1);
        assert!(!overflow.promotion_possible());
        assert_eq!(core, before, "exhaustion must not persist fallback output");
    }

    #[test]
    fn installed_default_rank_names_drive_global_promotion() {
        let script = r#"#strict 2
func Award()
{
    return [DoCrewExp(1000), GetRank(),
            GetObjectInfoCoreVal("RankName", "ObjectInfo")];
}
"#;

        for (rank_names, expected) in [
            (vec!["Clonk", "Ensign"], "Ensign"),
            (vec!["Clonk", "Fähnrich"], "Fähnrich"),
        ] {
            let mut engine = Engine::with_seed(0);
            engine.set_default_rank_names(rank_names.into_iter().map(str::to_owned).collect());
            let mut crew = Definition::from_script("CREW", "Crew", script)
                .expect("global-rank promotion probe compiles");
            crew.set_crew_member(true);
            engine.register_definition(crew).expect("crew registers");

            let mut start = PlayerStart::default();
            start.ready_crew = vec![("CREW".to_string(), 1)];
            engine.set_player_starts(vec![start]);
            engine
                .join_player(rank_join_config(Vec::new()))
                .expect("rank owner joins");

            let crew_id = engine.player(0).expect("rank owner exists").crew()[0];
            let crew_index = engine.find_object_index(crew_id).expect("crew exists");
            assert_eq!(
                engine
                    .call_object_function(crew_index, "Award", Vec::new())
                    .expect("global-rank award succeeds"),
                Value::Array(vec![
                    Value::Bool(true),
                    Value::Int(1),
                    Value::String(expected.to_string().into()),
                ])
            );

            let info = engine
                .crew_object_info(crew_id)
                .expect("promoted crew keeps its info");
            assert_eq!((info.rank, info.experience), (1, 1_000));
            assert_eq!(info.rank_name, expected);
            let state = engine.capture_state();
            let link = state.crew_info_links[&crew_id];
            assert_eq!(
                state.crew_info_rosters[&link.player_id][link.roster_index].rank_name,
                expected
            );
            assert_eq!(
                state
                    .messages
                    .iter()
                    .find(|message| message.snapshot.target == Some(crew_id))
                    .expect("promotion message is presented")
                    .snapshot
                    .lines,
                [
                    "Clonk is promoted".to_string(),
                    format!("to {expected}!")
                ]
            );
        }
    }

    #[test]
    fn object_info_core_reflects_fresh_custom_progression_and_all_scalar_fields() {
        let script = r#"#strict 2
func ReadCore()
{
    return [GetObjectInfoCoreVal("id", "ObjectInfo"),
            GetObjectInfoCoreVal("Name", "ObjectInfo"),
            GetObjectInfoCoreVal("DeathMessage", "ObjectInfo"),
            GetObjectInfoCoreVal("PortraitFile", "ObjectInfo"),
            GetObjectInfoCoreVal("Rank", "ObjectInfo"),
            GetObjectInfoCoreVal("RankName", "ObjectInfo"),
            GetObjectInfoCoreVal("NextRankName", "ObjectInfo"),
            GetObjectInfoCoreVal("TypeName", "ObjectInfo"),
            GetObjectInfoCoreVal("Participation", "ObjectInfo"),
            GetObjectInfoCoreVal("Experience", "ObjectInfo"),
            GetObjectInfoCoreVal("NextRankExp", "ObjectInfo"),
            GetObjectInfoCoreVal("Rounds", "ObjectInfo"),
            GetObjectInfoCoreVal("DeathCount", "ObjectInfo"),
            GetObjectInfoCoreVal("Birthday", "ObjectInfo"),
            GetObjectInfoCoreVal("TotalPlayingTime", "ObjectInfo"),
            GetObjectInfoCoreVal("Age", "ObjectInfo"),
            GetObjectInfoCoreVal("ExtraData", "ObjectInfo"),
            GetObjectInfoCoreVal("Energy", "Physical"),
            GetObjectInfoCoreVal("Rounds", "ObjectInfo", 0, 1)];
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut custom = Definition::from_script("CLNK", "Clonk Type", script)
            .expect("custom-rank reflection fixture compiles");
        custom.set_crew_member(true);
        custom.set_rank_system(Some(shipped_clonk_rank_names()), Some(1_000));
        engine.register_definition(custom).expect("custom crew registers");
        let mut plain = Definition::from_script("NONE", "Plain Type", script)
            .expect("plain-rank reflection fixture compiles");
        plain.set_crew_member(true);
        engine.register_definition(plain).expect("plain crew registers");
        let mut half = Definition::from_script(
            "HALF",
            "123456789012345678901234567890X",
            script,
        )
            .expect("custom-base reflection fixture compiles");
        half.set_crew_member(true);
        half.set_rank_system(
            Some(vec!["Recruit".to_string(), "Veteran".to_string()]),
            Some(500),
        );
        engine.register_definition(half).expect("half-base crew registers");

        let mut start = PlayerStart::default();
        start.ready_crew = vec![
            ("CLNK".to_string(), 1),
            ("NONE".to_string(), 1),
            ("HALF".to_string(), 1),
        ];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(rank_join_config(Vec::new()))
            .expect("fresh infos are created");

        let crew = engine.player(0).expect("rank owner exists").crew().to_vec();
        let read = |engine: &mut Engine, definition: &str| {
            let object = *crew
                .iter()
                .find(|object| {
                    engine
                        .crew_object_info(**object)
                        .is_some_and(|info| info.definition_id.as_str() == definition)
                })
                .expect("requested crew object exists");
            let index = engine.find_object_index(object).expect("crew object remains live");
            engine
                .call_object_function(index, "ReadCore", Vec::new())
                .expect("core reflection succeeds")
        };

        assert_eq!(
            read(&mut engine, "CLNK"),
            Value::Array(vec![
                Value::C4Id("CLNK".to_string()),
                Value::String("Clonk".to_string().into()),
                Value::String(String::new().into()),
                Value::String(String::new().into()),
                Value::Int(0),
                Value::String("Clonk".to_string().into()),
                Value::String("Ensign".to_string().into()),
                Value::String("Clonk Type".to_string().into()),
                Value::Int(1),
                Value::Int(0),
                Value::Int(1_000),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(50_000),
                Value::Nil,
            ])
        );
        let Value::Array(plain) = read(&mut engine, "NONE") else {
            panic!("plain info reflection must return an array");
        };
        assert_eq!(plain[6], Value::String(String::new().into()));
        assert_eq!(plain[7], Value::String("Plain Type".to_string().into()));
        assert_eq!(plain[10], Value::Int(0));
        let Value::Array(half) = read(&mut engine, "HALF") else {
            panic!("half-base info reflection must return an array");
        };
        assert_eq!(half[6], Value::String("Veteran".to_string().into()));
        assert_eq!(
            half[7],
            Value::String("123456789012345678901234567890".to_string().into())
        );
        assert_eq!(half[10], Value::Int(500));
    }

    #[test]
    fn object_info_core_extra_data_reflects_scalar_compiler_primitives() {
        // C4ValueMapData::CompileFunc emits the count, then each raw name and
        // C4Value's type tag/payload. C4Value::CompileFunc serializes bool,
        // C4ID, object, and string payloads as integer primitives; a runtime
        // string that has not passed EnumStrings still carries enum ID -1.
        let script = r#"#strict 2
func ReadExtra(int entry_nr)
{
    var no_section;
    return GetObjectInfoCoreVal("ExtraData", "ObjectInfo", no_section, entry_nr);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("TARG"))
            .expect("target definition registers");
        let mut crew = Definition::from_script("CLNK", "Clonk", script)
            .expect("extra-data reflection fixture compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("object-payload target spawns");

        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CLNK".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(rank_join_config(vec![crew_info_with_extra_data(vec![
                ("number".to_string(), Value::Int(-7)),
                ("flag".to_string(), Value::Bool(true)),
                ("kind".to_string(), Value::C4Id("ABCD".to_string())),
                ("target".to_string(), Value::Object(target.as_u64())),
                ("text".to_string(), Value::String("hello".to_string().into())),
            ])]))
            .expect("extra-data owner joins");
        let crew = engine.player(0).expect("owner exists").crew()[0];
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        let mut read = |entry_nr| {
            engine
                .call_object_function(crew_index, "ReadExtra", vec![Value::Int(entry_nr)])
                .expect("extra-data primitive reflects")
        };

        let expected = [
            Value::Int(5),
            Value::String("number".to_string().into()),
            Value::String("i".to_string().into()),
            Value::Int(-7),
            Value::String("flag".to_string().into()),
            Value::String("b".to_string().into()),
            Value::Int(1),
            Value::String("kind".to_string().into()),
            Value::String("I".to_string().into()),
            Value::Int(clonk_script::c4_id_raw("ABCD") as i32),
            Value::String("target".to_string().into()),
            Value::String("O".to_string().into()),
            Value::Int(target.as_u64() as i32),
            Value::String("text".to_string().into()),
            Value::String("S".to_string().into()),
            Value::Int(-1),
        ];
        let out_of_range = expected.len() as i32;
        for (entry_nr, expected) in expected.into_iter().enumerate() {
            assert_eq!(read(entry_nr as i32), expected, "EntryNr {entry_nr}");
        }
        assert_eq!(read(out_of_range), Value::Nil);
        assert_eq!(read(-1), Value::Nil);
        assert_eq!(read(i32::MIN), Value::Nil, "signed-overflow input is hardened");
    }

    #[test]
    fn object_info_core_extra_data_recurses_through_arrays_and_maps() {
        // C4Value arrays and maps serialize depth-first. C4ValueHash iterates
        // its stable keyOrder, represented by ValueMap's insertion order.
        let script = r#"#strict 2
func ReadExtra(int entry_nr)
{
    var no_section;
    return GetObjectInfoCoreVal("ExtraData", "ObjectInfo", no_section, entry_nr);
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut crew = Definition::from_script("CLNK", "Clonk", script)
            .expect("recursive extra-data fixture compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");

        let mut map = clonk_script::ValueMap::new();
        map.insert_key(Value::Int(1), Value::Bool(true));
        map.insert_key(
            Value::C4Id("ABCD".to_string()),
            Value::Array(vec![Value::Int(-3), Value::Nil]),
        );
        let tree = Value::Array(vec![Value::Int(7), Value::Proplist(map)]);
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CLNK".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(rank_join_config(vec![crew_info_with_extra_data(vec![(
                "tree".to_string(),
                tree,
            )])]))
            .expect("recursive extra-data owner joins");
        let crew = engine.player(0).expect("owner exists").crew()[0];
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        let mut read = |entry_nr| {
            engine
                .call_object_function(crew_index, "ReadExtra", vec![Value::Int(entry_nr)])
                .expect("recursive extra-data primitive reflects")
        };

        let expected = [
            Value::Int(1),
            Value::String("tree".to_string().into()),
            Value::String("a".to_string().into()),
            Value::Int(2),
            Value::String("i".to_string().into()),
            Value::Int(7),
            Value::String("m".to_string().into()),
            Value::Int(2),
            Value::String("i".to_string().into()),
            Value::Int(1),
            Value::String("b".to_string().into()),
            Value::Int(1),
            Value::String("I".to_string().into()),
            Value::Int(clonk_script::c4_id_raw("ABCD") as i32),
            Value::String("a".to_string().into()),
            Value::Int(2),
            Value::String("i".to_string().into()),
            Value::Int(-3),
            Value::String("A".to_string().into()),
            Value::Int(0),
        ];
        let out_of_range = expected.len() as i32;
        for (entry_nr, expected) in expected.into_iter().enumerate() {
            assert_eq!(read(entry_nr as i32), expected, "EntryNr {entry_nr}");
        }
        assert_eq!(read(out_of_range), Value::Nil);
    }

    #[test]
    fn custom_progression_refreshes_on_save_and_exhausts_at_true_shipped_boundary() {
        let rank_names = shipped_clonk_rank_names();
        assert_eq!(rank_names.len(), 144, "24 base names times six tiers");
        assert_eq!(rank_names[137], "Exalted Fleet Admiral");
        assert_eq!(rank_names[143], "Exalted Premier");

        let script = r#"#strict 2
func ReadNext()
{
    return [GetObjectInfoCoreVal("RankName", "ObjectInfo"),
            GetObjectInfoCoreVal("NextRankName", "ObjectInfo"),
            GetObjectInfoCoreVal("NextRankExp", "ObjectInfo")];
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut definition = Definition::from_script("CLNK", "Clonk", script)
            .expect("save-refresh fixture compiles");
        definition.set_crew_member(true);
        definition.set_rank_system(Some(rank_names), Some(1_000));
        engine
            .register_definition(definition)
            .expect("save-refresh crew registers");
        let loaded = |name: &str, rank: i32, experience: i32| player_file::CrewInfo {
            id: "CLNK".to_string(),
            name: name.to_string(),
            death_message: String::new(),
            core: CrewInfoCoreFields {
                next_rank_name: "stale next".to_string(),
                next_rank_exp: 777,
                ..CrewInfoCoreFields::default()
            },
            rank,
            rank_name: "stale current".to_string(),
            experience,
            rounds: 0,
            physical: PhysicalInfo::default(),
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
        };
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CLNK".to_string(), 2)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(rank_join_config(vec![
                loaded("Rank 137", 137, 1),
                loaded("Rank 143", 143, 2),
            ]))
            .expect("loaded rank infos join");

        let crew = engine.player(0).expect("rank owner exists").crew().to_vec();
        let by_rank = |engine: &Engine, rank: i32| {
            *crew
                .iter()
                .find(|object| engine.crew_object_info(**object).is_some_and(|info| info.rank == rank))
                .expect("requested rank remains live")
        };
        for rank in [137, 143] {
            let object = by_rank(&engine, rank);
            let index = engine.find_object_index(object).expect("ranked crew remains live");
            let Value::Array(before) = engine
                .call_object_function(index, "ReadNext", Vec::new())
                .expect("loaded fields reflect")
            else {
                panic!("rank reflection must return an array");
            };
            assert_eq!(before[1], Value::String("stale next".to_string().into()));
            assert_eq!(before[2], Value::Int(777), "load must not recompute progression");
        }

        engine
            .set_player_status(0, PlayerStatus::Eliminated)
            .expect("rank owner status changes");
        engine
            .execute_synchronize_control(true, false)
            .expect("eliminated-player synchronization completes");
        assert_eq!(
            engine
                .crew_object_info(by_rank(&engine, 143))
                .expect("last-rank info remains live")
                .core
                .next_rank_exp,
            777,
            "eliminated players are not synchronized to local files"
        );
        engine
            .set_player_status(0, PlayerStatus::Active)
            .expect("rank owner reactivates for replay probe");
        engine.set_replay_control(true);
        engine
            .execute_synchronize_control(true, false)
            .expect("replay synchronization completes");
        assert_eq!(
            engine
                .crew_object_info(by_rank(&engine, 143))
                .expect("last-rank info remains live")
                .core
                .next_rank_exp,
            777,
            "C++ suppresses local crew-file saves during replay"
        );
        engine.set_replay_control(false);
        engine
            .execute_synchronize_control(true, false)
            .expect("save synchronization completes");

        let rank_137 = by_rank(&engine, 137);
        let rank_137_index = engine.find_object_index(rank_137).expect("rank 137 remains live");
        assert_eq!(
            engine
                .call_object_function(rank_137_index, "ReadNext", Vec::new())
                .expect("rank 137 reflects refreshed progression"),
            Value::Array(vec![
                Value::String("Exalted Fleet Admiral".to_string().into()),
                Value::String("Exalted Counsellor of State".to_string().into()),
                Value::Int(1_621_132),
            ]),
            "the ticket's old rank-137 exhaustion boundary was based on 23, not 24, base names"
        );
        let rank_143 = by_rank(&engine, 143);
        let rank_143_index = engine.find_object_index(rank_143).expect("rank 143 remains live");
        assert_eq!(
            engine
                .call_object_function(rank_143_index, "ReadNext", Vec::new())
                .expect("last rank reflects exhaustion"),
            Value::Array(vec![
                Value::String("Exalted Premier".to_string().into()),
                Value::String(String::new().into()),
                Value::Int(-1),
            ])
        );

        let state = engine.capture_state();
        for object in [rank_137, rank_143] {
            let link = state.crew_info_links[&object];
            let roster = &state.crew_info_rosters[&link.player_id][link.roster_index];
            let live = state.crew_object_infos.get(&object).expect("live info persists");
            assert_eq!(roster.core, live.core, "save refresh mirrors the shared C++ pointer");
        }
    }

    #[test]
    fn do_crew_exp_promotes_once_updates_same_call_and_persists() {
        let script = r#"#strict 2
func Award(int amount, object target) {
    return [DoCrewExp(amount, target), GetRank(target),
            GetObjectInfoCoreVal("Experience", "ObjectInfo", target)];
}
func AwardSelf(int amount) {
    return [DoCrewExp(amount), GetRank(),
            GetObjectInfoCoreVal("Rank", "ObjectInfo"),
            GetObjectInfoCoreVal("RankName", "ObjectInfo"),
            GetObjectInfoCoreVal("Experience", "ObjectInfo")];
}
"#;
        let mut engine = Engine::with_seed(0);
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("experience probe compiles");
        crew.set_crew_member(true);
        crew.set_rank_system(
            Some(vec![
                "Recruit".to_string(),
                "Custom One".to_string(),
                "Custom Two".to_string(),
            ]),
            Some(500),
        );
        engine.register_definition(crew).expect("crew registers");

        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CREW".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(JoinPlayerConfig {
                name: "Experience owner".to_string(),
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
                crew: vec![player_file::CrewInfo {
                    id: "CREW".to_string(),
                    name: "Rookie".to_string(),
                    ..Default::default()
                }],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("experience owner joins");
        let crew_id = engine.player(0).expect("player").crew()[0];
        let crew_index = engine.find_object_index(crew_id).expect("crew index");
        let raw_physical = PhysicalInfo {
            energy: 10_000,
            breath: 12_345,
            walk: 23_456,
            can_fly: 7,
            corrosion_resist: 8,
            breathe_water: 9,
            ..PhysicalInfo::default()
        };
        engine.objects[crew_index].state.info_physical = Some(raw_physical);
        engine.objects[crew_index].state.energy = 12_000;
        engine.pending_audio.clear();
        assert!(engine.use_fair_crew());
        let fair_physical_before = engine.object_physical(crew_index);

        assert_eq!(
            engine
                .call_object_function(crew_index, "AwardSelf", vec![Value::Int(500)])
                .expect("custom-base threshold probe succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(0),
                Value::Int(0),
                Value::String("Recruit".to_string().into()),
                Value::Int(500),
            ]),
            "DoExperience waits for the global base-1000 curve even when the definition uses base 500"
        );

        assert_eq!(
            engine
                .call_object_function(crew_index, "AwardSelf", vec![Value::Int(7_500)])
                .expect("large award succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(1),
                Value::Int(1),
                Value::String("Custom One".to_string().into()),
                Value::Int(8_000),
            ]),
            "DoExperience promotes at most one rank and exposes the write immediately"
        );
        assert_eq!(
            engine
                .pending_audio
                .iter()
                .filter(|command| matches!(
                    command,
                    AudioCommand::PlaySound {
                        name,
                        target,
                        volume: 100,
                        looped: false,
                        ..
                    } if name == "Trumpet" && *target == Some(crew_id)
                ))
                .count(),
            1,
            "the host preview presents the promotion once; its deferred command must not repeat it"
        );
        let promoted_object = engine
            .object_snapshot(crew_id)
            .expect("promoted crew remains live");
        assert_eq!(promoted_object.energy, 12_000, "promotion does not heal live Energy");
        let promoted_physical = promoted_object
            .info_physical
            .expect("DoCrewExp writes raw info physicals");
        assert_eq!(promoted_physical.energy, 55_000);
        assert_eq!(
            (
                promoted_physical.can_dig,
                promoted_physical.can_chop,
                promoted_physical.can_construct,
                promoted_physical.can_scale,
                promoted_physical.can_hangle,
            ),
            (1, 1, 1, 1, 1)
        );
        assert_eq!(promoted_physical.breath, raw_physical.breath);
        assert_eq!(promoted_physical.walk, raw_physical.walk);
        assert_eq!(promoted_physical.can_fly, raw_physical.can_fly);
        assert_eq!(
            promoted_physical.corrosion_resist,
            raw_physical.corrosion_resist
        );
        assert_eq!(promoted_physical.breathe_water, raw_physical.breathe_water);
        assert_eq!(
            engine.object_physical(crew_index),
            fair_physical_before,
            "promotion updates raw Info physicals but not FairCrew's effective physical"
        );
        let promotions = engine
            .capture_state()
            .messages
            .into_iter()
            .filter(|message| message.snapshot.target == Some(crew_id))
            .collect::<Vec<_>>();
        assert_eq!(promotions.len(), 1);
        let promotion = &promotions[0];
        assert_eq!(
            promotion.snapshot.lines,
            vec!["Rookie is promoted".to_string(), "to Custom One!".to_string()]
        );

        assert_eq!(
            engine
                .call_object_function(crew_index, "AwardSelf", vec![Value::Int(0)])
                .expect("zero award can catch up one rank"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(2),
                Value::Int(2),
                Value::String("Custom Two".to_string().into()),
                Value::Int(8_000),
            ])
        );
        assert_eq!(
            engine
                .call_object_function(crew_index, "AwardSelf", vec![Value::Int(-20_000)])
                .expect("negative award clamps"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(2),
                Value::Int(2),
                Value::String("Custom Two".to_string().into()),
                Value::Int(0),
            ]),
            "experience clamps at zero without demoting"
        );

        let message_count = engine.capture_state().messages.len();
        engine.pending_audio.clear();
        assert_eq!(
            engine
                .call_object_function(crew_index, "AwardSelf", vec![Value::Int(5_196)])
                .expect("promotion beyond the custom table succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(3),
                Value::Int(3),
                Value::String("Custom Two".to_string().into()),
                Value::Int(5_196),
            ])
        );
        assert_eq!(engine.capture_state().messages.len(), message_count);
        assert!(!engine.pending_audio.iter().any(|command| matches!(
            command,
            AudioCommand::PlaySound { name, target, .. }
                if name == "Trumpet" && *target == Some(crew_id)
        )), "an undefined custom rank promotes silently");
        assert_eq!(
            engine.object_physical(crew_index),
            fair_physical_before,
            "silent promotion also leaves FairCrew's effective physical unchanged"
        );
        assert_eq!(
            engine
                .call_object_function(crew_index, "AwardSelf", vec![Value::Int(-20_000)])
                .expect("experience resets after the silent promotion"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(3),
                Value::Int(3),
                Value::String("Custom Two".to_string().into()),
                Value::Int(0),
            ])
        );
        assert_eq!(
            engine
                .call_object_function(
                    crew_index,
                    "AwardSelf",
                    vec![Value::Int(100_000_000)],
                )
                .expect("maximum award clamps"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(3),
                Value::Int(3),
                Value::String("Custom Two".to_string().into()),
                Value::Int(100_000_000),
            ]),
            "the exact maximum suppresses promotion"
        );

        let persisted = engine.capture_state();
        let link = persisted.crew_info_links[&crew_id];
        let roster_info = &persisted.crew_info_rosters[&link.player_id][link.roster_index];
        assert_eq!((roster_info.rank, roster_info.experience), (3, 100_000_000));
        assert_eq!(roster_info.rank_name, "Custom Two");

        let encoded = persisted.to_json_string().expect("promotion state serializes");
        let restored = EngineState::from_json_str(&encoded).expect("promotion state deserializes");
        engine.restore_state(&restored).expect("promotion state restores");
        assert_eq!(
            engine
                .crew_object_info(crew_id)
                .expect("restored crew keeps info")
                .rank_name,
            "Custom Two"
        );

        let info_less = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("info-less object spawns");
        assert_eq!(
            engine
                .call_object_function(
                    crew_index,
                    "Award",
                    vec![Value::Int(5), Value::Object(info_less.as_u64())],
                )
                .expect("info-less award succeeds"),
            Value::Array(vec![Value::Bool(true), Value::Nil, Value::Nil])
        );

    }

    #[test]
    fn set_crew_status_retire_accrues_one_stint_and_recruit_restarts_it() {
        let script = r#"#strict 2
func Join() { return MakeCrewMember(this(), 0); }
func Retire() { return SetCrewStatus(0, false); }
func Recruit(object target) { return MakeCrewMember(target, 0); }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Playing time"))
            .expect("player registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("playing-time probe compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("target spawns");
        let target_index = engine.find_object_index(target).expect("target index");

        engine.game_time = 10;
        engine
            .call_object_function(target_index, "Join", Vec::new())
            .expect("target recruits");
        let link = *engine
            .capture_state()
            .crew_info_links
            .get(&target)
            .expect("target has an exact roster link");

        engine.game_time = 17;
        assert_eq!(
            engine
                .call_object_function(target_index, "Retire", Vec::new())
                .expect("first retirement completes"),
            Value::Int(1)
        );
        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("retired roster serializes");
        let retired = EngineState::from_json_str(&encoded).expect("retired roster deserializes");
        let entry = &retired.crew_info_rosters[&0][link.roster_index];
        assert!(!entry.in_action);
        assert_eq!(entry.total_playing_time, 7);

        engine.game_time = 21;
        assert_eq!(
            engine
                .call_object_function(target_index, "Retire", Vec::new())
                .expect("idempotent retirement completes"),
            Value::Int(1)
        );
        assert_eq!(
            engine.capture_state().crew_info_rosters[&0][link.roster_index]
                .total_playing_time,
            7,
            "already-absent removal cannot accrue the inactive interval"
        );

        let replacement = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("replacement spawns");
        engine.game_time = 23;
        assert_eq!(
            engine
                .call_object_function(
                    target_index,
                    "Recruit",
                    vec![Value::Object(replacement.as_u64())],
                )
                .expect("replacement recruits the retired info"),
            Value::Bool(true)
        );
        let recruited = engine.capture_state();
        assert_eq!(recruited.crew_info_links[&replacement], link);
        let entry = &recruited.crew_info_rosters[&0][link.roster_index];
        assert!(entry.in_action);
        assert_eq!(entry.in_action_time, 23);
        assert_eq!(entry.total_playing_time, 7);
    }

    #[test]
    fn set_crew_status_observes_callback_info_transfer_and_readd_mutations() {
        let script = r#"#strict 2
local callback_mode;
local callback_player;
local callback_target;

func Join(int player) { return MakeCrewMember(this(), player); }
func RemoveWithTransfer(int player, object target) {
    callback_mode = 1;
    callback_player = player;
    callback_target = target;
    SelectCrew(player, this(), true, true);
    return SetCrewStatus(player, false);
}
func RemoveWithReadd(int player) {
    callback_mode = 2;
    callback_player = player;
    SelectCrew(player, this(), true, true);
    return SetCrewStatus(player, false);
}
func CrewSelection(bool unselect, bool cursor) {
    if (!unselect) return 1;
    if (callback_mode == 1) callback_target->GrabObjectInfo(this());
    if (callback_mode == 2) SetCrewStatus(callback_player, true);
    return 1;
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Callback owner"))
            .expect("player registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("callback-mutation probe compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");

        let donor = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("transfer donor spawns");
        let receiver = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("transfer receiver spawns");
        let donor_index = engine.find_object_index(donor).expect("donor index");
        engine
            .call_object_function(donor_index, "Join", vec![Value::Int(0)])
            .expect("donor joins");
        let donor_info = engine
            .crew_object_info(donor)
            .expect("donor info exists")
            .clone();
        assert_eq!(
            engine
                .call_object_function(
                    donor_index,
                    "RemoveWithTransfer",
                    vec![Value::Int(0), Value::Object(receiver.as_u64())],
                )
                .expect("CrewSelection transfers info during removal"),
            Value::Int(1)
        );
        assert!(engine.crew_object_info(donor).is_none());
        assert_eq!(engine.crew_object_info(receiver), Some(&donor_info));
        assert_eq!(engine.player(0).expect("player").crew(), &[receiver]);

        let readded = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("readd target spawns");
        let readded_index = engine.find_object_index(readded).expect("readd index");
        engine
            .call_object_function(readded_index, "Join", vec![Value::Int(0)])
            .expect("readd target joins");
        assert_eq!(
            engine
                .call_object_function(
                    readded_index,
                    "RemoveWithReadd",
                    vec![Value::Int(0)],
                )
                .expect("CrewSelection readds during removal"),
            Value::Int(1)
        );
        assert!(
            engine.player(0).expect("player").crew().contains(&readded),
            "the synchronous callback's readd survives the outer removal"
        );
        assert!(
            engine.crew_object_info(readded).is_none(),
            "the outer removal still retires the current same-player Info after callback"
        );
    }

    #[test]
    fn grab_object_info_clears_every_roster_then_recruits_only_to_receiver_owner() {
        let script = r#"#strict 2
func JoinAndShare(int owner, int other) {
    var made = MakeCrewMember(this(), owner);
    var shared = SetCrewStatus(other, true);
    return [made, shared];
}
func Take(object donor) { return GrabObjectInfo(donor); }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Donor owner"))
            .expect("donor owner registers");
        engine
            .register_player(PlayerConfig::new(1, "Receiver owner"))
            .expect("receiver owner registers");
        for id in ["DONR", "RCVR"] {
            let mut crew = Definition::from_script(id, id, script)
                .expect("GrabObjectInfo roster probe compiles");
            crew.set_crew_member(true);
            engine.register_definition(crew).expect("crew registers");
        }
        let donor = engine
            .spawn_object(
                SpawnConfig::new("DONR")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("donor spawns");
        let receiver = engine
            .spawn_object(
                SpawnConfig::new("RCVR")
                    .with_owner(1)
                    .with_crew_member(false),
            )
            .expect("receiver spawns");
        for (object, owner, other) in [(donor, 0, 1), (receiver, 1, 0)] {
            let index = engine.find_object_index(object).expect("crew index");
            assert_eq!(
                engine
                    .call_object_function(
                        index,
                        "JoinAndShare",
                        vec![Value::Int(owner), Value::Int(other)],
                    )
                    .expect("cross-shared member joins"),
                Value::Array(vec![Value::Bool(true), Value::Int(1)])
            );
        }
        engine
            .set_crew_cursor(0, Some(donor))
            .expect("donor cursor installs");
        engine
            .set_crew_cursor(1, Some(receiver))
            .expect("receiver cursor installs");
        let receiver_index = engine.find_object_index(receiver).expect("receiver index");
        assert_eq!(
            engine
                .call_object_function(
                    receiver_index,
                    "Take",
                    vec![Value::Object(donor.as_u64())],
                )
                .expect("receiver grabs donor info"),
            Value::Bool(true)
        );

        assert!(engine.player(0).expect("donor owner").crew().is_empty());
        assert_eq!(
            engine.player(1).expect("receiver owner").crew(),
            &[receiver],
            "MakeCrewMember runs only for the receiver's Owner after both clears"
        );
        assert!(engine.crew_object_info(donor).is_none());
        assert_eq!(
            engine
                .crew_object_info(receiver)
                .expect("donor info moved")
                .definition_id
                .as_str(),
            "DONR"
        );
        assert!(!engine.object_snapshot(donor).expect("donor remains").crew_member);
        assert!(
            engine
                .object_snapshot(receiver)
                .expect("receiver remains")
                .crew_member
        );
        assert_eq!(engine.crew_cursor(0), None);
        assert_eq!(engine.crew_cursor(1), None);
    }

    #[test]
    fn grab_object_info_moves_the_full_identity_and_rewrites_life_status() {
        let receiver_script = r#"#strict 2
func Take(object donor) {
    return [GrabObjectInfo(donor), GetName(), GetRank(),
            GetObjectInfoCoreVal("RankName", "ObjectInfo"),
            GetObjectInfoCoreVal("Experience", "ObjectInfo"),
            GetName(donor), GetRank(donor),
            GetCrewCount(0), GetCrewCount(1)];
}
func Try(object donor) { return GrabObjectInfo(donor); }
func SelfOf(object target) { return GrabObjectInfo(target, target); }
"#;
        let donor_script = r#"#strict 2
func RemoveAndGrabSelf() {
    RemoveObject();
    return GrabObjectInfo(this(), this());
}
"#;

        let mut engine = Engine::with_seed(0);
        let mut donor_definition =
            Definition::from_script("DONR", "Donor definition", donor_script)
                .expect("donor definition compiles");
        donor_definition.set_crew_member(true);
        donor_definition.set_rank_names(Some(vec![
            "Custom Recruit".to_string(),
            "Custom Veteran".to_string(),
        ]));
        engine
            .register_definition(donor_definition)
            .expect("donor definition registers");
        let mut receiver_definition =
            Definition::from_script("RCVR", "Receiver definition", receiver_script)
                .expect("receiver definition compiles");
        receiver_definition.set_crew_member(true);
        engine
            .register_definition(receiver_definition)
            .expect("receiver definition registers");

        let mut start = PlayerStart::default();
        start.ready_crew = vec![("DONR".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(JoinPlayerConfig {
                name: "Identity owner".to_string(),
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
                crew: vec![player_file::CrewInfo {
                    id: "DONR".to_string(),
                    name: "Veteran Ada".to_string(),
                    rank: 4,
                    rank_name: "Major".to_string(),
                    experience: 8_000,
                    total_playing_time: 17,
                    participation: 3,
                    ..Default::default()
                }],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("identity owner joins");
        engine
            .register_player(PlayerConfig::new(1, "Receiver owner"))
            .expect("receiver owner registers");

        let donor = engine.player(0).expect("donor player").crew()[0];
        let donor_info = engine
            .crew_object_info(donor)
            .expect("donor starts with persistent info")
            .clone();
        assert_eq!(
            donor_info.rank_name, "Custom Veteran",
            "Recruit clamps an over-table custom rank to the last name"
        );
        let original_link = engine.capture_state().crew_info_links[&donor];
        let receiver = engine
            .spawn_object(
                SpawnConfig::new("RCVR")
                    .with_owner(1)
                    .with_crew_member(false)
                    .with_alive(true),
            )
            .expect("live receiver spawns");
        let receiver_index = engine.find_object_index(receiver).expect("receiver index");

        assert_eq!(
            engine
                .call_object_function(
                    receiver_index,
                    "Take",
                    vec![Value::Object(donor.as_u64())],
                )
                .expect("live identity transfer succeeds"),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("Veteran Ada".to_string().into()),
                Value::Int(4),
                Value::String("Custom Veteran".to_string().into()),
                Value::Int(8_000),
                Value::String("Donor definition".to_string().into()),
                Value::Nil,
                Value::Int(0),
                Value::Int(1),
            ])
        );
        assert!(engine.crew_object_info(donor).is_none());
        assert_eq!(engine.crew_object_info(receiver), Some(&donor_info));
        assert!(engine.player(0).expect("donor player").crew().is_empty());
        assert_eq!(engine.player(1).expect("receiver player").crew(), &[receiver]);
        let live_state = engine.capture_state();
        assert_eq!(live_state.crew_info_links[&receiver], original_link);
        let live_entry =
            &live_state.crew_info_rosters[&original_link.player_id][original_link.roster_index];
        assert!(live_entry.in_action);
        assert!(!live_entry.has_died);

        let info_less = engine
            .spawn_object(
                SpawnConfig::new("DONR")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("info-less donor spawns");
        assert_eq!(
            engine
                .call_object_function(
                    receiver_index,
                    "SelfOf",
                    vec![Value::Object(info_less.as_u64())],
                )
                .expect("active self-grab succeeds without info"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(
                    receiver_index,
                    "Try",
                    vec![Value::Object(info_less.as_u64())],
                )
                .expect("info-less donor is rejected"),
            Value::Bool(false)
        );
        assert_eq!(engine.crew_object_info(receiver), Some(&donor_info));

        let info_less_index = engine
            .find_object_index(info_less)
            .expect("info-less donor index");
        assert_eq!(
            engine
                .call_object_function(info_less_index, "RemoveAndGrabSelf", Vec::new())
                .expect("deleted self probe completes"),
            Value::Bool(false),
            "C4Object::GrabInfo checks Status before its self-transfer fast path"
        );

        let dead_receiver = engine
            .spawn_object(
                SpawnConfig::new("RCVR")
                    .with_owner(1)
                    .with_crew_member(false)
                    .with_alive(false),
            )
            .expect("dead receiver spawns");
        let dead_index = engine
            .find_object_index(dead_receiver)
            .expect("dead receiver index");
        assert_eq!(
            engine
                .call_object_function(
                    dead_index,
                    "Take",
                    vec![Value::Object(receiver.as_u64())],
                )
                .expect("dead receiver takes the same identity"),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("Veteran Ada".to_string().into()),
                Value::Int(4),
                Value::String("Custom Veteran".to_string().into()),
                Value::Int(8_000),
                Value::String("Receiver definition".to_string().into()),
                Value::Nil,
                Value::Int(0),
                Value::Int(1),
            ])
        );
        assert!(engine.crew_object_info(receiver).is_none());
        assert_eq!(engine.crew_object_info(dead_receiver), Some(&donor_info));
        assert_eq!(
            engine.player(1).expect("receiver player").crew(),
            &[dead_receiver],
            "MakeCrewMember still adds a dead receiver to its owner's crew"
        );
        let dead_state = engine.capture_state();
        assert_eq!(dead_state.crew_info_links[&dead_receiver], original_link);
        let dead_entry =
            &dead_state.crew_info_rosters[&original_link.player_id][original_link.roster_index];
        assert!(!dead_entry.in_action);
        assert!(dead_entry.has_died);
    }

    #[test]
    fn crew_info_owner_link_survives_save_restore_and_foreign_removal() {
        let script = r#"#strict 2
func Setup() {
    MakeCrewMember(this(), 0);
    SetCrewStatus(1, true);
    return [GetRank(), GetCrewCount(0), GetCrewCount(1)];
}
func RemoveFrom(int player) { return SetCrewStatus(player, false); }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Info owner"))
            .expect("info owner registers");
        engine
            .register_player(PlayerConfig::new(1, "Foreign roster"))
            .expect("foreign player registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("save-info-link probe compiles");
        crew.set_crew_member(true);
        engine
            .register_definition(crew.clone())
            .expect("crew registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("target spawns");
        let target_index = engine.find_object_index(target).expect("target index");
        assert_eq!(
            engine
                .call_object_function(target_index, "Setup", Vec::new())
                .expect("target gets owned info and shared membership"),
            Value::Array(vec![Value::Int(0), Value::Int(1), Value::Int(1)])
        );
        let expected_info = engine
            .crew_object_info(target)
            .expect("target has info")
            .clone();
        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("crew state serializes");
        let state = EngineState::from_json_str(&encoded).expect("crew state deserializes");

        let mut restored = Engine::with_seed(9);
        restored
            .register_definition(crew)
            .expect("definition registers for restore");
        restored.restore_state(&state).expect("crew state restores");
        assert_eq!(restored.crew_object_info(target), Some(&expected_info));
        assert_eq!(restored.player(0).expect("info owner").crew(), &[target]);
        assert_eq!(
            restored.player(1).expect("foreign player").crew(),
            &[target]
        );

        let restored_index = restored.find_object_index(target).expect("restored index");
        assert_eq!(
            restored
                .call_object_function(
                    restored_index,
                    "RemoveFrom",
                    vec![Value::Int(1)],
                )
                .expect("foreign membership removes"),
            Value::Int(1)
        );
        assert_eq!(
            restored.crew_object_info(target),
            Some(&expected_info),
            "foreign CrewInfoList must not retire the restored pointer"
        );
        assert_eq!(
            restored
                .call_object_function(
                    restored_index,
                    "RemoveFrom",
                    vec![Value::Int(0)],
                )
                .expect("own membership removes"),
            Value::Int(1)
        );
        assert!(restored.crew_object_info(target).is_none());
    }

    #[test]
    fn pre_roster_engine_state_imports_legacy_union_but_modern_empty_is_exact() {
        let mut engine = Engine::with_seed(0);
        let mut crew = simple_definition("CREW");
        crew.set_crew_member(true);
        engine
            .register_definition(crew.clone())
            .expect("crew registers");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(true),
            )
            .expect("legacy union member spawns");
        engine
            .register_player(PlayerConfig::new(0, "Legacy roster"))
            .expect("player registers");

        let mut legacy = engine.capture_state();
        legacy.player_crew_rosters_authoritative = false;
        legacy.players[0].crew.clear();
        let mut restored = Engine::with_seed(1);
        restored
            .register_definition(crew.clone())
            .expect("crew registers for legacy restore");
        restored
            .restore_state(&legacy)
            .expect("legacy state restores");
        assert_eq!(restored.player(0).expect("player").crew(), &[target]);

        let mut modern = legacy;
        modern.player_crew_rosters_authoritative = true;
        let mut restored = Engine::with_seed(2);
        restored
            .register_definition(crew)
            .expect("crew registers for modern restore");
        restored
            .restore_state(&modern)
            .expect("modern empty roster restores");
        assert!(restored.player(0).expect("player").crew().is_empty());
    }

    #[test]
    fn scenario_script_after_players_waits_for_the_next_tick35_elimination_check() {
        let object_script = r#"#strict 2
func Join() { return MakeCrewMember(this(), 0); }
func LateRemove() { return SetCrewStatus(0, false); }
"#;
        let scenario_script = r#"#strict 2
protected func Script0() { return 1; }
protected func Script1() { return 1; }
protected func Script2() { return 1; }
protected func Script3() { return 1; }
protected func Script4() { return 1; }
protected func Script5() { return 1; }
protected func Script6() { FindObject(CREW)->LateRemove(); return 1; }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Late removal"))
            .expect("player registers");
        let mut crew = Definition::from_script("CREW", "Crew", object_script)
            .expect("late-removal crew compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");
        engine
            .install_scenario_script_with_convention("Late removal", scenario_script, true)
            .expect("scenario script installs");
        let target = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("crew target spawns");
        let target_index = engine.find_object_index(target).expect("target index");
        engine
            .call_object_function(target_index, "Join", Vec::new())
            .expect("target joins player crew");
        engine.scenario_script_go = true;

        for _ in 0..70 {
            engine.tick_without_snapshot().expect("frame through late Script6 advances");
        }
        assert!(engine.player(0).expect("player").crew().is_empty());
        assert!(
            !engine.is_owner_eliminated(0),
            "Players.Execute checked CrewCnt before Script.Execute removed the member"
        );

        for _ in 70..105 {
            engine.tick_without_snapshot().expect("next Tick35 window advances");
        }
        assert!(engine.is_owner_eliminated(0));
    }

    #[test]
    fn object_phase_created_crew_exists_for_the_same_tick35_player_check() {
        let mut creator = Definition::from_script(
            "MAKE",
            "Crew creator",
            "#strict 2\nfunc Seed() { var crew = CreateObject(CREW, 0, 0, 0); MakeCrewMember(crew, 0); return 1; }\n",
        )
        .expect("creator compiles");
        creator.set_timer(35);
        creator.set_timer_call(Some("Seed".to_string()));
        let mut crew = simple_definition("CREW");
        crew.set_crew_member(true);

        let mut engine = Engine::with_seed(0);
        engine.register_definition(creator).expect("creator registers");
        engine.register_definition(crew).expect("crew registers");
        engine
            .register_player(PlayerConfig::new(0, "Tick35 recruit"))
            .expect("player registers");
        engine
            .spawn_object(SpawnConfig::new("MAKE"))
            .expect("creator spawns");

        for _ in 0..35 {
            engine.tick_without_snapshot().expect("Tick35 recruit window advances");
        }
        assert_eq!(engine.player(0).expect("player").crew().len(), 1);
        assert!(
            !engine.is_owner_eliminated(0),
            "object-phase CreateObject is live before Players.Execute snapshots CrewCnt"
        );
    }

    #[test]
    fn script_created_crew_definition_only_joins_through_set_crew_status() {
        let driver_script = r#"#strict 2
func Probe() {
    var recruit = CreateObject(CREW, 0, 0, 0);
    var before = GetCrewCount(0);
    var added = recruit->SetCrewStatus(0, true);
    return [recruit, before, added, GetCrewCount(0),
            recruit->GetOwner(), recruit->GetController(),
            recruit->RecruitmentCount()];
}
"#;
        let crew_script = r#"#strict 2
local recruitments;
func Recruitment(int player) { recruitments = recruitments + 1; return 1; }
func RecruitmentCount() { return recruitments; }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Script-created crew"))
            .expect("player registers");
        engine
            .register_script_definition("DRVR", "Driver", driver_script)
            .expect("driver registers");
        let mut crew =
            Definition::from_script("CREW", "Crew", crew_script).expect("crew compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");

        let driver = engine
            .spawn_object(SpawnConfig::new("DRVR").with_owner(OWNER_NONE))
            .expect("driver spawns");
        let driver_index = engine.find_object_index(driver).expect("driver index");
        let result = engine
            .call_object_function(driver_index, "Probe", Vec::new())
            .expect("creation and explicit crew admission complete");
        let Value::Array(values) = result else {
            panic!("Probe should return an array");
        };
        let child = match values.first() {
            Some(Value::Object(id)) => ObjectId::new(*id),
            other => panic!("Probe should return the created object, got {other:?}"),
        };
        assert_eq!(
            values,
            vec![
                Value::Object(child.as_u64()),
                Value::Int(0),
                Value::Int(1),
                Value::Int(1),
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
            ],
            "CreateObject only creates a crew-capable object; SetCrewStatus performs the one admission and callback"
        );
        assert_eq!(engine.player(0).expect("player").crew(), &[child]);

        let persisted = engine.capture_state();
        let child_snapshot = persisted
            .objects
            .iter()
            .find(|object| object.snapshot.id == child)
            .expect("created crew persists");
        assert_eq!(child_snapshot.snapshot.owner, 0);
        assert_eq!(child_snapshot.snapshot.controller, 0);
        assert_eq!(
            child_snapshot.snapshot.plr_view_range, 500,
            "MakeCrewMember(false) installs the C++ default crew view range"
        );
    }

    #[test]
    fn newest_equal_experience_crew_info_stays_first_across_callbacks_and_restore() {
        let script = r#"#strict 2
func Join() { return MakeCrewMember(this(), 0); }
func Retire() { return SetCrewStatus(0, false); }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Info order"))
            .expect("player registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("info-order probe compiles");
        crew.set_crew_member(true);
        engine
            .register_definition(crew.clone())
            .expect("crew registers");

        let objects: Vec<_> = (0..4)
            .map(|_| {
                engine
                    .spawn_object(
                        SpawnConfig::new("CREW")
                            .with_owner(0)
                            .with_crew_member(false),
                    )
                    .expect("crew-capable object spawns outside the roster")
            })
            .collect();
        for object in &objects[..2] {
            let index = engine.find_object_index(*object).expect("crew index");
            assert_eq!(
                engine
                    .call_object_function(index, "Join", Vec::new())
                    .expect("fresh info recruits in its own callback"),
                Value::Bool(true)
            );
        }
        let older_name = engine
            .crew_object_info(objects[0])
            .expect("older info exists")
            .name
            .clone();
        let newer_name = engine
            .crew_object_info(objects[1])
            .expect("newer info exists")
            .name
            .clone();
        assert_ne!(older_name, newer_name, "the exact infos are distinguishable");
        for object in &objects[..2] {
            let index = engine.find_object_index(*object).expect("crew index");
            assert_eq!(
                engine
                    .call_object_function(index, "Retire", Vec::new())
                    .expect("info retires in its own callback"),
                Value::Int(1)
            );
        }

        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("info order serializes");
        let state = EngineState::from_json_str(&encoded).expect("info order deserializes");
        let mut restored = Engine::with_seed(99);
        restored
            .register_definition(crew)
            .expect("crew registers for restore");
        restored.restore_state(&state).expect("crew state restores");

        for object in &objects[2..] {
            let index = restored.find_object_index(*object).expect("restored crew index");
            assert_eq!(
                restored
                    .call_object_function(index, "Join", Vec::new())
                    .expect("idle info recruits in its own callback"),
                Value::Bool(true)
            );
        }
        assert_eq!(
            restored
                .crew_object_info(objects[2])
                .expect("first replacement gets an info")
                .name,
            newer_name,
            "C4ObjectInfoList::New inserts at the head, so the newest equal-experience idle info wins"
        );
        assert_eq!(
            restored
                .crew_object_info(objects[3])
                .expect("second replacement gets an info")
                .name,
            older_name,
            "the older equal-experience info remains next in persistent list order"
        );
    }

    #[test]
    fn grab_object_info_clear_pointers_observes_forward_and_backward_callback_writes() {
        let script = r#"#strict 2
local write_player;
local write_target;
local writes;

func Join(int player) { return MakeCrewMember(this(), player); }
func Share(int player) { return SetCrewStatus(player, true); }
func Arm(int player, object target) {
    write_player = player;
    write_target = target;
    return 1;
}
func CrewSelection(bool unselect, bool cursor) {
    if (!unselect && !cursor && write_target) {
        writes = writes + 1;
        SetCursor(write_player, write_target, true, true, true);
    }
    return 1;
}
func Writes() { return writes; }
func Take(object donor) { return GrabObjectInfo(donor); }
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Earlier player"))
            .expect("player zero registers");
        engine
            .register_player(PlayerConfig::new(1, "Later player"))
            .expect("player one registers");
        let mut crew = Definition::from_script("CREW", "Crew", script)
            .expect("pointer-order probe compiles");
        crew.set_crew_member(true);
        engine.register_definition(crew).expect("crew registers");

        let donor = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("donor spawns");
        let receiver = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(OWNER_NONE)
                    .with_crew_member(false),
            )
            .expect("ownerless receiver spawns");
        let earlier_replacement = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(0)
                    .with_crew_member(false),
            )
            .expect("earlier replacement spawns");
        let later_replacement = engine
            .spawn_object(
                SpawnConfig::new("CREW")
                    .with_owner(1)
                    .with_crew_member(false),
            )
            .expect("later replacement spawns");

        let donor_index = engine.find_object_index(donor).expect("donor index");
        assert_eq!(
            engine
                .call_object_function(donor_index, "Join", vec![Value::Int(0)])
                .expect("donor joins player zero"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(donor_index, "Share", vec![Value::Int(1)])
                .expect("donor joins player one too"),
            Value::Int(1)
        );
        for (object, player) in [(earlier_replacement, 0), (later_replacement, 1)] {
            let index = engine.find_object_index(object).expect("replacement index");
            assert_eq!(
                engine
                    .call_object_function(index, "Join", vec![Value::Int(player)])
                    .expect("replacement joins its player"),
                Value::Bool(true)
            );
        }

        engine
            .set_crew_cursor(0, Some(donor))
            .expect("earlier player's donor cursor installs");
        engine
            .set_crew_cursor(1, Some(later_replacement))
            .expect("later player's replacement cursor installs");
        for (object, write_player) in [(earlier_replacement, 1), (later_replacement, 0)] {
            let index = engine.find_object_index(object).expect("replacement index");
            engine
                .call_object_function(
                    index,
                    "Arm",
                    vec![Value::Int(write_player), Value::Object(donor.as_u64())],
                )
                .expect("replacement callback arms");
        }

        let receiver_index = engine.find_object_index(receiver).expect("receiver index");
        assert_eq!(
            engine
                .call_object_function(
                    receiver_index,
                    "Take",
                    vec![Value::Object(donor.as_u64())],
                )
                .expect("receiver grabs donor info"),
            Value::Bool(true)
        );

        for object in [earlier_replacement, later_replacement] {
            let index = engine.find_object_index(object).expect("replacement index");
            assert_eq!(
                engine
                    .call_object_function(index, "Writes", Vec::new())
                    .expect("callback count reads"),
                Value::Int(1)
            );
        }
        assert_eq!(
            engine.crew_cursor(1),
            Some(later_replacement),
            "player zero's forward write to the later player is cleared when that player is visited"
        );
        assert_eq!(
            engine.crew_cursor(0),
            Some(donor),
            "the later player's backward write survives because player zero was already visited"
        );
        assert!(!engine.player(0).expect("player zero").crew().contains(&donor));
        assert!(!engine.player(1).expect("player one").crew().contains(&donor));
    }

    #[test]
    fn get_portrait_distinguishes_permanent_and_requires_info() {
        let script = r#"#strict
public func AttachInfo() { return MakeCrewMember(this(), 0); }
public func SetPermanent() { return SetPortrait("Permanent", this(), GetID(), true, false); }
public func SetTemporary() { return SetPortrait("Temporary", this(), GetID(), false, false); }
public func SetCopied() { return SetPortrait("Permanent", this(), GetID(), true, true); }
public func SetInvalid() { return SetPortrait("Missing", this(), GetID(), true, false); }
public func ClearPermanent() { return SetPortrait("none", this(), GetID(), true, false); }
public func TakeInfo(object donor) { return GrabObjectInfo(donor); }
public func ReadPortraits() {
    return [
        GetPortrait(this(), false, false),
        GetPortrait(this(), false, true),
        GetPortrait(this(), true, false),
        GetPortrait(this(), true, true)
    ];
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Portrait owner"))
            .expect("portrait owner registers");
        let mut crew = Definition::from_script("PORC", "Portrait crew", script)
            .expect("portrait crew compiles");
        crew.set_crew_member(true);
        attach_one_pixel_portrait(&mut engine, &mut crew, "Permanent");
        let image = crew
            .portrait_graphics("Permanent")
            .expect("portrait fixture exists")
            .clone();
        crew.set_portrait_graphics(vec![
            ("Permanent".to_string(), image.clone()),
            ("Temporary".to_string(), image),
        ]);
        engine
            .register_definition(crew)
            .expect("portrait crew registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("PORC")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(0),
            )
            .expect("portrait crew spawns");
        let index = engine.find_object_index(id).expect("portrait crew exists");
        assert_eq!(
            engine
                .call_object_function(index, "AttachInfo", Vec::new())
                .expect("crew info attaches"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(index, "SetPermanent", Vec::new())
                .expect("permanent portrait sets"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(index, "SetTemporary", Vec::new())
                .expect("temporary portrait sets"),
            Value::Bool(true)
        );
        let expected = Value::Array(vec![
            Value::String("Temporary".into()),
            Value::String("Permanent".into()),
            Value::C4Id("PORC".into()),
            Value::C4Id("PORC".into()),
        ]);
        assert_eq!(
            engine
                .call_object_function(index, "ReadPortraits", Vec::new())
                .expect("portraits read in a later callback"),
            expected.clone()
        );

        let saved = engine.capture_state();
        engine
            .restore_state(&saved)
            .expect("portrait state restores");
        let index = engine.find_object_index(id).expect("portrait crew restores");
        assert_eq!(
            engine
                .call_object_function(index, "ReadPortraits", Vec::new())
                .expect("restored portraits read"),
            expected.clone()
        );
        assert_eq!(
            engine
                .call_object_function(index, "SetInvalid", Vec::new())
                .expect("invalid portrait is rejected"),
            Value::Bool(false)
        );
        assert_eq!(
            engine
                .call_object_function(index, "ReadPortraits", Vec::new())
                .expect("rejected set preserves both portraits"),
            expected
        );
        assert_eq!(
            engine
                .call_object_function(index, "SetCopied", Vec::new())
                .expect("copied portrait sets"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(index, "ReadPortraits", Vec::new())
                .expect("owned portrait has no source definition"),
            Value::Array(vec![
                Value::String("custom".into()),
                Value::String("custom".into()),
                Value::Nil,
                Value::Nil,
            ])
        );

        // A temporary-only assignment does not manufacture a permanent
        // portrait; pNewPortrait remains absent and the empty fallback wins.
        let temporary_only = engine
            .spawn_object(
                SpawnConfig::new("PORC")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(0),
            )
            .expect("temporary-only crew spawns");
        let temporary_index = engine
            .find_object_index(temporary_only)
            .expect("temporary-only crew exists");
        for function in ["AttachInfo", "SetTemporary"] {
            assert_eq!(
                engine
                    .call_object_function(temporary_index, function, Vec::new())
                    .expect("temporary-only setup succeeds"),
                Value::Bool(true)
            );
        }
        assert_eq!(
            engine
                .call_object_function(temporary_index, "ReadPortraits", Vec::new())
                .expect("temporary-only portraits read"),
            Value::Array(vec![
                Value::String("Temporary".into()),
                Value::Nil,
                Value::C4Id("PORC".into()),
                Value::Nil,
            ])
        );
        assert_eq!(
            engine
                .call_object_function(
                    temporary_index,
                    "TakeInfo",
                    vec![Value::Object(id.as_u64())],
                )
                .expect("portrait info transfers"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(temporary_index, "ReadPortraits", Vec::new())
                .expect("receiver reads donor portrait state"),
            Value::Array(vec![
                Value::String("custom".into()),
                Value::String("custom".into()),
                Value::Nil,
                Value::Nil,
            ]),
            "the donor Info replaces, rather than merges with, receiver state"
        );
        let donor_index = engine.find_object_index(id).expect("donor object remains");
        assert_eq!(
            engine
                .call_object_function(donor_index, "ReadPortraits", Vec::new())
                .expect("donor without Info reads nil"),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil, Value::Nil])
        );

        // A loaded custom portrait is the permanent fallback. Permanent
        // clear allocates an empty pNewPortrait and must suppress it.
        let custom = engine
            .spawn_object(
                SpawnConfig::new("PORC")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(0),
            )
            .expect("custom portrait crew spawns");
        let custom_index = engine
            .find_object_index(custom)
            .expect("custom portrait crew exists");
        assert_eq!(
            engine
                .call_object_function(custom_index, "AttachInfo", Vec::new())
                .expect("custom portrait info attaches"),
            Value::Bool(true)
        );
        let custom_portrait = CrewPortrait {
            source: None,
            name: "custom".to_string(),
        };
        let mut custom_state = engine.capture_state();
        custom_state
            .crew_object_infos
            .get_mut(&custom)
            .expect("custom live info exists")
            .portraits = CrewPortraitState {
            current: Some(custom_portrait.clone()),
            fallback: Some(custom_portrait),
            permanent: CrewPermanentPortrait::Absent,
        };
        let link = custom_state.crew_info_links[&custom];
        let saved_custom_portraits = custom_state.crew_object_infos[&custom].portraits.clone();
        custom_state
            .crew_info_rosters
            .get_mut(&link.player_id)
            .and_then(|roster| roster.get_mut(link.roster_index))
            .expect("custom roster info exists")
            .portraits = saved_custom_portraits;
        engine
            .restore_state(&custom_state)
            .expect("custom portrait state installs");
        let custom_index = engine.find_object_index(custom).expect("custom crew restores");
        assert_eq!(
            engine
                .call_object_function(custom_index, "ReadPortraits", Vec::new())
                .expect("custom fallback reads"),
            Value::Array(vec![
                Value::String("custom".into()),
                Value::String("custom".into()),
                Value::Nil,
                Value::Nil,
            ])
        );
        assert_eq!(
            engine
                .call_object_function(custom_index, "ClearPermanent", Vec::new())
                .expect("permanent portrait clears"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(custom_index, "ReadPortraits", Vec::new())
                .expect("explicit clear suppresses custom fallback"),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil, Value::Nil])
        );
        let cleared_state = engine.capture_state();
        engine
            .restore_state(&cleared_state)
            .expect("explicit portrait clear restores");
        let custom_index = engine.find_object_index(custom).expect("cleared crew restores");
        assert_eq!(
            engine
                .call_object_function(custom_index, "ReadPortraits", Vec::new())
                .expect("restored clear still suppresses fallback"),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil, Value::Nil])
        );

        // FnGetPortrait requires pObj->Info (C4Script.cpp:5353-5357).
        let noncrew = engine
            .spawn_object(SpawnConfig::new("PORC").with_category(CATEGORY_OBJECT))
            .expect("info-less object spawns");
        let noncrew_index = engine
            .find_object_index(noncrew)
            .expect("info-less object exists");
        assert_eq!(
            engine
                .call_object_function(noncrew_index, "ReadPortraits", Vec::new())
                .expect("info-less portrait query executes"),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil, Value::Nil])
        );
    }

    // FnGrabObjectInfo (C4Script.cpp:2170-2176) -> C4Object::GrabInfo
    // (C4Object.cpp:5696-5726): `this` takes pFrom's info section; the
    // GoldRush TRPR Recruitment creates a temp COWB, grabs its info and
    // removes it (Trapper.c4d/Script.c:19-25) — the temp consumes an
    // object number (C++ hole at 1426). An unknown host fn aborted the
    // whole Recruitment, so the temp never existed in Rust.
    #[test]
    fn grab_object_info_supports_the_trapper_recruitment_hack() {
        let script = r#"#strict
local iGrabbed;
local iDrew;
local portrait_name;
func Recruit() {
    var cb = CreateObject(HAND, 0, 10, GetOwner());
    MakeCrewMember(cb, GetOwner());
    iGrabbed = GrabObjectInfo(cb);
    RemoveObject(cb);
    // AdjustPortrait's gate + synced draw (Cowboy.c4d/Script.c:552-564):
    // the grabbed info carries the donor's portrait source.
    if (GetPortrait(this(), true) != GetID()) iDrew = Random(3) + 1;
    SetPortrait(Format("%d", iDrew), this(), GetID());
    portrait_name = GetPortrait(this(), false);
    return(1);
}
"#;

        let mut engine = Engine::with_seed(0);
        engine
            .register_player(PlayerConfig::new(0, "Trapper owner"))
            .expect("trapper owner registers");
        let mut trapper =
            Definition::from_script("TRAP", "Trapper", script).expect("script compiles");
        trapper.set_crew_member(true);
        attach_one_pixel_portrait(&mut engine, &mut trapper, "1");
        let image = trapper
            .portrait_graphics("1")
            .expect("trapper portrait fixture exists")
            .clone();
        trapper.set_portrait_graphics(vec![
            ("1".to_string(), image.clone()),
            ("2".to_string(), image.clone()),
            ("3".to_string(), image),
        ]);
        engine
            .register_definition(trapper)
            .expect("trapper registers");
        let mut hand = simple_definition("HAND");
        hand.set_crew_member(true);
        engine.register_definition(hand).expect("hand registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("TRAP")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(0)
                    .with_crew_member(true),
            )
            .expect("trapper spawns");
        let before_next = engine.next_object_id;
        let idx = engine.find_object_index(id).expect("trapper exists");
        engine
            .call_object_function(idx, "Recruit", Vec::new())
            .expect("recruitment runs");

        let idx = engine.find_object_index(id).expect("trapper still exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iGrabbed"),
            Some(&Value::Bool(true)),
            "GrabObjectInfo succeeds for a crew donor (C4Object.cpp:5703)"
        );
        assert!(
            engine.objects[idx].state.crew_member,
            "the grabber stays crew (MakeCrewMember, C4Object.cpp:5722)"
        );
        assert!(
            engine
                .objects
                .iter()
                .all(|object| object.definition_id != "HAND"),
            "the temp cowboy is removed"
        );
        assert_eq!(
            engine.next_object_id,
            before_next + 1,
            "the temp consumed exactly one object number (the C++ 1426 hole)"
        );
        assert!(
            matches!(
                engine.objects[idx].state.local_vars.get("iDrew"),
                Some(&Value::Int(n)) if n >= 1
            ),
            "the donor's portrait source (HAND != TRAP) gated the synced \
             Random draw like C++ AdjustPortrait"
        );
        assert_eq!(
            engine
                .crew_object_info(id)
                .and_then(|info| info.portraits.current.as_ref())
                .and_then(|portrait| portrait.source.as_ref())
                .map(DefinitionId::as_str),
            Some("TRAP"),
            "SetPortrait(..., GetID()) re-sources the portrait to the own def"
        );
        assert!(
            matches!(
                engine.objects[idx].state.local_vars.get("portrait_name"),
                Some(Value::String(name)) if matches!(name.as_str(), "1" | "2" | "3")
            ),
            "GetPortrait(..., false) returns the selected filename suffix"
        );
    }

    // C4Object::GrabInfo moves the WHOLE info section (C4Object.cpp:5715:
    // `Info = pFrom->Info; pFrom->ClearInfo(pFrom->Info);`) — the
    // C4ObjectInfo carries the crew's permanent physicals
    // (C4ObjectInfoCore Physical, read by GetPhysical's info fallback,
    // C4Object.cpp:2118-2134), so the grabber takes the donor's trained
    // physicals and the donor falls back to its definition's.
    #[test]
    fn grab_object_info_transfers_the_info_physicals_like_cpp() {
        let script = r#"#strict
local iGrabbed;
func Grab() { iGrabbed = GrabObjectInfo(FindObject(HAND)); return 1; }
"#;
        let mut engine = Engine::with_seed(0);
        let mut trapper =
            Definition::from_script("TRAP", "Trapper", script).expect("script compiles");
        trapper.set_crew_member(true);
        engine
            .register_definition(trapper)
            .expect("trapper registers");
        let mut hand = simple_definition("HAND");
        hand.set_crew_member(true);
        engine.register_definition(hand).expect("hand registers");

        let grabber = engine
            .spawn_object(
                SpawnConfig::new("TRAP")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(0)
                    .with_crew_member(true),
            )
            .expect("trapper spawns");
        let donor = engine
            .spawn_object(
                SpawnConfig::new("HAND")
                    .with_category(CATEGORY_OBJECT)
                    .with_owner(0)
                    .with_crew_member(true),
            )
            .expect("hand spawns");

        let trained = PhysicalInfo {
            energy: 77_000,
            fight: 12_000,
            ..PhysicalInfo::default()
        };
        let donor_idx = engine.find_object_index(donor).expect("donor exists");
        engine.objects[donor_idx].state.info_physical = Some(trained);
        let grabber_idx = engine.find_object_index(grabber).expect("grabber exists");
        engine.objects[grabber_idx].state.info_physical = Some(PhysicalInfo {
            energy: 11_000,
            ..PhysicalInfo::default()
        });

        engine
            .call_object_function(grabber_idx, "Grab", Vec::new())
            .expect("grab runs");

        let grabber_idx = engine.find_object_index(grabber).expect("grabber exists");
        assert_eq!(
            engine.objects[grabber_idx].state.local_vars.get("iGrabbed"),
            Some(&Value::Bool(true)),
            "GrabObjectInfo succeeds for a crew donor (C4Object.cpp:5703)"
        );
        assert_eq!(
            engine.objects[grabber_idx].state.info_physical,
            Some(trained),
            "the grabber takes the donor's info physicals (C4Object.cpp:5715)"
        );
        let donor_idx = engine.find_object_index(donor).expect("donor exists");
        assert_eq!(
            engine.objects[donor_idx].state.info_physical, None,
            "ClearInfo leaves the donor without info physicals (C4Object.cpp:5715)"
        );
        assert!(
            !engine.objects[donor_idx].state.crew_member,
            "the donor loses its crew slot (Game.Players.ClearPointers, C4Object.cpp:5711-5713)"
        );
    }

    // C4Effect's constructor runs Fx*Start SYNCHRONOUSLY inside
    // FnAddEffect (C4Effect.cpp:96-152: insert, check chain, then
    // pFnStart->Exec before the ctor returns) — so objects the Start
    // callback creates get their numbers AT THE AddEffect INSTANT.
    // GoldRush oracle: each bandit's SetAI equip (2xAMBO+WINC from
    // FxAIBanditNoMoveStart) interleaves between the CreateObject(BNDT)
    // calls in DoInitialize; deferring the callback shifted 12 ids.
    #[test]
    fn add_effect_runs_start_callback_at_call_position_like_cpp() {
        let script = r#"#strict
func Trigger() {
    var first = CreateObject(MARK, 0, 0, -1);
    AddEffect("Equip", this(), 1, 0, this());
    var last = CreateObject(MARK, 0, 0, -1);
    return(1);
}

func FxEquipStart(pTarget, iNumber, iTemp) {
    CreateObject(MARK, 10, 0, -1);
    return(1);
}
"#;

        let mut engine = Engine::with_seed(0);
        let actor = Definition::from_script("Actor", "Actor", script).expect("script compiles");
        engine.register_definition(actor).expect("actor registers");
        engine
            .register_definition(simple_definition("MARK"))
            .expect("marker registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let idx = engine.find_object_index(id).expect("actor exists");
        engine
            .call_object_function(idx, "Trigger", Vec::new())
            .expect("trigger runs");

        let mut marks: Vec<(u64, i32)> = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "MARK")
            .map(|object| (object.id.as_u64(), object.state.position.x))
            .collect();
        marks.sort();
        assert_eq!(marks.len(), 3, "three markers created");
        assert_eq!(
            marks[1].1, 10,
            "the Start-callback marker (x=10) allocates BETWEEN the two \
             direct CreateObject calls (C4Effect.cpp:131-135)"
        );
    }

    #[test]
    fn queued_commands_apply_effect_changes() {
        let mut engine = Engine::with_seed(1);
        let definition = Definition::from_script(
            "Dummy",
            "Dummy",
            "global func Step(state, frame, random) { return 0; }",
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Dummy"))
            .expect("spawn succeeds");

        let command = QueuedCommand::immediate(ObjectUpdate::default())
            .with_delay(1)
            .with_effects(vec![EffectCommand::add(EffectState::new("Queued"))]);
        engine
            .queue_object_command(id, command)
            .expect("queue succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert!(object.effects.is_empty());

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.effects.len(), 1);
        assert_eq!(object.effects[0].name, "Queued");
    }

    #[test]
    fn effect_callbacks_fire_across_lifecycle_events() {
        let script = r#"#strict 3
        global func Initialize(state, random) {
            return { effects = [ { op = "add", name = "Pulse", interval = 2 } ] };
        }

        global func FxPulseStart(state, effect) {
            return nil;
        }

        global func FxPulseTimer(state, effect, timer) {
            return nil;
        }

        global func FxPulseStop(state, effect, reason) {
            return nil;
        }

        global func Step(state, frame, random) {
            if (frame == 3) {
                return { effects = [ { op = "remove", name = "Pulse" } ] };
            }
            return nil;
        }
        "#;

        let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let call_log = Arc::clone(&call_log);
            hooks.set_on_call(move |name, _args| {
                call_log.lock().unwrap().push(name.to_string());
            });
        }

        let mut definition =
            Definition::from_script("Actor", "Actor", script).expect("script compiles");
        definition.set_debugger_hooks(hooks);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Actor"))
            .expect("spawn succeeds");

        let first = engine.tick().expect("first tick succeeds");
        let object = first.object(id).expect("object present");
        assert!(object.effects.iter().any(|effect| effect.name == "Pulse"));

        let second = engine.tick().expect("second tick succeeds");
        let object = second.object(id).expect("object present");
        assert!(object.effects.iter().any(|effect| effect.name == "Pulse"));

        let third = engine.tick().expect("third tick succeeds");
        let object = third.object(id).expect("object present");
        assert!(object
            .effects
            .iter()
            .any(|effect| effect.name == "Pulse" && effect.priority == 0));

        let fourth = engine.tick().expect("fourth tick succeeds");
        let object = fourth.object(id).expect("object present");
        assert!(object.effects.is_empty());

        let calls = call_log.lock().unwrap().clone();
        let start_calls = calls.iter().filter(|name| *name == "FxPulseStart").count();
        let timer_calls = calls.iter().filter(|name| *name == "FxPulseTimer").count();
        let stop_calls = calls.iter().filter(|name| *name == "FxPulseStop").count();

        assert_eq!(start_calls, 1);
        assert!(timer_calls >= 1);
        assert_eq!(stop_calls, 1);
    }

    // C4Object::Enter adds to the container's contents with
    // C4ObjectList::Add stContents (C4Object.cpp:1587): a sorted insert
    // (C4ObjectList.cpp:110-176) — before the forward-first live link
    // with the same sorted category AND the same def (the same-id
    // cluster, :150-162), else before the forward-first live link whose
    // (Category & C4D_SortLimit) <= the entering object's (:164-173).
    // Equal-category items therefore enter at the FRONT: Contents(0) is
    // the newest item. The GoldRush bandits arm via CreateContents(AMBO)
    // x2 + CreateContents(WINC) (Goldrush.c4s/Locals.c4d/AI.c4d/Script.c:
    // 103-105) and FireRifle checks `Contents()->~IsRifle()`
    // (Cowboy.c4d/Script.c:439) — the rifle must be first.
    #[test]
    fn runtime_contents_enter_inserts_before_equal_category_like_cpp() {
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(simple_definition("Ches"))
            .expect("chest registers");
        engine
            .register_definition(simple_definition("AMBO"))
            .expect("ammo registers");
        engine
            .register_definition(simple_definition("WINC"))
            .expect("rifle registers");

        let chest = engine
            .spawn_object(SpawnConfig::new("Ches").with_category(CATEGORY_OBJECT))
            .expect("chest spawns");
        let ammo_a = engine
            .spawn_object(
                SpawnConfig::new("AMBO")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("ammo a spawns");
        let ammo_b = engine
            .spawn_object(
                SpawnConfig::new("AMBO")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("ammo b spawns");
        let rifle = engine
            .spawn_object(
                SpawnConfig::new("WINC")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("rifle spawns");

        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_idx].state.contents,
            vec![rifle, ammo_b, ammo_a],
            "each Enter inserts at the front of its category bracket, \
             same-id entries cluster (C4ObjectList.cpp:150-173)"
        );
    }

    // FnScrollContents (C4Script.cpp:1793-1805) removes the raw first
    // contents link, appends it with stNone, and returns the new front.
    // Same-definition items deliberately prove this is a one-link rotation,
    // not ShiftContents' search for the next different picture stack.
    #[test]
    fn scroll_contents_moves_first_to_back_and_returns_new_front_like_cpp() {
        let script = r#"#strict 3
        global func Cycle() { return ScrollContents(); }
        global func Invalid() { return ScrollContents(false); }
        "#;
        let mut engine = Engine::with_seed(3);
        engine.register_script_definition("CHES", "Chest", script).expect("chest registers");
        engine
            .register_definition(simple_definition("ITEM"))
            .expect("item registers");

        let filled = engine
            .spawn_object(SpawnConfig::new("CHES").with_category(CATEGORY_OBJECT))
            .expect("filled chest spawns");
        let empty = engine
            .spawn_object(SpawnConfig::new("CHES").with_category(CATEGORY_OBJECT))
            .expect("empty chest spawns");
        let c = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(filled),
            )
            .expect("C spawns");
        let b = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(filled),
            )
            .expect("B spawns");
        let a = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(filled),
            )
            .expect("A spawns");

        let filled_idx = engine.find_object_index(filled).expect("filled chest exists");
        assert_eq!(engine.objects[filled_idx].state.contents, vec![a, b, c]);
        let result = engine
            .call_object_function(filled_idx, "Cycle", Vec::new())
            .expect("ScrollContents runs");
        assert_eq!(result, Value::Object(b.as_u64()));
        let filled_idx = engine.find_object_index(filled).expect("filled chest exists");
        assert_eq!(
            engine.objects[filled_idx].state.contents,
            vec![b, c, a],
            "ScrollContents rotates exactly one raw contents link"
        );
        engine
            .call_object_function(filled_idx, "Invalid", Vec::new())
            .expect_err("strict typed false must fail object conversion");
        let filled_idx = engine.find_object_index(filled).expect("filled chest exists");
        assert_eq!(engine.objects[filled_idx].state.contents, vec![b, c, a]);

        let empty_idx = engine.find_object_index(empty).expect("empty chest exists");
        let result = engine
            .call_object_function(empty_idx, "Cycle", Vec::new())
            .expect("empty ScrollContents runs");
        assert_eq!(result, Value::Nil);
        let empty_idx = engine.find_object_index(empty).expect("empty chest exists");
        assert!(engine.objects[empty_idx].state.contents.is_empty());
    }

    // FnScrollContents' optional pObj only defaults nil to cthr->Obj. An
    // explicit foreign container is mutated while the caller stays intact.
    #[test]
    fn scroll_contents_rotates_an_explicit_foreign_container_like_cpp() {
        let script = r#"
        global func Poke(target) { return ScrollContents(target); }
        "#;
        let mut engine = Engine::with_seed(5);
        engine.register_script_definition("ACTR", "Actor", script).expect("actor registers");
        engine
            .register_definition(simple_definition("CHES"))
            .expect("chest registers");
        engine
            .register_definition(simple_definition("ITEM"))
            .expect("item registers");

        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let chest = engine
            .spawn_object(SpawnConfig::new("CHES").with_category(CATEGORY_OBJECT))
            .expect("chest spawns");
        let b = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("B spawns");
        let a = engine
            .spawn_object(
                SpawnConfig::new("ITEM")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("A spawns");

        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        let result = engine
            .call_object_function(actor_idx, "Poke", vec![Value::Object(chest.as_u64())])
            .expect("foreign ScrollContents runs");
        assert_eq!(result, Value::Object(b.as_u64()));
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(engine.objects[chest_idx].state.contents, vec![b, a]);
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        assert!(engine.objects[actor_idx].state.contents.is_empty());
    }

    // FnShiftContents (C4Script.cpp:1784-1797): the regular shift rotates
    // the contents CYCLICALLY to the next different item
    // (C4Object::ShiftContents C4Object.cpp:5728-5752,
    // C4ObjectList::ShiftContents C4ObjectList.cpp:815-833 — relative
    // order preserved); the idTarget form brings the first matching
    // content to the front (DirectComContents :5754-5775); a uniform
    // stack has nothing different to shift to and reports false.
    #[test]
    fn shift_contents_rotates_to_next_different_item_like_cpp() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func Cycle() { return ShiftContents(); }
        global func Pick() { return ShiftContents(0, 0, REVR); }
        "#;
        let mut engine = Engine::with_seed(3);
        engine.register_script_definition("Ches", "Ches", script).expect("chest registers");
        engine
            .register_definition(simple_definition("SWRD"))
            .expect("sword registers");
        engine
            .register_definition(simple_definition("REVR"))
            .expect("revolver registers");

        let chest = engine
            .spawn_object(SpawnConfig::new("Ches").with_category(CATEGORY_OBJECT))
            .expect("chest spawns");
        let sword = engine
            .spawn_object(
                SpawnConfig::new("SWRD")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("sword spawns");
        let revolver_a = engine
            .spawn_object(
                SpawnConfig::new("REVR")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("revolver a spawns");
        let revolver_b = engine
            .spawn_object(
                SpawnConfig::new("REVR")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("revolver b spawns");

        // Each Enter front-inserts within its category bracket / id
        // cluster (C4ObjectList::Add stContents, C4ObjectList.cpp:150-173).
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_idx].state.contents,
            vec![revolver_b, revolver_a, sword]
        );

        // Regular shift: the next DIFFERENT item after revolver_b is the
        // sword (revolver_a picture-concats); the rotation keeps relative
        // order.
        let result = engine
            .call_object_function(chest_idx, "Cycle", Vec::new())
            .expect("cycle runs");
        assert_eq!(result, Value::Bool(true));
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_idx].state.contents,
            vec![sword, revolver_b, revolver_a],
            "cyclic rotation to the next different item (C4ObjectList.cpp:815-833)"
        );

        // idTarget form: bring the forward-first REVR back to the front
        // (Contents.Find, C4Script.cpp:1791).
        let result = engine
            .call_object_function(chest_idx, "Pick", Vec::new())
            .expect("pick runs");
        assert_eq!(result, Value::Bool(true));
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_idx].state.contents,
            vec![revolver_b, revolver_a, sword],
            "DirectComContents rotates the target to the front (C4Object.cpp:5765)"
        );
    }

    #[test]
    fn shift_contents_skips_only_cpp_picture_concat_equivalents() {
        // C4Object::ShiftContents compares every candidate with the original
        // front through CanConcatPictureWith. Same-ID objects are skipped only
        // when their live pictures concatenate (C4Object.cpp:5751-5773,
        // 6173-6213), in both traversal directions.
        let chest_script = r#"#strict 3
        local controlled;
        func MutateAndCycle(back, target) {
            SetPicture(0, 76, 64, 64, target);
            return ShiftContents(nil, back, nil, true);
        }
        func Cycle(back) { return ShiftContents(nil, back); }
        func ControlContents(selected_id) { controlled = selected_id; return false; }
        "#;
        let item_script = r#"#strict 3
        local selected;
        func Selection(container) { selected = container; return true; }
        "#;
        let mut engine = Engine::with_seed(17);
        engine.register_script_definition("CHES", "Chest", chest_script).expect("chest registers");
        engine
            .register_script_definition("ITEM", "Item", item_script)
            .expect("ordinary item registers");
        let mut color_stack = simple_definition("PASS");
        color_stack.set_allow_picture_stack(APS_COLOR);
        engine
            .register_definition(color_stack)
            .expect("color-stack item registers");

        let chest = engine
            .spawn_object(SpawnConfig::new("CHES"))
            .expect("chest spawns");
        let visually_distinct = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(chest))
            .expect("picture target spawns");
        let equivalent = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(chest))
            .expect("equivalent item spawns");
        let front = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(chest))
            .expect("front item spawns");
        let chest_index = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_index].state.contents,
            vec![front, equivalent, visually_distinct]
        );

        for (shift_back, order, expected) in [
            (
                false,
                vec![front, equivalent, visually_distinct],
                vec![visually_distinct, front, equivalent],
            ),
            (
                true,
                vec![front, visually_distinct, equivalent],
                vec![visually_distinct, equivalent, front],
            ),
        ] {
            let target_index = engine
                .find_object_index(visually_distinct)
                .expect("picture target exists");
            engine.objects[target_index].state.picture_rect = DefinitionRect::default();
            engine.objects[target_index]
                .state
                .local_vars
                .insert("selected".into(), Value::Nil);
            let chest_index = engine.find_object_index(chest).expect("chest exists");
            engine.objects[chest_index].state.contents = order;
            engine.objects[chest_index]
                .state
                .local_vars
                .insert("controlled".into(), Value::Nil);
            engine.pending_audio.clear();
            assert_eq!(
                engine
                    .call_object_function(
                        chest_index,
                        "MutateAndCycle",
                        vec![
                            Value::Bool(shift_back),
                            Value::Object(visually_distinct.as_u64()),
                        ],
                    )
                    .expect("same-call picture-aware shift runs"),
                Value::Bool(true)
            );
            let chest_index = engine.find_object_index(chest).expect("chest exists");
            assert_eq!(
                engine.objects[chest_index].state.contents,
                expected,
                "direction {shift_back}: exact duplicate is skipped, but the same-ID picture changed earlier in this call is selected"
            );
            assert_eq!(
                engine.objects[chest_index].state.local_vars.get("controlled"),
                Some(&Value::C4Id("ITEM".into())),
                "ControlContents sees the selected same-definition target"
            );
            let target_index = engine
                .find_object_index(visually_distinct)
                .expect("picture target exists");
            assert_eq!(
                engine.objects[target_index].state.local_vars.get("selected"),
                Some(&Value::Object(chest.as_u64())),
                "the selected picture target receives Selection(container)"
            );
            assert!(
                !engine.pending_audio.iter().any(|command| matches!(
                    command,
                    AudioCommand::PlaySound { name, .. } if name == "Grab"
                )),
                "truthy Selection suppresses the Grab sound"
            );
        }

        // APS_Color makes tint differences concat-compatible, so the next
        // picture-rect difference is the first selectable picture stack.
        let color_chest = engine
            .spawn_object(SpawnConfig::new("CHES"))
            .expect("color-stack chest spawns");
        let picture_distinct = engine
            .spawn_object(
                SpawnConfig::new("PASS")
                    .with_picture_rect(DefinitionRect::new(0, 76, 64, 64))
                    .with_container(color_chest),
            )
            .expect("picture-distinct item spawns");
        let tint_ignored = engine
            .spawn_object(
                SpawnConfig::new("PASS")
                    .with_color_modulation(0x0040_4040)
                    .with_container(color_chest),
            )
            .expect("ignored-tint item spawns");
        let color_front = engine
            .spawn_object(SpawnConfig::new("PASS").with_container(color_chest))
            .expect("color-stack front spawns");
        let color_chest_index = engine
            .find_object_index(color_chest)
            .expect("color-stack chest exists");
        assert_eq!(
            engine.objects[color_chest_index].state.contents,
            vec![color_front, tint_ignored, picture_distinct]
        );
        assert_eq!(
            engine
                .call_object_function(
                    color_chest_index,
                    "Cycle",
                    vec![Value::Bool(false)],
                )
                .expect("APS-aware shift runs"),
            Value::Bool(true)
        );
        let color_chest_index = engine
            .find_object_index(color_chest)
            .expect("color-stack chest exists");
        assert_eq!(
            engine.objects[color_chest_index].state.contents,
            vec![picture_distinct, color_front, tint_ignored],
            "APS_Color skips the tint while the picture rectangle still splits the stack"
        );
    }

    // A container whose contents all picture-concat (same definition here)
    // has nothing different to shift to (C4Object.cpp:5741-5745).
    #[test]
    fn shift_contents_uniform_stack_reports_false_like_cpp() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func Cycle() { return ShiftContents(); }
        "#;
        let mut engine = Engine::with_seed(5);
        engine.register_script_definition("Ches", "Ches", script).expect("chest registers");
        engine
            .register_definition(simple_definition("REVR"))
            .expect("revolver registers");

        let chest = engine
            .spawn_object(SpawnConfig::new("Ches").with_category(CATEGORY_OBJECT))
            .expect("chest spawns");
        let revolver_a = engine
            .spawn_object(
                SpawnConfig::new("REVR")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("revolver a spawns");
        let revolver_b = engine
            .spawn_object(
                SpawnConfig::new("REVR")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("revolver b spawns");

        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        let result = engine
            .call_object_function(chest_idx, "Cycle", Vec::new())
            .expect("cycle runs");
        assert_eq!(result, Value::Bool(false));
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_idx].state.contents,
            vec![revolver_b, revolver_a],
            "a uniform stack keeps its (Enter-sorted, newest-first) order"
        );
    }

    // FnShiftContents' pObj parameter (C4Script.cpp:1786) targets ANOTHER
    // object: `if (!pObj) pObj = cthr->Obj` is only the local-call default
    // — a foreign container's contents rotate just the same.
    #[test]
    fn shift_contents_operates_on_a_foreign_container_like_cpp() {
        let script = r#"
        global func Initialize(state, random) { return 0; }
        global func Step(state, frame, random) { return 0; }
        global func Poke() { return ShiftContents(FindObject(CHES)); }
        "#;
        let mut engine = Engine::with_seed(7);
        engine.register_script_definition("Actr", "Actor", script).expect("actor registers");
        engine
            .register_definition(simple_definition("CHES"))
            .expect("chest registers");
        engine
            .register_definition(simple_definition("SWRD"))
            .expect("sword registers");
        engine
            .register_definition(simple_definition("REVR"))
            .expect("revolver registers");

        let actor = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let chest = engine
            .spawn_object(SpawnConfig::new("CHES").with_category(CATEGORY_OBJECT))
            .expect("chest spawns");
        let sword = engine
            .spawn_object(
                SpawnConfig::new("SWRD")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("sword spawns");
        let revolver = engine
            .spawn_object(
                SpawnConfig::new("REVR")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("revolver spawns");

        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        let result = engine
            .call_object_function(actor_idx, "Poke", Vec::new())
            .expect("poke runs");
        assert_eq!(result, Value::Bool(true));
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_idx].state.contents,
            vec![sword, revolver],
            "the foreign chest rotated the sword to the front \
             (Enter-sorted [revolver, sword], C4Object.cpp:5730-5752)"
        );
    }

    // DirectComContents with fDoCalls (C4Object.cpp:5760-5763): the
    // container's ~ControlContents(idNewFront) runs FIRST and a truthy
    // return takes over — the default rotation is skipped, yet
    // C4Object::ShiftContents still reports true.
    #[test]
    fn shift_contents_do_calls_control_contents_veto_like_cpp() {
        let script = r#"#strict
local iSeen;
func Cycle() { return ShiftContents(0, 0, 0, 1); }
func ControlContents(selected_id) { iSeen = selected_id; return 1; }
"#;
        let mut engine = Engine::with_seed(11);
        engine.register_script_definition("CHES", "Chest", script).expect("chest registers");
        engine
            .register_definition(simple_definition("SWRD"))
            .expect("sword registers");
        engine
            .register_definition(simple_definition("REVR"))
            .expect("revolver registers");

        let chest = engine
            .spawn_object(SpawnConfig::new("CHES").with_category(CATEGORY_OBJECT))
            .expect("chest spawns");
        let sword = engine
            .spawn_object(
                SpawnConfig::new("SWRD")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("sword spawns");
        let revolver = engine
            .spawn_object(
                SpawnConfig::new("REVR")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("revolver spawns");

        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        let result = engine
            .call_object_function(chest_idx, "Cycle", Vec::new())
            .expect("cycle runs");
        assert_eq!(
            result,
            Value::Bool(true),
            "the veto path still reports true (C4Object.cpp:5745-5746)"
        );
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_idx].state.contents,
            vec![revolver, sword],
            "a truthy ~ControlContents vetoes the rotation (C4Object.cpp:5762); \
             Enter sorted the later revolver to the front"
        );
        assert_eq!(
            engine.objects[chest_idx].state.local_vars.get("iSeen"),
            Some(&Value::C4Id("SWRD".into())),
            "~ControlContents receives the new front's id (C4VID(pTarget->id))"
        );
    }

    // DirectComContents' selection tail (C4Object.cpp:5765-5767): after the
    // relink the NEW front gets ~Selection(container); only a falsy return
    // plays the Grab sound on the container.
    #[test]
    fn shift_contents_do_calls_selection_and_grab_sound_like_cpp() {
        let chest_script = r#"#strict
func Cycle() { return ShiftContents(0, 0, 0, 1); }
"#;
        let sword_script = r#"#strict
local iSel;
func Selection(pFrom) { iSel = 1; return 1; }
"#;
        let mut engine = Engine::with_seed(13);
        engine.register_script_definition("CHES", "Chest", chest_script).expect("chest registers");
        engine.register_script_definition("SWRD", "Sword", sword_script).expect("sword registers");
        engine
            .register_definition(simple_definition("REVR"))
            .expect("revolver registers");

        // Spawn the sword first: the later revolver Enter-sorts to the
        // front (C4ObjectList.cpp:164-173), giving contents
        // [revolver, sword].
        let chest = engine
            .spawn_object(SpawnConfig::new("CHES").with_category(CATEGORY_OBJECT))
            .expect("chest spawns");
        let sword = engine
            .spawn_object(
                SpawnConfig::new("SWRD")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("sword spawns");
        let revolver = engine
            .spawn_object(
                SpawnConfig::new("REVR")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(chest),
            )
            .expect("revolver spawns");

        // First shift: SWRD comes to the front, its Selection returns 1 —
        // no Grab sound (C4Object.cpp:5767).
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        let result = engine
            .call_object_function(chest_idx, "Cycle", Vec::new())
            .expect("cycle runs");
        assert_eq!(result, Value::Bool(true));
        let sword_idx = engine.find_object_index(sword).expect("sword exists");
        assert_eq!(
            engine.objects[sword_idx].state.local_vars.get("iSel"),
            Some(&Value::Int(1)),
            "the new front got ~Selection (C4Object.cpp:5767)"
        );
        assert!(
            !engine
                .pending_audio
                .iter()
                .any(|command| matches!(command, AudioCommand::PlaySound { name, .. } if name == "Grab")),
            "a truthy Selection suppresses the Grab sound"
        );

        // Second shift: REVR (no Selection handler -> falsy) comes to the
        // front — the Grab sound plays on the container.
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        let result = engine
            .call_object_function(chest_idx, "Cycle", Vec::new())
            .expect("cycle runs");
        assert_eq!(result, Value::Bool(true));
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert_eq!(
            engine.objects[chest_idx].state.contents,
            vec![revolver, sword],
            "the second shift rotated back to the revolver"
        );
        assert!(
            engine.pending_audio.iter().any(|command| matches!(
                command,
                AudioCommand::PlaySound { name, target, looped: false, .. }
                    if name == "Grab" && *target == Some(chest)
            )),
            "a falsy Selection plays Grab on the container (StartSoundEffect, C4Object.cpp:5767)"
        );
    }

    // FnGetDamage (C4Script.cpp:1366-1370): `pObj->Damage` — the optional
    // object parameter reads a FOREIGN object's damage; the GoldRush
    // telegraph's Damage callback self-checks `GetDamage()>100`
    // (Telegraph.c4d/Script.c:9) once bandit fire lands.
    #[test]
    fn get_damage_reads_own_and_foreign_damage_like_cpp() {
        let script = r#"#strict
local iOwn, iOther;
public func Probe(pOther) {
  DoDamage(7);
  DoDamage(9, pOther);
  iOwn = GetDamage();
  iOther = GetDamage(pOther);
  return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.register_script_definition("Actr", "Actor", script).expect("actor registers");
        engine
            .register_definition(simple_definition("Targ"))
            .expect("target registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("Targ").with_category(CATEGORY_OBJECT))
            .expect("target spawns");

        let idx = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(idx, "Probe", vec![Value::Object(target.as_u64())])
            .expect("probe runs");

        let idx = engine.find_object_index(actor).expect("actor exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iOwn"),
            Some(&Value::Int(7)),
            "GetDamage() reads the caller's damage"
        );
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iOther"),
            Some(&Value::Int(9)),
            "GetDamage(pObj) reads the foreign object's damage"
        );
    }

    // C4Aul object calls mutate the ONE live C4Object (C4AulExec object
    // locals are by-reference): a nested call back onto an object whose
    // OUTER call is in flight reads the outer call's mid-call local writes
    // and its own writes surface in the outer VM when it resumes — for
    // EVERY outer-call kind, not just effect callbacks (the host-initiated
    // call_object_function / PSF-callback path here).
    #[test]
    fn nested_call_onto_outer_object_shares_live_locals_like_cpp() {
        let opener_script = r#"#strict
local state;
public func Open(pOther) {
  state = 7;
  var seen = pOther->Query(this());
  return(seen * 100 + state);
}
public func GetState() { return(state); }
public func SetState(v) { state = v; }
"#;
        let prober_script = r#"#strict
public func Query(pA) {
  var got = pA->GetState();
  pA->SetState(42);
  return(got);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine
            .register_script_definition("Opnr", "Opener", opener_script)
            .expect("opener registers");
        engine
            .register_script_definition("Prob", "Prober", prober_script)
            .expect("prober registers");
        let opener = engine
            .spawn_object(SpawnConfig::new("Opnr").with_category(CATEGORY_OBJECT))
            .expect("opener spawns");
        let prober = engine
            .spawn_object(SpawnConfig::new("Prob").with_category(CATEGORY_OBJECT))
            .expect("prober spawns");

        let idx = engine.find_object_index(opener).expect("opener exists");
        let result = engine
            .call_object_function(idx, "Open", vec![Value::Object(prober.as_u64())])
            .expect("open runs");
        // The nested GetState sees the outer `state = 7` (not the pre-call
        // snapshot) and the nested SetState(42) is live when the outer VM
        // resumes: 7 * 100 + 42.
        assert_eq!(
            result,
            Value::Int(742),
            "nested calls onto the outer object must share its live locals \
             (C++ mutates the one live C4Object)"
        );
        let idx = engine.find_object_index(opener).expect("opener exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("state"),
            Some(&Value::Int(42)),
            "the deepest write is the one that persists"
        );
    }

    // C4AulExec errors unwind the call but leave every mutation made
    // BEFORE the error in place — C++ mutates live state chronologically
    // (C4AulExec.cpp:1318-1342 logs and continues; nothing is rolled
    // back). An OUTER call that errors must still fold its partial
    // outcome: local writes, foreign-object mutations and the RNG draws
    // it made.
    #[test]
    fn outer_call_error_keeps_pre_error_mutations_like_cpp() {
        let script = r#"#strict
local state;
public func Break(pOther) {
  state = 5;
  DoDamage(3, pOther);
  var pNone;
  pNone->Boom();
  state = 9;
  return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.register_script_definition("Actr", "Actor", script).expect("actor registers");
        engine
            .register_definition(simple_definition("Targ"))
            .expect("target registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let target = engine
            .spawn_object(SpawnConfig::new("Targ").with_category(CATEGORY_OBJECT))
            .expect("target spawns");

        let idx = engine.find_object_index(actor).expect("actor exists");
        let result = engine.call_object_function(idx, "Break", vec![Value::Object(target.as_u64())]);
        assert!(
            matches!(result, Err(EngineError::Script { .. })),
            "the nil deref aborts the call with a script error"
        );

        let idx = engine.find_object_index(actor).expect("actor exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("state"),
            Some(&Value::Int(5)),
            "the pre-error local write persists; the post-error one never ran"
        );
        let target_idx = engine.find_object_index(target).expect("target exists");
        assert_eq!(
            engine.objects[target_idx].state.damage, 3,
            "the pre-error foreign DoDamage persists (C++ mutated the live object)"
        );
    }

    // FnEval (C4Script.cpp:4507-4520) -> C4AulScript::DirectExec
    // (C4AulExec.cpp:1658-1707): the string parses as ONE expression
    // (ParseFn fExprOnly, C4AulParse.cpp:1417-1424 — trailing text like a
    // stray ';' is ignored) and runs in the calling object's context.
    // The planet Schedule() helper drives GoldRush's intro-movie end
    // through it: FxIntScheduleTimer does eval(EffectVar(0, ...))
    // (planet/System.c4g/Helpers.c:125-132).
    #[test]
    fn eval_direct_execs_an_expression_in_the_object_context_like_cpp() {
        let script = r#"#strict
local iGot, iSum;
public func Poke() { iGot = 7; return(iGot); }
public func Boot() {
  iSum = eval("1+2");
  eval("Poke();");
  return(1);
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.register_script_definition("Actr", "Actor", script).expect("actor registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Actr").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let idx = engine.find_object_index(id).expect("actor exists");
        engine
            .call_object_function(idx, "Boot", Vec::new())
            .expect("boot runs");

        let idx = engine.find_object_index(id).expect("actor exists");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iSum"),
            Some(&Value::Int(3)),
            "eval returns the expression value"
        );
        assert_eq!(
            engine.objects[idx].state.local_vars.get("iGot"),
            Some(&Value::Int(7)),
            "the eval'd call runs in the object's own context \
             (cthr->Obj->Def->Script.DirectExec, C4Script.cpp:4514) — \
             a trailing ';' is tolerated like ParseFn fExprOnly"
        );
    }

    // FnDoEnergy's pObj (C4Script.cpp:492-499): `if (!pObj) pObj = cthr->Obj`
    // is only the local-call default — a named FOREIGN target takes the
    // change (C4Object::DoEnergy percent scale, C4Object.cpp:1345-1365).
    #[test]
    fn do_energy_reaches_a_foreign_target_like_cpp() {
        let script = r#"#strict
func Zap() { return DoEnergy(-10, FindObject(VCTM)); }
"#;
        let mut engine = Engine::with_seed(17);
        engine.register_script_definition("ACTR", "Actor", script).expect("actor registers");
        let mut victim_definition = simple_definition("VCTM");
        victim_definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(victim_definition)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(SpawnConfig::new("VCTM").with_category(CATEGORY_OBJECT))
            .expect("victim spawns");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[victim_idx].state.energy = 50_000;
        engine.objects[victim_idx].state.alive = true;

        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        let result = engine
            .call_object_function(actor_idx, "Zap", Vec::new())
            .expect("zap runs");
        assert_eq!(result, Value::Bool(true));
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.energy,
            40_000,
            "-10% of C4MaxPhysical lands on the foreign target (C4Object.cpp:1347,1361)"
        );
    }

    // C4Object::DoEnergy kills when a nonzero energy reaches zero
    // (C4Object.cpp:1363) — on FOREIGN targets too: the nested-outcome
    // fold must fire AssignDeath like the local fold does.
    #[test]
    fn do_energy_kills_a_foreign_target_at_zero_like_cpp() {
        let script = r#"#strict
func Zap() { return DoEnergy(-10, FindObject(VCTM)); }
"#;
        let mut engine = Engine::with_seed(23);
        engine.register_script_definition("ACTR", "Actor", script).expect("actor registers");
        let mut victim_definition = simple_definition("VCTM");
        victim_definition.set_physical(PhysicalInfo {
            energy: 10_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(victim_definition)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(
                SpawnConfig::new("VCTM")
                    .with_category(CATEGORY_OBJECT)
                    .with_alive(true),
            )
            .expect("victim spawns");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[victim_idx].state.energy = 10_000;

        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_idx, "Zap", Vec::new())
            .expect("zap runs");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(engine.objects[victim_idx].state.energy, 0);
        assert!(
            !engine.objects[victim_idx].state.alive,
            "a nonzero energy reaching 0 assigns death (C4Object.cpp:1363)"
        );
    }

    // FnExplode is an engine global even without System.c4g. It defaults a
    // nil object to cthr->Obj, snapshots cause/container before removal, but
    // reads x/y after Destruction. A nonempty known particle overrides both
    // the default Blast particle and the otherwise-unused effect id.
    #[test]
    fn native_explode_is_registered_and_matches_object_default_effect_arguments() {
        let script = r#"#strict 3
protected func Destruction() { SetPosition(31, 47); return true; }
public func Detonate() {
    var no_object;
    return Explode(14, no_object, FXID, "Custom");
}
"#;
        let mut engine = Engine::with_seed(23);
        for name in ["Blast", "Custom"] {
            engine
                .register_particle_definition(
                    particles::ParticleDefCore {
                        name: name.into(),
                        init_fn: "StdInit".into(),
                        exec_fn: "StdExec".into(),
                        draw_fn: "Std".into(),
                        delay: 1,
                        repeats: 1000,
                        ..Default::default()
                    },
                    4,
                    1.0,
                )
                .expect("explosion particle registers");
        }
        engine
            .register_definition(simple_definition("FXID"))
            .expect("effect definition registers");
        engine.register_script_definition("BOOM", "Bomb", script).expect("bomb registers");
        let bomb = engine
            .spawn_object(
                SpawnConfig::new("BOOM")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 20))
                    .with_controller(7),
            )
            .expect("bomb spawns");
        let bomb_index = engine.find_object_index(bomb).expect("bomb exists");

        assert_eq!(
            engine
                .call_object_function(bomb_index, "Detonate", Vec::new())
                .expect("native Explode executes"),
            Value::Bool(true)
        );
        assert!(engine.objects[bomb_index].destroyed, "Explode removes its target");
        let particles = engine.particle_system().particles();
        assert_eq!(particles.len(), 1, "the custom particle replaces Blast");
        assert_eq!(particles[0].def_name, "Custom");
        assert_eq!(particles[0].x.to_bits(), 31.0f32.to_bits());
        assert_eq!(particles[0].y.to_bits(), 47.0f32.to_bits());
        assert_eq!(particles[0].a.to_bits(), 14.0f32.to_bits());
        assert!(engine.pending_audio.iter().any(|command| matches!(
            command,
            AudioCommand::PlaySoundAt { name, position }
                if name == "Blast49" && *position == Vector2::new(31, 47)
        )));
        assert!(
            engine.objects.iter().all(|object| object.definition_id != "FXID"),
            "a resolved particle wins over idEffect"
        );
    }

    #[test]
    fn native_explode_builds_the_effect_id_when_no_particle_is_loaded() {
        let caller_script = r#"#strict 3
public func Detonate(object target) { return Explode(10, target, FXID); }
"#;
        let target_script = r#"#strict 3
protected func Destruction() { SetController(8); return true; }
"#;
        let effect_script = r#"#strict 3
local activated;
protected func Activate() { activated = 1; return true; }
"#;
        let mut engine = Engine::with_seed(23);
        engine
            .register_player(PlayerConfig::new(7, "Original"))
            .expect("original controller registers");
        engine
            .register_player(PlayerConfig::new(8, "Replacement"))
            .expect("replacement controller registers");
        for (id, name, script) in [
            ("CALL", "Caller", caller_script),
            ("TARG", "Target", target_script),
            ("FXID", "Explosion effect", effect_script),
            ("LAYR", "Layer", "#strict 3\n"),
        ] {
            engine.register_script_definition(id, name, script).expect("definition registers");
        }
        let layer = engine
            .spawn_object(SpawnConfig::new("LAYR").with_position(Vector2::new(500, 500)))
            .expect("layer spawns");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(300, 300)))
            .expect("caller spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("TARG")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 50))
                    .with_controller(7)
                    .with_layer(layer),
            )
            .expect("target spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Detonate",
                    vec![object_reference_value(target)],
                )
                .expect("foreign-target Explode executes"),
            Value::Bool(true)
        );
        assert!(!engine.objects[caller_index].destroyed, "explicit target wins");
        let effect = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "FXID")
            .expect("id effect is constructed when Blast is unavailable");
        assert_eq!(effect.state.owner, 7, "owner is the pre-removal cause");
        assert_eq!(
            effect.state.controller, 8,
            "creator controller is read after Destruction"
        );
        assert_eq!(effect.state.layer, Some(layer), "creator layer is retained");
        assert_eq!(effect.state.construction, FULL_CON / 2);
        assert_eq!(
            effect.state.local_vars.get("activated"),
            Some(&Value::Int(1)),
            "Activate follows successful construction"
        );
    }

    #[test]
    fn native_explode_initial_partial_effect_idles_after_full_construction() {
        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(
                Definition::from_script(
                    "CALL",
                    "Caller",
                    "#strict 3\npublic func Detonate(object target) { return Explode(-10, target, FXID); }\n",
                )
                .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(simple_definition("TARG"))
            .expect("target registers");
        let mut effect = Definition::from_script(
            "FXID",
            "Explosion effect",
            r#"#strict 3
protected func Construction() {
    DoCon(100);
    SetAction("Active");
    return true;
}
"#,
        )
        .expect("effect compiles");
        effect.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Active".to_string(), ActionSpec::default()),
            ]),
        );
        engine
            .register_definition(effect)
            .expect("effect registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(300, 300)))
            .expect("caller spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("TARG")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 50)),
            )
            .expect("target spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Detonate",
                    vec![object_reference_value(target)],
                )
                .expect("negative-level native Explode executes"),
            Value::Bool(true)
        );
        let effect = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "FXID" && !object.destroyed)
            .expect("partially constructed effect survives");
        assert_eq!(effect.state.construction, FULL_CON / 2);
        assert_eq!(
            effect.state.action.name, "Idle",
            "initial DoCon idles a Construction callback's full object when it decays"
        );
    }

    #[test]
    fn native_explode_script_overload_shadows_and_inherited_reaches_the_host() {
        let mut engine = Engine::with_seed(23);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/Explode.c".to_string(),
                "#strict 3\n\
                 global func Explode(int level, object target, id effect_id, string effect_name) {\n\
                   if (level == 1) return 4242;\n\
                   return inherited(level, target, effect_id, effect_name);\n\
                 }\n"
                    .to_string(),
            )]),
            1
        );
        engine
            .register_definition(
                Definition::from_script(
                    "BOOM",
                    "Bomb",
                    "#strict 3\npublic func Shadow() { return Explode(1); }\n\
                     public func Chain() { return Explode(10); }\n",
                )
                .expect("overload probe compiles"),
            )
            .expect("bomb registers");
        let bomb = engine
            .spawn_object(SpawnConfig::new("BOOM").with_category(CATEGORY_OBJECT))
            .expect("bomb spawns");
        let bomb_index = engine.find_object_index(bomb).expect("bomb exists");

        assert_eq!(
            engine
                .call_object_function(bomb_index, "Shadow", Vec::new())
                .expect("script overload executes"),
            Value::Int(4242)
        );
        assert!(!engine.objects[bomb_index].destroyed, "shadow did not call host");
        assert_eq!(
            engine
                .call_object_function(bomb_index, "Chain", Vec::new())
                .expect("inherited host fallback executes"),
            Value::Bool(true)
        );
        assert!(engine.objects[bomb_index].destroyed);
    }

    #[test]
    fn native_explode_contain_blast_suppresses_visuals_and_blasts_contents() {
        let caller_script =
            "#strict 3\npublic func Detonate(object target) { return Explode(12, target); }\n";
        let mut engine = Engine::with_seed(23);
        engine
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Blast".into(),
                    init_fn: "StdInit".into(),
                    exec_fn: "StdExec".into(),
                    draw_fn: "Std".into(),
                    delay: 1,
                    repeats: 1000,
                    ..Default::default()
                },
                4,
                1.0,
            )
            .expect("Blast particle registers");
        engine
            .register_script_definition("CALL", "Caller", caller_script)
            .expect("caller registers");
        let mut shield_definition = simple_definition("SHLD");
        shield_definition.set_contain_blast(1);
        engine
            .register_definition(shield_definition)
            .expect("shield registers");
        engine
            .register_definition(simple_definition("TARG"))
            .expect("target registers");
        engine
            .register_definition(simple_definition("SIBL"))
            .expect("sibling registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(300, 300)))
            .expect("caller spawns");
        let shield = engine
            .spawn_object(
                SpawnConfig::new("SHLD")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 50)),
            )
            .expect("shield spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("TARG")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(shield),
            )
            .expect("target spawns in shield");
        let sibling = engine
            .spawn_object(
                SpawnConfig::new("SIBL")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(shield),
            )
            .expect("sibling spawns in shield");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Detonate",
                    vec![object_reference_value(target)],
                )
                .expect("contained Explode executes"),
            Value::Bool(true)
        );
        let shield_index = engine.find_object_index(shield).expect("shield remains");
        let sibling_index = engine.find_object_index(sibling).expect("sibling remains");
        assert_eq!(engine.objects[shield_index].state.damage, 12);
        assert_eq!(engine.objects[sibling_index].state.damage, 12);
        assert!(
            engine.particle_system().particles().is_empty(),
            "ContainBlast suppresses uncontained visuals"
        );
    }

    #[test]
    fn native_explode_blasts_a_captured_container_after_an_effect_removes_it() {
        let caller_script = r#"#strict 3
local hits;
public func Mark() { hits++; return true; }
public func Detonate(object target) { return Explode(10, target, FXID); }
"#;
        let shield_script = r#"#strict 3
protected func Damage() { FindObject(CALL)->Mark(); return true; }
"#;
        let effect_script = r#"#strict 3
protected func Activate() { RemoveObject(FindObject(SHLD)); return true; }
"#;
        let mut engine = Engine::with_seed(23);
        for (id, name, script) in [
            ("CALL", "Caller", caller_script),
            ("SHLD", "Shield", shield_script),
            ("FXID", "Explosion effect", effect_script),
            ("TARG", "Target", "#strict 3\n"),
        ] {
            engine.register_script_definition(id, name, script).expect("definition registers");
        }
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(300, 300)))
            .expect("caller spawns");
        let shield = engine
            .spawn_object(
                SpawnConfig::new("SHLD")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 50)),
            )
            .expect("shield spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("TARG")
                    .with_category(CATEGORY_OBJECT)
                    .with_container(shield),
            )
            .expect("target spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let shield_index = engine.find_object_index(shield).expect("shield exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Detonate",
                    vec![object_reference_value(target)],
                )
                .expect("native Explode executes"),
            Value::Bool(true)
        );
        assert!(
            engine.objects[shield_index].destroyed,
            "effect callback removes the captured container"
        );
        assert_eq!(
            engine.objects[shield_index].state.damage, 10,
            "the raw captured pointer still receives native Blast damage"
        );
        assert_eq!(
            engine.objects[caller_index].state.local_vars.get("hits"),
            Some(&Value::Nil),
            "C4Object::Call suppresses the Damage callback after Status=0"
        );
    }

    #[test]
    fn native_explode_zero_level_runs_effect_destruction_without_activate() {
        let caller_script = r#"#strict 3
local events;
public func Mark(int value) { events += value; return true; }
public func Detonate(object target) { events = 0; return Explode(0, target, FXID); }
"#;
        let effect_script = r#"#strict 3
protected func Construction() { CreateContents(CHLD); return true; }
protected func Destruction() { FindObject(CALL)->Mark(1); return true; }
protected func Activate() { FindObject(CALL)->Mark(10); return true; }
"#;
        let mut engine = Engine::with_seed(23);
        for (id, name, script) in [
            ("CALL", "Caller", caller_script),
            ("CHLD", "Child", "#strict 3\n"),
            ("FXID", "Explosion effect", effect_script),
            ("TARG", "Target", "#strict 3\n"),
        ] {
            engine.register_script_definition(id, name, script).expect("definition registers");
        }
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(300, 300)))
            .expect("caller spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("TARG")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 50)),
            )
            .expect("target spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Detonate",
                    vec![object_reference_value(target)],
                )
                .expect("zero-level native Explode executes"),
            Value::Bool(true)
        );
        assert_eq!(
            engine.objects[caller_index].state.local_vars.get("events"),
            Some(&Value::Int(1)),
            "DoCon(0) assigns removal and skips Activate after creation fails"
        );
        assert!(
            engine.objects.iter().all(|object| object.definition_id != "FXID"),
            "the zero-construction effect never materializes"
        );
        let child = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "CHLD" && !object.destroyed)
            .expect("initial DoCon ejects Construction-created contents");
        assert_eq!(child.state.container, None);
    }

    #[test]
    fn native_explode_retries_incineration_after_flam_removes_itself() {
        let caller_script = r#"#strict 3
local attempts;
public func Mark() { attempts++; return true; }
public func Detonate(object target) { return Explode(10, target); }
"#;
        let flam_script = r#"#strict 3
protected func Construction() {
    FindObject(CALL)->Mark();
    RemoveObject();
    return true;
}
"#;
        let library = MaterialLibrary::parse(
            r#"
            [Material Oil]
            Name=Oil
            Density=100
            Friction=25
            Inflammable=1
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let oil = materials.id_of("Oil").expect("oil exists");
        let mut engine = Engine::with_seed(23);
        engine.set_materials(materials);
        let mut landscape = Landscape::flat_with_material(17, 40, Some(oil));
        landscape.set_world_height(80);
        engine.set_landscape(landscape);
        for (id, name, script) in [
            ("CALL", "Caller", caller_script),
            ("FLAM", "Fire", flam_script),
            ("TARG", "Target", "#strict 3\n"),
        ] {
            engine.register_script_definition(id, name, script).expect("definition registers");
        }
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(300, 300)))
            .expect("caller spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("TARG")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(8, 45)),
            )
            .expect("target spawns");
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Detonate",
                    vec![object_reference_value(target)],
                )
                .expect("native Explode executes"),
            Value::Bool(true)
        );
        assert_eq!(
            engine.objects[caller_index].state.local_vars.get("attempts"),
            Some(&Value::Int(3)),
            "failed FLAM creation probes the two other inflammable points"
        );
        assert!(
            engine
                .objects
                .iter()
                .all(|object| object.definition_id != "FLAM" || object.destroyed),
            "self-removing FLAMs do not survive"
        );
    }

    // FnBlastObjects is an engine global even without System.c4g's
    // same-named Explode.c helper (C4Script.cpp:2269-2275,6875). The native
    // C4Game walk first applies a direct C4Object::Blast, then its living
    // shockwave damage and mass-scaled fling (C4Game.cpp:1243-1296). An
    // encoded caused-by zero inherits the calling object's Controller.
    #[test]
    fn blast_objects_bare_host_damages_flings_and_inherits_caller_controller() {
        let caller_script = r#"#strict
func Detonate() { var no_container; return BlastObjects(50, 50, 20, no_container, 0); }
"#;
        let mut engine = Engine::with_seed(23);
        engine
            .register_player(PlayerConfig::new(7, "Blaster"))
            .expect("player registers");
        engine
            .register_script_definition("BLST", "Blaster", caller_script)
            .expect("caller registers");

        let mut victim_definition = simple_definition("VCTM");
        victim_definition.set_category(CATEGORY_LIVING | CATEGORY_OBJECT);
        victim_definition.set_mass(80);
        victim_definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        victim_definition.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(victim_definition)
            .expect("victim registers");

        // BlastObjects coordinates are global (unlike BlastFree): keep the
        // caller far away and place only the victim in the blast square.
        let caller = engine
            .spawn_object(
                SpawnConfig::new("BLST")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 100))
                    .with_controller(7),
            )
            .expect("caller spawns");
        let victim = engine
            .spawn_object(
                SpawnConfig::new("VCTM")
                    .with_category(CATEGORY_LIVING | CATEGORY_OBJECT)
                    // Fresh creation treats the supplied y as the shape
                    // bottom; this -1..+1 shape settles at center y=50.
                    .with_position(Vector2::new(54, 51))
                    .with_alive(true)
                    .with_energy(100_000),
            )
            .expect("victim spawns");

        let caller_idx = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(caller_idx, "Detonate", Vec::new())
                .expect("native BlastObjects executes"),
            Value::Nil,
            "FnBlastObjects is void"
        );

        let victim_idx = engine.find_object_index(victim).expect("victim remains");
        let victim = &engine.objects[victim_idx];
        assert_eq!(
            victim.state.damage, 30,
            "direct Blast(20) plus living shockwave DoDamage(10)"
        );
        assert_eq!(
            victim.state.energy,
            100_000 - (20 / 3 + 20 / 2) * (C4_MAX_PHYSICAL / 100),
            "direct blast and shockwave energy losses both land"
        );
        assert_eq!(
            victim.fixed_velocity,
            FixedVec2::new(itofix(16) / 8, itofix(-20) / 8),
            "level-distance force is divided by bounded mass/10"
        );
        assert_eq!(
            victim.last_energy_loss_cause, 7,
            "caused_by_plus_one=0 inherits the caller's controller"
        );
    }

    #[test]
    fn system_blast_objects_overload_shadows_the_native_host() {
        let mut engine = Engine::with_seed(23);
        assert_eq!(
            engine.install_global_scripts(&[(
                "System.c4g/Explode.c".to_string(),
                "#strict\nglobal func BlastObjects(x, y, level, inobj, caused_by) { return 4242; }\n"
                    .to_string(),
            )]),
            1,
            "System.c4g overload installs"
        );
        engine
            .register_definition(
                Definition::from_script(
                    "BLST",
                    "Blaster",
                    "#strict\nfunc Detonate() { return BlastObjects(50, 50, 20); }\n",
                )
                .expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("BLST").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let caller_idx = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(caller_idx, "Detonate", Vec::new())
                .expect("System BlastObjects executes"),
            Value::Int(4242),
            "the System.c4g global overload wins over the engine host"
        );
    }

    // FnBlastObject (C4Script.cpp:2281-2289) -> C4Object::Blast
    // (C4Object.cpp:1414-1419): Damage rises by the blast level and an
    // alive target additionally loses level/3 energy percent points
    // (DoEnergy fExact=false, C4FxCall_EngBlast).
    #[test]
    fn blast_object_damages_and_drains_an_alive_target_like_cpp() {
        let script = r#"#strict
func Zap() { return BlastObject(12, FindObject(VCTM)); }
"#;
        let mut engine = Engine::with_seed(23);
        engine.register_script_definition("ACTR", "Actor", script).expect("actor registers");
        let mut victim_definition = simple_definition("VCTM");
        victim_definition.set_physical(PhysicalInfo {
            energy: 10_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(victim_definition)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(
                SpawnConfig::new("VCTM")
                    .with_category(CATEGORY_OBJECT)
                    .with_alive(true),
            )
            .expect("victim spawns");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[victim_idx].state.energy = 10_000;

        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        let result = engine
            .call_object_function(actor_idx, "Zap", Vec::new())
            .expect("zap runs");
        assert_eq!(result, Value::Bool(true), "FnBlastObject returns true");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.damage, 12,
            "DoDamage(level) raises Damage (C4Object.cpp:1416)"
        );
        assert_eq!(
            engine.objects[victim_idx].state.energy,
            10_000 - 4 * (C4_MAX_PHYSICAL / 100),
            "alive targets lose level/3 percent points (C4Object.cpp:1418)"
        );
        assert!(engine.objects[victim_idx].state.alive);
    }

    // FnAddEffect refuses a missing/zero priority: `if (!iPrio) return 0`
    // (C4Script.cpp:5457) — an unfilled parameter nil-fills to 0 like
    // C4AulExec, so `AddEffect("Foo", 0)` creates NOTHING and returns 0.
    #[test]
    fn add_effect_without_priority_creates_nothing_like_cpp() {
        let script = r#"#strict
local iResult;
func Trigger() { iResult = AddEffect("Foo"); return(1); }
"#;
        let mut engine = Engine::with_seed(5);
        engine.register_script_definition("ACTR", "Actor", script).expect("actor registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_idx, "Trigger", Vec::new())
            .expect("trigger runs");
        assert_eq!(
            engine.objects[actor_idx].state.local_vars.get("iResult"),
            Some(&Value::Int(0)),
            "missing priority returns 0 (C4Script.cpp:5457)"
        );
        assert!(
            engine.objects[actor_idx].state.effects.is_empty(),
            "no effect is created"
        );
    }

    // C4Object::Blast's incinerate arm (C4Object.cpp:1420-1423): once the
    // ACCUMULATED Damage reaches Def->BlastIncinerate the target ignites
    // like C4Object::Incinerate(iCausedBy, fBlasted=true) — one synced
    // FirePhase = Random(15) draw and the Incineration engine callback
    // (fxFireStart core, C4Effect.cpp:632-638).
    #[test]
    fn blast_object_incinerates_past_the_blast_incinerate_threshold() {
        let victim_script = r#"#strict
local iIncinerated;
func Incineration(int iCausedBy) { iIncinerated = iCausedBy + 100; return(1); }
"#;
        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(
                Definition::from_script(
                    "ACTR",
                    "Actor",
                    "#strict\nfunc Zap(pVictim, iLevel) { return BlastObject(iLevel, pVictim); }\n",
                )
                .expect("actor compiles"),
            )
            .expect("actor registers");
        let mut victim_def =
            Definition::from_script("VCTM", "Victim", victim_script).expect("victim compiles");
        victim_def.set_blast_incinerate(10);
        engine
            .register_definition(victim_def)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(SpawnConfig::new("VCTM").with_category(CATEGORY_OBJECT))
            .expect("victim spawns");
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_idx].state.controller = 3;
        let victim_value = Value::Object(victim.as_u64());

        // Below the threshold: damage accumulates, no fire, no draw.
        let count_before = engine.rng.count;
        engine
            .call_object_function(actor_idx, "Zap", vec![victim_value.clone(), Value::Int(5)])
            .expect("zap runs");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(engine.objects[victim_idx].state.damage, 5);
        assert!(!engine.objects[victim_idx].state.on_fire);
        assert_eq!(engine.rng.count, count_before, "no FirePhase draw yet");

        // Crossing the threshold (5 + 6 = 11 >= 10) ignites.
        let mut expected_rng = engine.rng.clone();
        let expected_phase = expected_rng.random(15);
        let count_before = engine.rng.count;
        engine
            .call_object_function(actor_idx, "Zap", vec![victim_value, Value::Int(6)])
            .expect("zap runs");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(engine.objects[victim_idx].state.damage, 11);
        assert!(
            engine.objects[victim_idx].state.on_fire,
            "Damage >= BlastIncinerate incinerates (C4Object.cpp:1421-1423)"
        );
        assert_eq!(engine.objects[victim_idx].state.fire_caused_by, 3);
        assert_eq!(
            engine.objects[victim_idx].state.fire_phase, expected_phase,
            "FirePhase = Random(15) on the shared ledger"
        );
        assert_eq!(engine.rng.count, count_before + 1, "exactly one draw");
        assert_eq!(
            engine.objects[victim_idx].state.local_vars.get("iIncinerated"),
            Some(&Value::Int(103)),
            "Incineration callback got the causing player (C4Effect.cpp:638)"
        );
        // The host path creates the same Fire C4Effect entry as the
        // engine-side incinerate (C4Object.cpp:1263-1265; vars
        // C4Effect.cpp:628-631 — mode Object=3 for a C4D_Object victim,
        // blasted fire).
        let fire = engine.objects[victim_idx]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Fire")
            .cloned()
            .expect("fire effect entry staged through the host seam");
        assert_eq!(fire.priority, 100);
        assert_eq!(fire.interval, 1);
        assert_eq!(
            fire.vars(),
            &[
                EffectVarValue::Int(3),
                EffectVarValue::Int(3),
                EffectVarValue::Bool(true),
                EffectVarValue::Nil,
            ]
        );
    }

    // The host-path incinerate must match the engine-side
    // incinerate_object semantics (C4Object::Incinerate,
    // C4Object.cpp:1255-1266 + fxFireStart core, C4Effect.cpp:560-641):
    // already-burning and dead-living refusals draw nothing, extinguisher
    // material fires IncinerationEx instead of igniting, BurnTurnTo
    // changedefs, and burning containers eject their contents.
    #[test]
    fn blast_object_incinerate_matches_the_engine_incinerate_semantics(
    ) -> Result<(), EngineError> {
        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
            Friction=0
            Extinguisher=1

            [Material Earth]
            Name=Earth
            Density=100
            Friction=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let water = materials.id_of("Water").expect("water exists");
        let earth = materials.id_of("Earth").expect("earth exists");

        let recorder_script = r#"#strict
local iIncinerated, iIncineratedEx;
func Incineration(int iCausedBy) { iIncinerated = iCausedBy + 100; return(1); }
func IncinerationEx(int iCausedBy) { iIncineratedEx = iCausedBy + 100; return(1); }
"#;
        let mut engine = Engine::with_seed(70);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(40, 30, Some(earth)));
        engine
            .register_definition(
                Definition::from_script(
                    "ACTR",
                    "Actor",
                    "#strict\nfunc Zap(pVictim, iLevel) { return BlastObject(iLevel, pVictim); }\n",
                )
                .expect("actor compiles"),
            )
            .expect("actor registers");
        let mut tree_def =
            Definition::from_script("TREE", "Tree", recorder_script).expect("tree compiles");
        tree_def.set_blast_incinerate(10);
        engine.register_definition(tree_def).expect("tree registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let actor_idx = engine.find_object_index(actor).expect("actor exists");

        // Already burning: no draw, no callback, cause kept (C4Object.cpp:1258).
        let burning = engine
            .spawn_object(SpawnConfig::new("TREE").with_position(Vector2::new(10, 10)))
            .expect("tree spawns");
        let burning_idx = engine.find_object_index(burning).expect("tree exists");
        engine.objects[burning_idx].state.on_fire = true;
        let mirror = engine.rng.clone();
        engine
            .call_object_function(
                actor_idx,
                "Zap",
                vec![Value::Object(burning.as_u64()), Value::Int(12)],
            )
            .expect("zap runs");
        let burning_idx = engine.find_object_index(burning).expect("tree exists");
        assert_eq!(engine.objects[burning_idx].state.damage, 12);
        assert_eq!(engine.rng, mirror, "no FirePhase draw for a burning target");
        assert_eq!(
            engine.objects[burning_idx].state.fire_caused_by, OWNER_NONE,
            "the original fire cause is kept (C4Object.cpp:1258)"
        );
        assert_eq!(
            engine.objects[burning_idx].state.local_vars.get("iIncinerated"),
            None,
            "no Incineration callback for a burning target"
        );

        // Dead living: never ignites (C4Object.cpp:1260).
        let mut corpse_def =
            Definition::from_script("CRPS", "Corpse", recorder_script).expect("corpse compiles");
        corpse_def.set_blast_incinerate(10);
        corpse_def.set_category(CATEGORY_LIVING);
        engine
            .register_definition(corpse_def)
            .expect("corpse registers");
        let corpse = engine
            .spawn_object(
                SpawnConfig::new("CRPS")
                    .with_position(Vector2::new(20, 10))
                    .with_alive(false),
            )
            .expect("corpse spawns");
        engine
            .call_object_function(
                actor_idx,
                "Zap",
                vec![Value::Object(corpse.as_u64()), Value::Int(12)],
            )
            .expect("zap runs");
        let corpse_idx = engine.find_object_index(corpse).expect("corpse exists");
        assert!(!engine.objects[corpse_idx].state.on_fire);

        // Submerged in extinguisher material: IncinerationEx instead of
        // fire, and NO draw (C4Effect.cpp:574-583, 602-607).
        if let Some(landscape) = engine.landscape.as_mut() {
            landscape.set_liquid_column(30, vec![LiquidSegment::with_material(5, 12, Some(water))]);
        }
        let soaked = engine
            .spawn_object(SpawnConfig::new("TREE").with_position(Vector2::new(30, 8)))
            .expect("soaked tree spawns");
        let mirror = engine.rng.clone();
        engine
            .call_object_function(
                actor_idx,
                "Zap",
                vec![Value::Object(soaked.as_u64()), Value::Int(12)],
            )
            .expect("zap runs");
        let soaked_idx = engine.find_object_index(soaked).expect("soaked exists");
        assert!(!engine.objects[soaked_idx].state.on_fire);
        assert_eq!(engine.rng, mirror, "no draw when extinguished at start");
        assert_eq!(
            engine.objects[soaked_idx].state.local_vars.get("iIncineratedEx"),
            Some(&Value::Int(99)),
            "blasted-in-extinguisher fires IncinerationEx (caused_by NO_OWNER + 100)"
        );

        // BurnTurnTo changedef + contents ejection at the burn position
        // (C4Effect.cpp:579-594).
        let mut chest_def =
            Definition::from_script("CHST", "Chest", recorder_script).expect("chest compiles");
        chest_def.set_blast_incinerate(10);
        chest_def.set_burn_turn_to(Some("ASH1".to_string()));
        engine.register_definition(chest_def).expect("chest registers");
        engine
            .register_definition(simple_definition("ASH1"))
            .expect("ash registers");
        engine
            .register_definition(simple_definition("GEMM"))
            .expect("gem registers");
        let chest = engine
            .spawn_object(SpawnConfig::new("CHST").with_position(Vector2::new(12, 12)))
            .expect("chest spawns");
        let gem = engine
            .spawn_object(
                SpawnConfig::new("GEMM")
                    .with_position(Vector2::new(12, 12))
                    .with_container(chest),
            )
            .expect("gem spawns");
        engine
            .call_object_function(
                actor_idx,
                "Zap",
                vec![Value::Object(chest.as_u64()), Value::Int(12)],
            )
            .expect("zap runs");
        let chest_idx = engine.find_object_index(chest).expect("chest exists");
        assert!(engine.objects[chest_idx].state.on_fire);
        assert_eq!(
            engine.objects[chest_idx].definition_id.as_str(),
            "ASH1",
            "BurnTurnTo changedefs even when blasted (C4Effect.cpp:579-585)"
        );
        assert!(engine.objects[chest_idx].state.contents.is_empty());
        let gem_idx = engine.find_object_index(gem).expect("gem exists");
        assert_eq!(engine.objects[gem_idx].state.container, None, "ejected");
        assert_eq!(engine.objects[gem_idx].state.position, Vector2::new(12, 12));
        Ok(())
    }

    // C4Object::DoDamage asks the target's effects FIRST for non-living
    // objects (C4Object.cpp:1282-1286): every live Fx*Damage hook chains
    // the damage value on the effect's command target
    // (C4Effect::DoDamage, C4Effect.cpp:427-437) — and this must hold on
    // the HOST scope path (script DoDamage), not just the engine path.
    #[test]
    fn host_do_damage_asks_the_targets_fx_damage_effects_first_like_cpp() {
        let victim_script = r#"#strict
local iSeen, iCauseSeen, iDamageCalls;
func FxShieldDamage(pTarget, iNumber, iChange, iCause, iCausePlr) {
    iSeen = iChange;
    iCauseSeen = iCause;
    // C4Effect::DoDamage consumes the complete Bool Data.Int payload.
    return CastBool(7);
}
func Damage(int iChange, int iCausedBy) { iDamageCalls = iDamageCalls + 1; return(1); }
"#;
        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(
                Definition::from_script(
                    "ACTR",
                    "Actor",
                    "#strict\nfunc Zap(pVictim) {\n  AddEffect(\"Shield\", pVictim, 1, 0, pVictim);\n  return DoDamage(10, pVictim, 3, 8);\n}\n",
                )
                .expect("actor compiles"),
            )
            .expect("actor registers");
        engine
            .register_script_definition("VCTM", "Victim", victim_script)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(SpawnConfig::new("VCTM").with_category(CATEGORY_OBJECT))
            .expect("victim spawns");
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_idx, "Zap", vec![Value::Object(victim.as_u64())])
            .expect("zap runs");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        let locals = &engine.objects[victim_idx].state.local_vars;
        assert_eq!(
            locals.get("iSeen"),
            Some(&Value::Int(10)),
            "the hook sees the raw change"
        );
        assert_eq!(
            locals.get("iCauseSeen"),
            Some(&Value::Int(3)),
            "the damage type threads through (FnDoDamage iDmgType)"
        );
        assert_eq!(
            engine.objects[victim_idx].state.damage, 7,
            "the chained hook result is written (C4Object.cpp:1288)"
        );
        assert_eq!(
            locals.get("iDamageCalls"),
            Some(&Value::Int(1)),
            "~Damage still fires for a nonzero outcome"
        );
    }

    // C4Object::DoEnergy asks a LIVING target's effects AFTER the percent
    // scale (C4Object.cpp:1347 precedes :1355-1359): the Fx*Damage hook
    // sees the SCALED change with the C4FxCall_EngScript cause, and a
    // zero chain outcome aborts before the energy write — on the HOST
    // scope path too.
    #[test]
    fn host_do_energy_asks_fx_damage_effects_on_the_living_like_cpp() {
        let warded_script = r#"#strict
local iSeen, iCauseSeen;
func FxWardDamage(pTarget, iNumber, iChange, iCause, iCausePlr) {
    iSeen = iChange;
    iCauseSeen = iCause;
    return(iChange / 2);
}
"#;
        let nulled_script = r#"#strict
func FxNullDamage(pTarget, iNumber, iChange, iCause, iCausePlr) { return(0); }
"#;
        let mut engine = Engine::with_seed(12);
        engine
            .register_definition(
                Definition::from_script(
                    "ACTR",
                    "Actor",
                    "#strict\nfunc Zap(pVictim, szEffect) {\n  AddEffect(szEffect, pVictim, 1, 0, pVictim);\n  return DoEnergy(-10, pVictim);\n}\n",
                )
                .expect("actor compiles"),
            )
            .expect("actor registers");
        let mut warded_definition =
            Definition::from_script("WARD", "Warded", warded_script).expect("warded compiles");
        warded_definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(warded_definition)
            .expect("warded registers");
        let mut nulled_definition =
            Definition::from_script("NULD", "Nulled", nulled_script).expect("nulled compiles");
        nulled_definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(nulled_definition)
            .expect("nulled registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let warded = engine
            .spawn_object(
                SpawnConfig::new("WARD")
                    .with_category(CATEGORY_OBJECT)
                    .with_alive(true),
            )
            .expect("warded spawns");
        let nulled = engine
            .spawn_object(
                SpawnConfig::new("NULD")
                    .with_category(CATEGORY_OBJECT)
                    .with_alive(true),
            )
            .expect("nulled spawns");
        for id in [warded, nulled] {
            let idx = engine.find_object_index(id).expect("victim exists");
            engine.objects[idx].state.energy = 50_000;
        }
        let actor_idx = engine.find_object_index(actor).expect("actor exists");

        engine
            .call_object_function(
                actor_idx,
                "Zap",
                vec![
                    Value::Object(warded.as_u64()),
                    Value::String("Ward".into()),
                ],
            )
            .expect("zap runs");
        let warded_idx = engine.find_object_index(warded).expect("warded exists");
        let locals = &engine.objects[warded_idx].state.local_vars;
        assert_eq!(
            locals.get("iSeen"),
            Some(&Value::Int(-10 * (C4_MAX_PHYSICAL / 100))),
            "the hook sees the SCALED change"
        );
        assert_eq!(
            locals.get("iCauseSeen"),
            Some(&Value::Int(C4FX_CALL_ENG_SCRIPT)),
            "script DoEnergy carries C4FxCall_EngScript (C4Script.cpp:495)"
        );
        assert_eq!(
            engine.objects[warded_idx].state.energy,
            50_000 - 5 * (C4_MAX_PHYSICAL / 100),
            "the halved hook result is written"
        );

        engine
            .call_object_function(
                actor_idx,
                "Zap",
                vec![
                    Value::Object(nulled.as_u64()),
                    Value::String("Null".into()),
                ],
            )
            .expect("zap runs");
        let nulled_idx = engine.find_object_index(nulled).expect("nulled exists");
        assert_eq!(
            engine.objects[nulled_idx].state.energy, 50_000,
            "a zero chain outcome aborts before the write (C4Object.cpp:1358)"
        );
    }

    // FnDoEnergy's caused-by (C4Script.cpp:496-497): iCausedByPlusOne - 1,
    // or the CALLER's controller when unset — marked on the target's
    // LastEnergyLossCausePlayer kill trace for negative changes
    // (C4Object.cpp:1351-1353).
    #[test]
    fn do_energy_threads_the_caused_by_player_like_cpp() {
        let script = r#"#strict
func Zap() { return DoEnergy(-10, FindObject(VCTM)); }
func ZapAs() { return DoEnergy(-10, FindObject(VCTM), 0, 0, 8); }
"#;
        let mut engine = Engine::with_seed(29);
        engine.register_script_definition("ACTR", "Actor", script).expect("actor registers");
        let mut victim_definition = simple_definition("VCTM");
        victim_definition.set_physical(PhysicalInfo {
            energy: 100_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(victim_definition)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(
                SpawnConfig::new("VCTM")
                    .with_category(CATEGORY_OBJECT)
                    .with_alive(true),
            )
            .expect("victim spawns");
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_idx].state.controller = 5;
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[victim_idx].state.energy = 90_000;

        engine
            .call_object_function(actor_idx, "Zap", Vec::new())
            .expect("zap runs");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].last_energy_loss_cause, 5,
            "unset caused-by falls back to the caller's controller (C4Script.cpp:497)"
        );

        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine
            .call_object_function(actor_idx, "ZapAs", Vec::new())
            .expect("zap-as runs");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].last_energy_loss_cause, 7,
            "an explicit plus-one caused-by decodes to player 7 (C4Script.cpp:496)"
        );
    }

    // FnDoDamage (C4Script.cpp:508-515) -> C4Object::DoDamage
    // (C4Object.cpp:1279-1291): the change lands on a FOREIGN target too,
    // and the Damage script callback fires with (iChange, iCausedBy) —
    // the caused-by defaulting to the CALLER's controller.
    #[test]
    fn do_damage_reaches_a_foreign_target_and_fires_damage_like_cpp() {
        let actor_script = r#"#strict
func Zap() { return DoDamage(3, FindObject(VCTM)); }
"#;
        let victim_script = r#"#strict
local iSaw;
local iBy;
func Damage(iChange, iCausedBy) { iSaw = iChange; iBy = iCausedBy; return 1; }
"#;
        let mut engine = Engine::with_seed(31);
        engine.register_script_definition("ACTR", "Actor", actor_script).expect("actor registers");
        engine
            .register_script_definition("VCTM", "Victim", victim_script)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(SpawnConfig::new("VCTM").with_category(CATEGORY_OBJECT))
            .expect("victim spawns");
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_idx].state.controller = 5;

        let result = engine
            .call_object_function(actor_idx, "Zap", Vec::new())
            .expect("zap runs");
        assert_eq!(result, Value::Bool(true));
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.damage,
            3,
            "the foreign target takes the damage (C4Object.cpp:1288)"
        );
        assert_eq!(
            engine.objects[victim_idx].state.local_vars.get("iSaw"),
            Some(&Value::Int(3)),
            "the Damage callback fires with the change (C4Object.cpp:1290)"
        );
        assert_eq!(
            engine.objects[victim_idx].state.local_vars.get("iBy"),
            Some(&Value::Int(5)),
            "caused-by defaults to the caller's controller (C4Script.cpp:511)"
        );
    }

    // ObjectComPunch routes the energy loss through DoEnergy with the
    // ATTACKER's controller (C4ObjectCom.cpp:749: DoEnergy(-punch, false,
    // C4FxCall_EngGetPunched, cObj->Controller)) — punching an enemy off
    // a cliff must credit the puncher's kill.
    #[test]
    fn punch_marks_the_attackers_controller_on_the_kill_trace_like_cpp() {
        let script = r#"#strict
func Hit() { return Punch(FindObject(VCTM), 5); }
"#;
        let mut engine = Engine::with_seed(37);
        engine.register_script_definition("ACTR", "Actor", script).expect("actor registers");
        let mut victim_definition = simple_definition("VCTM");
        victim_definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(victim_definition)
            .expect("victim registers");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_category(CATEGORY_OBJECT))
            .expect("actor spawns");
        let victim = engine
            .spawn_object(
                SpawnConfig::new("VCTM")
                    .with_category(CATEGORY_OBJECT)
                    .with_alive(true),
            )
            .expect("victim spawns");
        let actor_idx = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_idx].state.controller = 4;
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[victim_idx].state.energy = 50_000;

        engine
            .call_object_function(actor_idx, "Hit", Vec::new())
            .expect("hit runs");
        let victim_idx = engine.find_object_index(victim).expect("victim exists");
        assert_eq!(
            engine.objects[victim_idx].state.energy,
            45_000,
            "the victim loses punch% energy (C4ObjectCom.cpp:749)"
        );
        assert_eq!(
            engine.objects[victim_idx].last_energy_loss_cause, 4,
            "the punch energy loss carries the attacker's controller (C4ObjectCom.cpp:749)"
        );
    }

    // C4Object::UpdatLastEnergyLossCause (C4Object.cpp:1369-1378):
    // self-administered damage (cause == own Controller) does not steal an
    // already-tracked killer — "stop-stop-throw while falling into teh
    // abyss" keeps the pusher's kill credit.
    #[test]
    fn update_last_energy_loss_cause_keeps_the_tracked_killer_like_cpp() {
        let mut engine = Engine::with_seed(19);
        let mut victim_definition = simple_definition("VCTM");
        victim_definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(victim_definition)
            .expect("victim registers");
        let victim = engine
            .spawn_object(
                SpawnConfig::new("VCTM")
                    .with_category(CATEGORY_OBJECT)
                    .with_alive(true)
                    .with_energy(50_000),
            )
            .expect("victim spawns");
        let idx = engine.find_object_index(victim).expect("victim exists");
        engine.objects[idx].state.controller = 3;

        // An enemy (player 5) hits first: tracked.
        engine
            .change_object_energy(idx, -1, C4FX_CALL_ENG_SCRIPT, 5)
            .expect("energy change succeeds");
        assert_eq!(engine.objects[idx].last_energy_loss_cause, 5);
        // Self-administered damage does not steal the kill
        // (iNewCausePlr == Controller and a tracked player >= 0).
        engine
            .change_object_energy(idx, -1, C4FX_CALL_ENG_SCRIPT, 3)
            .expect("energy change succeeds");
        assert_eq!(
            engine.objects[idx].last_energy_loss_cause, 5,
            "the tracked killer survives self-damage (C4Object.cpp:1373-1377)"
        );
        // A DIFFERENT player always updates.
        engine
            .change_object_energy(idx, -1, C4FX_CALL_ENG_SCRIPT, 6)
            .expect("energy change succeeds");
        assert_eq!(engine.objects[idx].last_energy_loss_cause, 6);
    }

