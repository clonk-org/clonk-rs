    #[test]
    fn walk_procedure_uses_walk_physical_limit_and_const_accel() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Walker", "Walker", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);
        definition.set_physical(PhysicalInfo {
            walk: 35_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );

        let id = engine
            .spawn_object(
                SpawnConfig::new("Walker")
                    .with_position(Vector2::new(0, 0))
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("walker spawns");
        let idx = engine.find_object_index(id).expect("walker exists");

        // DFA_WALK (C4Object.cpp:4771-4786): xdir += WalkAccel = FIXED100(50)
        // (C4Movement.cpp:34) = raw 32768 per frame, clamped to
        // lLimit = ValByPhysical(280, 35000) = itofix(35000*56, 2000000)
        // = raw 64225.
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 32768);
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 64225);
    }

    #[test]
    fn exec_action_resets_kill_trace_on_walk_before_later_death() {
        let script = r#"#strict
local death_by;
func Death(by) { death_by = by; return 1; }
"#;
        let mut definition =
            Definition::from_script("KTRC", "Kill trace walker", script).unwrap();
        definition.set_category(CATEGORY_LIVING);
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Dead".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));
        let id = engine
            .spawn_object(
                SpawnConfig::new("KTRC")
                    .with_category(CATEGORY_LIVING)
                    .with_alive(true)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("walker spawns");
        let idx = engine.find_object_index(id).expect("walker exists");
        engine.objects[idx].last_energy_loss_cause = 7;

        engine
            .apply_physics_at_index(idx)
            .expect("walk action executes");
        assert_eq!(
            engine.objects[idx].last_energy_loss_cause, OWNER_NONE,
            "a controllable action clears the stale attacker"
        );

        engine.assign_death(idx, false).expect("death assigns");
        let idx = engine.find_object_index(id).expect("corpse remains");
        assert_eq!(
            engine.objects[idx].state.local_vars.get("death_by"),
            Some(&Value::Int(OWNER_NONE)),
            "a later environmental death is credited to NO_OWNER"
        );
    }

    #[test]
    fn exec_action_retains_kill_trace_for_idle_flight_swim_disabled_and_fire() {
        let mut definition = simple_definition("KTRC");
        definition.set_category(CATEGORY_LIVING);
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                (
                    "Swim".to_string(),
                    ActionSpec::default().with_procedure("SWIM"),
                ),
                (
                    "DisabledWalk".to_string(),
                    ActionSpec::default()
                        .with_procedure("WALK")
                        .with_disabled(true),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        for (action, on_fire) in [
            ("Idle", false),
            ("Jump", false),
            ("Swim", false),
            ("DisabledWalk", false),
            ("Walk", true),
        ] {
            let id = engine
                .spawn_object(
                    SpawnConfig::new("KTRC")
                        .with_category(CATEGORY_LIVING)
                        .with_alive(true)
                        .with_action(ActionState::new(action)),
                )
                .expect("object spawns");
            let idx = engine.find_object_index(id).expect("object exists");
            engine.objects[idx].last_energy_loss_cause = 7;
            engine.objects[idx].state.on_fire = on_fire;

            engine
                .apply_physics_at_index(idx)
                .expect("action executes");
            assert_eq!(
                engine.objects[idx].last_energy_loss_cause, 7,
                "{action} (on_fire={on_fire}) must retain the attacker"
            );
        }
    }

    fn contained_flight_definition(id: &str, script: &str) -> Definition {
        let mut definition = Definition::from_script(id, id, script).expect("script compiles");
        definition.set_category(CATEGORY_LIVING);
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                (
                    "Tumble".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_disabled(true),
                ),
            ]),
        );
        definition
    }

    #[test]
    fn contained_flight_queues_replacing_exit_only_on_tick10() {
        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(contained_flight_definition("FALL", ""))
            .expect("flight definition registers");
        engine
            .register_definition(simple_definition("CONT"))
            .expect("container definition registers");
        engine.set_physics(PhysicsSettings::new(100, 20, -20));
        let container = engine
            .spawn_object(SpawnConfig::new("CONT"))
            .expect("container spawns");
        let container_idx = engine
            .find_object_index(container)
            .expect("container exists");
        engine.objects[container_idx].state.entrance_status = true;

        let mut actors = Vec::new();
        for action in ["Jump", "Tumble"] {
            let actor = engine
                .spawn_object(
                    SpawnConfig::new("FALL")
                        .with_category(CATEGORY_LIVING)
                        .with_alive(true)
                        .with_container(container)
                        .with_action(ActionState::new(action)),
                )
                .expect("contained flier spawns");
            let idx = engine.find_object_index(actor).expect("flier exists");
            engine.objects[idx].state.no_collect_delay = 2;
            engine.objects[idx]
                .commands
                .push_front(
                    CommandRequest::new(CommandId::MoveTo)
                        .with_tx(Some(20))
                        .with_ty(Some(5)),
                )
                .expect("old command queues");
            actors.push((actor, action));
        }

        engine.frame = 9;
        for &(actor, action) in &actors {
            let idx = engine.find_object_index(actor).expect("flier exists");
            engine
                .apply_physics_at_index(idx)
                .expect("non-Tick10 flight executes");
            let snapshot = engine.object_snapshot(actor).expect("flier remains");
            assert_eq!(snapshot.action.name, action);
            assert_eq!(engine.objects[idx].state.no_collect_delay, 2);
            assert_eq!(snapshot.command_stack.command_names(), vec!["MoveTo"]);
        }

        engine.frame = 10;
        for &(actor, _) in &actors {
            let idx = engine.find_object_index(actor).expect("flier exists");
            engine
                .apply_physics_at_index(idx)
                .expect("Tick10 flight executes");
            let snapshot = engine.object_snapshot(actor).expect("flier remains");
            assert_eq!(snapshot.action.name, "Walk");
            assert_eq!(snapshot.command_direction, CommandDirection::Stop);
            assert_eq!(snapshot.container, Some(container));
            assert_eq!(engine.objects[idx].state.no_collect_delay, 1);
            assert_eq!(
                engine.objects[idx].fixed_velocity.y,
                math::fixed100(20),
                "captured FLIGHT still applies gravity after stopping"
            );
            assert!(engine.objects[idx].state.mobile);
            assert_eq!(
                snapshot.command_stack.command_names(),
                vec!["Exit"],
                "SetCommand clears both the old command and delayed Wait"
            );
        }

        // The current command port executes Exit on frame 11; C++ spends
        // that frame on InitEvaluation and exits on frame 12. Checking after
        // both command phases accepts either timing without pinning L120.
        engine.tick_without_snapshot().expect("first Exit command phase runs");
        engine.tick_without_snapshot().expect("second Exit command phase runs");
        assert_eq!(engine.frame, 12);
        for &(actor, _) in &actors {
            let snapshot = engine.object_snapshot(actor).expect("flier remains");
            assert_eq!(snapshot.container, None);
            assert!(snapshot.command_stack.command_names().is_empty());
        }
    }

    #[test]
    fn exit_init_evaluation_cancels_live_attach_before_exit() -> Result<(), EngineError> {
        // An ordinary Exit spends its first Execute in InitEvaluation:
        // ObjectComCancelAttach changes DFA_ATTACH to ActIdle, but the
        // object remains contained until the next Execute
        // (C4Command.cpp:1554-1555,1654-1657).
        let mut actor = Definition::from_script("EXAT", "Attached exiter", "#strict 2\n")?;
        actor.configure_actions(
            Some("Attach".to_string()),
            HashMap::from([(
                "Attach".to_string(),
                ActionSpec::default().with_procedure("ATTACH"),
            )]),
        );

        let mut engine = Engine::with_seed(101);
        engine.register_definition(actor)?;
        engine.register_definition(simple_definition("EXAC"))?;
        let container_id = engine.spawn_object(
            SpawnConfig::new("EXAC").with_position(Vector2::new(40, 70)),
        )?;
        let container_index = engine
            .find_object_index(container_id)
            .expect("container exists");
        engine.objects[container_index].state.entrance_status = true;
        let actor_id = engine.spawn_object(
            SpawnConfig::new("EXAT")
                .with_category(CATEGORY_STATIC_BACK)
                .with_container(container_id)
                .with_action(ActionState::new("Attach")),
        )?;
        let actor_index = engine.find_object_index(actor_id).expect("actor exists");
        engine.objects[actor_index]
            .commands
            .push_back(CommandRequest::new(CommandId::Exit).with_mode(CommandMode::Base))
            .expect("Exit queues");

        engine.tick_without_snapshot()?;
        let snapshot = engine.object_snapshot(actor_id).expect("actor remains");
        assert_eq!(snapshot.action.name, "Idle");
        assert_eq!(snapshot.container, Some(container_id));
        assert_eq!(
            snapshot.command_stack.command_names(),
            vec!["Exit".to_string()]
        );

        engine.tick_without_snapshot()?;
        let snapshot = engine.object_snapshot(actor_id).expect("actor remains");
        assert_eq!(snapshot.action.name, "Idle");
        assert_eq!(snapshot.container, None);
        assert!(snapshot.command_stack.command_names().is_empty());
        Ok(())
    }

    #[test]
    fn collection_exit_runs_live_jump_with_preserved_comdir() -> Result<(), EngineError> {
        // C4Command::Exit's collection arm places the carryable one pixel
        // above the collection top, then calls ObjectComJump before Finish
        // (C4Command.cpp:643-649). ObjectComJump must still see the incoming
        // ComDir; facing is only its fallback (C4ObjectCom.cpp:284-296).
        let mut actor = Definition::from_script("EXJP", "Collection exit jumper", "#strict 2\n")?;
        actor.set_collectible(true);
        actor.set_physical(PhysicalInfo {
            walk: 50_001,
            jump: 60_001,
            ..PhysicalInfo::default()
        });
        actor.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
            ]),
        );

        let mut container = simple_definition("EXCT");
        container.set_collection_rect(Some(DefinitionRect::new(-13, -17, 26, 34)));

        let mut engine = Engine::with_seed(102);
        engine.register_definition(actor)?;
        engine.register_definition(container)?;
        let container_id = engine.spawn_object(
            SpawnConfig::new("EXCT").with_position(Vector2::new(120, 180)),
        )?;
        let container_index = engine
            .find_object_index(container_id)
            .expect("container exists");
        engine.objects[container_index].state.entrance_status = true;
        assert_eq!(
            engine.objects[container_index].state.ocf & ocf::ENTRANCE,
            0,
            "an actual entrance area would take priority over Collection"
        );

        let actor_id = engine.spawn_object(
            SpawnConfig::new("EXJP")
                // Keep the post-command state stable through this tick's
                // ExecAction/ExecMovement so the exact native launch is
                // observable after Engine::tick returns.
                .with_category(CATEGORY_STATIC_BACK)
                .with_construction(FULL_CON)
                .with_container(container_id)
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Left)
                .with_command_direction(CommandDirection::Right),
        )?;
        let actor_index = engine.find_object_index(actor_id).expect("actor exists");
        engine.objects[actor_index]
            .commands
            .push_back(
                CommandRequest::new(CommandId::Exit)
                    .with_mode(CommandMode::Base)
                    .with_evaluated(true),
            )
            .expect("evaluated Exit queues");

        engine.tick_without_snapshot()?;

        let actor_index = engine.find_object_index(actor_id).expect("actor remains");
        let actor = &engine.objects[actor_index];
        assert_eq!(actor.state.container, None);
        assert_eq!(actor.state.position, Vector2::new(120, 162));
        assert_eq!(
            actor.fixed_position,
            FixedVec2::from_ints(120, 162),
            "Exit installs the collection-area position before jumping"
        );
        assert_eq!(actor.state.action.name, "Jump");
        assert_eq!(
            actor.state.command_direction,
            CommandDirection::Right,
            "Exit must not replace the ComDir consumed by ObjectComJump"
        );
        assert_eq!(
            actor.fixed_velocity,
            FixedVec2::new(
                math::val_by_physical(280, 50_001),
                -math::val_by_physical(1000, 60_001),
            ),
            "Right ComDir wins over the actor's Left facing"
        );
        assert!(actor.state.mobile);
        assert!(actor.commands.command_names().is_empty());
        Ok(())
    }

    #[test]
    fn contained_flight_exit_honors_inside_vehicle_control_overload() {
        let actor_script = r#"#strict
local own_control_calls;
protected func ControlCommand() { own_control_calls = 1; return 1; }
"#;
        let container_script = r#"#strict
local control_calls, control_command, control_by;
protected func ControlCommand(command, target, tx, ty, target2, data, by)
{
    control_calls++;
    control_command = command;
    control_by = by;
    return 1;
}
"#;
        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(contained_flight_definition("FALL", actor_script))
            .expect("flight definition registers");
        let mut container_definition =
            Definition::from_script("CONT", "Control vehicle", container_script)
                .expect("container script compiles");
        container_definition.set_vehicle_control(VEHICLE_CONTROL_INSIDE);
        engine
            .register_definition(container_definition)
            .expect("container definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));
        let container = engine
            .spawn_object(SpawnConfig::new("CONT"))
            .expect("container spawns");
        let actor = engine
            .spawn_object(
                SpawnConfig::new("FALL")
                    .with_category(CATEGORY_LIVING)
                    .with_alive(true)
                    .with_controller(7)
                    .with_container(container)
                    .with_action(ActionState::new("Jump")),
            )
            .expect("contained flier spawns");
        let actor_idx = engine.find_object_index(actor).expect("flier exists");
        engine.objects[actor_idx].state.no_collect_delay = 2;
        engine.objects[actor_idx]
            .commands
            .push_front(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
            .expect("old command queues");

        engine.frame = 10;
        engine
            .apply_physics_at_index(actor_idx)
            .expect("Tick10 flight executes");

        let actor_snapshot = engine.object_snapshot(actor).expect("flier remains");
        assert_eq!(actor_snapshot.action.name, "Walk");
        assert_eq!(engine.objects[actor_idx].state.no_collect_delay, 1);
        assert!(
            actor_snapshot.command_stack.command_names().is_empty(),
            "truthy inside control consumes Exit after SetCommand clears the stack"
        );
        assert!(
            actor_snapshot
                .local_vars
                .get("own_control_calls")
                .is_none(),
            "native fControl=false skips the actor's own ControlCommand"
        );

        let container_idx = engine
            .find_object_index(container)
            .expect("container remains");
        let container_state = &engine.objects[container_idx].state;
        assert_eq!(container_state.controller, 7);
        assert_eq!(
            container_state.local_vars.get("control_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            container_state.local_vars.get("control_command"),
            Some(&Value::String("Exit".to_string().into()))
        );
        assert_eq!(
            container_state.local_vars.get("control_by"),
            Some(&compat::object_reference_value(actor))
        );
    }

    #[test]
    fn walk_procedure_automatically_steers_rotation_to_floor_slope() {
        // DFA_WALK calls AdjustWalkRotation(20,20,100) after xdir steering
        // when the rotateable shape retained a material attachment
        // (C4Object.cpp:4817-4821). This is the same hand-derived slope
        // oracle as the script-host test: left offset +9, right 0 => -9 deg.
        let mut definition = Definition::from_script("Walker", "Walker", "").unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);
        definition.set_rotateable(45);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM)]);
        definition.set_physical(PhysicalInfo {
            walk: 100_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine.register_definition(definition).unwrap();
        let mut surface = vec![25; 32];
        surface.extend(vec![5; 32]);
        engine.set_landscape(Landscape::new(64, surface).unwrap());
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let id = engine
            .spawn_object(
                SpawnConfig::new("Walker")
                    .with_category(CATEGORY_OBJECT)
                    .with_command_direction(CommandDirection::Right),
            )
            .unwrap();
        let idx = engine.find_object_index(id).unwrap();
        engine.objects[idx].state.shape_attach = ShapeAttachRecord {
            mat_valid: true,
            mat_vehicle: false,
            x: 30,
            y: 15,
            vtx: 0,
        };

        engine
            .apply_physics_at_index(idx)
            .expect("walk physics applies");

        assert_eq!(
            engine.objects[idx].rotation_velocity,
            C4Fixed::from_raw(-9 * 65536 / 100)
        );
    }

    #[test]
    fn stationary_walk_on_offset_vertex_still_adjusts_rotation() {
        // The internal WALK gate uses the DEFINITION vertex x, so a walker
        // attached away from its center rotates even with xdir == 0. The live
        // positive vertex selects the -50 target, clamped to -15 degrees.
        let mut definition = Definition::from_script("Walker", "Walker", "").unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);
        definition.set_rotateable(45);
        definition.set_shape_vertices(vec![ObjectVertex::new(5, 0).with_cnat(CNAT_BOTTOM)]);
        definition.set_physical(PhysicalInfo {
            walk: 100_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine.register_definition(definition).unwrap();
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let id = engine
            .spawn_object(SpawnConfig::new("Walker").with_category(CATEGORY_OBJECT))
            .unwrap();
        let idx = engine.find_object_index(id).unwrap();
        engine.objects[idx].state.shape_attach = ShapeAttachRecord {
            mat_valid: true,
            mat_vehicle: false,
            x: 0,
            y: 0,
            vtx: 0,
        };

        engine
            .apply_physics_at_index(idx)
            .expect("stationary walk physics applies");

        assert_eq!(
            engine.objects[idx].rotation_velocity,
            C4Fixed::from_raw(-15 * 65536 / 100)
        );
    }

    #[test]
    fn walk_rotation_fallback_stops_existing_spin() {
        // C++'s `else rdir = 0` is unconditional: a centered stationary
        // walker does not retain angular velocity from its previous action.
        let mut definition = Definition::from_script("Walker", "Walker", "").unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        definition.configure_actions(Some("Walk".to_string()), actions);
        definition.set_rotateable(45);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0).with_cnat(CNAT_BOTTOM)]);
        definition.set_physical(PhysicalInfo {
            walk: 100_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine.register_definition(definition).unwrap();
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let id = engine
            .spawn_object(SpawnConfig::new("Walker").with_category(CATEGORY_OBJECT))
            .unwrap();
        let idx = engine.find_object_index(id).unwrap();
        engine.objects[idx].state.shape_attach = ShapeAttachRecord {
            mat_valid: true,
            mat_vehicle: false,
            x: 0,
            y: 0,
            vtx: 0,
        };
        engine.objects[idx].rotation_velocity = itofix(3);

        engine
            .apply_physics_at_index(idx)
            .expect("walk fallback physics applies");

        assert_eq!(engine.objects[idx].rotation_velocity, C4Fixed::ZERO);
    }

    #[test]
    fn scale_procedure_uses_scale_physical_limit_and_trains_at_limit() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Climber", "Climber", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Scale".to_string(),
            ActionSpec::default().with_procedure("scale"),
        );
        definition.configure_actions(Some("Scale".to_string()), actions);
        definition.set_physical(PhysicalInfo {
            scale: 30_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Climber")
                    .with_position(Vector2::new(0, 0))
                    .with_command_direction(CommandDirection::Up),
            )
            .expect("climber spawns");
        let idx = engine.find_object_index(id).expect("climber exists");
        engine.objects[idx].state.temporary_physical = Some(PhysicalInfo {
            scale: 30_000,
            ..PhysicalInfo::default()
        });

        // DFA_SCALE (C4Object.cpp:4805-4837): ydir -= WalkAccel (raw 32768),
        // clamped to lLimit = ValByPhysical(200, 30000)
        // = itofix(30000*40, 2000000) = raw 39321.
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.y.val(), -32768);
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.y.val(), -39321);
        assert_eq!(engine.objects[idx].state.info_physical, None);

        // Tick5 at-limit training (C4Object.cpp:4810-4812): frame 5 sees
        // |ydir| == lLimit and trains Scale by 1.
        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        let trained = engine.objects[idx]
            .state
            .temporary_physical
            .expect("at-limit Tick5 trains the temporary physicals");
        assert_eq!(trained.scale, 30_001);
    }

    #[test]
    fn attached_scaler_hangles_when_up_hits_the_ceiling_like_cpp() {
        // C4Movement::DoMovement accumulates the ceiling vertex's CNAT_Top
        // even on the attached-shape path (C4Movement.cpp:337-372). The
        // ensuing ContactAction changes a left-facing scaler pressing Up to
        // Hangle facing Right (C4Object.cpp:4369-4390).
        let mut climber = Definition::from_script("Climber", "Climber", "#strict\n")
            .expect("definition compiles");
        climber.set_shape_vertices(vec![
            ObjectVertex::new(-1, 0).with_cnat(CNAT_LEFT),
            ObjectVertex::new(0, -1).with_cnat(CNAT_TOP),
        ]);
        climber.set_contact_density(50);
        climber.set_physical(PhysicalInfo {
            scale: 30_000,
            hangle: 30_000,
            can_hangle: 1,
            ..PhysicalInfo::default()
        });
        climber.configure_actions(
            None,
            HashMap::from([
                (
                    "Scale".to_string(),
                    ActionSpec::default().with_procedure("SCALE"),
                ),
                (
                    "Hangle".to_string(),
                    ActionSpec::default().with_procedure("HANGLE"),
                ),
            ]),
        );

        // Keep the left attachment at x=8 and put the top vertex into the
        // ceiling at (10,9). Starting from an exact pixel, Scale's first
        // -0.5 y step still rounds to y=10, matching the C++ oracle fixture.
        let mut pixels = vec![0_u8; 20 * 20];
        pixels[10 * 20 + 8] = 1;
        pixels[9 * 20 + 10] = 1;
        let grid = landscape::PixelGrid::new(
            20,
            20,
            pixels,
            vec![0, 100],
            vec![None, Some("Earth".to_string())],
            vec![None; 2],
        );
        let mut landscape = Landscape::new(20, vec![0; 20]).expect("landscape builds");
        // The 20x20 pixel plane is this fixture's GBackWdt/GBackHgt. Pin
        // GBackHgt explicitly just like the real landscape loader; the
        // surface-depth fallback is zero because this fixture only paints
        // two isolated contact pixels.
        landscape.set_world_height(20);
        landscape.set_pixel_grid(grid);

        let mut engine = Engine::with_seed(0);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(climber)
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Climber")
                    .with_category(CATEGORY_OBJECT | CATEGORY_LIVING)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_action(ActionState::new("Scale"))
                    .with_direction(Direction::Left)
                    .with_command_direction(CommandDirection::Up)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("climber spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(id).expect("climber exists");
        let object = &engine.objects[idx];
        assert_eq!(object.state.action.name, "Hangle");
        assert_eq!(object.state.direction, Direction::Right);
        assert_eq!(object.state.command_direction, CommandDirection::Up);
        assert_eq!(object.state.position, Vector2::new(10, 10));
        assert_eq!(object.fixed_position, FixedVec2::from_ints(10, 10));
        assert_eq!(object.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(object.state.velocity, Vector2::ZERO);
    }

    #[test]
    fn hangle_procedure_uses_hangle_physical_limit_and_trains_at_limit() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Hangler", "Hangler", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Hangle".to_string(),
            // C++ maps procedure names case-sensitively and Directions=1
            // only permits DIR_Left; this fixture exercises rightward
            // HANGLE steering, so it needs both direction slots.
            ActionSpec::default()
                .with_procedure("HANGLE")
                .with_directions(2),
        );
        definition.configure_actions(Some("Hangle".to_string()), actions);
        definition.set_physical(PhysicalInfo {
            hangle: 40_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Hangler")
                    .with_position(Vector2::new(0, 0))
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("hangler spawns");
        let idx = engine.find_object_index(id).expect("hangler exists");
        engine.objects[idx].state.temporary_physical = Some(PhysicalInfo {
            hangle: 40_000,
            ..PhysicalInfo::default()
        });

        // DFA_HANGLE (C4Object.cpp:4840-4872): xdir += WalkAccel (raw 32768),
        // clamped to lLimit = ValByPhysical(160, 40000)
        // = itofix(40000*32, 2000000) = raw 41943; ydir = 0.
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 32768);
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 41943);
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(engine.objects[idx].state.direction, Direction::Right);
        assert_eq!(engine.objects[idx].state.info_physical, None);

        // Tick5 at-limit training (C4Object.cpp:4844-4846).
        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        engine.tick_without_snapshot().expect("tick succeeds");
        let trained = engine.objects[idx]
            .state
            .temporary_physical
            .expect("at-limit Tick5 trains the temporary physicals");
        assert_eq!(trained.hangle, 40_001);
    }

    // DFA_SWIM animates at fixtoi(swim-limit * 10) — the PHYSICAL limit
    // (ValByPhysical(160, Swim)), not the velocity: a drifting fish with
    // xdir 0 still flips through its Turn phases (probe-verified: FISH
    // adv=16, WIPF adv=10 at frame 1).
    #[test]
    fn swim_phase_advances_by_the_physical_limit_like_cpp() {
        let mut fish = Definition::from_script("Fish", "Fish", "#strict\n").expect("compiles");
        fish.set_physical(PhysicalInfo {
            swim: 100_000,
            ..PhysicalInfo::default()
        });
        fish.configure_actions(
            None,
            HashMap::from([(
                "Turn".to_string(),
                ActionSpec::default()
                    .with_procedure("SWIM")
                    .with_delay(3)
                    .with_length(15)
                    .with_next("Turn"),
            )]),
        );
        let mut engine = Engine::with_seed(0);
        engine.set_landscape(Landscape::flat(50, 50));
        engine.register_definition(fish).expect("registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Fish")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_action(ActionState::new("Turn")),
            )
            .expect("spawns");
        // C4Object InLiquid: these fixtures have no water — arm the flag
        // so the DFA_SWIM out-of-liquid exit (C4Object.cpp:4946-4956)
        // does not convert the swimmer to Walk.
        {
            let idx = engine.find_object_index(id).expect("swimmer exists");
            engine.objects[idx].state.in_liquid = true;
        }
        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(
            engine.objects[idx].state.action.phase, 1,
            "PhaseDelay += fixtoi(ValByPhysical(160,100000)*10)=16 >= Delay 3 on the first exec"
        );
    }

    // DFA_SWIM COMD_Stop decay (C4Object.cpp:4952-4958): each exec takes
    // one SwimAccel = FIXED100(20) = raw 13107 off xdir, then the dead
    // zone `(xdir > -SwimAccel) && (xdir < +SwimAccel)` snaps to 0. From
    // the FISH's full swim speed lLimit = ValByPhysical(160, 100000) =
    // raw 104857 the ladder is 91750, 78643, 65536, 52429, 39322, 26215,
    // 13108, then 13108-13107=1 falls in the dead zone -> EXACTLY 0 on
    // the 9th exec. NB the ladder passes 26215, never 26214=2*13107 —
    // a swimmer showing 26214 got there by ACCELERATING from 0 (comdir
    // Left/Right), not by a Stop decay (the f100 fish-wall tell).
    #[test]
    fn swim_stop_decays_fish_xdir_to_exact_zero_on_the_cpp_schedule() {
        let mut definition =
            Definition::from_script("FISH", "Fish", "#strict\n").expect("compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Swim".to_string(),
            ActionSpec::default().with_procedure("SWIM"),
        );
        definition.configure_actions(Some("Swim".to_string()), actions);
        definition.set_physical(PhysicalInfo {
            swim: 100_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine.register_definition(definition).expect("registers");
        engine.set_physics(PhysicsSettings::new(2, 20, -20));
        let id = engine
            .spawn_object(
                SpawnConfig::new("FISH")
                    .with_position(Vector2::new(0, 0))
                    .with_command_direction(CommandDirection::Stop),
            )
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");
        engine.objects[idx].state.in_liquid = true;
        engine.objects[idx].fixed_velocity.x = C4Fixed::from_raw(104857);
        engine.objects[idx].state.mobile = true;

        let ladder = [
            91750, 78643, 65536, 52429, 39322, 26215, 13108, 0, 0,
        ];
        for (tick, expected) in ladder.iter().enumerate() {
            engine.tick_without_snapshot().expect("tick");
            let idx = engine.find_object_index(id).expect("exists");
            assert_eq!(
                engine.objects[idx].fixed_velocity.x.val(),
                *expected,
                "COMD_Stop decay after tick {}",
                tick + 1
            );
            assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        }
    }

    // The fish's TurnLeft chain executed from a TimerCall Activity
    // (post-movement, C4Object.cpp:1094-1102): Stuck() (quiet
    // Shape.CheckContact, C4Script.cpp:1858-1861), GetXDir() at default
    // precision 10 (0.4 px/f reads as 4, C4Script.cpp:1167), SetXDir(0),
    // then SetDir(DIR_Left) firing the Swim TurnAction
    // (C4Object.cpp:4236-4256) and SetComDir — the drifting xdir must be
    // ZERO afterwards even though the SetDir switches the action.
    #[test]
    fn timer_call_turn_left_zeroes_a_small_positive_swim_drift() {
        let script = r#"#strict
protected func Activity()
{
  if (Stuck() || (GetAction() ne "Walk" && GetAction() ne "Swim")) return();
  if (GetXDir() > 0) SetXDir(0);
  SetDir(DIR_Left());
  SetComDir(COMD_Left());
  return(1);
}
"#;
        let mut fish = Definition::from_script("FISH", "Fish", script).expect("compiles");
        fish.set_c4_callback_convention(true);
        fish.set_shape_rect(Some(DefinitionRect::new(-8, -6, 16, 12)));
        fish.set_shape_vertices(vec![
            ObjectVertex {
                x: -4,
                y: 0,
                cnat: CNAT_LEFT,
                friction: 100,
            },
            ObjectVertex {
                x: 0,
                y: -3,
                cnat: CNAT_TOP,
                friction: 100,
            },
            ObjectVertex {
                x: 4,
                y: 0,
                cnat: CNAT_RIGHT,
                friction: 100,
            },
            ObjectVertex {
                x: 0,
                y: 4,
                cnat: CNAT_BOTTOM,
                friction: 100,
            },
        ]);
        let mut actions = HashMap::new();
        actions.insert(
            "Swim".to_string(),
            ActionSpec::default().with_procedure("SWIM"),
        );
        actions.insert(
            "Turn".to_string(),
            ActionSpec::default()
                .with_procedure("SWIM")
                .with_delay(3)
                .with_length(15)
                .with_next("Swim"),
        );
        let mut swim_spec = actions.get("Swim").cloned().expect("swim spec");
        swim_spec = swim_spec.with_turn_action("Turn");
        actions.insert("Swim".to_string(), swim_spec);
        fish.configure_actions(Some("Swim".to_string()), actions);
        fish.set_physical(PhysicalInfo {
            swim: 100_000,
            ..PhysicalInfo::default()
        });
        fish.set_timer(1);
        fish.set_timer_call(Some("Activity".to_string()));

        let mut engine = Engine::with_seed(5);
        engine.set_physics(PhysicsSettings::new(2, 20, -20));
        engine.register_definition(fish).expect("registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("FISH")
                    .with_position(Vector2::new(100, 50))
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");
        engine.objects[idx].state.in_liquid = true;
        engine.objects[idx].state.direction = Direction::Right;
        engine.objects[idx].fixed_velocity.x = C4Fixed::from_raw(13107);
        engine.objects[idx].state.mobile = true;

        // One tick: the swim arm accelerates 13107 -> 26214 (comdir
        // Right), movement integrates it, THEN the TimerCall Activity
        // turns: SetXDir(0) must survive the TurnAction switch.
        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("exists");
        let object = &engine.objects[idx];
        assert_eq!(
            object.fixed_velocity.x,
            C4Fixed::ZERO,
            "TurnLeft's SetXDir(0) zeroes the 26214 drift (C4Script SetXDir \
             + C4Object::SetDir TurnAction, C4Object.cpp:4236-4256)"
        );
        assert_eq!(object.state.action.name, "Turn", "TurnAction fired");
        assert_eq!(object.state.command_direction, CommandDirection::Left);
    }

    // FnSetActionTargets assigns Action.Target/Target2 UNCONDITIONALLY
    // (C4Script.cpp:1108-1116): unfilled parameters are nil, so a bare
    // `SetActionTargets()` CLEARS both targets. The GoldRush intro's
    // DisconnectWagon relies on this (Horse.c4d Script.c:398) — treating
    // missing args as "keep" left the horse's pull target alive, so the
    // following SetGait(3) saw fPulling and kept Pull3 where C++ galloped
    // (the f105 wall: rust Pull3 vs cpp Gallop).
    #[test]
    fn bare_set_action_targets_clears_both_targets_like_cpp() {
        let script = r#"#strict
protected func Activity() { SetActionTargets(); return(1); }
"#;
        let mut horse = Definition::from_script("HRSE", "Horse", script).expect("compiles");
        horse.set_c4_callback_convention(true);
        let mut actions = HashMap::new();
        actions.insert("Pull3".to_string(), ActionSpec::default());
        horse.configure_actions(Some("Pull3".to_string()), actions);
        horse.set_timer(1);
        horse.set_timer_call(Some("Activity".to_string()));

        let mut engine = Engine::with_seed(0);
        engine.register_definition(horse).expect("registers");
        let coach = engine
            .spawn_object(SpawnConfig::new("HRSE").with_position(Vector2::new(50, 50)))
            .expect("spawns");
        let id = engine
            .spawn_object(SpawnConfig::new("HRSE").with_position(Vector2::new(100, 50)))
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");
        engine.objects[idx].state.action.target = Some(coach);
        engine.objects[idx].state.action.target2 = Some(coach);

        engine.tick_without_snapshot().expect("tick");
        let idx = engine.find_object_index(id).expect("exists");
        assert_eq!(
            engine.objects[idx].state.action.target, None,
            "SetActionTargets() nil-fills pTarget1 -> Action.Target cleared"
        );
        assert_eq!(
            engine.objects[idx].state.action.target2, None,
            "SetActionTargets() nil-fills pTarget2 -> Action.Target2 cleared"
        );
    }

    #[test]
    fn swim_procedure_uses_swim_physical_limit_and_trains_on_tick10() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Swimmer", "Swimmer", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Swim".to_string(),
            // DIR_Right is valid only when the ActMap declares two
            // directions (C4Object::SetDir); procedure mapping is exact.
            ActionSpec::default()
                .with_procedure("SWIM")
                .with_directions(2),
        );
        definition.configure_actions(Some("Swim".to_string()), actions);
        definition.set_physical(PhysicalInfo {
            swim: 50_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        // Nonzero gravity pins that DFA_SWIM never applies gravity
        // (C4Object.cpp:4920-4970 has no DoGravity call).
        engine.set_physics(PhysicsSettings::new(2, 20, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Swimmer")
                    .with_position(Vector2::new(0, 0))
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("swimmer spawns");
        let idx = engine.find_object_index(id).expect("swimmer exists");
        engine.objects[idx].state.temporary_physical = Some(PhysicalInfo {
            swim: 50_000,
            ..PhysicalInfo::default()
        });

        // DFA_SWIM (C4Object.cpp:4920-4960): xdir += SwimAccel = FIXED100(20)
        // (C4Movement.cpp:34) = raw 13107, clamped to
        // lLimit = ValByPhysical(160, 50000) = itofix(50000*32, 2000000)
        // = raw 52428 — reached exactly on the fourth frame.
        // C4Object InLiquid: these fixtures have no water — arm the flag
        // so the DFA_SWIM out-of-liquid exit (C4Object.cpp:4946-4956)
        // does not convert the swimmer to Walk.
        {
            let idx = engine.find_object_index(id).expect("swimmer exists");
            engine.objects[idx].state.in_liquid = true;
        }
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 13107);
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        for _ in 0..4 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 52428);
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(engine.objects[idx].state.direction, Direction::Right);
        assert_eq!(engine.objects[idx].state.info_physical, None);

        // Tick10 at-limit training (C4Object.cpp:4924-4926).
        for _ in 0..5 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        let trained = engine.objects[idx]
            .state
            .temporary_physical
            .expect("at-limit Tick10 trains the temporary physicals");
        assert_eq!(trained.swim, 50_001);
    }

    #[test]
    fn pxs_wind_drift_dies_in_tunnel_background() {
        // GBackWind (C4Wrappers.h:189-192): IFT pixels read wind 0; the PXS
        // free-fall drift (C4PXS.cpp:62-74) is position-dependent. The
        // Random(1200) jitter draws happen either way.
        let library = MaterialLibrary::parse(
            r#"
            [Material Dust]
            Name=Dust
            Density=25
            WindDrift=30
            SplashRate=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let dust = materials.id_of("Dust").expect("dust exists");

        let build_engine = |tunnel: bool| {
            let mut engine = Engine::with_seed(21);
            engine.set_materials(materials.clone());
            let mut landscape = Landscape::flat(8, 100);
            if tunnel {
                landscape.set_tunnel_column(2, vec![(0, 50)]);
            }
            engine.set_landscape(landscape);
            engine.set_environment(EnvironmentSettings::new(80));
            assert!(engine.pxs_system.create(
                dust,
                math::itofix(2),
                math::itofix(5),
                math::C4Fixed::ZERO,
                math::C4Fixed::ZERO,
            ));
            engine
        };

        // Expected in-tunnel xdir: wind 0 ⇒ txdir = FIXED256(r1 - 600),
        // xdir += (txdir - 0) * (30-20) * itofix(1, 800).
        let mut tunnel_engine = build_engine(true);
        let mut mirror = tunnel_engine.rng.clone();
        let r1 = mirror.random(1200);
        let _ = mirror.random(1200);
        let expected_txdir = math::fixed256(r1 - 600);
        let expected_xdir = expected_txdir * 10 * math::itofix_prec(1, 800);
        tunnel_engine.tick_pxs();
        let pixel: Vec<pxs::Pxs> = tunnel_engine.pxs_system.iter().copied().collect();
        assert_eq!(pixel.len(), 1);
        assert_eq!(pixel[0].xdir, expected_xdir, "tunnel reads wind 0");
        assert_eq!(tunnel_engine.rng, mirror, "jitter draws happen either way");

        // Control: the same pixel outside the tunnel picks up the wind term
        // itofix(80, 15) in txdir.
        let mut open_engine = build_engine(false);
        open_engine.tick_pxs();
        let open_pixel: Vec<pxs::Pxs> = open_engine.pxs_system.iter().copied().collect();
        assert_eq!(open_pixel.len(), 1);
        let open_txdir = math::itofix_prec(80, 15) + math::fixed256(r1 - 600);
        let open_xdir = open_txdir * 10 * math::itofix_prec(1, 800);
        assert_eq!(open_pixel[0].xdir, open_xdir);
        assert_ne!(open_pixel[0].xdir, pixel[0].xdir);
    }

    #[test]
    fn pxs_wind_drift_uses_the_grid_ift_bit() {
        // GBackIFT reads the live landscape byte's 0x80 bit through GetPix,
        // not only the fixture tunnel columns (C4Wrappers.h:159-162). PXS
        // therefore sees wind zero at an IFT grid pixel (C4PXS.cpp:62-74).
        let library = MaterialLibrary::parse(
            r#"
            [Material Dust]
            Name=Dust
            Density=25
            WindDrift=30
            SplashRate=0
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let dust = materials.id_of("Dust").expect("dust exists");

        let build_engine = |ift: bool| {
            let mut engine = Engine::with_seed(21);
            engine.set_materials(materials.clone());
            let mut bytes = vec![0_u8; 8 * 100];
            if ift {
                bytes[5 * 8 + 2] = 0x80;
            }
            let grid = landscape::PixelGrid::new(
                8,
                100,
                bytes,
                vec![0],
                vec![None],
                vec![None],
            );
            let mut landscape = Landscape::flat(8, 100);
            landscape.set_pixel_grid(grid);
            engine.set_landscape(landscape);
            engine.set_environment(EnvironmentSettings::new(80));
            assert!(engine.pxs_system.create(
                dust,
                math::itofix(2),
                math::itofix(5),
                math::C4Fixed::ZERO,
                math::C4Fixed::ZERO,
            ));
            engine
        };

        let mut ift_engine = build_engine(true);
        assert!(ift_engine
            .landscape()
            .is_some_and(|landscape| landscape.is_ift_at(2, 5)));
        let mut mirror = ift_engine.rng.clone();
        let r1 = mirror.random(1200);
        let _ = mirror.random(1200);
        let expected_ift_xdir =
            math::fixed256(r1 - 600) * 10 * math::itofix_prec(1, 800);
        ift_engine.tick_pxs();
        let ift_pixel: Vec<pxs::Pxs> = ift_engine.pxs_system.iter().copied().collect();
        assert_eq!(ift_pixel.len(), 1);
        assert_eq!(ift_pixel[0].xdir, expected_ift_xdir, "IFT grid byte reads wind 0");
        assert_eq!(ift_engine.rng, mirror, "jitter draws remain unconditional");

        let mut open_engine = build_engine(false);
        assert!(open_engine
            .landscape()
            .is_some_and(|landscape| !landscape.is_ift_at(2, 5)));
        open_engine.tick_pxs();
        let open_pixel: Vec<pxs::Pxs> = open_engine.pxs_system.iter().copied().collect();
        assert_eq!(open_pixel.len(), 1);
        let open_txdir = math::itofix_prec(80, 15) + math::fixed256(r1 - 600);
        let expected_open_xdir = open_txdir * 10 * math::itofix_prec(1, 800);
        assert_eq!(open_pixel[0].xdir, expected_open_xdir);
        assert_ne!(open_pixel[0].xdir, ift_pixel[0].xdir);
        assert_eq!(open_engine.rng, mirror, "both paths consume two draws");
    }

    #[test]
    fn dig_procedure_uses_dig_physical_speed() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Digger", "Digger", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            // DIG's UpRight arm calls SetDir(DIR_Right), which C++ accepts
            // only for an action with Directions=2.
            ActionSpec::default()
                .with_procedure("DIG")
                .with_directions(2),
        );
        definition.configure_actions(Some("Dig".to_string()), actions);
        definition.set_physical(PhysicalInfo {
            dig: 40_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(2, 20, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_position(Vector2::new(0, 0))
                    .with_command_direction(CommandDirection::UpRight),
            )
            .expect("digger spawns");
        let idx = engine.find_object_index(id).expect("digger exists");

        // DFA_DIG (C4Object.cpp:4888-4915): direct dirs from
        // lLimit = ValByPhysical(125, 40000) = itofix(40000*25, 2000000)
        // = raw 32768; COMD_UpRight sets xdir = +lLimit, ydir = -lLimit/2.
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 32768);
        assert_eq!(engine.objects[idx].fixed_velocity.y.val(), -16384);
        assert_eq!(engine.objects[idx].state.direction, Direction::Right);
    }

    #[test]
    fn stopped_dig_freezes_phase_but_counts_action_time_like_cpp() {
        let mut definition =
            Definition::from_script("Digger", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default()
                .with_procedure("dig")
                .with_length(16)
                .with_delay(15)
                .with_next("Dig"),
        );
        definition.configure_actions(Some("Dig".to_string()), actions);
        definition.set_physical(PhysicalInfo {
            dig: 40_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_action(ActionState::new("Dig"))
                    .with_command_direction(CommandDirection::Stop),
            )
            .expect("digger spawns");

        engine.tick_without_snapshot().expect("tick succeeds");

        // DFA_DIG COMD_Stop sets iPhaseAdvance=0 (C4Object.cpp:4906-4935),
        // while Action.Time still increments before phase handling (:4763,
        // :5463-5471). The action remains Dig on its current visual frame.
        let action = &engine.object_snapshot(id).expect("digger exists").action;
        assert_eq!(action.phase, 0);
        assert_eq!(action.ticks, 0);
        assert_eq!(action.time, 1);
    }

    #[test]
    fn float_procedure_uses_float_physical_limit_and_const_accel() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Boat", "Boat", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Float".to_string(),
            ActionSpec::default().with_procedure("float"),
        );
        definition.configure_actions(Some("Float".to_string()), actions);
        definition.set_physical(PhysicalInfo {
            float: 20,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        // Nonzero gravity pins that DFA_FLOAT never applies gravity
        // (C4Object.cpp:5268-5287 has no DoGravity call).
        engine.set_physics(PhysicsSettings::new(2, 20, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Boat")
                    .with_position(Vector2::new(0, 0))
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("boat spawns");
        let idx = engine.find_object_index(id).expect("boat exists");

        // DFA_FLOAT (C4Object.cpp:5268-5286): xdir += FloatAccel = FIXED100(10)
        // (C4Movement.cpp:33) = raw 6553, clamped to lLimit = FIXED100(Float)
        // = FIXED100(20) = raw 13107 — NOT ValByPhysical.
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 6553);
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 13106);
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 13107);
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
    }

    fn grab_lost_push_fixture(
        pusher_position: Vector2,
        vehicle_position: Vector2,
    ) -> (Engine, ObjectId, ObjectId) {
        let mut pusher = Definition::from_script("PSHR", "Pusher", "#strict")
            .expect("pusher compiles");
        pusher.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        pusher.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        pusher.set_contact_density(50);
        pusher.set_physical(PhysicalInfo {
            walk: 35_000,
            push: 45_000,
            ..PhysicalInfo::default()
        });
        pusher.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Push".to_string(),
                    ActionSpec::default().with_procedure("PUSH"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
            ]),
        );

        let vehicle_script = r#"#strict
local pusher, grab_lost_calls, action_seen;
public func Arm(actor) { pusher = actor; }
protected func GrabLost()
{
    grab_lost_calls = grab_lost_calls + 1;
    action_seen = GetAction(pusher);
}
"#;
        let mut vehicle = Definition::from_script("VEHI", "Vehicle", vehicle_script)
            .expect("vehicle compiles");
        vehicle.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        vehicle.set_grab(1);
        vehicle.set_mass(200);

        let mut engine = Engine::with_seed(7);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(pusher).expect("pusher registers");
        engine
            .register_definition(vehicle)
            .expect("vehicle registers");
        let vehicle_id = engine
            .spawn_object(
                SpawnConfig::new("VEHI")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(vehicle_position)
                    .with_fixed_position(FixedVec2::from_ints(
                        vehicle_position.x,
                        vehicle_position.y,
                    ))
                    .with_loaded(true),
            )
            .expect("vehicle spawns");
        let mut push = ActionState::new("Push");
        push.target = Some(vehicle_id);
        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("PSHR")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(pusher_position)
                    .with_fixed_position(FixedVec2::from_ints(
                        pusher_position.x,
                        pusher_position.y,
                    ))
                    .with_action(push)
                    .with_command_direction(CommandDirection::Right)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("pusher spawns");

        let vehicle_idx = engine
            .find_object_index(vehicle_id)
            .expect("vehicle exists");
        engine
            .call_object_function(
                vehicle_idx,
                "Arm",
                vec![object_reference_value(pusher_id)],
            )
            .expect("vehicle arms callback");
        let pusher_idx = engine
            .find_object_index(pusher_id)
            .expect("pusher exists");
        let commands = &mut engine.objects[pusher_idx].commands;
        commands
            .push_back(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(20))
                    .with_ty(Some(5)),
            )
            .expect("MoveTo queues");
        commands
            .push_back(CommandRequest::new(CommandId::PushTo).with_target(Some(vehicle_id)))
            .expect("PushTo queues");
        commands
            .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
            .expect("Wait queues");
        assert_eq!(
            commands.command_names(),
            vec!["MoveTo", "PushTo", "Wait"]
        );

        (engine, pusher_id, vehicle_id)
    }

    fn no_attach_fighter_definition() -> Definition {
        let mut fighter = Definition::from_script(
            "FGTR",
            "Fighter",
            r#"#strict
local death_by;
func Death(by) { death_by = by; return 1; }
"#,
        )
        .expect("fighter compiles");
        fighter.set_category(CATEGORY_LIVING);
        fighter.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        fighter.set_contact_density(50);
        fighter.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Fight".to_string(),
                    ActionSpec::default().with_procedure("FIGHT"),
                ),
                (
                    "Jump".to_string(),
                    ActionSpec::default().with_procedure("FLIGHT"),
                ),
                ("Dead".to_string(), ActionSpec::default()),
            ]),
        );
        fighter
    }

    #[test]
    fn fight_no_attach_credits_opponent_controller_through_fall_death() {
        let mut engine = Engine::with_seed(7);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(no_attach_fighter_definition())
            .expect("fighter registers");
        engine
            .register_definition(simple_definition("OPPN"))
            .expect("opponent registers");

        let opponent = engine
            .spawn_object(
                SpawnConfig::new("OPPN")
                    .with_owner(2)
                    .with_controller(9)
                    .with_position(Vector2::new(8, 5)),
            )
            .expect("opponent spawns");
        let mut fight = ActionState::new("Fight");
        fight.target = Some(opponent);
        let fighter = engine
            .spawn_object(
                SpawnConfig::new("FGTR")
                    .with_category(CATEGORY_LIVING)
                    .with_owner(1)
                    .with_controller(1)
                    .with_alive(true)
                    .with_energy(100_000)
                    .with_position(Vector2::new(5, 5))
                    .with_fixed_position(FixedVec2::from_ints(5, 5))
                    .with_action(fight)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("fighter spawns");

        let mut landscape = vehicle_grid_landscape(16, 16);
        landscape.set_world_height(16);
        landscape.grid_write_byte(5, 6, 1);
        engine.set_landscape(landscape);
        let fighter_idx = engine.find_object_index(fighter).expect("fighter exists");
        engine.objects[fighter_idx].last_energy_loss_cause = 4;
        engine.objects[fighter_idx].frame_t_attach = CNAT_BOTTOM;
        engine.objects[fighter_idx]
            .set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));
        let definition_id = engine.objects[fighter_idx].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("fighter definition exists")
            .action_library()
            .clone();

        assert!(
            engine
                .exec_object_movement(fighter_idx, &actions, &definition_id, &[])
                .expect("ledge movement succeeds")
                .alive
        );
        let fighter_idx = engine.find_object_index(fighter).expect("fighter remains");
        assert_eq!(engine.objects[fighter_idx].state.action.name, "Jump");
        assert_eq!(
            engine.objects[fighter_idx].last_energy_loss_cause, 9,
            "NoAttachAction uses the fight target's Controller, not Owner"
        );

        // Falling out of the world calls AssignDeath(true) before removal
        // (C4Movement.cpp:613-614). Exercise that exact death half while the
        // corpse is still inspectable.
        engine
            .assign_death(fighter_idx, true)
            .expect("fall death assigns");
        let fighter_idx = engine.find_object_index(fighter).expect("corpse remains");
        assert_eq!(
            engine.objects[fighter_idx].state.local_vars.get("death_by"),
            Some(&Value::Int(9)),
            "the later fall death stays credited to the fight opponent"
        );
    }

    #[test]
    fn non_fight_no_attach_leaves_the_kill_trace_untouched() {
        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(no_attach_fighter_definition())
            .expect("fighter registers");
        let actor = engine
            .spawn_object(
                SpawnConfig::new("FGTR")
                    .with_category(CATEGORY_LIVING)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("walker spawns");
        let actor_idx = engine.find_object_index(actor).expect("walker exists");
        engine.objects[actor_idx].last_energy_loss_cause = 7;
        engine.objects[actor_idx].state.t_attach = CNAT_BOTTOM | CNAT_RIGHT;
        engine.objects[actor_idx].frame_t_attach = CNAT_BOTTOM | CNAT_RIGHT;
        engine.objects[actor_idx]
            .set_fixed_velocity(FixedVec2::new(itofix(-4), itofix(-5)));
        let definition_id = engine.objects[actor_idx].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("fighter definition exists")
            .action_library()
            .clone();

        engine
            .apply_no_attach_action(actor_idx, &definition_id, &actions, &[])
            .expect("no-attach transition succeeds");
        assert_eq!(engine.objects[actor_idx].state.action.name, "Jump");
        assert_eq!(
            engine.objects[actor_idx].fixed_velocity,
            FixedVec2::new(itofix(-4), itofix(-5))
        );
        assert!(engine.objects[actor_idx].state.mobile);
        assert_eq!(engine.objects[actor_idx].state.t_attach, CNAT_RIGHT);
        assert_eq!(engine.objects[actor_idx].frame_t_attach, CNAT_RIGHT);
        assert_eq!(
            engine.objects[actor_idx].last_energy_loss_cause, 7,
            "the NoAttachAction path only rewrites Fight kill tracing"
        );
    }

    #[test]
    fn push_no_attach_calls_grab_lost_before_jump_and_restores_push_to() {
        let (mut engine, pusher_id, vehicle_id) =
            grab_lost_push_fixture(Vector2::new(5, 5), Vector2::new(8, 5));
        let mut landscape = vehicle_grid_landscape(16, 16);
        landscape.set_world_height(16);
        landscape.grid_write_byte(5, 6, 1);
        engine.set_landscape(landscape);

        let pusher_idx = engine
            .find_object_index(pusher_id)
            .expect("pusher exists");
        engine.objects[pusher_idx].frame_t_attach = CNAT_BOTTOM;
        engine.objects[pusher_idx]
            .set_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO));
        let definition_id = engine.objects[pusher_idx].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("pusher definition exists")
            .action_library()
            .clone();

        assert!(
            engine
                .exec_object_movement(pusher_idx, &actions, &definition_id, &[])
                .expect("ledge movement succeeds")
                .alive
        );

        let pusher = engine.object_snapshot(pusher_id).expect("pusher remains");
        assert_eq!(pusher.position, Vector2::new(6, 5));
        assert_eq!(pusher.action.name, "Jump");
        assert_eq!(
            pusher.command_stack.command_names(),
            vec!["PushTo", "Wait"]
        );
        let vehicle = engine.object_snapshot(vehicle_id).expect("vehicle remains");
        assert_eq!(
            vehicle.local_vars.get("grab_lost_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            vehicle.local_vars.get("action_seen"),
            Some(&Value::String("Push".to_owned().into())),
            "GrabLost must run before the pusher's Jump transition"
        );
    }

    #[test]
    fn push_got_hold_loss_clears_delay_and_approach_above_push_to() {
        let (mut engine, pusher_id, vehicle_id) =
            grab_lost_push_fixture(Vector2::new(100, 5), Vector2::new(8, 5));
        let pusher_idx = engine
            .find_object_index(pusher_id)
            .expect("pusher exists");

        assert!(
            engine
                .apply_physics_at_index(pusher_idx)
                .expect("out-of-range push resolves")
        );

        let pusher = engine.object_snapshot(pusher_id).expect("pusher remains");
        assert_eq!(pusher.action.name, "Walk");
        assert_eq!(pusher.command_direction, CommandDirection::Stop);
        assert_eq!(
            pusher.command_stack.command_names(),
            vec!["PushTo", "Wait"],
            "GrabLost clears the new delay and MoveTo but preserves PushTo's tail"
        );
        let vehicle = engine.object_snapshot(vehicle_id).expect("vehicle remains");
        assert_eq!(
            vehicle.local_vars.get("grab_lost_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            vehicle.local_vars.get("action_seen"),
            Some(&Value::String("Walk".to_owned().into())),
            "StopActionDelayCommand precedes GrabLost"
        );
    }

    #[test]
    fn command_enter_push_target_replaces_vehicle_stack_without_controller_transfer() {
        // C4Command::Enter hands the pushed vehicle its own Enter command
        // once the vehicle (rather than the actor) reaches the entrance.
        // Unlike C4Object::Push, this SetCommand does not transfer Controller
        // (C4Command.cpp:577-597).
        let mut actor = Definition::from_script("ACTR", "Actor", "#strict")
            .expect("actor compiles");
        actor.configure_actions(
            Some("Push".to_string()),
            HashMap::from([(
                "Push".to_string(),
                ActionSpec::default().with_procedure("PUSH"),
            )]),
        );
        actor.set_physical(PhysicalInfo {
            walk: 35_000,
            push: 45_000,
            ..PhysicalInfo::default()
        });

        // Leave this event-handler fixture non-grabbable so the actor's
        // later same-frame PUSH physics cannot independently copy Controller.
        // The command-state regression above uses a normal Grab=1 vehicle.
        let vehicle = simple_definition("VEHI");

        let mut entrance = simple_definition("ENTR");
        entrance.set_shape_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
        entrance.set_entrance_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));

        let mut engine = Engine::with_seed(7);
        engine.register_definition(actor).expect("actor registers");
        engine
            .register_definition(vehicle)
            .expect("vehicle registers");
        engine
            .register_definition(entrance)
            .expect("entrance registers");

        // Loaded objects append to the execution list in this exact order.
        // The vehicle therefore consumes its old Wait before the actor
        // replaces that stack later in the same frame.
        let entrance_id = engine
            .spawn_object(
                SpawnConfig::new("ENTR")
                    .with_category(CATEGORY_STRUCTURE)
                    .with_position(Vector2::new(100, 100))
                    .with_entrance_status(true)
                    .with_loaded(true),
            )
            .expect("entrance spawns");
        let vehicle_id = engine
            .spawn_object(
                SpawnConfig::new("VEHI")
                    // Keep ExecAction from performing a real Push and
                    // independently transferring the actor's controller.
                    .with_category(CATEGORY_STATIC_BACK)
                    .with_position(Vector2::new(100, 100))
                    .with_controller(9)
                    .with_loaded(true),
            )
            .expect("vehicle spawns");
        let vehicle_index = engine
            .find_object_index(vehicle_id)
            .expect("vehicle exists");
        engine.objects[vehicle_index]
            .commands
            .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
            .expect("old Wait queues");

        let mut push = ActionState::new("Push");
        push.target = Some(vehicle_id);
        let actor_id = engine
            .spawn_object(
                SpawnConfig::new("ACTR")
                    .with_position(Vector2::new(0, 100))
                    .with_controller(7)
                    .with_action(push)
                    .with_loaded(true),
            )
            .expect("actor spawns");
        let actor_index = engine.find_object_index(actor_id).expect("actor exists");
        engine.objects[actor_index]
            .commands
            .push_back(
                CommandRequest::new(CommandId::Enter)
                    .with_target(Some(entrance_id))
                    .with_data(CommandData::Integer(2)),
            )
            .expect("actor Enter queues");

        engine.tick_without_snapshot().expect("actor hands Enter to vehicle");

        let actor = engine.object_snapshot(actor_id).expect("actor remains");
        assert!(
            !actor.command_stack.command_names().contains(&"Enter".to_string()),
            "actor Enter completes"
        );
        let vehicle = engine
            .object_snapshot(vehicle_id)
            .expect("vehicle remains");
        let commands = vehicle.command_stack.command_views();
        assert_eq!(commands.len(), 1, "SetCommand replaces the old Wait");
        assert_eq!(commands[0].name, "Enter");
        assert_eq!(commands[0].target, Some(entrance_id));
        assert_eq!(vehicle.controller, 9, "SetCommand preserves Controller");
        assert_eq!(vehicle.container, None, "vehicle executes on its next turn");

        engine.tick_without_snapshot().expect("vehicle executes its Enter");
        assert_eq!(
            engine
                .object_snapshot(vehicle_id)
                .expect("vehicle remains")
                .container,
            Some(entrance_id)
        );
    }

    fn push_pull_fixture() -> (Engine, ObjectId, ObjectId) {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut pusher_definition = Definition::from_script("Pusher", "Pusher", script).unwrap();
        let mut pusher_actions = HashMap::new();
        pusher_actions.insert(
            "Idle".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        pusher_actions.insert(
            "Push".to_string(),
            ActionSpec::default()
                .with_procedure("push")
                .with_delay(13)
                .with_length(20)
                .with_next("Push"),
        );
        pusher_actions.insert(
            "Pull".to_string(),
            ActionSpec::default()
                .with_procedure("pull")
                .with_delay(13)
                .with_length(20)
                .with_next("Pull"),
        );
        pusher_definition.configure_actions(Some("Idle".to_string()), pusher_actions);
        pusher_definition.set_physical(PhysicalInfo {
            walk: 35_000,
            push: 45_000,
            ..PhysicalInfo::default()
        });

        let mut crate_definition = Definition::from_script("Crate", "Crate", script).unwrap();
        crate_definition.set_grab(1);
        crate_definition.set_mass(200);

        let mut engine = Engine::with_seed(18);
        engine
            .register_definition(pusher_definition)
            .expect("pusher registers");
        engine
            .register_definition(crate_definition)
            .expect("crate registers");
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );

        let vertices = vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ];
        let crate_id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    // Pushables are vehicles: StaticBack never has OCF_Grab
                    // (SetOCF, C4Object.cpp:553-555).
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(10, 0))
                    .with_vertices(vertices.clone()),
            )
            .expect("crate spawns");
        (engine, crate_id, ObjectId::new(0))
    }

    fn stuck_push_fixture(
        command_direction: CommandDirection,
        horizontal_fix: i32,
    ) -> (Engine, ObjectId, ObjectId) {
        let mut pusher =
            Definition::from_script("PSHR", "Pusher", "#strict").expect("pusher compiles");
        pusher.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        pusher.set_physical(PhysicalInfo {
            walk: 35_000,
            push: 45_000,
            ..PhysicalInfo::default()
        });
        pusher.configure_actions(
            Some("Walk".to_owned()),
            HashMap::from([
                (
                    "Walk".to_owned(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Push".to_owned(),
                    ActionSpec::default()
                        .with_procedure("PUSH")
                        .with_delay(13)
                        .with_length(20)
                        .with_next("Push"),
                ),
            ]),
        );

        let vehicle_script = r#"#strict
local stuck_calls;
func Stuck()
{
    stuck_calls = stuck_calls + 1;
    return 0;
}
"#;
        let mut vehicle = Definition::from_script("VEHI", "Vehicle", vehicle_script)
            .expect("vehicle compiles");
        vehicle.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        vehicle.set_shape_vertices(vec![ObjectVertex::new(8, 0).with_cnat(CNAT_RIGHT)]);
        vehicle.set_contact_density(50);
        vehicle.set_grab(1);
        vehicle.set_mass(200);
        vehicle.set_no_horizontal_move(horizontal_fix);

        let mut landscape = vehicle_grid_landscape(32, 32);
        landscape.set_world_height(32);
        // VEHI is centered at (8,10), so its CNAT_Right vertex probes here.
        landscape.grid_write_byte(16, 10, 1);

        let mut engine = Engine::with_seed(18);
        engine.set_landscape(landscape);
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine.register_definition(pusher).expect("pusher registers");
        engine
            .register_definition(vehicle)
            .expect("vehicle registers");

        let vehicle_id = engine
            .spawn_object(
                SpawnConfig::new("VEHI")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(8, 10))
                    .with_fixed_position(FixedVec2::from_ints(8, 10))
                    .with_loaded(true),
            )
            .expect("vehicle spawns");
        let mut push = ActionState::new("Push");
        push.target = Some(vehicle_id);
        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("PSHR")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(8, 10))
                    .with_fixed_position(FixedVec2::from_ints(8, 10))
                    .with_action(push)
                    .with_command_direction(command_direction)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("pusher spawns");

        (engine, pusher_id, vehicle_id)
    }

    #[test]
    fn push_tick35_refreshes_contact_and_reports_stuck() {
        // C4Object::Push runs ContactCheck only on Tick35 while txdir is
        // nonzero. ContactCheck replaces t_contact, then the stuck message
        // and ~Stuck callback fire once (C4Object.cpp:1827-1832).
        let (mut engine, pusher_id, vehicle_id) =
            stuck_push_fixture(CommandDirection::Right, 0);
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        let vehicle_idx = engine
            .find_object_index(vehicle_id)
            .expect("vehicle exists");
        engine.objects[vehicle_idx].frame_t_contact = CNAT_LEFT;

        for frame in 1..=34 {
            engine.frame = frame;
            let _ = engine
                .apply_physics_at_index(pusher_idx)
                .expect("pre-Tick35 push succeeds");
        }
        assert_eq!(engine.objects[vehicle_idx].frame_t_contact, CNAT_LEFT);
        assert_eq!(
            engine.objects[vehicle_idx]
                .state
                .local_vars
                .get("stuck_calls"),
            None
        );
        assert!(engine.snapshot().hud.messages.is_empty());

        engine.frame = 35;
        let _ = engine
            .apply_physics_at_index(pusher_idx)
            .expect("Tick35 push succeeds");
        assert_eq!(
            engine.objects[vehicle_idx].frame_t_contact, CNAT_RIGHT,
            "ContactCheck replaces the previous t_contact latch"
        );
        assert_eq!(
            engine.objects[vehicle_idx]
                .state
                .local_vars
                .get("stuck_calls"),
            Some(&Value::Int(1))
        );
        let first_message = engine
            .snapshot()
            .hud
            .messages
            .into_iter()
            .next()
            .expect("Tick35 emits the stuck message");
        assert_eq!(first_message.kind, MessageKind::Target);
        assert_eq!(first_message.target, Some(vehicle_id));
        assert_eq!(first_message.player, None);
        assert_eq!(first_message.lines, vec!["Vehicle is stuck!"]);

        for frame in 36..=69 {
            engine.frame = frame;
            let _ = engine
                .apply_physics_at_index(pusher_idx)
                .expect("between-boundary push succeeds");
        }
        assert_eq!(
            engine.objects[vehicle_idx]
                .state
                .local_vars
                .get("stuck_calls"),
            Some(&Value::Int(1)),
            "no second callback before the next Tick35"
        );

        engine.frame = 70;
        let _ = engine
            .apply_physics_at_index(pusher_idx)
            .expect("second Tick35 push succeeds");
        assert_eq!(
            engine.objects[vehicle_idx]
                .state
                .local_vars
                .get("stuck_calls"),
            Some(&Value::Int(2)),
            "a continuously stuck push calls Stuck once per 35 frames"
        );
        let messages = engine.snapshot().hud.messages;
        assert_eq!(messages.len(), 1, "same-target messages replace each other");
        assert_ne!(
            messages[0].id, first_message.id,
            "the Tick70 check emitted a fresh stuck message"
        );
    }

    #[test]
    fn push_tick35_stuck_check_honors_stop_and_horizontal_fix() {
        // txdir==0 and Def->NoHorizontalMove both bypass ContactCheck,
        // leaving t_contact untouched and producing neither notification.
        for (label, command_direction, horizontal_fix) in [
            ("stopped", CommandDirection::Stop, 0),
            ("horizontal-fix", CommandDirection::Right, 1),
        ] {
            let (mut engine, pusher_id, vehicle_id) =
                stuck_push_fixture(command_direction, horizontal_fix);
            let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
            let vehicle_idx = engine
                .find_object_index(vehicle_id)
                .expect("vehicle exists");
            engine.objects[vehicle_idx].frame_t_contact = CNAT_LEFT;
            engine.frame = 35;

            let _ = engine
                .apply_physics_at_index(pusher_idx)
                .unwrap_or_else(|error| panic!("{label} push failed: {error}"));
            assert_eq!(
                engine.objects[vehicle_idx].frame_t_contact, CNAT_LEFT,
                "{label}: skipped ContactCheck leaves t_contact untouched"
            );
            assert_eq!(
                engine.objects[vehicle_idx]
                    .state
                    .local_vars
                    .get("stuck_calls"),
                None,
                "{label}: Stuck is not called"
            );
            assert!(
                engine.snapshot().hud.messages.is_empty(),
                "{label}: no stuck message"
            );
        }
    }

    #[test]
    fn push_procedure_uses_walk_and_push_physicals() {
        // DFA_PUSH (C4Object.cpp:5040-5097): the target is pushed via
        // C4Object::Push (C4Object.cpp:1758-1808) with
        // dforce = ValByPhysical(250, Push)*100/Mass, the pusher follows at
        // the full lLimit = ValByPhysical(280, Walk).
        let (mut engine, crate_id, _) = push_pull_fixture();
        let vertices = vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ];
        let mut push_state = ActionState::new("Push");
        push_state.target = Some(crate_id);
        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_position(Vector2::new(0, 0))
                    .with_controller(7)
                    .with_vertices(vertices)
                    .with_action(push_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("pusher spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        let crate_idx = engine.find_object_index(crate_id).expect("crate exists");
        // Target: Towards(0, +64225) by dforce = 73728*100/200 = raw 36864
        // (ValByPhysical(250, 45000) = itofix(45000*50, 2000000) = 73728).
        assert_eq!(engine.objects[crate_idx].fixed_velocity.x.val(), 36864);
        // Pusher: follow-x BoundBy(0, 2, 17) = 2 → xdir = +lLimit
        // = ValByPhysical(280, 35000) = raw 64225 (C4Object.cpp:5085-5087).
        assert_eq!(engine.objects[pusher_idx].fixed_velocity.x.val(), 64225);
        assert_eq!(engine.objects[pusher_idx].fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(engine.objects[pusher_idx].state.direction, Direction::Right);
        assert_eq!(engine.objects[pusher_idx].state.action.name, "Push");
        assert_eq!(
            (
                engine.objects[pusher_idx].state.action.phase,
                engine.objects[pusher_idx].state.action.ticks,
            ),
            (0, 10),
            "raw xdir 64225 advances PhaseDelay by fixtoi(|xdir|*10)=10"
        );
        assert_eq!(
            engine.objects[crate_idx].state.controller, 7,
            "a successful PUSH copies the pusher Controller before range checks"
        );
    }

    #[test]
    fn push_procedure_resets_without_grab_ocf() {
        // C4Object::Push refuses targets without OCF_Grab
        // (C4Object.cpp:1761) → StopActionDelayCommand.
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;
        let (mut engine, _, _) = push_pull_fixture();
        let plain = Definition::from_script("Rock", "Rock", script).unwrap();
        engine.register_definition(plain).expect("rock registers");
        let rock_id = engine
            .spawn_object(SpawnConfig::new("Rock").with_position(Vector2::new(10, 0)))
            .expect("rock spawns");
        let mut push_state = ActionState::new("Push");
        push_state.target = Some(rock_id);
        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_position(Vector2::new(0, 0))
                    .with_action(push_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("pusher spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        let rock_idx = engine.find_object_index(rock_id).expect("rock exists");
        assert_ne!(engine.objects[pusher_idx].state.action.name, "Push");
        assert_eq!(engine.objects[rock_idx].fixed_velocity.x, C4Fixed::ZERO);
    }

    #[test]
    fn pull_procedure_uses_walk_and_push_physicals() {
        // DFA_PULL (C4Object.cpp:5099-5170): puller right-pulls — target
        // force iTXDir = fMove + fWalk*BoundBy(iPullX-target.x,-10,10)/10,
        // own xdir from the pulling position, ComDir transfer onto walking
        // targets.
        let (mut engine, crate_id, _) = push_pull_fixture();
        // Reposition the crate for the pull geometry.
        engine
            .apply_object_update(
                crate_id,
                ObjectUpdate::new().with_position(Vector2::new(12, 0)),
            )
            .expect("crate moves");
        let vertices = vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ];
        let mut pull_state = ActionState::new("Pull");
        pull_state.target = Some(crate_id);
        let puller_id = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_position(Vector2::new(0, 0))
                    .with_controller(9)
                    .with_vertices(vertices)
                    .with_action(pull_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("puller spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let puller_idx = engine.find_object_index(puller_id).expect("puller exists");
        let crate_idx = engine.find_object_index(crate_id).expect("crate exists");
        // iPullDistance = 8+8; iPullX = 0-16 = -16; iTXDir = 64225 +
        // 64225*BoundBy(-28,-10,10)/10 = 0 → target velocity stays zero.
        assert_eq!(engine.objects[crate_idx].fixed_velocity.x, C4Fixed::ZERO);
        // Own move: iTargetX = 12+16 = 28; xdir = 64225 +
        // 64225*BoundBy(28,-10,10)/10 = raw 128450 (C4Object.cpp:5164).
        assert_eq!(engine.objects[puller_idx].fixed_velocity.x.val(), 128450);
        assert_eq!(engine.objects[puller_idx].fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(engine.objects[puller_idx].state.direction, Direction::Right);
        assert_eq!(engine.objects[puller_idx].state.action.name, "Pull");
        assert_eq!(
            (
                engine.objects[puller_idx].state.action.phase,
                engine.objects[puller_idx].state.action.ticks,
            ),
            (1, 0),
            "raw xdir 128450 advances by 20, crosses Delay 13 once, and discards overshoot"
        );
        assert_eq!(
            engine.objects[crate_idx].state.controller, 9,
            "a successful PULL copies the puller Controller before range checks"
        );
    }

    #[test]
    fn stationary_push_retains_one_phase_step_but_pull_freezes() {
        // ExecAction starts with iPhaseAdvance=1. DFA_PUSH overwrites it
        // only for a nonzero raw xdir; DFA_PULL first resets it to zero
        // (C4Object.cpp:5106-5108,5189-5192).
        for (action, stationary_advance) in [("Push", 1_i32), ("Pull", 0_i32)] {
            let (mut engine, crate_id, _) = push_pull_fixture();
            let mut state = ActionState::new(action);
            state.target = Some(crate_id);
            let actor = engine
                .spawn_object(
                    SpawnConfig::new("Pusher")
                        // The zero-width synthetic target's inclusive right
                        // edge is x=9, so this is the exact follow position.
                        .with_position(Vector2::new(9, 0))
                        .with_action(state)
                        .with_command_direction(CommandDirection::Stop),
                )
                .expect("actor spawns");

            for frame in 1_u32..=20 {
                engine.tick_without_snapshot().expect("stationary procedure tick succeeds");
                let index = engine.find_object_index(actor).expect("actor survives");
                let object = &engine.objects[index];
                assert_eq!(
                    object.fixed_velocity.x,
                    C4Fixed::ZERO,
                    "{action} is stationary"
                );
                assert_eq!(object.state.action.name, action);

                let accumulated = frame as i32 * stationary_advance;
                let expected_phase = accumulated / 13;
                let expected_delay = accumulated % 13;
                assert_eq!(
                    (object.state.action.phase, object.state.action.ticks),
                    (expected_phase, expected_delay),
                    "{action} phase state after frame {frame}"
                );
            }
        }
    }

    #[test]
    fn pinned_push_phase_trajectory_matches_cpp_for_one_hundred_frames() {
        // With the fixture's raw follow xdir 64225, PUSH contributes 10 to
        // PhaseDelay each successful frame. Delay 13 therefore advances one
        // phase every second frame, discarding the seven-point overshoot;
        // Length 20 wraps the same-name action every 40 frames.
        let (mut engine, crate_id, _) = push_pull_fixture();
        let mut push = ActionState::new("Push");
        push.target = Some(crate_id);
        let pusher = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_position(Vector2::ZERO)
                    .with_action(push)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("pusher spawns");

        for frame in 1_u32..=100 {
            let pusher_index = engine.find_object_index(pusher).expect("pusher survives");
            engine.objects[pusher_index].set_position(Vector2::ZERO);
            engine.objects[pusher_index].set_fixed_velocity(FixedVec2::ZERO);
            let crate_index = engine.find_object_index(crate_id).expect("crate survives");
            engine.objects[crate_index].set_position(Vector2::new(10, 0));
            engine.objects[crate_index].set_fixed_velocity(FixedVec2::ZERO);

            engine.tick_without_snapshot().expect("pinned push tick succeeds");
            let pusher_index = engine.find_object_index(pusher).expect("pusher survives");
            let object = &engine.objects[pusher_index];
            assert_eq!(object.fixed_velocity.x.val(), 64225);
            assert_eq!(
                (object.state.action.phase, object.state.action.ticks),
                (
                    ((frame / 2) % 20) as i32,
                    if frame % 2 == 0 { 0 } else { 10 },
                ),
                "PUSH phase trajectory at frame {frame}"
            );
        }
    }

    #[test]
    fn pinned_horse_pull_wraps_and_refires_start_call_after_twenty_frames() {
        // Western Horse Pull: Delay=13, Length=20, NextAction=Pull,
        // StartCall=Pulling. At the pinned full pull force, raw xdir 183500
        // gives advance 28, so C++ advances exactly one phase per frame and
        // the same-action SetAction refires Pulling on frame 20.
        use std::sync::{Arc, Mutex};

        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let calls = Arc::clone(&calls);
            hooks.set_on_call(move |name, _args| {
                if name == "Pulling" {
                    calls.lock().unwrap().push(name.to_string());
                }
            });
        }

        let mut horse = Definition::from_script(
            "HRSE",
            "Horse",
            "#strict 2\nfunc Pulling() { return 1; }\n",
        )
        .expect("horse compiles");
        horse.set_debugger_hooks(hooks);
        horse.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        horse.set_physical(PhysicalInfo {
            walk: 50_000,
            push: 100_000,
            ..PhysicalInfo::default()
        });
        horse.configure_actions(
            Some("Pull".to_string()),
            HashMap::from([(
                "Pull".to_string(),
                ActionSpec::default()
                    .with_procedure("pull")
                    .with_delay(13)
                    .with_length(20)
                    .with_next("Pull")
                    .with_start_call("Pulling"),
            )]),
        );

        let mut wagon = simple_definition("WAGN");
        wagon.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        wagon.set_category(CATEGORY_VEHICLE);
        wagon.set_grab(1);
        wagon.set_mass(200);

        let mut engine = Engine::with_seed(0);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(horse).expect("horse registers");
        engine.register_definition(wagon).expect("wagon registers");
        let wagon = engine
            .spawn_object(SpawnConfig::new("WAGN").with_position(Vector2::new(12, 0)))
            .expect("wagon spawns");
        let mut pull = ActionState::new("Pull");
        pull.target = Some(wagon);
        let horse = engine
            .spawn_object(
                SpawnConfig::new("HRSE")
                    .with_position(Vector2::ZERO)
                    .with_action(pull)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("horse spawns");
        calls.lock().unwrap().clear();

        for frame in 1_i32..=20 {
            let horse_index = engine.find_object_index(horse).expect("horse survives");
            engine.objects[horse_index].set_position(Vector2::ZERO);
            engine.objects[horse_index].set_fixed_velocity(FixedVec2::ZERO);
            let wagon_index = engine.find_object_index(wagon).expect("wagon survives");
            engine.objects[wagon_index].set_position(Vector2::new(12, 0));
            engine.objects[wagon_index].set_fixed_velocity(FixedVec2::ZERO);

            engine.tick_without_snapshot().expect("pinned pull tick succeeds");
            let horse_index = engine.find_object_index(horse).expect("horse survives");
            let action = &engine.objects[horse_index].state.action;
            assert_eq!(engine.objects[horse_index].fixed_velocity.x.val(), 183500);
            assert_eq!(action.phase, frame % 20, "PULL phase at frame {frame}");
            assert_eq!(action.ticks, 0, "advance 28 crosses Delay 13 each frame");
            assert_eq!(
                calls.lock().unwrap().len(),
                usize::from(frame == 20),
                "Pulling StartCall count at frame {frame}"
            );
        }
    }

    fn train_physical_crew_fixture(use_fair_crew: bool) -> (Engine, ObjectId, usize) {
        let mut definition = Definition::from_script(
            "TRNR",
            "Training crew",
            r#"#strict
public func TrainScale()
{
    return([TrainPhysical("Scale", 5, 100000), GetPhysical("Scale"), GetPhysical("Scale", 1)]);
}

public func TrainTemporaryScale()
{
    SetPhysical("Scale", 50000, 3);
    return([TrainPhysical("Scale", 5, 100000), GetPhysical("Scale"), GetPhysical("Scale", 2)]);
}
"#,
        )
        .expect("training crew script compiles");
        definition.set_crew_member(true);
        definition.set_physical(PhysicalInfo {
            scale: 30_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("training crew definition registers");
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("TRNR".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine.set_use_fair_crew(use_fair_crew);
        engine
            .join_player(JoinPlayerConfig {
                name: "Training owner".to_string(),
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
                    id: "TRNR".to_string(),
                    name: "Trainee".to_string(),
                    death_message: String::new(),
                    core: Default::default(),
                    rank: 0,
                    rank_name: "Clonk".to_string(),
                    experience: 0,
                    rounds: 0,
                    physical: PhysicalInfo {
                        scale: 80_000,
                        ..PhysicalInfo::default()
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
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("training player joins");
        let crew = engine.player(0).expect("training player exists").crew()[0];
        let crew_index = engine
            .find_object_index(crew)
            .expect("training crew exists");
        (engine, crew, crew_index)
    }

    fn persisted_crew_scale(engine: &Engine, crew: ObjectId) -> i32 {
        let state = engine.capture_state();
        let link = state
            .crew_info_links
            .get(&crew)
            .expect("crew retains its exact roster link");
        state
            .crew_info_rosters
            .get(&link.player_id)
            .and_then(|roster| roster.get(link.roster_index))
            .expect("linked persistent crew info exists")
            .physical
            .scale
    }

    #[test]
    fn train_physical_under_fair_crew_persists_without_changing_live_physicals() {
        let (mut engine, crew, crew_index) = train_physical_crew_fixture(true);
        assert_eq!(engine.object_physical(crew_index).scale, 33_500);

        assert_eq!(
            engine
                .call_object_function(crew_index, "TrainScale", Vec::new())
                .expect("script trains the raw info physical"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(33_500),
                Value::Int(33_500),
            ]),
            "both current and permanent reads stay on the fair projection"
        );
        assert_eq!(engine.object_physical(crew_index).scale, 33_500);
        assert_eq!(
            engine.objects[crew_index]
                .state
                .info_physical
                .expect("crew carries raw info physicals")
                .scale,
            80_005
        );
        assert_eq!(persisted_crew_scale(&engine, crew), 80_005);

        engine.set_use_fair_crew(false);
        assert_eq!(
            engine.object_physical(crew_index).scale,
            80_005,
            "disabling fair crew reveals the training instead of losing it"
        );
    }

    #[test]
    fn train_physical_trains_temporary_and_stacked_values_under_fair_crew() {
        let (mut engine, crew, crew_index) = train_physical_crew_fixture(true);

        assert_eq!(
            engine
                .call_object_function(crew_index, "TrainTemporaryScale", Vec::new())
                .expect("script trains temporary and raw physicals"),
            Value::Array(vec![
                Value::Bool(true),
                Value::Int(50_005),
                Value::Int(50_005),
            ])
        );
        assert_eq!(engine.object_physical(crew_index).scale, 50_005);
        assert_eq!(
            engine.objects[crew_index]
                .state
                .temporary_physical
                .expect("temporary mode stays active")
                .scale,
            50_005
        );
        assert_eq!(
            engine.objects[crew_index].state.physical_changes,
            vec![("Scale".to_string(), 33_505)],
            "TrainPhysical also trains every stacked previous value"
        );
        assert_eq!(
            engine.objects[crew_index]
                .state
                .info_physical
                .expect("raw info is trained alongside temporary state")
                .scale,
            80_005
        );
        assert_eq!(persisted_crew_scale(&engine, crew), 80_005);
    }

    #[test]
    fn train_physical_is_live_and_persistent_when_fair_crew_is_off() {
        let (mut engine, crew, crew_index) = train_physical_crew_fixture(false);

        assert_eq!(engine.object_physical(crew_index).scale, 80_000);
        assert!(engine.train_physical(crew_index, "Scale", 5, C4_MAX_PHYSICAL));
        assert_eq!(engine.object_physical(crew_index).scale, 80_005);
        assert_eq!(persisted_crew_scale(&engine, crew), 80_005);
    }

    #[test]
    fn train_physical_requires_info_or_temporary_physicals() {
        // C4Object::TrainPhysical (C4Object.cpp:2136-2146) trains the
        // temporary set when active and the info physicals when the object
        // carries a C4ObjectInfo — an object with neither trains NOTHING.
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let definition = Definition::from_script("Sheep", "Sheep", script).unwrap();
        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let sheep_id = engine
            .spawn_object(SpawnConfig::new("Sheep"))
            .expect("sheep spawns");
        let sheep_idx = engine.find_object_index(sheep_id).expect("sheep exists");
        assert!(
            !engine.train_physical(sheep_idx, "Fight", 1, C4_MAX_PHYSICAL),
            "non-crew object without temporary physicals trains nothing"
        );
        assert_eq!(engine.objects[sheep_idx].state.info_physical, None);
        assert_eq!(engine.objects[sheep_idx].state.temporary_physical, None);

        // Crew membership alone is not C4Object::Info. Script-created crew
        // can exist without a persistent object-info pointer.
        let crew_id = engine
            .spawn_object(SpawnConfig::new("Sheep").with_crew_member(true))
            .expect("crew spawns");
        let crew_idx = engine.find_object_index(crew_id).expect("crew exists");
        assert!(!engine.train_physical(crew_idx, "Fight", 1, C4_MAX_PHYSICAL));
        assert_eq!(engine.objects[crew_idx].state.info_physical, None);

        // A temporary set is independently trainable even without Info.
        engine.objects[crew_idx].state.temporary_physical = Some(PhysicalInfo {
            fight: 100,
            ..PhysicalInfo::default()
        });
        assert!(engine.train_physical(crew_idx, "Fight", 1, C4_MAX_PHYSICAL));
        assert_eq!(
            engine.objects[crew_idx]
                .state
                .temporary_physical
                .expect("temporary set remains active")
                .fight,
            101
        );
        assert_eq!(engine.objects[crew_idx].state.info_physical, None);
    }

    #[test]
    fn object_physical_uses_promoted_info_when_fair_crew_is_off() {
        // C4Object::GetPhysical returns Info->Physical when the round's
        // UseFairCrew flag is false (C4Object.cpp:2149-2152). PromotionUpdate
        // preserves trained Walk values that came from the player info.
        let mut definition = simple_definition("CLNK");
        definition.set_crew_member(true);
        definition.set_physical(PhysicalInfo {
            walk: 30_000,
            ..PhysicalInfo::default()
        });
        let mut engine = Engine::new();
        engine
            .register_definition(definition)
            .expect("crew definition registers");
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CLNK".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine.set_use_fair_crew(false);
        engine
            .join_player(JoinPlayerConfig {
                name: "Info owner".to_string(),
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
                    id: "CLNK".to_string(),
                    name: "Trained".to_string(),
                    death_message: String::new(),
                    core: Default::default(),
                    rank: 2,
                    rank_name: "Lieutenant".to_string(),
                    experience: 3_000,
                    rounds: 0,
                    physical: PhysicalInfo {
                        energy: 60_000,
                        walk: 80_000,
                        can_scale: 1,
                        can_hangle: 1,
                        can_dig: 1,
                        can_construct: 1,
                        can_chop: 1,
                        ..PhysicalInfo::default()
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
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("player joins");
        let crew = engine.player(0).expect("player exists").crew()[0];
        let crew_index = engine.find_object_index(crew).expect("crew exists");
        assert_eq!(engine.fair_crew_strength(), 1_000);
        assert!(!engine.use_fair_crew());

        assert_eq!(engine.object_physical(crew_index).walk, 80_000);
    }

    #[test]
    fn object_physical_trains_fair_crew_from_live_strength() {
        // FairCrewStrength=5000 maps to rank 2 with the default rank curve
        // (1000, 2828, 5196). PromotionUpdate moves the four trainable
        // physicals 2/20 of the way from the definition to C4MaxPhysical
        // (C4Def.cpp:860-874; C4InfoCore.cpp:214-219).
        let mut definition = Definition::from_script(
            "OLDP",
            "Original crew",
            "#strict\npublic func ReadFair() { return([GetPhysical(\"Scale\"), GetPhysical(\"Scale\", 1)]); }\npublic func Swap() { return(ChangeDef(NEWP)); }\n",
        )
        .expect("original crew compiles");
        definition.set_crew_member(true);
        definition.set_physical(PhysicalInfo {
            scale: 30_000,
            hangle: 40_000,
            swim: 50_000,
            fight: 60_000,
            ..PhysicalInfo::default()
        });
        let mut engine = Engine::new();
        engine.set_fair_crew_strength(5_000);
        engine
            .register_definition(definition)
            .expect("crew definition registers");
        let mut changed = Definition::from_script(
            "NEWP",
            "Changed crew",
            "#strict\npublic func ReadFair() { return([GetPhysical(\"Scale\"), GetPhysical(\"Scale\", 1)]); }\n",
        )
        .expect("changed crew compiles");
        changed.set_physical(PhysicalInfo {
            scale: 90_000,
            hangle: 90_000,
            swim: 90_000,
            fight: 90_000,
            ..PhysicalInfo::default()
        });
        engine
            .register_definition(changed)
            .expect("changed definition registers");
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("OLDP".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(JoinPlayerConfig {
                name: "Fair owner".to_string(),
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
                    id: "OLDP".to_string(),
                    name: "Fair".to_string(),
                    death_message: String::new(),
                    core: Default::default(),
                    rank: 0,
                    rank_name: "Clonk".to_string(),
                    experience: 0,
                    rounds: 0,
                    physical: PhysicalInfo {
                        scale: 99_000,
                        hangle: 99_000,
                        swim: 99_000,
                        fight: 99_000,
                        ..PhysicalInfo::default()
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
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("player joins");
        let crew = engine.player(0).expect("player exists").crew()[0];
        let crew_index = engine.find_object_index(crew).expect("crew exists");

        let physical = engine.object_physical(crew_index);
        assert_eq!(physical.scale, 37_000);
        assert_eq!(physical.hangle, 46_000);
        assert_eq!(physical.swim, 55_000);
        assert_eq!(physical.fight, 64_000);
        assert_eq!(
            engine
                .call_object_function(crew_index, "ReadFair", Vec::new())
                .expect("script reads fair physicals"),
            Value::Array(vec![Value::Int(37_000), Value::Int(37_000)])
        );

        engine
            .call_object_function(crew_index, "Swap", Vec::new())
            .expect("ChangeDef succeeds");
        let crew_index = engine.find_object_index(crew).expect("crew survives");
        assert_eq!(engine.objects[crew_index].definition_id, "NEWP");
        assert_eq!(
            engine.object_physical(crew_index),
            physical,
            "fair crew remains sourced from Info->pDef after ChangeDef"
        );
        assert_eq!(
            engine
                .call_object_function(crew_index, "ReadFair", Vec::new())
                .expect("changed object script reads fair physicals"),
            Value::Array(vec![Value::Int(37_000), Value::Int(37_000)])
        );
    }

    #[test]
    fn fair_crew_projection_is_cached_per_definition_until_synchronize() {
        let mut definition = Definition::from_script(
            "FCCH",
            "Fair crew cache probe",
            r#"#strict
static hook_calls, order_errors;
static probe_armed, probe_target, probe_owner, probe_reentrant, probe_nested, probe_definition;

protected func GetFairCrewPhysical(string name, int rank, &value)
{
    order_errors += 0;
    var names = ["Energy", "Breath", "Walk", "Jump", "Scale", "Hangle", "Dig",
                 "Swim", "Throw", "Push", "Fight", "Magic", "Float", "CanScale",
                 "CanHangle", "CanDig", "CanConstruct", "CanChop", "CanFly",
                 "CorrosionResist", "BreatheWater"];
    if (name ne names[hook_calls % 21]) order_errors += 1;
    hook_calls += 1;
    if (name eq "Energy" && probe_armed)
    {
        probe_armed = false;
        SetWealth(0, 1234);
        probe_owner = GetOwner();
        probe_reentrant = GetPhysical("Energy", 0, probe_target);
        probe_nested = probe_target->ReadImplicit();
        probe_definition = DefinitionCall(OTHR, "Probe");
    }
    value = Random(1000000);
    return true;
}

public func ArmProbe() { probe_target = this(); probe_armed = true; return true; }
public func HookState() { return [hook_calls, order_errors]; }
public func ProbeState() { return [probe_owner, probe_reentrant, probe_nested, probe_definition]; }
public func ReadEnergy() { return GetPhysical("Energy"); }
public func ReadImplicit() { return [GetID(), GetPhysical("Energy")]; }
public func FailMagicUnderload() { return DoMagicEnergy(-1, this(), false); }
"#,
        )
        .expect("fair-crew cache probe compiles");
        definition.set_crew_member(true);

        let crew_info = |name: &str| player_file::CrewInfo {
            id: "FCCH".to_string(),
            name: name.to_string(),
            death_message: String::new(),
            core: Default::default(),
            rank: 0,
            rank_name: "Clonk".to_string(),
            experience: 0,
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

        let mut engine = Engine::with_seed(0x148);
        engine.set_use_fair_crew(true);
        engine
            .register_definition(
                Definition::from_script(
                    "OTHR",
                    "Nested definition probe",
                    "#strict\npublic func Probe() { return GetID(); }\n",
                )
                .expect("nested definition probe compiles"),
            )
            .expect("nested definition probe registers");
        engine
            .register_definition(definition)
            .expect("fair-crew cache probe registers");
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("FCCH".to_string(), 2)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(JoinPlayerConfig {
                name: "Cache owner".to_string(),
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
                crew: vec![crew_info("First"), crew_info("Second")],
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("cache owner joins");

        let crew = engine
            .player(0)
            .expect("cache owner exists")
            .crew()
            .to_vec();
        assert_eq!(crew.len(), 2);
        let first_index = engine
            .find_object_index(crew[0])
            .expect("first crew exists");
        let second_index = engine
            .find_object_index(crew[1])
            .expect("second crew exists");
        let rng_after_join = engine.rng.clone();
        assert_eq!(
            engine
                .call_object_function(first_index, "HookState", Vec::new())
                .expect("hook state reads"),
            Value::Array(vec![Value::Int(21), Value::Int(0)]),
            "both crew share one definition cache fill in native field order"
        );

        let initial = engine.object_physical(first_index);
        assert_eq!(engine.object_physical(second_index), initial);
        assert_eq!(engine.object_physical(first_index), initial);
        assert_eq!(
            engine
                .call_object_function(second_index, "ReadEnergy", Vec::new())
                .expect("script reads the shared projection"),
            Value::Int(initial.energy)
        );
        assert_eq!(
            engine.rng, rng_after_join,
            "repeated reads and a second same-definition crew consume no Random calls"
        );

        engine
            .execute_synchronize_control(false, false)
            .expect("game synchronization succeeds");
        let rng_after_synchronize = engine.rng.clone();
        assert_eq!(
            engine
                .call_object_function(first_index, "HookState", Vec::new())
                .expect("post-sync hook state reads"),
            Value::Array(vec![Value::Int(21), Value::Int(0)]),
            "synchronization clears the cache without eagerly refilling it"
        );
        assert_eq!(engine.rng, rng_after_synchronize);
        assert_eq!(
            engine
                .call_object_function(first_index, "FailMagicUnderload", Vec::new())
                .expect("failed magic underload returns"),
            Value::Bool(false)
        );
        assert_eq!(
            engine
                .call_object_function(first_index, "HookState", Vec::new())
                .expect("failed underload leaves hook state readable"),
            Value::Array(vec![Value::Int(21), Value::Int(0)]),
            "a native early-return branch must not eagerly fill the cache"
        );
        assert_eq!(engine.rng, rng_after_synchronize);
        engine
            .tick_without_snapshot()
            .expect("idle post-sync frame executes");
        assert_eq!(
            engine
                .call_object_function(first_index, "HookState", Vec::new())
                .expect("idle frame leaves hook state readable"),
            Value::Array(vec![Value::Int(21), Value::Int(0)]),
            "idle action and structural command snapshots do not refill unrelated physicals"
        );
        assert_eq!(engine.rng, rng_after_synchronize);

        let mut expected_rng = rng_after_synchronize;
        let expected = PhysicalInfo {
            energy: expected_rng.random(1_000_000),
            breath: expected_rng.random(1_000_000),
            walk: expected_rng.random(1_000_000),
            jump: expected_rng.random(1_000_000),
            scale: expected_rng.random(1_000_000),
            hangle: expected_rng.random(1_000_000),
            dig: expected_rng.random(1_000_000),
            swim: expected_rng.random(1_000_000),
            throw: expected_rng.random(1_000_000),
            push: expected_rng.random(1_000_000),
            fight: expected_rng.random(1_000_000),
            magic: expected_rng.random(1_000_000),
            float: expected_rng.random(1_000_000),
            can_scale: expected_rng.random(1_000_000),
            can_hangle: expected_rng.random(1_000_000),
            can_dig: expected_rng.random(1_000_000),
            can_construct: expected_rng.random(1_000_000),
            can_chop: expected_rng.random(1_000_000),
            can_fly: expected_rng.random(1_000_000),
            corrosion_resist: expected_rng.random(1_000_000),
            breathe_water: expected_rng.random(1_000_000),
        };
        assert_eq!(
            engine
                .call_object_function(first_index, "ArmProbe", Vec::new())
                .expect("post-sync compat-path probe arms"),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .call_object_function(first_index, "ReadEnergy", Vec::new())
                .expect("script triggers the first post-sync cache fill"),
            Value::Int(expected.energy)
        );
        assert_eq!(
            engine.rng, expected_rng,
            "the lazy refill invokes exactly 21 ordered Random hooks"
        );
        assert_eq!(
            engine.player(0).expect("cache owner remains live").wealth(),
            1_234,
            "a mutable host call inside the definition-only hook persists"
        );
        assert_eq!(
            engine
                .call_object_function(first_index, "ProbeState", Vec::new())
                .expect("definition-only hook probe state reads"),
            Value::Array(vec![
                Value::Int(OWNER_NONE),
                Value::Int(55_000),
                Value::Array(vec![Value::C4Id("FCCH".to_string()), Value::Int(55_000)]),
                Value::C4Id("OTHR".to_string()),
            ]),
            "the definition hook has no implicit object, while nested object and other-definition frames restore their own GetID/GetPhysical context"
        );
        assert_eq!(engine.object_physical(first_index), expected);
        let rng_after_refill = engine.rng.clone();
        assert_eq!(engine.object_physical(second_index), expected);
        assert_eq!(engine.object_physical(first_index), expected);
        assert_eq!(
            engine
                .call_object_function(second_index, "ReadEnergy", Vec::new())
                .expect("script reuses the refreshed projection"),
            Value::Int(expected.energy)
        );
        assert_eq!(engine.rng, rng_after_refill);
        assert_eq!(
            engine
                .call_object_function(first_index, "HookState", Vec::new())
                .expect("refill hook state reads"),
            Value::Array(vec![Value::Int(42), Value::Int(0)])
        );
    }

    #[test]
    fn fair_crew_uses_custom_rank_base_and_definition_script_override() {
        let mut definition = Definition::from_script(
            "RANK",
            "Ranked crew",
            r#"#strict
protected func GetFairCrewPhysical(string name, int rank, &value)
{
    if (name eq "Magic")
    {
        value = GetPhysical("Magic", 0, 0, GetID()) + rank * 1000 + 7;
        return true;
    }
    if (name eq "Breath")
    {
        if (GetPhysical("Magic")) value = 999;
        else value = 123;
        return true;
    }
    value = 777777;
    return false;
}
public func ReadFair() { return [GetPhysical("Magic"), GetPhysical("Energy"), GetPhysical("Breath")]; }
"#,
        )
        .expect("ranked crew compiles");
        definition.set_crew_member(true);
        definition.set_rank_system(Some(vec!["Recruit".to_string()]), Some(500));
        definition.set_physical(PhysicalInfo {
            magic: 45_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::new();
        engine.set_use_fair_crew(true);
        engine.set_fair_crew_strength(500);
        engine
            .register_definition(definition)
            .expect("ranked definition registers");
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("RANK".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(JoinPlayerConfig {
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
                crew: vec![player_file::CrewInfo {
                    id: "RANK".to_string(),
                    name: "Ranked".to_string(),
                    death_message: String::new(),
                    core: Default::default(),
                    rank: 0,
                    rank_name: "Recruit".to_string(),
                    experience: 0,
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
                }],
                startup_player_count: 1,
                control_style: false,
                auto_context_menu: false,
            })
            .expect("player joins");
        let crew = engine.player(0).expect("player exists").crew()[0];
        let crew_index = engine.find_object_index(crew).expect("crew exists");

        let base_boundary = engine.object_physical(crew_index);
        assert_eq!(base_boundary.magic, 46_007);
        assert_eq!(base_boundary.energy, 55_000);

        // Plain parameter mutation does not invalidate C4Def's projection;
        // only C4DefList::Synchronize or the runtime FairCrew control does.
        engine.set_fair_crew_strength(1_000);
        let physical = engine.object_physical(crew_index);
        assert_eq!(physical.magic, 46_007);
        assert_eq!(physical.energy, 55_000);
        assert_eq!(physical.breath, 123);
        assert_eq!(
            engine
                .call_object_function(crew_index, "ReadFair", Vec::new())
                .expect("script reads custom fair physicals"),
            Value::Array(vec![
                Value::Int(46_007),
                Value::Int(55_000),
                Value::Int(123),
            ])
        );
    }

    #[test]
    fn engine_state_retains_forced_control_style_for_later_joins() {
        // Savegame restoration preserves C4S.Head, which remains the source
        // for C4Player::ApplyForcedControl on runtime joins
        // (C4Game.cpp:4234; C4Player.cpp:2369-2389).
        let mut state = Engine::new().capture_state();
        state.forced_control_style = Some(true);

        let encoded = state.to_json_string().expect("state serializes");
        let decoded = EngineState::from_json_str(&encoded).expect("state deserializes");
        let mut restored = Engine::new();
        restored.restore_state(&decoded).expect("state restores");

        assert_eq!(restored.capture_state().forced_control_style, Some(true));
    }

    #[test]
    fn engine_state_retains_fair_crew_parameters_with_legacy_defaults() {
        let mut engine = Engine::new();
        engine.set_use_fair_crew(false);
        engine.set_fair_crew_strength(5_000);

        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("state serializes");
        let decoded = EngineState::from_json_str(&encoded).expect("state deserializes");
        let mut restored = Engine::new();
        restored.restore_state(&decoded).expect("state restores");
        assert!(!restored.use_fair_crew());
        assert_eq!(restored.fair_crew_strength(), 5_000);

        let mut legacy: serde_json::Value =
            serde_json::from_str(&encoded).expect("state JSON parses");
        let object = legacy.as_object_mut().expect("state is an object");
        object.remove("use_fair_crew");
        object.remove("fair_crew_strength");
        let legacy = EngineState::from_json_str(&legacy.to_string())
            .expect("legacy state without fair-crew fields deserializes");
        let mut restored = Engine::new();
        restored.restore_state(&legacy).expect("legacy state restores");
        assert!(restored.use_fair_crew());
        assert_eq!(restored.fair_crew_strength(), 1_000);
    }

    #[test]
    fn engine_state_retains_forced_auto_context_menu_for_later_joins() {
        // Savegame restoration preserves C4S.Head, which remains the source
        // for both preferences in C4Player::ApplyForcedControl
        // (C4Player.cpp:2369-2375).
        let mut state = Engine::new().capture_state();
        state.forced_auto_context_menu = Some(true);

        let encoded = state.to_json_string().expect("state serializes");
        let decoded = EngineState::from_json_str(&encoded).expect("state deserializes");
        let mut restored = Engine::new();
        restored.restore_state(&decoded).expect("state restores");

        assert_eq!(
            restored.capture_state().forced_auto_context_menu,
            Some(true)
        );
    }

    #[test]
    fn forced_control_style_overrides_player_preference_both_ways() {
        // C4Player::ApplyForcedControl chooses C4S.Head.ForcedControlStyle
        // whenever it is non-negative, regardless of PrefControlStyle
        // (C4Player.cpp:2369-2374).
        for (forced, preference) in [(true, false), (false, true)] {
            let mut engine = Engine::new();
            engine.set_forced_control_style(Some(forced));
            let joined = engine
                .join_player(JoinPlayerConfig {
                    name: "Tester".into(),
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
                    control_style: preference,
                    auto_context_menu: preference,
                    startup_player_count: 1,
                })
                .expect("player joins");
            assert_eq!(
                engine
                    .player(joined.number())
                    .expect("joined player")
                    .control_style(),
                forced
            );
        }
    }

    #[test]
    fn forced_auto_context_menu_overrides_player_preference_both_ways() {
        // C4Player::ApplyForcedControl chooses
        // C4S.Head.ForcedAutoContextMenu whenever it is non-negative,
        // otherwise it keeps PrefAutoContextMenu (C4Player.cpp:2369-2375).
        for (forced, preference) in [(true, false), (false, true)] {
            let mut engine = Engine::new();
            engine.set_forced_auto_context_menu(Some(forced));
            let joined = engine
                .join_player(JoinPlayerConfig {
                    name: "Tester".into(),
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
                    control_style: preference,
                    auto_context_menu: preference,
                    startup_player_count: 1,
                })
                .expect("player joins");
            assert_eq!(
                engine
                    .player(joined.number())
                    .expect("joined player")
                    .control
                    .auto_context_menu,
                forced
            );
        }
    }

    #[test]
    fn state_round_trip_preserves_physicals_and_energy_loss_cause() {
        // C++ persists with the object: LastEngLossPlr (C4Object.cpp:2740)
        // and the temporary physicals with their stacked changes
        // (C4Object.cpp:2777,2798-2801; C4InfoCore.cpp:306); info training
        // rides on the object until the C4ObjectInfo model lands.
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let definition = Definition::from_script("Clonk", "Clonk", script).unwrap();
        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Clonk").with_energy(40))
            .expect("clonk spawns");
        let idx = engine.find_object_index(id).expect("clonk exists");
        engine.objects[idx].state.info_physical = Some(PhysicalInfo {
            fight: 12_345,
            walk: 35_000,
            ..PhysicalInfo::default()
        });
        engine.objects[idx].state.temporary_physical = Some(PhysicalInfo {
            walk: 99_000,
            ..PhysicalInfo::default()
        });
        engine.objects[idx].state.physical_changes = vec![("Walk".to_string(), 35_000)];
        engine.objects[idx].last_energy_loss_cause = 3;

        let state = engine.capture_state();
        let mut restored = Engine::with_seed(1);
        let definition = Definition::from_script("Clonk", "Clonk", script).unwrap();
        restored
            .register_definition(definition)
            .expect("definition registers");
        restored.restore_state(&state).expect("state restores");

        let idx = restored.find_object_index(id).expect("clonk restored");
        assert_eq!(
            restored.objects[idx].state.info_physical,
            Some(PhysicalInfo {
                fight: 12_345,
                walk: 35_000,
                ..PhysicalInfo::default()
            })
        );
        assert_eq!(
            restored.objects[idx].state.temporary_physical,
            Some(PhysicalInfo {
                walk: 99_000,
                ..PhysicalInfo::default()
            })
        );
        assert_eq!(
            restored.objects[idx].state.physical_changes,
            vec![("Walk".to_string(), 35_000)]
        );
        assert_eq!(restored.objects[idx].last_energy_loss_cause, 3);
    }

    #[test]
    fn fire_effect_executes_once_per_tick() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;
        let mut definition = Definition::from_script("Torch", "Torch", script).unwrap();
        definition.set_fire_properties(1, true, true);
        let mut engine = Engine::with_seed(1);
        engine.register_definition(definition).expect("registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Torch"))
            .expect("spawns");
        let idx = engine.find_object_index(id).expect("exists");
        // C++ has no OnFire without the fire effect: ignition goes through
        // C4Object::Incinerate, which creates the timer-driven entry
        // (C4Object.cpp:1257-1266).
        assert!(engine
            .incinerate_object(idx, -1, false, None)
            .expect("incinerates"));
        engine.objects[idx].state.fire_phase = 0;
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(
            engine.objects[idx].state.fire_phase, 1,
            "ExecFire once per frame"
        );
    }

    #[test]
    fn exec_life_breath_depletes_then_asphyxiates_in_semisolid() {
        // ExecLife breathing (C4Object.cpp:878-919): Tick5, alive, no
        // breathable supply at the mouth (y + Shape.y/2 semi-solid) →
        // breath -= 2*C4MaxPhysical/100, the BubbleOut x argument draws
        // Random(5), and only at zero breath DoEnergy(-1, EngAsphyxiation).
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let library = MaterialLibrary::parse(
            r#"
            [Material Water]
            Name=Water
            Density=25
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);

        let mut definition = Definition::from_script("Diver", "Diver", script).unwrap();
        definition.set_category(CATEGORY_LIVING);
        definition.set_physical(PhysicalInfo {
            breath: 50_000,
            energy: 50_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(77);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        let mut landscape = Landscape::flat(8, 200);
        // Submerge the breathe-check column: the mouth sits at
        // y + shape.y/2 = 26 + (-8)/2 = 22.
        landscape.set_liquid_column(2, vec![LiquidSegment::new(10, 40)]);
        engine.set_landscape(landscape);

        let vertices = vec![
            ObjectVertex::new(-4, -8),
            ObjectVertex::new(4, -8),
            ObjectVertex::new(4, 8),
            ObjectVertex::new(-4, 8),
        ];
        let id = engine
            .spawn_object(
                SpawnConfig::new("Diver")
                    .with_alive(true)
                    .with_position(Vector2::new(2, 26))
                    .with_vertices(vertices)
                    .with_energy(50_000),
            )
            .expect("diver spawns");
        let idx = engine.find_object_index(id).expect("diver exists");
        assert_eq!(
            engine.objects[idx].state.breath, 50_000,
            "breath fills from the physicals at birth (C4Object.cpp:193)"
        );

        let mut mirror = engine.rng.clone();
        for _ in 0..4 {
            engine.tick_without_snapshot().expect("tick succeeds");
            let _ = mirror.random(i32::MAX); // the per-object Step draw
        }
        assert_eq!(engine.objects[idx].state.breath, 50_000, "Tick5 gate");
        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(
            engine.objects[idx].state.breath,
            50_000 - 2 * C4_MAX_PHYSICAL / 100
        );
        assert_eq!(
            engine.objects[idx].state.energy, 50_000,
            "breath before energy"
        );
        let _ = mirror.random(5); // the BubbleOut x argument (C4Object.cpp:905)
        let _ = mirror.random(i32::MAX); // the per-object Step draw
        assert_eq!(engine.rng, mirror, "exactly one extra synced draw");

        // Out of breath: the same gate now costs energy with the
        // asphyxiation cause (C4Object.cpp:904).
        engine.objects[idx].state.breath = 0;
        engine.objects[idx].last_energy_loss_cause = 7;
        for _ in 0..5 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert_eq!(engine.objects[idx].state.energy, 49_000);
        assert_eq!(engine.objects[idx].last_energy_loss_cause, 7);
    }

    #[test]
    fn exec_life_vehicle_material_is_breathable_like_cpp() {
        // C4Object::ExecLife treats MVehic at the mouth as breathable
        // before the ordinary semi-solid check (src/C4Object.cpp:884-899).
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;
        let library = MaterialLibrary::parse(
            r#"
            [Material Vehicle]
            Name=Vehicle
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let vehicle = materials.id_of("Vehicle").expect("vehicle material exists");
        let mut definition = Definition::from_script("Crew", "Crew", script).unwrap();
        definition.set_physical(PhysicalInfo {
            breath: 50_000,
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        definition.set_shape_rect(Some(DefinitionRect::new(-4, -8, 8, 16)));

        let mut engine = Engine::with_seed(77);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        let mut densities = vec![0; 128];
        densities[20] = 100;
        let mut material_names = vec![None; 128];
        material_names[20] = Some("Vehicle".to_string());
        let bytes = vec![20; 8 * 40];
        let grid = landscape::PixelGrid::new(
            8,
            40,
            bytes,
            densities,
            material_names,
            vec![None; 128],
        );
        let mut landscape = Landscape::new(8, vec![40; 8]).expect("landscape builds");
        landscape.set_world_height(40);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let vertices = vec![
            ObjectVertex::new(-4, -8),
            ObjectVertex::new(4, -8),
            ObjectVertex::new(4, 8),
            ObjectVertex::new(-4, 8),
        ];
        let id = engine
            .spawn_object(
                SpawnConfig::new("Crew")
                    .with_alive(true)
                    .with_position(Vector2::new(2, 26))
                    .with_vertices(vertices)
                    .with_energy(50_000),
            )
            .expect("crew spawns");
        let idx = engine.find_object_index(id).expect("crew exists");
        assert_eq!(
            engine
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.material_at(2, 22)),
            Some(vehicle)
        );
        assert!(engine
            .landscape
            .as_ref()
            .is_some_and(|landscape| landscape.is_solid_at(2, 22)));
        assert_eq!(
            engine.objects[idx].current_shape_rect(),
            Some(DefinitionRect::new(-4, -8, 8, 16))
        );
        engine.objects[idx].state.breath = 10_000;

        for _ in 0..5 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert_eq!(engine.objects[idx].state.breath, 50_000);
        assert_eq!(engine.objects[idx].state.energy, 50_000);
    }

    #[test]
    fn exec_life_breath_restores_with_supply() {
        // Supply branch (C4Object.cpp:911-918): takebreath restores to the
        // physical maximum in one gulp.
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Diver", "Diver", script).unwrap();
        definition.set_physical(PhysicalInfo {
            breath: 50_000,
            energy: 50_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(77);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Diver").with_alive(true).with_energy(50))
            .expect("diver spawns");
        let idx = engine.find_object_index(id).expect("diver exists");
        engine.objects[idx].state.breath = 10_000;

        for _ in 0..5 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert_eq!(engine.objects[idx].state.breath, 50_000);
    }

    fn exec_life_material_landscape(
        width: u32,
        height: u32,
        material: &str,
        points: &[(u32, u32)],
    ) -> Landscape {
        let mut pixels = vec![0_u8; (width * height) as usize];
        for &(x, y) in points {
            pixels[(y * width + x) as usize] = 10;
        }
        let mut densities = vec![0_i32; 128];
        densities[10] = 0;
        let mut names = vec![None; 128];
        names[10] = Some(material.to_string());
        let grid =
            landscape::PixelGrid::new(width, height, pixels, densities, names, vec![None; 128]);
        let mut landscape = Landscape::new(width, vec![height as i32; width as usize]).unwrap();
        landscape.set_pixel_grid(grid);
        landscape
    }

    #[test]
    fn exec_life_periodic_base_energy_buys_and_transfers_exact_units() -> Result<(), EngineError> {
        let mut base = Definition::from_script(
            "BASE",
            "Base",
            r#"
local debit_by;
func FxOwnerSwapDamage(pTarget, iNumber, iChange, iCause, iCausePlr) {
    if (iChange < 0) {
        debit_by = iCausePlr;
        SetOwner(2);
    }
    return(iChange);
}
"#,
        )?;
        base.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        let mut crew = Definition::from_script(
            "CREW",
            "Crew",
            r#"
local credit_by;
func FxCreditProbeDamage(pTarget, iNumber, iChange, iCause, iCausePlr) {
    if (iChange > 0) credit_by = iCausePlr;
    return(iChange);
}
"#,
        )?;
        crew.set_category(CATEGORY_LIVING);
        crew.set_no_breath(true);
        crew.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });

        let mut engine = Engine::with_seed(1);
        engine.register_definition(base)?;
        engine.register_definition(crew)?;
        engine.register_player(PlayerConfig::new(1, "Home").with_wealth(12))?;
        engine.register_player(PlayerConfig::new(2, "New base owner"))?;
        engine.set_base_regenerate_energy_price(7);
        let base_id = engine.spawn_object(
            SpawnConfig::new("BASE")
                .with_owner(1)
                .with_alive(true)
                .with_energy(0),
        )?;
        let base_idx = engine.find_object_index(base_id).unwrap();
        engine.objects[base_idx].state.effects.push(
            EffectState::new("OwnerSwap").with_command_target(Some(base_id.as_u64() as i32)),
        );
        engine.objects[base_idx].state.base = 1;
        let crew_id = engine.spawn_object(
            SpawnConfig::new("CREW")
                .with_owner(1)
                .with_alive(true)
                .with_energy(10_000)
                .with_container(base_id),
        )?;
        let crew_idx = engine.find_object_index(crew_id).unwrap();
        engine.objects[crew_idx].state.effects.push(
            EffectState::new("CreditProbe").with_command_target(Some(crew_id.as_u64() as i32)),
        );

        engine.tick_without_snapshot()?;
        engine.tick_without_snapshot()?;
        assert_eq!(engine.object_snapshot(crew_id).unwrap().energy, 10_000);
        engine.tick_without_snapshot()?;
        assert_eq!(engine.object_snapshot(crew_id).unwrap().energy, 12_000);
        assert_eq!(engine.object_snapshot(base_id).unwrap().energy, 48_000);
        assert_eq!(engine.player(1).unwrap().wealth(), 5);
        assert_eq!(
            engine.objects[base_idx].state.local_vars.get("debit_by"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine.objects[crew_idx].state.local_vars.get("credit_by"),
            Some(&Value::Int(2)),
            "the credit re-reads Contained->Owner after donor callbacks"
        );

        engine.objects[base_idx].state.energy = 1_200;
        engine.objects[crew_idx].state.energy = 49_500;
        for _ in 0..3 {
            engine.tick_without_snapshot()?;
        }
        assert_eq!(engine.object_snapshot(crew_id).unwrap().energy, 50_000);
        assert_eq!(engine.object_snapshot(base_id).unwrap().energy, 700);

        engine.objects[crew_idx].state.energy = 48_000;
        for _ in 0..3 {
            engine.tick_without_snapshot()?;
        }
        assert_eq!(engine.object_snapshot(crew_id).unwrap().energy, 48_700);
        assert_eq!(engine.object_snapshot(base_id).unwrap().energy, 0);
        Ok(())
    }

    #[test]
    fn exec_life_periodic_magic_uses_global_debit_gate_and_whole_points() -> Result<(), EngineError>
    {
        fn run(override_source: &str) -> Result<(i32, i32), EngineError> {
            let mut donor = Definition::from_script("DONR", "Donor", override_source)?;
            donor.set_physical(PhysicalInfo {
                magic: 50_000,
                ..PhysicalInfo::default()
            });
            let mut recipient = Definition::from_script("RCPT", "Recipient", "")?;
            recipient.set_category(CATEGORY_LIVING);
            recipient.set_no_breath(true);
            recipient.set_physical(PhysicalInfo {
                magic: 50_000,
                ..PhysicalInfo::default()
            });
            let mut engine = Engine::with_seed(2);
            engine.register_definition(donor)?;
            engine.register_definition(recipient)?;
            let donor_id = engine.spawn_object(
                SpawnConfig::new("DONR")
                    .with_owner(1)
                    .with_magic_energy(1_500),
            )?;
            let recipient_id = engine.spawn_object(
                SpawnConfig::new("RCPT")
                    .with_owner(1)
                    .with_alive(true)
                    .with_magic_energy(0)
                    .with_container(donor_id),
            )?;
            for _ in 0..3 {
                engine.tick_without_snapshot()?;
            }
            Ok((
                engine.object_snapshot(donor_id).unwrap().magic_energy,
                engine.object_snapshot(recipient_id).unwrap().magic_energy,
            ))
        }

        assert_eq!(run("")?, (500, 1_000));
        assert_eq!(
            run("func DoMagicEnergy(int change, object target) { return(false); }")?,
            (500, 1_000),
            "an object-local function does not shadow the engine-global debit"
        );
        assert_eq!(
            run("global func DoMagicEnergy(int change, object target) { return(change >= 0); }")?,
            (1_500, 0),
            "a false engine-global debit suppresses the credit"
        );
        Ok(())
    }

    #[test]
    fn exec_life_periodic_corrosion_and_closed_container_use_cached_in_mat(
    ) -> Result<(), EngineError> {
        let library =
            MaterialLibrary::parse("[Material Acid]\nName=Acid\nDensity=0\nCorrosive=31\n")
                .unwrap();
        let mut engine = Engine::with_seed(3);
        engine.set_materials(MaterialSet::from_resource_library(&library));
        let points = (0..4)
            .flat_map(|y| (0..4).map(move |x| (x, y)))
            .collect::<Vec<_>>();
        engine.set_landscape(exec_life_material_landscape(4, 4, "Acid", &points));
        engine.set_physics(PhysicsSettings::new(0, 200, -200));

        let mut victim = Definition::from_script(
            "VCTM",
            "Victim",
            r#"
local corrosion_change, corrosion_cause, corrosion_by;
func FxCorrosionProbeDamage(pTarget, iNumber, iChange, iCause, iCausePlr) {
    corrosion_change = iChange;
    corrosion_cause = iCause;
    corrosion_by = iCausePlr;
    return(iChange);
}
"#,
        )?;
        victim.set_category(CATEGORY_LIVING);
        victim.set_no_breath(true);
        victim.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        let mut resistant = Definition::from_script("RSST", "Resistant", "")?;
        resistant.set_category(CATEGORY_LIVING);
        resistant.set_no_breath(true);
        resistant.set_physical(PhysicalInfo {
            energy: 50_000,
            corrosion_resist: 1,
            ..PhysicalInfo::default()
        });
        let open = Definition::from_script("OPEN", "Open", "")?;
        let mut closed = Definition::from_script("CLSD", "Closed", "")?;
        closed.set_closed_container(2);
        engine.register_definition(victim)?;
        engine.register_definition(resistant)?;
        engine.register_definition(open)?;
        engine.register_definition(closed)?;

        let direct = engine.spawn_object(
            SpawnConfig::new("VCTM")
                .with_position(Vector2::new(1, 1))
                .with_alive(true)
                .with_energy(50_000),
        )?;
        let direct_idx = engine.find_object_index(direct).unwrap();
        engine.objects[direct_idx].state.effects.push(
            EffectState::new("CorrosionProbe")
                .with_command_target(Some(direct.as_u64() as i32)),
        );
        engine.objects[direct_idx].last_energy_loss_cause = 7;
        let resist = engine.spawn_object(
            SpawnConfig::new("RSST")
                .with_position(Vector2::new(1, 1))
                .with_alive(true)
                .with_energy(50_000),
        )?;
        let open_id =
            engine.spawn_object(SpawnConfig::new("OPEN").with_position(Vector2::new(1, 1)))?;
        let closed_id =
            engine.spawn_object(SpawnConfig::new("CLSD").with_position(Vector2::new(1, 1)))?;
        let open_child = engine.spawn_object(
            SpawnConfig::new("VCTM")
                .with_alive(true)
                .with_energy(50_000)
                .with_container(open_id),
        )?;
        let closed_child = engine.spawn_object(
            SpawnConfig::new("VCTM")
                .with_alive(true)
                .with_energy(50_000)
                .with_container(closed_id),
        )?;

        for _ in 0..9 {
            engine.tick_without_snapshot()?;
        }
        assert_eq!(engine.object_snapshot(direct).unwrap().energy, 50_000);
        engine.tick_without_snapshot()?;
        assert_eq!(engine.object_snapshot(direct).unwrap().energy, 48_000);
        assert_eq!(engine.objects[direct_idx].last_energy_loss_cause, 7);
        assert_eq!(
            engine.objects[direct_idx]
                .state
                .local_vars
                .get("corrosion_change"),
            Some(&Value::Int(-2_000))
        );
        assert_eq!(
            engine.objects[direct_idx]
                .state
                .local_vars
                .get("corrosion_cause"),
            Some(&Value::Int(C4FX_CALL_ENG_CORROSION))
        );
        assert_eq!(
            engine.objects[direct_idx]
                .state
                .local_vars
                .get("corrosion_by"),
            Some(&Value::Int(7))
        );
        assert_eq!(engine.object_snapshot(resist).unwrap().energy, 50_000);
        assert_eq!(engine.object_snapshot(open_child).unwrap().energy, 48_000);
        assert_eq!(engine.object_snapshot(closed_child).unwrap().energy, 50_000);

        let mild_library =
            MaterialLibrary::parse("[Material Mild]\nName=Mild\nDensity=0\nCorrosive=14\n")
                .unwrap();
        let mut mild_engine = Engine::with_seed(33);
        mild_engine.set_materials(MaterialSet::from_resource_library(&mild_library));
        mild_engine.set_landscape(exec_life_material_landscape(3, 3, "Mild", &[(1, 1)]));
        let mut mild_definition = Definition::from_script(
            "MILD",
            "Mild corrosion probe",
            r#"
local corrosion_change, corrosion_cause, corrosion_by;
func FxCorrosionProbeDamage(pTarget, iNumber, iChange, iCause, iCausePlr) {
    corrosion_change = iChange + 1;
    corrosion_cause = iCause;
    corrosion_by = iCausePlr;
    return(-1000);
}
"#,
        )?;
        mild_definition.set_category(CATEGORY_LIVING);
        mild_definition.set_no_breath(true);
        mild_definition.set_physical(PhysicalInfo {
            energy: 50_000,
            ..PhysicalInfo::default()
        });
        mild_engine.register_definition(mild_definition)?;
        let mild = mild_engine.spawn_object(
            SpawnConfig::new("MILD")
                .with_position(Vector2::new(1, 1))
                .with_category(CATEGORY_LIVING)
                .with_alive(true)
                .with_energy(50_000),
        )?;
        let mild_idx = mild_engine.find_object_index(mild).unwrap();
        mild_engine.objects[mild_idx].state.effects.push(
            EffectState::new("CorrosionProbe")
                .with_command_target(Some(mild.as_u64() as i32)),
        );
        mild_engine.objects[mild_idx].last_energy_loss_cause = 9;
        mild_engine.frame = 9;
        mild_engine.tick_without_snapshot()?;
        assert_eq!(mild_engine.object_snapshot(mild).unwrap().energy, 49_000);
        assert_eq!(
            mild_engine.objects[mild_idx]
                .state
                .local_vars
                .get("corrosion_change"),
            Some(&Value::Int(1)),
            "Corrosive < 15 still visits the head damage effect with zero"
        );
        assert_eq!(
            mild_engine.objects[mild_idx]
                .state
                .local_vars
                .get("corrosion_cause"),
            Some(&Value::Int(C4FX_CALL_ENG_CORROSION))
        );
        assert_eq!(
            mild_engine.objects[mild_idx]
                .state
                .local_vars
                .get("corrosion_by"),
            Some(&Value::Int(9))
        );
        Ok(())
    }

    #[test]
    fn exec_life_periodic_lava_uses_pre_movement_cache_and_standard_fire_draw(
    ) -> Result<(), EngineError> {
        let library =
            MaterialLibrary::parse("[Material Lava]\nName=Lava\nDensity=0\nIncindiary=1\n")
                .unwrap();
        let mut engine = Engine::with_seed(4);
        engine.set_materials(MaterialSet::from_resource_library(&library));
        engine.set_landscape(exec_life_material_landscape(20, 3, "Lava", &[(1, 1)]));
        engine.set_physics(PhysicsSettings::new(0, 200, -200));
        let mut definition = Definition::from_script(
            "FIRE",
            "Fire target",
            "local cause; func Incineration(int by) { cause = by; return(1); }",
        )?;
        definition.set_category(CATEGORY_OBJECT);
        definition.set_fire_properties(37, true, true);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("FIRE")
                .with_position(Vector2::new(1, 1))
                .with_velocity(Vector2::new(10, 0))
                .with_mobile(true)
                .with_alive(false),
        )?;
        let idx = engine.find_object_index(id).unwrap();
        engine.objects[idx].last_energy_loss_cause = 7;
        engine.frame = 9;
        let mut expected_rng = engine.rng.clone();
        let expected_phase = expected_rng.random(MAX_FIRE_PHASE);
        if expected_rng.random(60) == 0 {
            let _ = expected_rng.random(100);
        }
        if expected_rng.random(35) == 0 {
            let _ = expected_rng.random(100);
        }
        if expected_rng.random(50) == 0 {
            let _ = expected_rng.random(100);
        }
        if expected_rng.random(60) == 0 {
            let _ = expected_rng.random(100);
        }
        engine.tick_without_snapshot()?;

        let state = engine.object_snapshot(id).unwrap();
        assert_ne!(state.position.x, 1, "movement left the lava pixel");
        assert!(state.on_fire, "cached frame-start Lava still ignites");
        assert_eq!(state.fire_phase, expected_phase);
        assert_eq!(state.local_vars.get("cause"), Some(&Value::Int(7)));
        assert_eq!(engine.rng.count, expected_rng.count);
        assert_eq!(engine.rng.hold, expected_rng.hold);
        Ok(())
    }

    #[test]
    fn exec_life_periodic_nonlife_decay_respects_base_and_energy_holder() -> Result<(), EngineError>
    {
        let mut normal = Definition::from_script("NORM", "Normal", "")?;
        normal.set_physical(PhysicalInfo {
            energy: 10_000,
            ..PhysicalInfo::default()
        });
        let mut holder = Definition::from_script("HOLD", "Holder", "")?;
        holder.set_physical(PhysicalInfo {
            energy: 10_000,
            ..PhysicalInfo::default()
        });
        holder.set_line_connect(LINE_CONNECT_ENERGY_HOLDER);
        let mut engine = Engine::with_seed(5);
        engine.register_definition(normal)?;
        engine.register_definition(holder)?;
        engine.register_player(PlayerConfig::new(1, "Base owner"))?;
        let ordinary = engine.spawn_object(
            SpawnConfig::new("NORM")
                .with_alive(false)
                .with_energy(5_000),
        )?;
        let held = engine.spawn_object(
            SpawnConfig::new("HOLD")
                .with_alive(false)
                .with_energy(5_000),
        )?;
        let protected = engine.spawn_object(
            SpawnConfig::new("NORM")
                .with_alive(false)
                .with_energy(5_000),
        )?;
        let protected_idx = engine.find_object_index(protected).unwrap();
        engine.objects[protected_idx].state.base = 1;

        for _ in 0..10 {
            engine.tick_without_snapshot()?;
        }
        assert_eq!(engine.object_snapshot(ordinary).unwrap().energy, 4_000);
        assert_eq!(engine.object_snapshot(held).unwrap().energy, 5_000);
        assert_eq!(engine.object_snapshot(protected).unwrap().energy, 5_000);

        engine.set_base_regenerate_energy_enabled(false);
        for _ in 0..10 {
            engine.tick_without_snapshot()?;
        }
        assert_eq!(engine.object_snapshot(protected).unwrap().energy, 4_000);
        Ok(())
    }

    #[test]
    fn exec_life_periodic_growth_stays_suppressed_while_on_fire() -> Result<(), EngineError> {
        let mut definition = Definition::from_script("TREE", "Tree", "")?;
        definition.set_category(CATEGORY_STATIC_BACK);
        definition.set_growth(4);
        let mut engine = Engine::with_seed(6);
        engine.register_definition(definition)?;
        let id = engine.spawn_object(
            SpawnConfig::new("TREE")
                .with_construction(50_000)
                .with_alive(false),
        )?;
        let idx = engine.find_object_index(id).unwrap();
        engine.objects[idx].state.on_fire = true;
        engine.frame = 34;
        engine.tick_without_snapshot()?;
        assert_eq!(engine.object_snapshot(id).unwrap().construction, 50_000);
        Ok(())
    }

    #[test]
    fn exec_life_periodic_birthday_updates_info_and_presents_once() -> Result<(), EngineError> {
        let mut crew = Definition::from_script("BDAY", "Birthday crew", "")?;
        crew.set_crew_member(true);
        crew.set_category(CATEGORY_LIVING);
        crew.set_no_breath(true);
        let mut engine = Engine::with_seed(7);
        engine.register_definition(crew)?;
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("BDAY".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine.join_player(JoinPlayerConfig {
            name: "Birthday player".to_string(),
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
                id: "BDAY".to_string(),
                name: "Rookie".to_string(),
                death_message: String::new(),
                core: Default::default(),
                rank: 0,
                rank_name: "Clonk".to_string(),
                experience: 0,
                rounds: 6,
                physical: PhysicalInfo::default(),
                death_count: 0,
                total_playing_time: 17_999,
                birthday: 123,
                age: 0,
                participation: 1,
                in_action: false,
                was_in_action: false,
                in_action_time: 0,
                has_died: false,
                extra_data: Vec::new(),
                portraits: Default::default(),
            }],
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })?;
        let crew_id = *engine.player(0).unwrap().crew().first().unwrap();
        let linked_state = engine.capture_state();
        let link = linked_state.crew_info_links[&crew_id];
        let roster_entry = &linked_state.crew_info_rosters[&link.player_id][link.roster_index];
        let expected_info_fields = (
            roster_entry.rounds,
            roster_entry.total_playing_time,
            roster_entry.birthday,
            roster_entry.age,
            roster_entry.in_action_time,
        );

        let mut stale_info = linked_state.crew_object_infos[&crew_id].clone();
        stale_info.rounds = -9;
        stale_info.total_playing_time = -1;
        stale_info.birthday = -2;
        stale_info.age = -3;
        stale_info.in_action_time = -4;
        engine.apply_player_commands(vec![PlayerCommand::LinkCrewInfo {
            object_id: crew_id,
            link: Some(link),
            info: stale_info,
            created_entry: None,
            recruit: false,
            has_died: false,
        }])?;
        let live_info = engine.crew_object_info(crew_id).unwrap();
        assert_eq!(
            (
                live_info.rounds,
                live_info.total_playing_time,
                live_info.birthday,
                live_info.age,
                live_info.in_action_time,
            ),
            expected_info_fields,
            "LinkCrewInfo refreshes the live projection from its roster node"
        );

        let mut legacy_state = engine.capture_state();
        let legacy_info = legacy_state.crew_object_infos.get_mut(&crew_id).unwrap();
        legacy_info.rounds = -10;
        legacy_info.total_playing_time = -5;
        legacy_info.birthday = -6;
        legacy_info.age = -7;
        legacy_info.in_action_time = -8;
        engine.restore_state(&legacy_state)?;
        let restored_info = engine.crew_object_info(crew_id).unwrap();
        assert_eq!(
            (
                restored_info.rounds,
                restored_info.total_playing_time,
                restored_info.birthday,
                restored_info.age,
                restored_info.in_action_time,
            ),
            expected_info_fields,
            "restore reconciles legacy duplicated live fields from the roster node"
        );

        engine.game_time = 1;
        engine.frame = 254;
        let snapshot = engine.tick()?;
        let info_state = engine.capture_state();
        let link = info_state.crew_info_links[&crew_id];
        assert_eq!(
            info_state.crew_info_rosters[&link.player_id][link.roster_index].age,
            1
        );
        assert!(snapshot.hud.messages.iter().any(|message| {
            message.target == Some(crew_id)
                && message.lines == ["Rookie becomes 1!", "Happy birthday!"]
        }));
        assert_eq!(
            snapshot.audio,
            vec![
                AudioCommand::SetMusicPlaylist {
                    playlist: None,
                    restart: false,
                },
                AudioCommand::SetMusicLevel { level: 100 },
                AudioCommand::PlaySound {
                    name: "Trumpet".to_string(),
                    target: Some(crew_id),
                    volume: 100,
                    looped: false,
                    multiple: false,
                    custom_falloff: None,
                },
            ]
        );

        engine.frame = 509;
        let repeat = engine.tick()?;
        assert!(
            !repeat.audio.iter().any(|command| {
                matches!(command, AudioCommand::PlaySound { name, .. } if name == "Trumpet")
            }),
            "unchanged age has no second trumpet: {:?}",
            repeat.audio
        );
        Ok(())
    }

    #[test]
    fn set_physical_host_fn_applies_to_engine_state() {
        // SetPhysical(PHYS_Temporary) from a script callback auto-enables
        // temporary mode (C4Script.cpp:584-597); the scope's pending update
        // writes it back into engine state, where GetPhysical resolves the
        // temporary set first (C4Object.cpp:2118-2121).
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            if (frame == 1) {
                SetPhysical("Walk", 42000, 2);
            }
            return 0;
        }
        "#;

        let definition = Definition::from_script("Mutant", "Mutant", script).unwrap();
        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(SpawnConfig::new("Mutant"))
            .expect("mutant spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(id).expect("mutant exists");
        assert_eq!(
            engine.objects[idx].state.temporary_physical,
            Some(PhysicalInfo {
                walk: 42_000,
                ..PhysicalInfo::default()
            })
        );
        assert_eq!(engine.object_physical(idx).walk, 42_000);
    }

    #[test]
    fn chop_procedure_zeroes_velocity_and_damages_on_aligned_tick() {
        // DFA_CHOP calls Target->Chop on Tick3 frames; Chop emits one +10
        // damage event only on Tick10 frames. The counters advance together
        // before object execution, so the first aligned frame is 30
        // (C4Game.cpp:1899-1911; C4Object.cpp:1775-1782, 5202-5221).
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Chopper", "Chopper", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert(
            "Chop".to_string(),
            ActionSpec::default().with_procedure("chop"),
        );
        definition.configure_actions(Some("Chop".to_string()), actions);

        let mut engine = Engine::with_seed(20);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let damage_events = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = DebuggerHooks::new();
        {
            let damage_events = Arc::clone(&damage_events);
            hooks.set_on_call(move |name, args| {
                if name == "Damage" {
                    damage_events.lock().unwrap().push(args.to_vec());
                }
            });
        }
        let mut tree = Definition::from_script(
            "Tree",
            "Tree",
            "#strict\nfunc Damage(int change, int caused_by) { return(1); }",
        )
        .expect("tree script compiles");
        tree.set_debugger_hooks(hooks);
        tree.set_chopable(true);
        tree.set_shape_rect(Some(DefinitionRect::new(-10, -10, 20, 20)));
        engine.register_definition(tree).expect("tree registers");
        engine.set_physics(PhysicsSettings::new(3, 40, -20));

        let target = engine
            .spawn_object(
                SpawnConfig::new("Tree")
                    .with_position(Vector2::new(0, 0))
                    .with_category(CATEGORY_STATIC_BACK)
                    .with_loaded(true),
            )
            .expect("tree spawns");
        let mut action = ActionState::new("Chop");
        action.target = Some(target);
        let id = engine
            .spawn_object(
                SpawnConfig::new("Chopper")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(5, -3))
                    .with_owner(7)
                    .with_action(action),
            )
            .expect("spawn succeeds");
        let target_idx = engine.find_object_index(target).expect("tree exists");
        assert_ne!(engine.objects[target_idx].state.ocf & ocf::CHOP, 0);
        engine.objects[target_idx].state.status = ObjectStatus::Inactive;

        for frame in 1..30 {
            let snapshot = engine.tick().expect("tick succeeds");
            let object = snapshot.object(id).expect("object present");
            assert_eq!(object.action.name, "Chop");
            assert_eq!(object.action.target, Some(target));
            assert_eq!(object.velocity, Vector2::ZERO);
            assert_eq!(object.position, Vector2::new(0, 0));
            assert_eq!(
                snapshot.object(target).expect("tree present").damage,
                0,
                "frame {frame} is not both Tick3 and Tick10"
            );
        }
        assert!(damage_events.lock().unwrap().is_empty());

        let snapshot = engine.tick().expect("aligned tick succeeds");
        assert_eq!(snapshot.object(target).expect("tree present").damage, 10);
        assert_eq!(
            damage_events.lock().unwrap().as_slice(),
            [vec![Value::Int(10), Value::Int(7)]],
            "frame 30 emits exactly one +10 chop damage event by the chopper owner"
        );
    }

    #[test]
    fn chop_procedure_stands_in_walk_when_target_stops_being_choppable() {
        // DFA_CHOP calls ObjectActionStand when Target->Chop/At no longer
        // succeeds, so a Clonk resumes Walk rather than the action library's
        // generic Idle default (C4Object.cpp:5202-5221;
        // C4ObjectCom.cpp:41-46).
        let script = "#strict\nfunc Initialize() { return; }";
        let mut chopper = Definition::from_script("CLNK", "Clonk", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert(
            "Walk".to_string(),
            ActionSpec::default().with_procedure("walk"),
        );
        actions.insert(
            "Chop".to_string(),
            ActionSpec::default().with_procedure("chop"),
        );
        chopper.configure_actions(Some("Idle".to_string()), actions);

        let target = Definition::from_script("TREE", "Tree", script).unwrap();
        let mut engine = Engine::with_seed(21);
        engine.register_definition(chopper).unwrap();
        engine.register_definition(target).unwrap();
        let target = engine
            .spawn_object(SpawnConfig::new("TREE").with_loaded(true))
            .unwrap();
        let mut action = ActionState::new("Chop");
        action.target = Some(target);
        let clonk = engine
            .spawn_object(SpawnConfig::new("CLNK").with_action(action))
            .unwrap();

        let snapshot = engine.tick().unwrap();
        assert_eq!(snapshot.object(clonk).unwrap().action.name, "Walk");
    }

    #[test]
    fn wind_force_respects_variation_and_period() {
        // The old sinusoidal per-frame wind model was an invention; the C++
        // wind is mutable state (C4Weather::Wind) advanced by the tick gates
        // in advance_frame - wind_force reports it regardless of the frame.
        let settings = EnvironmentSettings::new(2).with_wind_variation(4, 4);
        for frame in 0..5 {
            assert_eq!(settings.wind_force(frame), 2);
        }

        let default_period = EnvironmentSettings::new(1).with_wind_variation(3, 0);
        assert_eq!(default_period.wind_variation, 3);
        assert_eq!(default_period.wind_period, 2);
    }

    #[test]
    fn ambient_temperature_cycles_with_climate() {
        let settings = EnvironmentSettings::new(0)
            .with_temperature(10)
            .with_climate(5)
            .with_temperature_cycle(8, 12, 3);

        assert_eq!(settings.ambient_temperature(0), 15);
        assert_eq!(settings.ambient_temperature(3), 23);
        assert_eq!(settings.ambient_temperature(9), 7);
    }

    #[test]
    fn temperature_at_height_respects_gradient() {
        let settings = EnvironmentSettings::new(0)
            .with_temperature(0)
            .with_climate(0)
            .with_temperature_range(40);
        let world_height = 200;
        let top = settings.temperature_at_height(0, 0, world_height);
        let middle = settings.temperature_at_height(0, world_height / 2, world_height);
        let bottom = settings.temperature_at_height(0, world_height, world_height);

        assert_eq!(middle, settings.ambient_temperature(0));
        assert!(
            top < middle,
            "expected top of map to be colder than mid level"
        );
        assert!(
            bottom > middle,
            "expected bottom of map to be warmer than mid level"
        );
    }

    #[test]
    fn ambient_temperature_resets_when_cycle_disabled() {
        let mut settings = EnvironmentSettings::new(0)
            .with_temperature(5)
            .with_climate(-2)
            .with_temperature_cycle(10, 6, 0);
        assert_ne!(settings.ambient_temperature(1), 3);

        settings = settings.with_temperature_cycle(0, 6, 0);
        assert_eq!(settings.temperature_variation, 0);
        assert_eq!(settings.temperature_period, 0);
        assert_eq!(settings.temperature_phase, 0);
        assert_eq!(settings.ambient_temperature(1), 3);
    }

    #[test]
    fn temperature_rises_towards_target_without_year_speed() {
        let mut settings = EnvironmentSettings::new(0)
            .with_climate(20)
            .with_temperature_range(30)
            .with_season(0)
            .with_temperature(-40);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.temperature, -39);
    }

    #[test]
    fn temperature_falls_towards_target_without_year_speed() {
        let mut settings = EnvironmentSettings::new(0)
            .with_climate(-10)
            .with_temperature_range(20)
            .with_season(50)
            .with_temperature(40);
        let mut rng = LcgRng::seed_from_u64(1);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.temperature, 39);
    }

    // C4Weather::SetSeasonGamma (C4Weather.cpp:259-285): season num/offset
    // from `(Season / 25) % 4` and `BoundBy(Season % 25, 5, 19) - 5`, the
    // 3-point ramp interpolated between the SeasonColors rows (:251-257),
    // negative Temperature shifting red/green down and blue up by
    // `Temperature / 2` (truncating division).
    #[test]
    fn season_gamma_winter_start_is_the_exact_winter_ramp() {
        // Season=0: iSeason1=0 (winter), iSeasonOff1=BoundBy(0,5,19)-5=0 —
        // the blend collapses to SeasonColors[0] verbatim.
        let settings = EnvironmentSettings::new(0)
            .with_season(0)
            .with_gamma_enabled();
        assert_eq!(
            settings.season_gamma(),
            Some((
                RgbColor::new(0x00, 0x00, 0x00),
                RgbColor::new(0x7f, 0x7f, 0x90),
                RgbColor::new(0xef, 0xef, 0xff),
            ))
        );
    }

    #[test]
    fn season_gamma_blends_winter_into_spring_with_truncating_division() {
        // Season=12: off1=7, off2=8; channel = (c1*8 + c2*7) / 15 with C++
        // integer truncation (C4Weather.cpp:272), e.g. mid red
        // (0x7f*8 + 0x90*7)/15 = 2024/15 = 134 (not 135).
        let settings = EnvironmentSettings::new(0)
            .with_season(12)
            .with_gamma_enabled();
        assert_eq!(
            settings.season_gamma(),
            Some((
                RgbColor::new(3, 7, 0),
                RgbColor::new(134, 142, 136),
                RgbColor::new(246, 246, 240),
            ))
        );
    }

    #[test]
    fn season_gamma_negative_temperature_shifts_blue_up_red_green_down() {
        // Temperature=-25: Temperature/2 = -12 (truncation toward zero,
        // C4Weather.cpp:274-279); red+green += -12, blue -= -12, channels
        // clamped to 0..255.
        let settings = EnvironmentSettings::new(0)
            .with_season(0)
            .with_temperature(-25)
            .with_gamma_enabled();
        assert_eq!(
            settings.season_gamma(),
            Some((
                RgbColor::new(0, 0, 12),
                RgbColor::new(115, 115, 156),
                RgbColor::new(227, 227, 255),
            ))
        );
    }

    #[test]
    fn season_gamma_season_hundred_wraps_to_winter() {
        // Season=100: (100/25)%4 = 0 and 100%25 = 0 — identical to
        // Season=0 (C4Weather.cpp:263-264).
        let at_hundred = EnvironmentSettings::new(0)
            .with_season(100)
            .with_gamma_enabled();
        let at_zero = EnvironmentSettings::new(0)
            .with_season(0)
            .with_gamma_enabled();
        assert_eq!(at_hundred.season_gamma(), at_zero.season_gamma());
    }

    #[test]
    fn season_gamma_uses_cpp_truncating_remainder_for_small_negative_season() {
        // Scenario C4SVal bounds are data, so a custom StartSeason may yield
        // -1. C++ integer division truncates toward zero: -1/25 == 0 and the
        // negative remainder is clamped to 5, selecting the exact winter
        // start (C4Weather.cpp:263-264). Euclidean modulo would select fall.
        let mut negative = EnvironmentSettings::new(0).with_gamma_enabled();
        negative.season = -1;
        let zero = EnvironmentSettings::new(0)
            .with_season(0)
            .with_gamma_enabled();

        assert_eq!(negative.season_gamma(), zero.season_gamma());
    }

    #[test]
    fn season_gamma_suppressed_by_no_gamma() {
        // `if (NoGamma) return;` (C4Weather.cpp:261); C4Weather::Default
        // starts with NoGamma=true (:193).
        let settings = EnvironmentSettings::new(0).with_season(0);
        assert!(settings.no_gamma);
        assert_eq!(settings.season_gamma(), None);
    }

    // C4Weather::Execute season advance (C4Weather.cpp:74-86):
    //   SeasonDelay += YearSpeed;
    //   if (SeasonDelay >= 200) { SeasonDelay = 0; Season++;
    //       if (Season > StartSeason.Max) Season = StartSeason.Min; }
    // — one step per Tick35, delay reset to ZERO, wrap only past the
    // scenario StartSeason.Max (default C4SVal bounds 0/100,
    // C4Scenario.h:30).
    #[test]
    fn season_advance_resets_delay_to_zero() {
        let mut settings = EnvironmentSettings::new(0)
            .with_season(10)
            .with_year_speed(150);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!((settings.season, settings.season_delay), (10, 150));
        settings.advance_frame(&mut rng, 70);
        // 300 >= 200: advance once, delay = 0 (NOT 300 - 200 = 100).
        assert_eq!((settings.season, settings.season_delay), (11, 0));
    }

    #[test]
    fn season_advances_one_step_per_tick35_even_with_huge_year_speed() {
        // YearSpeed=450 overshoots 200 twice over, but C++ still advances
        // exactly one season per Tick35 (no loop, C4Weather.cpp:78-81).
        let mut settings = EnvironmentSettings::new(0)
            .with_season(10)
            .with_year_speed(450);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!((settings.season, settings.season_delay), (11, 0));
    }

    #[test]
    fn season_reaches_the_default_max_of_one_hundred_before_wrapping() {
        // Default bounds Min=0/Max=100: Season 99 -> 100 (100 > 100 is
        // false, so no wrap yet), then 101 > 100 wraps to Min=0.
        let mut settings = EnvironmentSettings::new(0)
            .with_season(99)
            .with_year_speed(200);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.season, 100);
        settings.advance_frame(&mut rng, 70);
        assert_eq!(settings.season, 0);
    }

    #[test]
    fn season_wraps_to_scenario_min_beyond_scenario_max() {
        // Scenario StartSeason Min/Max are the wrap bounds
        // (C4Weather.cpp:82-83), not hard-coded 0/100.
        let mut settings = EnvironmentSettings::new(0)
            .with_season(60)
            .with_season_bounds(20, 60)
            .with_year_speed(200);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.season, 20);
    }

    #[test]
    fn season_pins_at_min_when_max_below_min() {
        // SeasonMax < SeasonMin: values <= Max keep incrementing; once
        // past Max every advance re-assigns Min, which is itself > Max —
        // the season pins at Min.
        let mut settings = EnvironmentSettings::new(0)
            .with_season(5)
            .with_season_bounds(50, 10)
            .with_year_speed(200);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.season, 6, "below Max the advance is plain ++");
        settings.season = 10;
        settings.advance_frame(&mut rng, 70);
        assert_eq!(settings.season, 50, "11 > Max(10) wraps to Min(50)");
        settings.advance_frame(&mut rng, 105);
        assert_eq!(settings.season, 50, "51 > Max(10) re-pins to Min(50)");
    }

    #[test]
    fn preloaded_season_delay_advances_even_with_zero_year_speed() {
        // C++ has no YearSpeed gate: a (savegame-)preloaded SeasonDelay
        // >= 200 still advances once (C4Weather.cpp:77-81).
        let mut settings = EnvironmentSettings::new(0).with_season(3);
        settings.season_delay = 200;
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!((settings.season, settings.season_delay), (4, 0));
    }

    #[test]
    fn negative_year_speed_never_advances_or_regresses_season() {
        // C++ only tests SeasonDelay >= 200; a negative YearSpeed just
        // accumulates negative delay forever (C4Weather.cpp:77-78).
        let mut settings = EnvironmentSettings::new(0)
            .with_season(40)
            .with_year_speed(-300);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        settings.advance_frame(&mut rng, 70);
        assert_eq!((settings.season, settings.season_delay), (40, -600));
    }

    // C4Weather::Execute's temperature step (C4Weather.cpp:88-93):
    //   iTemperature = Climate - int32(TemperatureRange
    //                    * cos(6.28 * float(Season) / 100.0));
    // then Temperature moves one degree toward it every Tick35.
    #[test]
    fn temperature_drifts_toward_climate_when_range_is_zero() {
        // No TemperatureRange gate in C++: with range 0 the target is
        // plain Climate and the drift still runs.
        let mut settings = EnvironmentSettings::new(0)
            .with_climate(10)
            .with_temperature_range(0)
            .with_temperature(0);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.temperature, 1);
    }

    #[test]
    fn temperature_target_truncates_the_cpp_cos_product() {
        // Season=12: cos(6.28 * 12 / 100.0) = 0.72923..., * 30 = 21.877 —
        // the C++ int cast TRUNCATES to 21 (not rounds to 22), so a
        // temperature of -21 already sits at the target and must not move.
        let mut settings = EnvironmentSettings::new(0)
            .with_climate(0)
            .with_temperature_range(30)
            .with_season(12)
            .with_temperature(-21);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.temperature, -21);
    }

    #[test]
    fn temperature_target_truncates_toward_zero_past_half_year() {
        // Season=50: cos(3.14) = -0.9999987, * 20 = -19.99997 — the C++
        // cast truncates toward ZERO to -19, so the target is
        // Climate + 19 = 9 and a temperature of 9 must not move.
        let mut settings = EnvironmentSettings::new(0)
            .with_climate(-10)
            .with_temperature_range(20)
            .with_season(50)
            .with_temperature(9);
        let mut rng = LcgRng::seed_from_u64(0);
        settings.advance_frame(&mut rng, 35);
        assert_eq!(settings.temperature, 9);
    }

    #[test]
    fn weather_init_ingests_start_season_bounds_without_extra_clamp() {
        // C4Weather::Init (C4Weather.cpp:41): `Season =
        // StartSeason.Evaluate()` — already bounded by the C4SVal Min/Max,
        // with NO additional 0..100 clamp; Execute's wrap then reuses the
        // same scenario bounds (:82-83).
        let mut engine = Engine::with_seed(7);
        let flat = |value: i32| clonk_engine::scenario::LegacyC4SVal::new(value, 0, 0, 100);
        let init = clonk_engine::scenario::LegacyWeatherInit {
            season: clonk_engine::scenario::LegacyC4SVal::new(120, 0, 110, 130),
            year_speed: flat(0),
            climate: flat(50),
            wind: flat(0),
            rain: flat(0),
            precipitation: "Water".to_string(),
            lightning: flat(0),
            meteorite: flat(0),
            volcano: flat(0),
            earthquake: flat(0),
            no_initialize: true,
            no_gamma: true,
        };
        engine
            .apply_weather_init(&init)
            .expect("weather init applies");
        assert_eq!(engine.environment.season, 120);
        assert_eq!(
            (
                engine.environment.season_min,
                engine.environment.season_max
            ),
            (110, 130)
        );
    }

    fn test_precipitation_definition() -> Definition {
        let script = r#"#strict
local iMat, iLength, iStrength, iMovement;

func Movement()
{
    iMovement++;
    SetXDir(BoundBy(GetWind(0, 3), -100, 100));
    if (GetX() > LandscapeWidth() - 20) SetPosition(25, -1);
    if (GetX() < 20) SetPosition(LandscapeWidth() - 25, -1);
}

func Activate(inMat, inLength, inStrength)
{
    SetAction("Process");
    iMat = inMat;
    iLength = inLength;
    iStrength = inStrength;
    return(1);
}
"#;
        let mut definition =
            Definition::from_script("FXP1", "Precipitation", script).expect("cloud compiles");
        definition.set_c4_callback_convention(true);
        definition.set_category(CATEGORY_VEHICLE);
        definition.set_mass(1);
        definition.set_shape_rect(Some(DefinitionRect::new(-50, 0, 100, 1)));
        definition.configure_actions(
            None,
            HashMap::from([(
                "Process".to_string(),
                ActionSpec::default()
                    .with_procedure("FLOAT")
                    .with_length(15)
                    .with_delay(2)
                    .with_next("Process")
                    .with_start_call("Movement"),
            )]),
        );
        definition
    }

    fn fixed_weather_init(
        rain: i32,
        wind: i32,
        precipitation: &str,
    ) -> clonk_engine::scenario::LegacyWeatherInit {
        let flat = |value: i32| clonk_engine::scenario::LegacyC4SVal::new(value, 0, -100, 100);
        clonk_engine::scenario::LegacyWeatherInit {
            season: flat(50),
            year_speed: flat(0),
            climate: flat(50),
            wind: flat(wind),
            rain: flat(rain),
            precipitation: precipitation.to_string(),
            lightning: flat(0),
            meteorite: flat(0),
            volcano: flat(0),
            earthquake: flat(0),
            no_initialize: false,
            no_gamma: true,
        }
    }

    #[test]
    fn get_temperature_returns_weather_temperature_after_init_and_season_drift() {
        // C4Weather::Init assigns Temperature = Climate after evaluating the
        // scenario climate (C4Weather.cpp:43-45). C4Weather::Execute then
        // moves that one Temperature field toward the seasonal target on
        // Tick35 (:87-93), and FnGetTemperature returns it verbatim through
        // C4Weather::GetTemperature (:173-176). Climate is not added again.
        let mut engine = Engine::with_seed(17);
        engine
            .register_definition(
                Definition::from_script(
                    "WTMP",
                    "Weather temperature probe",
                    "func ReadTemperature() { return(GetTemperature()); }",
                )
                .expect("temperature probe compiles"),
            )
            .expect("temperature probe registers");
        let mut init = fixed_weather_init(0, 0, "Water");
        init.season = clonk_engine::scenario::LegacyC4SVal::new(0, 0, 0, 100);
        init.climate = clonk_engine::scenario::LegacyC4SVal::new(30, 0, 0, 100);
        engine
            .apply_weather_init(&init)
            .expect("weather init applies");
        let probe = engine
            .spawn_object(SpawnConfig::new("WTMP"))
            .expect("temperature probe spawns");
        let probe_index = engine.find_object_index(probe).expect("probe exists");

        assert_eq!(engine.environment().climate, 20);
        assert_eq!(engine.environment().temperature, 20);
        assert_eq!(
            engine
                .call_object_function(probe_index, "ReadTemperature", Vec::new())
                .expect("GetTemperature succeeds after weather init"),
            Value::Int(20)
        );

        for _ in 0..35 {
            engine.tick_without_snapshot().expect("weather tick succeeds");
        }
        assert_eq!(engine.environment().temperature, 19);
        let probe_index = engine.find_object_index(probe).expect("probe remains");
        assert_eq!(
            engine
                .call_object_function(probe_index, "ReadTemperature", Vec::new())
                .expect("GetTemperature succeeds after seasonal drift"),
            Value::Int(19)
        );
    }

    #[test]
    fn weather_init_launches_cpp_precipitation_objects_in_draw_order() {
        // C4Weather::Init evaluates Rain once as a gate, then for every
        // min(GBackWdt/500, 5) cloud draws Random(320), Random(GBackWdt),
        // and Rain.Evaluate before LaunchCloud (C4Weather.cpp:48-58).
        // LaunchCloud creates FXP1 at (x,-1), NO_OWNER/full-con, then calls
        // Activate(material,width,strength) (:205-214). FXP1 Activate sets
        // Process synchronously; its StartCall Movement sets xdir before
        // the three locals are assigned (Precipitation.c4d Script.c/ActMap).
        let materials = MaterialSet::from_resource_library(
            &MaterialLibrary::parse("[Material]\nName=Water\nDensity=50\n")
                .expect("water parses"),
        );
        let water = materials.id_of("Water").expect("water exists");
        assert_eq!(water.index(), 0, "fixture exercises the falsy material id");
        let mut engine = Engine::with_seed(17);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat(1_100, 100));
        engine
            .register_definition(test_precipitation_definition())
            .expect("cloud registers");
        let init = fixed_weather_init(77, 37, "Water");

        let mut mirror = engine.rng.clone();
        let _season = init.season.evaluate(&mut mirror);
        let _year_speed = init.year_speed.evaluate(&mut mirror);
        let _climate = init.climate.evaluate(&mut mirror);
        let _wind = init.wind.evaluate(&mut mirror);
        let _rain_gate = init.rain.evaluate(&mut mirror);
        let mut expected = Vec::new();
        for _ in 0..2 {
            let width = 1_100 / 15 + mirror.random(320);
            let x = mirror.random(1_100);
            let strength = init.rain.evaluate(&mut mirror);
            expected.push((x, width, strength));
        }
        let _lightning = init.lightning.evaluate(&mut mirror);
        let _meteorite = init.meteorite.evaluate(&mut mirror);
        let _volcano = init.volcano.evaluate(&mut mirror);
        let _earthquake = init.earthquake.evaluate(&mut mirror);

        engine
            .apply_weather_init(&init)
            .expect("weather init applies");

        assert_eq!(engine.rng, mirror, "LaunchCloud adds no RNG draws");
        let clouds: Vec<_> = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "FXP1")
            .collect();
        assert_eq!(clouds.len(), 2, "GBackWdt 1100 launches two clouds");
        for (cloud, (x, width, strength)) in clouds.into_iter().zip(expected) {
            assert_eq!(cloud.state.position, Vector2::new(x, -2));
            assert_eq!(cloud.state.owner, OWNER_NONE);
            assert_eq!(cloud.state.controller, OWNER_NONE);
            assert_eq!(cloud.state.category, CATEGORY_VEHICLE);
            assert_eq!(cloud.state.construction, FULL_CON);
            assert_eq!(cloud.state.status, ObjectStatus::Normal);
            assert_eq!(cloud.state.action.name, "Process");
            assert_eq!((cloud.state.action.phase, cloud.state.action.ticks), (0, 0));
            assert_eq!(
                cloud.state.local_vars.get("iMat"),
                Some(&Value::Nil),
                "material index zero is normalized at the pre-strict-3 Activate boundary"
            );
            assert_eq!(
                cloud.state.local_vars.get("iLength"),
                Some(&Value::Int(width))
            );
            assert_eq!(
                cloud.state.local_vars.get("iStrength"),
                Some(&Value::Int(strength))
            );
            assert_eq!(
                cloud.state.local_vars.get("iMovement"),
                Some(&Value::Int(1))
            );
            assert_eq!(cloud.fixed_velocity.x, math::fixed10(37));
            assert_eq!(cloud.fixed_velocity.y, C4Fixed::ZERO);
            assert!(cloud.state.mobile, "SetXDir mobilizes the cloud");
        }
    }

    #[test]
    fn weather_init_missing_precipitation_material_consumes_draws_without_spawning() {
        // LaunchCloud resolves the material before CreateObject and returns
        // false for MNone (C4Weather.cpp:205-208). Its argument draws have
        // already happened, but no object number is consumed.
        let mut engine = Engine::with_seed(17);
        engine.set_landscape(Landscape::flat(1_100, 100));
        engine
            .register_definition(test_precipitation_definition())
            .expect("cloud registers");
        let init = fixed_weather_init(77, 37, "MissingRain");
        let before_number = engine.next_object_id;

        engine
            .apply_weather_init(&init)
            .expect("weather init applies");

        assert!(engine.objects.is_empty());
        assert_eq!(engine.next_object_id, before_number);
    }

    #[test]
    fn snapshot_reports_environment_metrics() {
        let mut engine = Engine::with_seed(15);
        let environment = EnvironmentSettings::new(4)
            .with_wind_variation(6, 8)
            .with_temperature(12)
            .with_climate(-4)
            .with_temperature_cycle(6, 16, 5)
            .with_time_of_day(900)
            .with_time_speed(30)
            .with_precipitation(-45)
            .with_sky_color(RgbColor::new(24, 48, 192));
        engine.set_environment(environment);

        let snapshot = engine.snapshot();
        assert_eq!(snapshot.environment.settings, environment);
        assert_eq!(snapshot.environment.wind_force, environment.wind_force(0));
        assert_eq!(
            snapshot.environment.ambient_temperature,
            environment.ambient_temperature(0)
        );
        assert_eq!(
            snapshot.environment.precipitation,
            environment.precipitation()
        );
        assert_eq!(snapshot.environment.sky_color, environment.sky_color());
    }

    #[test]
    fn environment_sky_color_can_be_configured_and_cleared() {
        let configured = EnvironmentSettings::new(0).with_sky_color(RgbColor::new(5, 10, 15));
        assert_eq!(configured.sky_color(), Some(RgbColor::new(5, 10, 15)));

        let cleared = configured.without_sky_color();
        assert!(cleared.sky_color().is_none());
    }

    #[test]
    fn resolved_sky_color_reflects_time_and_temperature() {
        let midnight = EnvironmentSettings::new(0).with_time_of_day(0);
        let midnight_color = midnight.resolved_sky_color(midnight.ambient_temperature(0));

        let midday = midnight.with_time_of_day(1200);
        let midday_color = midday.resolved_sky_color(midday.ambient_temperature(0));

        assert!(
            midday_color.r > midnight_color.r
                && midday_color.g > midnight_color.g
                && midday_color.b > midnight_color.b,
            "daylight should brighten sky color"
        );

        let cold = midday.with_temperature(-40);
        let warm = midday.with_temperature(40);
        let cold_color = cold.resolved_sky_color(cold.ambient_temperature(0));
        let warm_color = warm.resolved_sky_color(warm.ambient_temperature(0));

        assert!(
            warm_color.r >= cold_color.r,
            "warmer temperatures should not reduce red channel"
        );
        assert!(
            warm_color.b >= cold_color.b,
            "warmer temperatures should not reduce blue channel"
        );
    }

    #[test]
    fn environment_time_advances_each_tick() {
        let mut engine = Engine::with_seed(7);
        engine.set_environment(
            EnvironmentSettings::new(0)
                .with_time_of_day(2300)
                .with_time_speed(75),
        );

        assert_eq!(engine.environment().time_of_day, 2300);

        engine.tick_without_snapshot().expect("first tick succeeds");
        assert_eq!(engine.environment().time_of_day, 2375);

        engine.tick_without_snapshot().expect("second tick succeeds");
        assert_eq!(engine.environment().time_of_day, 50);
    }

    #[test]
    fn precipitation_clamps_to_range() {
        let wet = EnvironmentSettings::new(0).with_precipitation(140);
        assert_eq!(wet.precipitation(), 100);

        let balanced = EnvironmentSettings::new(0).with_precipitation(42);
        assert_eq!(balanced.precipitation(), 42);

        let dry = EnvironmentSettings::new(0).with_precipitation(-180);
        assert_eq!(dry.precipitation(), -100);
    }

    #[test]
    fn lightning_event_initializes_at_cpp_default_position_before_activate() {
        // C4Weather::LaunchLightning creates FXL1 without x/y arguments and
        // only passes the requested position to Activate afterwards
        // (C4Weather.cpp:158-168). Initialize therefore observes the default
        // position (50,50), and a no-op Activate leaves the object there.
        let script = r#"
        func Initialize(state, random) { return 0; }
        func Step(state, frame, random) { return 0; }
        func Activate(x, y, xdir, xrange, ydir, yrange, gamma)
        {
            MissingWeatherCallback();
            return true;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FXL1", "Lightning", script).expect("definition builds"),
            )
            .expect("definition registers");

        assert!(
            engine
                .trigger_lightning(120)
                .expect("C++ fail-safe lightning callback keeps the trigger running"),
            "lightning definition should spawn effect"
        );

        let index = engine
            .objects
            .iter()
            .position(|object| object.definition_id == "FXL1")
            .expect("lightning effect spawned");
        assert_eq!(
            engine.objects[index].state.position,
            Vector2::new(50, 50),
            "C++ creates FXL1 at the native default position before Activate"
        );
    }

    #[test]
    fn lightning_weather_launch_succeeds_without_fxl1_like_cpp() {
        // C4Weather::LaunchLightning returns true even when
        // Game.CreateObject(FXL1) returns null, so the successful weather
        // event is still recorded (C4Weather.cpp:153-165).
        let mut engine = Engine::with_seed(7);
        engine.set_landscape(Landscape::flat(64, 40));
        let mut environment = engine.environment();
        environment.lightning = 100;
        engine.set_environment(environment);

        for frame in (10..=20_000).step_by(10) {
            engine
                .tick_weather_events(frame)
                .expect("weather tick succeeds");
            if engine
                .snapshot()
                .weather_events
                .iter()
                .any(|event| matches!(event, WeatherEvent::Lightning { .. }))
            {
                return;
            }
        }
        panic!("the C++-successful missing-FXL1 launch should be recorded");
    }

    #[test]
    fn launch_lightning_script_host_forwards_creatorless_exact_arguments() {
        // FnLaunchLightning forwards its six integer arguments verbatim,
        // normalizes gamma to bool, and ignores both missing definitions and
        // fail-safe Activate errors. C4Weather creates FXL1 without inheriting
        // caller owner/controller/layer at native position (50,50).
        let caller_script = r#"#strict
local first, second;
func Trigger()
{
    first = LaunchLightning(-7, 8, -9, 10, -11, 12, 42, 999);
    second = LaunchLightning(1, 2, 3, 4, 5, 6);
    return(first && second);
}
"#;
        let lightning_script = r#"#strict
local construction_creator, construction_x, construction_y;
local seen_x, seen_y, seen_xdir, seen_xrange, seen_ydir, seen_yrange, seen_gamma, touched;
func Construction(object creator)
{
    construction_creator = creator;
    construction_x = GetX();
    construction_y = GetY();
}
func Activate(int x, int y, int xdir, int xrange, int ydir, int yrange, bool gamma)
{
    seen_x = x; seen_y = y; seen_xdir = xdir; seen_xrange = xrange;
    seen_ydir = ydir; seen_yrange = yrange; seen_gamma = gamma; touched = 1;
    if (x < 0) MissingLightningCallback();
    return(false);
}
"#;

        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(
                Definition::from_script("LAYR", "Layer", "#strict\n")
                    .expect("layer compiles"),
            )
            .expect("layer registers");
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("FXL1", "Lightning", lightning_script)
                    .expect("lightning compiles"),
            )
            .expect("lightning registers");
        let layer = engine
            .spawn_object(SpawnConfig::new("LAYR"))
            .expect("layer spawns");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_position(Vector2::new(91, 82))
                    .with_owner(3)
                    .with_controller(7)
                    .with_layer(layer),
            )
            .expect("caller spawns");
        let rng_count_before = engine.debug_rng_clone().count;
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(caller_index, "Trigger", Vec::new())
                .expect("LaunchLightning remains fail-safe"),
            Value::Bool(true)
        );
        assert_eq!(
            engine.debug_rng_clone().count,
            rng_count_before,
            "native LaunchLightning draws no synchronized RNG itself"
        );
        let caller_state = engine.object_snapshot(caller).expect("caller survives");
        assert_eq!(caller_state.local_vars.get("first"), Some(&Value::Int(1)));
        assert_eq!(caller_state.local_vars.get("second"), Some(&Value::Int(1)));

        let lightning = engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "FXL1")
            .collect::<Vec<_>>();
        assert_eq!(lightning.len(), 2);
        for object in &lightning {
            assert_eq!(object.state.position, Vector2::new(50, 50));
            assert_eq!(object.state.owner, OWNER_NONE);
            assert_eq!(object.state.controller, OWNER_NONE);
            assert_eq!(object.state.layer, None);
            assert_eq!(
                object.state.local_vars.get("construction_creator"),
                Some(&Value::Nil)
            );
            assert_eq!(
                object.state.local_vars.get("construction_x"),
                Some(&Value::Int(50))
            );
            assert_eq!(
                object.state.local_vars.get("construction_y"),
                Some(&Value::Int(50))
            );
            assert_eq!(object.state.local_vars.get("touched"), Some(&Value::Int(1)));
        }
        let first = lightning[0];
        for (name, value) in [
            ("seen_x", -7),
            ("seen_y", 8),
            ("seen_xdir", -9),
            ("seen_xrange", 10),
            ("seen_ydir", -11),
            ("seen_yrange", 12),
        ] {
            assert_eq!(first.state.local_vars.get(name), Some(&Value::Int(value)));
        }
        assert_eq!(
            first.state.local_vars.get("seen_gamma"),
            Some(&Value::Bool(true))
        );
        let second = lightning[1];
        assert_eq!(second.state.local_vars.get("seen_x"), Some(&Value::Int(1)));
        assert_eq!(
            second.state.local_vars.get("seen_gamma"),
            Some(&Value::Nil)
        );
    }

    #[test]
    fn launch_lightning_returns_true_without_fxl1() {
        let caller_script =
            "#strict\nfunc Trigger() { return(LaunchLightning()); }\n";
        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let rng_count_before = engine.debug_rng_clone().count;
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        assert_eq!(
            engine
                .call_object_function(caller_index, "Trigger", Vec::new())
                .expect("missing FXL1 remains successful"),
            Value::Int(1)
        );
        assert_eq!(engine.debug_rng_clone().count, rng_count_before);
        assert_eq!(engine.objects.len(), 1);
    }

    #[test]
    fn volcano_event_initializes_at_cpp_default_position_before_activate() {
        // C4Weather::LaunchVolcano likewise creates FXV1 at the native
        // default (50,50) and supplies coordinates only to Activate.
        let script = r#"
        func Initialize(state, random) { return 0; }
        func Step(state, frame, random) { return 0; }
        func Activate(x, y, size, material)
        {
            MissingWeatherCallback();
            return true;
        }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FXV1", "Volcano", script).expect("definition builds"),
            )
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(64, 40));
        let mut environment = engine.environment();
        environment.volcano = 100;
        engine.set_environment(environment);

        for frame in (10..=20_000).step_by(10) {
            engine
                .tick_weather_events(frame)
                .expect("weather tick succeeds");
            if let Some(volcano) = engine
                .objects
                .iter()
                .find(|object| object.definition_id == "FXV1")
            {
                assert_eq!(
                    volcano.state.position,
                    Vector2::new(50, 50),
                    "C++ creates FXV1 at the native default position before Activate"
                );
                return;
            }
        }
        panic!("seed should launch a volcano in the bounded weather sweep");
    }

    #[test]
    fn volcano_weather_launch_succeeds_without_fxv1_like_cpp() {
        // C4Weather::LaunchVolcano returns true unconditionally even if
        // Game.CreateObject(FXV1) returns null (C4Weather.cpp:178-184).
        let mut engine = Engine::with_seed(7);
        engine.set_landscape(Landscape::flat(64, 40));
        let mut environment = engine.environment();
        environment.volcano = 100;
        engine.set_environment(environment);

        for frame in (10..=20_000).step_by(10) {
            engine
                .tick_weather_events(frame)
                .expect("weather tick succeeds");
            if engine
                .snapshot()
                .weather_events
                .iter()
                .any(|event| matches!(event, WeatherEvent::Volcano { .. }))
            {
                return;
            }
        }
        panic!("the C++-successful missing-FXV1 launch should be recorded");
    }

    #[test]
    fn launch_volcano_script_host_uses_cpp_single_x_parameter_contract() {
        // Native FnLaunchVolcano takes only x, so any additional y,
        // thickness, and material values reaching it are ignored. It calls
        // C4Weather::LaunchVolcano(Lava, x, GBackHgt - 1,
        // BoundBy(15 * GBackHgt / 500 + Random(10), 10, 60)), whose FXV1
        // creation has no creator and whose fail-safe Activate result is
        // ignored (C4Script.cpp:3086-3093; C4Weather.cpp:178-184).
        let caller_script = r#"#strict
local result;
func Trigger() {
    result = LaunchVolcano(37, 123, 456, "Acid");
    return(result);
}
"#;
        let volcano_script = r#"#strict
local construction_creator, seen_x, seen_y, seen_size, seen_material;
func Construction(creator) { construction_creator = creator; }
func Activate(x, y, size, material) {
    seen_x = x;
    seen_y = y;
    seen_size = size;
    seen_material = material;
    return(false);
}
"#;
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100

            [Material Lava]
            Name=Lava
            Density=50
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let lava = materials.id_of("Lava").expect("Lava exists");
        let mut engine = Engine::with_seed(7);
        engine.set_materials(materials);
        let mut landscape = Landscape::flat(64, 40);
        landscape.set_world_height(500);
        engine.set_landscape(landscape);
        engine
            .register_definition(
                Definition::from_script("FXV1", "Volcano", volcano_script)
                    .expect("volcano compiles"),
            )
            .expect("volcano registers");
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");

        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(91, 82))
                    .with_owner(3),
            )
            .expect("caller spawns");
        let mut expected_rng = engine.debug_rng_clone();
        let expected_size = (15 + expected_rng.random(10)).clamp(10, 60);
        let rng_count_before = engine.debug_rng_clone().count;
        let caller_index = engine.find_object_index(caller).expect("caller exists");
        let result = engine
            .call_object_function(caller_index, "Trigger", Vec::new())
            .expect("LaunchVolcano succeeds");

        assert_eq!(result, Value::Int(1), "Activate(false) is ignored");
        assert_eq!(
            engine.debug_rng_clone().count,
            rng_count_before + 1,
            "the bounded size consumes exactly Random(10)"
        );
        let volcano = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "FXV1")
            .expect("FXV1 spawns");
        assert_eq!(volcano.state.position, Vector2::new(50, 50));
        assert_eq!(volcano.state.owner, OWNER_NONE);
        assert_eq!(
            volcano.state.local_vars.get("construction_creator"),
            Some(&Value::Nil),
            "C4Weather creates FXV1 without a creator"
        );
        assert_eq!(volcano.state.local_vars.get("seen_x"), Some(&Value::Int(37)));
        assert_eq!(
            volcano.state.local_vars.get("seen_y"),
            Some(&Value::Int(499)),
            "the extra y argument is ignored in favor of GBackHgt - 1"
        );
        assert_eq!(
            volcano.state.local_vars.get("seen_size"),
            Some(&Value::Int(expected_size)),
            "the extra thickness argument is ignored"
        );
        assert_eq!(
            volcano.state.local_vars.get("seen_material"),
            Some(&Value::Int(lava.index() as i32)),
            "the extra material argument is ignored in favor of Lava"
        );
    }

    #[test]
    fn launch_volcano_returns_true_and_draws_size_without_fxv1() {
        // C4Weather::LaunchVolcano returns true unconditionally even when
        // Game.CreateObject(FXV1) returns null; FnLaunchVolcano evaluates the
        // bounded Random(10) size first (C4Script.cpp:3086-3093;
        // C4Weather.cpp:178-184).
        let caller_script = r#"#strict
func Trigger() { return(LaunchVolcano(12)); }
"#;
        let mut engine = Engine::with_seed(9);
        let mut landscape = Landscape::flat(32, 20);
        landscape.set_world_height(300);
        engine.set_landscape(landscape);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
            .expect("caller spawns");
        let rng_count_before = engine.debug_rng_clone().count;
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(caller_index, "Trigger", Vec::new())
                .expect("missing FXV1 is fail-safe"),
            Value::Int(1)
        );
        assert_eq!(engine.debug_rng_clone().count, rng_count_before + 1);
        assert_eq!(engine.objects.len(), 1, "missing FXV1 creates no object");
    }

    #[test]
    fn launch_earthquake_script_host_creates_creatorless_fxq1_and_tolerates_activate_error() {
        // FnLaunchEarthquake forwards only x/y, creates FXQ1 creatorless at
        // that exact position, fail-safe-calls Activate(), and returns void
        // regardless of activation success (C4Script.cpp:3094-3097;
        // C4Weather.cpp:196-203).
        let caller_script = r#"#strict
func Trigger() { return(LaunchEarthquake(-7, 83, 999)); }
"#;
        let earthquake_script = r#"#strict
local construction_creator, construction_x, construction_y;
local activated, activate_argument;
func Construction(object creator) {
    construction_creator = creator;
    construction_x = GetX();
    construction_y = GetY();
}
func Activate(unexpected) {
    activated = 1;
    activate_argument = unexpected;
    MissingEarthquakeCallback();
    return(1);
}
"#;

        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(simple_definition("LAYR"))
            .expect("layer registers");
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("caller compiles"),
            )
            .expect("caller registers");
        engine
            .register_definition(
                Definition::from_script("FXQ1", "Earthquake", earthquake_script)
                    .expect("earthquake compiles"),
            )
            .expect("earthquake registers");
        let layer = engine
            .spawn_object(SpawnConfig::new("LAYR"))
            .expect("layer spawns");
        let caller = engine
            .spawn_object(
                SpawnConfig::new("CALL")
                    .with_position(Vector2::new(91, 82))
                    .with_owner(3)
                    .with_controller(7)
                    .with_layer(layer),
            )
            .expect("caller spawns");
        let rng_count_before = engine.debug_rng_clone().count;
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(caller_index, "Trigger", Vec::new())
                .expect("Activate errors remain fail-safe"),
            Value::Nil
        );
        assert_eq!(engine.debug_rng_clone().count, rng_count_before);
        assert!(engine.object_snapshot(caller).is_some(), "caller survives");

        let quake = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "FXQ1")
            .expect("FXQ1 spawns");
        assert_eq!(quake.state.position, Vector2::new(-7, 83));
        assert_eq!(quake.state.owner, OWNER_NONE);
        assert_eq!(quake.state.controller, OWNER_NONE);
        assert_eq!(quake.state.layer, None);
        assert_eq!(
            quake.state.local_vars.get("construction_creator"),
            Some(&Value::Nil)
        );
        assert_eq!(
            quake.state.local_vars.get("construction_x"),
            Some(&Value::Int(-7))
        );
        assert_eq!(
            quake.state.local_vars.get("construction_y"),
            Some(&Value::Int(83))
        );
        assert_eq!(
            quake.state.local_vars.get("activated"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            quake.state.local_vars.get("activate_argument"),
            Some(&Value::Nil),
            "native Activate receives no arguments"
        );
    }

    #[test]
    fn launch_earthquake_returns_nil_without_fxq1_and_script_continues() {
        let caller_script = r#"#strict
func Trigger() {
    var result = LaunchEarthquake(60, 140);
    return([result, 1]);
}
"#;
        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(
                Definition::from_script("CALL", "Caller", caller_script)
                    .expect("Tutorial06-shaped call compiles"),
            )
            .expect("caller registers");
        let caller = engine
            .spawn_object(SpawnConfig::new("CALL"))
            .expect("caller spawns");
        let rng_count_before = engine.debug_rng_clone().count;
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine
                .call_object_function(caller_index, "Trigger", Vec::new())
                .expect("missing FXQ1 is a successful void call"),
            Value::Array(vec![Value::Nil, Value::Int(1)])
        );
        assert_eq!(engine.debug_rng_clone().count, rng_count_before);
        assert_eq!(engine.objects.len(), 1, "missing FXQ1 creates nothing");
    }

    #[test]
    fn earthquake_event_requires_truthy_activate_like_cpp() {
        // LaunchEarthquake returns true only when FXQ1::Activate returns a
        // truthy C4Value (C4Weather.cpp:196-203). Object creation alone does
        // not make the launch successful.
        let script = r#"
        func Initialize(state, random) { return 0; }
        func Step(state, frame, random) { return 0; }
        func Activate() { return false; }
        "#;

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(
                Definition::from_script("FXQ1", "Earthquake", script)
                    .expect("definition builds"),
            )
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(64, 40));
        let mut environment = engine.environment();
        environment.earthquake = 100;
        engine.set_environment(environment);

        for frame in (10..=20_000).step_by(10) {
            engine
                .tick_weather_events(frame)
                .expect("weather tick succeeds");
            if engine
                .objects
                .iter()
                .any(|object| object.definition_id == "FXQ1")
            {
                assert!(
                    !engine
                        .snapshot()
                        .weather_events
                        .iter()
                        .any(|event| matches!(event, WeatherEvent::Earthquake { .. })),
                    "false Activate rejects the earthquake launch in C++"
                );
                return;
            }
        }
        panic!("seed should reach an earthquake gate in the bounded weather sweep");
    }

