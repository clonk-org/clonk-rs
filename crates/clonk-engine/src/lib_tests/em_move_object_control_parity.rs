use super::*;

fn number(id: ObjectId) -> i32 {
    crate::TestValueExt::test_value(i32::try_from(id.as_u64()))
}

fn control(action: u8, objects: Vec<i32>) -> EmMoveObjectControlData {
    EmMoveObjectControlData {
        action,
        tx: 0,
        ty: 0,
        target_object: -1,
        objects,
        strictness: ScriptStrictness::Strict3,
        script: LegacyCString::default(),
        by_client: 0,
    }
}

fn register_definition(engine: &mut Engine, id: &str, script: &str) {
    crate::TestValueExt::test_value(engine.register_script_definition(id, id, script));
}

#[test]
fn em_move_object_move_uses_live_array_order_and_league_gate() {
    let mut engine = Engine::new();
    register_definition(&mut engine, "MOVE", "");
    let active = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("MOVE")
                .with_position(Vector2::new(10, 20))
                .with_velocity(Vector2::new(3, -4))
                .with_mobile(true),
        ),
    );
    let inactive = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("MOVE")
                .with_position(Vector2::new(1, 2))
                .with_velocity(Vector2::new(-2, 5))
                .with_mobile(true)
                .with_status(ObjectStatus::Inactive),
        ),
    );
    let mut packet = control(
        EMMO_MOVE,
        vec![
            number(active),
            number(active),
            number(inactive),
            -1,
            999_999,
        ],
    );
    packet.tx = 2;
    packet.ty = -3;

    assert!(engine
        .execute_em_move_object_control(&packet, ScriptControlPolicy::live(false))
        .expect("move packet executes"));
    let active_state =
        &engine.objects[crate::TestValueExt::test_value(engine.find_object_index(active))];
    assert_eq!(active_state.state.position, Vector2::new(14, 14));
    assert_eq!(active_state.fixed_position, FixedVec2::from_ints(14, 14));
    assert_eq!(active_state.state.velocity, Vector2::ZERO);
    assert_eq!(active_state.fixed_velocity, FixedVec2::ZERO);
    assert!(!active_state.state.mobile);
    let inactive_state =
        &engine.objects[crate::TestValueExt::test_value(engine.find_object_index(inactive))];
    assert_eq!(inactive_state.state.position, Vector2::new(3, -1));
    assert_eq!(inactive_state.fixed_velocity, FixedVec2::ZERO);
    assert!(!inactive_state.state.mobile);

    let before = engine.objects[crate::TestValueExt::test_value(engine.find_object_index(active))]
        .state
        .position;
    engine.set_league_game(true);
    assert!(!engine
        .execute_em_move_object_control(&packet, ScriptControlPolicy::live(false))
        .expect("league packet is ignored"));
    assert_eq!(
        engine.objects[engine.find_object_index(active).unwrap()]
            .state
            .position,
        before
    );
}

#[test]
fn em_move_object_duplicate_uses_create_object_lifecycle_not_state_clone() {
    let mut engine = Engine::new();
    register_definition(&mut engine, "LAYR", "");
    register_definition(
        &mut engine,
            "DUPL",
            "#strict 3\nstatic Made;\nfunc Construction(creator) { if (Made == nil) Made = 0; Made = Made + 1; return true; }\nfunc ReadMade() { return Made; }",
    );
    let layer = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("LAYR")));
    let source = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("DUPL")
                .with_position(Vector2::new(23, 41))
                .with_velocity(Vector2::new(7, -5))
                .with_rotation(67)
                .with_owner(4)
                .with_layer(layer),
        ),
    );
    let source_index = crate::TestValueExt::test_value(engine.find_object_index(source));
    assert_eq!(
        engine
            .call_object_function(source_index, "ReadMade", Vec::new())
            .expect("creation count reads"),
        Value::Int(1)
    );

    assert!(engine
        .execute_em_move_object_control(
            &control(EMMO_DUPLICATE, vec![number(source), number(source)]),
            ScriptControlPolicy::live(false),
        )
        .expect("duplicate packet executes"));

    let duplicates = engine
        .objects
        .iter()
        .filter(|object| object.definition_id == "DUPL" && object.id != source)
        .collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 2);
    for duplicate in duplicates {
        assert_eq!(duplicate.state.position, Vector2::new(23, 41));
        assert_eq!(duplicate.state.owner, 4);
        assert_eq!(duplicate.state.rotation, 0);
        assert_eq!(duplicate.fixed_velocity, FixedVec2::ZERO);
        assert_eq!(duplicate.state.layer, Some(layer));
        assert_eq!(duplicate.state.construction, FULL_CON);
    }
    let source_index = crate::TestValueExt::test_value(engine.find_object_index(source));
    assert_eq!(
        engine
            .call_object_function(source_index, "ReadMade", Vec::new())
            .expect("ordered lifecycle count reads"),
        Value::Int(3)
    );
}

