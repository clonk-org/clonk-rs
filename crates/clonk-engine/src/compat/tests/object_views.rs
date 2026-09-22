#[test]
fn unchanged_scope_overlay_keeps_shared_object_state() {
    let mut engine = crate::Engine::new();
    engine.register_test_definition(test_definition(
        "TEST",
        "Test",
        "func Probe() { return 0; }",
    ));
    let id = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    let world = engine.host_world_context_for_object(0);
    let state = world
        .get_shared(id)
        .test_value()
        .full_state()
        .cloned()
        .test_value();
    let caller = HostObjectContext {
        id,
        ..idle_object_context()
    };
    let (result, _) = with_compat_context!(Some(caller), world, 2, || {
        with_host_context((), |context| {
            let projected = context.get_world_object(id).test_value();
            assert!(
                Rc::ptr_eq(&state, projected.full_state().test_value()),
                "an unchanged construction/mass overlay must not copy the full state"
            );
        });
        Ok::<_, RuntimeError>(())
    });
    result.test_value();
}

#[test]
fn scalar_coordinates_read_live_scopes_without_cloning_objects() {
    // FnGetX/FnGetY and FnObjectDistance read live coordinates directly
    // (C4Script.cpp:1198-1202, 1293-1297, 3315-3319).
    let mut engine = crate::Engine::new();
    engine.register_test_definition(test_definition(
        "TEST",
        "Test",
        "func Probe() { return 0; }",
    ));
    let caller_id = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    let target = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    let world = engine.host_world_context_for_object(0);
    let caller = HostObjectContext {
        id: caller_id,
        ..idle_object_context()
    };
    let (result, _) = with_compat_context!(Some(caller), world, 3, || {
        set_position(&[v_int(30), v_int(40), v_object(target)])?;
        crate::HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(|count| count.set(0));
        assert_eq!(get_x(&[v_object(target)])?, v_int(30));
        assert_eq!(get_y(&[v_object(target)])?, v_int(40));
        assert_eq!(object_distance(&[v_object(target)])?, v_int(50));
        with_host_context((), |context| {
            assert_eq!(
                context.query_object_position(target),
                Some(Vector2::new(30, 40))
            );
            assert_eq!(context.query_object_position(ObjectId::new(999)), None);
        });
        assert_eq!(crate::HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(Cell::get), 0);
        Ok::<_, RuntimeError>(())
    });
    result.test_value();
}

#[test]
fn foreign_object_views_share_a_deferred_full_snapshot() {
    let mut engine = crate::Engine::new();
    engine.register_test_definition(test_definition(
        "TEST",
        "Test",
        "func Probe() { return 0; }",
    ));
    engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    let target = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    engine.objects[1].set_fixed_velocity(FixedVec2::new(
        C4Fixed::from_raw(333),
        C4Fixed::from_raw(-444),
    ));
    let expected = engine.objects[1].script_state_snapshot();
    let world = engine.host_world_context_for_object(0);
    crate::SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
    let first = world.get_shared(target).test_value();
    let mut second = (*first).clone();
    assert_eq!(
        crate::SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        0
    );
    let state = first.full_state().test_value();
    assert_eq!(state.as_ref(), &expected);
    assert!(Rc::ptr_eq(state, second.full_state().test_value()));
    assert_eq!(
        crate::SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        1
    );
    // Preview writes must detach from the shared callback-entry state.
    Rc::make_mut(second.full_state_mut().test_value()).damage = 17;
    assert_eq!(first.full_state().test_value().as_ref(), &expected);
    assert_eq!(second.full_state().test_value().damage, 17);
    assert!(!Rc::ptr_eq(state, second.full_state().test_value()));
    assert_eq!(
        crate::SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
        1
    );
}

#[test]
fn scalar_fields_read_modified_foreign_scopes_without_object_copies() {
    // C++ getters read the target's fields: C4Script.cpp:1119-1136,
    // 1168-1179, 1305-1320, 1360-1364, 1392-1396.
    let mut engine = crate::Engine::new();
    engine.register_test_definition(test_definition(
        "TEST",
        "Test",
        "func Probe() { return 0; }",
    ));
    let caller_id = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    let target = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    let world = engine.host_world_context_for_object(0);
    let caller = HostObjectContext {
        id: caller_id,
        ..idle_object_context()
    };
    let (result, _) = with_compat_context!(Some(caller), world, 3, || {
        with_host_context_mut((), |context| {
            assert!(context.ensure_object_scope(target));
            let scope = context.object_scope_mut(target).test_value();
            scope.set_owner(3);
            scope.set_controller(4);
            scope.set_fixed_velocity(FixedVec2::new(
                C4Fixed::from_raw(333),
                C4Fixed::from_raw(-444),
            ));
            scope.adjust_damage(17);
            scope.current_action_ticks = 23;
            scope.set_action_phase(7);
            scope.current_direction = Direction::Right;
        });
        crate::HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(|count| count.set(0));
        assert_eq!(get_owner(&[v_object(target)])?, v_int(3));
        assert_eq!(get_controller(&[v_object(target)])?, v_int(4));
        assert_eq!(get_damage(&[v_object(target)])?, v_int(17));
        assert_eq!(get_act_time(&[v_object(target)])?, v_int(23));
        assert_eq!(get_phase(&[v_object(target)])?, v_int(7));
        assert_eq!(get_dir(&[v_object(target)])?, v_int(1));
        // Precision 65536 exposes every raw fixed-point bit, including sign.
        assert_eq!(get_x_dir(&[v_object(target), v_int(65536)])?, v_int(333));
        assert_eq!(get_y_dir(&[v_object(target), v_int(65536)])?, v_int(-444));
        assert_eq!(crate::HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(Cell::get), 0);
        Ok::<_, RuntimeError>(())
    });
    result.test_value();
}

