    #[test]
    fn flight_procedure_suppresses_gravity_and_wind() {
        let mut definition = Definition::from_script("Glider", "Glider", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Fly".to_string(), ActionSpec::for_procedure("flight"));
        definition.configure_actions(Some("Fly".to_string()), actions);

        let mut engine = Engine::with_seed(1);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(4, 12, -20)
            .expect("physics settings valid")
            .with_max_horizontal_speed(24)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(5));

        let id = engine
            .spawn_object(SpawnConfig::new("Glider").with_category(CATEGORY_OBJECT))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.velocity.x, 0);
        assert_eq!(
            object
                .fixed_velocity
                .expect("gravity should remain sub-pixel")
                .y
                .val(),
            524
        );
    }

    #[test]
    fn flight_command_direction_updates_velocity() {
        let mut definition = Definition::from_script("Glider", "Glider", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Fly".to_string(), ActionSpec::for_procedure("flight"));
        definition.configure_actions(Some("Fly".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_float_speed(6)
                .with_float_acceleration(3),
        );

        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Glider")
                    .with_category(CATEGORY_OBJECT)
                    .with_command_direction(CommandDirection::DownRight),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        // DFA_FLIGHT is gravity + Mobile only (C4Object.cpp:4875-4886):
        // ComDir never steers a flier, so only GravAccel accumulates.
        assert_eq!(object.velocity, Vector2::new(0, 0));
        assert_eq!(
            object
                .fixed_velocity
                .map(|velocity| velocity.y.val())
                .unwrap_or(0),
            engine.physics.gravity_as_c4fixed().val()
        );
    }

    #[test]
    fn float_procedure_reduces_gravity_pull() {
        let mut definition =
            Definition::from_script("Balloon", "Balloon", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Float".to_string(), ActionSpec::for_procedure("float"));
        definition.configure_actions(Some("Float".to_string()), actions);

        let mut engine = Engine::with_seed(2);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(6, 20, -30)
            .expect("physics settings valid")
            .with_max_horizontal_speed(20)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(SpawnConfig::new("Balloon").with_category(CATEGORY_OBJECT))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        // DFA_FLOAT never runs DoGravity (C4Object.cpp:5268-5290): a
        // floater with no ComDir input holds its velocity exactly.
        assert_eq!(
            object
                .fixed_velocity
                .map(|velocity| velocity.y.val())
                .unwrap_or(0),
            0
        );
    }

    #[test]
    fn float_command_direction_updates_velocity() {
        let mut definition =
            Definition::from_script("Balloon", "Balloon", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Float".to_string(), ActionSpec::for_procedure("float"));
        definition.configure_actions(Some("Float".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_float_speed(6)
                .with_float_acceleration(2),
        );

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Balloon")
                    .with_category(CATEGORY_OBJECT)
                    .with_command_direction(CommandDirection::UpRight),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(2, -2));
        // No gravity rides on DFA_FLOAT (C4Object.cpp:5268-5290): the
        // velocity is exactly the accumulated float acceleration.
        assert_eq!(
            object
                .fixed_velocity
                .map(|velocity| velocity.y.val())
                .unwrap_or(object.velocity.y << 16),
            -131072
        );
    }

    #[test]
    fn float_physical_preserves_hazard_bullet_velocity_above_synthetic_limit() {
        // Hazard's SHT1 Travel action uses DFA_FLOAT with Float=100000. C++
        // clamps xdir/ydir only to FIXED100(Float), then sets Mobile
        // (oracle-src-pinned src/C4Object.cpp:5291-5310); it has no global
        // 12 px/frame cap after the procedure. The raw velocity is a
        // representative Pistol Fire1 launch at 76 degrees.
        let script =
            format!("{PROCEDURE_MOVEMENT_SCRIPT}\nfunc Traveling() {{ return true; }}\n");
        let mut definition =
            Definition::from_script("SHT1", "Shot", &script).expect("script compiles");
        definition.configure_actions(
            Some("Travel".to_string()),
            HashMap::from([(
                "Travel".to_string(),
                ActionSpec::default()
                    .with_procedure("FLOAT")
                    .with_delay(1)
                    .with_length(1)
                    .with_next("Travel")
                    .with_start_call("Traveling"),
            )]),
        );
        definition.set_physical(PhysicalInfo {
            float: 100_000,
            ..PhysicalInfo::default()
        });
        definition.set_incomplete_activity(true);

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let launch_velocity = FixedVec2::new(
            C4Fixed::from_raw(1_592_524),
            C4Fixed::from_raw(-393_216),
        );
        let bullet = engine
            .spawn_object(SpawnConfig::new("SHT1").with_category(CATEGORY_OBJECT))
            .expect("bullet spawns");
        let bullet_idx = engine.find_object_index(bullet).expect("bullet exists");
        engine.objects[bullet_idx].set_fixed_velocity(launch_velocity);
        assert_eq!(
            engine.objects[bullet_idx].fixed_velocity, launch_velocity,
            "script launch keeps raw C4Fixed velocity before ExecAction"
        );
        assert_eq!(engine.objects[bullet_idx].state.action.name, "Travel");

        engine
            .apply_physics_at_index(bullet_idx)
            .expect("DFA_FLOAT executes");

        assert_eq!(
            engine.objects[bullet_idx].fixed_velocity, launch_velocity,
            "DFA_FLOAT must not steepen the bullet by clamping only its horizontal speed"
        );

        engine
            .tick_without_snapshot()
            .expect("the complete object frame executes");
        let bullet_idx = engine.find_object_index(bullet).expect("bullet remains live");
        assert_eq!(
            engine.objects[bullet_idx].fixed_velocity, launch_velocity,
            "callback outcome folds must preserve the same native DFA_FLOAT velocity"
        );
    }

    #[test]
    fn float_callback_uses_same_outcome_physical_before_terminal_clamp() {
        // SetPhysical mutates the live C++ object before the following
        // SetXDir/SetYDir calls return from the callback
        // (oracle-src-pinned src/C4Script.cpp:557-601). DFA_FLOAT then owns
        // the only speed bounds (src/C4Object.cpp:5291-5310).
        let script = format!(
            r#"{PROCEDURE_MOVEMENT_SCRIPT}
global func Step(state, frame, random) {{
    if (frame == 1) {{
        SetPhysical("Float", 100000, 2);
        SetXDir(243, this(), 10);
        SetYDir(-60, this(), 10);
    }}
    return 0;
}}

func ArmBullet() {{
    SetPhysical("Float", 100000, 2);
    SetXDir(243, this(), 10);
    SetYDir(-60, this(), 10);
}}
"#
        );
        let mut definition =
            Definition::from_script("SHT1", "Shot", &script).expect("script compiles");
        definition.configure_actions(
            Some("Travel".to_string()),
            HashMap::from([(
                "Travel".to_string(),
                ActionSpec::default().with_procedure("FLOAT"),
            )]),
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let bullet = engine
            .spawn_object(SpawnConfig::new("SHT1").with_category(CATEGORY_OBJECT))
            .expect("bullet spawns");
        let bullet_idx = engine.find_object_index(bullet).expect("bullet exists");

        engine
            .call_object_function(bullet_idx, "ArmBullet", Vec::new())
            .expect("bullet callback executes");

        assert_eq!(
            engine.objects[bullet_idx]
                .state
                .temporary_physical
                .map(|physical| physical.float),
            Some(100_000)
        );
        assert_eq!(
            (
                engine.objects[bullet_idx].fixed_velocity.x.val(),
                engine.objects[bullet_idx].fixed_velocity.y.val(),
            ),
            (1_592_524, -393_216),
            "the fold must resolve Float after applying the callback's physical update"
        );

        let stepped_bullet = engine
            .spawn_object(SpawnConfig::new("SHT1").with_category(CATEGORY_OBJECT))
            .expect("Step-driven bullet spawns");
        engine
            .tick_without_snapshot()
            .expect("the definition Step callback executes");
        let stepped_idx = engine
            .find_object_index(stepped_bullet)
            .expect("Step-driven bullet exists");
        assert_eq!(
            (
                engine.objects[stepped_idx].fixed_velocity.x.val(),
                engine.objects[stepped_idx].fixed_velocity.y.val(),
            ),
            (1_592_524, -393_216),
            "the Step fold must also resolve Float after its physical update"
        );
    }

    #[test]
    fn swim_procedure_reduces_gravity_and_blocks_wind() {
        let mut definition =
            Definition::from_script("Swimmer", "Swimmer", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Swim".to_string(), ActionSpec::for_procedure("swim"));
        definition.configure_actions(Some("Swim".to_string()), actions);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(6, 20, -30)
            .expect("physics settings valid")
            .with_max_horizontal_speed(20)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(5));

        let id = engine
            .spawn_object(SpawnConfig::new("Swimmer"))
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.velocity.x, 0);
        // DFA_SWIM steers with SwimAccel only — no GravAccel component
        // (C4Object.cpp:4920-4985).
        assert_eq!(
            object
                .fixed_velocity
                .map(|velocity| velocity.y.val())
                .unwrap_or(0),
            0
        );
    }

    #[test]
    fn swim_command_direction_updates_velocity_and_stop_decelerates() {
        let mut definition =
            Definition::from_script("Swimmer", "Swimmer", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Swim".to_string(), ActionSpec::for_procedure("swim"));
        definition.configure_actions(Some("Swim".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_swim_speed(10)
                .with_swim_acceleration(2),
        );

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(0, 20, -20)
            .expect("physics settings valid")
            .with_max_horizontal_speed(20)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Swimmer").with_command_direction(CommandDirection::DownRight),
            )
            .expect("spawn succeeds");

        // C4Object InLiquid: these fixtures have no water — arm the flag
        // so the DFA_SWIM out-of-liquid exit (C4Object.cpp:4946-4956)
        // does not convert the swimmer to Walk.
        {
            let idx = engine.find_object_index(id).expect("swimmer exists");
            engine.objects[idx].state.in_liquid = true;
        }
        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(2, 2));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(4, 4));

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new().with_command_direction(CommandDirection::Stop),
            )
            .expect("update succeeds");

        let snapshot = engine.tick().expect("third tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(2, 2));

        let snapshot = engine.tick().expect("fourth tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(0, 0));
    }

    #[test]
    fn lift_procedure_matches_cpp_mass_scaled_force_and_terminal_speeds() {
        let lifter_definition = build_lift_definition("Lifter");
        let mut crate_definition = build_idle_definition("Crate");
        crate_definition.set_mass(100);

        let mut engine = Engine::with_seed(31);
        engine
            .register_definition(lifter_definition)
            .expect("lifter registers");
        engine
            .register_definition(crate_definition)
            .expect("crate registers");
        // Deliberately tighter than Lift's +/-2 targets: C++ Lift does not
        // apply the generic terminal-speed clamp.
        engine.set_physics(PhysicsSettings::new(20, 1, -1));

        let target_id = engine
            .spawn_object(SpawnConfig::new("Crate").with_category(CATEGORY_OBJECT))
            .expect("target spawns");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        engine.objects[target_idx].set_fixed_velocity(FixedVec2::ZERO);
        engine.objects[target_idx].state.mobile = true;

        let mut lift_action = ActionState::new("Lift");
        lift_action.target = Some(target_id);

        let lifter_id = engine
            .spawn_object(
                SpawnConfig::new("Lifter")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(lift_action)
                    .with_command_direction(CommandDirection::Up),
            )
            .expect("lifter spawns");

        let lifter_idx = engine.find_object_index(lifter_id).expect("lifter exists");
        let target_definition_id = engine.objects[target_idx].definition_id.clone();
        let target_actions = engine
            .definition(&target_definition_id)
            .expect("target definition exists")
            .action_library()
            .clone();
        let mut expected_fix_y = engine.objects[target_idx].fixed_position.y.val();

        // C4Object.cpp:1847-1855,5269-5280: FIXED100(50)*100/Mass=0.5
        // works toward the constant +/-2 target without terminal-speed
        // clamping. Each step is followed by the target's C++ DoMovement
        // integration so both ydir and fix_y are pinned byte-for-byte.
        for expected in [-32_768, -65_536, -98_304, -131_072, -131_072] {
            engine
                .apply_physics_at_index(lifter_idx)
                .expect("upward lift succeeds");
            assert_eq!(engine.objects[target_idx].fixed_velocity.y.val(), expected);
            engine
                .exec_object_movement(
                    target_idx,
                    &target_actions,
                    &target_definition_id,
                    &[],
                )
                .expect("target movement succeeds");
            expected_fix_y += expected;
            assert_eq!(engine.objects[target_idx].fixed_position.y.val(), expected_fix_y);
        }

        engine.objects[lifter_idx].state.command_direction = CommandDirection::Down;
        for expected in [
            -98_304, -65_536, -32_768, 0, 32_768, 65_536, 98_304, 131_072,
        ] {
            engine
                .apply_physics_at_index(lifter_idx)
                .expect("downward lift succeeds");
            assert_eq!(engine.objects[target_idx].fixed_velocity.y.val(), expected);
            engine
                .exec_object_movement(
                    target_idx,
                    &target_actions,
                    &target_definition_id,
                    &[],
                )
                .expect("target movement succeeds");
            expected_fix_y += expected;
            assert_eq!(engine.objects[target_idx].fixed_position.y.val(), expected_fix_y);
        }

        engine.objects[lifter_idx].state.command_direction = CommandDirection::Stop;
        for expected in [98_304, 65_536, 32_768, 0, -2_621] {
            engine
                .apply_physics_at_index(lifter_idx)
                .expect("stopped lift succeeds");
            assert_eq!(engine.objects[target_idx].fixed_velocity.y.val(), expected);
            engine
                .exec_object_movement(
                    target_idx,
                    &target_actions,
                    &target_definition_id,
                    &[],
                )
                .expect("target movement succeeds");
            expected_fix_y += expected;
            assert_eq!(engine.objects[target_idx].fixed_position.y.val(), expected_fix_y);
        }
        // COMD_Stop's target is exactly -GravAccel. The target's own
        // DoGravity therefore cancels it byte-for-byte in the same cycle.
        let held_fix_y = engine.objects[target_idx].fixed_position.y;
        engine
            .apply_physics_at_index(target_idx)
            .expect("target gravity succeeds");
        assert_eq!(engine.objects[target_idx].fixed_velocity.y, C4Fixed::ZERO);
        engine
            .exec_object_movement(
                target_idx,
                &target_actions,
                &target_definition_id,
                &[],
            )
            .expect("held target movement succeeds");
        assert_eq!(
            engine.objects[target_idx].fixed_position.y, held_fix_y,
            "-GravAccel plus DoGravity produces no fix_y movement"
        );

        // A second live mass proves this is the C++ division, not a
        // hard-coded half-pixel step: (Def Mass 100 + OwnMass 100) gives
        // 32768*100/200 = raw 16384.
        let mut heavy_definition = build_idle_definition("Heavy");
        heavy_definition.set_mass(100);
        engine
            .register_definition(heavy_definition)
            .expect("heavy target registers");
        let heavy_id = engine
            .spawn_object(SpawnConfig::new("Heavy").with_position(Vector2::new(7, 9)))
            .expect("heavy target spawns");
        let mut heavy_lift = ActionState::new("Lift");
        heavy_lift.target = Some(heavy_id);
        let heavy_lifter_id = engine
            .spawn_object(
                SpawnConfig::new("Lifter")
                    .with_action(heavy_lift)
                    .with_command_direction(CommandDirection::Up),
            )
            .expect("heavy lifter spawns");
        let heavy_idx = engine.find_object_index(heavy_id).expect("heavy target exists");
        engine.objects[heavy_idx].state.own_mass = 100;
        engine.objects[heavy_idx].state.mobile = false;
        engine.objects[heavy_idx].fixed_velocity =
            FixedVec2::new(itofix(3), itofix(4));
        engine.objects[heavy_idx].fixed_position = FixedVec2::new(
            itofix(7) + fixed100(25),
            itofix(9) + fixed100(25),
        );
        let heavy_lifter_idx = engine
            .find_object_index(heavy_lifter_id)
            .expect("heavy lifter exists");
        engine
            .apply_physics_at_index(heavy_lifter_idx)
            .expect("heavy lift succeeds");
        assert_eq!(engine.objects[heavy_idx].fixed_velocity.y.val(), -16_384);
        assert_eq!(engine.objects[heavy_idx].fixed_velocity.x, C4Fixed::ZERO);
        assert_eq!(
            engine.objects[heavy_idx].fixed_position,
            FixedVec2::from_ints(7, 9)
        );
        assert!(engine.objects[heavy_idx].state.mobile);
    }

    #[test]
    fn lift_full_tick_matches_cpp_exec_order_for_raw_ydir_and_fix_y() {
        let lifter_definition = build_lift_definition("Lifter");
        let mut target_definition = build_idle_definition("Crate");
        target_definition.set_mass(100);
        let mut landscape = vehicle_grid_landscape(200, 200);
        landscape.set_world_height(200);

        let mut engine = Engine::with_seed(31);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(20, 20, -20));
        engine
            .register_definition(lifter_definition)
            .expect("lifter registers");
        engine
            .register_definition(target_definition)
            .expect("target registers");

        // Loaded objects execute in file order. The target therefore runs
        // DoGravity+DoMovement before the lifter applies its 0.5 force,
        // exactly matching C4Game::ExecObjects for this two-object oracle.
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 100))
                    .with_fixed_position(FixedVec2::from_ints(50, 100))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("target spawns");
        let mut lift = ActionState::new("Lift");
        lift.target = Some(target_id);
        engine
            .spawn_object(
                SpawnConfig::new("Lifter")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 120))
                    .with_action(lift)
                    .with_command_direction(CommandDirection::Up)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("lifter spawns");

        let initial_fix_y = itofix(100).val();
        for (expected_ydir, expected_fix_delta) in [
            (-30_147, 2_621),
            (-60_294, -24_905),
            (-90_441, -82_578),
            (-120_588, -170_398),
            (-131_072, -288_365),
        ] {
            engine.tick_without_snapshot().expect("full lift frame succeeds");
            let target_idx = engine.find_object_index(target_id).expect("target exists");
            assert_eq!(
                engine.objects[target_idx].fixed_velocity.y.val(),
                expected_ydir
            );
            assert_eq!(
                engine.objects[target_idx].fixed_position.y.val(),
                initial_fix_y + expected_fix_delta
            );
        }
    }

    #[test]
    fn lift_contact_reports_stuck_except_for_gravity_hold() {
        let lifter_definition = build_lift_definition("Lifter");
        let target_script = r#"#strict
local stuck_calls;
func Stuck()
{
    stuck_calls = stuck_calls + 1;
}
"#;
        let mut target_definition =
            Definition::from_script("Crate", "Crate", target_script).expect("script compiles");
        target_definition.set_mass(100);
        target_definition
            .set_shape_vertices(vec![ObjectVertex::new(8, 0).with_cnat(CNAT_RIGHT)]);
        target_definition.set_contact_density(50);

        let mut landscape = vehicle_grid_landscape(32, 32);
        landscape.set_world_height(32);
        landscape.grid_write_byte(16, 10, 1);

        let mut engine = Engine::with_seed(31);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(20, 20, -20));
        engine
            .register_definition(lifter_definition)
            .expect("lifter registers");
        engine
            .register_definition(target_definition)
            .expect("target registers");
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_position(Vector2::new(8, 10))
                    .with_fixed_position(FixedVec2::from_ints(8, 10)),
            )
            .expect("target spawns");
        let mut lift = ActionState::new("Lift");
        lift.target = Some(target_id);
        let lifter_id = engine
            .spawn_object(
                SpawnConfig::new("Lifter")
                    .with_action(lift)
                    .with_command_direction(CommandDirection::Up),
            )
            .expect("lifter spawns");
        let lifter_idx = engine.find_object_index(lifter_id).expect("lifter exists");
        let target_idx = engine.find_object_index(target_id).expect("target exists");

        // Unlike Push, Lift runs ContactCheck on every non-hold call
        // (C4Object.cpp:1856-1862), with no Tick35 gate.
        engine
            .apply_physics_at_index(lifter_idx)
            .expect("upward lift succeeds");
        assert_eq!(engine.objects[target_idx].frame_t_contact, CNAT_RIGHT);
        assert_eq!(
            engine.objects[target_idx]
                .state
                .local_vars
                .get("stuck_calls"),
            Some(&Value::Int(1))
        );
        let message = engine
            .snapshot()
            .hud
            .messages
            .into_iter()
            .next()
            .expect("stuck message emitted");
        assert_eq!(message.kind, MessageKind::Target);
        assert_eq!(message.target, Some(target_id));
        assert_eq!(message.lines, vec!["Crate is stuck!"]);
        let message_id = message.id;

        // The exact -GravAccel hold bypasses ContactCheck altogether.
        engine.objects[target_idx].frame_t_contact = CNAT_LEFT;
        engine.objects[lifter_idx].state.command_direction = CommandDirection::Stop;
        engine
            .apply_physics_at_index(lifter_idx)
            .expect("stopped lift succeeds");
        assert_eq!(engine.objects[target_idx].frame_t_contact, CNAT_LEFT);
        assert_eq!(
            engine.objects[target_idx]
                .state
                .local_vars
                .get("stuck_calls"),
            Some(&Value::Int(1))
        );
        let messages = engine.snapshot().hud.messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].id, message_id,
            "gravity hold must not replace the existing target message"
        );
    }

    #[test]
    fn lift_top_callback_runs_on_lifter_before_its_gravity() {
        let lifter_script = r#"#strict
local lift_top_calls, lift_top_seen_time, lift_top_seen_y_dir, lift_top_reflected;
func LiftTop()
{
    lift_top_calls = lift_top_calls + 1;
    lift_top_seen_time = GetActTime();
    lift_top_seen_y_dir = GetYDir();
    lift_top_reflected = GetDefCoreVal("LiftTop", "DefCore", LIFT);
    SetGravity(40);
    SetYDir(5);
}
"#;
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Lift.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=LIFT\nName=Lifter\nCategory=C4D_Object\nLiftTop=20\n",
        )
        .expect("write DefCore");
        std::fs::write(def_dir.join("Script.c"), lifter_script).expect("write Script.c");
        std::fs::write(
            def_dir.join("ActMap.txt"),
            b"[Action]\nName=Lift\nProcedure=LIFT\nLength=1\nNextAction=Lift\n",
        )
        .expect("write ActMap.txt");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load definition resource");
        let lifter_definition =
            Definition::from_resource(&resource).expect("compile resource definition");
        assert_eq!(lifter_definition.lift_top(), 20);
        let mut legacy_definition =
            Definition::from_script("LEGC", "Legacy lifter", "#strict")
                .expect("compile legacy definition");
        Engine::apply_resource_core(&mut legacy_definition, &resource.core);
        assert_eq!(
            legacy_definition.lift_top(),
            20,
            "legacy scenario core mapping retains LiftTop"
        );
        let mut target_definition = build_idle_definition("Crate");
        target_definition.set_mass(100);

        let mut engine = Engine::with_seed(31);
        engine.set_physics(PhysicsSettings::new(20, 20, -20));
        engine
            .register_definition(lifter_definition)
            .expect("lifter registers");
        engine
            .register_definition(target_definition)
            .expect("target registers");
        let target_id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(10, 31)))
            .expect("target spawns");
        let mut lift = ActionState::new("Lift");
        lift.target = Some(target_id);
        let lifter_id = engine
            .spawn_object(
                SpawnConfig::new("LIFT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_action(lift)
                    .with_command_direction(CommandDirection::Up)
                    .with_mobile(true),
            )
            .expect("lifter spawns");
        let lifter_idx = engine.find_object_index(lifter_id).expect("lifter exists");

        // One pixel outside the inclusive Def->LiftTop threshold must not
        // call the hook even though the command direction is Up.
        engine
            .apply_physics_at_index(lifter_idx)
            .expect("out-of-range lift succeeds");
        assert_eq!(
            engine.objects[lifter_idx]
                .state
                .local_vars
                .get("lift_top_calls"),
            None
        );

        let target_idx = engine.find_object_index(target_id).expect("target exists");
        engine.objects[target_idx].state.position.y = 30;
        engine.objects[target_idx].fixed_position.y = itofix(30);
        engine.objects[lifter_idx].set_fixed_velocity(FixedVec2::ZERO);

        // Inclusive boundary and order from C4Object.cpp:5281-5289:
        // Action.Time has already advanced; LiftTop sees pre-gravity ydir,
        // changes Gravity to 40 and sets ydir=fixed10(5), then DoGravity
        // consumes the NEW raw GravAccel 5242 in this same call.
        engine
            .apply_physics_at_index(lifter_idx)
            .expect("lift succeeds");
        assert_eq!(
            engine.objects[lifter_idx]
                .state
                .local_vars
                .get("lift_top_calls"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine.objects[lifter_idx]
                .state
                .local_vars
                .get("lift_top_seen_time"),
            Some(&Value::Int(2))
        );
        assert_eq!(
            engine.objects[lifter_idx]
                .state
                .local_vars
                .get("lift_top_seen_y_dir"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            engine.objects[lifter_idx]
                .state
                .local_vars
                .get("lift_top_reflected"),
            Some(&Value::Int(20))
        );
        assert_eq!(
            engine.objects[lifter_idx].fixed_velocity.y.val(),
            math::fixed10(5).val() + 5_242
        );

        engine.objects[lifter_idx].state.command_direction = CommandDirection::Down;
        engine
            .apply_physics_at_index(lifter_idx)
            .expect("downward lift succeeds");
        assert_eq!(
            engine.objects[lifter_idx]
                .state
                .local_vars
                .get("lift_top_calls"),
            Some(&Value::Int(1)),
            "the height alone does not fire LiftTop while moving down"
        );

        engine.objects[lifter_idx].state.command_direction = CommandDirection::Up;
        engine
            .apply_physics_at_index(lifter_idx)
            .expect("second upward lift succeeds");
        assert_eq!(
            engine.objects[lifter_idx]
                .state
                .local_vars
                .get("lift_top_calls"),
            Some(&Value::Int(2)),
            "LiftTop is level-triggered on every qualifying frame"
        );
    }

    #[test]
    fn lift_procedure_resets_without_target() {
        let lifter_definition = build_lift_definition("Lifter");

        let mut engine = Engine::with_seed(37);
        engine
            .register_definition(lifter_definition)
            .expect("definition registers");

        let lifter_id = engine
            .spawn_object(
                SpawnConfig::new("Lifter")
                    .with_action(ActionState::new("Lift"))
                    .with_command_direction(CommandDirection::Up),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let lifter = snapshot.object(lifter_id).expect("lifter present");
        assert_eq!(lifter.action.name, "Idle");
        assert!(lifter.action.target.is_none());

        // Action.Time++ precedes the DFA_LIFT switch. If NoOtherAction
        // rejects SetAction(Idle), C++ returns with the increment retained.
        let mut locked_definition =
            Definition::from_script("LockedLift", "LockedLift", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        locked_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Lift".to_string(),
                    ActionSpec::default()
                        .with_procedure("lift")
                        .with_no_other_action(true),
                ),
            ]),
        );
        let mut locked_engine = Engine::with_seed(37);
        locked_engine
            .register_definition(locked_definition)
            .expect("locked definition registers");
        let locked_id = locked_engine
            .spawn_object(SpawnConfig::new("LockedLift").with_action(ActionState::new("Lift")))
            .expect("locked lifter spawns");
        let locked_idx = locked_engine
            .find_object_index(locked_id)
            .expect("locked lifter exists");
        assert!(
            locked_engine
                .apply_physics_at_index(locked_idx)
                .expect("invalid locked lift resolves")
        );
        assert_eq!(locked_engine.objects[locked_idx].state.action.name, "Lift");
        assert_eq!(locked_engine.objects[locked_idx].state.action.time, 1);
    }

    #[test]
    fn hang_procedure_locks_vertical_velocity() {
        let mut definition =
            Definition::from_script("Clinger", "Clinger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Hang".to_string(), ActionSpec::for_procedure("hang"));
        definition.configure_actions(Some("Hang".to_string()), actions);

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let physics = PhysicsSettings::checked(6, 20, -30)
            .expect("physics settings valid")
            .with_max_horizontal_speed(20)
            .expect("horizontal speed valid");
        engine.set_physics(physics);
        engine.set_environment(EnvironmentSettings::new(4));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Clinger")
                    .with_velocity(Vector2::new(1, 5))
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.velocity.x, 0);
    }

    #[test]
    fn set_bridge_action_data_updates_action_data() {
        let mut definition =
            Definition::from_script("Bridger", "Bridger", SET_BRIDGE_ACTION_DATA_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Bridge".to_string(), ActionSpec::for_procedure("bridge"));
        definition.configure_actions(Some("Bridge".to_string()), actions);

        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Bridger").with_action(ActionState::new("Bridge")))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.energy, 1);
        // The fixture has no loaded materials, so C4Action::SetBridgeData
        // clamps material 7 through Num-1 (-1) to the 0xff sentinel.
        let expected = encode_bridge_action_data(200, true, false, -1);
        assert_eq!(snapshot.action.data, expected);
    }

    #[test]
    fn set_bridge_action_data_returns_false_when_not_in_bridge_procedure() {
        let mut definition = Definition::from_script(
            "IdleActor",
            "IdleActor",
            SET_BRIDGE_ACTION_DATA_FAILURE_SCRIPT,
        )
        .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(41);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("IdleActor"))
            .expect("spawn succeeds");

        let snapshot = engine
            .object_snapshot(id)
            .expect("object snapshot available");
        assert_eq!(snapshot.energy, 0);
        assert_eq!(snapshot.action.data, 0);
    }

    #[test]
    fn bridge_procedure_freezes_velocity_and_ignores_wind() {
        let mut definition =
            Definition::from_script("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Bridge".to_string(), ActionSpec::for_procedure("bridge"));
        definition.configure_actions(Some("Bridge".to_string()), actions);

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(6));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Bridger")
                    .with_velocity(Vector2::new(8, -3))
                    .with_action(ActionState::new("Bridge")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
    }

    fn wall_bridge_test_engine(blocker: Option<(usize, usize)>) -> (Engine, MaterialId) {
        let mut definition =
            Definition::from_script("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        definition.set_shape_rect(Some(DefinitionRect::new(-5, -10, 10, 20)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        definition.set_contact_density(50);
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Bridge".to_string(),
                    ActionSpec::default().with_procedure("BRIDGE"),
                ),
            ]),
        );

        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut bytes = vec![0; 160 * 160];
        if let Some((x, y)) = blocker {
            bytes[y * 160 + x] = 1;
        }
        let grid = landscape::PixelGrid::new(
            160,
            160,
            bytes,
            vec![0, 80],
            vec![None, Some("Earth".into())],
            vec![None; 2],
        );
        let mut landscape = Landscape::new(160, vec![160; 160]).expect("landscape constructs");
        landscape.set_world_height(160);
        landscape.set_pixel_grid(grid);

        let mut engine = Engine::with_seed(31);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        (engine, earth)
    }

    #[test]
    fn wall_left_bridge_forces_stationary_progression() {
        // DoBridge locally clears fMoveClonk for wall-Left before calculating
        // dt, checking contact, or calling MovePosition (C4Object.cpp:4587-4590,
        // 4606,4629,4651). The stored action-data bit remains set.
        let (mut engine, earth) = wall_bridge_test_engine(Some((100, 79)));
        let encoded = encode_bridge_action_data(100, true, true, earth.index() as i32);
        let mut action = ActionState::new("Bridge");
        action.data = encoded;
        action.time = 3;
        let id = engine
            .spawn_object(
                SpawnConfig::new("Bridger")
                    .with_position(Vector2::new(100, 80))
                    .with_fixed_position(FixedVec2::from_ints(100, 80))
                    .with_command_direction(CommandDirection::Left)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("spawn succeeds");

        let index = engine.find_object_index(id).expect("object index remains");
        // C++ load starts at ActIdle and SetAction(BRIDGE) clears Data when
        // the procedure changes (C4Object.cpp:2867-2877,4106-4114). Stage
        // this running BRIDGE state after loading; the test targets DoBridge,
        // not the save loader.
        engine.objects[index].state.action = action;
        engine
            .apply_physics_at_index(index)
            .expect("bridge procedure succeeds");

        let object = engine.object_snapshot(id).expect("object remains");
        assert_eq!(object.action.time, 4);
        assert_eq!(object.action.data, encoded, "the override is local");
        assert_eq!(object.position, Vector2::new(100, 80));
        assert_eq!(
            engine.objects[index].fixed_position,
            FixedVec2::from_ints(100, 80)
        );
        let landscape = engine.landscape().expect("landscape remains");
        for x in 93..97 {
            assert_eq!(landscape.material_at(x, 89), Some(earth));
            assert_eq!(landscape.material_at(x, 92), None);
        }
    }

    #[test]
    fn moving_wall_up_bridge_preserves_doubled_collision_retry() {
        // Wall-Up is the sole wall arm that keeps fMoveClonk. A blocked first
        // step converts the remaining 95 frames into a stationary 190-frame
        // roof at Action.Time 95 and redraws immediately (C4Object.cpp:4631-4645).
        let (mut engine, earth) = wall_bridge_test_engine(Some((101, 79)));
        let mut action = ActionState::new("Bridge");
        action.data = encode_bridge_action_data(100, true, true, earth.index() as i32);
        action.time = 4;
        let id = engine
            .spawn_object(
                SpawnConfig::new("Bridger")
                    .with_position(Vector2::new(100, 80))
                    .with_fixed_position(FixedVec2::from_ints(100, 80))
                    .with_direction(Direction::Right)
                    .with_command_direction(CommandDirection::Up)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("spawn succeeds");

        let index = engine.find_object_index(id).expect("object index remains");
        // A loaded ActIdle -> BRIDGE transition clears Action.Data in C++;
        // inject the already-running action afterward to isolate the blocked
        // DoBridge retry under test.
        engine.objects[index].state.action = action;
        engine
            .apply_physics_at_index(index)
            .expect("bridge procedure succeeds");

        let object = engine.object_snapshot(id).expect("object remains");
        assert_eq!(object.action.time, 95);
        assert_eq!(
            object.action.data,
            encode_bridge_action_data(190, false, true, earth.index() as i32)
        );
        assert_eq!(object.position, Vector2::new(100, 80));
        assert_eq!(
            engine.objects[index].fixed_position,
            FixedVec2::from_ints(100, 80)
        );
        let landscape = engine.landscape().expect("landscape remains");
        for y in 67..70 {
            for x in 98..102 {
                assert_eq!(landscape.material_at(x, y), Some(earth));
            }
        }
    }

    #[test]
    fn moving_up_left_bridge_uses_action_time_and_draws_cpp_rectangles() {
        // DoBridge (C4Object.cpp:4581-4652): Action.Time has already been
        // incremented when the procedure runs; a moving UpLeft bridge advances
        // at times 6,12,...,96, draws a 4x3 material rect at
        // (x-4, y+Shape.Hgt/2-1), and MovePosition(-1,-1)s the Clonk.
        let mut definition =
            Definition::from_script("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        definition.set_shape_rect(Some(DefinitionRect::new(-5, -10, 10, 20)));
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("Walk".to_string(), ActionSpec::for_procedure("WALK"));
        actions.insert("Bridge".to_string(), ActionSpec::for_procedure("BRIDGE"));
        definition.configure_actions(Some("Idle".to_string()), actions);

        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(17);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let grid = landscape::PixelGrid::new(
            160,
            160,
            vec![0; 160 * 160],
            vec![0, 80],
            vec![None, Some("Earth".into())],
            vec![None; 2],
        );
        let mut landscape = Landscape::new(160, vec![160; 160]).expect("landscape constructs");
        landscape.set_world_height(160);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let mut action = ActionState::new("Bridge");
        action.data = encode_bridge_action_data(100, true, false, earth.index() as i32);

        let id = engine
            .spawn_object(
                SpawnConfig::new("Bridger")
                    .with_position(Vector2::new(100, 80))
                    .with_fixed_position(FixedVec2::from_ints(100, 80))
                    .with_direction(Direction::Right)
                    .with_command_direction(CommandDirection::UpLeft)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("spawn succeeds");
        let index = engine.find_object_index(id).expect("object index remains");
        // Save loading correctly clears BRIDGE data on its DFA_NONE ->
        // DFA_BRIDGE transition. This fixture needs a live, post-transition
        // action so Action.Time drives the C++ DoBridge cadence.
        engine.objects[index].state.action = action;

        for _ in 0..5 {
            engine.tick_without_snapshot().expect("pre-advance tick succeeds");
        }
        let object = engine
            .object_snapshot(id)
            .expect("object remains before first bridge step");
        assert_eq!(object.position, Vector2::new(100, 80));
        assert_eq!(object.action.time, 5);

        engine.tick_without_snapshot().expect("first bridge step succeeds");
        let object = engine
            .object_snapshot(id)
            .expect("object remains after first bridge step");
        assert_eq!(object.position, Vector2::new(99, 79));
        let index = engine.find_object_index(id).expect("object index remains");
        assert_eq!(engine.objects[index].fixed_position, FixedVec2::from_ints(99, 79));
        let landscape = engine.landscape().expect("landscape present");
        for y in 89..92 {
            for x in 96..100 {
                assert_eq!(landscape.material_at(x, y), Some(earth));
            }
        }

        for _ in 6..100 {
            engine.tick_without_snapshot().expect("bridge tick succeeds");
        }

        let object = engine.object_snapshot(id).expect("object present");
        assert_eq!(object.position, Vector2::new(84, 64));
        let index = engine.find_object_index(id).expect("object index remains");
        assert_eq!(engine.objects[index].fixed_position, FixedVec2::from_ints(84, 64));
        assert_eq!(object.direction, Direction::Left);
        assert_eq!(
            object.action.name, "Walk",
            "ObjectActionStand selects Walk even though the fixture default is Idle"
        );
        assert_eq!(object.command_direction, CommandDirection::Stop);
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.action.time, 0);
        let index = engine.find_object_index(id).expect("object index remains");
        assert_eq!(engine.objects[index].frame_t_attach, CNAT_NONE);
        assert_eq!(engine.objects[index].state.t_attach, CNAT_NONE);
    }

    #[test]
    fn blocked_moving_bridge_retries_stationary_and_preserves_ift() {
        // DoBridge's moving collision arm (C4Object.cpp:4631-4646) tests the
        // candidate one pixel upward, converts the remaining duration to a
        // stationary bridge, resets Action.Time to zero, and recursively draws
        // that frame. DrawMaterialRect keeps the destination IFT bit
        // (C4Landscape.cpp:1064-1072).
        let mut definition =
            Definition::from_script("Bridger", "Bridger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        definition.set_shape_rect(Some(DefinitionRect::new(-5, -10, 10, 20)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        definition.set_contact_density(50);
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Bridge".to_string(),
                    ActionSpec::default().with_procedure("BRIDGE"),
                ),
            ]),
        );

        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1

            [Material Granite]
            Name=Granite
            Density=100
            DigFree=0
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut bytes = vec![0; 160 * 160];
        bytes[78 * 160 + 99] = 2; // candidate CheckContact(99, 78)
        bytes[92 * 160 + 93] = 0x80; // first stationary bridge pixel
        let grid = landscape::PixelGrid::new(
            160,
            160,
            bytes,
            vec![0, 80, 100],
            vec![None, Some("Earth".into()), Some("Granite".into())],
            vec![None; 3],
        );
        let mut landscape = Landscape::new(160, vec![160; 160]).expect("landscape constructs");
        landscape.set_world_height(160);
        landscape.set_pixel_grid(grid);

        let mut engine = Engine::with_seed(19);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let mut action = ActionState::new("Bridge");
        action.data = encode_bridge_action_data(100, true, false, earth.index() as i32);
        let id = engine
            .spawn_object(
                SpawnConfig::new("Bridger")
                    .with_position(Vector2::new(100, 80))
                    .with_command_direction(CommandDirection::UpLeft)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("spawn succeeds");
        let index = engine.find_object_index(id).expect("object index remains");
        // C4Object::CompileFunc cannot preserve Data while selecting a
        // different procedure from ActIdle. Stage the running action after
        // load so this test begins at the collision arm it verifies.
        engine.objects[index].state.action = action;

        for _ in 0..6 {
            engine.tick_without_snapshot().expect("bridge tick succeeds");
        }

        let object = engine.object_snapshot(id).expect("object present");
        assert_eq!(object.position, Vector2::new(100, 80));
        assert_eq!(object.action.time, 0);
        let retry = BridgeParameters::from_action_data(object.action.data);
        assert_eq!(retry.duration, 94);
        assert!(!retry.move_clonk);
        assert_eq!(
            engine
                .landscape()
                .expect("landscape remains")
                .grid_byte_at(93, 92),
            Some(0x81),
            "stationary retry draws Earth while preserving tunnel IFT"
        );
    }

    #[test]
    fn connect_procedure_freezes_velocity_and_ignores_wind() {
        let mut definition =
            Definition::from_script("Connector", "Connector", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Connect".to_string(), ActionSpec::for_procedure("connect"));
        definition.configure_actions(Some("Connect".to_string()), actions);

        let mut engine = Engine::with_seed(29);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(10));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Connector")
                    .with_velocity(Vector2::new(-7, 4))
                    .with_action(ActionState::new("Connect")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
    }

    #[test]
    fn object_motion_ignores_wind() {
        // C++ wind reaches only PXS and particles via GBackWind
        // (C4PXS.cpp:67, C4Particles.cpp:652, C4Wrappers.h:189-192) —
        // nothing in C4Movement.cpp/C4Object.cpp ever applies the weather
        // wind to object velocities.
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let definition = Definition::from_script("Crate", "Crate", script).unwrap();

        let mut engine = Engine::with_seed(4);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.set_environment(EnvironmentSettings::new(80));

        let id = engine
            .spawn_object(SpawnConfig::new("Crate").with_position(Vector2::new(0, 0)))
            .expect("crate spawns");
        let idx = engine.find_object_index(id).expect("crate exists");

        engine.tick_without_snapshot().expect("tick succeeds");
        assert_eq!(
            engine.objects[idx].fixed_velocity.x,
            C4Fixed::ZERO,
            "weather wind never drives object motion"
        );
    }

    #[test]
    fn kneel_procedure_locks_vertical_velocity_and_blocks_wind() {
        let mut definition =
            Definition::from_script("Kneeler", "Kneeler", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Kneel".to_string(), ActionSpec::for_procedure("kneel"));
        definition.configure_actions(Some("Kneel".to_string()), actions);

        let mut engine = Engine::with_seed(19);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(8));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Kneeler")
                    .with_velocity(Vector2::new(5, -4))
                    .with_action(ActionState::new("Kneel")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.y, 0);
        assert_eq!(object.velocity.x, 5);
    }

    #[test]
    fn dig_procedure_zeroes_velocity_when_stopped() {
        let mut definition = Definition::from_script("Digger", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Dig".to_string(), ActionSpec::for_procedure("dig"));
        definition.configure_actions(Some("Dig".to_string()), actions);

        let mut engine = Engine::with_seed(29);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_physics(PhysicsSettings::default());
        engine.set_environment(EnvironmentSettings::new(7));

        let initial_velocity = Vector2::new(4, -3);

        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_velocity(initial_velocity)
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
    }

    #[test]
    fn control_command_invokes_object_script() -> Result<(), EngineError> {
        let script = r#"
global func Initialize(state, random) { return 0; }
func ControlDig() { SetAction("Dig"); return true; }
"#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("control script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Dig".to_string(), ActionSpec::for_procedure("dig"));
        definition.configure_actions(Some("Idle".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default());

        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;

        let object_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Idle")),
            )
            .expect("spawn succeeds");

        engine.set_crew_cursor(1, Some(object_id))?;
        let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
        assert!(handled, "control command should report handled");

        let snapshot = engine.snapshot();
        let object = snapshot.object(object_id).expect("object present");
        assert_eq!(object.action.name, "Dig");
        Ok(())
    }

    #[test]
    fn control_command_coerces_int_returns_like_cpp_bool_cast() -> Result<(), EngineError> {
        // C4Object::CallControl (C4Object.cpp:3300): the Control<Com> result
        // goes through `static_cast<bool>(Call(...))` — C4Value raw-data
        // truthiness (C4Value.h:76,183-185). Real content returns ints
        // (Clonk.c4d/Script.c:195-203 `return(1)` / `return(0)`); C++ never
        // rejects them.
        let script = r#"
global func Initialize(state, random) { return 0; }
func ControlDig() { SetAction("Dig"); return 1; }
func ControlThrow() { return 0; }
"#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("control script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Dig".to_string(), ActionSpec::for_procedure("dig"));
        definition.configure_actions(Some("Idle".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default());

        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;

        let object_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Idle")),
            )
            .expect("spawn succeeds");

        engine.set_crew_cursor(1, Some(object_id))?;
        let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
        assert!(handled, "return(1) is truthy like C++'s bool cast");
        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.object(object_id).expect("object present").action.name,
            "Dig"
        );

        let handled =
            engine.handle_control_command(1, ControlCommand::Throw, CommandKind::Press)?;
        assert!(!handled, "return(0) is falsy like C++'s bool cast");
        Ok(())
    }

    #[test]
    fn control_dispatch_forwards_to_effects_via_effect_call_like_clnk() -> Result<(), EngineError> {
        // The verbatim CLNK Control2Effect chain (Clonk.c4d/Script.c:
        // 195-203, 860-875): ControlDig walks *Control* effects and feeds
        // each GetEffect number into EffectCall (FnEffectCall,
        // C4Script.cpp:5589-5601), which runs Fx<Name><CallFn> on the
        // effect's COMMAND TARGET (C4Effect::DoCall, C4Effect.cpp:439-456)
        // with (pTarget, iNumber, ...) arguments. TRPR/COWB hit exactly
        // this path in GoldRush.
        let clonk_script = r#"
#strict
protected func ControlDig()
{
  if (Control2Effect("ControlDig")) return(1);
  return(0);
}
private func Control2Effect(string szControl)
{
  var i = GetEffectCount(0, this()), iEffect;
  var res;
  while (i--)
  {
    iEffect = GetEffect("*Control*", this(), i);
    if ( GetEffect(0, this(), iEffect, 1) )
      res += EffectCall(this(), iEffect, szControl);
  }
  return(res);
}
"#;
        let gun_script = r#"
#strict
public func Arm()
{
  AddEffect("GunControl", FindObject(CLNK), 100, 0, this());
  return(1);
}
public func FxGunControlControlDig(pTarget, iNumber)
{
  // this() is the command target (the gun): mark it and echo the args.
  Enter(FindObject(BOXX));
  SetR(9);
  EffectVar(0, pTarget, iNumber) = 7;
  return(1);
}
"#;
        let mut clonk =
            Definition::from_script("CLNK", "Clonk", clonk_script).expect("clonk script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        clonk.configure_actions(Some("Idle".to_string()), actions);
        clonk.set_movement_profile(MovementProfile::default());
        let gun = Definition::from_script("GUNX", "Gun", gun_script).expect("gun script compiles");

        let mut engine = Engine::new();
        engine.register_definition(clonk)?;
        engine.register_definition(gun)?;
        engine.register_definition(simple_definition("BOXX"))?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;

        let clonk_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Idle")),
            )
            .expect("spawn clonk");
        let gun_id = engine
            .spawn_object(SpawnConfig::new("GUNX").with_owner(1))
            .expect("spawn gun");
        let box_id = engine
            .spawn_object(SpawnConfig::new("BOXX"))
            .expect("spawn box");
        engine.set_crew_cursor(1, Some(clonk_id))?;

        let armed = engine.execute_context_menu(gun_id, "Arm")?;
        assert!(armed, "the gun installed its control effect");
        let snapshot = engine.snapshot();
        let clonk_effects = &snapshot.object(clonk_id).expect("clonk present").effects;
        assert_eq!(clonk_effects.len(), 1, "GunControl effect attached");

        let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
        assert!(handled, "EffectCall's 1 propagates through Control2Effect");

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.object(gun_id).expect("gun present").rotation,
            9,
            "Fx callback ran with the command target as context"
        );
        assert_eq!(
            snapshot.object(gun_id).expect("gun present").container,
            Some(box_id),
            "omitted-subject Enter uses the effect command target's this()"
        );
        assert_eq!(
            snapshot.object(clonk_id).expect("clonk present").container,
            None,
            "the affected effect carrier is not FnEnter's cthr->Obj"
        );
        let clonk_effects = &snapshot.object(clonk_id).expect("clonk present").effects;
        assert_eq!(
            clonk_effects[0].vars.first(),
            Some(&clonk_engine::effect::EffectVarValue::Int(7)),
            "the Fx callback received (pTarget, iNumber) and wrote the effect var"
        );
        Ok(())
    }

    #[test]
    fn contained_clonk_routes_dig_to_the_container_like_cpp() -> Result<(), EngineError> {
        // C4Object::DirectCom (C4Object.cpp:3363-3367): a contained clonk
        // hands every non-Special com to the container -
        // `Contained->Controller = Controller; ContainedControl(byCom);
        // return;` - which runs the container's Contained<Com> script with
        // the clonk as parameter (sf->Exec(Contained, {C4VObj(this)}),
        // C4Object.cpp:3221,3230). The clonk's own Control<Com> is NOT
        // consulted. Specials bypass containment (:3364).
        let clonk_script = r#"
global func Initialize(state, random) { return 0; }
func ControlDig() { SetR(3); return 1; }
func ControlSpecial() { SetR(4); return 1; }
"#;
        let hut_script = r#"
global func Initialize(state, random) { return 0; }
func ContainedDig(pClonk) { SetR(5); return 1; }
"#;
        let mut clonk =
            Definition::from_script("CLNK", "Clonk", clonk_script).expect("clonk script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        clonk.configure_actions(Some("Idle".to_string()), actions);
        clonk.set_movement_profile(MovementProfile::default());
        let hut = Definition::from_script("HUTX", "Hut", hut_script).expect("hut script compiles");

        let mut engine = Engine::new();
        engine.register_definition(clonk)?;
        engine.register_definition(hut)?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;

        let hut_id = engine
            .spawn_object(SpawnConfig::new("HUTX"))
            .expect("spawn hut");
        let clonk_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Idle"))
                    .with_container(hut_id),
            )
            .expect("spawn clonk");
        engine.set_crew_cursor(1, Some(clonk_id))?;

        let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
        assert!(handled, "the container consumed the com (DirectCom returns)");
        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.object(hut_id).expect("hut present").rotation,
            5,
            "ContainedDig ran on the container"
        );
        assert_ne!(
            snapshot.object(clonk_id).expect("clonk present").rotation,
            3,
            "the clonk's ControlDig was bypassed"
        );

        // Specials skip containment: the clonk's own override runs.
        engine.handle_control_command(1, ControlCommand::Special, CommandKind::Press)?;
        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot.object(clonk_id).expect("clonk present").rotation,
            4,
            "ControlSpecial ran on the clonk despite containment"
        );
        Ok(())
    }

    #[test]
    fn context_menu_callback_coerces_int_returns_like_cpp_bool_cast() -> Result<(), EngineError> {
        // C4Object::MenuCommand (C4Object.cpp:3732-3736): the executed menu
        // function's result goes through `static_cast<bool>(DirectExec(...))`
        // — raw truthiness. Context functions in real content return ints
        // (Waterskin.c4d/Script.c:110 `return(1)`).
        let script = r#"
global func Initialize(state, random) { return 0; }
func EmptyContainer() { SetR(7); return 1; }
"#;
        let mut definition =
            Definition::from_script("WSKI", "Waterskin", script).expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(0);
        engine.register_definition(definition)?;
        let id = engine
            .spawn_object(SpawnConfig::new("WSKI").with_owner(1))
            .expect("spawn succeeds");

        let handled = engine.execute_context_menu(id, "EmptyContainer")?;
        assert!(handled, "return(1) is truthy like C++'s bool cast");
        let snapshot = engine.object_snapshot(id).expect("object snapshot");
        assert_eq!(snapshot.rotation, 7, "the context function ran");
        Ok(())
    }

    #[test]
    fn player_context_menu_includes_and_executes_legacy_annotated_function(
    ) -> Result<(), EngineError> {
        // C4ObjectMenu::AddContextFunctions enumerates annotated Context*
        // functions, evaluates their Condition with (menu crew, image id),
        // and executes the selected function with that same menu crew object
        // (C4ObjectMenu.cpp:398-399,650-682). MagiClonk::ContextMagic uses
        // exactly this path and calls SetComDir on its pByObject argument.
        let script = r#"
#strict 2
public func ContextMagic(object pByObject)
{
  [Magic|Image=MCMS|Condition=ReadyToMagic|Desc=Cast a spell.]
  if (pByObject == this()) { SetR(7); return(1); }
  SetR(8);
  return(0);
}
protected func ReadyToMagic(object pByObject, id image)
{
  return(pByObject == this() && image == MCMS);
}
"#;
        let definition =
            Definition::from_script("MAGE", "Mage", script).expect("mage script compiles");
        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        let mage = engine
            .spawn_object(SpawnConfig::new("MAGE").with_owner(1))
            .expect("mage spawns");

        assert_eq!(
            engine.context_menu_entries(mage)?,
            vec![ContextMenuEntry {
                function: "ContextMagic".to_string(),
                label: "Magic".to_string(),
                description: Some("Cast a spell.".to_string()),
            }],
            "the app-facing context list includes legacy ContextMagic"
        );

        assert!(
            engine.execute_context_menu(mage, "ContextMagic")?,
            "the legacy callback's integer return uses C4Value truthiness"
        );
        assert_eq!(
            engine
                .object_snapshot(mage)
                .expect("mage snapshot")
                .rotation,
            7,
            "ContextMagic receives the live menu crew object, not a state proplist"
        );
        Ok(())
    }

    #[test]
    fn object_function_this_is_the_current_object_not_nil() -> Result<(), EngineError> {
        // `this` used to evaluate to nil (vm.rs hardcoded Expr::This => Nil), so a
        // script that branches on `this` took the wrong path. Here SetAction is
        // gated on `this` being truthy: before the fix `this` was nil (falsy) and
        // the action stayed "Idle"; now `this` is the object reference so the
        // action becomes "Dig".
        let script = r#"
global func Initialize(state, random) { return 0; }
func ControlDig() { if (this) { SetAction("Dig"); } return true; }
"#;
        let mut definition =
            Definition::from_script("CLNK", "Clonk", script).expect("control script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Dig".to_string(), ActionSpec::for_procedure("dig"));
        definition.configure_actions(Some("Idle".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default());

        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Test"))?;

        let object_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_owner(1)
                    .with_crew_member(true)
                    .with_action(ActionState::new("Idle")),
            )
            .expect("spawn succeeds");

        engine.set_crew_cursor(1, Some(object_id))?;
        let handled = engine.handle_control_command(1, ControlCommand::Dig, CommandKind::Press)?;
        assert!(handled, "control command should report handled");

        let snapshot = engine.snapshot();
        let object = snapshot.object(object_id).expect("object present");
        assert_eq!(
            object.action.name, "Dig",
            "`this` should be truthy (the current object), so the gated SetAction runs"
        );
        Ok(())
    }

    #[test]
    fn dig_procedure_carves_diggable_material() {
        let mut definition = Definition::from_script("Digger", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig").with_dig_free(6),
        );
        definition.configure_actions(Some("Dig".to_string()), actions);
        // C4D_StaticBack objects skip ExecMovement (C4Movement.cpp:553-567),
        // and DFA_DIG requires a bottom attachment (C4Object.cpp:4906-4911).
        definition.set_category(CATEGORY_OBJECT);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
        "#;
        let library =
            clonk_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_position(Vector2::new(12, 4))
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let mut snapshot = engine.tick().expect("tick succeeds");
        for _ in 0..5 {
            snapshot = engine.tick().expect("tick succeeds");
        }

        let landscape = snapshot.landscape.as_ref().expect("landscape present");
        let center_height = landscape.surface()[12];
        let edge_height = landscape.surface()[2];
        assert!(center_height > 6);
        assert_eq!(edge_height, 6);

        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Dig");
    }

    #[test]
    fn dig_procedure_stops_before_moving_without_bottom_attachment() {
        // DFA_DIG first calls Shape.Attach(..., CNAT_Bottom); failure runs
        // ObjectComStopDig and returns before assigning dig velocity
        // (src/C4Object.cpp:4906-4911; src/C4ObjectCom.cpp:776-784).
        let mut definition =
            Definition::from_script("Digger", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Dig".to_string(),
                    ActionSpec::default().with_procedure("dig").with_dig_free(6),
                ),
            ]),
        );
        definition.set_category(CATEGORY_OBJECT);
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);
        definition.set_physical(PhysicalInfo {
            dig: C4_MAX_PHYSICAL,
            ..PhysicalInfo::default()
        });

        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            "#,
        )
        .expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 24, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let start = Vector2::new(12, 4);
        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_position(start)
                    .with_action(ActionState::new("Dig"))
                    .with_command_direction(CommandDirection::UpLeft)
                    .with_mobile(true),
            )
            .expect("digger spawns");
        let initial_position = engine
            .object_snapshot(id)
            .expect("spawned digger is observable")
            .position;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("digger survives");
        assert_eq!(object.action.name, "Walk");
        assert_eq!(object.position, initial_position);
        assert_eq!(object.velocity, Vector2::ZERO);
    }

    #[test]
    fn dig_free_uses_post_steering_predicted_center_on_pixel_grid_like_cpp() {
        // DFA_DIG assigns xdir during ExecAction (C4Object.cpp:4906-4935),
        // then DoMovement digs at fixtoi(fix_x+xdir), fixtoi(fix_y+ydir)
        // on the authoritative landscape plane (C4Movement.cpp:227-245).
        let mut definition =
            Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        definition.set_physical(PhysicalInfo {
            dig: C4_MAX_PHYSICAL,
            ..PhysicalInfo::default()
        });
        definition.set_shape_rect(Some(DefinitionRect::new(-1, -1, 2, 2)));
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);
        definition.configure_actions(
            Some("Dig".to_string()),
            HashMap::from([(
                "Dig".to_string(),
                ActionSpec::default()
                    .with_procedure("DIG")
                    .with_length(16)
                    .with_delay(15)
                    .with_next("Dig")
                    .with_dig_free(2),
            )]),
        );

        let library = clonk_resources::MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1

            [Material Granite]
            Name=Granite
            Density=100
            DigFree=0
            "#,
        )
        .expect("materials parse");
        let materials = MaterialSet::from_resource_library(&library);

        let mut bytes = vec![0_u8; 32 * 32];
        // With xdir=1.25, C++ predicts center x=11. Radius two's conditional
        // right edge reaches x=13; the obsolete pre-steering center reaches
        // x=7 on its left edge instead. Keep both pixels as sentinels.
        bytes[9 * 32 + 7] = 1;
        bytes[9 * 32 + 13] = 1;
        // A non-diggable support pixel keeps this a valid bottom attachment
        // for the C++ DFA_DIG precondition.
        bytes[12 * 32 + 10] = 2;
        let grid = landscape::PixelGrid::new(
            32,
            32,
            bytes,
            vec![0, 80, 100],
            vec![None, Some("Earth".into()), Some("Granite".into())],
            vec![None; 3],
        );
        let mut landscape = Landscape::new(32, vec![30; 32]).expect("landscape builds");
        landscape.set_world_height(32);
        landscape.set_pixel_grid(grid);

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_action(ActionState::new("Dig"))
                    .with_command_direction(CommandDirection::Right)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("digger spawns");

        engine.tick_without_snapshot().expect("dig frame succeeds");
        let landscape = engine.landscape().expect("landscape remains");
        assert_eq!(
            landscape.grid_byte_at(13, 9),
            Some(0),
            "the post-steering predicted circle clears its leading edge"
        );
        assert_eq!(
            landscape.grid_byte_at(7, 9),
            Some(1),
            "the obsolete pre-steering circle must not clear its trailing sentinel"
        );
        assert_eq!(
            landscape.grid_byte_at(10, 12),
            Some(2),
            "non-DigFree support remains solid"
        );
    }

    #[test]
    fn dig_procedure_removes_surface_pixel_when_circle_touches_ground() -> Result<(), EngineError> {
        let mut definition = Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig").with_dig_free(6),
        );
        definition.configure_actions(Some("Dig".to_string()), actions);
        definition.set_category(CATEGORY_OBJECT);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
        "#;
        let library =
            clonk_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 20, Some(earth)));

        let position_y = 18;
        let column_x = 12;

        engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_position(Vector2::new(column_x, position_y))
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        for _ in 0..12 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }

        let snapshot = engine.snapshot();
        let landscape = snapshot.landscape.as_ref().expect("landscape present");
        let height = landscape
            .surface()
            .get(column_x as usize)
            .copied()
            .expect("column present");
        assert!(
            height > 20,
            "expected dig to raise surface beyond 20, got {height}"
        );
        Ok(())
    }

    #[test]
    fn dig_procedure_spawns_dig2object_when_ratio_reached() {
        let mut digger = Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig").with_dig_free(6),
        );
        digger.configure_actions(Some("Dig".to_string()), actions);
        digger.set_category(CATEGORY_OBJECT);
        digger.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        digger.set_contact_density(50);

        let gem = Definition::from_script(
            "GEM_",
            "Gem",
            "global func Initialize(state, random) { return 0; }\n",
        )
        .expect("script compiles");

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=3
        "#;
        let library =
            clonk_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(11);
        engine
            .register_definition(digger)
            .expect("digger registers");
        engine.register_definition(gem).expect("gem registers");
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

        engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_position(Vector2::new(12, 4))
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let mut spawned = false;
        for _ in 0..20 {
            let snapshot = engine.tick().expect("tick succeeds");
            if snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "GEM_")
            {
                spawned = true;
                break;
            }
        }

        assert!(
            spawned,
            "expected Dig2Object conversion to spawn target definition"
        );
    }

    #[test]
    fn dig2object_rotation_uses_one_cpp_random_draw() {
        // C4Object::DigOutMaterialCast passes Random(360) to CreateObject
        // with the digger as creator and NO_OWNER (C4Object.cpp:4017-4030).
        // The creator supplies the layer and Construction argument. This
        // seed makes gen_range reject its first raw RngCore sample, exposing
        // the extra ledger draw.
        const SEED: u32 = 28;

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
        "#;
        let library = MaterialLibrary::parse(material_source).expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);

        let mut engine = Engine::with_seed(0);
        let mut digger_definition =
            Definition::from_script("DGRR", "Digger", "").expect("digger compiles");
        digger_definition.set_shape_rect(Some(DefinitionRect::new(-2, 2, 4, 7)));
        engine
            .register_definition(digger_definition)
            .expect("digger registers");
        let mut gem_definition = Definition::from_script(
            "GEM_",
            "Gem",
            "#strict 2\nlocal creator_seen;\nfunc Construction(pCreator) { creator_seen = pCreator; }\n",
        )
        .expect("gem compiles");
        gem_definition.set_rotateable(1);
        engine
            .register_definition(gem_definition)
            .expect("gem registers");
        engine
            .register_definition(simple_definition("LAYR"))
            .expect("layer registers");
        engine.set_materials(materials);

        let mut pixels = vec![0_u8; 25];
        pixels[2 * 5 + 2] = 10;
        let mut densities = vec![0_i32; 128];
        densities[10] = 80;
        let mut material_names = vec![None; 128];
        material_names[10] = Some("Earth".to_string());
        let grid = landscape::PixelGrid::new(
            5,
            5,
            pixels,
            densities,
            material_names,
            vec![None; 128],
        );
        let mut landscape = Landscape::flat(5, 5);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);

        let layer = engine
            .spawn_object(SpawnConfig::new("LAYR").with_loaded(true))
            .expect("layer spawns");
        let digger = engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_position(Vector2::new(2, 2))
                    .with_owner(7)
                    .with_layer(layer)
                    .with_loaded(true),
            )
            .expect("digger spawns");
        engine.rng = LcgRng::new(SEED);
        let before = engine.debug_rng_clone();

        engine.apply_landscape_operations(vec![LandscapeOperation::DigRect {
            origin: Vector2::new(2, 2),
            width: 1,
            height: 1,
            requested: false,
            by_object: Some(digger),
        }]);

        let expected_hold = SEED.wrapping_mul(214_013).wrapping_add(2_531_011);
        let expected_rotation = ((expected_hold >> 16) % 360) as i32;
        let snapshot = engine.snapshot();
        let spawned = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "GEM_")
            .expect("Dig2Object conversion spawns a gem");
        assert_eq!(spawned.rotation, expected_rotation);
        assert_eq!(spawned.position, Vector2::new(2, 11));
        assert_eq!(spawned.owner, OWNER_NONE);
        assert_eq!(spawned.controller, OWNER_NONE);
        assert_eq!(spawned.layer, Some(layer));
        assert_eq!(
            spawned.local_vars.get("creator_seen"),
            Some(&object_reference_value(digger)),
            "Dig2Object Construction receives the digger as creator"
        );
        assert_eq!(snapshot.rng.hold, expected_hold);
        assert_eq!(snapshot.rng.count, before.count + 1);
    }

    #[test]
    fn legacy_dig_conversion_recomputes_creator_geometry_between_materials() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=80
            DigFree=1
            Dig2Object=GEMA
            Dig2ObjectRatio=1

            [Material Rock]
            Name=Rock
            Density=100
            DigFree=1
            Dig2Object=GEMB
            Dig2ObjectRatio=1
        "#,
        )
        .expect("dig materials parse");
        let materials = MaterialSet::from_resource_library(&library);
        let mut engine = Engine::with_seed(44);
        engine.set_materials(materials);

        let mut digger = Definition::from_script("DGR3", "Digger", "#strict 3")
            .expect("digger compiles");
        digger.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 2)));
        let first = Definition::from_script(
            "GEMA",
            "First gem",
            r#"#strict 3
func Construction(object creator)
{
    SetPosition(10, 20, creator);
    SetShape(-1, 3, 4, 7, creator);
}
"#,
        )
        .expect("first gem compiles");
        engine.register_definition(digger).expect("digger registers");
        engine.register_definition(first).expect("first gem registers");
        engine
            .register_definition(simple_definition("GEMB"))
            .expect("second gem registers");

        let grid = landscape::PixelGrid::new(
            2,
            1,
            vec![1, 2],
            vec![0, 80, 100],
            vec![None, Some("Earth".to_owned()), Some("Rock".to_owned())],
            vec![None; 3],
        );
        let mut landscape = Landscape::new(2, vec![1; 2]).expect("landscape builds");
        landscape.set_world_height(1);
        landscape.set_pixel_grid(grid);
        engine.set_landscape(landscape);
        let digger = engine
            .spawn_object(SpawnConfig::new("DGR3"))
            .expect("digger spawns");

        engine.apply_landscape_operations(vec![LandscapeOperation::DigRect {
            origin: Vector2::ZERO,
            width: 2,
            height: 1,
            requested: false,
            by_object: Some(digger),
        }]);

        let first = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "GEMA")
            .expect("first gem exists");
        let second = engine
            .objects
            .iter()
            .find(|object| object.definition_id == "GEMB")
            .expect("second gem exists");
        assert_eq!(
            first.state.position,
            Vector2::ZERO,
            "initial NewObject growth preserves the raw y=0 shape bottom"
        );
        assert_eq!(
            second.state.position,
            Vector2::new(10, 30),
            "legacy movement/direct dig conversion observes prior lifecycle writes"
        );
    }

    #[test]
    fn dig_procedure_spawns_at_most_one_dig2object_per_tick() {
        let mut digger = Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert(
            "Dig".to_string(),
            ActionSpec::default().with_procedure("dig").with_dig_free(6),
        );
        digger.configure_actions(Some("Dig".to_string()), actions);
        digger.set_category(CATEGORY_OBJECT);
        digger.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        digger.set_contact_density(50);

        let gem = Definition::from_script(
            "GEM_",
            "Gem",
            "global func Initialize(state, random) { return 0; }\n",
        )
        .expect("script compiles");

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
        "#;
        let library =
            clonk_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(digger)
            .expect("digger registers");
        engine.register_definition(gem).expect("gem registers");
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

        engine
            .spawn_object(
                SpawnConfig::new("DGRR")
                    .with_position(Vector2::new(12, 4))
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let mut previous_count = 0;
        let mut observed_spawn = false;
        for _ in 0..20 {
            let snapshot = engine.tick().expect("tick succeeds");
            let current_count = snapshot
                .objects
                .iter()
                .filter(|object| object.definition_id == "GEM_")
                .count();
            if current_count > previous_count {
                assert_eq!(
                    current_count - previous_count,
                    1,
                    "expected at most one Dig2Object spawn per tick"
                );
                observed_spawn = true;
                break;
            }
            previous_count = current_count;
        }

        assert!(
            observed_spawn,
            "expected Dig2Object conversion to occur within 20 ticks"
        );
    }

    #[test]
    fn dig2object_request_only_requires_explicit_request() {
        fn build_digger_definition() -> Definition {
            let mut digger = Definition::from_script("DGRR", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
            let mut actions = HashMap::new();
            actions.insert(
                "Dig".to_string(),
                ActionSpec::default().with_procedure("dig").with_dig_free(6),
            );
            digger.configure_actions(Some("Dig".to_string()), actions);
            digger.set_category(CATEGORY_OBJECT);
            digger.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
            digger.set_contact_density(50);
            digger
        }

        fn build_gem_definition() -> Definition {
            Definition::from_script(
                "GEM_",
                "Gem",
                "global func Initialize(state, random) { return 0; }\n",
            )
            .expect("script compiles")
        }

        let material_source = r#"
            [Material Earth]
            Name=Earth
            Density=80
            Friction=25
            DigFree=1
            Dig2Object=GEM_
            Dig2ObjectRatio=1
            Dig2ObjectRequest=1
        "#;
        let library =
            clonk_resources::MaterialLibrary::parse(material_source).expect("material parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        // Without request flag set on the action we should not spawn anything.
        {
            let mut engine = Engine::with_seed(19);
            engine
                .register_definition(build_digger_definition())
                .expect("digger registers");
            engine
                .register_definition(build_gem_definition())
                .expect("gem registers");
            engine.set_materials(materials.clone());
            engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

            engine
                .spawn_object(
                    SpawnConfig::new("DGRR")
                        .with_position(Vector2::new(12, 4))
                        .with_action(ActionState::new("Dig")),
                )
                .expect("spawn succeeds");

            for _ in 0..20 {
                let snapshot = engine.tick().expect("tick succeeds");
                assert!(
                    !snapshot
                        .objects
                        .iter()
                        .any(|object| object.definition_id == "GEM_"),
                    "expected no Dig2Object spawn without request"
                );
            }
        }

        // With request flag set, the conversion should occur.
        {
            let mut engine = Engine::with_seed(19);
            engine
                .register_definition(build_digger_definition())
                .expect("digger registers");
            engine
                .register_definition(build_gem_definition())
                .expect("gem registers");
            engine.set_materials(materials);
            engine.set_landscape(Landscape::flat_with_material(32, 6, Some(earth)));

            let mut requested_action = ActionState::new("Dig");
            requested_action.data = 1;
            engine
                .spawn_object(
                    SpawnConfig::new("DGRR")
                        .with_position(Vector2::new(12, 4))
                        .with_action(requested_action),
                )
                .expect("spawn succeeds");

            let mut spawned = false;
            for _ in 0..20 {
                let snapshot = engine.tick().expect("tick succeeds");
                if snapshot
                    .objects
                    .iter()
                    .any(|object| object.definition_id == "GEM_")
                {
                    spawned = true;
                    break;
                }
            }

            assert!(
                spawned,
                "expected Dig2Object conversion to respect request flag when set"
            );
        }
    }

    #[test]
    fn throw_procedure_zeroes_velocity() {
        let mut definition =
            Definition::from_script("Thrower", "Thrower", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("Throw".to_string(), ActionSpec::for_procedure("throw"));
        definition.configure_actions(Some("Throw".to_string()), actions);

        let mut engine = Engine::with_seed(17);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Thrower")
                    .with_velocity(Vector2::new(6, -3))
                    .with_action(ActionState::new("Throw")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.action.name, "Throw");
    }

    #[test]
    fn object_action_throw_exits_content_after_action_gate() {
        // ObjectActionThrow computes force/facing, changes action without an
        // Action.Target argument, then Exit's the item with one Random(360)
        // draw (C4ObjectCom.cpp:120-137; C4Object.cpp:1532-1563).
        let mut clonk =
            Definition::from_script("CLNK", "Clonk", "#strict 2\n").expect("script compiles");
        clonk.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        let mut physical = PhysicalInfo::default();
        physical.throw = 50_000;
        clonk.set_physical(physical);
        let mut actions = HashMap::new();
        actions.insert("Walk".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Throw".to_string(), ActionSpec::for_procedure("throw"));
        clonk.configure_actions(Some("Walk".to_string()), actions);
        let item =
            Definition::from_script("FLAG", "Flag", "#strict 2\n").expect("script compiles");

        let mut engine = Engine::with_seed(7);
        engine.register_definition(clonk).expect("CLNK registers");
        engine.register_definition(item).expect("FLAG registers");
        let clonk_id = engine
            .spawn_object(
                SpawnConfig::new("CLNK")
                    .with_position(Vector2::new(100, 200))
                    .with_direction(Direction::Right)
                    .with_action(ActionState::new("Walk")),
            )
            .expect("CLNK spawns");
        let flag_id = engine
            .spawn_object(SpawnConfig::new("FLAG").with_container(clonk_id))
            .expect("FLAG spawns");
        engine
            .apply_object_update(clonk_id, ObjectUpdate::new().with_action_update(
                ActionUpdate::default().with_target(Some(flag_id)),
            ))
            .expect("action target is seeded");

        let mut expected_rng = engine.debug_rng_clone();
        let expected_rotation = expected_rng.random(360);
        let before = engine.object_snapshot(clonk_id).expect("CLNK is ready");
        let before_index = engine.find_object_index(clonk_id).expect("CLNK is indexed");
        let shape_top = engine.objects[before_index]
            .current_shape_rect()
            .map(|rect| rect.y)
            .unwrap_or(0);
        let expected_exit = Vector2::new(
            before.position.x,
            before.position.y + shape_top - 1,
        );
        assert!(
            engine
                .try_object_action_throw(clonk_id, flag_id)
                .expect("throw succeeds")
        );

        let clonk = engine.object_snapshot(clonk_id).expect("CLNK remains");
        let flag = engine.object_snapshot(flag_id).expect("FLAG remains");
        let throw_force = math::val_by_physical(400, 50_000);
        assert_eq!(clonk.action.name, "Throw");
        assert_eq!(clonk.action.target, Some(flag_id));
        assert!(clonk.contents.is_empty());
        assert_eq!(flag.container, None);
        assert_eq!(flag.position, expected_exit);
        assert_eq!(flag.rotation, expected_rotation);
        let flag_index = engine.find_object_index(flag_id).expect("FLAG is indexed");
        assert_eq!(
            engine.objects[flag_index].fixed_velocity,
            FixedVec2::new(throw_force, -throw_force)
        );
        assert_eq!(engine.objects[flag_index].rotation_velocity, throw_force);
        assert_eq!(engine.debug_rng_clone(), expected_rng);
    }

    #[test]
    fn scale_procedure_zeroes_horizontal_velocity() {
        let mut definition = Definition::from_script("Scaler", "Scaler", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Scale".to_string(), ActionSpec::for_procedure("scale"));
        definition.configure_actions(Some("Scale".to_string()), actions);

        let mut engine = Engine::with_seed(23);
        engine
            .register_definition(definition)
            .expect("definition registers");

        engine.set_environment(EnvironmentSettings::new(3));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Scaler")
                    .with_velocity(Vector2::new(-7, 2))
                    .with_action(ActionState::new("Scale")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.x, 0);
        assert_eq!(object.velocity.y, 1);
    }

    #[test]
    fn scale_command_direction_moves_up_when_pressing_wall_direction() {
        let mut definition = Definition::from_script("Scaler", "Scaler", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Scale".to_string(), ActionSpec::for_procedure("scale"));
        definition.configure_actions(Some("Scale".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_scale_speed(6)
                .with_scale_acceleration(3),
        );

        let mut engine = Engine::with_seed(41);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Scaler")
                    .with_direction(Direction::Left)
                    .with_command_direction(CommandDirection::Left)
                    .with_action(ActionState::new("Scale")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(0, -3));
        assert_eq!(object.direction, Direction::Left);
    }

    #[test]
    fn hangle_command_direction_updates_velocity_and_direction() {
        let mut definition =
            Definition::from_script("Hangler", "Hangler", PROCEDURE_MOVEMENT_SCRIPT)
                .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Hangle".to_string(), ActionSpec::for_procedure("hang"));
        definition.configure_actions(Some("Hangle".to_string()), actions);
        definition.set_movement_profile(
            MovementProfile::default()
                .with_hangle_speed(5)
                .with_hangle_acceleration(2),
        );

        let mut engine = Engine::with_seed(43);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Hangler")
                    .with_direction(Direction::Right)
                    .with_command_direction(CommandDirection::Left)
                    .with_action(ActionState::new("Hangle")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(-2, 0));
        assert_eq!(object.direction, Direction::Left);
    }

    #[test]
    fn dig_command_direction_sets_directional_velocity() {
        let mut definition = Definition::from_script("Digger", "Digger", PROCEDURE_MOVEMENT_SCRIPT)
            .expect("script compiles");
        let mut actions = HashMap::new();
        actions.insert("Dig".to_string(), ActionSpec::for_procedure("dig"));
        definition.configure_actions(Some("Dig".to_string()), actions);
        definition.set_movement_profile(MovementProfile::default().with_dig_speed(6));

        let mut engine = Engine::with_seed(47);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(
                SpawnConfig::new("Digger")
                    .with_direction(Direction::Right)
                    .with_command_direction(CommandDirection::DownLeft)
                    .with_action(ActionState::new("Dig")),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(-6, 6));
        assert_eq!(object.direction, Direction::Left);

        engine
            .apply_object_update(
                id,
                ObjectUpdate::new().with_command_direction(CommandDirection::Up),
            )
            .expect("update succeeds");

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.velocity, Vector2::new(-6, -3));
        assert_eq!(object.direction, Direction::Left);
    }

    fn construction_builder_definition(
        id: &str,
        script: &str,
        no_other_action: bool,
    ) -> Definition {
        let mut definition =
            Definition::from_script(id, id, script).expect("builder definition compiles");
        definition.set_category(CATEGORY_OBJECT);
        definition.set_physical(PhysicalInfo {
            can_construct: 100,
            ..PhysicalInfo::default()
        });
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Build".to_string(),
                    ActionSpec::default()
                        .with_procedure("BUILD")
                        .with_length(4)
                        .with_delay(5)
                        .with_no_other_action(no_other_action),
                ),
            ]),
        );
        definition
    }

    fn construction_target_definition(script: &str) -> Definition {
        let mut definition =
            Definition::from_script("BTGT", "Build target", script).expect("target compiles");
        definition.set_category(CATEGORY_STRUCTURE);
        definition.set_mass(100);
        definition.set_shape_rect(Some(DefinitionRect::new(-10, -20, 20, 40)));
        definition
    }

    #[test]
    fn build_contained_target_requires_live_powered_build_container() {
        // DFA_BUILD's first target guard requires a contained target's live
        // container to be building without NeedEnergy (C4Object.cpp:5016-5020).
        // It precedes the full-target check and returns without stopping.
        for (label, container_action, need_energy, inactive, construction, expected_delta) in [
            ("idle", "Idle", false, false, 50_000, 0),
            ("needs energy", "Build", true, false, 50_000, 0),
            ("powered build", "Build", false, false, 50_000, 150),
            ("inactive powered build", "Build", false, true, 50_000, 150),
            ("idle full target", "Idle", false, false, FULL_CON, 0),
        ] {
            let mut engine = Engine::with_seed(67);
            engine
                .register_definition(construction_builder_definition("BLDR", "", false))
                .expect("builder registers");
            engine
                .register_definition(construction_builder_definition("BCON", "", false))
                .expect("container registers");
            engine
                .register_definition(construction_target_definition(""))
                .expect("target registers");

            let container = engine
                .spawn_object(
                    SpawnConfig::new("BCON")
                        .with_action(ActionState::new(container_action))
                        .with_need_energy(need_energy)
                        .with_position(Vector2::new(100, 200)),
                )
                .expect("container spawns");
            if inactive {
                let container_idx = engine
                    .find_object_index(container)
                    .expect("container exists");
                engine.objects[container_idx].state.status = ObjectStatus::Inactive;
            }
            let target = engine
                .spawn_object(
                    SpawnConfig::new("BTGT")
                        .with_construction(construction)
                        .with_position(Vector2::new(100, 200))
                        .with_container(container),
                )
                .expect("target spawns");
            let target_idx = engine.find_object_index(target).expect("target exists");
            let target_position = engine.objects[target_idx].state.position;
            let shape = engine.objects[target_idx]
                .current_shape_rect()
                .expect("target has a live shape");
            let mut build = ActionState::new("Build");
            build.target = Some(target);
            build.phase = 2;
            build.ticks = 3;
            build.time = 41;
            let sentinel_velocity = FixedVec2::new(fixed100(125), fixed100(-75));
            let builder = engine
                .spawn_object(
                    SpawnConfig::new("BLDR")
                        .with_position(Vector2::new(
                            target_position.x + shape.x,
                            target_position.y + shape.y,
                        ))
                        .with_fixed_velocity(sentinel_velocity)
                        .with_command_direction(CommandDirection::Right)
                        .with_action(build),
                )
                .expect("builder spawns");
            let builder_idx = engine.find_object_index(builder).expect("builder exists");

            let returned_early = engine
                .apply_physics_at_index(builder_idx)
                .expect("Build procedure executes");
            let target_state = engine.object_snapshot(target).expect("target remains");
            assert_eq!(
                target_state.construction,
                construction.saturating_add(expected_delta).min(FULL_CON),
                "{label}"
            );
            let builder_idx = engine.find_object_index(builder).expect("builder remains");
            let builder_state = &engine.objects[builder_idx];
            assert_eq!(builder_state.state.action.name, "Build", "{label}");
            assert_eq!(builder_state.state.action.time, 42, "{label}");
            assert_eq!(builder_state.state.action.phase, 2, "{label}");
            assert_eq!(builder_state.state.action.ticks, 3, "{label}");
            if expected_delta == 0 {
                assert!(returned_early, "{label} must return from ExecAction");
                assert_eq!(builder_state.state.command_direction, CommandDirection::Right);
                assert_eq!(builder_state.fixed_velocity, sentinel_velocity, "{label}");
                assert_eq!(builder_state.frame_t_attach, CNAT_NONE, "{label}");
                assert_eq!(builder_state.state.t_attach, CNAT_NONE, "{label}");
            } else {
                assert!(!returned_early, "{label} must reach the phase tail");
                assert_eq!(builder_state.fixed_velocity, FixedVec2::ZERO);
                assert_eq!(builder_state.frame_t_attach, CNAT_BOTTOM, "{label}");
                assert_eq!(builder_state.state.t_attach, CNAT_BOTTOM, "{label}");
            }
        }
    }

    #[test]
    fn build_area_uses_live_shape_and_inclusive_vertical_margin() {
        // DFA_BUILD compares the builder against the target's live Shape and
        // uses inclusive Inside bounds, including Wdt and Hgt+16
        // (C4Object.cpp:5027-5032).
        for (label, position_case, should_build, no_other_action, inactive) in [
            ("inclusive bottom-right", 0, true, false, false),
            ("inactive inclusive bottom-right", 0, true, false, true),
            ("one past right", 1, false, false, false),
            ("one past bottom margin", 2, false, false, false),
            ("locked one past right", 1, false, true, false),
        ] {
            let mut engine = Engine::with_seed(67);
            engine
                .register_definition(construction_builder_definition(
                    "BLDR",
                    "",
                    no_other_action,
                ))
                .expect("builder registers");
            engine
                .register_definition(construction_target_definition(""))
                .expect("target registers");
            let target = engine
                .spawn_object(
                    SpawnConfig::new("BTGT")
                        .with_construction(50_000)
                        .with_position(Vector2::new(100, 200)),
                )
                .expect("target spawns");
            let target_idx = engine.find_object_index(target).expect("target exists");
            engine.objects[target_idx].state.damage = 37;
            if inactive {
                engine.objects[target_idx].state.status = ObjectStatus::Inactive;
            }
            let target_position = engine.objects[target_idx].state.position;
            let shape = engine.objects[target_idx]
                .current_shape_rect()
                .expect("target has a live shape");
            let origin = Vector2::new(
                target_position.x + shape.x,
                target_position.y + shape.y,
            );
            let builder_position = match position_case {
                0 => Vector2::new(
                    origin.x + shape.width,
                    origin.y + shape.height + 16,
                ),
                1 => Vector2::new(origin.x + shape.width + 1, origin.y),
                _ => Vector2::new(origin.x, origin.y + shape.height + 17),
            };
            let mut build = ActionState::new("Build");
            build.target = Some(target);
            let sentinel_velocity = FixedVec2::new(fixed100(200), fixed100(-100));
            let builder = engine
                .spawn_object(
                    SpawnConfig::new("BLDR")
                        .with_position(builder_position)
                        .with_fixed_velocity(sentinel_velocity)
                        .with_command_direction(CommandDirection::Right)
                        .with_action(build),
                )
                .expect("builder spawns");
            let builder_idx = engine.find_object_index(builder).expect("builder exists");

            let returned_early = engine
                .apply_physics_at_index(builder_idx)
                .expect("Build procedure executes");
            let target_state = engine.object_snapshot(target).expect("target remains");
            let builder_idx = engine.find_object_index(builder).expect("builder remains");
            let builder_state = &engine.objects[builder_idx];
            if should_build {
                assert!(!returned_early, "{label}");
                assert_eq!(target_state.construction, 51_500, "{label}");
                assert_eq!(builder_state.state.action.name, "Build", "{label}");
                assert_eq!(builder_state.fixed_velocity, FixedVec2::ZERO, "{label}");
            } else {
                assert!(returned_early, "{label}");
                assert_eq!(target_state.construction, 50_000, "{label}");
                assert_eq!(target_state.damage, 37, "{label}");
                assert_eq!(builder_state.state.command_direction, CommandDirection::Stop);
                if no_other_action {
                    assert_eq!(builder_state.state.action.name, "Build", "{label}");
                    assert_eq!(builder_state.fixed_velocity, sentinel_velocity, "{label}");
                } else {
                    assert_eq!(builder_state.state.action.name, "Walk", "{label}");
                    assert_eq!(builder_state.fixed_velocity, FixedVec2::ZERO, "{label}");
                }
                assert_eq!(builder_state.frame_t_attach, CNAT_NONE, "{label}");
            }
        }

        // The range guard also precedes the completed-target branch. A full
        // internal target outside the live shape stops the builder but must
        // not receive SetCommand(Exit).
        let mut engine = Engine::with_seed(68);
        engine
            .register_definition(construction_builder_definition("BLDR", "", false))
            .expect("builder registers");
        engine
            .register_definition(construction_target_definition(""))
            .expect("target registers");
        let builder = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_position(Vector2::new(100, 200))
                    .with_action(ActionState::new("Build")),
            )
            .expect("builder spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("BTGT")
                    .with_position(Vector2::new(100, 200))
                    .with_construction(FULL_CON)
                    .with_container(builder),
            )
            .expect("target spawns");
        let target_idx = engine.find_object_index(target).expect("target exists");
        let target_position = engine.objects[target_idx].state.position;
        let shape = engine.objects[target_idx]
            .current_shape_rect()
            .expect("target has a live shape");
        engine.objects[target_idx].state.no_collect_delay = 2;
        engine.objects[target_idx]
            .commands
            .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
            .expect("old Wait queues");
        let builder_idx = engine.find_object_index(builder).expect("builder exists");
        engine.objects[builder_idx].state.action.target = Some(target);
        engine
            .apply_object_update(
                builder,
                ObjectUpdate::new().with_position(Vector2::new(
                    target_position.x + shape.x + shape.width + 1,
                    target_position.y + shape.y,
                )),
            )
            .expect("builder moves outside target shape");
        let builder_idx = engine.find_object_index(builder).expect("builder remains");
        assert!(
            engine
                .apply_physics_at_index(builder_idx)
                .expect("out-of-range full Build executes")
        );
        let target_idx = engine.find_object_index(target).expect("target remains");
        assert_eq!(engine.objects[target_idx].state.no_collect_delay, 2);
        assert_eq!(
            engine.objects[target_idx].commands.command_names(),
            vec!["Wait".to_string()],
            "area failure must run before completed-target Exit"
        );
    }

    #[test]
    fn build_completed_internal_target_replaces_stack_with_base_exit() {
        // Target::Build returns true on the FullCon crossing. The next BUILD
        // tick stops, then calls plain SetCommand(Exit) on an internal target
        // (C4Object.cpp:5033-5043; SetCommand :3937-3985).
        let target_script = r#"#strict
local own_control_calls;
protected func ControlCommand() { own_control_calls++; return 1; }
"#;
        let mut engine = Engine::with_seed(67);
        engine
            .register_definition(construction_builder_definition("BLDR", "", false))
            .expect("builder registers");
        engine
            .register_definition(construction_target_definition(target_script))
            .expect("target registers");
        let builder = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_position(Vector2::new(100, 200))
                    .with_action(ActionState::new("Build")),
            )
            .expect("builder spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("BTGT")
                    .with_position(Vector2::new(100, 200))
                    .with_construction(FULL_CON - 1)
                    .with_controller(7)
                    .with_container(builder),
            )
            .expect("target spawns");
        let builder_idx = engine.find_object_index(builder).expect("builder exists");
        engine.objects[builder_idx].state.action.target = Some(target);
        let target_idx = engine.find_object_index(target).expect("target exists");
        engine.objects[target_idx].state.controller = 7;
        engine.objects[target_idx].state.no_collect_delay = 2;
        engine.objects[target_idx]
            .commands
            .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
            .expect("old Wait queues");
        engine.objects[target_idx]
            .commands
            .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(10)))
            .expect("old MoveTo queues");

        assert!(
            !engine
                .apply_physics_at_index(builder_idx)
                .expect("crossing Build executes")
        );
        let target_idx = engine.find_object_index(target).expect("target remains");
        assert_eq!(engine.objects[target_idx].state.construction, FULL_CON);
        assert_eq!(
            engine.objects[target_idx].commands.command_names(),
            vec!["Wait".to_string(), "MoveTo".to_string()],
            "the successful FullCon crossing does not issue Exit yet"
        );
        assert_eq!(engine.objects[target_idx].state.no_collect_delay, 2);

        let builder_idx = engine.find_object_index(builder).expect("builder remains");
        assert!(
            engine
                .apply_physics_at_index(builder_idx)
                .expect("completed Build executes")
        );
        let builder_state = engine.object_snapshot(builder).expect("builder remains");
        assert_eq!(builder_state.action.name, "Walk");
        assert_eq!(builder_state.command_direction, CommandDirection::Stop);
        let target_idx = engine.find_object_index(target).expect("target remains");
        assert_eq!(engine.objects[target_idx].state.container, Some(builder));
        assert_eq!(engine.objects[target_idx].state.no_collect_delay, 1);
        assert_eq!(
            engine.objects[target_idx].commands.command_names(),
            vec!["Exit".to_string()],
            "SetCommand replaces the whole old stack"
        );
        let stack = serde_json::to_value(engine.objects[target_idx].commands.snapshot())
            .expect("command stack serializes");
        assert_eq!(stack["commands"][0]["mode"], serde_json::json!("Base"));
        assert!(
            !engine.objects[target_idx]
                .state
                .local_vars
                .contains_key("own_control_calls"),
            "plain SetCommand skips the target's own ControlCommand"
        );
    }

    #[test]
    fn build_completed_internal_target_honors_inside_vehicle_control() {
        let builder_script = r#"#strict
local control_calls, control_command, control_by, control_action;
protected func ControlCommand(command, target, tx, ty, target2, data, by)
{
    control_calls++;
    control_command = command;
    control_by = by;
    control_action = GetAction();
    return 1;
}
"#;
        let target_script = r#"#strict
local own_control_calls;
protected func ControlCommand() { own_control_calls++; return 1; }
"#;
        let mut builder_definition = construction_builder_definition("BLDR", builder_script, false);
        builder_definition.set_vehicle_control(VEHICLE_CONTROL_INSIDE);
        let mut engine = Engine::with_seed(67);
        engine
            .register_definition(builder_definition)
            .expect("builder registers");
        engine
            .register_definition(construction_target_definition(target_script))
            .expect("target registers");
        let builder = engine
            .spawn_object(
                SpawnConfig::new("BLDR")
                    .with_position(Vector2::new(100, 200))
                    .with_controller(1)
                    .with_action(ActionState::new("Build")),
            )
            .expect("builder spawns");
        let target = engine
            .spawn_object(
                SpawnConfig::new("BTGT")
                    .with_position(Vector2::new(100, 200))
                    .with_construction(FULL_CON)
                    .with_controller(7)
                    .with_container(builder),
            )
            .expect("target spawns");
        let builder_idx = engine.find_object_index(builder).expect("builder exists");
        engine.objects[builder_idx].state.action.target = Some(target);
        let target_idx = engine.find_object_index(target).expect("target exists");
        // C4Object::Enter normally inherits a nonliving target's controller
        // from its container; seed a distinct saved/live value so the
        // SetCommand vehicle-overload transfer is observable.
        engine.objects[target_idx].state.controller = 7;
        engine.objects[target_idx].state.no_collect_delay = 2;
        engine.objects[target_idx]
            .commands
            .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
            .expect("old Wait queues");

        assert!(
            engine
                .apply_physics_at_index(builder_idx)
                .expect("completed Build executes")
        );

        let target_idx = engine.find_object_index(target).expect("target remains");
        assert_eq!(engine.objects[target_idx].state.no_collect_delay, 1);
        assert!(
            engine.objects[target_idx].commands.command_names().is_empty(),
            "truthy inside control consumes Exit after clearing the old stack"
        );
        assert!(
            !engine.objects[target_idx]
                .state
                .local_vars
                .contains_key("own_control_calls"),
            "plain SetCommand skips the target's own ControlCommand"
        );
        let builder_idx = engine.find_object_index(builder).expect("builder remains");
        let builder_state = &engine.objects[builder_idx].state;
        assert_eq!(builder_state.action.name, "Walk");
        assert_eq!(builder_state.controller, 7);
        assert_eq!(builder_state.local_vars.get("control_calls"), Some(&Value::Int(1)));
        assert_eq!(
            builder_state.local_vars.get("control_command"),
            Some(&Value::String("Exit".to_string().into()))
        );
        assert_eq!(
            builder_state.local_vars.get("control_by"),
            Some(&compat::object_reference_value(target))
        );
        assert_eq!(
            builder_state.local_vars.get("control_action"),
            Some(&Value::String("Walk".to_string().into())),
            "ObjectComStop precedes target SetCommand"
        );
    }

    #[test]
    fn build_procedure_requires_components_before_progress() -> Result<(), EngineError> {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut builder_definition = Definition::from_script("Builder", "Builder", script)?;
        let mut builder_actions = HashMap::new();
        builder_actions.insert("Walk".to_string(), ActionSpec::for_procedure("walk"));
        builder_actions.insert("Build".to_string(), ActionSpec::for_procedure("build"));
        builder_definition.configure_actions(Some("Walk".to_string()), builder_actions);
        builder_definition.set_category(DEFAULT_CATEGORY);
        builder_definition.set_crew_member(true);
        builder_definition.set_mass(50);

        let mut structure_definition = Definition::from_script("Structure", "Structure", script)?;
        structure_definition.set_constructable(true);
        structure_definition.set_category(CATEGORY_STRUCTURE);
        structure_definition.set_mass(100);
        structure_definition.set_components(vec![DefinitionComponent {
            id: "Wood".to_string(),
            count: 1,
        }]);

        let mut material_definition = Definition::from_script("Wood", "Wood", script)?;
        material_definition.set_mass(20);

        let mut engine = Engine::with_seed(7);
        engine.register_definition(builder_definition)?;
        engine.register_definition(structure_definition)?;
        engine.register_definition(material_definition)?;
        engine.set_construction_needs_material(true);

        let structure_id = engine
            // CreateConstruction sites enter the world at one percent;
            // a zero-construction NewObject is removed before return
            // (C4Game.cpp:1110-1129; C4Object.cpp:1513-1517).
            .spawn_object(SpawnConfig::new("Structure").with_construction(1_000))
            .expect("structure spawns");

        let mut build_state = ActionState::new("Build");
        build_state.target = Some(structure_id);
        let builder_id = engine
            .spawn_object(
                SpawnConfig::new("Builder")
                    .with_action(build_state)
                    .with_alive(true)
                    .with_crew_member(true)
                    .with_controller(4)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("builder spawns");

        let before = engine
            .object_snapshot(structure_id)
            .expect("structure present")
            .construction;
        let snapshot = engine.tick()?;
        let after = snapshot
            .object(structure_id)
            .expect("structure present")
            .construction;
        assert_eq!(before, 1_000);
        assert_eq!(
            after, 1_000,
            "construction should not progress without components"
        );
        assert_eq!(
            snapshot
                .object(builder_id)
                .and_then(|builder| builder.action_procedure.as_deref()),
            Some("walk"),
            "a material refusal must stop DFA_BUILD like ObjectComStop"
        );
        assert_eq!(
            snapshot
                .object(builder_id)
                .map(|builder| builder.command_stack.command_names()),
            Some(vec!["Acquire".to_owned()]),
            "C4Object::Build must queue Acquire for the first missing component"
        );
        assert_eq!(
            snapshot
                .object(builder_id)
                .and_then(|builder| builder.command_stack.command_views().first().cloned())
                .map(|command| command.data),
            Some(CommandData::Text("Wood".to_owned())),
            "Acquire must request the exact missing component (C4Object.cpp:1725-1747)"
        );
        assert_eq!(snapshot.hud.messages.len(), 1);
        assert_eq!(snapshot.hud.messages[0].kind, MessageKind::Target);
        assert_eq!(snapshot.hud.messages[0].target, Some(builder_id));
        assert_eq!(snapshot.hud.messages[0].player, Some(4));
        assert_eq!(
            snapshot.hud.messages[0].lines,
            vec!["Structure", "needs", "1x Wood"]
        );
        Ok(())
    }

    #[test]
    fn failed_build_command_does_not_duplicate_live_needed_material_message(
    ) -> Result<(), EngineError> {
        // C4Object::Build first creates the target message. If its retained
        // Build command later fails, C4Command::Fail uses Append with
        // fNoDuplicates=true, so C4GameMessage::Append keeps the existing
        // identical text instead of drawing it twice (C4Object.cpp:1733-1747;
        // C4Command.cpp:2185-2194,2229-2235; C4GameMessage.cpp:73-83,315-328).
        let mut builder = Definition::from_script("BLDR", "Builder", "#strict")?;
        builder.set_crew_member(true);
        builder.set_physical(PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        });
        builder.configure_actions(
            Some("Walk".to_owned()),
            HashMap::from([
                (
                    "Walk".to_owned(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Build".to_owned(),
                    ActionSpec::default().with_procedure("build"),
                ),
            ]),
        );

        let mut site = Definition::from_script("SITE", "Site", "#strict")?;
        site.set_constructable(true);
        site.set_category(CATEGORY_STRUCTURE);
        site.set_components(vec![DefinitionComponent {
            id: "WOOD".to_owned(),
            count: 1,
        }]);

        let mut engine = Engine::with_seed(71);
        engine.register_definition(builder)?;
        engine.register_definition(site)?;
        engine.register_definition(Definition::from_script("WOOD", "Wood", "#strict")?)?;
        engine.set_construction_needs_material(true);

        let site_id = engine.spawn_object(
            SpawnConfig::new("SITE")
                .with_construction(1_000)
                .with_ordered_components(vec![("WOOD".to_owned(), 0)]),
        )?;
        let mut action = ActionState::new("Build");
        action.target = Some(site_id);
        let builder_id = engine.spawn_object(
            SpawnConfig::new("BLDR")
                .with_action(action)
                .with_alive(true)
                .with_crew_member(true)
                .with_controller(4),
        )?;
        let builder_index = engine
            .find_object_index(builder_id)
            .expect("builder exists");
        engine.objects[builder_index]
            .commands
            .push_front(
                CommandRequest::new(CommandId::Build)
                    .with_target(Some(site_id))
                    .with_mode(CommandMode::Base),
            )
            .expect("Build queues");

        let first = engine.tick()?;
        assert_eq!(first.hud.messages.len(), 1);
        assert_eq!(
            first.hud.messages[0].lines,
            vec!["Site", "needs", "1x Wood"]
        );
        let first_message_id = first.hud.messages[0].id;
        assert_eq!(first.hud.messages[0].player, Some(4));
        assert_eq!(
            first
                .object(builder_id)
                .expect("builder remains")
                .command_stack
                .command_names(),
            vec!["Acquire", "Build"]
        );

        let builder_index = engine
            .find_object_index(builder_id)
            .expect("builder remains");
        assert_eq!(
            engine.objects[builder_index].commands.front_command_name(),
            Some("Acquire")
        );
        engine.objects[builder_index].commands.clear_front();
        assert!(
            engine.objects[builder_index]
                .commands
                .fail_front_if(CommandId::Build),
            "retained Build command is forced through its native failure tail"
        );

        let failed = engine.tick()?;
        assert_eq!(
            failed.hud.messages.len(),
            1,
            "the failed Build retains exactly the original HUD message"
        );
        let material_messages = failed
            .hud
            .messages
            .iter()
            .filter(|message| {
                message.kind == MessageKind::Target
                    && message.target == Some(builder_id)
                    && message.lines == vec!["Site", "needs", "1x Wood"]
            })
            .collect::<Vec<_>>();
        assert_eq!(
            material_messages.len(),
            1,
            "C++ appends with duplicate suppression instead of rendering a second message"
        );
        assert_eq!(material_messages[0].id, first_message_id);
        assert_eq!(
            material_messages[0].player,
            Some(4),
            "Append retains the original C4GameMessage metadata"
        );
        Ok(())
    }

    #[test]
    fn build_procedure_noncrew_reports_material_without_acquire() -> Result<(), EngineError> {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut builder_definition = Definition::from_script("Machine", "Machine", script)?;
        builder_definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Build".to_string(),
                    ActionSpec::default().with_procedure("build"),
                ),
            ]),
        );

        let mut structure_definition = Definition::from_script("Site", "Site", script)?;
        structure_definition.set_constructable(true);
        structure_definition.set_category(CATEGORY_STRUCTURE);
        structure_definition.set_components(vec![DefinitionComponent {
            id: "Wood".to_string(),
            count: 1,
        }]);
        let material_definition = Definition::from_script("Wood", "Wood", script)?;

        let mut engine = Engine::with_seed(8);
        engine.register_definition(builder_definition)?;
        engine.register_definition(structure_definition)?;
        engine.register_definition(material_definition)?;
        engine.set_construction_needs_material(true);

        let structure_id = engine.spawn_object(
            SpawnConfig::new("Site")
                .with_construction(1_000)
                .with_ordered_components(vec![("Wood".to_owned(), 0)]),
        )?;
        let mut build_state = ActionState::new("Build");
        build_state.target = Some(structure_id);
        let builder_id = engine.spawn_object(
            SpawnConfig::new("Machine")
                .with_action(build_state)
                .with_controller(6),
        )?;

        let snapshot = engine.tick()?;
        let builder = snapshot.object(builder_id).expect("builder remains");
        assert!(
            builder.command_stack.is_empty(),
            "noncrew builders must not receive Acquire"
        );
        assert_eq!(builder.action_procedure.as_deref(), Some("walk"));
        assert_eq!(snapshot.hud.messages.len(), 1);
        assert_eq!(snapshot.hud.messages[0].kind, MessageKind::Target);
        assert_eq!(snapshot.hud.messages[0].target, Some(builder_id));
        assert_eq!(snapshot.hud.messages[0].player, Some(6));
        assert_eq!(
            snapshot.hud.messages[0].lines,
            vec!["Site", "needs", "1x Wood"]
        );
        Ok(())
    }

    #[test]
    fn build_needs_material_truthy_runs_after_grab_and_before_stop() -> Result<(), EngineError> {
        let builder_script = r#"#strict 2
local missing_id, missing_count, contents_seen, action_seen, callback_order;

protected func BuildNeedsMaterial(component_id, count)
{
    missing_id = component_id;
    missing_count = count;
    contents_seen = ContentsCount();
    action_seen = GetAction();
    callback_order = callback_order * 10 + 1;
    return 1;
}

protected func BuildAbort()
{
    callback_order = callback_order * 10 + 2;
}
"#;
        let mut builder_definition =
            Definition::from_script("Bldr", "Builder", builder_script)?;
        builder_definition.set_c4_callback_convention(true);
        builder_definition.set_crew_member(true);
        builder_definition.configure_actions(
            Some("Walk".to_string()),
            HashMap::from([
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Build".to_string(),
                    ActionSpec::default()
                        .with_procedure("build")
                        .with_abort_call("BuildAbort"),
                ),
            ]),
        );

        let mut structure_definition = Definition::from_script("Site", "Structure", "#strict")?;
        structure_definition.set_constructable(true);
        structure_definition.set_category(CATEGORY_STRUCTURE);
        structure_definition.set_mass(100);
        structure_definition.set_components(vec![DefinitionComponent {
            id: "Wood".to_string(),
            count: 5,
        }]);
        let material_definition = Definition::from_script("Wood", "Wood", "#strict")?;
        let mut container_definition =
            Definition::from_script("Cntn", "Container", "#strict")?;
        container_definition.set_category(CATEGORY_STRUCTURE);
        container_definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Build".to_string(),
                    ActionSpec::default().with_procedure("build"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(9);
        engine.register_definition(builder_definition)?;
        engine.register_definition(structure_definition)?;
        engine.register_definition(material_definition)?;
        engine.register_definition(container_definition)?;
        engine.set_construction_needs_material(true);

        let container_id = engine.spawn_object(
            SpawnConfig::new("Cntn").with_action(ActionState::new("Build")),
        )?;
        let structure_id = engine.spawn_object(
            SpawnConfig::new("Site")
                .with_construction(75_000)
                .with_ordered_components(vec![("Wood".to_owned(), 1)])
                .with_container(container_id),
        )?;
        let mut build_state = ActionState::new("Build");
        build_state.target = Some(structure_id);
        let builder_id = engine.spawn_object(
            SpawnConfig::new("Bldr")
                .with_action(build_state)
                .with_alive(true)
                .with_crew_member(true),
        )?;
        let wood_id = engine.spawn_object(
            SpawnConfig::new("Wood")
                .with_construction(FULL_CON)
                .with_container(builder_id),
        )?;
        let container_wood_id = engine.spawn_object(
            SpawnConfig::new("Wood")
                .with_construction(FULL_CON)
                .with_container(container_id),
        )?;

        let snapshot = engine.tick()?;
        let builder = snapshot.object(builder_id).expect("builder remains");
        assert_eq!(
            builder.local_vars.get("missing_id"),
            Some(&Value::C4Id("Wood".into()))
        );
        assert_eq!(
            builder.local_vars.get("missing_count"),
            Some(&Value::Int(2))
        );
        assert_eq!(
            builder.local_vars.get("contents_seen"),
            Some(&Value::Int(0))
        );
        assert_eq!(
            builder.local_vars.get("action_seen"),
            Some(&Value::String("Build".to_owned().into()))
        );
        assert_eq!(
            builder.local_vars.get("callback_order"),
            Some(&Value::Int(12)),
            "BuildNeedsMaterial must run before ObjectComStop's abort callback"
        );
        assert_eq!(builder.action_procedure.as_deref(), Some("walk"));
        assert!(builder.command_stack.is_empty());
        assert!(snapshot.hud.messages.is_empty());
        assert!(
            snapshot.object(wood_id).is_none(),
            "grabbed material is consumed"
        );
        assert!(
            snapshot.object(container_wood_id).is_none(),
            "the construction-container pass also precedes the callback"
        );
        let structure = snapshot.object(structure_id).expect("structure remains");
        assert_eq!(structure.construction, 75_000);
        assert_eq!(structure.components.get("Wood"), Some(&3));
        Ok(())
    }

    #[test]
    fn build_procedure_consumes_components_from_builder() -> Result<(), EngineError> {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut builder_definition = Definition::from_script("Builder", "Builder", script)?;
        let mut builder_actions = HashMap::new();
        builder_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        builder_actions.insert("Build".to_string(), ActionSpec::for_procedure("build"));
        builder_definition.configure_actions(Some("Idle".to_string()), builder_actions);
        builder_definition.set_category(DEFAULT_CATEGORY);
        builder_definition.set_mass(50);
        builder_definition.set_physical(PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        });

        let mut structure_definition = Definition::from_script("Structure", "Structure", script)?;
        structure_definition.set_constructable(true);
        structure_definition.set_category(CATEGORY_STRUCTURE);
        structure_definition.set_mass(100);
        structure_definition.set_components(vec![DefinitionComponent {
            id: "Wood".to_string(),
            count: 1,
        }]);

        let mut material_definition = Definition::from_script("Wood", "Wood", script)?;
        material_definition.set_mass(20);

        let mut engine = Engine::with_seed(11);
        engine.register_definition(builder_definition)?;
        engine.register_definition(structure_definition)?;
        engine.register_definition(material_definition)?;
        engine.set_construction_needs_material(true);

        let structure_id = engine
            .spawn_object(SpawnConfig::new("Structure").with_construction(0))
            .expect("structure spawns");

        let mut build_state = ActionState::new("Build");
        build_state.target = Some(structure_id);
        let builder_id = engine
            .spawn_object(
                SpawnConfig::new("Builder")
                    .with_action(build_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("builder spawns");

        let wood_id = engine
            .spawn_object(SpawnConfig::new("Wood").with_construction(FULL_CON))
            .expect("wood spawns");
        engine
            .apply_object_update(wood_id, ObjectUpdate::new().with_container(builder_id))
            .expect("assign container succeeds");

        let snapshot = engine.tick()?;
        let structure = snapshot
            .object(structure_id)
            .expect("structure present after tick");
        assert!(
            structure.construction > 0,
            "construction should advance when components are available"
        );
        let components = structure.components.get("Wood");
        assert_eq!(components, Some(&1));
        assert!(
            snapshot.object(wood_id).is_none(),
            "component should be consumed during build"
        );
        Ok(())
    }

    #[test]
    fn build_consumes_only_eligible_first_material_via_assign_removal(
    ) -> Result<(), EngineError> {
        let builder_script = r#"#strict
local removal_order, contents_seen, component_seen, contained_seen;

protected func ContentsDestruction(material)
{
    removal_order = removal_order * 10 + 1;
    contents_seen = ContentsCount(WOOD);
    component_seen = GetComponent(WOOD, 0, GetActionTarget());
    contained_seen = Contained(material) == this();
}

public func MaterialDestruction()
{
    removal_order = removal_order * 10 + 2;
}
"#;
        let material_script = r#"#strict
protected func Destruction()
{
    if (Contained()) Contained()->MaterialDestruction();
}
"#;

        let mut builder = Definition::from_script("BLDR", "Builder", builder_script)?;
        builder.set_c4_callback_convention(true);
        builder.configure_actions(
            Some("Walk".to_owned()),
            HashMap::from([
                (
                    "Walk".to_owned(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Build".to_owned(),
                    ActionSpec::default().with_procedure("build"),
                ),
            ]),
        );
        builder.set_physical(PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        });

        let mut site = Definition::from_script("SITE", "Site", "#strict")?;
        site.set_category(CATEGORY_STRUCTURE);
        site.set_mass(100);
        site.set_components(vec![DefinitionComponent {
            id: "WOOD".to_owned(),
            // At Con=10%, inserting one object exactly satisfies the
            // material gate. The following successful DoCon must retain one
            // rather than auto-gaining a second component while
            // fNoComponentChange/fNeedMaterial is true.
            count: 10,
        }]);

        let wood = Definition::from_script("WOOD", "Wood", material_script)?;
        let mut engine = Engine::with_seed(55);
        engine.register_definition(builder)?;
        engine.register_definition(site)?;
        engine.register_definition(wood)?;
        engine.set_construction_needs_material(true);

        let spawn_pair = |engine: &mut Engine| -> Result<(ObjectId, ObjectId), EngineError> {
            let site_id = engine.spawn_object(
                SpawnConfig::new("SITE")
                    .with_construction(1_000)
                    .with_ordered_components(vec![("WOOD".to_owned(), 0)]),
            )?;
            let mut action = ActionState::new("Build");
            action.target = Some(site_id);
            let builder_id = engine.spawn_object(
                SpawnConfig::new("BLDR")
                    .with_action(action)
                    .with_command_direction(CommandDirection::Right),
            )?;
            Ok((builder_id, site_id))
        };

        let (valid_builder, valid_site) = spawn_pair(&mut engine)?;
        let valid_wood = engine.spawn_object(
            SpawnConfig::new("WOOD")
                .with_construction(FULL_CON)
                .with_container(valid_builder),
        )?;

        let (burning_builder, burning_site) = spawn_pair(&mut engine)?;
        let burning_a = engine.spawn_object(
            SpawnConfig::new("WOOD")
                .with_construction(FULL_CON)
                .with_container(burning_builder),
        )?;
        let burning_b = engine.spawn_object(
            SpawnConfig::new("WOOD")
                .with_construction(FULL_CON)
                .with_container(burning_builder),
        )?;
        let burning_head = engine
            .object_snapshot(burning_builder)
            .and_then(|builder| builder.contents.first().copied())
            .expect("burning builder has a head component");
        let mut fire = ObjectUpdate::new();
        fire.stage_ignite(0, 0);
        engine.apply_object_update(burning_head, fire)?;

        let (partial_builder, partial_site) = spawn_pair(&mut engine)?;
        let partial_a = engine.spawn_object(
            SpawnConfig::new("WOOD")
                .with_construction(FULL_CON)
                .with_container(partial_builder),
        )?;
        let partial_b = engine.spawn_object(
            SpawnConfig::new("WOOD")
                .with_construction(FULL_CON)
                .with_container(partial_builder),
        )?;
        let partial_head = engine
            .object_snapshot(partial_builder)
            .and_then(|builder| builder.contents.first().copied())
            .expect("partial builder has a head component");
        engine.apply_object_update(
            partial_head,
            ObjectUpdate::new().with_construction(FULL_CON / 2),
        )?;

        let snapshot = engine.tick()?;
        assert!(snapshot.object(valid_wood).is_none());
        let valid_builder = snapshot.object(valid_builder).expect("builder survives");
        assert_eq!(valid_builder.local_vars.get("removal_order"), Some(&Value::Int(12)));
        assert_eq!(valid_builder.local_vars.get("contents_seen"), Some(&Value::Int(0)));
        assert_eq!(valid_builder.local_vars.get("component_seen"), Some(&Value::Int(1)));
        assert_eq!(valid_builder.local_vars.get("contained_seen"), Some(&Value::Bool(true)));
        assert_eq!(
            snapshot
                .object(valid_site)
                .and_then(|site| site.components.get("WOOD")),
            Some(&1)
        );

        for (label, material) in [
            ("burning first", burning_a),
            ("burning duplicate", burning_b),
            ("partial first", partial_a),
            ("partial duplicate", partial_b),
        ] {
            assert!(snapshot.object(material).is_some(), "{label} survives");
        }
        for site in [burning_site, partial_site] {
            let site = snapshot.object(site).expect("blocked site survives");
            assert_eq!(site.construction, 1_000);
            assert_eq!(site.components.get("WOOD"), Some(&0));
        }
        Ok(())
    }

    #[test]
    fn build_uses_definition_custom_components_with_builder_argument(
    ) -> Result<(), EngineError> {
        let builder_script = r#"#strict
local component_queries;

public func RecordComponentQuery()
{
    component_queries++;
}
"#;
        let site_script = r#"#strict
protected func GetCustomComponents(builder)
{
    builder->RecordComponentQuery();
    return [METL];
}
"#;

        let mut builder = Definition::from_script("BLDR", "Builder", builder_script)?;
        builder.configure_actions(
            Some("Walk".to_owned()),
            HashMap::from([
                (
                    "Walk".to_owned(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Build".to_owned(),
                    ActionSpec::default().with_procedure("build"),
                ),
            ]),
        );
        builder.set_physical(PhysicalInfo {
            can_construct: 1,
            ..PhysicalInfo::default()
        });

        let mut site = Definition::from_script("SITE", "Site", site_script)?;
        site.set_category(CATEGORY_STRUCTURE);
        site.set_mass(100);
        site.set_components(vec![DefinitionComponent {
            id: "WOOD".to_owned(),
            count: 1,
        }]);

        let mut engine = Engine::with_seed(57);
        engine.register_definition(builder)?;
        engine.register_definition(site)?;
        engine.register_definition(Definition::from_script("WOOD", "Wood", "#strict")?)?;
        engine.register_definition(Definition::from_script("METL", "Metal", "#strict")?)?;
        engine.set_construction_needs_material(true);

        let site_id = engine.spawn_object(
            SpawnConfig::new("SITE")
                .with_construction(1_000)
                .with_ordered_components(vec![("WOOD".to_owned(), 0)]),
        )?;
        let mut action = ActionState::new("Build");
        action.target = Some(site_id);
        let builder_id = engine.spawn_object(SpawnConfig::new("BLDR").with_action(action))?;
        let metal_id = engine.spawn_object(
            SpawnConfig::new("METL")
                .with_construction(FULL_CON)
                .with_container(builder_id),
        )?;

        let snapshot = engine.tick()?;
        assert!(snapshot.object(metal_id).is_none());
        let site = snapshot.object(site_id).expect("site survives");
        assert_eq!(site.construction, 2_500);
        assert_eq!(site.components.get("METL"), Some(&1));
        assert_eq!(site.components.get("WOOD"), Some(&0));
        assert_eq!(
            snapshot
                .object(builder_id)
                .and_then(|builder| builder.local_vars.get("component_queries")),
            Some(&Value::Int(1)),
            "Build calls the definition hook once with the live builder"
        );
        Ok(())
    }

    #[test]
    fn build_uses_can_construct_turn_to_docon_components_and_repair(
    ) -> Result<(), EngineError> {
        fn builder_definition(id: &str, can_construct: i32) -> Result<Definition, EngineError> {
            let mut definition = Definition::from_script(
                id,
                id,
                r#"#strict
local turn_damage;
public func RecordTurnDamage(value) { turn_damage = value; }
"#,
            )?;
            definition.configure_actions(
                Some("Walk".to_owned()),
                HashMap::from([
                    (
                        "Walk".to_owned(),
                        ActionSpec::default().with_procedure("walk"),
                    ),
                    (
                        "Build".to_owned(),
                        ActionSpec::default().with_procedure("build"),
                    ),
                ]),
            );
            definition.set_physical(PhysicalInfo {
                can_construct,
                ..PhysicalInfo::default()
            });
            Ok(definition)
        }

        let mut site = Definition::from_script("SITE", "Site", "#strict")?;
        site.set_category(CATEGORY_STRUCTURE);
        site.set_mass(100);
        site.set_components(vec![DefinitionComponent {
            id: "STON".to_owned(),
            count: 100,
        }]);
        site.set_build_turn_to(Some("DONE".to_owned()));

        let mut engine = Engine::with_seed(56);
        engine.register_definition(builder_definition("FAST", 200)?)?;
        engine.register_definition(builder_definition("ZERO", 0)?)?;
        engine.register_definition(site)?;
        let mut done = Definition::from_script(
            "DONE",
            "Done",
            r#"#strict
protected func RejectEntrance(container)
{
    container->RecordTurnDamage(GetDamage());
    return false;
}
"#,
        )?;
        done.set_c4_callback_convention(true);
        engine.register_definition(done)?;
        engine.register_definition(Definition::from_script("STON", "Stone", "#strict")?)?;

        let spawn_build =
            |engine: &mut Engine, builder: &str| -> Result<(ObjectId, ObjectId), EngineError> {
                let site_id = engine.spawn_object(
                    SpawnConfig::new("SITE")
                        .with_construction(1_000)
                        .with_ordered_components(vec![("STON".to_owned(), 0)]),
                )?;
                engine.apply_object_update(site_id, ObjectUpdate::new().with_damage(77))?;
                let mut action = ActionState::new("Build");
                action.target = Some(site_id);
                let builder_id =
                    engine.spawn_object(SpawnConfig::new(builder).with_action(action))?;
                Ok((builder_id, site_id))
            };
        let (fast_builder, fast_site) = spawn_build(&mut engine, "FAST")?;
        let (zero_builder, zero_site) = spawn_build(&mut engine, "ZERO")?;

        let full_site = engine.spawn_object(
            SpawnConfig::new("SITE")
                .with_construction(99_000)
                .with_ordered_components(vec![("STON".to_owned(), 0)]),
        )?;
        let mut full_action = ActionState::new("Build");
        full_action.target = Some(full_site);
        engine.spawn_object(SpawnConfig::new("FAST").with_action(full_action))?;

        // A contained construction silently exits and re-enters its builder
        // during BuildTurnTo. The new definition's RejectEntrance observes
        // Damage before Build's following repair assignment.
        let mut internal_action = ActionState::new("Build");
        let internal_builder =
            engine.spawn_object(SpawnConfig::new("FAST").with_action(internal_action.clone()))?;
        let internal_site = engine.spawn_object(
            SpawnConfig::new("SITE")
                .with_construction(1_000)
                .with_container(internal_builder),
        )?;
        internal_action.target = Some(internal_site);
        let internal_builder_idx = engine
            .find_object_index(internal_builder)
            .expect("internal builder exists");
        engine.objects[internal_builder_idx].state.action = internal_action;
        engine.apply_object_update(internal_site, ObjectUpdate::new().with_damage(77))?;

        let snapshot = engine.tick()?;
        let fast = snapshot.object(fast_site).expect("fast site survives");
        assert_eq!(fast.construction, 4_000);
        assert_eq!(fast.components.get("STON"), Some(&4));
        assert_eq!(fast.definition_id, "DONE");
        assert_eq!(fast.damage, 0);
        assert_eq!(
            snapshot
                .object(fast_builder)
                .map(|builder| builder.action.name.as_str()),
            Some("Build")
        );

        let zero = snapshot.object(zero_site).expect("zero-speed site survives");
        assert_eq!(zero.construction, 1_000);
        assert_eq!(zero.components.get("STON"), Some(&0));
        assert_eq!(zero.definition_id, "SITE");
        assert_eq!(zero.damage, 77);
        assert_eq!(
            snapshot
                .object(zero_builder)
                .and_then(|builder| builder.action_procedure.as_deref()),
            Some("walk")
        );

        let full = snapshot.object(full_site).expect("full site survives");
        assert_eq!(full.construction, FULL_CON);
        assert_eq!(full.components.get("STON"), Some(&100));

        let internal = snapshot
            .object(internal_site)
            .expect("internal site survives");
        assert_eq!(internal.definition_id, "DONE");
        assert_eq!(internal.damage, 0);
        assert_eq!(
            snapshot
                .object(internal_builder)
                .and_then(|builder| builder.local_vars.get("turn_damage")),
            Some(&Value::Int(77)),
            "BuildTurnTo callbacks run before the successful-build repair write"
        );
        Ok(())
    }

    #[test]
    fn applies_velocity_changes_from_step_callback() {
        let mut engine = Engine::with_seed(123);
        engine
            .register_definition(build_definition())
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(1, 0))
                    .with_energy(50),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(1, 0));
        assert_eq!(object.velocity, Vector2::new(2, 0));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(3, 0));
        assert_eq!(object.velocity, Vector2::new(3, 0));
    }
    #[test]
    fn push_procedure_without_target_resets_to_default() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Pusher", "Pusher", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Push".to_string(), ActionSpec::for_procedure("push"));
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(12);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let push_state = ActionState::new("Push");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_action(push_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Idle");
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.command_direction, CommandDirection::Stop);
    }

    #[test]
    fn failed_push_stands_in_walk_and_adds_cpp_delay_command() {
        // Every DFA_PUSH failure calls StopActionDelayCommand: ObjectComStop
        // stands the Clonk in Walk, then a 50-frame Wait is added to the top
        // of its command stack (C4Object.cpp:4677-4681,5060-5094).
        let mut definition = Definition::from_script("Pusher", "Pusher", "").unwrap();
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("walk"),
                ),
                (
                    "Push".to_string(),
                    ActionSpec::default().with_procedure("push").with_delay(2),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(13);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_action(ActionState::new("Push"))
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("pusher spawns");

        engine.tick_without_snapshot().expect("failed Push executes");

        let object = engine.object_snapshot(id).expect("pusher survives");
        assert_eq!(object.action.name, "Walk");
        assert_eq!(
            (object.action.phase, object.action.ticks, object.action.time),
            (0, 0, 0),
            "the failed Push return skips its stale phase tail"
        );
        assert_eq!(object.velocity, Vector2::ZERO);
        assert_eq!(object.position, Vector2::ZERO);
        assert_eq!(object.command_direction, CommandDirection::Stop);
        let index = engine.find_object_index(id).expect("pusher exists");
        assert_eq!(
            engine.objects[index].commands.snapshot().command_names(),
            vec!["Wait".to_string()]
        );
        let stack = serde_json::to_value(engine.objects[index].commands.snapshot())
            .expect("command stack serializes");
        assert_eq!(stack["commands"][0]["mode"], serde_json::json!("SilentSub"));
        assert_eq!(stack["commands"][0]["update_interval"], serde_json::json!(50));
    }

    fn push_containment_engine(with_physical: bool) -> Engine {
        let mut pusher = Definition::from_script("PCPS", "Containment pusher", "")
            .expect("pusher definition compiles");
        pusher.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        if with_physical {
            pusher.set_physical(PhysicalInfo {
                walk: 35_000,
                push: 45_000,
                ..PhysicalInfo::default()
            });
        } else {
            pusher.set_movement_profile(
                MovementProfile::default()
                    .with_walk_speed(6)
                    .with_walk_acceleration(3),
            );
        }
        pusher.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Push".to_string(),
                    ActionSpec::default().with_procedure("PUSH"),
                ),
            ]),
        );

        let mut target = Definition::from_script("PCTG", "Containment target", "")
            .expect("target definition compiles");
        target.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        target.set_grab(1);
        target.set_mass(200);

        let mut engine = Engine::with_seed(65);
        engine.register_definition(pusher).expect("pusher registers");
        engine.register_definition(target).expect("target registers");
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );
        engine
    }

    fn spawn_push_direction_case(
        engine: &mut Engine,
        target_action: &str,
    ) -> (ObjectId, ObjectId) {
        let target_script = r#"#strict
local turn_starts, turn_start_dir;
public func ReadDirection() { return GetDir(); }
protected func TurnStart()
{
    turn_starts = turn_starts + 1;
    turn_start_dir = GetDir();
    return 1;
}
"#;
        let mut target = Definition::from_script("PCDR", "Direction target", target_script)
            .expect("direction target compiles");
        target.set_c4_callback_convention(true);
        target.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        target.set_grab(1);
        target.set_mass(200);
        target.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Drive".to_string(),
                    ActionSpec::default()
                        .with_directions(2)
                        .with_turn_action("Turn"),
                ),
                (
                    "Turn".to_string(),
                    ActionSpec::default()
                        .with_directions(2)
                        .with_start_call("TurnStart"),
                ),
            ]),
        );
        engine
            .register_definition(target)
            .expect("direction target registers");

        let target_id = engine
            .spawn_object(
                SpawnConfig::new("PCDR")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(10, 0))
                    .with_action(ActionState::new(target_action))
                    .with_direction(Direction::Left)
                    // Raw positive xdir that still rounds to integer zero:
                    // Push/SetDir must inspect C4Fixed directly.
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::from_raw(12_345),
                        C4Fixed::ZERO,
                    ))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("direction target spawns");
        let mut push = ActionState::new("Push");
        push.target = Some(target_id);
        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("PCPS")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::ZERO)
                    .with_action(push)
                    .with_command_direction(CommandDirection::Right)
                    .with_loaded(true),
            )
            .expect("direction pusher spawns");
        (pusher_id, target_id)
    }

    #[test]
    fn push_keeps_an_idle_targets_direction() {
        // C4Object::Push calls SetDir from the target's pre-force raw xdir,
        // but SetDir rejects ActIdle. The positive xdir still receives the
        // push force; only Action.Dir remains Left/zero.
        let mut engine = push_containment_engine(true);
        let (pusher_id, target_id) = spawn_push_direction_case(&mut engine, "Idle");
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");

        engine
            .apply_physics_at_index(pusher_idx)
            .expect("idle-target push executes");

        let target_idx = engine.find_object_index(target_id).expect("target remains");
        assert!(engine.objects[target_idx].fixed_velocity.x.val() > 12_345);
        assert_eq!(engine.objects[target_idx].state.direction, Direction::Left);
        assert_eq!(
            engine
                .call_object_function(target_idx, "ReadDirection", Vec::new())
                .expect("GetDir reads the idle target"),
            Value::Int(0)
        );
    }

    #[test]
    fn push_runs_an_active_targets_turn_action_once() {
        // SetDir validates Drive's two directions, runs TurnAction before
        // assigning the new direction, then Push continues from the live
        // post-callback velocity.
        let mut engine = push_containment_engine(true);
        let (pusher_id, target_id) = spawn_push_direction_case(&mut engine, "Drive");
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");

        engine
            .apply_physics_at_index(pusher_idx)
            .expect("active-target push executes");

        let target_idx = engine.find_object_index(target_id).expect("target remains");
        assert_eq!(engine.objects[target_idx].state.action.name, "Turn");
        assert_eq!(engine.objects[target_idx].state.direction, Direction::Right);
        assert_eq!(
            engine.objects[target_idx].state.local_vars.get("turn_starts"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            engine.objects[target_idx]
                .state
                .local_vars
                .get("turn_start_dir"),
            Some(&Value::Int(0)),
            "TurnAction Start observes the old direction"
        );

        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher remains");
        engine
            .apply_physics_at_index(pusher_idx)
            .expect("same-facing push executes");
        let target_idx = engine.find_object_index(target_id).expect("target remains");
        assert_eq!(
            engine.objects[target_idx].state.local_vars.get("turn_starts"),
            Some(&Value::Int(1)),
            "the TurnAction runs only for the facing change"
        );
    }

    #[test]
    fn push_inside_action_target_stops_before_force_and_controller_transfer() {
        // DFA_PUSH checks no target first, then whether the PUSHER is inside
        // Action.Target, before calculating or applying any force
        // (C4Object.cpp:5058-5063). StopActionDelayCommand must leave the
        // existing stack below its pristine SilentSub Wait(50).
        let mut engine = push_containment_engine(true);
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("PCTG")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(10, 0))
                    .with_controller(3)
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::from_raw(12_345),
                        C4Fixed::ZERO,
                    ))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("target spawns");
        let mut push = ActionState::new("Push");
        push.target = Some(target_id);
        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("PCPS")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::ZERO)
                    .with_container(target_id)
                    .with_controller(7)
                    .with_action(push)
                    .with_command_direction(CommandDirection::Right)
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::from_raw(54_321),
                        C4Fixed::from_raw(7_654),
                    ))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("contained pusher spawns");
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        engine.objects[pusher_idx]
            .commands
            .push_back(
                CommandRequest::new(CommandId::MoveTo)
                    .with_tx(Some(20))
                    .with_ty(Some(0)),
            )
            .expect("tail command queues");

        assert!(
            engine
                .apply_physics_at_index(pusher_idx)
                .expect("inside-target Push resolves")
        );

        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher remains");
        let pusher = &engine.objects[pusher_idx];
        assert_eq!(pusher.state.action.name, "Walk");
        assert_eq!(pusher.state.command_direction, CommandDirection::Stop);
        assert_eq!(pusher.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(pusher.state.container, Some(target_id));
        assert_eq!(
            pusher.commands.command_names(),
            vec!["Wait".to_string(), "MoveTo".to_string()]
        );
        let stack = serde_json::to_value(pusher.commands.snapshot())
            .expect("command stack serializes");
        assert_eq!(stack["commands"][0]["mode"], serde_json::json!("SilentSub"));
        assert_eq!(stack["commands"][0]["update_interval"], serde_json::json!(50));

        let target_idx = engine.find_object_index(target_id).expect("target remains");
        assert_eq!(engine.objects[target_idx].fixed_velocity.x.val(), 12_345);
        assert_eq!(engine.objects[target_idx].state.controller, 3);
    }

    #[test]
    fn push_rejects_contained_target_on_zero_physical_fallback() {
        // C4Object::Push rejects every contained target before applying force
        // (C4Object.cpp:1785-1790). The zero-physical compatibility path does
        // not call push_object, so ExecAction must preserve that gate too.
        let mut engine = push_containment_engine(false);
        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("PCPS")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::ZERO)
                    .with_controller(7)
                    .with_action(ActionState::new("Push"))
                    .with_command_direction(CommandDirection::Right)
                    .with_loaded(true),
            )
            .expect("pusher spawns");
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("PCTG")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(10, 0))
                    .with_container(pusher_id)
                    .with_controller(3)
                    .with_fixed_velocity(FixedVec2::new(
                        C4Fixed::from_raw(12_345),
                        C4Fixed::ZERO,
                    ))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("contained target spawns");
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        engine.objects[pusher_idx].state.action.target = Some(target_id);

        assert!(
            engine
                .apply_physics_at_index(pusher_idx)
                .expect("contained-target Push resolves")
        );

        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher remains");
        let target_idx = engine.find_object_index(target_id).expect("target remains");
        assert_eq!(engine.objects[pusher_idx].state.action.name, "Walk");
        assert_eq!(
            engine.objects[pusher_idx].commands.command_names(),
            vec!["Wait".to_string()]
        );
        assert_eq!(engine.objects[target_idx].state.container, Some(pusher_id));
        assert_eq!(engine.objects[target_idx].fixed_velocity.x.val(), 12_345);
        assert_eq!(engine.objects[target_idx].state.controller, 3);
    }

    #[test]
    fn push_from_unrelated_container_still_applies_force_to_inactive_target() {
        // `Contained == Action.Target` is identity, not a generic contained
        // check. Being inside some other object must leave PUSH unchanged,
        // and C4Object::Push accepts every nonzero target Status.
        let mut engine = push_containment_engine(true);
        let unrelated_id = engine
            .spawn_object(
                SpawnConfig::new("PCTG")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(-30, 0))
                    .with_loaded(true),
            )
            .expect("unrelated container spawns");
        let target_id = engine
            .spawn_object(
                SpawnConfig::new("PCTG")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(10, 0))
                    .with_controller(3)
                    .with_loaded(true),
            )
            .expect("target spawns");
        let mut push = ActionState::new("Push");
        push.target = Some(target_id);
        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("PCPS")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::ZERO)
                    .with_container(unrelated_id)
                    .with_controller(7)
                    .with_action(push)
                    .with_command_direction(CommandDirection::Right)
                    .with_loaded(true),
            )
            .expect("pusher spawns");
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        engine.objects[target_idx].state.status = ObjectStatus::Inactive;

        engine
            .apply_physics_at_index(pusher_idx)
            .expect("unrelated-container Push executes");

        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher remains");
        let target_idx = engine.find_object_index(target_id).expect("target remains");
        assert_eq!(engine.objects[pusher_idx].state.action.name, "Push");
        assert_eq!(engine.objects[pusher_idx].state.container, Some(unrelated_id));
        assert_eq!(engine.objects[target_idx].state.container, None);
        assert!(engine.objects[pusher_idx].commands.is_empty());
        assert_eq!(engine.objects[pusher_idx].fixed_velocity.x.val(), 64_225);
        assert_eq!(engine.objects[target_idx].fixed_velocity.x.val(), 36_864);
        assert_eq!(engine.objects[target_idx].state.controller, 7);
    }

    #[test]
    fn push_procedure_moves_target_and_pusher() {
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
        pusher_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        pusher_actions.insert("Push".to_string(), ActionSpec::for_procedure("push"));
        pusher_definition.configure_actions(Some("Idle".to_string()), pusher_actions);
        pusher_definition.set_movement_profile(
            MovementProfile::default()
                .with_walk_speed(6)
                .with_walk_acceleration(3),
        );

        let mut target_definition = Definition::from_script("Crate", "Crate", script).unwrap();
        let mut target_actions = HashMap::new();
        target_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        target_definition.configure_actions(Some("Idle".to_string()), target_actions);

        let mut engine = Engine::with_seed(18);
        engine
            .register_definition(pusher_definition)
            .expect("pusher registers");
        engine
            .register_definition(target_definition)
            .expect("target registers");
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );

        let target_id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 0)),
            )
            .expect("target spawns");
        let target_initial_position = engine
            .object_snapshot(target_id)
            .expect("snapshot available")
            .position;

        let mut push_state = ActionState::new("Push");
        push_state.target = Some(target_id);

        let pusher_id = engine
            .spawn_object(
                SpawnConfig::new("Pusher")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0))
                    .with_action(push_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("pusher spawns");
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        engine.objects[pusher_idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[pusher_idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let pusher = snapshot
            .object(pusher_id)
            .expect("pusher present after tick");
        assert_eq!(pusher.action.name, "Push");
        assert!(pusher.velocity.x > 0, "pusher should move forward");
        assert_eq!(pusher.direction, Direction::Right);

        let target = snapshot
            .object(target_id)
            .expect("target present after tick");
        assert!(target.velocity.x >= 0);
        let pusher_idx = engine.find_object_index(pusher_id).expect("pusher exists");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        assert_eq!(engine.objects[pusher_idx].fixed_velocity.x.val(), 294912);
        assert_eq!(engine.objects[target_idx].fixed_velocity.x.val(), 196608);

        let snapshot = engine.tick().expect("second tick succeeds");
        let target_after = snapshot
            .object(target_id)
            .expect("target present after second tick");
        assert!(
            target_after.position.x > target_initial_position.x,
            "target should advance horizontally"
        );
    }

    #[test]
    fn pull_without_target_stops_in_walk_with_silent_wait() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Puller", "Puller", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Walk".to_string(), ActionSpec::for_procedure("walk"));
        actions.insert("Pull".to_string(), ActionSpec::for_procedure("pull"));
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(3);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let pull_state = ActionState::new("Pull");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Puller")
                    .with_action(pull_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("spawn succeeds");
        let index = engine.find_object_index(id).expect("puller exists");
        engine.objects[index]
            .set_fixed_velocity(FixedVec2::new(fixed100(125), fixed100(-75)));
        engine.objects[index]
            .commands
            .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
            .expect("tail command queues");

        assert!(
            engine
                .apply_physics_at_index(index)
                .expect("targetless Pull resolves")
        );

        let index = engine.find_object_index(id).expect("puller remains");
        let object = &engine.objects[index];
        assert_eq!(object.state.action.name, "Walk");
        assert_eq!(object.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(object.state.velocity, Vector2::ZERO);
        assert_eq!(object.state.command_direction, CommandDirection::Stop);
        assert_eq!(
            object.commands.command_names(),
            vec!["Wait".to_string(), "MoveTo".to_string()]
        );
        let stack = serde_json::to_value(object.commands.snapshot())
            .expect("command stack serializes");
        assert_eq!(stack["commands"][0]["mode"], serde_json::json!("SilentSub"));
        assert_eq!(stack["commands"][0]["update_interval"], serde_json::json!(50));
    }

    fn pull_failure_engine() -> Engine {
        let mut puller = Definition::from_script("L73P", "Puller", "#strict")
            .expect("puller compiles");
        puller.set_category(CATEGORY_OBJECT);
        puller.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        puller.set_physical(PhysicalInfo {
            walk: 35_000,
            push: 45_000,
            ..PhysicalInfo::default()
        });
        puller.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Walk".to_string(),
                    ActionSpec::default().with_procedure("WALK"),
                ),
                (
                    "Pull".to_string(),
                    ActionSpec::default().with_procedure("PULL"),
                ),
            ]),
        );

        let wagon_script = r#"#strict