#[test]
fn em_move_object_enter_exit_and_remove_use_native_object_paths() {
    let mut engine = Engine::new();
    register_definition(&mut engine, "CONT", "");
    register_definition(&mut engine, "ITEM", "");
    let container = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("CONT")
                .with_position(Vector2::new(50, 60))
                .with_velocity(Vector2::new(2, 3)),
        ),
    );
    let child = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("ITEM")
                .with_position(Vector2::new(4, 5))
                .with_velocity(Vector2::new(8, -9))
                .with_rotation(27),
        ),
    );
    let mut enter = control(EMMO_ENTER, vec![number(child)]);
    enter.target_object = number(container);
    assert!(engine
        .execute_em_move_object_control(&enter, ScriptControlPolicy::live(false))
        .expect("enter executes"));
    let child_index = crate::TestValueExt::test_value(engine.find_object_index(child));
    assert_eq!(engine.objects[child_index].state.container, Some(container));
    assert_eq!(
        engine.objects[child_index].state.position,
        Vector2::new(50, 60)
    );

    assert!(engine
        .execute_em_move_object_control(
            &control(EMMO_EXIT, vec![number(child)]),
            ScriptControlPolicy::live(false),
        )
        .expect("exit executes"));
    let child_index = crate::TestValueExt::test_value(engine.find_object_index(child));
    assert_eq!(engine.objects[child_index].state.container, None);
    // The fixture definition declares no `Rotate`, and `C4Def::Rotateable`
    // defaults to 0 (C4Def.cpp:156,376), so `C4Object::Init` drops the
    // requested rotation before the object ever exists — Exit's `0, 0, 0`
    // then has nothing left to clear.
    assert_eq!(engine.objects[child_index].state.rotation, 0);
    assert_eq!(engine.objects[child_index].fixed_velocity, FixedVec2::ZERO);
    assert_eq!(engine.objects[child_index].rotation_velocity, C4Fixed::ZERO);
    assert!(engine.objects[child_index].state.mobile);

    let content = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("ITEM").with_container(child)),
    );
    assert!(engine
        .execute_em_move_object_control(
            &control(EMMO_REMOVE, vec![number(child), number(child)]),
            ScriptControlPolicy::live(false),
        )
        .expect("remove executes"));
    assert_eq!(
        engine.objects[engine.find_object_index(child).unwrap()]
            .state
            .status,
        ObjectStatus::Deleted
    );
    assert_eq!(
        engine.objects[engine.find_object_index(content).unwrap()]
            .state
            .status,
        ObjectStatus::Deleted
    );
}

#[test]
fn em_move_object_script_preserves_raw_targets_fallbacks_and_policy() {
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_global_scripts(&[(
                "EMGlobal.c".to_string(),
                "static GlobalMarks, Unset; global func Mark() { if (GlobalMarks == Unset) GlobalMarks = 0; GlobalMarks = GlobalMarks + 1; return true; }".to_string(),
        )]),
        1
    );
    register_definition(
        &mut engine,
            "SCRP",
            "#strict 3\nlocal Marks;\nfunc Mark() { if (Marks == nil) Marks = 0; Marks = Marks + 1; return true; }\nfunc ReadMarks() { return Marks; }",
    );
    let object = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("SCRP")));
    let mut packet = control(EMMO_SCRIPT, vec![number(object), 999_999, number(object)]);
    packet.script = crate::TestValueExt::test_value(LegacyCString::from_bytes(b"Mark()".to_vec()));
    assert!(engine
        .execute_em_move_object_control(&packet, ScriptControlPolicy::replay(false))
        .expect("host-authored replay script executes"));
    let index = crate::TestValueExt::test_value(engine.find_object_index(object));
    assert_eq!(
        engine
            .call_object_function(index, "ReadMarks", Vec::new())
            .expect("object marks read"),
        Value::Int(2)
    );
    let global =
        crate::TestValueExt::test_value(engine.script_globals.borrow().get("GlobalMarks").cloned());
    assert_eq!(*global.borrow(), Value::Int(1));

    packet.by_client = 7;
    assert!(engine
        .execute_em_move_object_control(&packet, ScriptControlPolicy::replay(false))
        .expect("outer packet remains accepted"));
    assert_eq!(*global.borrow(), Value::Int(1));
    let index = crate::TestValueExt::test_value(engine.find_object_index(object));
    assert_eq!(
        engine
            .call_object_function(index, "ReadMarks", Vec::new())
            .expect("suppressed marks read"),
        Value::Int(2)
    );
}
