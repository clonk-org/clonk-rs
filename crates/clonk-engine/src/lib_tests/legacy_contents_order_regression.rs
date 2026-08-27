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
fn state_restore_preserves_runtime_contents_order() {
    // C4Object::Enter inserts with stContents (C4Object.cpp:1601), whose
    // same-category/id pass places a new link before the existing cluster
    // (C4ObjectList.cpp:147-175). Contents is then compiled in forward link
    // order and denumerated by tail-appending those saved links
    // (C4Object.cpp:2812; C4ObjectList.cpp:457-465,476-497).
    let container_definition = test_definition("CONT", "Container", "");
    let item_definition = test_definition("ITEM", "Item", "");
    let mut engine = Engine::new();
    engine.register_test_definition(container_definition.clone());
    engine.register_test_definition(item_definition.clone());

    let parent = spawn_fixture!(engine, "CONT", with_id: ObjectId::new(1));
    let first = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(2), with_container: parent);
    let second = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(3), with_container: parent);
    for child in [first, second] {
        engine
            .apply_container_change(child, Some(parent), None, false)
            .expect("runtime Exit removes the current link");
        engine
            .apply_container_change(child, None, Some(parent), false)
            .expect("runtime Enter allocates a replacement link");
    }
    let parent_index = engine.test_object_index(parent);
    assert_eq!(engine.objects[parent_index].state.contents, [second, first]);
    for child in [first, second] {
        assert_eq!(
            engine.objects[engine.test_object_index(child)]
                .state
                .contents_link_generation,
            2,
            "the source state has a later link incarnation to discard on load"
        );
    }

    let state = engine.capture_state();
    let mut restored = Engine::new();
    restored.register_test_definition(container_definition);
    restored.register_test_definition(item_definition);
    crate::TestValueExt::test_value(restored.restore_state(&state));

    let parent_index = restored.test_object_index(parent);
    assert_eq!(
        restored.objects[parent_index].state.contents,
        [second, first]
    );
    assert_eq!(
        restored.objects[restored.test_object_index(first)]
            .state
            .container,
        Some(parent)
    );
    assert_eq!(
        restored.objects[restored.test_object_index(second)]
            .state
            .container,
        Some(parent)
    );
    assert_eq!(
        restored.objects[restored.test_object_index(first)]
            .state
            .contents_link_generation,
        1,
        "a freshly denumerated link starts its first runtime incarnation"
    );
    assert_eq!(
        restored.objects[restored.test_object_index(second)]
            .state
            .contents_link_generation,
        1,
        "each restored child owns a distinct first link incarnation"
    );
}

#[test]
fn legacy_link_repair_sorts_omitted_contained_children() {
    // C4GameObjects::Load repairs a valid Contained pointer missing from the
    // parent's saved Contents with Add(stContents), not a tail append
    // (C4GameObjects.cpp:597-610). Equal category/id children therefore use
    // C4ObjectList::Add's newest-first cluster insertion
    // (C4ObjectList.cpp:147-175).
    let mut engine = Engine::new();
    engine.register_test_definition(test_definition("CONT", "Container", ""));
    engine.register_test_definition(test_definition("ITEM", "Item", ""));
    let parent = spawn_fixture!(engine, "CONT", with_id: ObjectId::new(1));
    let first = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(2));
    let omitted_a = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(3));
    let omitted_b = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(4));
    // C4GameObjects::Load walks the forward main list; Engine::exec_list is
    // its reverse representation. Two omitted equal-key children make that
    // traversal observable because each Add(stContents) prepends its cluster.
    engine.execution.exec_list = vec![parent, first, omitted_a, omitted_b];

    engine.restore_legacy_object_links(
        &[(first, parent), (omitted_a, parent), (omitted_b, parent)],
        &[(parent, vec![first])],
    );

    let parent_index = engine.test_object_index(parent);
    assert_eq!(
        engine.objects[parent_index].state.contents,
        [omitted_a, omitted_b, first]
    );
}

#[test]
fn legacy_link_repair_interleaves_contained_and_contents_in_master_order() {
    // C4GameObjects::Load repairs each object's missing Contained link before
    // retargeting the children in that same object's Contents list, all in one
    // forward master-list walk (C4GameObjects.cpp:597-631). A later parent's
    // saved Contents may therefore retarget a child without removing the link
    // that an earlier Contained repair just inserted.
    let mut engine = Engine::new();
    engine.register_test_definition(test_definition("CONT", "Container", ""));
    engine.register_test_definition(test_definition("ITEM", "Item", ""));
    let child = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(1));
    let first_parent = spawn_fixture!(engine, "CONT", with_id: ObjectId::new(2));
    let later_parent = spawn_fixture!(engine, "CONT", with_id: ObjectId::new(3));
    // Native forward master order is child, first_parent, later_parent.
    engine.execution.exec_list = vec![later_parent, first_parent, child];

    engine.restore_legacy_object_links(
        &[(child, first_parent)],
        &[(first_parent, Vec::new()), (later_parent, vec![child])],
    );

    let child_index = engine.test_object_index(child);
    let first_parent_index = engine.test_object_index(first_parent);
    let later_parent_index = engine.test_object_index(later_parent);
    assert_eq!(
        engine.objects[child_index].state.container,
        Some(later_parent)
    );
    assert_eq!(engine.objects[first_parent_index].state.contents, [child]);
    assert_eq!(engine.objects[later_parent_index].state.contents, [child]);
}