local puller, action_seen;
public func Arm(object actor) { puller = actor; return true; }
protected func GrabLost()
{
    if (puller) action_seen = GetAction(puller);
    return true;
}
"#;
        let mut wagon = Definition::from_script("L73W", "Wagon", wagon_script)
            .expect("wagon compiles");
        wagon.set_category(CATEGORY_VEHICLE);
        wagon.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        wagon.set_grab(1);
        wagon.set_mass(200);

        let mut rejected = Definition::from_script("L73R", "Ungrabable wagon", "#strict")
            .expect("ungrabable target compiles");
        rejected.set_category(CATEGORY_VEHICLE);
        rejected.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        rejected.set_grab(0);
        rejected.set_mass(200);

        let container = Definition::from_script("L73C", "Container", "#strict")
            .expect("container compiles");

        let mut engine = Engine::with_seed(73);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(puller).expect("puller registers");
        engine.register_definition(wagon).expect("wagon registers");
        engine
            .register_definition(rejected)
            .expect("ungrabable target registers");
        engine
            .register_definition(container)
            .expect("container registers");
        engine
    }

    fn spawn_puller(
        engine: &mut Engine,
        target: ObjectId,
        position: Vector2,
        container: Option<ObjectId>,
        seed_tail: bool,
    ) -> ObjectId {
        let mut pull = ActionState::new("Pull");
        pull.target = Some(target);
        let mut config = SpawnConfig::new("L73P")
            .with_category(CATEGORY_OBJECT)
            .with_position(position)
            .with_controller(7)
            .with_action(pull)
            .with_command_direction(CommandDirection::Right)
            .with_fixed_velocity(FixedVec2::new(fixed100(125), fixed100(-75)))
            .with_mobile(true);
        if let Some(container) = container {
            config = config.with_container(container);
        }
        let puller = engine.spawn_object(config).expect("puller spawns");
        if seed_tail {
            let index = engine.find_object_index(puller).expect("puller exists");
            engine.objects[index]
                .commands
                .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
                .expect("tail command queues");
        }
        puller
    }

    fn assert_l073_pull_stopped(
        engine: &Engine,
        puller: ObjectId,
        expected_commands: &[&str],
        label: &str,
    ) {
        let index = engine.find_object_index(puller).expect("puller remains");
        let object = &engine.objects[index];
        assert_eq!(object.state.action.name, "Walk", "{label}");
        assert_eq!(
            object.state.command_direction,
            CommandDirection::Stop,
            "{label}"
        );
        assert_eq!(object.fixed_velocity, FixedVec2::ZERO, "{label}");
        assert_eq!(object.state.velocity, Vector2::ZERO, "{label}");
        assert_eq!(
            object.commands.command_names(),
            expected_commands
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>(),
            "{label}"
        );
        let stack = serde_json::to_value(object.commands.snapshot())
            .expect("command stack serializes");
        assert_eq!(
            stack["commands"][0]["mode"],
            serde_json::json!("SilentSub"),
            "{label}"
        );
        assert_eq!(
            stack["commands"][0]["update_interval"],
            serde_json::json!(50),
            "{label}"
        );
    }

    #[test]
    fn physical_pull_target_failures_stop_in_walk_with_silent_wait() {
        #[derive(Clone, Copy)]
        enum Failure {
            InsideTarget,
            TargetContained,
            PushRejected,
        }

        for (label, failure) in [
            ("puller inside target", Failure::InsideTarget),
            ("target contained", Failure::TargetContained),
            ("target Push rejected", Failure::PushRejected),
        ] {
            let mut engine = pull_failure_engine();
            let containing = matches!(failure, Failure::TargetContained)
                .then(|| {
                    engine
                        .spawn_object(SpawnConfig::new("L73C"))
                        .expect("container spawns")
                });
            let target_definition = if matches!(failure, Failure::PushRejected) {
                "L73R"
            } else {
                "L73W"
            };
            let mut target_config = SpawnConfig::new(target_definition)
                .with_category(CATEGORY_VEHICLE)
                .with_position(Vector2::new(10, 0))
                .with_controller(3);
            if let Some(container) = containing {
                target_config = target_config.with_container(container);
            }
            let target = engine
                .spawn_object(target_config)
                .expect("pull target spawns");
            let puller_container = matches!(failure, Failure::InsideTarget).then_some(target);
            let puller = spawn_puller(
                &mut engine,
                target,
                Vector2::ZERO,
                puller_container,
                true,
            );
            let index = engine.find_object_index(puller).expect("puller exists");

            let _ = engine
                .apply_physics_at_index(index)
                .unwrap_or_else(|error| panic!("{label}: Pull failed: {error}"));

            assert_l073_pull_stopped(&engine, puller, &["Wait", "MoveTo"], label);
        }
    }

    #[test]
    fn horse_like_pull_range_loss_stops_before_grab_lost() {
        let mut engine = pull_failure_engine();
        let wagon = engine
            .spawn_object(
                SpawnConfig::new("L73W")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::ZERO)
                    .with_controller(3),
            )
            .expect("wagon spawns");
        let horse = spawn_puller(
            &mut engine,
            wagon,
            Vector2::new(100, 0),
            None,
            true,
        );
        let wagon_index = engine.find_object_index(wagon).expect("wagon exists");
        engine
            .call_object_function(
                wagon_index,
                "Arm",
                vec![compat::object_reference_value(horse)],
            )
            .expect("wagon arms loss trace");

        engine.tick_without_snapshot().expect("horse loses the distant wagon");

        assert_l073_pull_stopped(
            &engine,
            horse,
            &["Wait", "MoveTo"],
            "horse range loss without PushTo",
        );
        let horse_index = engine.find_object_index(horse).expect("horse remains");
        assert_eq!(engine.objects[horse_index].state.action.target, None);
        let wagon_index = engine.find_object_index(wagon).expect("wagon remains");
        assert_eq!(engine.objects[wagon_index].state.controller, 7);
        assert_ne!(engine.objects[wagon_index].fixed_velocity.x, C4Fixed::ZERO);
        assert_eq!(
            engine.objects[wagon_index]
                .state
                .local_vars
                .get("action_seen"),
            Some(&Value::String("Walk".to_string().into())),
            "GrabLost observes StopActionDelayCommand first"
        );
    }

    #[test]
    fn pull_range_loss_clears_back_to_push_to() {
        let mut engine = pull_failure_engine();
        let wagon = engine
            .spawn_object(
                SpawnConfig::new("L73W")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::ZERO)
                    .with_controller(3),
            )
            .expect("wagon spawns");
        let horse = spawn_puller(
            &mut engine,
            wagon,
            Vector2::new(100, 0),
            None,
            false,
        );
        let wagon_index = engine.find_object_index(wagon).expect("wagon exists");
        engine
            .call_object_function(
                wagon_index,
                "Arm",
                vec![compat::object_reference_value(horse)],
            )
            .expect("wagon arms loss trace");

        let horse_index = engine.find_object_index(horse).expect("horse exists");
        let commands = &mut engine.objects[horse_index].commands;
        commands
            .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
            .expect("approach queues");
        commands
            .push_back(CommandRequest::new(CommandId::PushTo).with_target(Some(wagon)))
            .expect("PushTo queues");
        commands
            .push_back(CommandRequest::new(CommandId::Wait).with_update_interval(90))
            .expect("tail Wait queues");

        assert!(
            engine
                .apply_physics_at_index(horse_index)
                .expect("horse loses the distant wagon")
        );

        let horse_index = engine.find_object_index(horse).expect("horse remains");
        let horse = &engine.objects[horse_index];
        assert_eq!(horse.state.action.name, "Walk");
        assert_eq!(horse.state.command_direction, CommandDirection::Stop);
        assert_eq!(horse.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(horse.state.velocity, Vector2::ZERO);
        assert_eq!(horse.state.action.target, None);
        assert_eq!(
            horse.commands.command_names(),
            vec!["PushTo", "Wait"],
            "GrabLost removes the new delay and approach but preserves PushTo's tail"
        );
        let wagon_index = engine.find_object_index(wagon).expect("wagon remains");
        assert_eq!(
            engine.objects[wagon_index]
                .state
                .local_vars
                .get("action_seen"),
            Some(&Value::String("Walk".to_string().into())),
            "StopActionDelayCommand precedes GrabLost"
        );
    }

    #[test]
    fn pull_procedure_moves_target_and_puller() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut puller_definition = Definition::from_script("Puller", "Puller", script).unwrap();
        let mut puller_actions = HashMap::new();
        puller_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        puller_actions.insert("Pull".to_string(), ActionSpec::for_procedure("pull"));
        puller_definition.configure_actions(Some("Idle".to_string()), puller_actions);
        puller_definition.set_movement_profile(
            MovementProfile::default()
                .with_walk_speed(6)
                .with_walk_acceleration(3),
        );

        let mut target_definition = Definition::from_script("Crate", "Crate", script).unwrap();
        let mut target_actions = HashMap::new();
        target_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        target_definition.configure_actions(Some("Idle".to_string()), target_actions);

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(puller_definition)
            .expect("puller registers");
        engine
            .register_definition(target_definition)
            .expect("target registers");
        engine.set_physics(
            PhysicsSettings::new(0, 20, -20)
                .with_max_horizontal_speed(20)
                .expect("horizontal speed valid"),
        );

        let vertices = vec![
            ObjectVertex::new(-10, -10),
            ObjectVertex::new(10, -10),
            ObjectVertex::new(10, 10),
            ObjectVertex::new(-10, 10),
        ];

        let target_id = engine
            .spawn_object(
                SpawnConfig::new("Crate")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0))
                    .with_vertices(vertices.clone()),
            )
            .expect("target spawns");
        let target_initial_position = engine
            .object_snapshot(target_id)
            .expect("target snapshot available")
            .position;

        let mut pull_state = ActionState::new("Pull");
        pull_state.target = Some(target_id);

        let puller_id = engine
            .spawn_object(
                SpawnConfig::new("Puller")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 0))
                    .with_vertices(vertices)
                    .with_action(pull_state)
                    .with_command_direction(CommandDirection::Right),
            )
            .expect("puller spawns");
        let puller_idx = engine.find_object_index(puller_id).expect("puller exists");
        engine.objects[puller_idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[puller_idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let puller = snapshot
            .object(puller_id)
            .expect("puller present after tick");
        assert_eq!(puller.action.name, "Pull");
        assert!(puller.velocity.x > 0, "puller should move forward");
        assert_eq!(puller.direction, Direction::Right);

        let target = snapshot
            .object(target_id)
            .expect("target present after tick");
        assert!(target.velocity.x >= 0);
        let puller_idx = engine.find_object_index(puller_id).expect("puller exists");
        let target_idx = engine.find_object_index(target_id).expect("target exists");
        assert_eq!(engine.objects[puller_idx].fixed_velocity.x.val(), 294912);
        assert_eq!(engine.objects[target_idx].fixed_velocity.x.val(), 196608);

        let snapshot = engine.tick().expect("second tick succeeds");
        let target_after = snapshot
            .object(target_id)
            .expect("target present after second tick");
        assert!(
            target_after.position.x > target_initial_position.x,
            "target should advance horizontally",
        );
    }

    fn fight_failure_engine() -> Engine {
        let mut fighter = Definition::from_script("L73F", "Fighter", "#strict")
            .expect("fighter compiles");
        fighter.set_category(CATEGORY_OBJECT);
        fighter.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        fighter.set_shape_vertices(vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ]);
        fighter.set_physical(PhysicalInfo {
            walk: 35_000,
            ..PhysicalInfo::default()
        });
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
            ]),
        );

        let mut opponent = Definition::from_script("L73O", "Opponent", "#strict")
            .expect("opponent compiles");
        opponent.set_category(CATEGORY_OBJECT);
        opponent.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        opponent.set_shape_vertices(vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ]);
        opponent.configure_actions(
            Some("Fight".to_string()),
            HashMap::from([(
                "Fight".to_string(),
                ActionSpec::default().with_procedure("FIGHT"),
            )]),
        );

        let mut passive = Definition::from_script("L73N", "Passive", "#strict")
            .expect("passive target compiles");
        passive.set_category(CATEGORY_OBJECT);
        passive.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        passive.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([("Idle".to_string(), ActionSpec::default())]),
        );

        let container = Definition::from_script("L73D", "Closed container", "#strict")
            .expect("container compiles");

        let mut engine = Engine::with_seed(73);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine.register_definition(fighter).expect("fighter registers");
        engine
            .register_definition(opponent)
            .expect("opponent registers");
        engine
            .register_definition(passive)
            .expect("passive target registers");
        engine
            .register_definition(container)
            .expect("container registers");
        engine
    }

    fn spawn_fighter(
        engine: &mut Engine,
        target: Option<ObjectId>,
        container: Option<ObjectId>,
    ) -> ObjectId {
        let mut fight = ActionState::new("Fight");
        fight.target = target;
        let mut config = SpawnConfig::new("L73F")
            .with_category(CATEGORY_OBJECT)
            .with_position(Vector2::ZERO)
            .with_action(fight)
            .with_command_direction(CommandDirection::Right)
            .with_fixed_velocity(FixedVec2::new(fixed100(125), fixed100(-75)))
            .with_mobile(true);
        if let Some(container) = container {
            config = config.with_container(container);
        }
        let fighter = engine.spawn_object(config).expect("fighter spawns");
        let index = engine.find_object_index(fighter).expect("fighter exists");
        engine.objects[index]
            .commands
            .push_back(CommandRequest::new(CommandId::MoveTo).with_tx(Some(20)))
            .expect("tail command queues");
        fighter
    }

    fn assert_l073_fighter_stands(engine: &Engine, fighter: ObjectId, label: &str) {
        let index = engine.find_object_index(fighter).expect("fighter remains");
        let object = &engine.objects[index];
        assert_eq!(object.state.action.name, "Walk", "{label}");
        assert_eq!(
            object.state.command_direction,
            CommandDirection::Stop,
            "{label}"
        );
        assert_eq!(object.fixed_velocity, FixedVec2::ZERO, "{label}");
        assert_eq!(object.state.velocity, Vector2::ZERO, "{label}");
        assert_eq!(
            object.commands.command_names(),
            vec!["MoveTo".to_string()],
            "FIGHT failure must not add PULL's delayed Wait: {label}"
        );
    }

    #[test]
    fn fight_without_target_stands_in_walk_without_wait() {
        let mut engine = fight_failure_engine();
        let fighter = spawn_fighter(&mut engine, None, None);
        let index = engine.find_object_index(fighter).expect("fighter exists");

        assert!(
            engine
                .apply_physics_at_index(index)
                .expect("targetless Fight resolves")
        );

        assert_l073_fighter_stands(&engine, fighter, "no target");
    }

    #[test]
    fn fight_target_door_and_range_failures_stand_without_wait() {
        #[derive(Clone, Copy)]
        enum Failure {
            TargetNotFighting,
            FighterBehindClosedDoor,
            TargetBehindClosedDoor,
            OutOfRange,
        }

        for (label, failure) in [
            ("target not fighting", Failure::TargetNotFighting),
            (
                "fighter behind closed door",
                Failure::FighterBehindClosedDoor,
            ),
            (
                "target behind closed door",
                Failure::TargetBehindClosedDoor,
            ),
            ("fight target out of range", Failure::OutOfRange),
        ] {
            let mut engine = fight_failure_engine();
            let closed_container = matches!(
                failure,
                Failure::FighterBehindClosedDoor | Failure::TargetBehindClosedDoor
            )
            .then(|| {
                engine
                    .spawn_object(
                        SpawnConfig::new("L73D")
                            .with_position(Vector2::ZERO)
                            .with_entrance_status(false),
                    )
                    .expect("closed container spawns")
            });
            let target_definition = if matches!(failure, Failure::TargetNotFighting) {
                "L73N"
            } else {
                "L73O"
            };
            let target_position = if matches!(failure, Failure::OutOfRange) {
                Vector2::new(40, 0)
            } else {
                Vector2::new(10, 0)
            };
            let mut target_config = SpawnConfig::new(target_definition)
                .with_category(CATEGORY_OBJECT)
                .with_position(target_position);
            if matches!(failure, Failure::TargetBehindClosedDoor) {
                target_config = target_config.with_container(
                    closed_container.expect("target has a closed container"),
                );
            }
            if target_definition == "L73O" {
                target_config = target_config.with_action(ActionState::new("Fight"));
            }
            let target = engine
                .spawn_object(target_config)
                .expect("fight target spawns");
            let fighter_container = if matches!(failure, Failure::FighterBehindClosedDoor) {
                closed_container
            } else {
                None
            };
            let fighter = spawn_fighter(&mut engine, Some(target), fighter_container);
            let index = engine.find_object_index(fighter).expect("fighter exists");

            let _ = engine
                .apply_physics_at_index(index)
                .unwrap_or_else(|error| panic!("{label}: Fight failed: {error}"));

            assert_l073_fighter_stands(&engine, fighter, label);
        }
    }

    #[test]
    fn fight_continues_through_open_container_mismatches() {
        #[derive(Clone, Copy)]
        enum ContainerCase {
            FighterInside,
            TargetInside,
            BothInsideDifferent,
            BothInsideSameClosed,
        }

        for (label, case) in [
            ("fighter inside an open container", ContainerCase::FighterInside),
            ("target inside an open container", ContainerCase::TargetInside),
            (
                "fighters inside different open containers",
                ContainerCase::BothInsideDifferent,
            ),
            (
                "fighters inside the same closed container",
                ContainerCase::BothInsideSameClosed,
            ),
        ] {
            let mut engine = fight_failure_engine();
            let spawn_container = |engine: &mut Engine, entrance_status| {
                engine
                    .spawn_object(
                        SpawnConfig::new("L73D")
                            .with_position(Vector2::ZERO)
                            .with_entrance_status(entrance_status),
                    )
                    .expect("container spawns")
            };
            let (fighter_container, target_container) = match case {
                ContainerCase::FighterInside => {
                    (Some(spawn_container(&mut engine, true)), None)
                }
                ContainerCase::TargetInside => {
                    (None, Some(spawn_container(&mut engine, true)))
                }
                ContainerCase::BothInsideDifferent => (
                    Some(spawn_container(&mut engine, true)),
                    Some(spawn_container(&mut engine, true)),
                ),
                ContainerCase::BothInsideSameClosed => {
                    let container = spawn_container(&mut engine, false);
                    (Some(container), Some(container))
                }
            };

            let mut target_config = SpawnConfig::new("L73O")
                .with_category(CATEGORY_OBJECT)
                .with_position(Vector2::new(10, 0))
                .with_action(ActionState::new("Fight"));
            if let Some(container) = target_container {
                target_config = target_config.with_container(container);
            }
            let target = engine
                .spawn_object(target_config)
                .expect("fight target spawns");
            let fighter = spawn_fighter(&mut engine, Some(target), fighter_container);
            let index = engine.find_object_index(fighter).expect("fighter exists");

            assert!(
                !engine
                    .apply_physics_at_index(index)
                    .unwrap_or_else(|error| panic!("{label}: Fight failed: {error}")),
                "a continuing Fight must reach the normal ExecAction phase tail: {label}"
            );

            let fighter = &engine.objects[index];
            assert_eq!(fighter.state.action.name, "Fight", "{label}");
        }
    }

    #[test]
    fn fight_procedure_retains_inactive_action_target_like_cpp() {
        let mut engine = fight_failure_engine();
        let target = engine
            .spawn_object(
                SpawnConfig::new("L73O")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 0))
                    .with_action(ActionState::new("Fight")),
            )
            .expect("fight target spawns");
        let target_index = engine.find_object_index(target).expect("target exists");
        engine.objects[target_index].state.status = ObjectStatus::Inactive;
        let fighter = spawn_fighter(&mut engine, Some(target), None);
        let fighter_index = engine.find_object_index(fighter).expect("fighter exists");

        assert!(
            !engine
                .apply_physics_at_index(fighter_index)
                .expect("Fight with inactive target executes"),
            "the retained fight reaches the ordinary phase tail"
        );
        assert_eq!(
            engine.objects[fighter_index].state.action.name,
            "Fight",
            "C4OS_INACTIVE does not clear Action.Target"
        );
    }

    fn wide_vertex_fight_pair(separation: i32) -> (Engine, ObjectId, ObjectId) {
        let mut engine = fight_failure_engine();
        // Deliberately disagree with the 16px shape rect. DFA_FIGHT uses the
        // live Shape.Wdt for both its approach point and give-up distance,
        // never the span of the contact vertices.
        let wide_vertices = vec![
            ObjectVertex::new(-20, -8),
            ObjectVertex::new(20, -8),
            ObjectVertex::new(20, 8),
            ObjectVertex::new(-20, 8),
        ];
        let target = engine
            .spawn_object(
                SpawnConfig::new("L73O")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(separation, 0))
                    .with_vertices(wide_vertices.clone())
                    .with_action(ActionState::new("Fight")),
            )
            .expect("fight target spawns");
        let mut fight = ActionState::new("Fight");
        fight.target = Some(target);
        let fighter = engine
            .spawn_object(
                SpawnConfig::new("L73F")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::ZERO)
                    .with_vertices(wide_vertices)
                    .with_action(fight),
            )
            .expect("fighter spawns");
        (engine, fighter, target)
    }

    #[test]
    fn fight_approach_uses_target_shape_rect_width() {
        let (mut engine, fighter, target) = wide_vertex_fight_pair(10);
        let fighter_idx = engine.find_object_index(fighter).expect("fighter exists");
        let target_idx = engine.find_object_index(target).expect("target exists");
        engine.objects[fighter_idx].state.shape_override =
            Some(DefinitionRect::new(-6, -8, 12, 16));
        assert_eq!(
            engine.objects[fighter_idx]
                .current_shape_rect()
                .expect("fighter has a live shape rect")
                .width,
            12,
            "the approach point must not use the fighter width"
        );
        assert_eq!(
            engine.objects[target_idx]
                .current_shape_rect()
                .expect("target has a live shape rect")
                .width,
            16
        );

        let _ = engine
            .apply_physics_at_index(fighter_idx)
            .expect("Fight equilibrium resolves");

        let fighter = &engine.objects[fighter_idx];
        assert_eq!(fighter.state.action.name, "Fight");
        assert_eq!(fighter.state.direction, Direction::Right);
        assert_eq!(
            fighter.fixed_velocity.x,
            C4Fixed::ZERO,
            "target x=10 and Shape.Wdt=16 put the right-facing equilibrium at x=0"
        );
    }

    #[test]
    fn fight_give_up_uses_inclusive_own_shape_rect_width() {
        for (target_width, separation, expected_action) in [
            (16, 16, "Fight"),
            (16, 17, "Walk"),
            (32, 16, "Fight"),
            (32, 17, "Walk"),
        ] {
            let (mut engine, fighter, target) = wide_vertex_fight_pair(separation);
            let fighter_idx = engine.find_object_index(fighter).expect("fighter exists");
            let target_idx = engine.find_object_index(target).expect("target exists");
            engine.objects[target_idx].state.shape_override = Some(DefinitionRect::new(
                -target_width / 2,
                -8,
                target_width,
                16,
            ));
            assert_eq!(
                engine.objects[fighter_idx]
                    .current_shape_rect()
                    .expect("fighter has a live shape rect")
                    .width,
                16
            );
            assert_eq!(
                engine.objects[target_idx]
                    .current_shape_rect()
                    .expect("target has a live shape rect")
                    .width,
                target_width
            );

            let _ = engine
                .apply_physics_at_index(fighter_idx)
                .unwrap_or_else(|error| panic!("Fight separation {separation} failed: {error}"));

            let fighter = &engine.objects[fighter_idx];
            assert_eq!(
                fighter.state.action.name, expected_action,
                "own Shape.Wdt=16 keeps distance 16 and gives up at 17, independent of target width {target_width}"
            );
            if separation == 17 {
                assert_eq!(fighter.state.command_direction, CommandDirection::Stop);
                assert_eq!(fighter.fixed_velocity, FixedVec2::ZERO);
                assert_eq!(fighter.state.velocity, Vector2::ZERO);
            }
        }
    }

    fn attach_actor_definition(
        id: &str,
        script: &str,
        abort_call: Option<&str>,
    ) -> Definition {
        let mut definition = Definition::from_script(id, id, script).expect("actor compiles");
        definition.set_category(CATEGORY_OBJECT);
        definition.set_c4_callback_convention(true);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        let mut attach = ActionSpec::default().with_procedure("ATTACH");
        if let Some(abort_call) = abort_call {
            attach = attach.with_abort_call(abort_call);
        }
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("Attach".to_string(), attach),
                ("Marker".to_string(), ActionSpec::default()),
            ]),
        );
        definition
    }

    fn point_definition(id: &str, script: &str) -> Definition {
        let mut definition = Definition::from_script(id, id, script).expect("point compiles");
        definition.set_category(CATEGORY_OBJECT);
        definition.set_c4_callback_convention(true);
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        definition
    }

    #[test]
    fn attach_lost_target_sets_idle_before_lost_callback() {
        let script = r#"#strict 2
local callback_order, abort_action, lost_action;

protected func AttachAbort(int old_phase)
{
    callback_order = callback_order * 10 + 1;
    abort_action = GetAction();
    return 1;
}

protected func AttachTargetLost()
{
    callback_order = callback_order * 10 + 2;
    lost_action = GetAction();
    SetAction("Marker");
    return 1;
}
"#;
        let mut engine = Engine::with_seed(75);
        engine
            .register_definition(attach_actor_definition(
                "L75A",
                script,
                Some("AttachAbort"),
            ))
            .expect("actor registers");
        let actor = engine
            .spawn_object(
                SpawnConfig::new("L75A")
                    .with_action(ActionState::new("Attach"))
                    .with_loaded(true),
            )
            .expect("actor spawns");
        let index = engine.find_object_index(actor).expect("actor exists");

        assert!(
            engine
                .apply_physics_at_index(index)
                .expect("lost Attach target resolves")
        );

        let index = engine.find_object_index(actor).expect("actor remains");
        let object = &engine.objects[index];
        assert_eq!(
            object.state.local_vars.get("callback_order"),
            Some(&Value::Int(12)),
            "SetAction(ActIdle)'s Attach AbortCall precedes AttachTargetLost"
        );
        assert_eq!(
            object.state.local_vars.get("abort_action"),
            Some(&Value::String("Idle".to_string().into()))
        );
        assert_eq!(
            object.state.local_vars.get("lost_action"),
            Some(&Value::String("Idle".to_string().into()))
        );
        assert_eq!(
            object.state.action.name, "Marker",
            "AttachTargetLost runs after the Idle transition and may replace it"
        );
    }

    #[test]
    fn attach_incomplete_target_respects_incomplete_activity() {
        let actor_script = r#"#strict 2
local lost_calls;
protected func AttachTargetLost() { lost_calls += 1; return 1; }
"#;
        let mut blocked = point_definition("L75N", "#strict 2");
        blocked.set_incomplete_activity(false);
        let mut allowed = point_definition("L75Y", "#strict 2");
        allowed.set_incomplete_activity(true);

        let mut engine = Engine::with_seed(75);
        engine
            .register_definition(attach_actor_definition("L75I", actor_script, None))
            .expect("actor registers");
        engine
            .register_definition(blocked)
            .expect("blocked target registers");
        engine
            .register_definition(allowed)
            .expect("allowed target registers");

        for (target_definition, permits_attach, offset) in
            [("L75N", false, 0), ("L75Y", true, 20)]
        {
            let target_position = Vector2::new(50 + offset, 60);
            let target = engine
                .spawn_object(
                    SpawnConfig::new(target_definition)
                        .with_position(target_position)
                        .with_construction(FULL_CON / 2)
                        .with_loaded(true),
                )
                .expect("partial target spawns");
            let actor_position = Vector2::new(5 + offset, 6);
            let mut attach = ActionState::new("Attach");
            attach.target = Some(target);
            let actor = engine
                .spawn_object(
                    SpawnConfig::new("L75I")
                        .with_position(actor_position)
                        .with_action(attach)
                        .with_loaded(true),
                )
                .expect("actor spawns");
            let index = engine.find_object_index(actor).expect("actor exists");

            let _ = engine
                .apply_physics_at_index(index)
                .expect("partial-target Attach resolves");

            let index = engine.find_object_index(actor).expect("actor remains");
            let object = &engine.objects[index];
            assert_eq!(
                object.state.local_vars.get("lost_calls"),
                None,
                "an extant incomplete target is not a lost target"
            );
            if permits_attach {
                assert_eq!(object.state.action.name, "Attach");
                assert_eq!(object.state.position, target_position);
            } else {
                assert_eq!(object.state.action.name, "Idle");
                assert_eq!(object.state.position, actor_position);
                assert_eq!(
                    object.state.action.target,
                    Some(target),
                    "SetAction(ActIdle) preserves an unsupplied target"
                );
            }
        }
    }

    #[test]
    fn attach_forced_enter_callbacks_recheck_cleared_target() {
        let actor_script = r#"#strict 2
local callback_order, lost_action;

public func Mark(int step)
{
    callback_order = callback_order * 10 + step;
    return 1;
}

protected func RejectEntrance(object container)
{
    Mark(1);
    return 0;
}

protected func Entrance(object container)
{
    Mark(3);
    SetActionTargets();
    return 1;
}

protected func AttachTargetLost()
{
    Mark(4);
    lost_action = GetAction();
    return 1;
}
"#;
        let container_script = r#"#strict 2
protected func Collection2(object item) { item->Mark(2); return 1; }
"#;
        let mut engine = Engine::with_seed(75);
        engine
            .register_definition(attach_actor_definition("L75E", actor_script, None))
            .expect("actor registers");
        engine
            .register_definition(point_definition("L75C", container_script))
            .expect("container registers");
        engine
            .register_definition(point_definition("L75T", "#strict 2"))
            .expect("target registers");

        let container = engine
            .spawn_object(SpawnConfig::new("L75C"))
            .expect("container spawns");
        let target_position = Vector2::new(80, 90);
        let target = engine
            .spawn_object(
                SpawnConfig::new("L75T")
                    .with_position(target_position)
                    .with_container(container)
                    .with_loaded(true),
            )
            .expect("contained target spawns");
        let actor_position = Vector2::new(5, 6);
        let mut attach = ActionState::new("Attach");
        attach.target = Some(target);
        let actor = engine
            .spawn_object(
                SpawnConfig::new("L75E")
                    .with_position(actor_position)
                    .with_action(attach)
                    .with_loaded(true),
            )
            .expect("actor spawns");
        let index = engine.find_object_index(actor).expect("actor exists");

        let _ = engine
            .apply_physics_at_index(index)
            .expect("forced Enter resolves");

        let index = engine.find_object_index(actor).expect("actor remains");
        let object = &engine.objects[index];
        assert_eq!(object.state.container, Some(container));
        assert_eq!(
            object.state.local_vars.get("callback_order"),
            Some(&Value::Int(1234)),
            "RejectEntrance -> Collection2 -> Entrance -> AttachTargetLost"
        );
        assert_eq!(
            object.state.local_vars.get("lost_action"),
            Some(&Value::String("Idle".to_string().into()))
        );
        assert_eq!(object.state.action.name, "Idle");
        assert_eq!(object.state.action.target, None);
        assert_eq!(
            object.state.position,
            Vector2::ZERO,
            "Enter copies the container motion, and clearing the target prevents a later stale force-position"
        );
    }

    #[test]
    fn attach_forced_exit_runs_ejection_and_departure() {
        let actor_script = r#"#strict 2
local callback_order;
public func Mark(int step) { callback_order = callback_order * 10 + step; return 1; }
protected func Departure(object container) { Mark(2); return 1; }
"#;
        let container_script = r#"#strict 2
protected func Ejection(object item) { item->Mark(1); return 1; }
"#;
        let mut engine = Engine::with_seed(75);
        engine
            .register_definition(attach_actor_definition("L75X", actor_script, None))
            .expect("actor registers");
        engine
            .register_definition(point_definition("L75O", container_script))
            .expect("old container registers");
        engine
            .register_definition(point_definition("L75U", "#strict 2"))
            .expect("target registers");

        let old_container = engine
            .spawn_object(SpawnConfig::new("L75O"))
            .expect("old container spawns");
        let target_position = Vector2::new(70, 80);
        let target = engine
            .spawn_object(
                SpawnConfig::new("L75U")
                    .with_position(target_position)
                    .with_loaded(true),
            )
            .expect("uncontained target spawns");
        let mut attach = ActionState::new("Attach");
        attach.target = Some(target);
        let actor = engine
            .spawn_object(
                SpawnConfig::new("L75X")
                    .with_position(Vector2::new(7, 8))
                    .with_rotation(37)
                    .with_fixed_rotation(itofix(37))
                    .with_rotation_velocity(itofix(4))
                    .with_container(old_container)
                    .with_action(attach)
                    .with_loaded(true),
            )
            .expect("contained actor spawns");
        let index = engine.find_object_index(actor).expect("actor exists");

        let _ = engine
            .apply_physics_at_index(index)
            .expect("forced Exit resolves");

        let index = engine.find_object_index(actor).expect("actor remains");
        let object = &engine.objects[index];
        assert_eq!(object.state.container, None);
        assert_eq!(
            object.state.local_vars.get("callback_order"),
            Some(&Value::Int(12)),
            "Ejection precedes Departure"
        );
        assert_eq!(object.state.action.name, "Attach");
        assert_eq!(object.state.action.target, Some(target));
        assert_eq!(object.state.position, target_position);
        assert_eq!(object.state.rotation, 37);
        assert_eq!(object.fixed_rotation, itofix(37));
        assert_eq!(object.rotation_velocity, C4Fixed::ZERO);
        assert_eq!(object.fixed_velocity, FixedVec2::ZERO);
    }

    #[test]
    fn fight_procedure_moves_toward_target() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut fighter_definition = Definition::from_script("Fighter", "Fighter", script).unwrap();
        fighter_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        let mut fighter_actions = HashMap::new();
        fighter_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        fighter_actions.insert("Fight".to_string(), ActionSpec::for_procedure("fight"));
        fighter_definition.configure_actions(Some("Idle".to_string()), fighter_actions);
        // DFA_FIGHT approaches with the Walk physical (C4Object.cpp:5225-5228),
        // not the movement profile. 35000 is the stock Clonk DefCore value.
        fighter_definition.set_physical(PhysicalInfo {
            walk: 35_000,
            ..PhysicalInfo::default()
        });

        let mut opponent_definition =
            Definition::from_script("Opponent", "Opponent", script).unwrap();
        opponent_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        let mut opponent_actions = HashMap::new();
        opponent_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        opponent_actions.insert("Fight".to_string(), ActionSpec::for_procedure("fight"));
        opponent_definition.configure_actions(Some("Idle".to_string()), opponent_actions);

        let mut engine = Engine::with_seed(33);
        engine
            .register_definition(fighter_definition)
            .expect("fighter definition registers");
        engine
            .register_definition(opponent_definition)
            .expect("opponent definition registers");
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

        let opponent_id = engine
            .spawn_object(
                SpawnConfig::new("Opponent")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(12, 0))
                    .with_vertices(vertices.clone())
                    .with_action(ActionState::new("Fight")),
            )
            .expect("opponent spawns");

        let mut fight_state = ActionState::new("Fight");
        fight_state.target = Some(opponent_id);
        let fighter_id = engine
            .spawn_object(
                SpawnConfig::new("Fighter")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0))
                    .with_vertices(vertices.clone())
                    .with_action(fight_state),
            )
            .expect("fighter spawns");
        let fighter_idx = engine
            .find_object_index(fighter_id)
            .expect("fighter exists");
        engine.objects[fighter_idx]
            .set_fixed_velocity(FixedVec2::new(C4Fixed::from_raw(98304), C4Fixed::ZERO));
        // dir writes mobilize (FnSetXDir/FnSetYDir, C4Script.cpp:705,732)
        engine.objects[fighter_idx].state.mobile = true;

        engine
            .apply_object_update(
                opponent_id,
                ObjectUpdate::new()
                    .with_action_update(ActionUpdate::default().with_target(Some(fighter_id))),
            )
            .expect("opponent target update succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        let fighter = snapshot
            .object(fighter_id)
            .expect("fighter present after tick");
        assert_eq!(fighter.action.name, "Fight");
        assert!(
            fighter.velocity.x > 0,
            "fighter should advance towards the opponent"
        );
        assert_eq!(fighter.direction, Direction::Right);
        assert_eq!(fighter.velocity.y, 0);
        assert!(
            fighter.position.x > 0,
            "fighter should have moved horizontally"
        );
        let fighter_idx = engine
            .find_object_index(fighter_id)
            .expect("fighter exists");
        // C4Object.cpp:5221-5228: facing Right, stand-beside target_x at
        // 12 - 16/2 - 2 = 2; lLimit = ValByPhysical(95, 35000)
        // = itofix(35000*19, 2000000) = raw 21790; Towards steps the initial
        // raw 98304 down by one lLimit: 98304 - 21790 = 76514.
        assert_eq!(engine.objects[fighter_idx].fixed_velocity.x.val(), 76514);
    }

    #[test]
    fn fight_procedure_stands_when_target_not_fighting() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut fighter_definition = Definition::from_script("Fighter", "Fighter", script).unwrap();
        let mut fighter_actions = HashMap::new();
        fighter_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        fighter_actions.insert("Walk".to_string(), ActionSpec::for_procedure("walk"));
        fighter_actions.insert("Fight".to_string(), ActionSpec::for_procedure("fight"));
        fighter_definition.configure_actions(Some("Idle".to_string()), fighter_actions);
        fighter_definition.set_movement_profile(
            MovementProfile::default()
                .with_walk_speed(6)
                .with_walk_acceleration(3),
        );

        let mut passive_definition = Definition::from_script("Passive", "Passive", script).unwrap();
        let mut passive_actions = HashMap::new();
        passive_actions.insert("Idle".to_string(), ActionSpec::for_procedure("walk"));
        passive_definition.configure_actions(Some("Idle".to_string()), passive_actions);

        let mut engine = Engine::with_seed(41);
        engine
            .register_definition(fighter_definition)
            .expect("fighter definition registers");
        engine
            .register_definition(passive_definition)
            .expect("passive definition registers");

        let vertices = vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ];

        let passive_id = engine
            .spawn_object(
                SpawnConfig::new("Passive")
                    .with_position(Vector2::new(10, 0))
                    .with_vertices(vertices.clone())
                    .with_action(ActionState::new("Idle")),
            )
            .expect("passive target spawns");

        let mut fight_state = ActionState::new("Fight");
        fight_state.target = Some(passive_id);
        let fighter_id = engine
            .spawn_object(
                SpawnConfig::new("Fighter")
                    .with_position(Vector2::new(0, 0))
                    .with_vertices(vertices)
                    .with_action(fight_state),
            )
            .expect("fighter spawns");

        let snapshot = engine.tick().expect("tick succeeds");
        let fighter = snapshot
            .object(fighter_id)
            .expect("fighter present after tick");
        assert_eq!(fighter.action.name, "Walk");
        assert_eq!(fighter.velocity, Vector2::ZERO);
    }

    #[test]
    fn fight_procedure_trains_fight_physical_on_tick5() {
        let script = r#"
        global func Initialize(state, random) {
            return 0;
        }

        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut fighter_definition = Definition::from_script("Fighter", "Fighter", script).unwrap();
        fighter_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        let mut fighter_actions = HashMap::new();
        fighter_actions.insert("Fight".to_string(), ActionSpec::for_procedure("fight"));
        fighter_definition.configure_actions(Some("Fight".to_string()), fighter_actions);
        fighter_definition.set_physical(PhysicalInfo {
            walk: 35_000,
            fight: 20_000,
            ..PhysicalInfo::default()
        });

        let mut opponent_definition =
            Definition::from_script("Opponent", "Opponent", script).unwrap();
        opponent_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        let mut opponent_actions = HashMap::new();
        opponent_actions.insert("Fight".to_string(), ActionSpec::for_procedure("fight"));
        opponent_definition.configure_actions(Some("Fight".to_string()), opponent_actions);

        let mut engine = Engine::with_seed(33);
        engine
            .register_definition(fighter_definition)
            .expect("fighter definition registers");
        engine
            .register_definition(opponent_definition)
            .expect("opponent definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let vertices = vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ];

        let opponent_id = engine
            .spawn_object(
                SpawnConfig::new("Opponent")
                    .with_position(Vector2::new(12, 0))
                    .with_vertices(vertices.clone())
                    .with_action(ActionState::new("Fight")),
            )
            .expect("opponent spawns");
        let mut fight_state = ActionState::new("Fight");
        fight_state.target = Some(opponent_id);
        let fighter_id = engine
            .spawn_object(
                SpawnConfig::new("Fighter")
                    .with_position(Vector2::new(0, 0))
                    .with_vertices(vertices)
                    .with_action(fight_state),
            )
            .expect("fighter spawns");
        let fighter_idx = engine
            .find_object_index(fighter_id)
            .expect("fighter exists");
        engine.objects[fighter_idx].state.temporary_physical = Some(PhysicalInfo {
            walk: 35_000,
            fight: 20_000,
            ..PhysicalInfo::default()
        });
        engine
            .apply_object_update(
                opponent_id,
                ObjectUpdate::new()
                    .with_action_update(ActionUpdate::default().with_target(Some(fighter_id))),
            )
            .expect("opponent target update succeeds");

        // C4Object.cpp:5214-5216: `if (!Tick5) TrainPhysical(Fight, 1,
        // C4MaxPhysical)` — the gate fires on frames divisible by 5 only;
        // temporary physicals train whenever they exist (C4Object.cpp:2136-2146).
        for _ in 0..4 {
            engine.tick_without_snapshot().expect("tick succeeds");
        }
        assert_eq!(
            engine.objects[fighter_idx]
                .state
                .temporary_physical
                .expect("temporary physicals remain installed")
                .fight,
            20_000,
            "no training before the first Tick5 frame"
        );

        engine.tick_without_snapshot().expect("tick succeeds");
        let trained = engine.objects[fighter_idx]
            .state
            .temporary_physical
            .expect("Tick5 training updates the temporary physicals");
        assert_eq!(trained.fight, 20_001);
        assert_eq!(trained.walk, 35_000, "other physicals copied untouched");
    }

    #[test]
    fn fight_tick35_awards_experience_and_applies_one_native_promotion() {
        let mut fighter_definition =
            Definition::from_script("CREW", "Crew", "").expect("crew definition compiles");
        fighter_definition.set_crew_member(true);
        fighter_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        fighter_definition.set_line_connect(LINE_CONNECT_ENERGY_HOLDER);
        fighter_definition.configure_actions(
            Some("Fight".to_string()),
            HashMap::from([(
                "Fight".to_string(),
                ActionSpec::default().with_procedure("fight"),
            )]),
        );
        fighter_definition.set_shape_vertices(vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ]);

        let mut opponent_definition =
            Definition::from_script("OPPN", "Opponent", "").expect("opponent compiles");
        opponent_definition.set_shape_rect(Some(DefinitionRect::new(-8, -8, 16, 16)));
        opponent_definition.configure_actions(
            Some("Fight".to_string()),
            HashMap::from([(
                "Fight".to_string(),
                ActionSpec::default().with_procedure("fight"),
            )]),
        );
        opponent_definition.set_shape_vertices(vec![
            ObjectVertex::new(-8, -8),
            ObjectVertex::new(8, -8),
            ObjectVertex::new(8, 8),
            ObjectVertex::new(-8, 8),
        ]);

        let mut engine = Engine::with_seed(37);
        engine
            .register_definition(fighter_definition)
            .expect("crew definition registers");
        engine
            .register_definition(opponent_definition)
            .expect("opponent definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        let mut start = PlayerStart::default();
        start.ready_crew = vec![("CREW".to_string(), 1)];
        engine.set_player_starts(vec![start]);
        engine
            .join_player(JoinPlayerConfig {
                name: "Fighter owner".to_string(),
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
                    experience: 998,
                    ..Default::default()
                }],
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("fighter owner joins");
        let fighter_id = engine.player(0).expect("player exists").crew()[0];
        let fighter_index = engine
            .find_object_index(fighter_id)
            .expect("fighter exists");
        let position = engine.objects[fighter_index].state.position;

        let mut opponent_action = ActionState::new("Fight");
        opponent_action.target = Some(fighter_id);
        let opponent_id = engine
            .spawn_object(
                SpawnConfig::new("OPPN")
                    .with_position(position)
                    .with_action(opponent_action),
            )
            .expect("opponent spawns");
        engine
            .apply_object_update(
                fighter_id,
                ObjectUpdate::new().with_action_update(
                    ActionUpdate::default()
                        .with_name("Fight")
                        .with_target(Some(opponent_id)),
                ),
            )
            .expect("fighter targets opponent");

        let raw_physical = PhysicalInfo {
            energy: 10_000,
            breath: 12_345,
            walk: 0,
            jump: 23_456,
            can_fly: 7,
            corrosion_resist: 8,
            breathe_water: 9,
            ..PhysicalInfo::default()
        };
        engine.objects[fighter_index].state.info_physical = Some(raw_physical);
        engine.objects[fighter_index].state.energy = 12_000;
        engine.pending_audio.clear();

        for expected_frame in 1..35 {
            let snapshot = engine.tick().expect("pre-Tick35 fight succeeds");
            assert_eq!(snapshot.frame, expected_frame);
            assert!(snapshot.audio.iter().all(|command| !matches!(
                command,
                AudioCommand::PlaySound { name, target, .. }
                    if name == "Trumpet" && *target == Some(fighter_id)
            )));
            assert_eq!(
                engine
                    .crew_object_info(fighter_id)
                    .expect("fighter keeps info")
                    .experience,
                998,
                "non-Tick35 fight frames do not award experience"
            );
        }

        let promotion_frame = engine.tick().expect("Tick35 fight succeeds");
        assert_eq!(promotion_frame.frame, 35);
        let info = engine
            .crew_object_info(fighter_id)
            .expect("fighter keeps info");
        assert_eq!((info.experience, info.rank), (1_000, 1));
        assert_eq!(info.rank_name, "Ensign");
        let state = engine.capture_state();
        let link = state.crew_info_links[&fighter_id];
        let roster = &state.crew_info_rosters[&link.player_id][link.roster_index];
        assert_eq!((roster.experience, roster.rank), (1_000, 1));
        assert_eq!(roster.rank_name, "Ensign");

        let fighter = promotion_frame
            .object(fighter_id)
            .expect("promoted fighter remains live");
        assert_eq!(fighter.energy, 12_000, "promotion does not heal live Energy");
        let promoted = fighter
            .info_physical
            .expect("promotion writes raw info physicals");
        assert_eq!(promoted.energy, 55_000);
        assert_eq!(
            (
                promoted.can_dig,
                promoted.can_chop,
                promoted.can_construct,
                promoted.can_scale,
                promoted.can_hangle,
            ),
            (1, 1, 1, 1, 1)
        );
        assert_eq!(promoted.breath, raw_physical.breath);
        assert_eq!(promoted.walk, raw_physical.walk);
        assert_eq!(promoted.jump, raw_physical.jump);
        assert_eq!(promoted.can_fly, raw_physical.can_fly);
        assert_eq!(promoted.corrosion_resist, raw_physical.corrosion_resist);
        assert_eq!(promoted.breathe_water, raw_physical.breathe_water);

        let promotion_messages = promotion_frame
            .hud
            .messages
            .iter()
            .filter(|message| message.target == Some(fighter_id))
            .collect::<Vec<_>>();
        assert_eq!(promotion_messages.len(), 1);
        assert_eq!(
            promotion_messages[0].lines,
            ["Rookie is promoted".to_string(), "to Ensign!".to_string()]
        );
        assert_eq!(
            promotion_frame
                .audio
                .iter()
                .filter(|command| matches!(
                    command,
                    AudioCommand::PlaySound {
                        name,
                        target,
                        volume: 100,
                        looped: false,
                        ..
                    } if name == "Trumpet" && *target == Some(fighter_id)
                ))
                .count(),
            1,
            "native promotion emits one Trumpet"
        );

        for expected_frame in 36..70 {
            let snapshot = engine.tick().expect("post-promotion fight succeeds");
            assert_eq!(snapshot.frame, expected_frame);
            assert_eq!(
                engine
                    .crew_object_info(fighter_id)
                    .expect("fighter keeps info")
                    .experience,
                1_000,
                "experience stays fixed between Tick35 boundaries"
            );
        }

        let second_award = engine.tick().expect("second Tick35 fight succeeds");
        assert_eq!(second_award.frame, 70);
        let info = engine
            .crew_object_info(fighter_id)
            .expect("fighter keeps info after the second award");
        assert_eq!((info.experience, info.rank), (1_002, 1));
        assert!(second_award.audio.iter().all(|command| !matches!(
            command,
            AudioCommand::PlaySound { name, target, .. }
                if name == "Trumpet" && *target == Some(fighter_id)
        )));
    }

    /// `C4Object::SetAction` stops the outgoing action's ActMap sound and
    /// starts the incoming one as an object-attached LOOP at volume 100
    /// (C4Object.cpp:4149-4152, 4186-4190 — `StartSoundEffect(..., +1, 100,
    /// this)`), both gated on the numeric action slot actually changing.
    /// EkeReloaded's Uzi is the shape under test: `Shoot` declares
    /// `Sound=UZ_Shoot` with `NextAction=Shoot`, so the burst must be one
    /// continuous loop rather than silence or a per-frame retrigger.
    #[test]
    fn actmap_sound_loops_while_its_action_slot_stays_selected() {
        let uzi_sound = |snapshot: &SimulationSnapshot, id| {
            snapshot
                .audio
                .iter()
                .filter(|command| {
                    matches!(
                        command,
                        AudioCommand::PlaySound { name, target, .. } | AudioCommand::StopSound { name, target }
                            if name == "UZ_Shoot" && *target == Some(id)
                    )
                })
                .cloned()
                .collect::<Vec<_>>()
        };

        let mut definition = Definition::from_script("Uzi", "Uzi", "func Initialize() { }")
            .expect("script compiles");
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Shoot".to_string(),
                    ActionSpec::default()
                        .with_length(1)
                        .with_delay(1)
                        .with_next("Shoot")
                        .with_sound("UZ_Shoot"),
                ),
                (
                    "Burst".to_string(),
                    ActionSpec::default()
                        .with_length(2)
                        .with_delay(1)
                        .with_next("Idle")
                        .with_sound("UZ_Shoot"),
                ),
            ]),
        );

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let shooter = engine
            .spawn_object(
                SpawnConfig::new("Uzi")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Shoot")),
            )
            .expect("shooter spawns");

        // Entering the slot starts one attached loop at volume 100.
        let started = engine.tick().expect("first tick succeeds");
        assert!(
            matches!(
                uzi_sound(&started, shooter).as_slice(),
                [AudioCommand::PlaySound {
                    volume: 100,
                    looped: true,
                    // StartSoundEffect calls NewInstance unconditionally
                    // (C4SoundSystem.cpp:54-58); only FnSound gates on
                    // IsSoundPlaying (C4Script.cpp:2317-2319).
                    multiple: true,
                    ..
                }]
            ),
            "entering Shoot starts exactly one looped attached sound, got {:?}",
            uzi_sound(&started, shooter)
        );

        // NextAction=Shoot re-selects the SAME numeric slot every frame, and
        // C++ gates both the stop and the start on `iAct != iLastAction`, so
        // the loop must keep running untouched.
        for frame in 0..8 {
            let snapshot = engine.tick().expect("self-transition tick succeeds");
            assert_eq!(
                snapshot.object(shooter).expect("shooter present").action.name,
                "Shoot",
            );
            assert!(
                uzi_sound(&snapshot, shooter).is_empty(),
                "frame {frame}: a same-slot NextAction must not retrigger the loop, got {:?}",
                uzi_sound(&snapshot, shooter)
            );
        }

        // Leaving the slot stops it, and Idle carries no sound of its own.
        let burst = engine
            .spawn_object(
                SpawnConfig::new("Uzi")
                    .with_category(CATEGORY_OBJECT)
                    .with_action(ActionState::new("Burst")),
            )
            .expect("burst shooter spawns");
        let burst_started = engine.tick().expect("burst tick succeeds");
        assert!(
            matches!(
                uzi_sound(&burst_started, burst).as_slice(),
                [AudioCommand::PlaySound { looped: true, .. }]
            ),
            "entering Burst starts its loop, got {:?}",
            uzi_sound(&burst_started, burst)
        );
        let stopped = engine.tick().expect("burst-to-idle tick succeeds");
        assert_eq!(
            stopped.object(burst).expect("burst present").action.name,
            "Idle",
        );
        assert!(
            matches!(
                uzi_sound(&stopped, burst).as_slice(),
                [AudioCommand::StopSound { .. }]
            ),
            "leaving the slot stops the loop exactly once, got {:?}",
            uzi_sound(&stopped, burst)
        );
    }