#[test]
fn foreign_controller_reads_do_not_materialize_full_state() {
    // FnGetController reads C4Object::Controller (C4Script.cpp:1316-1320).
    let mut engine = crate::Engine::new();
    engine.register_test_definition(test_definition(
        "TEST",
        "Test",
        "func Probe() { return 0; }",
    ));
    let caller_id = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    let target = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    engine.objects[1].state.controller = 7;
    let world = engine.host_world_context_for_object(0);
    let caller = HostObjectContext {
        id: caller_id,
        ..idle_object_context()
    };
    let (result, _) = with_compat_context!(Some(caller), world, 3, || {
        crate::SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(|count| count.set(0));
        crate::HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(|count| count.set(0));
        assert_eq!(get_controller(&[v_object(target)])?, v_int(7));
        assert_eq!(
            crate::SCRIPT_STATE_SNAPSHOT_MATERIALIZATIONS.with(Cell::get),
            0
        );
        assert_eq!(crate::HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(Cell::get), 0);
        Ok::<_, RuntimeError>(())
    });
    result.test_value();
}

#[test]
fn unchanged_layer_and_base_overlays_keep_shared_object_state() {
    let mut engine = crate::Engine::new();
    engine.register_test_definition(test_definition(
        "TEST",
        "Test",
        "func Probe() { return 0; }",
    ));
    let id = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
    let world = engine.host_world_context_for_object(0);
    let state = world
        .get_shared(id)
        .test_value()
        .full_state()
        .cloned()
        .test_value();
    let caller = HostObjectContext {
        id,
        ..idle_object_context()
    };
    let (result, _) = with_compat_context!(Some(caller), world, 2, || {
        with_host_context_mut((), |context| {
            let scope = context.object_scope_mut(id).test_value();
            scope.pending_update.layer = Some(state.layer);
            scope.pending_update.base = Some(state.base);
        });
        with_host_context((), |context| {
            let projected = context.get_world_object(id).test_value();
            assert!(
                Rc::ptr_eq(&state, projected.full_state().test_value()),
                "unchanged layer/base overlays must not copy the full state"
            );
        });
        Ok::<_, RuntimeError>(())
    });
    result.test_value();
}

#[test]
fn a_spawn_links_into_the_master_list_without_copying_the_objects_it_walks_past() {
    // C4ObjectList::Add reads only the Status, Unsorted, Category and id of
    // each link it passes before the insertion point (C4ObjectList.cpp:
    // 155-173). A StaticBack object walks past every higher category.
    let copies_walking_past = |walked: usize| {
        let mut engine = crate::Engine::new();
        let mut back = test_definition("BACK", "Static back", "");
        back.set_category(crate::CATEGORY_STATIC_BACK);
        engine.register_test_definition(back);
        let mut item = test_definition("ITEM", "Item", "");
        item.set_category(crate::CATEGORY_OBJECT);
        engine.register_test_definition(item);
        engine.register_test_definition(test_definition(
            "TEST",
            "Test",
            "func Probe() { return 0; }",
        ));
        let caller_id = engine.spawn_test_object(crate::SpawnConfig::new("TEST"));
        for _ in 0..walked {
            engine.spawn_test_object(crate::SpawnConfig::new("ITEM"));
        }
        let world = engine.host_world_context_for_object(0);
        let caller = HostObjectContext {
            id: caller_id,
            ..idle_object_context()
        };
        let next_object_id = walked as u64 + 2;
        let (result, _) = with_effect_context_with_state_and_spawn_previews(
            Some(caller),
            &[],
            world,
            next_object_id,
            false,
            || {
                crate::HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(|count| count.set(0));
                crate::HOST_WORLD_OBJECT_MATERIALIZATIONS.with(|count| count.set(0));
                create_object(&[v_id("BACK".into())])?;
                Ok::<_, RuntimeError>(
                    crate::HOST_WORLD_OBJECT_GET_DEEP_CLONES.with(Cell::get)
                        + crate::HOST_WORLD_OBJECT_MATERIALIZATIONS.with(Cell::get),
                )
            },
        );
        result.test_value()
    };
    assert_eq!(copies_walking_past(40), copies_walking_past(0));
}