#[test]
fn legacy_link_repair_deduplicates_when_each_parent_is_visited() {
    // Missing Contained links are inserted before C++ reaches that parent and
    // removes earlier duplicate Contents links (C4GameObjects.cpp:605-631).
    // Pre-deduplicating [D,Y,D] would put X after Y; native Add(stContents)
    // first inserts same-definition X before the first D, then duplicate
    // repair keeps the final D and leaves [X,Y,D].
    let mut engine = Engine::new();
    engine.register_test_definition(test_definition("CONT", "Container", ""));
    let mut item_definition = test_definition("ITEM", "Item", "");
    item_definition.set_category(CATEGORY_OBJECT);
    engine.register_test_definition(item_definition);
    let mut other_definition = test_definition("OTHR", "Other", "");
    other_definition.set_category(CATEGORY_OBJECT);
    engine.register_test_definition(other_definition);
    let inserted = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(1));
    let parent = spawn_fixture!(engine, "CONT", with_id: ObjectId::new(2));
    let duplicate = spawn_fixture!(engine, "ITEM", with_id: ObjectId::new(3));
    let middle = spawn_fixture!(engine, "OTHR", with_id: ObjectId::new(4));
    // Native forward master order is inserted, parent, duplicate, middle.
    engine.execution.exec_list = vec![middle, duplicate, parent, inserted];

    engine.restore_legacy_object_links(
        &[(inserted, parent)],
        &[(parent, vec![duplicate, middle, duplicate])],
    );

    let parent_index = engine.test_object_index(parent);
    assert_eq!(
        engine.objects[parent_index].state.contents,
        [inserted, middle, duplicate]
    );
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
fn state_restore_preserves_mutual_containment_cycles() {
    // Objects.txt compiles Contained as an enumerated pointer and resolves all
    // pointers only after every object exists (C4Object.cpp:2914-2924;
    // C4GameObjects.cpp:597-610). The raw two-phase relink therefore accepts
    // a mutual cycle even though runtime Enter rejects one.
    let definition = test_definition("CYCL", "Cycle", "");
    let mut engine = Engine::new();
    engine.register_test_definition(definition.clone());
    let first = spawn_fixture!(engine, "CYCL", with_id: ObjectId::new(1));
    let second = spawn_fixture!(engine, "CYCL", with_id: ObjectId::new(2));
    engine.restore_legacy_object_links(&[(first, second), (second, first)], &[]);
    let state = engine.capture_state();

    let mut restored = Engine::new();
    restored.register_test_definition(definition);
    restored
        .restore_state(&state)
        .expect("compiled containment cycles denumerate without runtime Enter validation");

    let first_state = &restored.objects[restored.test_object_index(first)].state;
    let second_state = &restored.objects[restored.test_object_index(second)].state;
    assert_eq!(first_state.container, Some(second));
    assert_eq!(second_state.container, Some(first));
    assert_eq!(first_state.contents, [second]);
    assert_eq!(second_state.contents, [first]);
}

#[test]
fn state_restore_preserves_self_containment() {
    // C4Object::DenumeratePointers resolves the compiled Contained pointer and
    // Contents list without runtime Enter's self/cycle guards
    // (C4Object.cpp:2914-2924; C4GameObjects.cpp:597-610).
    let definition = test_definition("CYCL", "Cycle", "");
    let mut source = Engine::new();
    source.register_test_definition(definition.clone());
    let object = spawn_fixture!(source, "CYCL", with_id: ObjectId::new(1));
    let mut state = source.capture_state();
    let snapshot = &mut state.objects[0].snapshot;
    snapshot.container = Some(object);
    snapshot.contents = vec![object];

    let mut restored = Engine::new();
    restored.register_test_definition(definition);
    restored
        .restore_state(&state)
        .expect("compiled self-containment denumerates without runtime Enter validation");

    let restored_object = &restored.objects[restored.test_object_index(object)];
    assert_eq!(restored_object.state.container, Some(restored_object.id));
    assert_eq!(restored_object.state.contents, [restored_object.id]);
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
