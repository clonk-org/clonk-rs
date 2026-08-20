use super::*;
use crate::lib_test_support::{register_fixture, spawn_fixture, EngineTestExt};

#[test]
fn native_compiled_defaults_are_distinct_from_generic_loaded_fixtures() {
    let mut engine = Engine::new();
    register_fixture!(
        engine,
        "STAT",
        "Static",
        "",
        set_category(CATEGORY_STATIC_BACK),
        set_mass(100),
        set_physical(PhysicalInfo {
            energy: 50_000,
            breath: 30_000,
            ..PhysicalInfo::default()
        })
    );

    let fixed_velocity = FixedVec2::from_ints(7, -5);
    let object = engine.spawn_test_object(
        SpawnConfig::new("STAT")
            .with_loaded(true)
            .with_native_compiled_object_defaults()
            .with_alive(true)
            .with_velocity(Vector2::new(7, -5))
            .with_fixed_velocity(fixed_velocity),
    );
    let index = engine.test_object_index(object);
    assert_eq!(engine.objects[index].state.category, 0);
    assert_eq!(engine.objects[index].state.energy, 0);
    assert_eq!(engine.objects[index].state.breath, 0);
    assert_eq!(engine.objects[index].compiled_mass, Some(0));
    assert_eq!(engine.objects[index].fixed_velocity, fixed_velocity);

    let fixture = spawn_fixture!(engine, "STAT", with_loaded: true, with_alive: true);
    let fixture = engine.test_object_index(fixture);
    assert_eq!(engine.objects[fixture].state.category, CATEGORY_STATIC_BACK);
    assert_eq!(engine.objects[fixture].state.energy, 50_000);
    assert_eq!(engine.objects[fixture].state.breath, 30_000);
    assert_eq!(engine.objects[fixture].compiled_mass, None);
}

#[test]
fn compiled_contents_keep_saved_order_and_cpp_duplicate_repair() {
    let mut engine = Engine::new();
    for id in ["CONT", "ITEM"] {
        crate::TestValueExt::test_value(engine.register_script_definition(id, id, ""));
    }
    let parent = spawn_fixture!(engine, "CONT", with_id: ObjectId::new(1));
    let first = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(2), with_container: parent);
    let second = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(3), with_container: parent);

    engine.restore_legacy_contents_order(&[(parent, vec![second, first])]);
    let parent_index = engine.test_object_index(parent);
    assert_eq!(engine.objects[parent_index].state.contents, [second, first]);

    // C4GameObjects::Load removes the earlier link when it encounters a
    // duplicate, leaving the final occurrence in its saved position.
    engine.restore_legacy_contents_order(&[(parent, vec![second, first, second])]);
    assert_eq!(engine.objects[parent_index].state.contents, [first, second]);
}

#[test]
fn deferred_legacy_containment_preserves_mutual_cycles() {
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_script_definition("CYCL", "Cycle", ""));
    let first = spawn_fixture!(engine, "CYCL", with_id: ObjectId::new(1));
    let second = spawn_fixture!(engine, "CYCL", with_id: ObjectId::new(2));

    engine.restore_legacy_object_links(&[(first, second), (second, first)], &[]);

    let first_state = &engine.objects[engine.test_object_index(first)].state;
    let second_state = &engine.objects[engine.test_object_index(second)].state;
    assert_eq!(first_state.container, Some(second));
    assert_eq!(second_state.container, Some(first));
    assert_eq!(first_state.contents, [second]);
    assert_eq!(second_state.contents, [first]);
}

#[test]
fn loaded_mass_cache_survives_subpercent_docon_until_update_mass() {
    let mut engine = Engine::new();
    register_fixture!(engine, "MASS", "Mass", "", set_mass(100));

    let construction = FULL_CON / 2 + 10;
    let mut config = SpawnConfig::new("MASS")
        .with_loaded(true)
        .with_construction(construction);
    config.compiled_mass = Some(777);
    let id = engine.spawn_test_object(config);
    let index = engine.test_object_index(id);
    assert_eq!(engine.effective_object_mass(index), 777);

    crate::TestValueExt::test_value(engine.do_con(index, 1));
    assert_eq!(engine.effective_object_mass(index), 777);

    crate::TestValueExt::test_value(engine.do_con(index, FULL_CON / 100));
    let expected = (100 * engine.objects[index].state.construction / FULL_CON).max(1);
    assert_eq!(engine.effective_object_mass(index), expected);
}

