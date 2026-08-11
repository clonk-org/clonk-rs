    fn rect_mask_attach_definition(with_own_mask: bool) -> Definition {
        let mut definition = simple_definition("Climber");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);
        if with_own_mask {
            definition.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 3, 1, 0, 2)));
        }
        definition.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                (
                    "Single".to_string(),
                    ActionSpec::default().with_attach(CNAT_BOTTOM),
                ),
                (
                    "Multi".to_string(),
                    ActionSpec::default().with_attach(CNAT_BOTTOM | CNAT_MULTI_ATTACH),
                ),
                ("Jump".to_string(), ActionSpec::default()),
            ]),
        );
        definition
    }

    #[test]
    fn multi_attach_matches_single_attach_on_rect_solid_mask() {
        let mut platform = simple_definition("Platform");
        platform.set_solid_mask(Some(DefinitionTargetRect::new(0, 0, 8, 1, 0, 0)));

        let mut engine = Engine::with_seed(49);
        engine.set_landscape(Landscape::flat(20, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(platform)
            .expect("platform definition registers");
        engine
            .register_definition(rect_mask_attach_definition(false))
            .expect("climber definition registers");
        engine
            .spawn_object(
                SpawnConfig::new("Platform")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 7))
                    .with_loaded(true),
            )
            .expect("platform spawns");
        let single = engine
            .spawn_object(
                SpawnConfig::new("Climber")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 5))
                    .with_action(ActionState::new("Single"))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("single-attach climber spawns");
        let multi = engine
            .spawn_object(
                SpawnConfig::new("Climber")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 5))
                    .with_action(ActionState::new("Multi"))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("multi-attach climber spawns");

        engine.tick_without_snapshot().expect("attachment tick succeeds");
        for (id, action, x) in [(single, "Single", 5), (multi, "Multi", 10)] {
            let index = engine.find_object_index(id).expect("climber remains");
            let object = &engine.objects[index];
            assert_eq!(object.state.action.name, action, "must not run NoAttachAction");
            assert_eq!(object.state.position, Vector2::new(x, 5));
            assert_eq!(
                object.state.shape_attach,
                ShapeAttachRecord {
                    mat_valid: true,
                    mat_vehicle: true,
                    x,
                    y: 7,
                    vtx: 0,
                }
            );
        }
    }

    #[test]
    fn multi_attach_excludes_own_rect_solid_mask_after_first_motion() {
        let mut engine = Engine::with_seed(49);
        engine.set_landscape(Landscape::flat(24, 20));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(rect_mask_attach_definition(true))
            .expect("climber definition registers");

        let spawn = |engine: &mut Engine, x, action| {
            engine
                .spawn_object(
                    SpawnConfig::new("Climber")
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(x, 5))
                        .with_fixed_position(FixedVec2::from_ints(x, 5))
                        .with_action(ActionState::new(action))
                        .with_fixed_velocity(FixedVec2::new(itofix(2), C4Fixed::ZERO))
                        .with_mobile(true)
                        .with_loaded(true),
                )
                .expect("self-masked climber spawns")
        };
        let single = spawn(&mut engine, 5, "Single");
        let multi = spawn(&mut engine, 12, "Multi");

        // Execute C4Object::DoMovement directly with the attachment bits
        // already latched by ExecAction. The custom-procedure ExecAction
        // default otherwise zeroes xdir before movement (C4Object.cpp:5427).
        let solid_mask_indices = (0..engine.objects.len()).collect::<Vec<_>>();
        for (id, attach) in [
            (single, CNAT_BOTTOM),
            (multi, CNAT_BOTTOM | CNAT_MULTI_ATTACH),
        ] {
            let index = engine.find_object_index(id).expect("climber exists");
            engine.objects[index].frame_t_attach = attach;
            let definition_id = engine.objects[index].definition_id.clone();
            let actions = engine
                .definition(&definition_id)
                .expect("climber definition exists")
                .action_library()
                .clone();
            engine
                .exec_object_movement(
                    index,
                    &actions,
                    &definition_id,
                    &solid_mask_indices,
                )
                .expect("two-pixel movement succeeds");
        }
        for (id, expected_x) in [(single, 7), (multi, 14)] {
            let index = engine.find_object_index(id).expect("climber remains");
            let object = &engine.objects[index];
            assert_eq!(object.state.position, Vector2::new(expected_x, 5));
            assert_eq!(
                object.state.action.name, "Jump",
                "the second pixel must exclude the mover's stale own mask"
            );
        }
    }

    #[test]
    fn free_rotation_preserves_fractional_translation_accumulator() {
        let mut definition = simple_definition("Spinner");
        definition.set_rotateable(360);
        definition.set_shape_vertices(vec![ObjectVertex::new(2, 0).with_cnat(CNAT_RIGHT)]);

        let mut engine = Engine::with_seed(53);
        engine.set_landscape(Landscape::flat(80, 60));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let half_pixel = fixed100(50);
        let velocity = FixedVec2::new(half_pixel, half_pixel);
        let spinning_start = FixedVec2::from_ints(10, 10);
        let spinning_id = engine
            .spawn_object(
                SpawnConfig::new("Spinner")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(spinning_start)
                    .with_fixed_velocity(velocity)
                    .with_rotation_velocity(itofix(1))
                    .with_mobile(true),
            )
            .expect("spinning object spawns");
        let control_start = FixedVec2::from_ints(40, 10);
        let control_id = engine
            .spawn_object(
                SpawnConfig::new("Spinner")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(40, 10))
                    .with_fixed_position(control_start)
                    .with_fixed_velocity(velocity)
                    .with_mobile(true),
            )
            .expect("control object spawns");

        let mut expected_spinning = spinning_start;
        let mut expected_control = control_start;
        for _ in 0..4 {
            engine.tick_without_snapshot().expect("tick succeeds");
            expected_spinning += velocity;
            expected_control += velocity;
        }

        let spinning_idx = engine
            .find_object_index(spinning_id)
            .expect("spinning object exists");
        let control_idx = engine
            .find_object_index(control_id)
            .expect("control object exists");
        assert_ne!(engine.objects[spinning_idx].state.rotation, 0);
        assert_eq!(engine.objects[spinning_idx].fixed_position, expected_spinning);
        assert_eq!(engine.objects[control_idx].fixed_position, expected_control);
        assert_eq!(
            engine.objects[spinning_idx].fixed_position.x - spinning_start.x,
            engine.objects[control_idx].fixed_position.x - control_start.x
        );
        assert_eq!(
            engine.objects[spinning_idx].fixed_position.y - spinning_start.y,
            engine.objects[control_idx].fixed_position.y - control_start.y
        );
    }

    #[test]
    fn attached_rotation_shifts_only_integer_position() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("AttachedSpinner");
        definition.set_rotateable(360);
        definition.set_shape_vertices(vec![ObjectVertex::new(-30, 0).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);

        let mut engine = Engine::with_seed(59);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(100, 11, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let fixed_position = FixedVec2::new(
            itofix(50) + fixed100(25),
            itofix(10) + fixed100(25),
        );
        let id = engine
            .spawn_object(
                SpawnConfig::new("AttachedSpinner")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 10))
                    .with_fixed_position(fixed_position)
                    // DoMovement multiplies rdir by five; 0.2 yields one
                    // contact-aware integer rotation step.
                    .with_rotation_velocity(fixed100(20))
                    .with_mobile(true),
            )
            .expect("attached spinner spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].frame_t_attach = CNAT_BOTTOM;
        let definition_id = engine.objects[idx].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("definition exists")
            .action_library()
            .clone();

        engine
            .exec_object_movement(idx, &actions, &definition_id, &[])
            .expect("attached rotation succeeds");
        assert_eq!(engine.objects[idx].state.rotation, 1);
        assert_eq!(engine.objects[idx].state.position, Vector2::new(50, 11));
        assert_eq!(engine.objects[idx].fixed_position, fixed_position);

        // Without another attachment override, the following frame walks the
        // integer position back toward fixtoi(fix_y), proving that rotation
        // did not permanently fold its one-pixel correction into fix_y.
        engine.objects[idx].frame_t_attach = CNAT_NONE;
        engine.objects[idx].rotation_velocity = C4Fixed::ZERO;
        engine
            .exec_object_movement(idx, &actions, &definition_id, &[])
            .expect("next movement succeeds");
        assert_eq!(engine.objects[idx].state.position, Vector2::new(50, 10));
        assert_eq!(engine.objects[idx].fixed_position, fixed_position);
    }

    #[test]
    fn movement_rotation_entry_uses_cached_ocf_rotate_con_gate() {
        let mut definition = simple_definition("ConstructionWheel");
        definition.set_rotateable(360);

        let mut engine = Engine::with_seed(61);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("construction wheel registers");
        let spawn = |engine: &mut Engine, x, construction| {
            engine
                .spawn_object(
                    SpawnConfig::new("ConstructionWheel")
                        .with_category(CATEGORY_OBJECT)
                        .with_position(Vector2::new(x, 5))
                        .with_construction(construction)
                        .with_rotation_velocity(itofix(1))
                        .with_mobile(true),
                )
                .expect("construction wheel spawns")
        };
        let minimum = spawn(&mut engine, 5, 100);
        let rotateable = spawn(&mut engine, 15, 101);

        engine.tick_without_snapshot().expect("rotation gate tick succeeds");

        let minimum = engine
            .objects
            .get(engine.find_object_index(minimum).expect("minimum wheel remains"))
            .expect("minimum wheel exists");
        assert_eq!(minimum.state.ocf & ocf::ROTATE, 0);
        assert_eq!(minimum.state.rotation, 0);
        assert_eq!(minimum.fixed_rotation, C4Fixed::ZERO);
        assert_eq!(minimum.rotation_velocity, itofix(1));

        let rotateable = engine
            .objects
            .get(
                engine
                    .find_object_index(rotateable)
                    .expect("rotateable wheel remains"),
            )
            .expect("rotateable wheel exists");
        assert_ne!(rotateable.state.ocf & ocf::ROTATE, 0);
        assert_eq!(rotateable.state.rotation, 5);
        assert_eq!(rotateable.fixed_rotation, itofix(5));
        assert_eq!(rotateable.rotation_velocity, itofix(1));
    }

    fn rotation_redirect_fixture_at_con(
        ydir: C4Fixed,
        rdir: C4Fixed,
        construction: i32,
    ) -> (Engine, ObjectId) {
        let mut definition = simple_definition("RedirectWheel");
        definition.set_rotateable(360);
        definition
            .set_shape_vertices(vec![ObjectVertex::new(40, 0).with_cnat(CNAT_TOP)]);
        definition.set_contact_density(50);
        definition.set_no_stabilize(true);

        let mut landscape = vehicle_grid_landscape(100, 30);
        landscape.set_world_height(30);
        for x in 0..100 {
            landscape.grid_write_byte(x, 10, 1);
        }

        let mut engine = Engine::with_seed(61);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("redirect wheel registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("RedirectWheel")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(30, 11))
                    .with_fixed_position(FixedVec2::from_ints(30, 11))
                    .with_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, ydir))
                    .with_rotation_velocity(rdir)
                    .with_construction(construction)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("redirect wheel spawns");
        let idx = engine.find_object_index(id).expect("redirect wheel exists");
        assert!(!engine.objects[idx].state.alive);
        assert_eq!(
            engine.objects[idx].state.ocf & ocf::ROTATE != 0,
            construction > 100
        );
        (engine, id)
    }

    fn rotation_redirect_fixture(ydir: C4Fixed, rdir: C4Fixed) -> (Engine, ObjectId) {
        rotation_redirect_fixture_at_con(ydir, rdir, FULL_CON)
    }

    fn exec_redirect_wheel_movement(engine: &mut Engine, id: ObjectId) {
        let idx = engine.find_object_index(id).expect("redirect wheel exists");
        let definition_id = engine.objects[idx].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("redirect wheel definition exists")
            .action_library()
            .clone();
        engine
            .exec_object_movement(idx, &actions, &definition_id, &[])
            .expect("redirect wheel movement succeeds");
    }

    #[test]
    fn vertical_redirect_suppresses_same_movement_rotation_redirect() {
        // The upward step puts the sole x=40 vertex into the solid row with
        // both neighboring pixels blocked. Starting at +0.25 rdir, C++
        // transfers 0.5 from -1.0 ydir to produce -0.25 rdir and latches
        // fRedirectYR. The ensuing -1-degree rotation contacts the same row;
        // that contact zeros rdir without restoring it to ydir
        // (C4Movement.cpp:311-316,422-425).
        let (mut engine, id) = rotation_redirect_fixture(-itofix(1), fixed100(25));

        exec_redirect_wheel_movement(&mut engine, id);

        let idx = engine.find_object_index(id).expect("redirect wheel exists");
        let object = &engine.objects[idx];
        assert_eq!(object.state.position, Vector2::new(30, 11));
        assert_eq!(object.state.rotation, 0, "contact rolls rotation back");
        assert_eq!(object.fixed_rotation, C4Fixed::ZERO);
        assert_eq!(object.fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(object.rotation_velocity, C4Fixed::ZERO);
    }

    #[test]
    fn vertical_redirect_uses_cached_ocf_rotate_con_gate() {
        let (mut minimum, minimum_id) =
            rotation_redirect_fixture_at_con(-itofix(1), fixed100(25), 100);
        exec_redirect_wheel_movement(&mut minimum, minimum_id);
        let minimum = &minimum.objects[minimum
            .find_object_index(minimum_id)
            .expect("minimum wheel remains")];
        assert_eq!(minimum.fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(minimum.rotation_velocity, fixed100(25));
        assert_eq!(minimum.fixed_rotation, C4Fixed::ZERO);
        assert_eq!(minimum.state.rotation, 0);

        let (mut rotateable, rotateable_id) =
            rotation_redirect_fixture_at_con(-itofix(1), fixed100(25), 101);
        exec_redirect_wheel_movement(&mut rotateable, rotateable_id);
        let rotateable = &rotateable.objects[rotateable
            .find_object_index(rotateable_id)
            .expect("rotateable wheel remains")];
        assert_eq!(rotateable.fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(rotateable.rotation_velocity, C4Fixed::ZERO);
        assert_eq!(rotateable.fixed_rotation, C4Fixed::ZERO);
        assert_eq!(rotateable.state.rotation, 0);
    }

    #[test]
    fn rotation_contact_without_vertical_redirect_transfers_rdir_to_ydir() {
        let (mut engine, id) = rotation_redirect_fixture(C4Fixed::ZERO, -fixed100(50));

        exec_redirect_wheel_movement(&mut engine, id);

        let idx = engine.find_object_index(id).expect("redirect wheel exists");
        let object = &engine.objects[idx];
        assert_eq!(object.state.rotation, 0);
        assert_eq!(object.fixed_rotation, C4Fixed::ZERO);
        assert_eq!(object.fixed_velocity.y, -fixed100(50));
        assert_eq!(object.rotation_velocity, C4Fixed::ZERO);
    }

    #[test]
    fn vertical_rotation_redirect_guard_resets_each_movement() {
        let (mut engine, id) = rotation_redirect_fixture(-itofix(1), fixed100(25));
        exec_redirect_wheel_movement(&mut engine, id);
        let idx = engine.find_object_index(id).expect("redirect wheel exists");
        assert_eq!(engine.objects[idx].fixed_velocity.y, C4Fixed::ZERO);
        assert_eq!(engine.objects[idx].rotation_velocity, C4Fixed::ZERO);

        // A fresh DoMovement has no vertical step. Its identical rotation
        // contact must therefore perform the normal rdir -> ydir transfer;
        // fRedirectYR is a local, not persistent object state.
        engine.objects[idx].rotation_velocity = -fixed100(50);
        exec_redirect_wheel_movement(&mut engine, id);

        let idx = engine.find_object_index(id).expect("redirect wheel exists");
        assert_eq!(engine.objects[idx].fixed_velocity.y, -fixed100(50));
        assert_eq!(engine.objects[idx].rotation_velocity, C4Fixed::ZERO);
    }

    #[test]
    fn rotation_steps_rollback_on_shape_contact() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Wheel");
        definition.set_rotateable(360);
        definition.set_shape_vertices(vec![ObjectVertex::new(2, 0).with_cnat(CNAT_RIGHT)]);
        definition.set_contact_density(50);

        let mut engine = Engine::with_seed(43);
        engine.set_materials(materials);
        let mut surface = vec![20; 12];
        surface[6] = 0;
        let mut landscape =
            Landscape::new_with_material(12, surface, Some(earth)).expect("landscape constructs");
        landscape.fill_solid_material(Some(earth));
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let fixed_position = FixedVec2::new(
            itofix(4) + fixed100(25),
            itofix(10) + fixed100(25),
        );
        let id = engine
            .spawn_object(
                SpawnConfig::new("Wheel")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(4, 10))
                    .with_fixed_position(fixed_position),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].rotation_velocity = itofix(1);
        // SetRDir mobilizes (C4Script.cpp:718)
        engine.objects[idx].state.mobile = true;

        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.rotation, 0);
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_rotation, itofix(0));
        assert_eq!(engine.objects[idx].rotation_velocity, C4Fixed::ZERO);
        assert_eq!(engine.objects[idx].state.vertices[0].x, 2);
        assert_eq!(engine.objects[idx].fixed_position, fixed_position);
    }

    #[test]
    fn horizontal_fix_zeroes_set_x_dir_and_preserves_vertical_movement() {
        let script = r#"#strict 2
func Arm()
{
    SetXDir(10);
    return 1;
}
"#;
        let mut definition =
            Definition::from_script("RailMover", "RailMover", script).expect("script compiles");
        definition.set_no_horizontal_move(1);

        let mut engine = Engine::with_seed(67);
        engine.set_physics(PhysicsSettings::new(0, 0, 0));
        engine
            .register_definition(definition)
            .expect("rail mover registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("RailMover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_loaded(true),
            )
            .expect("rail mover spawns");
        let idx = engine.find_object_index(id).expect("rail mover exists");
        engine
            .call_object_function(idx, "Arm", Vec::new())
            .expect("SetXDir arm runs");
        assert_eq!(engine.objects[idx].fixed_velocity.x, itofix(1));
        assert!(engine.objects[idx].state.mobile);
        // Give the same object independent vertical momentum; HorizontalFix
        // must restrict only the script-written xdir at movement entry.
        let xdir = engine.objects[idx].fixed_velocity.x;
        engine.objects[idx].set_fixed_velocity(FixedVec2::new(xdir, itofix(2)));
        assert_eq!(
            engine.objects[idx].fixed_velocity,
            FixedVec2::new(itofix(1), itofix(2))
        );

        let definition_id = engine.objects[idx].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("rail mover definition exists")
            .action_library()
            .clone();
        engine
            .exec_object_movement(idx, &actions, &definition_id, &[])
            .expect("rail movement succeeds");

        let object = &engine.objects[idx];
        assert_eq!(object.state.position, Vector2::new(10, 12));
        assert_eq!(object.fixed_position, FixedVec2::from_ints(10, 12));
        assert_eq!(object.fixed_velocity.x, C4Fixed::ZERO);
        assert_eq!(object.state.velocity.x, 0);
        assert_eq!(object.fixed_velocity.y, itofix(2));
        assert_eq!(object.state.velocity.y, 2);
    }

    #[test]
    fn horizontal_fix_hit_callbacks_receive_zero_old_xdir() {
        let (mut definition, calls) = hit_gate_probe_definition("HorizontalFixHitProbe");
        definition.set_no_horizontal_move(1);
        definition
            .set_shape_vertices(vec![ObjectVertex::new(0, 1).with_cnat(CNAT_BOTTOM)]);
        definition.set_contact_density(50);

        let mut landscape = vehicle_grid_landscape(24, 24);
        landscape.set_world_height(24);
        for x in 0..24 {
            landscape.grid_write_byte(x, 12, 1);
        }

        let mut engine = Engine::with_seed(71);
        engine.set_landscape(landscape);
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("horizontal-fix hit probe registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("HorizontalFixHitProbe")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_fixed_velocity(FixedVec2::new(itofix(1), itofix(2)))
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("horizontal-fix hit probe spawns");
        let idx = engine
            .find_object_index(id)
            .expect("horizontal-fix hit probe exists");
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.objects[idx].state.ocf & ocf::HIT_SPEED1, 0);
        assert_ne!(engine.objects[idx].state.ocf & ocf::HIT_SPEED2, 0);
        assert_eq!(engine.objects[idx].state.ocf & ocf::HIT_SPEED3, 0);

        let definition_id = engine.objects[idx].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("horizontal-fix hit probe definition exists")
            .action_library()
            .clone();
        engine
            .exec_object_movement(idx, &actions, &definition_id, &[])
            .expect("horizontal-fix contact movement succeeds");

        assert_eq!(engine.objects[idx].state.position, Vector2::new(10, 10));
        assert_eq!(engine.objects[idx].fixed_velocity.x, C4Fixed::ZERO);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                (
                    "Hit".to_string(),
                    vec![clonk_script::Value::Nil, clonk_script::Value::Int(200)],
                ),
                (
                    "Hit2".to_string(),
                    vec![clonk_script::Value::Nil, clonk_script::Value::Int(200)],
                ),
            ],
            "HorizontalFix runs before oldxdir is captured for Hit arguments"
        );
    }

    #[test]
    fn set_x_dir_script_applies_subpixel_velocity_end_to_end() {
        // A script calling SetXDir(15) with the default precision (10) must set
        // xdir = itofix(15, 10) = 1.5 px/frame (raw 16.16 value 98304), matching
        // C++ FnSetXDir (`C4Script.cpp:697`) — NOT the pre-fix integer-mirror
        // behaviour that treated 15 as 15 whole px/frame (a 10x desync).
        let mut engine = Engine::with_seed(1);
        let definition = Definition::from_script(
            "Mover",
            "Mover",
            r#"
            global func Initialize(state, random) { SetXDir(15); return 0; }
            global func Step(state, frame, random) { return 0; }
            "#,
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Mover")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        // Initialize ran at spawn: the live object holds true sub-pixel velocity.
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_velocity.x.val(), 98304);

        // One frame advances 1.5 px; fixtoi(1.5) = 2 (the old bug produced 15).
        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position.x, 2);
        assert_eq!(object.velocity.x, 2);

        // A second frame accumulates to 3.0 px; fixtoi(3.0) = 3.
        let snapshot = engine.tick().expect("tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position.x, 3);
    }

    #[test]
    fn set_r_dir_script_rotates_object_like_cpp() {
        // SetRDir(10) with the default precision (10) sets rdir = itofix(10, 10)
        // = 1.0 deg/frame (`C4Script.cpp:710`). C++ applies fix_r += rdir * 5
        // each frame (`C4Movement.cpp:376`), so the object turns 5°/frame.
        let mut engine = Engine::with_seed(1);
        let mut definition = Definition::from_script(
            "Spinner",
            "Spinner",
            r#"
            global func Initialize(state, random) { SetRDir(10); return 0; }
            global func Step(state, frame, random) { return 0; }
            "#,
        )
        .expect("script compiles");
        definition.set_rotateable(1);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Spinner")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        // Initialize ran at spawn: rdir = itofix(10, 10) = 1.0 deg/frame (raw 65536).
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].rotation_velocity.val(), 65536);

        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(snapshot.object(id).expect("object present").rotation, 5);
        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(snapshot.object(id).expect("object present").rotation, 10);
    }

    #[test]
    fn set_r_dir_persists_for_non_rotateable_definition() {
        let mut engine = Engine::with_seed(2);
        let definition = Definition::from_script(
            "Fixed",
            "Fixed",
            r#"
            global func Initialize(state, random) { SetRDir(10); return 0; }
            global func Step(state, frame, random) { return 0; }
            public func ReadRDir() { return GetRDir(); }
            "#,
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Fixed")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        let expected_rdir = math::itofix_prec(10, 10);
        let saved_fix_r = C4Fixed::from_raw(123_456);
        engine.objects[idx].fixed_rotation = saved_fix_r;
        assert_eq!(engine.objects[idx].rotation_velocity, expected_rdir);
        assert!(engine.objects[idx].state.mobile);

        for frame in 1..=12 {
            let snapshot = engine.tick().expect("tick succeeds");
            assert_eq!(
                snapshot.object(id).expect("object present").rotation,
                0,
                "frame {frame}"
            );
            let idx = engine.find_object_index(id).expect("object exists");
            let object = &engine.objects[idx];
            assert_eq!(object.rotation_velocity, expected_rdir, "frame {frame}");
            assert_eq!(object.fixed_rotation, saved_fix_r, "frame {frame}");
            assert!(object.state.mobile, "frame {frame}");
        }

        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine
                .call_object_function(idx, "ReadRDir", Vec::new())
                .expect("GetRDir executes"),
            Value::Int(10)
        );
    }

    #[test]
    fn non_rotateable_static_movement_zeroes_rotation_without_resetting_fixed_state() {
        let mut engine = Engine::with_seed(2);
        let mut definition = Definition::from_script("FixedStatic", "Fixed static", "")
            .expect("script compiles");
        definition.set_no_stabilize(true);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let saved_fix_r = C4Fixed::from_raw(4_654_321);
        let saved_rdir = math::itofix_prec(30, 10);
        let id = engine
            .spawn_object(
                SpawnConfig::new("FixedStatic")
                    .with_loaded(true)
                    .with_category(CATEGORY_OBJECT)
                    .with_rotation(23)
                    .with_fixed_rotation(saved_fix_r)
                    .with_rotation_velocity(saved_rdir)
                    .with_mobile(false),
            )
            .expect("loaded object spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].state.rotation, 23);
        assert_eq!(engine.objects[idx].fixed_rotation, saved_fix_r);
        assert_eq!(engine.objects[idx].rotation_velocity, saved_rdir);
        assert!(!engine.objects[idx].state.mobile);

        let snapshot = engine.tick().expect("static movement executes");
        assert_eq!(snapshot.object(id).expect("object present").rotation, 0);
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_rotation, saved_fix_r);
        assert_eq!(engine.objects[idx].rotation_velocity, saved_rdir);
        assert!(!engine.objects[idx].state.mobile);
    }

    #[test]
    fn finite_rotateable_range_clamps_fixed_rotation_and_stops_rdir() {
        let mut engine = Engine::with_seed(3);
        let mut definition = Definition::from_script(
            "Limited",
            "Limited",
            r#"
            global func Initialize(state, random) { SetRDir(10); return 0; }
            global func Step(state, frame, random) { return 0; }
            "#,
        )
        .expect("script compiles");
        definition.set_rotateable(4);
        // Without NoStabilize the same frame's Stabilize would upright the
        // freshly clamped 4° tilt in free air (rdir just hit 0 and 4 is
        // within ±StableRange, C4Movement.cpp:574,495) — opt out so the
        // clamp itself stays observable.
        definition.set_no_stabilize(true);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Limited")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.tick().expect("tick succeeds");
        assert_eq!(snapshot.object(id).expect("object present").rotation, 4);
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_rotation, itofix(4));
        assert_eq!(engine.objects[idx].rotation_velocity, C4Fixed::ZERO);
    }

    #[test]
    fn ocf_reads_use_the_cached_field_refreshed_at_events_like_cpp() {
        // C++ readers consume the CACHED obj->OCF: it refreshes at specific
        // events (SetAlive -> SetOCF, C4Object.h:361; death,
        // C4Object.cpp:1177; Init, C4Object.cpp:215) and once per frame at
        // Execute-start (UpdateOCF, C4Object.cpp:1058). A raw field poke
        // with no event keeps the stale mask until the next frame.
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Crew");
        definition.set_crew_member(true);
        // Crew are livings: OCF_Alive needs Category & C4D_Living
        // (SetOCF, C4Object.cpp:600-605).
        definition.set_category(CATEGORY_LIVING);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 0, 0));

        let id = engine
            .spawn_object(SpawnConfig::new("Crew").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");

        engine
            .apply_object_update(id, ObjectUpdate::new().with_alive(true))
            .expect("update succeeds");
        assert_ne!(
            engine.object_ocf_at_index(idx) & ocf::ALIVE,
            0,
            "SetAlive-style updates refresh the cache (C4Object.h:361)"
        );

        // A raw poke is no event: the cache stays stale like C++.
        engine.objects[idx].state.alive = false;
        assert_ne!(
            engine.object_ocf_at_index(idx) & ocf::ALIVE,
            0,
            "no event, no refresh — readers see the stale mask"
        );

        // The next frame's Execute-start refresh picks it up.
        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::ALIVE,
            0,
            "UpdateOCF at Execute-start sees the new state"
        );
    }

    #[test]
    fn rotateable_definition_reports_ocf_rotate() {
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Wheel");
        definition.set_rotateable(1);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Wheel").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");

        assert_ne!(engine.object_ocf_at_index(idx) & ocf::ROTATE, 0);
    }

    #[test]
    fn ocf_rotate_requires_con_above_100() {
        // OCF_Rotate skips minimum (invisible) construction sites: the def
        // must be Rotateable AND Con > 100 (SetOCF, C4Object.cpp:576-580).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Wheel");
        definition.set_rotateable(1);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Wheel").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");

        engine.objects[idx].state.construction = 100;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::ROTATE,
            0,
            "Con == 100 fails the Con > 100 gate (C4Object.cpp:579)"
        );

        engine.objects[idx].state.construction = 101;
        engine.refresh_object_ocf(idx);
        assert_ne!(
            engine.object_ocf_at_index(idx) & ocf::ROTATE,
            0,
            "Con 101 passes the gate"
        );
    }

    #[test]
    fn ocf_grab_requires_non_static_back_category() {
        // OCF_Grab: Def->Grab AND !(Category & C4D_StaticBack)
        // (SetOCF, C4Object.cpp:553-555).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Cart");
        definition.set_grab(1);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let static_back = engine
            .spawn_object(SpawnConfig::new("Cart").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(static_back).expect("object exists");
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::GRAB,
            0,
            "StaticBack objects are never grabbable (C4Object.cpp:554)"
        );

        let vehicle = engine
            .spawn_object(
                SpawnConfig::new("Cart")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(vehicle).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::GRAB, 0);
    }

    #[test]
    fn ocf_construct_requires_incomplete_unrotated_unburning_constructable() {
        // OCF_Construct: Def->Constructable && Con < FullCon && r == 0 &&
        // !OnFire (SetOCF, C4Object.cpp:549-552).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Site");
        definition.set_constructable(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Site").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::CONSTRUCT,
            0,
            "a completed object is not a construction site (Con < FullCon fails)"
        );

        engine.objects[idx].state.construction = FULL_CON / 2;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::CONSTRUCT, 0);

        engine.objects[idx].state.rotation = 10;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::CONSTRUCT,
            0,
            "rotated objects cannot be built (r == 0 fails)"
        );
        engine.objects[idx].state.rotation = 0;

        engine.objects[idx].state.on_fire = true;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::CONSTRUCT,
            0,
            "burning objects cannot be built (!OnFire fails)"
        );
    }

    #[test]
    fn ocf_living_and_alive_require_living_category() {
        // OCF_Living: Category & C4D_Living; OCF_Alive additionally needs
        // the Alive flag (SetOCF, C4Object.cpp:600-605). Neither derives
        // from Def->CrewMember.
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Beast");
        definition.set_crew_member(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        // Default StaticBack category: alive or not, no Living/Alive bits.
        let static_back = engine
            .spawn_object(SpawnConfig::new("Beast").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(static_back).expect("object exists");
        engine.objects[idx].state.alive = true;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & (ocf::LIVING | ocf::ALIVE),
            0,
            "non-Living categories never get OCF_Living/OCF_Alive"
        );

        let living = engine
            .spawn_object(
                SpawnConfig::new("Beast")
                    .with_category(CATEGORY_LIVING)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(living).expect("object exists");
        engine.objects[idx].state.alive = false;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::LIVING, 0);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::ALIVE,
            0,
            "dead livings keep OCF_Living but lose OCF_Alive"
        );

        engine.objects[idx].state.alive = true;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::ALIVE, 0);
    }

    #[test]
    fn ocf_exclusive_comes_from_the_def_flag() {
        // OCF_Exclusive: no action through this, no construction in front
        // of this — straight from Def->Exclusive (SetOCF,
        // C4Object.cpp:581-583; DefCore "Exclusive", C4Def.cpp:313).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Gate");
        definition.set_exclusive(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Gate").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::EXCLUSIVE, 0);
    }

    #[test]
    fn ocf_edible_comes_from_the_def_flag() {
        // OCF_Edible: straight from Def->Edible (SetOCF,
        // C4Object.cpp:630-632).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Loaf");
        definition.set_edible(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Loaf").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::EDIBLE, 0);
    }

    #[test]
    fn ocf_prey_requires_def_flag_and_raw_alive() {
        // OCF_Prey: Def->Prey && the RAW Alive flag (SetOCF,
        // C4Object.cpp:615-618) — no category gate.
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Sheep");
        definition.set_prey(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Sheep").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");

        engine.objects[idx].state.alive = false;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::PREY,
            0,
            "dead prey is no prey (C4Object.cpp:617)"
        );

        engine.objects[idx].state.alive = true;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::PREY, 0);
    }

    #[test]
    fn ocf_attract_lightning_requires_full_con() {
        // OCF_AttractLightning: Def->AttractLightning at FullCon (SetOCF,
        // C4Object.cpp:623-626).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Mast");
        definition.set_attract_lightning(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Mast").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::ATTRACT_LIGHTNING, 0);

        engine.objects[idx].state.construction = FULL_CON - 1;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::ATTRACT_LIGHTNING,
            0,
            "incomplete objects do not attract lightning (C4Object.cpp:625)"
        );
    }

    #[test]
    fn ocf_entrance_requires_area_full_con_and_rotation_gate() {
        // OCF_Entrance: Entrance.Wdt/Hgt > 0, OCF_FullCon, and either
        // RotatedEntrance == 1 (any rotation) or r <= RotatedEntrance
        // (SetOCF, C4Object.cpp:584-587).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Hut");
        definition.set_entrance_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::ENTRANCE, 0);
        assert_ne!(
            engine.object_ocf_at_index(idx) & ocf::CONTAINER,
            0,
            "an entrance makes a container (C4Object.cpp:658-660)"
        );

        engine.objects[idx].state.construction = FULL_CON - 1;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & (ocf::ENTRANCE | ocf::CONTAINER),
            0,
            "incomplete buildings cannot be entered (OCF_FullCon gate)"
        );
        engine.objects[idx].state.construction = FULL_CON;

        engine.objects[idx].state.rotation = 10;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::ENTRANCE,
            0,
            "RotatedEntrance defaults 0: rotated objects close (r <= 0 fails)"
        );
    }

    #[test]
    fn ocf_entrance_rotation_thresholds_match_cpp() {
        // RotatedEntrance == 1 admits ANY rotation; N admits r <= N
        // (SetOCF, C4Object.cpp:586).
        let mut engine = Engine::with_seed(4);
        let mut any_rotation = simple_definition("Windmill");
        any_rotation.set_entrance_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        any_rotation.set_rotated_entrance(1);
        engine
            .register_definition(any_rotation)
            .expect("definition registers");
        let mut threshold = simple_definition("Tower");
        threshold.set_entrance_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        threshold.set_rotated_entrance(45);
        engine
            .register_definition(threshold)
            .expect("definition registers");

        let spinner = engine
            .spawn_object(SpawnConfig::new("Windmill").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(spinner).expect("object exists");
        engine.objects[idx].state.rotation = 270;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::ENTRANCE, 0);

        let tower = engine
            .spawn_object(SpawnConfig::new("Tower").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(tower).expect("object exists");
        engine.objects[idx].state.rotation = 45;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::ENTRANCE, 0);
        engine.objects[idx].state.rotation = 46;
        engine.refresh_object_ocf(idx);
        assert_eq!(engine.object_ocf_at_index(idx) & ocf::ENTRANCE, 0);
    }

    #[test]
    fn ocf_container_comes_from_grab_put_get_without_entrance() {
        // OCF_Container: C4D_Grab_Put or C4D_Grab_Get suffices even with
        // no entrance (SetOCF, C4Object.cpp:658-660).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Chest");
        definition.set_grab_put_get(1); // C4D_Grab_Put
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Chest").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::CONTAINER, 0);
        assert_eq!(engine.object_ocf_at_index(idx) & ocf::ENTRANCE, 0);
    }

    #[test]
    fn ocf_line_construct_requires_non_energy_holder_line_connect() {
        // OCF_LineConstruct: FullCon && LineConnect & ~C4D_EnergyHolder
        // (SetOCF, C4Object.cpp:611-614).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Plant");
        definition.set_line_connect(LINE_CONNECT_POWER_INPUT);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let mut holder_only = simple_definition("Lorry");
        holder_only.set_line_connect(LINE_CONNECT_ENERGY_HOLDER);
        engine
            .register_definition(holder_only)
            .expect("definition registers");

        let plant = engine
            .spawn_object(SpawnConfig::new("Plant").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(plant).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::LINE_CONSTRUCT, 0);

        engine.objects[idx].state.construction = FULL_CON - 1;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::LINE_CONSTRUCT,
            0,
            "line construction needs OCF_FullCon (C4Object.cpp:612)"
        );

        let lorry = engine
            .spawn_object(SpawnConfig::new("Lorry").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(lorry).expect("object exists");
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::LINE_CONSTRUCT,
            0,
            "a pure C4D_EnergyHolder is no line target (C4Object.cpp:613)"
        );
    }

    #[test]
    fn ocf_power_consumer_requires_line_connect_bit_and_full_con() {
        // OCF_PowerConsumer: LineConnect & C4D_Power_Consumer at FullCon
        // (SetOCF, C4Object.cpp:649-652).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Elevator");
        definition.set_line_connect(LINE_CONNECT_POWER_CONSUMER);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Elevator").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::POWER_CONSUMER, 0);

        engine.objects[idx].state.construction = FULL_CON - 1;
        engine.refresh_object_ocf(idx);
        assert_eq!(engine.object_ocf_at_index(idx) & ocf::POWER_CONSUMER, 0);
    }

    #[test]
    fn ocf_power_supply_from_generator_or_energized_output() {
        // OCF_PowerSupply: (LineConnect & C4D_Power_Generator) OR
        // (LineConnect & C4D_Power_Output && Energy > 0), at FullCon
        // (SetOCF, C4Object.cpp:653-657).
        let mut engine = Engine::with_seed(4);
        let mut generator = simple_definition("Windbag");
        generator.set_line_connect(LINE_CONNECT_POWER_GENERATOR);
        engine
            .register_definition(generator)
            .expect("definition registers");
        let mut output = simple_definition("Battery");
        output.set_line_connect(LINE_CONNECT_POWER_OUTPUT);
        engine
            .register_definition(output)
            .expect("definition registers");

        let windbag = engine
            .spawn_object(SpawnConfig::new("Windbag").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(windbag).expect("object exists");
        engine.objects[idx].state.energy = 0;
        engine.refresh_object_ocf(idx);
        assert_ne!(
            engine.object_ocf_at_index(idx) & ocf::POWER_SUPPLY,
            0,
            "generators supply power regardless of stored energy"
        );

        let battery = engine
            .spawn_object(SpawnConfig::new("Battery").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(battery).expect("object exists");
        engine.objects[idx].state.energy = 0;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::POWER_SUPPLY,
            0,
            "an empty power output supplies nothing (Energy > 0 fails)"
        );
        engine.objects[idx].state.energy = 50;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::POWER_SUPPLY, 0);
    }

    #[test]
    fn ocf_collection_gates_on_con_action_and_collect_delay() {
        // OCF_Collection (SetOCF, C4Object.cpp:593-599): needs OCF_FullCon
        // or IncompleteActivity, a positive Collection area, a free
        // CollectionLimit slot, an action without ObjectDisabled, and
        // NoCollectDelay == 0.
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Kiln");
        definition.set_collection_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        let mut specs = HashMap::new();
        specs.insert(
            "Build".to_string(),
            ActionSpec::default().with_disabled(true),
        );
        definition.configure_actions(None, specs);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Kiln").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::COLLECTION, 0);

        // Below FullCon without IncompleteActivity: no collection.
        engine.objects[idx].state.construction = FULL_CON - 1;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::COLLECTION,
            0,
            "incomplete objects without IncompleteActivity collect nothing (C4Object.cpp:594)"
        );
        engine.objects[idx].state.construction = FULL_CON;

        // An ObjectDisabled action suspends collection.
        engine.objects[idx].state.action.name = "Build".to_string();
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::COLLECTION,
            0,
            "ObjectDisabled actions veto collection (C4Object.cpp:597)"
        );
        engine.objects[idx].state.action.name = "Idle".to_string();

        // A fresh drop delay suspends collection.
        engine.objects[idx].state.no_collect_delay = 2;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::COLLECTION,
            0,
            "NoCollectDelay != 0 vetoes collection (C4Object.cpp:598)"
        );
        engine.objects[idx].state.no_collect_delay = 0;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::COLLECTION, 0);
    }

    #[test]
    fn ocf_collection_incomplete_activity_overrides_full_con_gate() {
        // IncompleteActivity keeps collection alive below FullCon
        // (SetOCF, C4Object.cpp:594).
        let mut engine = Engine::with_seed(4);
        let mut definition = simple_definition("Hive");
        definition.set_collection_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
        definition.set_incomplete_activity(true);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let id = engine
            .spawn_object(SpawnConfig::new("Hive").with_position(Vector2::new(0, 0)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        engine.objects[idx].state.construction = FULL_CON / 2;
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::COLLECTION, 0);
    }

    #[test]
    fn ocf_fight_ready_respects_no_fight_and_disabled_actions() {
        // OCF_FightReady (SetOCF, C4Object.cpp:606-610): the OCF_Alive bit
        // plus an action without ObjectDisabled plus !Def->NoFight.
        let mut engine = Engine::with_seed(4);
        let mut pacifist = simple_definition("Monk");
        pacifist.set_category(CATEGORY_LIVING);
        pacifist.set_no_fight(true);
        engine
            .register_definition(pacifist)
            .expect("definition registers");
        let mut fighter = simple_definition("Knight");
        fighter.set_category(CATEGORY_LIVING);
        let mut specs = HashMap::new();
        specs.insert(
            "Build".to_string(),
            ActionSpec::default().with_disabled(true),
        );
        fighter.configure_actions(None, specs);
        engine
            .register_definition(fighter)
            .expect("definition registers");

        let monk = engine
            .spawn_object(
                SpawnConfig::new("Monk")
                    .with_alive(true)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(monk).expect("object exists");
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::FIGHT_READY,
            0,
            "NoFight defs never become fight-ready (C4Object.cpp:609)"
        );

        let knight = engine
            .spawn_object(
                SpawnConfig::new("Knight")
                    .with_alive(true)
                    .with_position(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");
        let idx = engine.find_object_index(knight).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::FIGHT_READY, 0);

        engine.objects[idx].state.action.name = "Build".to_string();
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::FIGHT_READY,
            0,
            "ObjectDisabled actions veto fight readiness (C4Object.cpp:608)"
        );
    }

    #[test]
    fn ocf_chop_requires_static_back_and_clear_center() {
        // OCF_Chop (SetOCF, C4Object.cpp:570-575): Def->Chopable, a
        // StaticBack category (unfelled trees), and no exclusive object
        // covering the center (Game.Objects.AtObject(x, y, OCF_Exclusive)).
        let mut engine = Engine::with_seed(4);
        let mut tree = simple_definition("Tree");
        tree.set_chopable(true);
        tree.set_shape_rect(Some(DefinitionRect::new(-8, -20, 16, 40)));
        engine.register_definition(tree).expect("tree registers");
        let mut gate = simple_definition("Gate");
        gate.set_exclusive(true);
        gate.set_shape_rect(Some(DefinitionRect::new(-10, -20, 20, 40)));
        engine.register_definition(gate).expect("gate registers");

        // Spawn y is the con-0 bottom: 60 - (40 - 20) puts the center at 40.
        let standing = engine
            .spawn_object(SpawnConfig::new("Tree").with_position(Vector2::new(40, 60)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(standing).expect("object exists");
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::CHOP, 0);

        // A felled tree loses StaticBack — and the Chop bit.
        engine.objects[idx].state.category = CATEGORY_OBJECT;
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::CHOP,
            0,
            "non-StaticBack chopables are already felled (C4Object.cpp:573)"
        );
        engine.objects[idx].state.category = CATEGORY_STATIC_BACK;
        engine.refresh_object_ocf(idx);

        // An exclusive object over the trunk center blocks chopping.
        engine
            .spawn_object(SpawnConfig::new("Gate").with_position(Vector2::new(40, 60)))
            .expect("gate spawns");
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::CHOP,
            0,
            "an exclusive blocker at the center vetoes Chop (C4Object.cpp:574)"
        );
    }

    #[test]
    fn ocf_in_solid_and_in_free_follow_the_landscape() {
        // OCF_InSolid: !Contained && GBackSolid(x, y)
        // (SetOCF, C4Object.cpp:637-640); OCF_InFree: !Contained &&
        // !GBackSemiSolid(x, y - 1) (SetOCF, C4Object.cpp:641-644).
        let mut engine = Engine::with_seed(4);
        // Landscape::flat(120, 60): ground surface at y = 60.
        engine.set_landscape(Landscape::flat(120, 60));
        engine
            .register_definition(simple_definition("Rock"))
            .expect("definition registers");

        // Center in free air: InFree, not InSolid.
        let airborne = engine
            .spawn_object(SpawnConfig::new("Rock").with_position(Vector2::new(40, 20)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(airborne).expect("object exists");
        assert_eq!(engine.object_ocf_at_index(idx) & ocf::IN_SOLID, 0);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::IN_FREE, 0);

        // Center buried in the ground: InSolid, and the pixel above is
        // semi-solid, so not InFree.
        engine.objects[idx].state.position = Vector2::new(40, 80);
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::IN_SOLID, 0);
        assert_eq!(engine.object_ocf_at_index(idx) & ocf::IN_FREE, 0);

        // Standing ON the surface (y = 60 solid, y - 1 = 59 free): the
        // center pixel is solid AND the pixel above is free.
        engine.objects[idx].state.position = Vector2::new(40, 60);
        engine.refresh_object_ocf(idx);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::IN_SOLID, 0);
        assert_ne!(engine.object_ocf_at_index(idx) & ocf::IN_FREE, 0);
    }

    #[test]
    fn ocf_available_follows_the_burial_probe() {
        // OCF_Available landscape clause (SetOCF, C4Object.cpp:646-648):
        // !GBackSemiSolid(x, y-1) || (!GBackSolid(x, y-1) &&
        // !GBackSemiSolid(x, y-8)) — free above, or under a thin
        // non-solid cover with clearance eight pixels up.
        let mut engine = Engine::with_seed(4);
        let mut landscape = Landscape::flat(120, 60);
        // Shallow water 55..60 at x=80 (5 px deep), deep water 40..60 at
        // x=100 (20 px deep).
        landscape.set_liquid_column(
            80,
            vec![LiquidSegment {
                top: 55,
                bottom: 60,
                material: None,
            }],
        );
        landscape.set_liquid_column(
            100,
            vec![LiquidSegment {
                top: 40,
                bottom: 60,
                material: None,
            }],
        );
        engine.set_landscape(landscape);
        engine
            .register_definition(simple_definition("Rock"))
            .expect("definition registers");

        let rock = engine
            .spawn_object(SpawnConfig::new("Rock").with_position(Vector2::new(40, 20)))
            .expect("spawn succeeds");
        let idx = engine.find_object_index(rock).expect("object exists");
        assert_ne!(
            engine.object_ocf_at_index(idx) & ocf::AVAILABLE,
            0,
            "free air above: available"
        );

        // Buried: y-1 is solid ground and y-8 is still underground.
        engine.objects[idx].state.position = Vector2::new(40, 80);
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::AVAILABLE,
            0,
            "buried objects are not available (C4Object.cpp:647)"
        );

        // Under 2 px of shallow water (y = 57): y-1 = 56 is water
        // (semi-solid) but not solid, and y-8 = 49 is above the surface.
        engine.objects[idx].state.position = Vector2::new(80, 57);
        engine.refresh_object_ocf(idx);
        assert_ne!(
            engine.object_ocf_at_index(idx) & ocf::AVAILABLE,
            0,
            "thin liquid cover keeps the object available"
        );

        // Deep under water (y = 55 at the 40..60 column): y-8 = 47 is
        // still water.
        engine.objects[idx].state.position = Vector2::new(100, 55);
        engine.refresh_object_ocf(idx);
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::AVAILABLE,
            0,
            "deep-sunk objects are not available (C4Object.cpp:647)"
        );
    }

    #[test]
    fn ocf_available_inside_containers_needs_get_access_or_entrance() {
        // OCF_Available container clause (SetOCF, C4Object.cpp:646):
        // !Contained || (Contained->Def->GrabPutGet & C4D_Grab_Get) ||
        // (Contained->OCF & OCF_Entrance).
        let mut engine = Engine::with_seed(4);
        let mut chest = simple_definition("Chest");
        chest.set_grab_put_get(2); // C4D_Grab_Get
        engine.register_definition(chest).expect("chest registers");
        let mut safe = simple_definition("Safe");
        safe.set_grab_put_get(1); // C4D_Grab_Put only
        engine.register_definition(safe).expect("safe registers");
        engine
            .register_definition(simple_definition("Rock"))
            .expect("rock registers");

        let chest_id = engine
            .spawn_object(SpawnConfig::new("Chest").with_position(Vector2::new(40, 20)))
            .expect("chest spawns");
        let safe_id = engine
            .spawn_object(SpawnConfig::new("Safe").with_position(Vector2::new(80, 20)))
            .expect("safe spawns");
        let rock = engine
            .spawn_object(
                SpawnConfig::new("Rock")
                    .with_position(Vector2::new(40, 20))
                    .with_container(chest_id),
            )
            .expect("rock spawns");
        let idx = engine.find_object_index(rock).expect("rock exists");
        assert_ne!(
            engine.object_ocf_at_index(idx) & ocf::AVAILABLE,
            0,
            "contents of a Grab_Get container stay available"
        );

        engine
            .apply_object_update(rock, ObjectUpdate::new().with_container(safe_id))
            .expect("move to safe");
        let idx = engine.find_object_index(rock).expect("rock exists");
        assert_eq!(
            engine.object_ocf_at_index(idx) & ocf::AVAILABLE,
            0,
            "a put-only container without entrance hides its contents"
        );
    }

    #[test]
    fn at_object_verifies_entrance_and_collection_areas_like_get_ocf_for_pos() {
        // C4Object::At runs GetOCFForPos on a hit (C4Object.cpp:1131-1160):
        // the returned mask keeps OCF_Entrance/OCF_Collection only when the
        // probe point lies inside the def's Entrance/Collection areas, and
        // a stripped mask no longer matching the request BLOCKS the scan
        // (C4GameObjects::AtObject, C4GameObjects.cpp:243-248).
        let mut engine = Engine::with_seed(4);
        let mut hut = simple_definition("Hut");
        hut.set_shape_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
        // Entrance only in the left half of the shape.
        hut.set_entrance_rect(Some(DefinitionRect::new(-20, -20, 20, 40)));
        engine.register_definition(hut).expect("hut registers");

        // Spawn y is the con-0 bottom: 60 - (40 - 20) puts the center at 40.
        engine
            .spawn_object(SpawnConfig::new("Hut").with_position(Vector2::new(40, 60)))
            .expect("hut spawns");

        let inside_entrance = engine.at_object(Vector2::new(30, 40), ocf::ENTRANCE, None);
        assert!(
            inside_entrance.is_some(),
            "probe inside the entrance area keeps OCF_Entrance"
        );
        assert_ne!(inside_entrance.expect("hit").2 & ocf::ENTRANCE, 0);

        assert!(
            engine
                .at_object(Vector2::new(50, 40), ocf::ENTRANCE, None)
                .is_none(),
            "probe inside the shape but outside the entrance area strips the bit"
        );
    }

    #[test]
    fn physics_clamps_horizontal_velocity() {
        let mut engine = Engine::with_seed(7);
        let definition = Definition::from_script(
            "Actor",
            "Actor",
            r#"
            global func Initialize(state, random) { return 0; }
            global func Step(state, frame, random) { return 0; }
            "#,
        )
        .expect("script compiles");
        engine
            .register_definition(definition)
            .expect("definition registers");
        let physics = PhysicsSettings::checked(1, 12, -20)
            .expect("physics valid")
            .with_max_horizontal_speed(4)
            .expect("horizontal speed valid");
        engine.set_physics(physics);

        let id = engine
            .spawn_object(
                SpawnConfig::new("Actor")
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(20, 0)),
            )
            .expect("spawn succeeds");

        let snapshot = engine.object_snapshot(id).expect("object snapshot");
        assert_eq!(snapshot.velocity.x, 4);

        engine
            .apply_object_update(id, ObjectUpdate::new().with_velocity(Vector2::new(-9, 0)))
            .expect("update applies");

        let snapshot = engine.object_snapshot(id).expect("object snapshot");
        assert_eq!(snapshot.velocity.x, -4);

        let tick_snapshot = engine.tick().expect("tick succeeds");
        let object = tick_snapshot.object(id).expect("object present");
        assert_eq!(object.velocity.x, -4);
    }

    #[test]
    fn queued_commands_apply_on_next_tick() {
        let script = r#"
        global func Step(state, frame, random) {
            return 0;
        }
        "#;

        let mut definition = Definition::from_script("Actor", "Actor", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        actions.insert("Jump".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(9);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Actor")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0))
                    .with_velocity(Vector2::new(0, 0)),
            )
            .expect("spawn succeeds");

        engine
            .queue_object_command(
                id,
                QueuedCommand::immediate(
                    ObjectUpdate::new()
                        .with_velocity(Vector2::new(3, -5))
                        .with_action("Jump"),
                ),
            )
            .expect("command enqueues");

        let snapshot = engine.tick().expect("first tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.action.name, "Jump");
        assert_eq!(object.velocity, Vector2::new(3, -5));
        assert_eq!(object.position, Vector2::new(3, -5));

        let snapshot = engine.tick().expect("second tick succeeds");
        let object = snapshot.object(id).expect("object present");
        assert_eq!(object.position, Vector2::new(6, -10));
    }

    #[test]
    fn queued_commands_lower_landscape_columns() {
        let script = r#"#strict 3
        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    commands = [
                        {
                            landscape = [
                                { op = "lower", start = 4, width = 3, height = 18 }
                            ]
                        }
                    ]
                };
            }
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Miner", "Miner", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(5);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(16, 10));

        engine
            .spawn_object(SpawnConfig::new("Miner"))
            .expect("spawn succeeds");

        engine.tick_without_snapshot().expect("first tick succeeds");
        let surface = engine
            .landscape()
            .expect("landscape present")
            .surface()
            .to_vec();
        assert_eq!(surface[4], 10);
        assert_eq!(surface[6], 10);

        engine.tick_without_snapshot().expect("second tick succeeds");
        let surface = engine
            .landscape()
            .expect("landscape present")
            .surface()
            .to_vec();
        assert_eq!(&surface[4..7], &[18, 18, 18]);
        assert_eq!(surface[7], 10);
    }

    #[test]
    fn queued_commands_set_and_clear_liquid_columns() {
        let script = r#"#strict 3
        global func Step(state, frame, random) {
            if (frame == 1) {
                return {
                    commands = [
                        {
                            landscape = [
                                { op = "set_liquid", column = 3, segments = [ { top = 5, bottom = 8 } ] }
                            ]
                        }
                    ]
                };
            }
            if (frame == 2) {
                return {
                    commands = [
                        {
                            landscape = [
                                { op = "clear_liquid", column = 3 }
                            ]
                        }
                    ]
                };
            }
            return nil;
        }
        "#;

        let mut definition = Definition::from_script("Diver", "Diver", script).unwrap();
        let mut actions = HashMap::new();
        actions.insert("Idle".to_string(), ActionSpec::default());
        definition.configure_actions(Some("Idle".to_string()), actions);

        let mut engine = Engine::with_seed(7);
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.set_landscape(Landscape::flat(16, 12));

        let diver_id = engine
            .spawn_object(SpawnConfig::new("Diver"))
            .expect("spawn succeeds");

        assert_eq!(engine.frame(), 0);

        engine.tick_without_snapshot().expect("first tick succeeds");
        assert!(engine.landscape().expect("landscape present").liquids()[3]
            .segments()
            .is_empty());

        engine.tick_without_snapshot().expect("second tick succeeds");
        assert_eq!(
            engine.landscape().expect("landscape present").liquids()[3].segments(),
            &[LiquidSegment::new(5, 8)]
        );

        engine.tick_without_snapshot().expect("third tick succeeds");
        assert!(engine.landscape().expect("landscape present").liquids()[3]
            .segments()
            .is_empty());

        // Ensure object persistence unaffected by landscape edits
        assert!(engine.object_snapshot(diver_id).is_some());
    }

    #[test]
    fn scenario_script_applies_landscape_commands() -> Result<(), EngineError> {
        const SCRIPT: &str = r#"#strict 3
        global func Initialize(state, random)
        {
            return {
                landscape = [
                    { op = "lower", start = 2, width = 2, height = 12 }
                ]
            };
        }

        global func Step(state, frame, random)
        {
            if (frame == 1)
            {
                return {
                    landscape = [
                        { op = "lower", start = 5, width = 2, height = 16 }
                    ]
                };
            }
            return nil;
        }
        "#;

        let mut engine = Engine::with_seed(11);
        engine.set_landscape(Landscape::flat(12, 8));

        engine
            .install_scenario_script("Scenario", SCRIPT)
            .expect("scenario script installs");

        let surface = engine
            .landscape()
            .expect("landscape present after install")
            .surface()
            .to_vec();
        assert_eq!(&surface[0..2], &[8, 8]);
        assert_eq!(&surface[2..4], &[12, 12]);

        let _snapshot = engine.tick()?;
        let surface = engine
            .landscape()
            .expect("landscape present after tick")
            .surface()
            .to_vec();
        assert_eq!(&surface[5..7], &[16, 16]);

        Ok(())
    }

    #[test]
    fn register_player_invokes_scenario_callbacks() -> Result<(), EngineError> {
        const SCRIPT: &str = r#"#strict 3
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }

        global func PreInitializePlayer(state, player)
        {
            return { physics = { gravity = 100 } };
        }

        global func InitializePlayer(state, player, x, y, base, team, extra)
        {
            return {
                spawn = [
                    { definition = "Flag", owner = player, position = [x, y] }
                ]
            };
        }

        global func RemovePlayer(state, player, team) { return nil; }
        global func OnGameOver(state) { return nil; }
        "#;

        let mut engine = Engine::with_seed(5);

        let mut crew_def = simple_definition("Crew");
        crew_def.set_crew_member(true);
        engine.register_definition(crew_def)?;

        let mut base_def = simple_definition("Base");
        base_def.set_category(CATEGORY_STRUCTURE);
        engine.register_definition(base_def)?;

        let mut flag_def = simple_definition("Flag");
        flag_def.set_category(CATEGORY_STRUCTURE);
        engine.register_definition(flag_def)?;

        let _crew_id = engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_alive(true)
                .with_owner(1)
                .with_crew_member(true)
                .with_position(Vector2::new(100, 200)),
        )?;
        let _base_id = engine.spawn_object(
            SpawnConfig::new("Base")
                .with_owner(1)
                .with_position(Vector2::new(150, 220)),
        )?;

        engine.install_scenario_script("Scenario", SCRIPT)?;

        engine.register_player(PlayerConfig::new(1, "Player"))?;

        assert_eq!(engine.physics().gravity, 100);

        let snapshot = engine.snapshot();
        let flag_snapshot = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "Flag")
            .expect("flag spawned by InitializePlayer");
        assert_eq!(flag_snapshot.owner, 1);
        assert_eq!(flag_snapshot.position, Vector2::new(100, 200));

        Ok(())
    }

    #[test]
    fn no_scenario_init_script_player_skips_broadcasts_and_runs_its_extra_callback(
    ) -> Result<(), EngineError> {
        const SCENARIO: &str = r#"
        global func PreInitializePlayer(player)
        {
            CreateObject(PREI, 0, 0, player);
        }
        global func InitializePlayer(player, x, y, base, team, extra)
        {
            CreateObject(INIT, 0, 0, player);
        }
        "#;
        const AI: &str = r#"
        func InitializeScriptPlayer(player, team)
        {
            CreateObject(MARK, team, 0, player);
        }
        "#;

        let mut engine = Engine::with_seed(17);
        for id in ["PREI", "INIT", "MARK"] {
            engine.register_definition(Definition::from_script(id, id, "")?)?;
        }
        engine.register_definition(Definition::from_script("__AI", "AI", AI)?)?;
        engine.install_scenario_script_with_convention("Scenario", SCENARIO, true)?;

        let info = ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Bot".to_vec()).expect("valid name"),
            flags: PLAYER_INFO_FLAG_NO_SCENARIO_INIT,
            id: 7,
            player_type: PLAYER_INFO_TYPE_SCRIPT,
            color: 0x0044_5566,
            original_color: 0x0044_5566,
            team: 2,
            extra_data: *b"__AI",
            ..Default::default()
        };
        let join = JoinPlayerControlData {
            info_id: info.id,
            ..Default::default()
        };
        let config = prepare_join_player_config(JoinPlayerPreparation {
            join: &join,
            info: &info,
            player_file: None,
            startup_player_count: 1,
        })
        .expect("script player config prepares");
        let rng_before = engine.snapshot().rng;
        let joined = engine.join_player_with_info(config, &info)?.number();
        let snapshot = engine.snapshot();

        assert_eq!(snapshot.rng, rng_before, "NoScenarioInit burns no setup RNG");
        let player = snapshot
            .players
            .iter()
            .find(|player| player.id == joined)
            .expect("script player joined");
        assert_eq!(player.team, Some(2));
        assert_eq!(player.color, Some(RgbColor::new(0x44, 0x55, 0x66)));
        assert!(snapshot.objects.iter().all(|object| {
            object.definition_id != "PREI" && object.definition_id != "INIT"
        }));
        let marker = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "MARK")
            .expect("extra-ID definition callback ran");
        assert_eq!(marker.owner, joined);
        assert_eq!(marker.position.x, 2);
        Ok(())
    }

    #[test]
    fn ordinary_script_player_forwards_extra_id_to_initialize_player() -> Result<(), EngineError> {
        const SCENARIO: &str = r#"
        global func InitializePlayer(player, x, y, base, team, extra)
        {
            if (extra == __AI) CreateObject(MARK, 0, 0, player);
        }
        "#;

        let mut engine = Engine::with_seed(19);
        engine.set_landscape(Landscape::flat(64, 48));
        engine.register_definition(Definition::from_script("MARK", "Marker", "")?)?;
        engine.install_scenario_script_with_convention("Scenario", SCENARIO, true)?;
        let info = ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Bot".to_vec()).expect("valid name"),
            id: 8,
            player_type: PLAYER_INFO_TYPE_SCRIPT,
            color: 0x0011_2233,
            original_color: 0x0011_2233,
            extra_data: *b"__AI",
            ..Default::default()
        };
        let join = JoinPlayerControlData {
            info_id: info.id,
            ..Default::default()
        };
        let config = prepare_join_player_config(JoinPlayerPreparation {
            join: &join,
            info: &info,
            player_file: None,
            startup_player_count: 1,
        })
        .expect("script player config prepares");
        let joined = engine.join_player_with_info(config, &info)?.number();

        assert!(engine.snapshot().objects.iter().any(|object| {
            object.definition_id == "MARK" && object.owner == joined
        }));
        Ok(())
    }

    #[test]
    fn script_player_type_queries_follow_joined_player_info() -> Result<(), EngineError> {
        const PROBE: &str = r#"#strict
public func QueryTypes(int user_player, int script_player)
{
    return [
        GetPlayerType(user_player),
        GetPlayerType(script_player),
        GetPlayerCount(),
        GetPlayerCount(1),
        GetPlayerCount(2),
        GetPlayerByIndex(0, 1),
        GetPlayerByIndex(0, 2),
        GetPlayerByIndex(1, 2)
    ];
}
"#;

        let mut engine = Engine::new();
        engine.set_landscape(Landscape::flat(64, 48));
        engine.register_definition(Definition::from_script("PROB", "Probe", PROBE)?)?;
        let probe = engine.spawn_object(SpawnConfig::new("PROB").with_loaded(true))?;
        let probe_index = engine.find_object_index(probe).expect("probe exists");

        let user = engine
            .join_player(JoinPlayerConfig {
                name: "User".to_string(),
                player_info_id: 1,
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
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 2,
            })?
            .number();
        assert_eq!(
            engine.call_object_function(
                probe_index,
                "QueryTypes",
                vec![Value::Int(user), Value::Int(99)],
            )?,
            Value::Array(vec![
                Value::Int(1),
                Value::Nil,
                Value::Int(1),
                Value::Int(1),
                Value::Int(0),
                Value::Int(user),
                Value::Int(OWNER_NONE),
                Value::Int(OWNER_NONE),
            ]),
            "a user-only player list has no type-2 entry"
        );

        let info = ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Bot".to_vec()).expect("valid name"),
            id: 2,
            player_type: PLAYER_INFO_TYPE_SCRIPT,
            color: 0x0000_ff00,
            original_color: 0x0000_ff00,
            ..Default::default()
        };
        let join = JoinPlayerControlData {
            info_id: info.id,
            ..Default::default()
        };
        let config = prepare_join_player_config(JoinPlayerPreparation {
            join: &join,
            info: &info,
            player_file: None,
            startup_player_count: 2,
        })
        .expect("script player config prepares");
        let script_player = engine.join_player_with_info(config, &info)?.number();

        assert_eq!(
            engine.call_object_function(
                probe_index,
                "QueryTypes",
                vec![Value::Int(user), Value::Int(script_player)],
            )?,
            Value::Array(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(2),
                Value::Int(1),
                Value::Int(1),
                Value::Int(user),
                Value::Int(script_player),
                Value::Int(OWNER_NONE),
            ])
        );
        Ok(())
    }

    #[test]
    fn scenario_scoreboard_writes_preserve_cpp_row_column_order_and_header_keys(
    ) -> Result<(), EngineError> {
        // FnSetScoreboardData receives (row, col) but forwards (col, row),
        // and C4Scoreboard::SetCell appends missing columns/rows while header
        // cells keep their key instead of iData (C4Script.cpp:5881-5884;
        // C4Scoreboard.cpp:138-175).
        const SCRIPT: &str = r#"
        global func Initialize()
        {
            var race = ScoreboardCol(RACE);
            SetScoreboardData(SBRD_Caption, SBRD_Caption, "Race", 123);
            SetScoreboardData(SBRD_Caption, race, "{{RACE}}", 456);
            SetScoreboardData(7, SBRD_Caption, "Team", 789);
            SetScoreboardData(7, race, "75%", 75);
        }
        "#;

        let mut engine = Engine::new();
        engine.install_scenario_script_with_convention("Scoreboard", SCRIPT, true)?;
        let scoreboard = engine.snapshot().hud.scoreboard;

        assert_eq!(scoreboard.row_count(), 2);
        assert_eq!(scoreboard.column_count(), 2);
        assert_eq!(
            scoreboard.cell(0, 0).and_then(ScoreboardCell::text),
            Some("Race")
        );
        assert_eq!(scoreboard.cell(0, 0).map(ScoreboardCell::value), Some(-1));
        assert_eq!(
            scoreboard.cell(0, 1).and_then(ScoreboardCell::text),
            Some("{{RACE}}")
        );
        assert_eq!(
            scoreboard.cell(0, 1).map(ScoreboardCell::value),
            Some(i32::from_le_bytes(*b"RACE"))
        );
        assert_eq!(
            scoreboard.cell(1, 0).and_then(ScoreboardCell::text),
            Some("Team")
        );
        assert_eq!(scoreboard.cell(1, 0).map(ScoreboardCell::value), Some(7));
        assert_eq!(
            scoreboard.cell(1, 1).and_then(ScoreboardCell::text),
            Some("75%")
        );
        assert_eq!(scoreboard.cell(1, 1).map(ScoreboardCell::value), Some(75));
        Ok(())
    }

    #[test]
    fn team_query_hosts_follow_cpp_list_order_and_missing_value_rules() {
        // FnGetTeamName/Color/ByIndex/Count query Game.Teams directly
        // (C4Script.cpp:5803-5824); GetTeamByIndex is list-order and rejects
        // out-of-range indices (C4Teams.cpp:423-430).
        let script = r#"#strict
func Probe() {
  return [GetTeamCount(), GetTeamByIndex(0), GetTeamByIndex(1), GetTeamByIndex(2),
          GetTeamColor(2), GetTeamColor(99), GetTeamName(1), GetTeamName(99)];
}
"#;
        let mut engine = Engine::with_seed(0);
        engine.set_teams(vec![
            TeamInfo::new(2, "Right", 0x00f4_faf4),
            TeamInfo::new(1, "Left", 0x00e0_4a9c),
        ]);
        engine
            .register_definition(Definition::from_script("TEAM", "Teams", script).unwrap())
            .unwrap();
        let object = engine
            .spawn_object(SpawnConfig::new("TEAM"))
            .expect("team probe spawns");
        let index = engine.find_object_index(object).expect("team probe exists");

        assert_eq!(
            engine
                .call_object_function(index, "Probe", Vec::new())
                .expect("team queries run"),
            Value::Array(vec![
                Value::Int(2),
                Value::Int(2),
                Value::Int(1),
                Value::Nil,
                Value::Int(0x00f4_faf4),
                Value::Nil,
                Value::String("Left".to_string().into()),
                Value::Nil,
            ])
        );
    }

    #[test]
    fn scenario_scoreboard_sort_is_stable_and_keeps_the_caption_row() -> Result<(), EngineError> {
        // C4Scoreboard::SortBy leaves row zero fixed and cocktail-sorts data
        // rows by iVal; strict comparisons preserve equal-key insertion
        // order, and fReverse selects descending order
        // (C4Scoreboard.cpp:199-225; FnSortScoreboard at C4Script.cpp:5910-5913).
        const SCRIPT: &str = r#"
        global func Initialize()
        {
            SetScoreboardData(SBRD_Caption, 9, "Value", 0);
            SetScoreboardData(1, SBRD_Caption, "one", 0);
            SetScoreboardData(1, 9, "20a", 20);
            SetScoreboardData(2, SBRD_Caption, "two", 0);
            SetScoreboardData(2, 9, "10", 10);
            SetScoreboardData(3, SBRD_Caption, "three", 0);
            SetScoreboardData(3, 9, "20b", 20);
            SortScoreboard(9, true);
        }
        "#;

        let mut engine = Engine::new();
        engine.install_scenario_script_with_convention("Scoreboard", SCRIPT, true)?;
        let scoreboard = engine.snapshot().hud.scoreboard;
        let row_keys = (0..scoreboard.row_count())
            .filter_map(|row| scoreboard.cell(row, 0).map(ScoreboardCell::value))
            .collect::<Vec<_>>();
        assert_eq!(row_keys, vec![SCOREBOARD_CAPTION, 1, 3, 2]);
        Ok(())
    }

    #[test]
    fn scoreboard_nil_prunes_but_empty_string_persists_through_save_restore(
    ) -> Result<(), EngineError> {
        // SetCell prunes rows/columns only when every scanned StdStrBuf is
        // null; Copy("") keeps a non-null empty buffer, so it survives. The
        // complete matrix and runtime iDlgShow are saved by CompileFunc
        // (C4Scoreboard.cpp:156-173,266-286; StdBuf.h:438,527).
        const SCRIPT: &str = r#"
        global func Initialize()
        {
            SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores", -1);
            SetScoreboardData(10, 1, "gone", 10);
            SetScoreboardData(10, 1);
            SetScoreboardData(20, 2, "", 20);
        }
        global func Show()
        {
            DoScoreboardShow(3);
        }
        "#;

        let mut engine = Engine::new();
        engine.install_scenario_script_with_convention("Scoreboard", SCRIPT, true)?;
        assert_eq!(
            engine.snapshot().hud.scoreboard.show_count(),
            0,
            "exclusive initialization returns before mutating iDlgShow"
        );
        engine.begin_scoreboard_presentation_capture();
        engine.call_scenario_script_function("Show", Vec::new())?;
        let scoreboard = engine.snapshot().hud.scoreboard;
        assert_eq!(scoreboard.row_count(), 2);
        assert_eq!(scoreboard.column_count(), 2);
        assert_eq!(scoreboard.cell(1, 0).map(ScoreboardCell::value), Some(20));
        assert_eq!(scoreboard.cell(0, 1).map(ScoreboardCell::value), Some(2));
        assert_eq!(
            scoreboard.cell(1, 1).and_then(ScoreboardCell::text),
            Some("")
        );
        assert_eq!(scoreboard.show_count(), 3);

        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("scoreboard state serializes");
        let decoded = EngineState::from_json_str(&encoded).expect("scoreboard state deserializes");
        let mut restored = Engine::new();
        restored.restore_state(&decoded)?;
        assert_eq!(restored.snapshot().hud.scoreboard, scoreboard);
        Ok(())
    }

    #[test]
    fn scenario_scoreboard_show_respects_the_engine_local_player_set() -> Result<(), EngineError> {
        // FnDoScoreboardShow returns true but does not touch iDlgShow when
        // iForPlr names an existing non-local player (C4Script.cpp:5896-5908).
        const SCRIPT: &str = r#"
        global func Initialize()
        {
            SetScoreboardData(SBRD_Caption, SBRD_Caption, "Scores", -1);
        }
        global func Show()
        {
            DoScoreboardShow(2, 1);
        }
        "#;

        let mut engine = Engine::new();
        engine.register_player(PlayerConfig::new(0, "Remote"))?;
        engine.set_local_players([]);
        engine.install_scenario_script_with_convention("Scoreboard", SCRIPT, true)?;
        engine.begin_scoreboard_presentation_capture();
        engine.call_scenario_script_function("Show", Vec::new())?;
        assert_eq!(engine.snapshot().hud.scoreboard.show_count(), 0);
        Ok(())
    }

    #[test]
    fn runtime_scoreboard_requests_capture_call_time_dimensions_and_order(
    ) -> Result<(), EngineError> {
        // SetCell never reconciles pDlg. In particular, a DoDlgShow while the
        // matrix is empty cannot become visible retroactively when a cell is
        // added later in the same frame (C4Scoreboard.cpp:138-175,234-256).
        const SCRIPT: &str = r#"
        global func EmptyThenCell()
        {
            DoScoreboardShow(1);
            SetScoreboardData(SBRD_Caption, SBRD_Caption, "late");
        }
        global func OpenThenClose()
        {
            DoScoreboardShow(1);
            DoScoreboardShow(-2);
        }
        "#;

        let mut engine = Engine::new();
        engine.install_scenario_script_with_convention("Scoreboard", SCRIPT, true)?;
        engine.begin_scoreboard_presentation_capture();
        engine.call_scenario_script_function("EmptyThenCell", Vec::new())?;
        let empty_then_cell = engine.tick()?;
        assert_eq!(empty_then_cell.hud.scoreboard.show_count(), 1);
        assert_eq!(empty_then_cell.hud.scoreboard.row_count(), 1);
        let requests = &empty_then_cell.hud.scoreboard_presentations;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            (
                requests[0].rows,
                requests[0].columns,
                requests[0].show_count,
                requests[0].layout_revision,
                requests[0].title_widget_present,
            ),
            (0, 0, 1, 0, false),
        );
        assert_eq!(requests[0].scoreboard.row_count(), 0);
        assert_eq!(requests[0].scoreboard.column_count(), 0);
        assert_eq!(requests[0].scoreboard.show_count(), 1);

        engine.call_scenario_script_function("OpenThenClose", Vec::new())?;
        let open_then_close = engine.tick()?;
        let requests = &open_then_close.hud.scoreboard_presentations;
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests
                .iter()
                .map(|request| {
                    (
                        request.rows,
                        request.columns,
                        request.show_count,
                        request.layout_revision,
                        request.title_widget_present,
                        request.scoreboard.show_count(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![(1, 1, 2, 1, true, 2), (1, 1, 0, 1, true, 0)],
        );
        Ok(())
    }

    #[test]
    fn tutorial_show_control_mask_reaches_snapshots_and_save_state() -> Result<(), EngineError> {
        // FnSetPlrShowControl writes C4Player::ShowControl during
        // InitializePlayer (C4Script.cpp:2546-2551); C4Player::CompileFunc
        // persists it as "ShowControl" (C4Player.cpp:1583).
        const SCRIPT: &str = r#"
        global func InitializePlayer(player)
        {
            SetPlrShowControl(player, "x_ x");
        }
        "#;

        let mut engine = Engine::with_seed(5);
        engine.install_scenario_script_with_convention("Scenario", SCRIPT, true)?;
        engine.register_player(PlayerConfig::new(0, "Player"))?;

        let snapshot = engine.snapshot();
        let player = snapshot
            .players
            .iter()
            .find(|player| player.id == 0)
            .expect("registered player is present");
        assert_eq!(player.show_control, 9);

        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("state encodes");
        let decoded = EngineState::from_json_str(&encoded).expect("state decodes");
        assert_eq!(decoded.players[0].show_control, 9);

        let mut restored = Engine::with_seed(0);
        restored.restore_state(&decoded)?;
        assert_eq!(restored.snapshot().players[0].show_control, 9);
        Ok(())
    }

    #[test]
    fn register_player_survives_initialize_player_script_error() -> Result<(), EngineError> {
        // C4Player.cpp:769 calls Game.Script.GRBroadcast(PSF_InitializePlayer,
        // ...) with the default fPassError=false: a script error is logged by
        // the fail-safe exec (C4AulExec.cpp:1318-1342) and the join continues.
        const SCRIPT: &str = r#"
        global func PreInitializePlayer(state, player) { return ThisFunctionDoesNotExist(); }
        global func InitializePlayer(state, player, x, y, base, team, extra)
        {
            return ThisFunctionDoesNotExist();
        }
        "#;

        let mut engine = Engine::with_seed(5);
        engine.install_scenario_script("Scenario", SCRIPT)?;
        engine.register_player(PlayerConfig::new(1, "Player"))?;
        assert!(engine.players().any(|player| player.id() == 1));
        Ok(())
    }

    #[test]
    fn scenario_cast_particles_creates_and_executes_system_particles() -> Result<(), EngineError> {
        // End-to-end FnCastParticles → C4ParticleSystem::Cast → fxStdExec:
        // (C4Script.cpp:4881-4903, C4Particles.cpp:421-443,614-697).
        // level = 0 makes the cast velocity spread deterministic (zero), so
        // PushParticles and gravity fully determine the motion.
        const SCRIPT: &str = r#"
        global func Initialize(state, random) {
            CastParticles("Flame", 5, 0, 60, 40, 10, 20, 100, 200);
            PushParticles("Flame", 10, 0);
            return 0;
        }
        global func Step(state, frame, random) { return 0; }
        "#;
        let mut engine = Engine::with_seed(3);
        let core = particles::ParticleDefCore {
            name: "Flame".into(),
            init_fn: "StdInit".into(),
            exec_fn: "StdExec".into(),
            draw_fn: "Std".into(),
            gravity_acc: 100,
            delay: 1,
            repeats: 1000,
            ..Default::default()
        };
        engine
            .register_particle_definition(core, 8, 1.0)
            .expect("def registers");
        engine.install_scenario_script("Scenario", SCRIPT)?;

        let system = engine.particle_system();
        assert_eq!(system.particles().len(), 5, "cast created 5 particles");
        assert_eq!(system.get_def("Flame").unwrap().count, 5);
        for particle in system.particles() {
            assert_eq!(particle.x.to_bits(), 60.0f32.to_bits());
            assert_eq!(particle.y.to_bits(), 40.0f32.to_bits());
            assert_eq!(particle.xdir.to_bits(), 1.0f32.to_bits(), "pushed");
            assert_eq!(particle.ydir.to_bits(), 0.0f32.to_bits());
        }

        engine.tick_without_snapshot()?;
        let gravity = engine.physics().gravity_as_c4fixed();
        let expected_ydir =
            math::fixtof(math::C4Fixed::from_raw(gravity.val().wrapping_mul(100))) / 100.0;
        for particle in engine.particle_system().particles() {
            assert_eq!(particle.x.to_bits(), 61.0f32.to_bits(), "moved by xdir");
            assert_eq!(particle.ydir.to_bits(), expected_ydir.to_bits(), "gravity");
            assert_eq!(particle.life, 1, "delay lifetime advanced");
        }

        let snapshot = engine.snapshot();
        assert_eq!(
            snapshot
                .particles
                .iter()
                .filter(|particle| particle.definition_id == "Flame")
                .count(),
            5,
            "system particles appear in the snapshot"
        );
        Ok(())
    }

    #[test]
    fn particle_wind_is_suppressed_by_tunnel_background_like_cpp() -> Result<(), EngineError> {
        // Both fxStdExec WindDrift and fxSmokeExec read GBackWind at the
        // particle position (C4Particles.cpp:556-562,649-660). GBackWind is
        // zero exactly where GBackIFT is set (C4Wrappers.h:189-192), while an
        // adjacent sky pixel receives the current Weather.Wind.
        const SCRIPT: &str = r#"
        global func Initialize(state, random) {
            CreateParticle("Leaf", 5, 10, 0, 0, 0, 1);
            CreateParticle("Leaf", 6, 10, 0, 0, 0, 1);
            return 0;
        }
        global func Step(state, frame, random) { return 0; }
        "#;

        let mut engine = Engine::with_seed(3);
        engine.set_environment(EnvironmentSettings::new(60));
        let mut landscape = Landscape::flat(32, 100);
        landscape.set_tunnel_column(5, vec![(0, 20)]);
        engine.set_landscape(landscape);
        engine
            .register_particle_definition(
                particles::ParticleDefCore {
                    name: "Leaf".into(),
                    init_fn: "StdInit".into(),
                    exec_fn: "StdExec".into(),
                    draw_fn: "Std".into(),
                    wind_drift: 100,
                    delay: 1,
                    repeats: 1000,
                    ..Default::default()
                },
                4,
                1.0,
            )
            .expect("particle definition registers");
        engine.install_scenario_script("Scenario", SCRIPT)?;

        engine.tick_without_snapshot()?;

        let particles = engine.particle_system().particles();
        assert_eq!(particles.len(), 2);
        assert_eq!(
            particles[0].xdir.to_bits(),
            0.0f32.to_bits(),
            "IFT tunnel background blocks wind"
        );
        let expected_open_xdir = ((60.0f32 / 15.0) * 80.0) / 800.0;
        assert_eq!(
            particles[1].xdir.to_bits(),
            expected_open_xdir.to_bits(),
            "adjacent sky receives Weather.Wind"
        );
        Ok(())
    }

    #[test]
    fn remove_player_triggers_on_game_over() -> Result<(), EngineError> {
        const SCRIPT: &str = r#"#strict 3
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random) { return nil; }
        global func PreInitializePlayer(state, player) { return nil; }
        global func InitializePlayer(state, player, x, y, base, team, extra) { return nil; }
        global func RemovePlayer(state, player, team)
        {
            return { physics = { gravity = 50 } };
        }
        global func OnGameOver(state)
        {
            return { physics = { gravity = 77 } };
        }
        "#;

        let mut engine = Engine::with_seed(11);
        engine.register_definition(simple_definition("Crew"))?;

        engine.install_scenario_script("Scenario", SCRIPT)?;

        engine.register_player(PlayerConfig::new(1, "Player"))?;

        let _ = engine.remove_player(1)?;

        assert_eq!(engine.physics().gravity, 77);

        Ok(())
    }

    #[test]
    fn pending_team_selection_counts_as_not_eliminated_for_game_over() -> Result<(), EngineError> {
        // C4Game::GameOverCheck uses Players.GetCountNotEliminated, which
        // counts every registered player whose separate Eliminated flag is
        // false—including PS_TeamSelection and PS_TeamSelectionPending
        // (C4Game.cpp:652-665; C4PlayerList.cpp:565-572).
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(1, "Left", 0x00f4_0000),
            TeamInfo::new(2, "Right", 0x0000_c800),
        ]);
        engine.set_runtime_join_team_choice(true);
        let joined = engine.join_player(JoinPlayerConfig {
            name: "Chooser".to_string(),
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
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })?;
        assert_eq!(
            joined,
            JoinPlayerOutcome::AwaitingTeamSelection { number: 0 }
        );

        let selection = engine.tick()?;
        assert!(!selection.game_over);
        engine.mark_team_selection_pending(0)?;
        let pending = engine.tick()?;
        assert!(!pending.game_over);
        Ok(())
    }

    #[test]
    fn get_player_team_distinguishes_team_selection_from_unteamed() -> Result<(), EngineError> {
        // FnGetPlayerTeam returns an assigned roster team first, then -1 for
        // PS_TeamSelection/PS_TeamSelectionPending, 0 for a settled teamless
        // player, and nil for a missing player (C4Script.cpp:5716-5728).
        let script = r#"#strict 2
func ReadTeam(int player) { return GetPlayerTeam(player); }
"#;
        let mut engine = Engine::new();
        engine.register_definition(
            Definition::from_script("TEAM", "Team reader", script)
                .expect("team reader compiles"),
        )?;
        let reader = engine.spawn_object(SpawnConfig::new("TEAM"))?;
        engine.set_teams(vec![
            TeamInfo::new(1, "Left", 0x00f4_0000),
            TeamInfo::new(2, "Right", 0x0000_c800),
        ]);
        engine.set_runtime_join_team_choice(true);

        let outcome = engine.join_player(lifecycle_join_config("Chooser", Vec::new()))?;
        assert_eq!(
            outcome,
            JoinPlayerOutcome::AwaitingTeamSelection { number: 0 }
        );
        let number = outcome.number();
        let read_team = |engine: &mut Engine, player| {
            let index = engine.find_object_index(reader).expect("reader exists");
            engine.call_object_function(index, "ReadTeam", vec![Value::Int(player)])
        };

        assert_eq!(read_team(&mut engine, number)?, Value::Int(-1));
        engine.mark_team_selection_pending(number)?;
        assert_eq!(read_team(&mut engine, number)?, Value::Int(-1));

        engine
            .initialize_scenario_player(number, 2)?
            .expect("selected team is accepted");
        assert_eq!(read_team(&mut engine, number)?, Value::Int(2));
        assert_eq!(read_team(&mut engine, 99)?, Value::Nil);
        Ok(())
    }

    #[test]
    fn init_scenario_player_builtin_resumes_pending_team_choice() -> Result<(), EngineError> {
        // FnInitScenarioPlayer resolves a live player and synchronously runs
        // ScenarioAndTeamInit. Missing players return false; a rejected team
        // reopens a pending selection; an accepted team resumes the existing
        // ScenarioInit/FinalInit path (C4Script.cpp:5827-5832;
        // C4Player.cpp:111-151).
        let script = r#"#strict 2
func InitPlayer(int player, int team) { return InitScenarioPlayer(player, team); }
func ReadTeam(int player) { return GetPlayerTeam(player); }
"#;
        let mut engine = Engine::new();
        engine.register_definition(
            Definition::from_script("TEAM", "Team initializer", script)
                .expect("team initializer compiles"),
        )?;
        let caller = engine.spawn_object(SpawnConfig::new("TEAM"))?;
        engine.set_teams(vec![
            TeamInfo::new(1, "Left", 0x00f4_0000),
            TeamInfo::new(2, "Right", 0x0000_c800),
        ]);
        engine.set_runtime_join_team_choice(true);
        let outcome = engine.join_player(lifecycle_join_config("Chooser", Vec::new()))?;
        assert_eq!(
            outcome,
            JoinPlayerOutcome::AwaitingTeamSelection { number: 0 }
        );
        let number = outcome.number();
        let call = |engine: &mut Engine, function: &str, args| {
            let index = engine.find_object_index(caller).expect("caller exists");
            engine.call_object_function(index, function, args)
        };

        assert_eq!(
            call(
                &mut engine,
                "InitPlayer",
                vec![Value::Int(99), Value::Int(2)]
            )?,
            Value::Bool(false),
            "missing player is rejected"
        );

        engine.mark_team_selection_pending(number)?;
        assert_eq!(
            call(
                &mut engine,
                "InitPlayer",
                vec![Value::Int(number), Value::Int(99)]
            )?,
            Value::Bool(false),
            "missing team returns ScenarioAndTeamInit's false result"
        );
        assert_eq!(
            engine.player(number).map(Player::status),
            Some(PlayerStatus::TeamSelection),
            "OnTeamSelectionFailed reopens the selection"
        );

        engine.mark_team_selection_pending(number)?;
        assert_eq!(
            call(
                &mut engine,
                "InitPlayer",
                vec![Value::Int(number), Value::Int(2)]
            )?,
            Value::Bool(true)
        );
        assert_eq!(engine.player(number).and_then(Player::team), Some(2));
        assert_eq!(
            call(&mut engine, "ReadTeam", vec![Value::Int(number)])?,
            Value::Int(2),
            "the script-visible player now carries the selected team"
        );
        Ok(())
    }

    #[test]
    fn surrender_player_builtin_rejects_eliminated_players_and_retires_valid_player(
    ) -> Result<(), EngineError> {
        // FnSurrenderPlayer rejects missing/already-eliminated players, sets
        // Surrendered and Eliminated synchronously, and starts the same
        // 60-frame retirement path as C4ControlSurrenderPlayer
        // (C4Script.cpp:2843-2850; C4Player.cpp:971-979).
        let script = r#"#strict 2
func Probe(int player, int eliminated)
{
    var no_section;
    return [SurrenderPlayer(99), SurrenderPlayer(eliminated),
            SurrenderPlayer(player),
            GetPlayerVal("Eliminated", no_section, player),
            GetPlayerVal("Surrendered", no_section, player),
            GetPlayerVal("Status", no_section, player),
            EliminatePlayer(player), SurrenderPlayer(player)];
}
"#;
        let scenario_script = r#"#strict 2
global func RemovePlayer(int player, int team)
{
    var no_section;
    SetGravity(1);
    if (player == 7 && team == 3
        && GetPlayerVal("Evaluated", no_section, player)
        && GetPlayerVal("Eliminated", no_section, player)
        && GetPlayerVal("Surrendered", no_section, player)
        && GetCrewCount(player) == 1)
        SetGravity(73);
    return true;
}
"#;
        let crew_script = r#"#strict 2
local destruction_calls;
protected func Destruction() { destruction_calls++; return true; }
"#;
        let item_script = r#"#strict 2
local departure_calls;
protected func Departure(object old_container) { departure_calls++; return true; }
"#;
        let mut engine = Engine::new();
        engine.register_definition(
            Definition::from_script("SURR", "Surrender probe", script)
                .expect("surrender probe compiles"),
        )?;
        let mut crew_definition =
            Definition::from_script("CREW", "Departing crew", crew_script)?;
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition)?;
        engine.register_definition(Definition::from_script(
            "ITEM",
            "Ejected item",
            item_script,
        )?)?;
        engine.install_scenario_script_with_convention(
            "Scenario",
            scenario_script,
            true,
        )?;
        let caller = engine.spawn_object(SpawnConfig::new("SURR"))?;
        engine.register_player(
            PlayerConfig::new(7, "Surrendering")
                .with_player_info_id(41)
                .with_team(Some(3))
                .with_score(250),
        )?;
        engine.register_player(
            PlayerConfig::new(8, "Eliminated").with_status(PlayerStatus::Eliminated),
        )?;
        let crew = engine.spawn_object(
            SpawnConfig::new("CREW")
                .with_owner(7)
                .with_alive(true)
                .with_crew_member(true)
                .with_position(Vector2::new(40, 50)),
        )?;
        let item = engine.spawn_object(SpawnConfig::new("ITEM").with_container(crew))?;
        assert_eq!(
            engine.player(7).expect("player exists").crew(),
            &[crew],
            "the removal cascade follows the stored player Crew list"
        );
        engine.select_crew(7, [crew])?;
        engine.set_crew_cursor(7, Some(crew))?;
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine.call_object_function(
                caller_index,
                "Probe",
                vec![Value::Int(7), Value::Int(8)],
            )?,
            Value::Array(vec![
                Value::Bool(false),
                Value::Bool(false),
                Value::Bool(true),
                Value::Int(1),
                Value::Int(1),
                Value::Int(1),
                Value::Int(0),
                Value::Bool(false),
            ])
        );
        let surrendered = engine.player(7).expect("surrendered player remains");
        assert_eq!(surrendered.status(), PlayerStatus::Surrendered);
        assert!(surrendered.surrendered());
        assert!(engine.is_owner_eliminated(7));

        // ObjectCom/ObjectCommand reject the eliminated flag immediately,
        // before hiding ShowStartup or mutating the selected crew object.
        assert!(surrendered.show_startup());
        engine.player_direct_com(7, COM_UP, 0)?;
        assert!(engine.player(7).expect("player remains").show_startup());
        assert!(!engine.player_object_command(7, CommandId::Wait, None, 0, 0)?);
        assert!(engine.player(7).expect("player remains").show_startup());
        assert!(
            engine
                .object_snapshot(crew)
                .expect("crew remains")
                .command_stack
                .is_empty()
        );

        for _ in 0..59 {
            engine.tick_player_systems()?;
        }
        assert!(engine.player(7).is_some(), "retirement waits for frame 60");
        assert_eq!(
            engine.object_snapshot(item).and_then(|item| item.container),
            Some(crew),
            "crew contents stay contained until retirement"
        );
        engine.tick_player_systems()?;
        assert!(engine.player(7).is_none(), "player retires on frame 60");
        assert_eq!(
            engine.physics().gravity,
            73,
            "Evaluate precedes RemovePlayer while the player and Crew stay live"
        );

        let removed_crew = engine.object_snapshot(crew).expect("crew tombstone remains");
        assert_eq!(removed_crew.status, ObjectStatus::Deleted);
        assert_eq!(
            removed_crew.local_vars.get("destruction_calls"),
            Some(&Value::Int(1)),
            "RemoveCrewObjects uses the full AssignRemoval callback path"
        );
        let ejected_item = engine.object_snapshot(item).expect("contained item survives");
        assert_eq!(ejected_item.container, None);
        assert_eq!(ejected_item.position, Vector2::new(40, 50));
        assert_eq!(
            ejected_item.local_vars.get("departure_calls"),
            Some(&Value::Int(1)),
            "AssignRemoval(true) exits rather than recursively deleting contents"
        );
        let result = engine
            .round_results
            .players
            .iter()
            .find(|result| result.player_info_id == 41)
            .expect("surrender evaluation is recorded");
        assert_eq!((result.score_old, result.score_new), (250, Some(250)));
        Ok(())
    }

    #[test]
    fn script_set_owner_runs_the_full_native_owner_change_sequence() -> Result<(), EngineError> {
        // C4Object::SetOwner validates first, refreshes the CURRENT graphics'
        // ColorByOwner surface, then writes Owner/Controller, transfers a
        // FLAG/FlyBase target's Base, and finally calls OnOwnerChanged.
        // An explicit foreign target must not redispatch to a script function
        // named SetOwner on that target.
        const FLAG_SCRIPT: &str = r#"#strict 2
local base_target, owner_changes, seen_new, seen_old;
local seen_owner, seen_controller, seen_color, seen_base, shadow_calls;

public func Arm(object target) { base_target = target; return true; }
public func SetOwner() { shadow_calls++; return false; }

protected func OnOwnerChanged(int new_owner, int old_owner)
{
    owner_changes++;
    seen_new = new_owner;
    seen_old = old_owner;
    seen_owner = GetOwner();
    seen_controller = GetController();
    seen_color = GetColorDw();
    seen_base = GetBase(base_target);
    return true;
}
"#;
        const CALLER_SCRIPT: &str = r#"#strict 2
public func Prepare(object target, object base)
{
    var no_graphics;
    target->Arm(base);
    return SetGraphics(no_graphics, target, SKIN);
}
public func Change(object target, int owner) { return SetOwner(owner, target); }
public func RefreshSame(object target)
{
    SetColorDw(0x123456, target);
    SetController(1, target);
    return SetOwner(GetOwner(target), target);
}
"#;
        const BIRTH_SCRIPT: &str = r#"#strict 2
local owner_changes, seen_after, seen_new, seen_old, seen_controller;

protected func Construction()
{
    SetOwner(2);
    seen_after = owner_changes;
}

protected func OnOwnerChanged(int new_owner, int old_owner)
{
    owner_changes++;
    seen_new = new_owner;
    seen_old = old_owner;
    seen_controller = GetController();
}
"#;

        let mut engine = Engine::new();
        engine.register_player(
            PlayerConfig::new(1, "Old")
                .with_color(Some(RgbColor::new(0xaa, 0, 0))),
        )?;
        engine.register_player(
            PlayerConfig::new(2, "New")
                .with_color(Some(RgbColor::new(0x44, 0x55, 0x66))),
        )?;

        engine.register_definition(simple_definition("BASE"))?;
        let mut skin = simple_definition("SKIN");
        skin.set_color_by_owner(true);
        engine.register_definition(skin)?;
        let mut flag = Definition::from_script("FLAG", "Flag", FLAG_SCRIPT)?;
        flag.configure_actions(
            Some("Idle".to_string()),
            HashMap::from([
                ("Idle".to_string(), ActionSpec::default()),
                ("FlyBase".to_string(), ActionSpec::default()),
            ]),
        );
        engine.register_definition(flag)?;
        engine.register_definition(Definition::from_script(
            "CALL",
            "Caller",
            CALLER_SCRIPT,
        )?)?;
        engine.register_definition(Definition::from_script(
            "BORN",
            "Construction owner probe",
            BIRTH_SCRIPT,
        )?)?;

        let base = engine.spawn_object(
            SpawnConfig::new("BASE")
                .with_owner(1)
                .with_status(ObjectStatus::Inactive),
        )?;
        engine.apply_object_update(base, ObjectUpdate::new().with_base(1))?;
        let mut fly_base = ActionState::new("FlyBase");
        fly_base.target = Some(base);
        let flag = engine.spawn_object(
            SpawnConfig::new("FLAG")
                .with_owner(1)
                .with_action(fly_base),
        )?;
        let caller = engine.spawn_object(SpawnConfig::new("CALL").with_owner(1))?;
        let caller_index = engine.find_object_index(caller).expect("caller exists");

        assert_eq!(
            engine.call_object_function(
                caller_index,
                "Prepare",
                vec![object_reference_value(flag), object_reference_value(base)],
            )?,
            Value::Bool(true)
        );
        assert_eq!(
            engine.call_object_function(
                caller_index,
                "Change",
                vec![object_reference_value(flag), Value::Int(2)],
            )?,
            Value::Bool(true)
        );

        let changed = engine.object_snapshot(flag).expect("flag survives");
        assert_eq!((changed.owner, changed.controller), (2, 2));
        assert_eq!(changed.color, 0x0044_5566);
        assert_eq!(
            engine.object_snapshot(base).map(|object| object.base),
            Some(2),
            "inactive FlyBase targets still have nonzero C++ Status"
        );
        for (name, value) in [
            ("owner_changes", Value::Int(1)),
            ("seen_new", Value::Int(2)),
            ("seen_old", Value::Int(1)),
            ("seen_owner", Value::Int(2)),
            ("seen_controller", Value::Int(2)),
            ("seen_color", Value::Int(0x0044_5566)),
            ("seen_base", Value::Int(2)),
            ("shadow_calls", Value::Nil),
        ] {
            assert_eq!(changed.local_vars.get(name), Some(&value), "local {name}");
        }

        // Invalid owners are a true no-op and do not fire the callback.
        assert_eq!(
            engine.call_object_function(
                caller_index,
                "Change",
                vec![object_reference_value(flag), Value::Int(99)],
            )?,
            Value::Bool(false)
        );
        let invalid = engine.object_snapshot(flag).expect("flag survives");
        assert_eq!((invalid.owner, invalid.controller, invalid.color), (2, 2, 0x0044_5566));
        assert_eq!(invalid.local_vars.get("owner_changes"), Some(&Value::Int(1)));
        assert_eq!(engine.object_snapshot(base).map(|object| object.base), Some(2));

        // Same-owner refresh occurs before the early return, but Controller
        // and callback state stay untouched.
        assert_eq!(
            engine.call_object_function(
                caller_index,
                "RefreshSame",
                vec![object_reference_value(flag)],
            )?,
            Value::Bool(true)
        );
        let same = engine.object_snapshot(flag).expect("flag survives");
        assert_eq!((same.owner, same.controller), (2, 1));
        assert_eq!(same.color, 0x0044_5566);
        assert_eq!(same.local_vars.get("owner_changes"), Some(&Value::Int(1)));
        assert_eq!(same.local_vars.get("shadow_calls"), Some(&Value::Nil));

        // Plain engine spawns run Construction before joining self.objects;
        // SetOwner must still enter the callback synchronously so the rest
        // of Construction observes its local writes.
        let born = engine.spawn_object(SpawnConfig::new("BORN").with_owner(1))?;
        let born = engine.object_snapshot(born).expect("spawn survives");
        assert_eq!((born.owner, born.controller), (2, 2));
        for (name, value) in [
            ("owner_changes", Value::Int(1)),
            ("seen_after", Value::Int(1)),
            ("seen_new", Value::Int(2)),
            ("seen_old", Value::Int(1)),
            ("seen_controller", Value::Int(2)),
        ] {
            assert_eq!(born.local_vars.get(name), Some(&value), "local {name}");
        }
        Ok(())
    }

    #[test]
    fn remove_player_assigns_departing_crew_removal() -> Result<(), EngineError> {
        // C4PlayerList::Remove calls C4Player::RemoveCrewObjects before deleting
        // the player; every crew object receives AssignRemoval(true)
        // (src/C4PlayerList.cpp:219-261; src/C4Player.cpp:1799-1805).
        let mut engine = Engine::new();
        let mut crew_definition = simple_definition("CLNK");
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition)?;
        let crew = engine.spawn_object(
            SpawnConfig::new("CLNK")
                .with_owner(1)
                .with_crew_member(true),
        )?;
        engine.register_player(PlayerConfig::new(1, "Departing"))?;

        let _ = engine.remove_player(1)?;

        assert_eq!(
            engine.object_snapshot(crew).map(|object| object.status),
            Some(ObjectStatus::Deleted)
        );
        Ok(())
    }

    #[test]
    fn remove_player_notifies_owned_objects_before_crew_removal_and_validation(
    ) -> Result<(), EngineError> {
        // NotifyOwnedObjects resolves the exact private native
        // "~OnOwnerRemoved" fallback. That fallback calls SetOwner, whose
        // ordinary OnOwnerChanged hook gives us an observable order ledger.
        const OWNED_SCRIPT: &str = r#"#strict 2
local callback_order, callback_crew, callback_owner, removed_hook;

func OnOwnerRemoved()
{
    removed_hook = true;
}

func OnOwnerChanged(int new_owner, int old_owner)
{
    callback_order = GetGravity();
    callback_crew = GetCrewCount(old_owner);
    callback_owner = GetOwner();
    SetGravity(callback_order + 1);
    return true;
}
"#;
        const SCENARIO_SCRIPT: &str = r#"#strict
func RemovePlayer(int player, int team)
{
    SetGravity(40);
    MissingAfterRemovePlayer();
}
"#;

        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(7, "Ordered", 0).with_player_ids(vec![33, 22, 11])
        ]);
        engine.register_player(
            PlayerConfig::new(1, "Departing")
                .with_player_info_id(11)
                .with_team(Some(7))
                .with_color(Some(RgbColor::new(0xff, 0, 0))),
        )?;
        engine.register_player(
            PlayerConfig::new(2, "Second")
                .with_player_info_id(22)
                .with_team(Some(7))
                .with_color(Some(RgbColor::new(0, 0xff, 0))),
        )?;
        engine.register_player(
            PlayerConfig::new(3, "First by team order")
                .with_player_info_id(33)
                .with_team(Some(7))
                .with_color(Some(RgbColor::new(0, 0, 0xff))),
        )?;

        let mut movable = Definition::from_script("MOVE", "Movable", OWNED_SCRIPT)?;
        movable.set_category(CATEGORY_OBJECT);
        movable.set_color_by_owner(true);
        engine.register_definition(movable)?;
        let mut flag = Definition::from_script("FLAG", "Flag", OWNED_SCRIPT)?;
        flag.set_category(CATEGORY_STATIC_BACK);
        engine.register_definition(flag)?;
        let mut background = Definition::from_script("BACK", "Background", OWNED_SCRIPT)?;
        background.set_category(CATEGORY_STATIC_BACK);
        engine.register_definition(background)?;
        let mut crew_definition = Definition::from_script("CREW", "Crew", OWNED_SCRIPT)?;
        crew_definition.set_category(CATEGORY_OBJECT);
        crew_definition.set_crew_member(true);
        engine.register_definition(crew_definition)?;

        let movable_a = engine.spawn_object(SpawnConfig::new("MOVE").with_owner(1))?;
        let movable_b = engine.spawn_object(SpawnConfig::new("MOVE").with_owner(1))?;
        let inactive = engine.spawn_object(
            SpawnConfig::new("MOVE")
                .with_owner(1)
                .with_status(ObjectStatus::Inactive),
        )?;
        let flag = engine.spawn_object(SpawnConfig::new("FLAG").with_owner(1))?;
        let background = engine.spawn_object(SpawnConfig::new("BACK").with_owner(1))?;
        let active_crew = engine.spawn_object(
            SpawnConfig::new("CREW")
                .with_owner(1)
                .with_crew_member(true),
        )?;
        let inactive_crew = engine.spawn_object(
            SpawnConfig::new("CREW")
                .with_owner(1)
                .with_crew_member(true)
                .with_status(ObjectStatus::Inactive),
        )?;
        engine.load_scenario_script_with_convention(
            "Script.c",
            SCENARIO_SCRIPT,
            true,
        )?;

        let transferable = HashSet::from([movable_a, movable_b, inactive, flag]);
        let exec_order = engine.debug_exec_order();
        let mut expected_callback_order = Vec::new();
        for status in [ObjectStatus::Normal, ObjectStatus::Inactive] {
            expected_callback_order.extend(exec_order.iter().rev().copied().filter(|object| {
                transferable.contains(object)
                    && engine
                        .object_snapshot(*object)
                        .is_some_and(|snapshot| snapshot.status == status)
            }));
        }
        assert_eq!(expected_callback_order.len(), 4);
        assert_eq!(
            expected_callback_order.iter().copied().collect::<HashSet<_>>(),
            transferable
        );

        let _ = engine.remove_player(1)?;

        for (position, object) in expected_callback_order.iter().enumerate() {
            let snapshot = engine.object_snapshot(*object).expect("owned object remains");
            assert_eq!((snapshot.owner, snapshot.controller), (3, 3));
            assert_eq!(
                snapshot.local_vars.get("callback_order"),
                Some(&Value::Int(40 + position as i32)),
                "main-list objects precede inactive-list objects"
            );
            assert_eq!(
                snapshot.local_vars.get("callback_crew"),
                Some(&Value::Int(2)),
                "the departing player and complete Crew list are still live"
            );
            assert_eq!(
                snapshot.local_vars.get("callback_owner"),
                Some(&Value::Int(3)),
                "SetOwner writes owner before OnOwnerChanged"
            );
            assert!(matches!(
                snapshot.local_vars.get("removed_hook"),
                None | Some(Value::Nil)
            ));
        }
        for object in [movable_a, movable_b, inactive] {
            assert_eq!(
                engine.object_snapshot(object).map(|snapshot| snapshot.color),
                Some(0x0000_00ff),
                "ColorByOwner follows the first ordered teammate"
            );
        }
        assert_eq!(
            engine
                .object_snapshot(flag)
                .map(|snapshot| (snapshot.owner, snapshot.controller)),
            Some((3, 3)),
            "FLAG transfers despite its StaticBack category"
        );
        let background = engine
            .object_snapshot(background)
            .expect("StaticBack object remains");
        assert_eq!(
            (background.owner, background.controller),
            (OWNER_NONE, OWNER_NONE),
            "StaticBack skips the fallback and is orphaned by validation"
        );
        assert!(matches!(
            background.local_vars.get("callback_order"),
            None | Some(Value::Nil)
        ));
        for crew in [active_crew, inactive_crew] {
            let crew = engine.object_snapshot(crew).expect("crew record remains");
            assert_eq!(crew.status, ObjectStatus::Deleted);
            assert!(matches!(
                crew.local_vars.get("callback_order"),
                None | Some(Value::Nil)
            ));
        }
        assert_eq!(
            engine.physics().gravity,
            40 + expected_callback_order.len() as i32
        );
        Ok(())
    }

    #[test]
    fn remove_player_uses_inactive_list_reinsertion_order() -> Result<(), EngineError> {
        // StatusDeactivate removes from Game.Objects and inserts into the
        // independent InactiveObjects stMain list. For one definition, the
        // most recently deactivated object is the forward-list first entry.
        const SCRIPT: &str = r#"#strict 2
local callback_order;
func OnOwnerChanged()
{
    callback_order = GetGravity();
    SetGravity(callback_order + 1);
}
"#;
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(7, "Team", 0).with_player_ids(vec![22, 11])
        ]);
        engine.register_player(
            PlayerConfig::new(1, "Departing")
                .with_player_info_id(11)
                .with_team(Some(7)),
        )?;
        engine.register_player(
            PlayerConfig::new(2, "Retained")
                .with_player_info_id(22)
                .with_team(Some(7)),
        )?;
        let mut definition = Definition::from_script("MOVE", "Movable", SCRIPT)?;
        definition.set_category(CATEGORY_OBJECT);
        engine.register_definition(definition)?;
        let a = engine.spawn_object(SpawnConfig::new("MOVE").with_owner(1))?;
        let b = engine.spawn_object(SpawnConfig::new("MOVE").with_owner(1))?;

        // B enters first; A enters second and therefore precedes B in the
        // C++ inactive forward list, regardless of their former main order.
        engine.apply_object_update(b, ObjectUpdate::new().with_status(ObjectStatus::Inactive))?;
        engine.apply_object_update(a, ObjectUpdate::new().with_status(ObjectStatus::Inactive))?;
        let mut physics = engine.physics();
        physics.gravity = 70;
        engine.set_physics(physics);
        let state = engine.capture_state();
        engine.restore_state(&state)?;

        let _ = engine.remove_player(1)?;

        assert_eq!(
            engine
                .object_snapshot(a)
                .and_then(|snapshot| snapshot.local_vars.get("callback_order").cloned()),
            Some(Value::Int(70))
        );
        assert_eq!(
            engine
                .object_snapshot(b)
                .and_then(|snapshot| snapshot.local_vars.get("callback_order").cloned()),
            Some(Value::Int(71))
        );
        Ok(())
    }

    #[test]
    fn remove_player_uses_main_list_reactivation_order() -> Result<(), EngineError> {
        // StatusActivate re-adds the object through Game.Objects.Add(stMain).
        // Reactivating A therefore places it before same-definition B in the
        // forward main list instead of restoring A's stale former position.
        const SCRIPT: &str = r#"#strict 2
local callback_order;
func OnOwnerChanged()
{
    callback_order = GetGravity();
    SetGravity(callback_order + 1);
}
"#;
        let mut engine = Engine::new();
        engine.set_teams(vec![
            TeamInfo::new(7, "Team", 0).with_player_ids(vec![22, 11])
        ]);
        engine.register_player(
            PlayerConfig::new(1, "Departing")
                .with_player_info_id(11)
                .with_team(Some(7)),
        )?;
        engine.register_player(
            PlayerConfig::new(2, "Retained")
                .with_player_info_id(22)
                .with_team(Some(7)),
        )?;
        let mut definition = Definition::from_script("MOVE", "Movable", SCRIPT)?;
        definition.set_category(CATEGORY_OBJECT);
        engine.register_definition(definition)?;
        let a = engine.spawn_object(SpawnConfig::new("MOVE").with_owner(1))?;
        let b = engine.spawn_object(SpawnConfig::new("MOVE").with_owner(1))?;
        engine.apply_object_update(a, ObjectUpdate::new().with_status(ObjectStatus::Inactive))?;
        engine.apply_object_update(a, ObjectUpdate::new().with_status(ObjectStatus::Normal))?;
        let mut physics = engine.physics();
        physics.gravity = 80;
        engine.set_physics(physics);

        let _ = engine.remove_player(1)?;

        assert_eq!(
            engine
                .object_snapshot(a)
                .and_then(|snapshot| snapshot.local_vars.get("callback_order").cloned()),
            Some(Value::Int(80))
        );
        assert_eq!(
            engine
                .object_snapshot(b)
                .and_then(|snapshot| snapshot.local_vars.get("callback_order").cloned()),
            Some(Value::Int(81))
        );
        Ok(())
    }

    #[test]
    fn remove_player_uses_last_eligible_non_hostile_fallback_owner(
    ) -> Result<(), EngineError> {
        // FnOnOwnerRemoved's teamless fallback scans the whole player list
        // without a break. Eliminated/surrendered and hostile players skip;
        // the last remaining candidate wins (C4Script.cpp:5863-5872).
        let mut engine = Engine::new();
        let mut definition = simple_definition("MOVE");
        definition.set_category(CATEGORY_OBJECT);
        engine.register_definition(definition)?;
        engine.register_player(PlayerConfig::new(1, "Departing"))?;
        engine.register_player(PlayerConfig::new(2, "Eligible first"))?;
        engine.register_player(PlayerConfig::new(3, "Eligible last"))?;
        engine.register_player(
            PlayerConfig::new(4, "Eliminated").with_status(PlayerStatus::Eliminated),
        )?;
        engine.register_player(PlayerConfig::new(5, "Hostile"))?;
        engine.register_player(
            PlayerConfig::new(6, "Surrendered").with_status(PlayerStatus::Surrendered),
        )?;
        engine.set_hostility(5, 1, true)?;
        let object = engine.spawn_object(SpawnConfig::new("MOVE").with_owner(1))?;

        let _ = engine.remove_player(1)?;

        let object = engine.object_snapshot(object).expect("object remains");
        assert_eq!((object.owner, object.controller), (3, 3));
        Ok(())
    }

    #[test]
    fn remove_player_clears_invalid_static_back_owner_and_controller() -> Result<(), EngineError> {
        // StaticBack objects skip the OnOwnerRemoved fallback, but the final
        // C4ObjectList::ValidateOwners pass still clears their now-invalid
        // Owner and Controller (src/C4Script.cpp:5837-5841;
        // src/C4PlayerList.cpp:260-264; src/C4Object.cpp:3130-3138).
        let mut engine = Engine::new();
        engine.register_definition(simple_definition("BACK"))?;
        let object = engine.spawn_object(
            SpawnConfig::new("BACK")
                .with_owner(1)
                .with_controller(1),
        )?;
        engine.register_player(PlayerConfig::new(1, "Departing"))?;

        let _ = engine.remove_player(1)?;

        let object = engine.object_snapshot(object).expect("StaticBack remains");
        assert_eq!((object.owner, object.controller), (OWNER_NONE, OWNER_NONE));
        Ok(())
    }

    #[test]
    fn skipped_restored_player_is_orphaned_without_removing_objects() -> Result<(), EngineError> {
        let mut engine = Engine::new();
        engine.register_definition(simple_definition("BACK"))?;
        let normal = engine.spawn_object(
            SpawnConfig::new("BACK")
                .with_owner(1)
                .with_controller(1)
                .with_color(0xff12_3456),
        )?;
        let inactive = engine.spawn_object(
            SpawnConfig::new("BACK")
                .with_owner(1)
                .with_controller(1)
                .with_status(ObjectStatus::Inactive)
                .with_color(0xff65_4321),
        )?;
        engine.register_player(PlayerConfig::new(1, "Skipped"))?;
        engine.register_player(PlayerConfig::new(2, "Retained"))?;
        for object in [normal, inactive] {
            let index = engine.find_object_index(object).expect("object exists");
            engine.objects[index].state.base = 1;
        }

        engine.retain_restored_players([2]);

        assert!(engine.player(1).is_none());
        assert!(engine.player(2).is_some());
        for (object, status, color) in [
            (normal, ObjectStatus::Normal, 0xff12_3456),
            (inactive, ObjectStatus::Inactive, 0xff65_4321),
        ] {
            let object = engine.object_snapshot(object).expect("saved object remains");
            assert_eq!(object.status, status);
            assert_eq!(object.color, color);
            assert_eq!(
                (object.owner, object.base, object.controller),
                (OWNER_NONE, OWNER_NONE, OWNER_NONE)
            );
        }
        Ok(())
    }

    #[test]
    fn script_game_over_triggers_on_game_over() -> Result<(), EngineError> {
        // DoGameOver broadcasts OnGameOver before survivor winner flags
        // (C4Game.cpp:3659-3670); C4Game::Execute closes DoSyncCheck before
        // Evaluate (C4Game.cpp:845-854). Player evaluation then applies the
        // cooperative AverageValueGain and winner bonus before RoundResults
        // evaluates goals and records Game.Time (C4Player.cpp:930-970;
        // C4RoundResults.cpp:280-313).
        const SCRIPT: &str = r#"#strict 3
        global func Initialize(state, random) { return nil; }
        global func Step(state, frame, random)
        {
            if (frame == 1)
            {
                GameOver();
            }
            return nil;
        }
        global func OnGameOver(state)
        {
            return { physics = { gravity = 42 } };
        }
        "#;

        let mut engine = Engine::with_seed(7);
        let mut crew_definition = simple_definition("Crew");
        crew_definition.set_value(165);
        engine.register_definition(crew_definition)?;
        let mut goal_definition = Definition::from_script(
            "GOAL",
            "Goal",
            r#"
            local calls;
            func IsFulfilled()
            {
                calls++;
                return true;
            }
            "#,
        )?;
        goal_definition.set_category(1 << 5); // C4D_Goal
        engine.register_definition(goal_definition)?;
        engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_alive(true)
                .with_owner(0)
                .with_crew_member(true)
                .with_position(Vector2::new(50, 50)),
        )?;
        let goal = engine.spawn_object(SpawnConfig::new("GOAL"))?;

        engine.install_scenario_script("Scenario", SCRIPT)?;
        engine.register_player(
            PlayerConfig::new(0, "Player")
                .with_player_info_id(41)
                .with_score(250)
                .with_rounds(11, 7, 4)
                .with_total_playing_time(1_234)
                .with_initial_value(100),
        )?;
        engine.replace_player_info_league_progress_data([(
            41,
            Some(vec![b'P', 0xff]),
        )]);
        engine.game_time = 19;
        engine.round_results.players = vec![
            RoundResultsPlayerState {
                player_info_id: 99,
                custom_evaluation_strings: "Other row".to_string(),
                ..RoundResultsPlayerState::default()
            },
            RoundResultsPlayerState {
                player_info_id: 41,
                custom_evaluation_strings: "Keep this".to_string(),
                ..RoundResultsPlayerState::default()
            },
        ];

        let first = engine.tick()?;
        assert!(first.game_over);
        assert_eq!(engine.physics().gravity, 42);
        let player = first
            .players
            .iter()
            .find(|player| player.id == 0)
            .expect("evaluated player");
        assert!(player.won, "post-OnGameOver survivor is a winner");
        assert!(player.evaluated);
        assert_eq!(player.score, 415, "65 gain + 100 winner bonus");
        assert_eq!((player.rounds, player.rounds_won, player.rounds_lost), (12, 8, 4));
        assert_eq!(player.total_playing_time, 1_253);
        let last_round = &player
            .player_info_core
            .as_ref()
            .expect("evaluation creates the inherited player core")
            .last_round;
        assert_eq!(last_round.title, "Default Title");
        assert!(last_round.date > 0);
        assert_eq!(last_round.duration, 19);
        assert_eq!(last_round.won, 1);
        assert_eq!(last_round.score, 65);
        assert_eq!(last_round.bonus, 100);
        assert_eq!(last_round.final_score, 165);
        assert_eq!(last_round.total_score, 415);
        assert_eq!(last_round.level, 0);
        assert_eq!(
            first.round_results.goals,
            vec![DefinitionId::from("GOAL")]
        );
        assert_eq!(
            first.round_results.fulfilled_goals,
            vec![DefinitionId::from("GOAL")]
        );
        assert_eq!(first.round_results.playing_time_seconds, 19);
        assert_eq!(
            first.round_results.players,
            vec![
                RoundResultsPlayerState {
                    player_info_id: 99,
                    custom_evaluation_strings: "Other row".to_string(),
                    ..RoundResultsPlayerState::default()
                },
                RoundResultsPlayerState {
                    player_info_id: 41,
                    total_playing_time: 1_253,
                    score_old: 250,
                    score_new: Some(415),
                    league_progress_data: Some(vec![b'P', 0xff]),
                    league_performance: 0,
                    custom_evaluation_strings: "Keep this".to_string(),
                    ..RoundResultsPlayerState::default()
                },
            ]
        );

        engine.replace_player_info_league_progress_data([(
            41,
            Some(b"changed".to_vec()),
        )]);
        let second = engine.tick()?;
        assert_eq!(second.round_results, first.round_results);
        let player = second
            .players
            .iter()
            .find(|player| player.id == 0)
            .expect("still-evaluated player");
        assert_eq!(player.score, 415, "second tick must not score again");
        assert_eq!((player.rounds, player.rounds_won, player.rounds_lost), (12, 8, 4));
        assert_eq!(player.total_playing_time, 1_253);
        assert_eq!(
            second
                .object(goal)
                .and_then(|goal| goal.local_vars.get("calls")),
            Some(&Value::Int(1)),
            "goal callback runs once"
        );

        Ok(())
    }

    #[test]
    fn round_goal_evaluation_recomputes_master_order_and_uses_rivalry_callback(
    ) -> Result<(), EngineError> {
        // C4RoundResults::EvaluateGoals asks GetListID for the cnt-th unique
        // goal on every iteration, then Find re-resolves the first live
        // instance (C4RoundResults.cpp:280-304; C4ObjectList.cpp:58-78,
        // 271-281). RVLR switches the callback to exact
        // IsFulfilledforPlr(first-local-player).
        const GOAL_SCRIPT: &str = r#"
        local generic_calls, per_player_calls, seen_player;
        func IsFulfilled()
        {
            generic_calls++;
            return false;
        }
        func IsFulfilledforPlr(player)
        {
            per_player_calls++;
            seen_player = player;
            return true;
        }
        "#;
        const REMOVING_GOAL_SCRIPT: &str = r#"
        local generic_calls, per_player_calls, seen_player;
        func IsFulfilled()
        {
            generic_calls++;
            return false;
        }
        func IsFulfilledforPlr(player)
        {
            per_player_calls++;
            seen_player = player;
            RemoveObject();
            return true;
        }
        "#;

        let mut engine = Engine::new();
        for (id, script) in [
            ("GOLB", REMOVING_GOAL_SCRIPT),
            ("GOLA", GOAL_SCRIPT),
            ("GOLC", GOAL_SCRIPT),
        ] {
            let mut definition = Definition::from_script(id, id, script)?;
            definition.set_category(CATEGORY_GOAL);
            engine.register_definition(definition)?;
        }
        engine.register_definition(simple_definition("RVLR"))?;

        let goal_b = engine.spawn_object(SpawnConfig::new("GOLB"))?;
        let goal_a = engine.spawn_object(SpawnConfig::new("GOLA"))?;
        let goal_c_first = engine.spawn_object(SpawnConfig::new("GOLC"))?;
        let goal_c_second = engine.spawn_object(SpawnConfig::new("GOLC"))?;
        let rivalry = engine.spawn_object(SpawnConfig::new("RVLR"))?;
        // exec_list is the reverse of C++ master-list order. B removes
        // itself while cnt=0; recomputing cnt=1 over [A,C] selects C, not A.
        engine.exec_list = vec![
            rivalry,
            goal_c_second,
            goal_c_first,
            goal_a,
            goal_b,
        ];
        engine.register_player(PlayerConfig::new(2, "Remote"))?;
        engine.register_player(PlayerConfig::new(7, "Local"))?;
        engine.set_local_players([7]);

        let (goals, fulfilled) = engine.evaluate_round_goals()?;
        assert_eq!(
            goals,
            vec![DefinitionId::from("GOLB"), DefinitionId::from("GOLC")]
        );
        assert_eq!(fulfilled, goals);

        let local = |id, name: &str| {
            engine
                .object_snapshot(id)
                .and_then(|object| object.local_vars.get(name).cloned())
        };
        assert_eq!(local(goal_b, "per_player_calls"), Some(Value::Int(1)));
        assert_eq!(local(goal_b, "seen_player"), Some(Value::Int(7)));
        assert_eq!(local(goal_b, "generic_calls"), Some(Value::Nil));
        assert_eq!(
            local(goal_a, "per_player_calls"),
            None,
            "A is skipped"
        );
        assert_eq!(
            local(goal_c_first, "per_player_calls"),
            Some(Value::Int(1)),
            "first live C instance handles the callback"
        );
        assert_eq!(local(goal_c_first, "seen_player"), Some(Value::Int(7)));
        assert_eq!(local(goal_c_first, "generic_calls"), Some(Value::Nil));
        assert_eq!(
            local(goal_c_second, "per_player_calls"),
            None,
            "duplicate goal instance is not called"
        );
        Ok(())
    }

    #[test]
    fn scenario_set_next_mission_survives_a_later_initialize_error() -> Result<(), EngineError> {
        // C4AulExec aborts only the continuation on error; host mutations
        // before the failing call remain live (C4AulExec.cpp:1318-1342).
        // FnSetNextMission writes the three C4Game fields immediately
        // (C4Script.cpp:6053-6081).
        let mut engine = Engine::with_seed(0);
        engine.load_scenario_script_with_convention(
            "Script.c",
            r#"
            #strict
            func Initialize() {
                SetNextMission("Tutorial.c4f\\Tutorial01.c4s", "Repeat", "Play again");
                MissingAfterNextMission();
            }
            "#,
            true,
        )?;

        engine.initialize_scenario_script()?;

        assert_eq!(
            engine.next_mission(),
            &NextMissionState {
                path: "Tutorial.c4f\\Tutorial01.c4s".to_string(),
                text: "Repeat".to_string(),
                description: "Play again".to_string(),
            }
        );
        Ok(())
    }

    #[test]
    fn is_network_reads_the_active_engine_session_like_cpp() -> Result<(), EngineError> {
        // FnIsNetwork returns Game.Parameters.IsNetworkGame
        // (C4Script.cpp:3554), which normal parameter setup copies from the
        // active Game.NetworkActive session (C4GameParameters.cpp:429-434).
        const SCRIPT: &str = r#"
            #strict
            func Initialize() {
                if (IsNetwork()) SetGravity(77);
                else SetGravity(23);
            }
        "#;

        let mut local = Engine::with_seed(0);
        local.set_network_game(false);
        local.install_scenario_script_with_convention("Script.c", SCRIPT, true)?;
        assert_eq!(local.physics().gravity, 23);

        let mut network = Engine::with_seed(0);
        network.set_network_game(true);
        network.install_scenario_script_with_convention("Script.c", SCRIPT, true)?;
        assert_eq!(network.physics().gravity, 77);
        Ok(())
    }

    #[test]
    fn next_mission_clear_and_save_restore_match_cpp() -> Result<(), EngineError> {
        // An empty mission clears only NextMission/NextMissionText
        // (C4Script.cpp:6055-6061); all three fields persist in exact saves
        // (C4Game.cpp:1963-1965).
        let mut engine = Engine::with_seed(0);
        engine.install_scenario_script_with_convention(
            "Script.c",
            r#"
            #strict
            func Initialize() {
                SetNextMission("Tutorial02", "Next", "Keep this description");
                SetNextMission(0);
            }
            "#,
            true,
        )?;
        assert_eq!(
            engine.next_mission(),
            &NextMissionState {
                path: String::new(),
                text: String::new(),
                description: "Keep this description".to_string(),
            }
        );

        let encoded = engine
            .capture_state()
            .to_json_string()
            .expect("next mission state serializes");
        let state =
            EngineState::from_json_str(&encoded).expect("next mission state deserializes");
        let mut restored = Engine::with_seed(1);
        restored.restore_state(&state)?;
        assert_eq!(restored.next_mission(), engine.next_mission());

        let mut legacy: serde_json::Value =
            serde_json::from_str(&encoded).expect("state JSON parses");
        legacy
            .as_object_mut()
            .expect("engine state is an object")
            .remove("next_mission");
        let legacy: EngineState =
            serde_json::from_value(legacy).expect("legacy state without next mission parses");
        assert_eq!(legacy.next_mission, NextMissionState::default());
        Ok(())
    }

    #[test]
    fn legacy_create_object_objective_triggers_game_over() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(0);

        let mut crew_def = simple_definition("Crew");
        crew_def.set_crew_member(true);
        engine.register_definition(crew_def)?;
        engine.register_definition(simple_definition("FLAG"))?;

        let objectives = ScenarioObjectives {
            create_objects: vec![CreateObjectObjective {
                definition: "FLAG".into(),
                count: 1,
            }],
            ..ScenarioObjectives::default()
        };
        engine.configure_objectives(objectives);

        engine.register_player(PlayerConfig::new(0, "Player"))?;

        engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_alive(true)
                .with_owner(0)
                .with_crew_member(true)
                .with_position(Vector2::new(10, 10)),
        )?;
        engine.spawn_object(
            SpawnConfig::new("FLAG")
                .with_owner(0)
                .with_construction(FULL_CON),
        )?;

        let mut triggered = false;
        for _ in 0..40 {
            let snapshot = engine.tick()?;
            if snapshot.game_over {
                triggered = true;
                break;
            }
        }

        assert!(triggered, "expected game over once required object exists");
        Ok(())
    }

    #[test]
    fn legacy_clear_object_objective_triggers_after_removal() -> Result<(), EngineError> {
        let mut engine = Engine::with_seed(0);

        let mut crew_def = simple_definition("Crew");
        crew_def.set_crew_member(true);
        engine.register_definition(crew_def)?;
        engine.register_definition(simple_definition("ROCK"))?;

        let objectives = ScenarioObjectives {
            clear_objects: vec![ClearObjectObjective {
                definition: "ROCK".into(),
                count: 0,
            }],
            ..ScenarioObjectives::default()
        };
        engine.configure_objectives(objectives);

        engine.register_player(PlayerConfig::new(0, "Player"))?;

        engine.spawn_object(
            SpawnConfig::new("Crew")
                .with_alive(true)
                .with_owner(0)
                .with_crew_member(true)
                .with_position(Vector2::new(15, 20)),
        )?;
        let rock_id =
            engine.spawn_object(SpawnConfig::new("ROCK").with_alive(true).with_owner(0))?;

        for _ in 0..5 {
            let snapshot = engine.tick()?;
            assert!(
                !snapshot.game_over,
                "game over should not trigger before removal"
            );
        }

        engine.apply_object_update(
            rock_id,
            ObjectUpdate::new().with_status(ObjectStatus::Deleted),
        )?;

        // Process removal and allow periodic polling to run.
        engine.tick_without_snapshot()?;

        let mut triggered = false;
        for _ in 0..40 {
            let snapshot = engine.tick()?;
            if snapshot.game_over {
                triggered = true;
                break;
            }
        }

        assert!(
            triggered,
            "expected game over once disallowed objects are cleared"
        );
        Ok(())
    }

    fn simple_definition(id: &str) -> Definition {
        Definition::from_script(
            id,
            id,
            r#"
            global func Initialize(state, random) { return 0; }
            global func Step(state, frame, random) { return 0; }
            "#,
        )
        .expect("script compiles")
    }

    #[test]
    fn shape_hosts_apply_live_vertices_and_definition_bottoms_end_to_end() {
        // FnAddVertex writes the live C4Object::Shape and forwards
        // C4Shape::AddVertex's false-at-30 result without setting
        // fOwnVertices (C4Script.cpp:1274-1278; C4Shape.cpp:26-32).
        // FnGetDefBottom reads y + Def->Shape.y + Def->Shape.Hgt, not that
        // live shape (C4Script.cpp:4445-4449).
        let script = r#"#strict 2
local add_self, add_foreign, add_overflow;
local self_count, self_bottom, foreign_bottom;
public func Probe(object target) {
    add_self = AddVertex(17, -9);
    add_foreign = AddVertex(500, 900, target);
    add_overflow = AddVertex(501, 901, target);
    self_count = GetVertexNum();
    self_bottom = GetDefBottom();
    foreign_bottom = GetDefBottom(target);
    return 1;
}
"#;
        let mut caller =
            Definition::from_script("CALL", "Caller", script).expect("caller compiles");
        caller.set_shape_rect(Some(DefinitionRect::new(-2, -6, 4, 12)));
        caller.set_shape_vertices(vec![ObjectVertex::new(0, 0)]);
        let mut target = simple_definition("TARG");
        target.set_shape_rect(Some(DefinitionRect::new(1, -4, 4, 9)));
        target.set_shape_vertices(
            (0..29)
                .map(|index| ObjectVertex::new(index, index))
                .collect(),
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(caller)
            .expect("caller registers");
        engine
            .register_definition(target)
            .expect("target registers");
        let caller_id = engine
            .spawn_object(SpawnConfig::new("CALL").with_position(Vector2::new(100, 200)))
            .expect("caller spawns");
        let target_id = engine
            .spawn_object(SpawnConfig::new("TARG").with_position(Vector2::new(300, 400)))
            .expect("target spawns");
        let caller_index = engine.find_object_index(caller_id).expect("caller exists");
        let self_bottom = engine.objects[caller_index]
            .state
            .position
            .y
            .wrapping_add(-6)
            .wrapping_add(12);
        let target_index = engine.find_object_index(target_id).expect("target exists");
        let foreign_bottom = engine.objects[target_index]
            .state
            .position
            .y
            .wrapping_add(-4)
            .wrapping_add(9);

        assert_eq!(
            engine
                .call_object_function(
                    caller_index,
                    "Probe",
                    vec![Value::Object(target_id.as_u64())],
                )
                .expect("shape hosts run"),
            Value::Int(1)
        );
        let caller_index = engine.find_object_index(caller_id).expect("caller remains");
        let locals = &engine.objects[caller_index].state.local_vars;
        assert_eq!(locals.get("add_self"), Some(&Value::Bool(true)));
        assert_eq!(locals.get("add_foreign"), Some(&Value::Bool(true)));
        assert_eq!(locals.get("add_overflow"), Some(&Value::Bool(false)));
        assert_eq!(locals.get("self_count"), Some(&Value::Int(2)));
        assert_eq!(locals.get("self_bottom"), Some(&Value::Int(self_bottom)));
        assert_eq!(
            locals.get("foreign_bottom"),
            Some(&Value::Int(foreign_bottom))
        );
        assert_eq!(engine.objects[caller_index].state.vertices.len(), 2);
        assert!(engine.objects[caller_index].own_shape_vertices.is_none());
        let target_index = engine.find_object_index(target_id).expect("target remains");
        assert_eq!(engine.objects[target_index].state.vertices.len(), 30);
        assert_eq!(
            engine.objects[target_index].state.vertices[29],
            ObjectVertex::new(500, 900)
        );
        assert!(engine.objects[target_index].own_shape_vertices.is_none());

        // A later UpdateShape restores definition vertices because AddVertex
        // did not switch on C4Object::fOwnVertices (C4Object.cpp:322-329).
        engine
            .apply_object_update(
                target_id,
                ObjectUpdate::new().with_construction(FULL_CON - 1),
            )
            .expect("construction refresh succeeds");
        let target_index = engine
            .find_object_index(target_id)
            .expect("target remains after refresh");
        assert_eq!(engine.objects[target_index].state.vertices.len(), 29);
        assert!(engine.objects[target_index].own_shape_vertices.is_none());
    }

    #[test]
    fn disabled_flight_flats_on_low_speed_bottom_contact() {
        // C4Object::ContactAction tries ObjectActionFlat when the current
        // FLIGHT action is ObjectDisabled even without OCF_HitSpeed4
        // (C4Object.cpp:4336-4340; C4ObjectCom.cpp:96-101).
        let mut definition = simple_definition("FLAT");
        definition.configure_actions(
            Some("Jump".to_string()),
            HashMap::from([
                (
                    "Jump".to_string(),
                    ActionSpec::default()
                        .with_procedure("FLIGHT")
                        .with_disabled(true),
                ),
                ("FlatUp".to_string(), ActionSpec::default()),
            ]),
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("FLAT")
                    .with_action(ActionState::new("Jump"))
                    .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                    .with_loaded(true),
            )
            .expect("object spawns");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].state.ocf & ocf::HIT_SPEED4, 0);
        let definition_id = engine.objects[idx].definition_id.clone();

        engine
            .exec_contact_action(idx, CNAT_BOTTOM, &definition_id)
            .expect("contact action applies");

        let object = &engine.objects[idx];
        assert_eq!(object.state.action.name, "FlatUp");
        assert_eq!(object.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(object.state.velocity, Vector2::ZERO);
    }

    #[test]
    fn disabled_tumble_reenters_on_top_and_side_contacts() {
        // C4Object::ContactAction's top/left/right FLIGHT arms use
        // `(OCF_HitSpeed3 || fDisabled)`: a low-speed disabled Tumble must
        // re-enter Tumble instead of taking the available Hangle/Scale paths
        // (C4Object.cpp:4400-4500). The later flight-stuck tail erases the
        // wall helper's transient +/-FIXED100(150) velocity.
        for (contact, expected_direction, expected_position) in [
            (CNAT_TOP, Direction::Right, Vector2::new(10, 11)),
            (CNAT_LEFT, Direction::Left, Vector2::new(11, 11)),
            (CNAT_RIGHT, Direction::Right, Vector2::new(9, 11)),
        ] {
            let mut definition = Definition::from_script(
                "TMBL",
                "Tumble callback probe",
                r#"#strict
local callback_order;
protected func TumbleStart() { callback_order = callback_order * 10 + 1; return(1); }
protected func TumbleAbort(int old_phase) { callback_order = callback_order * 10 + 2; return(1); }
protected func ScaleStart() { callback_order = callback_order * 10 + 3; return(1); }
protected func HangleStart() { callback_order = callback_order * 10 + 4; return(1); }
"#,
            )
            .expect("tumble callback probe compiles");
            definition.set_c4_callback_convention(true);
            definition.configure_actions(
                Some("Tumble".to_string()),
                HashMap::from([
                    (
                        "Tumble".to_string(),
                        ActionSpec::default()
                            .with_procedure("FLIGHT")
                            .with_disabled(true)
                            .with_start_call("TumbleStart")
                            .with_abort_call("TumbleAbort"),
                    ),
                    (
                        "Scale".to_string(),
                        ActionSpec::default()
                            .with_procedure("SCALE")
                            .with_start_call("ScaleStart"),
                    ),
                    (
                        "Hangle".to_string(),
                        ActionSpec::default()
                            .with_procedure("HANGLE")
                            .with_start_call("HangleStart"),
                    ),
                ]),
            );
            definition.set_physical(PhysicalInfo {
                can_scale: 1,
                can_hangle: 1,
                ..PhysicalInfo::default()
            });

            let mut engine = Engine::with_seed(0);
            engine
                .register_definition(definition)
                .expect("tumble definition registers");
            let id = engine
                .spawn_object(
                    SpawnConfig::new("TMBL")
                        .with_position(Vector2::new(10, 10))
                        .with_fixed_position(FixedVec2::from_ints(10, 10))
                        .with_action(ActionState::new("Tumble"))
                        .with_direction(Direction::Right)
                        .with_local_vars(HashMap::from([(
                            "callback_order".to_string(),
                            Value::Int(0),
                        )]))
                        .with_fixed_velocity(FixedVec2::new(
                            C4Fixed::from_raw(32_768),
                            C4Fixed::from_raw(6_553),
                        ))
                        .with_loaded(true),
                )
                .expect("tumbling object spawns");
            let idx = engine.find_object_index(id).expect("tumbling object exists");
            assert_eq!(engine.objects[idx].state.ocf & ocf::HIT_SPEED3, 0);
            let definition_id = engine.objects[idx].definition_id.clone();

            engine
                .exec_contact_action(idx, contact, &definition_id)
                .expect("contact action applies");

            let object = &engine.objects[idx];
            assert_eq!(object.state.action.name, "Tumble");
            assert_eq!(
                object.state.local_vars.get("callback_order"),
                Some(&Value::Int(12)),
                "Tumble Start/Abort must run without a Scale/Hangle StartCall"
            );
            assert_eq!(object.state.direction, expected_direction);
            assert_eq!(object.state.position, expected_position);
            assert_eq!(
                object.fixed_position,
                FixedVec2::from_ints(expected_position.x, expected_position.y,)
            );
            assert_eq!(object.fixed_velocity, FixedVec2::ZERO);
            assert_eq!(object.state.velocity, Vector2::ZERO);
        }
    }

    #[test]
    fn flight_slide_free_uses_live_position_for_each_contact() {
        // C4Object::ContactAction applies Right, Left, then Top corrections
        // through ForcePosition using the live x/y after every move
        // (C4Object.cpp:4543-4567). GoldRush MONS #582 reaches all three
        // contacts together at frame 410.
        let mut definition = simple_definition("MONS");
        definition.configure_actions(
            Some("Jump".to_string()),
            HashMap::from([(
                "Jump".to_string(),
                ActionSpec::default().with_procedure("FLIGHT"),
            )]),
        );

        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(definition)
            .expect("monster definition registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("MONS")
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_action(ActionState::new("Jump"))
                    .with_loaded(true),
            )
            .expect("monster spawns");
        let idx = engine.find_object_index(id).expect("monster exists");
        let definition_id = engine.objects[idx].definition_id.clone();

        engine.exec_contact_action(
            idx,
            CNAT_RIGHT | CNAT_LEFT | CNAT_TOP,
            &definition_id,
        )
        .expect("contact action applies");

        let object = &engine.objects[idx];
        assert_eq!(object.state.position, Vector2::new(10, 13));
        assert_eq!(object.fixed_position, FixedVec2::from_ints(10, 13));
        assert_eq!(object.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(object.state.velocity, Vector2::ZERO);
    }

    #[test]
    fn force_position_preserves_velocity_like_cpp() {
        // C4Object::ForcePosition only resynchronizes fix_x/fix_y and updates
        // spatial bookkeeping; callers zero xdir/ydir explicitly when needed
        // (C4Movement.cpp:531-539).
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("Rock"))
            .expect("rock registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Rock")
                    .with_position(Vector2::new(10, 10))
                    .with_fixed_position(FixedVec2::from_ints(10, 10))
                    .with_loaded(true),
            )
            .expect("rock spawns");
        let idx = engine.find_object_index(id).expect("rock exists");
        let velocity = FixedVec2::new(itofix(2), itofix(-3));
        engine.objects[idx].set_fixed_velocity(velocity);

        engine.force_object_position(idx, Vector2::new(12, 14));

        let object = &engine.objects[idx];
        assert_eq!(object.state.position, Vector2::new(12, 14));
        assert_eq!(object.fixed_position, FixedVec2::from_ints(12, 14));
        assert_eq!(object.fixed_velocity, velocity);
        assert_eq!(object.state.velocity, Vector2::new(2, -3));
    }

    #[test]
    fn loaded_rotation_without_fix_r_stays_independent_until_sync_clearance() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("Rock"))
            .expect("rock registers");
        let id = engine
            .spawn_object(
                SpawnConfig::new("Rock")
                    .with_rotation(-9)
                    .with_loaded(true),
            )
            .expect("loaded object spawns");

        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].state.rotation, -9);
        assert_eq!(engine.objects[idx].fixed_rotation, C4Fixed::ZERO);

        engine
            .game_start_synchronize()
            .expect("game-start synchronization succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].state.rotation, -9);
        assert_eq!(engine.objects[idx].fixed_rotation, itofix(-9));
    }

    // Mirrors C4Object::Init (C4Object.cpp:183-185): a freshly created
    // object is Mobile only when it spawns with a nonzero dir, and only
    // when Category != C4D_StaticBack — an EQUALITY test on the whole
    // category value, not a bitmask. Loaded objects bypass Init and keep
    // the serialized flag (default false, C4Object.cpp:2772).
    #[test]
    fn initial_mobility_follows_init_velocity_rule_like_cpp() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("Rock"))
            .expect("rock registers");

        let resting = engine
            .spawn_object(SpawnConfig::new("Rock").with_category(CATEGORY_OBJECT))
            .expect("resting spawns");
        let moving = engine
            .spawn_object(
                SpawnConfig::new("Rock")
                    .with_velocity(Vector2::new(1, 0))
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("moving spawns");
        let static_back = engine
            .spawn_object(
                SpawnConfig::new("Rock")
                    .with_velocity(Vector2::new(1, 0))
                    .with_category(CATEGORY_STATIC_BACK),
            )
            .expect("static spawns");
        let loaded = engine
            .spawn_object(
                SpawnConfig::new("Rock")
                    .with_velocity(Vector2::new(1, 0))
                    .with_category(CATEGORY_OBJECT)
                    .with_loaded(true),
            )
            .expect("loaded spawns");

        let mobile_of = |engine: &Engine, id| {
            engine
                .find_object_index(id)
                .map(|idx| engine.objects[idx].state.mobile)
                .expect("object exists")
        };
        assert!(
            !mobile_of(&engine, resting),
            "zero-dir spawn stays immobile (C4Object.cpp:184)"
        );
        assert!(
            mobile_of(&engine, moving),
            "nonzero xdir mobilizes a fresh spawn (C4Object.cpp:185)"
        );
        assert!(
            !mobile_of(&engine, static_back),
            "Category == C4D_StaticBack skips Init mobilization (C4Object.cpp:183)"
        );
        assert!(
            !mobile_of(&engine, loaded),
            "loaded objects keep the serialized default false (C4Object.cpp:2772)"
        );
    }

    // Mirrors C4Movement.cpp:566-587 + C4Object.cpp:4708-4712: a resting
    // (non-Mobile) object is fully frozen — no idle gravity, no movement —
    // until the Tick10 gravity mobilization re-mobilizes it with zeroed
    // dirs (the global tick counters advance BEFORE objects execute,
    // C4Game.cpp:1888, so the pulse fires on frames 10, 20, ...). Gravity
    // then applies from the NEXT frame's ExecAction because mobilization
    // runs in ExecMovement after that frame's ExecAction already saw
    // Mobile == false.
    #[test]
    fn exec_action_energy_usage_parses_and_gates_before_action_work() {
        let temp = tempfile::tempdir().expect("tempdir");
        let def_dir = temp.path().join("Powered.ocd");
        std::fs::create_dir(&def_dir).expect("create definition directory");
        std::fs::write(
            def_dir.join("DefCore.txt"),
            b"[DefCore]\nid=PWRD\nName=Powered\nCategory=C4D_Object\n",
        )
        .expect("write DefCore");
        std::fs::write(def_dir.join("Script.c"), b"#strict\n").expect("write Script.c");
        std::fs::write(
            def_dir.join("ActMap.txt"),
            br#"[Action]
Name=Work
Procedure=WALK
Length=20
Delay=1
Step=1
NextAction=Hold
InLiquidAction=Wet
EnergyUsage=10

[Action]
Name=Wet
Procedure=SWIM
Length=1
NextAction=Hold

[Action]
Name=Refund
EnergyUsage=-3

[Action]
Name=ConnectWork
Procedure=CONNECT
EnergyUsage=10
"#,
        )
        .expect("write ActMap.txt");
        let group = clonk_resources::Group::open(&def_dir).expect("open definition group");
        let resource = ResourceDefinitionData::load(&group).expect("load definition resource");
        let mut definition =
            Definition::from_resource(&resource).expect("compile resource definition");
        assert_eq!(
            definition
                .action_library()
                .energy_usage_for_action("Work"),
            10,
            "ActMap EnergyUsage reaches the runtime action library"
        );
        assert_eq!(
            definition
                .action_library()
                .energy_usage_for_action("Refund"),
            -3,
            "EnergyUsage remains signed like C4ActionDef::EnergyUsage"
        );
        // Definition::from_resource's older conversion seam does not yet
        // carry InLiquidAction. Add that pre-existing field synthetically so
        // this regression can pin the EnergyUsage ordering without expanding
        // into the unrelated conversion gap.
        let mut specs = definition.action_library().specs().clone();
        let work = specs
            .remove("Work")
            .expect("parsed Work action")
            .with_in_liquid_action("Wet");
        specs.insert("Work".to_string(), work);
        definition.configure_actions(None, specs);
        definition.set_line(1);

        let mut engine = Engine::with_seed(11);
        engine.set_structures_need_energy(true);
        engine.set_physics(PhysicsSettings::new(100, 200, -200));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let work_action = || {
            let mut action = ActionState::new("Work");
            action.time = 4;
            action.phase = 2;
            action
        };
        let stalled = engine
            .spawn_object(
                SpawnConfig::new("PWRD")
                    .with_position(Vector2::new(20, 20))
                    .with_fixed_position(FixedVec2::from_ints(20, 20))
                    .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                    .with_action(work_action())
                    .with_command_direction(CommandDirection::Right)
                    .with_energy(5)
                    .with_in_liquid(true)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("underpowered object spawns");
        let powered = engine
            .spawn_object(
                SpawnConfig::new("PWRD")
                    .with_position(Vector2::new(40, 20))
                    .with_fixed_position(FixedVec2::from_ints(40, 20))
                    .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                    .with_action(work_action())
                    .with_command_direction(CommandDirection::Right)
                    .with_energy(10)
                    .with_need_energy(true)
                    .with_alive(true)
                    .with_mobile(true)
                    .with_loaded(true),
            )
            .expect("powered object spawns");
        let stalled_connect = engine
            .spawn_object(
                SpawnConfig::new("PWRD")
                    .with_action(ActionState::new("ConnectWork"))
                    .with_energy(0)
                    .with_loaded(true),
            )
            .expect("underpowered CONNECT object spawns");

        engine.tick_without_snapshot().expect("energy-gated frame executes");
        let stalled_idx = engine
            .find_object_index(stalled)
            .expect("underpowered object remains");
        let stalled_object = &engine.objects[stalled_idx];
        assert_eq!(stalled_object.state.energy, 5);
        assert!(stalled_object.state.need_energy);
        assert_eq!(stalled_object.state.action.name, "Work");
        assert_eq!(stalled_object.state.action.time, 4);
        assert_eq!(stalled_object.state.action.phase, 2);
        assert_eq!(stalled_object.state.action.ticks, 0);
        assert_eq!(
            stalled_object.fixed_velocity.x,
            itofix(1),
            "insufficient energy skips WALK steering"
        );
        assert_eq!(
            stalled_object.fixed_velocity.y,
            PhysicsSettings::new(100, 200, -200).gravity_as_c4fixed(),
            "a Mobile stalled action still receives raw DoGravity"
        );

        let powered_idx = engine
            .find_object_index(powered)
            .expect("powered object remains");
        let powered_object = &engine.objects[powered_idx];
        assert_eq!(powered_object.state.energy, 0, "equality is sufficient");
        assert!(
            powered_object.state.alive,
            "direct EnergyUsage subtraction does not run DoEnergy death"
        );
        assert!(!powered_object.state.need_energy);
        assert_eq!(powered_object.state.action.time, 5);
        assert_eq!(powered_object.state.action.phase, 3);
        let stalled_connect_idx = engine
            .find_object_index(stalled_connect)
            .expect("energy gate keeps CONNECT line alive");
        assert!(engine.objects[stalled_connect_idx].state.need_energy);
        assert_eq!(engine.objects[stalled_connect_idx].state.action.time, 0);

        // ExecMovement's no-landscape liquid probe clears InLiquid after the
        // first action. Re-arm the saved flag to exercise the next ExecAction.
        engine.objects[stalled_idx].state.in_liquid = true;
        engine.set_structures_need_energy(false);
        engine.tick_without_snapshot().expect("rule-off frame executes");
        let stalled_idx = engine
            .find_object_index(stalled)
            .expect("underpowered object remains after rule-off frame");
        let powered_idx = engine
            .find_object_index(powered)
            .expect("powered object remains after rule-off frame");
        let stalled_object = &engine.objects[stalled_idx];
        assert_eq!(stalled_object.state.energy, 5, "rule-off skips the drain");
        assert!(
            stalled_object.state.need_energy,
            "rule-off leaves the stale NeedEnergy bit untouched"
        );
        assert_eq!(
            stalled_object.state.action.name, "Wet",
            "rule-off resumes later InLiquidAction work"
        );
        let powered_object = &engine.objects[powered_idx];
        assert_eq!(powered_object.state.energy, 0, "rule-off skips the drain");
        assert_eq!(powered_object.state.action.time, 6);
        assert!(
            engine.find_object_index(stalled_connect).is_none(),
            "once ungated, CONNECT reaches its missing-target LineBreak removal"
        );
    }

    #[test]
    fn exec_action_incomplete_objects_reset_before_action_work() {
        let script = r#"#strict
local abort_count, abort_phase, wet_start_count;

protected func WalkAbort(int phase)
{
    abort_count = abort_count + 1;
    abort_phase = phase;
}

protected func WetStart()
{
    wet_start_count = wet_start_count + 1;
}
"#;
        let make_definition = |id: &str, incomplete_activity: bool| {
            let mut definition =
                Definition::from_script(id, id, script).expect("definition compiles");
            definition.set_c4_callback_convention(true);
            definition.configure_actions(
                Some("Walk".to_string()),
                HashMap::from([
                    (
                        "Walk".to_string(),
                        ActionSpec::default()
                            .with_procedure("WALK")
                            .with_length(20)
                            .with_delay(1)
                            .with_next("Hold")
                            .with_abort_call("WalkAbort")
                            .with_energy_usage(10)
                            .with_in_liquid_action("Wet"),
                    ),
                    (
                        "Wet".to_string(),
                        ActionSpec::default()
                            .with_procedure("SWIM")
                            .with_start_call("WetStart"),
                    ),
                ]),
            );
            definition.set_incomplete_activity(incomplete_activity);
            definition
        };

        let mut engine = Engine::with_seed(12);
        engine.set_structures_need_energy(true);
        engine.set_physics(PhysicsSettings::new(100, 200, -200));
        engine
            .register_definition(make_definition("RST0", false))
            .expect("reset definition registers");
        engine
            .register_definition(make_definition("KEEP", true))
            .expect("incomplete-activity definition registers");
        let walk_action = || {
            let mut action = ActionState::new("Walk");
            action.time = 7;
            action.phase = 3;
            action
        };
        let callback_vars = || {
            HashMap::from([
                ("abort_count".to_string(), Value::Int(0)),
                ("abort_phase".to_string(), Value::Int(-1)),
                ("wet_start_count".to_string(), Value::Int(0)),
            ])
        };
        let reset = engine
            .spawn_object(
                SpawnConfig::new("RST0")
                    .with_category(CATEGORY_OBJECT)
                    .with_construction(FULL_CON / 2)
                    .with_action(walk_action())
                    .with_command_direction(CommandDirection::Right)
                    .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                    .with_energy(20)
                    .with_need_energy(true)
                    .with_in_liquid(true)
                    .with_mobile(true)
                    .with_local_vars(callback_vars())
                    .with_loaded(true),
            )
            .expect("reset object spawns");
        let keep = engine
            .spawn_object(
                SpawnConfig::new("KEEP")
                    .with_category(CATEGORY_OBJECT)
                    .with_construction(FULL_CON / 2)
                    .with_action(walk_action())
                    .with_command_direction(CommandDirection::Right)
                    .with_fixed_velocity(FixedVec2::new(itofix(1), C4Fixed::ZERO))
                    .with_energy(20)
                    .with_need_energy(true)
                    .with_mobile(true)
                    .with_local_vars(callback_vars())
                    .with_loaded(true),
            )
            .expect("incomplete-activity object spawns");

        // The loader correctly coerces a partial non-IncompleteActivity
        // object to ActIdle before restoring its saved counters. C++ can
        // still reach this ExecAction guard when an object becomes partial
        // after starting WALK, so stage that live runtime state explicitly.
        let reset_idx = engine.find_object_index(reset).expect("reset object exists");
        engine.objects[reset_idx].state.action = walk_action();

        for id in [reset, keep] {
            let idx = engine.find_object_index(id).expect("object exists");
            assert_eq!(
                engine.objects[idx].state.ocf & ocf::FULL_CON,
                0,
                "fixture must exercise the live OCF_FullCon gate"
            );
        }

        engine.tick_without_snapshot().expect("incomplete-action frame executes");
        let reset_idx = engine.find_object_index(reset).expect("reset object remains");
        let reset_object = &engine.objects[reset_idx];
        assert_eq!(reset_object.state.action, ActionState::new("Idle"));
        assert_eq!(reset_object.state.energy, 20);
        assert!(
            reset_object.state.need_energy,
            "incomplete reset precedes and therefore skips EnergyUsage"
        );
        assert_eq!(reset_object.fixed_velocity.x, itofix(1));
        assert_eq!(
            reset_object.fixed_velocity.y,
            C4Fixed::ZERO,
            "the reset frame returns before both WALK steering and idle gravity"
        );
        assert_eq!(
            reset_object.state.local_vars.get("abort_count"),
            Some(&Value::Int(1))
        );
        assert_eq!(
            reset_object.state.local_vars.get("abort_phase"),
            Some(&Value::Int(3)),
            "ordinary SetAction supplies the previous phase to AbortCall"
        );
        assert_eq!(
            reset_object.state.local_vars.get("wet_start_count"),
            Some(&Value::Int(0)),
            "incomplete reset precedes InLiquidAction"
        );

        let keep_idx = engine
            .find_object_index(keep)
            .expect("incomplete-activity object remains");
        let keep_object = &engine.objects[keep_idx];
        assert_eq!(keep_object.state.action.name, "Walk");
        assert_eq!(keep_object.state.action.time, 8);
        assert_eq!(keep_object.state.energy, 10);
        assert!(!keep_object.state.need_energy);
        assert_eq!(
            keep_object.state.local_vars.get("abort_count"),
            Some(&Value::Int(0))
        );
    }

    #[test]
    fn resting_object_freezes_until_tick10_mobilization_like_cpp() {
        let mut engine = Engine::with_seed(42);
        engine
            .register_definition(simple_definition("Test"))
            .expect("definition registers");
        engine.set_physics(PhysicsSettings::new(100, 200, -200));
        engine.set_environment(EnvironmentSettings::new(0));

        let id = engine
            .spawn_object(
                SpawnConfig::new("Test")
                    .with_position(Vector2::new(0, 0))
                    .with_category(CATEGORY_OBJECT),
            )
            .expect("spawn succeeds");

        for frame in 1..=9 {
            engine.tick_without_snapshot().expect("tick succeeds");
            let idx = engine.find_object_index(id).expect("object exists");
            let object = &engine.objects[idx];
            assert_eq!(
                object.fixed_velocity.y.val(),
                0,
                "no idle gravity while immobile (frame {frame}, C4Object.cpp:4710)"
            );
            assert!(
                !object.state.mobile,
                "iTick10 != 0 keeps the object demobilized (frame {frame})"
            );
            assert_eq!(object.state.position, Vector2::new(0, 0));
        }

        // Frame 10: the pulse mobilizes with zeroed dirs
        // (C4Movement.cpp:581-586); this frame's ExecAction already ran
        // without Mobile, so ydir is still zero.
        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert!(
            engine.objects[idx].state.mobile,
            "Tick10 re-mobilizes resting objects (C4Movement.cpp:586)"
        );
        assert_eq!(engine.objects[idx].fixed_velocity.y.val(), 0);

        // Frame 11: first gravity probe (ydir += GravAccel, raw 13107 for
        // Gravity=100 — parity/golden/parity_golden.json movement[0]).
        engine.tick_without_snapshot().expect("tick succeeds");
        let idx = engine.find_object_index(id).expect("object exists");
        assert_eq!(engine.objects[idx].fixed_velocity.y.val(), 13107);
    }

    // Mirrors C4Object::CopyMotion via ExecMovement's containment gate
    // (C4Movement.cpp:518-529,556-561): contained objects follow the
    // container's integer position each frame, snap fix_x/fix_y to
    // itofix(x/y) and copy the container's dirs — their own velocity
    // never integrates. The container executes first (spawn order = the
    // C++ tail-first walk for same-frame creations), so the content sees
    // this frame's container position.
    #[test]
    fn contained_object_copies_container_motion_like_cpp() {
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(simple_definition("Wagon"))
            .expect("wagon registers");
        engine
            .register_definition(simple_definition("Gem"))
            .expect("gem registers");

        let wagon = engine
            .spawn_object(
                SpawnConfig::new("Wagon")
                    .with_category(CATEGORY_VEHICLE)
                    .with_position(Vector2::new(10, 10))
                    .with_velocity(Vector2::new(2, 0)),
            )
            .expect("wagon spawns");
        let gem = engine
            .spawn_object(
                SpawnConfig::new("Gem")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(0, 0))
                    .with_container(wagon),
            )
            .expect("gem spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let wagon_idx = engine.find_object_index(wagon).expect("wagon exists");
        let gem_idx = engine.find_object_index(gem).expect("gem exists");
        let wagon_position = engine.objects[wagon_idx].state.position;
        assert_ne!(
            wagon_position,
            Vector2::new(10, 10),
            "the mobile wagon moves"
        );
        assert_eq!(
            engine.objects[gem_idx].state.position, wagon_position,
            "content follows the container (C4Movement.cpp:556-561)"
        );
        assert_eq!(
            engine.objects[gem_idx].fixed_velocity, engine.objects[wagon_idx].fixed_velocity,
            "dirs copied from the container (C4Movement.cpp:528)"
        );
        assert_eq!(
            engine.objects[gem_idx].fixed_position.x.val(),
            itofix(wagon_position.x).val(),
            "fix snapped to itofix(x), not the container's sub-pixel fix (C4Movement.cpp:527)"
        );
    }

    // C4Game::NewObject runs DoCon(FullCon, fInitial) on every freshly
    // CREATED object: the straight-con bottom y-adjust keeps the con-0
    // bottom — the given y — fixed while the shape grows, so the final
    // center is y - (Shape.Hgt + Shape.y) (C4Object.cpp:1401-1470). The
    // live oracle: CreateObject(COAC,28,270) rests at 250 (Hgt 40, y -20),
    // BNDT at 560 -> 550 (Hgt 20, y -10), NDWA 50 -> 49 (Hgt 2). Loaded
    // objects keep their saved center verbatim.
    #[test]
    fn created_objects_grow_from_the_given_bottom_like_cpp() {
        let mut bandit = simple_definition("BNDT");
        bandit.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        let mut engine = Engine::with_seed(0);
        engine
            .register_definition(bandit)
            .expect("bandit registers");
        engine
            .register_definition(simple_definition("MARK"))
            .expect("marker registers");

        let created = engine
            .spawn_object(
                SpawnConfig::new("BNDT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 560)),
            )
            .expect("bandit spawns");
        let idx = engine.find_object_index(created).expect("bandit exists");
        assert_eq!(
            engine.objects[idx].state.position,
            Vector2::new(100, 550),
            "created objects: y - (Hgt + Shape.y) = 560 - (20 - 10) (C4Object.cpp:1467)"
        );
        // DoCon's initial adjust moves the INT y only — C++ leaves fix_y
        // at the GIVEN center until a SetAction or the Tick10 rearm
        // resyncs it (C4Object.cpp:4144, C4Movement.cpp:581-586).
        assert_eq!(
            engine.objects[idx].fixed_position.y.val(),
            itofix(560).val(),
            "fix keeps the given center (the DoCon y/fix split)"
        );

        let loaded = engine
            .spawn_object(
                SpawnConfig::new("BNDT")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 560))
                    .with_loaded(true),
            )
            .expect("loaded spawns");
        let idx = engine.find_object_index(loaded).expect("loaded exists");
        assert_eq!(
            engine.objects[idx].state.position,
            Vector2::new(100, 560),
            "loaded objects keep the saved center"
        );

        // Shapeless fixture defs shift by nothing.
        let marker = engine
            .spawn_object(
                SpawnConfig::new("MARK")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 5)),
            )
            .expect("marker spawns");
        let idx = engine.find_object_index(marker).expect("marker exists");
        assert_eq!(engine.objects[idx].state.position, Vector2::new(5, 5));
    }

    // Mirrors C4Object::Stabilize (C4Movement.cpp:488-516) at the
    // ExecMovement static branch (:579): a resting object tilted within
    // ±StableRange (±10, C4Physics.h:23, after ±180 normalization) snaps
    // upright when the rotation-0 shape stands contact-free at the current
    // position; contact at rotation 0 keeps the tilt; larger tilts and
    // NoStabilize defs are untouched.
    #[test]
    fn stabilize_snaps_small_tilts_upright_like_cpp() {
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=50
        "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = simple_definition("Tilt");
        definition.set_shape_vertices(vec![ObjectVertex::new(0, 6).with_friction(100)]);
        definition.set_contact_density(50);
        definition.set_rotateable(1);

        let mut stiff = simple_definition("Stiff");
        stiff.set_shape_vertices(vec![ObjectVertex::new(0, 6).with_friction(100)]);
        stiff.set_contact_density(50);
        stiff.set_rotateable(1);
        stiff.set_no_stabilize(true);

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(60, 12, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");
        engine.register_definition(stiff).expect("stiff registers");

        // Rotation-0 vertex lands at y=9 (air): free, snaps upright.
        let free = engine
            .spawn_object(
                SpawnConfig::new("Tilt")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 3))
                    .with_rotation(356),
            )
            .expect("free spawns");
        // Rotation-0 vertex lands at y=13 (solid): contact, tilt kept.
        let blocked = engine
            .spawn_object(
                SpawnConfig::new("Tilt")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 7))
                    .with_rotation(356),
            )
            .expect("blocked spawns");
        // Tilt outside ±StableRange: untouched.
        let leaning = engine
            .spawn_object(
                SpawnConfig::new("Tilt")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(35, 3))
                    .with_rotation(340),
            )
            .expect("leaning spawns");
        // NoStabilize def: untouched (C4Movement.cpp:491).
        let stiff_id = engine
            .spawn_object(
                SpawnConfig::new("Stiff")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(50, 3))
                    .with_rotation(356),
            )
            .expect("stiff spawns");
        // Stabilize normalizes with repeated +/-360 steps, not a single
        // wrap (C4Movement.cpp:493-494). A raw saved r=716 is therefore -4
        // and falls inside StableRange.
        let multi_wrapped = engine
            .spawn_object(
                SpawnConfig::new("Tilt")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(55, 3))
                    .with_rotation(716),
            )
            .expect("multi-wrapped tilt spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let rotation_of = |engine: &Engine, id| {
            engine
                .find_object_index(id)
                .map(|idx| engine.objects[idx].state.rotation)
                .expect("object exists")
        };
        assert_eq!(
            rotation_of(&engine, free),
            0,
            "small tilt snaps upright when rotation 0 is contact-free (C4Movement.cpp:509-514)"
        );
        let free_idx = engine.find_object_index(free).expect("free exists");
        assert_eq!(
            engine.objects[free_idx].fixed_rotation.val(),
            0,
            "fix_r follows the stabilization (C4Movement.cpp:512)"
        );
        assert_eq!(
            rotation_of(&engine, blocked),
            356,
            "contact at rotation 0 keeps the tilt (C4Movement.cpp:503-508)"
        );
        assert_eq!(
            rotation_of(&engine, leaning),
            340,
            "tilts beyond ±StableRange stay (C4Movement.cpp:495)"
        );
        assert_eq!(
            rotation_of(&engine, stiff_id),
            356,
            "NoStabilize opts out (C4Movement.cpp:491)"
        );
        assert_eq!(
            rotation_of(&engine, multi_wrapped),
            0,
            "repeated angle normalization brings 716 degrees to -4"
        );
    }

    #[test]
    fn stabilize_contact_probe_dispatches_contact_callbacks_like_cpp() {
        // C4Object::Stabilize temporarily installs the upright shape and
        // calls the ordinary ContactCheck (C4Movement.cpp:498-507), which
        // dispatches ContactLeft/Right/Top/Bottom when ContactCalls is set
        // (:112-121,166-182). The callback runs even though stabilization is
        // rejected and the original tilt is restored.
        let library = MaterialLibrary::parse(
            r#"
            [Material Earth]
            Name=Earth
            Density=100
            Friction=50
            "#,
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");

        let mut definition = Definition::from_script(
            "CBST",
            "Callback stabilizer",
            r#"
            #strict
            local touched;
            public func ContactBottom() { touched = 1; return 0; }
            public func ReadTouched() { return touched; }
            "#,
        )
        .expect("definition compiles");
        definition.set_shape_vertices(vec![
            ObjectVertex::new(0, 6)
                .with_cnat(CNAT_BOTTOM)
                .with_friction(100),
        ]);
        definition.set_contact_density(50);
        definition.set_rotateable(1);
        definition.set_contact_function_calls(true);

        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(20, 12, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));
        engine
            .register_definition(definition)
            .expect("definition registers");
        let object = engine
            .spawn_object(
                SpawnConfig::new("CBST")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(10, 7))
                    .with_rotation(356),
            )
            .expect("object spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let index = engine.find_object_index(object).expect("object remains");
        assert_eq!(engine.objects[index].state.rotation, 356, "contact keeps tilt");
        assert_eq!(
            engine
                .call_object_function(index, "ReadTouched", Vec::new())
                .expect("read succeeds"),
            Value::Int(1),
            "Stabilize's ContactCheck dispatches ContactBottom"
        );
    }

    #[test]
    fn rejected_stabilize_keeps_the_trial_update_pos_sector_links_like_cpp() {
        // UpdateShape performs Stabilize's one UpdatePos before ContactCheck.
        // On rejection C++ restores only Shape and r; it deliberately does not
        // perform a second UpdatePos (oracle-src-pinned
        // src/C4Movement.cpp:493-519; src/C4Object.cpp:322-355).
        let library = MaterialLibrary::parse(
            "[Material Earth]\nName=Earth\nDensity=100\n",
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(120, 50, Some(earth)));

        let mut definition =
            Definition::from_script("SRJT", "Rejected stabilizer", "")
                .expect("definition compiles");
        definition.set_rotateable(1);
        definition.set_contact_density(50);
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        definition
            .set_shape_vertices(vec![ObjectVertex::new(0, 2).with_cnat(CNAT_BOTTOM)]);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let object_id = engine
            .spawn_object(
                SpawnConfig::new("SRJT")
                    .with_position(Vector2::new(49, 49))
                    .with_rotation(5),
            )
            .expect("object spawns");
        let index = engine.find_object_index(object_id).expect("object exists");
        assert_eq!(
            engine
                .sectors
                .as_ref()
                .expect("sectors exist")
                .shape_ids(sector::SectorKey::Inside { x: 1, y: 0 }),
            &[object_id],
            "the rotated entry shape crosses the sector boundary"
        );

        engine
            .stabilize_object(index, &[])
            .expect("stabilize executes");

        let object = &engine.objects[index];
        assert_eq!(object.state.rotation, 5, "ground contact rejects upright");
        assert_eq!(
            object.current_shape_rect(),
            Some(DefinitionRect::new(-2, -2, 4, 4)),
            "the rejected trial restores the rotated Shape value"
        );
        assert!(
            engine
                .sectors
                .as_ref()
                .expect("sectors exist")
                .shape_ids(sector::SectorKey::Inside { x: 1, y: 0 })
                .is_empty(),
            "sector links remain those produced by the upright trial UpdateShape"
        );
    }

    #[test]
    fn accepted_stabilize_commits_callback_live_rotation_and_rebuilds_face_like_cpp() {
        // ContactBottom's SetR rebuild clears Shape.ContactCount, so the outer
        // ContactCheck returns zero and accepts stabilization. The accepted
        // arm then reads callback-live r and runs UpdateFace(true), clearing
        // the later SetShape override (oracle-src-pinned
        // src/C4Movement.cpp:166-182,493-519; src/C4Object.cpp:322-365).
        let script = r#"#strict 3
public func ContactBottom()
{
    SetR(73);
    SetShape(-7, -8, 14, 16);
    return 0;
}
"#;
        let library = MaterialLibrary::parse(
            "[Material Earth]\nName=Earth\nDensity=100\n",
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(120, 50, Some(earth)));

        let mut definition =
            Definition::from_script("SACC", "Accepted stabilizer", script)
                .expect("definition compiles");
        definition.set_c4_callback_convention(true);
        definition.set_contact_function_calls(true);
        definition.set_rotateable(1);
        definition.set_contact_density(50);
        definition.set_shape_rect(Some(DefinitionRect::new(0, 0, 1, 1)));
        definition
            .set_shape_vertices(vec![ObjectVertex::new(0, 2).with_cnat(CNAT_BOTTOM)]);
        engine
            .register_definition(definition)
            .expect("definition registers");

        let object_id = engine
            .spawn_object(
                SpawnConfig::new("SACC")
                    .with_position(Vector2::new(49, 49))
                    .with_rotation(5),
            )
            .expect("object spawns");
        let index = engine.find_object_index(object_id).expect("object exists");

        engine
            .stabilize_object(index, &[])
            .expect("stabilize executes");

        let object = &engine.objects[index];
        assert_eq!(object.state.rotation, 73);
        assert_eq!(object.fixed_rotation, itofix(73));
        assert_eq!(
            object.state.shape_override, None,
            "the accepted arm's UpdateFace(true) rebuilds the definition shape"
        );
    }

    #[test]
    fn contact_removal_still_completes_exec_movement_tail_like_cpp() {
        // AssignRemoval inside DoMovement does not unwind C++ ExecMovement:
        // demobilization, Stabilize and the non-rotateable r=0 assignment run
        // before C4Object::Execute checks Status (oracle-src-pinned
        // src/C4Movement.cpp:558-620; src/C4Object.cpp:1082-1094).
        let script = r#"#strict 3
public func ContactBottom()
{
    RemoveObject();
    return 0;
}
"#;
        let library = MaterialLibrary::parse(
            "[Material Earth]\nName=Earth\nDensity=100\n",
        )
        .expect("material library parses");
        let materials = MaterialSet::from_resource_library(&library);
        let earth = materials.id_of("Earth").expect("earth exists");
        let mut engine = Engine::with_seed(0);
        engine.set_materials(materials);
        engine.set_landscape(Landscape::flat_with_material(120, 50, Some(earth)));
        engine.set_physics(PhysicsSettings::new(0, 20, -20));

        let mut victim_definition =
            Definition::from_script("TOMB", "Movement tombstone", script)
                .expect("victim definition compiles");
        victim_definition.set_c4_callback_convention(true);
        victim_definition.set_contact_function_calls(true);
        victim_definition.set_contact_density(50);
        victim_definition
            .set_shape_vertices(vec![ObjectVertex::new(0, 2).with_cnat(CNAT_BOTTOM)]);
        engine
            .register_definition(victim_definition)
            .expect("victim definition registers");

        let victim = engine
            .spawn_object(
                SpawnConfig::new("TOMB")
                    .with_position(Vector2::new(20, 49))
                    .with_rotation(20)
                    .with_fixed_velocity(FixedVec2::new(C4Fixed::ZERO, itofix(1)))
                    .with_mobile(true),
            )
            .expect("victim spawns");
        let victim_index = engine
            .find_object_index(victim)
            .expect("victim exists");
        let definition_id = engine.objects[victim_index].definition_id.clone();
        let actions = engine
            .definition(&definition_id)
            .expect("victim definition remains")
            .action_library()
            .clone();

        let outcome = engine
            .exec_mobile_object_movement(victim_index, &actions, &definition_id, &[])
            .expect("mobile ExecMovement completes");

        assert!(!outcome.alive, "ContactBottom removes the victim");
        let victim_index = engine
            .find_object_index(victim)
            .expect("the synchronous tombstone remains addressable");
        assert!(
            engine.objects[victim_index].destroyed,
            "victim state after movement: position={:?} velocity={:?} contact={} status={:?}",
            engine.objects[victim_index].state.position,
            engine.objects[victim_index].fixed_velocity,
            engine.objects[victim_index].frame_t_contact,
            engine.objects[victim_index].state.status
        );
        assert_eq!(
            engine.objects[victim_index].state.rotation, 0,
            "non-rotateable ExecMovement tail still runs after Contact removal"
        );
    }

    // Mirrors the ExecAction upright-attachment check
    // (C4Object.cpp:4698-4705): a resting (non-Mobile) object whose def
    // sets UprightAttach re-arms Mobile every frame while standing within
    // ±StableRange; tilts beyond the range stay demobilized until the
    // Tick10 pulse.
    // The GoldRush coach: UprightAttach=8, bottom vertices 5px above the
    // shape bottom, spawned where the attach probe finds NO ground in
    // range. C++ NoAttachAction falls through to ObjectActionJump, which
    // is `if (!SetActionByName("Jump")) return false`
    // (C4ObjectCom.cpp:54) -- a def without a Jump action KEEPS its
    // current action (the live Idle-vs-Turn class was the rust resetting
    // to the library default instead).
    #[test]
    fn upright_attached_vehicle_on_ground_keeps_its_action_like_cpp() {
        let mut coach = Definition::from_script("Coch", "Coach", "#strict\n").expect("compiles");
        coach.set_shape_rect(Some(DefinitionRect::new(-27, -20, 55, 40)));
        coach.set_shape_vertices(vec![
            ObjectVertex {
                x: 0,
                y: 1,
                cnat: 0,
                friction: 100,
            },
            ObjectVertex {
                x: -16,
                y: 15,
                cnat: 9,
                friction: 10,
            },
            ObjectVertex {
                x: 16,
                y: 15,
                cnat: 10,
                friction: 10,
            },
        ]);
        coach.set_upright_attach(CNAT_BOTTOM as i32);
        coach.configure_actions(
            None,
            HashMap::from([
                (
                    "Turn".to_string(),
                    ActionSpec::default()
                        .with_delay(2)
                        .with_length(20)
                        .with_next("Drive0"),
                ),
                (
                    "Drive0".to_string(),
                    ActionSpec::default()
                        .with_delay(10)
                        .with_length(1)
                        .with_next("Drive0"),
                ),
            ]),
        );
        let mut engine = Engine::with_seed(0);
        engine.set_landscape(Landscape::flat(200, 400));
        engine.set_physics(PhysicsSettings::new(20, 100, -100));
        engine.register_definition(coach).expect("registers");

        // The DoCon bottom adjust puts the given y=270 center at 250:
        // bottom vertices sit at 265, the road at 270 - inside the
        // 5px attach range.
        let coach_id = engine
            .spawn_object(
                SpawnConfig::new("Coch")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(100, 270)),
            )
            .expect("spawns");
        engine
            .apply_object_update(
                coach_id,
                ObjectUpdate {
                    action: Some(ActionUpdate::default().with_name("Turn").with_force(true)),
                    ..Default::default()
                },
            )
            .expect("action set");

        for frame in 0..3 {
            engine.tick_without_snapshot().expect("tick");
            let idx = engine.find_object_index(coach_id).expect("exists");
            assert_eq!(
                engine.objects[idx].state.action.name, "Turn",
                "no Jump in the ActMap: the failed jump keeps Turn (frame {frame})"
            );
        }
        // Gravity still pulls the unattached wagon (DoGravity per exec):
        // three frames of accumulation stay under two integer pixels.
        let idx = engine.find_object_index(coach_id).expect("exists");
        assert_eq!(
            engine.objects[idx].state.position.x, 100,
            "no horizontal drift"
        );
        assert!(
            (250..=251).contains(&engine.objects[idx].state.position.y),
            "slow free-fall, not an attach snap: y={}",
            engine.objects[idx].state.position.y
        );
    }

    #[test]
    fn upright_attach_rearms_mobile_every_frame_like_cpp() {
        let mut definition = simple_definition("Barrel");
        definition.set_upright_attach(CNAT_BOTTOM as i32);
        definition.set_rotateable(1);
        let mut engine = Engine::with_seed(0);
        engine.set_physics(PhysicsSettings::new(100, 200, -200));
        engine.set_environment(EnvironmentSettings::new(0));
        engine
            .register_definition(definition)
            .expect("definition registers");

        let upright = engine
            .spawn_object(
                SpawnConfig::new("Barrel")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(5, 5)),
            )
            .expect("upright spawns");
        let tilted = engine
            .spawn_object(
                SpawnConfig::new("Barrel")
                    .with_category(CATEGORY_OBJECT)
                    .with_position(Vector2::new(20, 5))
                    .with_rotation(340),
            )
            .expect("tilted spawns");

        engine.tick_without_snapshot().expect("tick succeeds");
        let mobile_of = |engine: &Engine, id| {
            engine
                .find_object_index(id)
                .map(|idx| engine.objects[idx].state.mobile)
                .expect("object exists")
        };
        assert!(
            mobile_of(&engine, upright),
            "UprightAttach re-arms a standing object at frame 1 (C4Object.cpp:4704)"
        );
        assert!(
            !mobile_of(&engine, tilted),
            "a 340-degree tilt is outside ±StableRange (C4Object.cpp:4701)"
        );
    }

    // C4Game::NewObj mints strictly increasing object numbers
    // (`Number = ++ObjectEnumerationIndex`); the counter never rewinds
    // within a session. A script-world snapshot's counter written back
    // after an interleaved engine-side spawn is STALE — taking it
    // verbatim re-mints a used id (the GoldRush intro _TLK collided
    // with a same-frame FXU1 and executed twice through the exec list).
    #[test]
    fn stale_script_world_counter_never_rewinds_object_ids() {
        let mut engine = Engine::with_seed(0);
        engine.next_object_id = 100;
        engine.sync_next_object_id(90);
        assert_eq!(
            engine.next_object_id, 100,
            "a stale snapshot counter must not rewind the allocator"
        );
        engine.sync_next_object_id(120);
        assert_eq!(
            engine.next_object_id, 120,
            "world-side allocations advance the engine counter"
        );
    }
