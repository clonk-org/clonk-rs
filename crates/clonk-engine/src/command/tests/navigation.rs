    // Command AI under the normal profile's navigation switch
    // (`sim-navigation-ai`). Everything here deliberately diverges from
    // C4Command; the switch-off behaviour stays pinned by the C++-cited
    // command tests above.

    const NAV_GROUND: i32 = 200;

    /// Solid ground below `NAV_GROUND` on a 480x300 map, then each
    /// (x0, y0, x1, y1, solid) rectangle.
    fn navigation_terrain(rects: &[(i32, i32, i32, i32, bool)]) -> crate::Landscape {
        let (width, height) = (480usize, 300usize);
        let mut pixels = vec![0u8; width * height];
        let mut set = |x0: i32, y0: i32, x1: i32, y1: i32, solid: bool| {
            for y in y0.max(0)..=y1.min(height as i32 - 1) {
                for x in x0.max(0)..=x1.min(width as i32 - 1) {
                    pixels[y as usize * width + x as usize] = u8::from(solid);
                }
            }
        };
        set(0, NAV_GROUND, width as i32 - 1, height as i32 - 1, true);
        for &(x0, y0, x1, y1, solid) in rects {
            set(x0, y0, x1, y1, solid);
        }
        let mut landscape =
            crate::Landscape::with_default_material(width as u32, vec![NAV_GROUND; width], None)
                .expect("navigation landscape");
        landscape.set_world_height(height as i32);
        landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
            width as u32,
            height as u32,
            pixels,
            vec![0, 100],
            vec![None, Some("Earth".to_owned())],
            vec![None; 2],
        ));
        landscape
    }

    /// A standing CLNK: DefCore vertices, physicals and 16x20 shape.
    fn navigating_clonk(position: Vector2) -> CommandObjectSnapshot {
        let mut clonk = walking_jumper(position);
        let vertices = [
            (0, 2, 0),
            (0, -7, crate::CNAT_TOP),
            (0, 9, crate::CNAT_BOTTOM),
            (-2, -3, crate::CNAT_LEFT),
            (2, -3, crate::CNAT_RIGHT),
            (-4, 3, crate::CNAT_LEFT),
            (4, 3, crate::CNAT_RIGHT),
        ]
        .map(|(x, y, cnat)| crate::ObjectVertex {
            x,
            y,
            cnat,
            friction: 0,
        });
        clonk.nav_body = crate::navigation::NavBody::from_vertices(&vertices);
        clonk.physical.walk = 70_000;
        clonk.physical.jump = 40_000;
        clonk.physical.scale = 30_000;
        clonk.physical.can_scale = 1;
        clonk.construction = FULL_CON;
        clonk.shape = DefinitionRect::new(position.x - 8, position.y - 10, 16, 20);
        clonk.shape_top = -10;
        clonk.shape_height = 20;
        clonk
    }

    /// (command, tx, ty, Data) from the stack front down.
    fn stack_moves(stack: &CommandStack) -> Vec<(CommandId, Option<i32>, Option<i32>, i32)> {
        stack
            .snapshot()
            .commands
            .iter()
            .filter_map(|command| command.request.as_ref())
            .map(|request| {
                let data = match request.data {
                    CommandData::Integer(value) => value,
                    _ => 0,
                };
                (request.id, request.tx, request.ty, data)
            })
            .collect()
    }

    fn navigation_kinds(stack: &CommandStack) -> Vec<Option<NavigationKind>> {
        stack_moves(stack)
            .into_iter()
            .map(|(_, _, _, data)| NavigationKind::from_data(data).map(|(kind, _)| kind))
            .collect()
    }

    #[test]
    fn navigation_goal_plans_a_climb_through_hinted_waypoints() {
        let landscape = navigation_terrain(&[(200, NAV_GROUND - 60, 479, NAV_GROUND - 1, true)]);
        let clonk = navigating_clonk(Vector2::new(150, NAV_GROUND - 10));
        let ctx = nav_jump_ctx(&clonk, &landscape);
        let mut stack = CommandStack::new();
        stack
            .push_front(request!(MoveTo, with_tx: Some(300), with_ty: Some(NAV_GROUND - 71), with_evaluated: true, with_mode: CommandMode::Base))
            .expect("MoveTo queues");

        let result = stack.step(&ctx).expect("MoveTo plans");

        assert_eq!(result.status, CommandStatus::Running);
        let moves = stack_moves(&stack);
        let kinds = navigation_kinds(&stack);
        assert_eq!(
            kinds.last(),
            Some(&None),
            "the goal stays at the bottom: {moves:?}"
        );
        assert_eq!(
            kinds[kinds.len() - 2],
            Some(NavigationKind::Walk),
            "on top, walk to the goal: {moves:?}"
        );
        let climb = kinds
            .iter()
            .position(|kind| *kind == Some(NavigationKind::Climb))
            .unwrap_or_else(|| panic!("the face is scaled: {moves:?}"));
        assert_eq!(
            (moves[climb].1, moves[climb].2),
            (Some(202), Some(NAV_GROUND - 70)),
            "measured KneelUp position"
        );
        assert!(
            NavigationKind::from_data(moves[climb].3).is_some_and(|(_, right)| right),
            "the climb faces the wall"
        );
        assert!(
            kinds[..climb]
                .iter()
                .all(|kind| matches!(kind, Some(NavigationKind::Walk | NavigationKind::Jump))),
            "reaching the face walks or jumps to it: {moves:?}"
        );
    }

    #[test]
    fn navigation_off_keeps_the_native_path_phase() {
        let landscape = navigation_terrain(&[(200, NAV_GROUND - 60, 479, NAV_GROUND - 1, true)]);
        let clonk = navigating_clonk(Vector2::new(150, NAV_GROUND - 10));
        let ctx = jump_ctx(&clonk, &landscape);
        let mut stack = CommandStack::new();
        stack
            .push_front(request!(MoveTo, with_tx: Some(300), with_ty: Some(NAV_GROUND - 71), with_evaluated: true, with_mode: CommandMode::Base))
            .expect("MoveTo queues");

        stack.step(&ctx).expect("MoveTo executes");

        assert!(
            navigation_kinds(&stack).iter().all(Option::is_none),
            "C4PathFinder waypoints carry no navigation hints: {:?}",
            stack_moves(&stack)
        );
    }

    #[test]
    fn navigation_walk_waypoint_steers_then_arrives() {
        let landscape = navigation_terrain(&[]);
        let walk = NavigationKind::Walk.data(true);
        let request = request!(MoveTo, with_tx: Some(180), with_ty: Some(NAV_GROUND - 10), with_data: CommandData::Integer(walk), with_update_interval: 60, with_evaluated: true, with_mode: CommandMode::SilentSub);

        let clonk = navigating_clonk(Vector2::new(150, NAV_GROUND - 10));
        let mut stack = CommandStack::new();
        stack.push_front(request.clone()).expect("waypoint queues");
        let steer = stack
            .step(&nav_jump_ctx(&clonk, &landscape))
            .expect("waypoint steers");
        assert_eq!(steer.status, CommandStatus::Running);
        assert_eq!(
            steer.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );
        assert!(
            steer.operations.is_empty(),
            "no heuristic jumps on a planned walk"
        );

        let arrived = navigating_clonk(Vector2::new(178, NAV_GROUND - 10));
        let done = stack
            .step(&nav_jump_ctx(&arrived, &landscape))
            .expect("waypoint arrives");
        assert_eq!(done.status, CommandStatus::Completed);
        assert_eq!(
            done.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Stop)
        );
    }

    #[test]
    fn navigation_climb_waypoint_walks_into_the_wall_then_scales_up() {
        let landscape = navigation_terrain(&[(200, NAV_GROUND - 60, 479, NAV_GROUND - 1, true)]);
        let climb = NavigationKind::Climb.data(true);
        let mut stack = CommandStack::new();
        stack
            .push_front(request!(MoveTo, with_tx: Some(202), with_ty: Some(NAV_GROUND - 70), with_data: CommandData::Integer(climb), with_update_interval: 300, with_evaluated: true, with_mode: CommandMode::SilentSub))
            .expect("waypoint queues");

        let walker = navigating_clonk(Vector2::new(195, NAV_GROUND - 10));
        let approach = stack
            .step(&nav_jump_ctx(&walker, &landscape))
            .expect("climb approaches");
        assert_eq!(
            approach.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right),
            "ComDir toward the face turns contact into SCALE"
        );

        let mut scaler = navigating_clonk(Vector2::new(195, NAV_GROUND - 40));
        scaler.action_procedure = ActionProcedure::Scale;
        let climbing = stack
            .step(&nav_jump_ctx(&scaler, &landscape))
            .expect("climb scales");
        assert_eq!(
            climbing.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Up)
        );

        let fallen = navigating_clonk(Vector2::new(185, NAV_GROUND - 10));
        let failed = stack
            .step(&nav_jump_ctx(&fallen, &landscape))
            .expect("fallen climber evaluates");
        assert_eq!(
            failed.status,
            CommandStatus::Failed,
            "standing again below the wall after scaling means the climb failed"
        );
    }

    #[test]
    fn navigation_jump_waypoint_launches_once_and_fails_on_a_wrong_landing() {
        let landscape = navigation_terrain(&[]);
        let jump = NavigationKind::Jump.data(true);
        let mut stack = CommandStack::new();
        stack
            .push_front(request!(MoveTo, with_tx: Some(228), with_ty: Some(NAV_GROUND - 10), with_data: CommandData::Integer(jump), with_update_interval: 120, with_evaluated: true, with_mode: CommandMode::SilentSub))
            .expect("waypoint queues");

        let takeoff = navigating_clonk(Vector2::new(150, NAV_GROUND - 10));
        let launch = stack
            .step(&nav_jump_ctx(&takeoff, &landscape))
            .expect("jump launches");
        assert_eq!(
            launch.update.and_then(|update| update.command_direction),
            Some(CommandDirection::Right)
        );
        assert_eq!(stack.command_names(), vec!["Jump", "MoveTo"]);
        assert!(stack.complete_front_if(CommandId::Jump));

        let mut airborne = navigating_clonk(Vector2::new(180, NAV_GROUND - 40));
        airborne.action_procedure = ActionProcedure::Flight;
        let flying = stack
            .step(&nav_jump_ctx(&airborne, &landscape))
            .expect("jump flies");
        assert_eq!(flying.status, CommandStatus::Running);
        assert!(flying.operations.is_empty(), "no second launch mid-air");

        let short = navigating_clonk(Vector2::new(200, NAV_GROUND - 10));
        let landed = stack
            .step(&nav_jump_ctx(&short, &landscape))
            .expect("jump lands");
        assert_eq!(
            landed.status,
            CommandStatus::Failed,
            "landing off the planned spot fails instead of jumping again"
        );
    }

    #[test]
    fn navigation_waypoint_expiring_before_arrival_fails_its_parent() {
        let landscape = navigation_terrain(&[]);
        let walk = NavigationKind::Walk.data(true);
        let clonk = navigating_clonk(Vector2::new(150, NAV_GROUND - 10));
        let mut stack = CommandStack::new();
        stack
            .push_back(request!(Wait, with_mode: CommandMode::Base))
            .expect("base queues");
        stack
            .push_front(request!(MoveTo, with_tx: Some(400), with_ty: Some(NAV_GROUND - 10), with_data: CommandData::Integer(walk), with_update_interval: 3, with_evaluated: true, with_mode: CommandMode::SilentSub))
            .expect("waypoint queues");

        for _ in 0..2 {
            stack
                .step(&nav_jump_ctx(&clonk, &landscape))
                .expect("waypoint steers");
        }
        let expired = stack
            .step(&nav_jump_ctx(&clonk, &landscape))
            .expect("waypoint expires");

        assert_eq!(expired.status, CommandStatus::Failed);
        let snapshot = stack.snapshot();
        assert_eq!(snapshot.commands.len(), 1);
        assert_eq!(
            snapshot.commands[0].failures, 1,
            "the base learns the route failed"
        );
    }

    #[test]
    fn navigation_goal_without_a_route_fails_instead_of_pacing() {
        let landscape = navigation_terrain(&[(300, NAV_GROUND - 80, 340, NAV_GROUND - 75, true)]);
        let clonk = navigating_clonk(Vector2::new(150, NAV_GROUND - 10));
        let mut stack = CommandStack::new();
        stack
            .push_front(request!(MoveTo, with_tx: Some(320), with_ty: Some(NAV_GROUND - 91), with_evaluated: true, with_mode: CommandMode::Base))
            .expect("MoveTo queues");

        let mut failed_at = None;
        for frame in 1..=1_000 {
            let Some(result) = stack.step(&nav_jump_ctx(&clonk, &landscape)) else {
                break;
            };
            while stack.len() > 1 {
                // Children (fallback waypoints, Jump, Retry) finish at once:
                // the walker never gets anywhere near the island.
                let front = stack.command_names()[0].clone();
                let id = CommandId::from_name(&front).expect("known command");
                assert!(stack.complete_front_if(id));
            }
            if result.status == CommandStatus::Failed {
                failed_at = Some(frame);
                break;
            }
        }
        assert!(
            failed_at.is_some_and(|frame| frame <= NAVIGATION_MAX_STEER_FRAMES as usize + 50),
            "an unreachable goal fails within the steering budget, got {failed_at:?}"
        );
    }

    fn loose_rock(id: u64, position: Vector2) -> CommandObjectSnapshot {
        command_object!(id; definition_id = "ROCK".into(); position = position;
            ocf = ocf::AVAILABLE | ocf::FULL_CON; collectible = true; construction = FULL_CON)
    }

    #[test]
    fn navigation_acquire_prefers_a_reachable_item_over_an_unreachable_nearer_one() {
        // A rock on a floating island is nearer in a straight line, which is
        // all C4Command::Acquire compares (C4Command.cpp:2108-2126).
        let landscape = navigation_terrain(&[(300, NAV_GROUND - 80, 340, NAV_GROUND - 75, true)]);
        let clonk = navigating_clonk(Vector2::new(360, NAV_GROUND - 10));
        let island = ObjectId::new(2);
        let ground = ObjectId::new(3);
        let objects = command_objects([
            clonk.clone(),
            loose_rock(island.as_u64(), Vector2::new(320, NAV_GROUND - 84)),
            loose_rock(ground.as_u64(), Vector2::new(60, NAV_GROUND - 4)),
        ]);
        let ctx = command_context!(command_ctx(&clonk, &objects, 0); landscape: Some(&landscape));
        let state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("ROCK".into())),
        )
        .expect("acquire state");
        let gravity = crate::PhysicsSettings::default().gravity_as_c4fixed();

        assert_eq!(state.find_candidate(&ctx), Some(island), "native: nearest");
        assert_eq!(
            state.find_navigation_candidate(&ctx, gravity, None),
            Some(ground),
            "navigation: the one the Clonk can walk to"
        );
        assert_eq!(
            state.find_navigation_candidate(&ctx, gravity, Some(ground)),
            None,
            "nothing reachable once the failed candidate is excluded: buy instead"
        );
    }

    #[test]
    fn navigation_acquire_keeps_the_native_pick_inside_a_building() {
        // A Clonk inside a workshop stands nowhere in the landscape, so no
        // route can start there. It still takes the nearest material, as
        // C4Command::Acquire does (C4Command.cpp:2108-2130), instead of
        // finding nothing reachable and trying to buy.
        let landscape = navigation_terrain(&[]);
        let mut clonk = navigating_clonk(Vector2::new(150, NAV_GROUND - 24));
        clonk.container = Some(ObjectId::new(9));
        let rock = ObjectId::new(2);
        let objects = command_objects([
            clonk.clone(),
            loose_rock(rock.as_u64(), Vector2::new(60, NAV_GROUND - 4)),
        ]);
        let ctx = command_context!(command_ctx(&clonk, &objects, 0); landscape: Some(&landscape));
        let state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("ROCK".into())),
        )
        .expect("acquire state");
        let gravity = crate::math::fixed100(100) / 5;

        assert_eq!(state.find_candidate(&ctx), Some(rock), "native: nearest");
        assert_eq!(
            state.find_navigation_candidate(&ctx, gravity, None),
            Some(rock)
        );
    }

    #[test]
    fn navigation_acquire_leaves_an_item_another_clonk_is_fetching() {
        let landscape = navigation_terrain(&[]);
        let clonk = navigating_clonk(Vector2::new(200, NAV_GROUND - 10));
        let near = ObjectId::new(2);
        let far = ObjectId::new(3);
        let mut other = navigating_clonk(Vector2::new(100, NAV_GROUND - 10));
        other.id = ObjectId::new(9);
        other.commands = vec![command_view(CommandId::Get, Some(near))];
        let rocks = [
            loose_rock(near.as_u64(), Vector2::new(240, NAV_GROUND - 4)),
            loose_rock(far.as_u64(), Vector2::new(330, NAV_GROUND - 4)),
        ];
        let objects = command_objects([
            clonk.clone(),
            other.clone(),
            rocks[0].clone(),
            rocks[1].clone(),
        ]);
        let ctx = command_context!(command_ctx(&clonk, &objects, 0); landscape: Some(&landscape));
        let state = AcquireState::from_request(
            &request!(Acquire, with_data: CommandData::Text("ROCK".into())),
        )
        .expect("acquire state");
        let gravity = crate::PhysicsSettings::default().gravity_as_c4fixed();

        assert_eq!(
            state.find_navigation_candidate(&ctx, gravity, None),
            Some(far)
        );

        // With the free one gone, the claimed rock is still better than none.
        let objects = command_objects([clonk.clone(), other, rocks[0].clone()]);
        let ctx = command_context!(command_ctx(&clonk, &objects, 0); landscape: Some(&landscape));
        assert_eq!(
            state.find_navigation_candidate(&ctx, gravity, None),
            Some(near)
        );
    }