#[test]
fn effective_object_mass_has_no_nesting_depth_cutoff() {
    let mut engine = Engine::new();
    register_fixture!(
        engine,
        "MASS",
        "Mass",
        "#strict\npublic func TryEnter(target) { return Enter(target); }\n",
        set_mass(10)
    );

    register_fixture!(
        engine,
        "NCMP",
        "No component mass",
        "",
        set_mass(7),
        set_no_component_mass(true)
    );

    register_fixture!(engine, "HEAV", "Hidden cargo", "", set_mass(1_000));

    let root = spawn_fixture!(engine, "MASS");
    let mut chain = vec![root];
    let mut expected_mass = 10;
    for depth in 1..=12 {
        let construction = if depth == 5 { FULL_CON / 2 } else { FULL_CON };
        let mut config = SpawnConfig::new("MASS")
            .with_container(*crate::TestValueExt::test_value(chain.last()))
            .with_construction(construction);
        config.own_mass = Some(depth);
        let child = engine.spawn_test_object(config);
        chain.push(child);
        expected_mass += ((10 + depth) * construction / FULL_CON).max(1);
    }

    let no_component = spawn_fixture!(engine, "NCMP", with_container: root);
    spawn_fixture!(engine, "HEAV", with_container: no_component);
    expected_mass += 7;

    let no_component_index = engine.test_object_index(no_component);
    assert_eq!(engine.effective_object_mass(no_component_index), 7);
    let root_index = engine.test_object_index(root);
    assert_eq!(engine.effective_object_mass(root_index), expected_mass);

    let deepest = *crate::TestValueExt::test_value(chain.last());
    let root_before = crate::TestValueExt::test_value(engine.object_snapshot(root));
    let deepest_before = crate::TestValueExt::test_value(engine.object_snapshot(deepest));
    assert_eq!(
        engine
            .call_object_function(
                root_index,
                "TryEnter",
                vec![Value::Object(deepest.as_u64())],
            )
            .expect("cycle attempt returns normally"),
        Value::Bool(false)
    );
    let root_after = crate::TestValueExt::test_value(engine.object_snapshot(root));
    let deepest_after = crate::TestValueExt::test_value(engine.object_snapshot(deepest));
    assert_eq!(root_after.container, root_before.container);
    assert_eq!(root_after.contents, root_before.contents);
    assert_eq!(deepest_after.container, deepest_before.container);
    assert_eq!(deepest_after.contents, deepest_before.contents);
    assert_eq!(engine.effective_object_mass(root_index), expected_mass);
}

#[test]
fn inactive_contents_count_while_deleted_holes_do_not_like_cpp() {
    let mut engine = Engine::new();
    let mut carrier = test_definition("MCAR", "Mass carrier", "");
    carrier.set_mass(100);
    carrier.set_collection_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
    carrier.set_collection_limit(2);
    let mut item = test_definition("MITM", "Mass item", "");
    item.set_mass(25);
    engine.register_test_definition(carrier);
    engine.register_test_definition(item);

    let carrier = spawn_fixture!(engine, "MCAR");
    let deleted = spawn_fixture!(engine, "MITM", with_container: carrier);
    let inactive = spawn_fixture!(engine, "MITM", with_container: carrier);
    let deleted_index = engine.test_object_index(deleted);
    let _ = engine.objects[deleted_index].mark_destroyed();
    let inactive_index = engine.test_object_index(inactive);
    engine.objects[inactive_index].state.status = ObjectStatus::Inactive;
    let carrier_index = engine.test_object_index(carrier);
    engine.objects[carrier_index].state.contents = vec![deleted, inactive];

    engine.refresh_object_ocf(carrier_index);
    assert_ne!(
        engine.objects[carrier_index].state.ocf & ocf::COLLECTION,
        0,
        "ObjectCount sees one retained entry, not the deleted list hole"
    );
    assert_eq!(
        engine.effective_object_mass(carrier_index),
        125,
        "MassCount includes inactive contents and skips Status==0"
    );
}

#[test]
fn objects_info_name_binds_the_named_idle_crew_entry_after_players_exist() {
    let mut engine = Engine::new();
    register_fixture!(engine, "CLNK", "Clonk", "", set_crew_member(true));

    let object = spawn_fixture!(engine, "CLNK", with_id: ObjectId::new(17), with_owner: 0, with_crew_member: false, with_alive: true, with_loaded: true);
    crate::TestValueExt::test_value(engine.register_player(PlayerConfig::new(0, "Player")));
    let physical = PhysicalInfo {
        energy: 77_000,
        ..PhysicalInfo::default()
    };
    engine.crew_rosters.insert(
        0,
        vec![player_file::CrewInfo {
            id: "OTHR".to_string(),
            name: "Captain".to_string(),
            rank: 3,
            rank_name: "Captain".to_string(),
            experience: 123,
            physical,
            ..Default::default()
        }],
    );
    engine.crew_info_order.insert(0, vec![0]);
    engine.remember_legacy_object_info(object, Some("captain".to_string()));

    crate::TestValueExt::test_value(engine.initialize_scenario_script());

    let info = crate::TestValueExt::test_value(engine.crew_object_info(object));
    assert_eq!(info.name, "Captain");
    assert_eq!(info.definition_id.as_str(), "OTHR");
    assert_eq!(engine.player(0).unwrap().crew(), [object]);
    assert_eq!(
        engine.crew_info_links.get(&object),
        Some(&CrewInfoLink {
            player_id: 0,
            roster_index: 0,
        })
    );
    let object = &engine.objects[engine.test_object_index(object)];
    assert_eq!(object.state.controller, 0);
    assert_eq!(object.state.plr_view_range, 500);
    assert_eq!(object.state.info_physical, Some(physical));
}
