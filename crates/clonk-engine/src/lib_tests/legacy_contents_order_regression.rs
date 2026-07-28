use super::*;

#[test]
fn native_compiled_defaults_are_distinct_from_generic_loaded_fixtures() {
    let mut engine = Engine::new();
    let mut definition =
        Definition::from_script("STAT", "Static", "").expect("fixture definition compiles");
    definition.set_category(CATEGORY_STATIC_BACK);
    definition.set_mass(100);
    definition.set_physical(PhysicalInfo {
        energy: 50_000,
        breath: 30_000,
        ..PhysicalInfo::default()
    });
    engine
        .register_definition(definition)
        .expect("fixture definition registers");

    let fixed_velocity = FixedVec2::from_ints(7, -5);
    let object = engine
        .spawn_object(
            SpawnConfig::new("STAT")
                .with_loaded(true)
                .with_native_compiled_object_defaults()
                .with_alive(true)
                .with_velocity(Vector2::new(7, -5))
                .with_fixed_velocity(fixed_velocity),
        )
        .expect("loaded object spawns");
    let index = engine.find_object_index(object).expect("object exists");
    assert_eq!(engine.objects[index].state.category, 0);
    assert_eq!(engine.objects[index].state.energy, 0);
    assert_eq!(engine.objects[index].state.breath, 0);
    assert_eq!(engine.objects[index].compiled_mass, Some(0));
    assert_eq!(engine.objects[index].fixed_velocity, fixed_velocity);

    let fixture = engine
        .spawn_object(SpawnConfig::new("STAT").with_loaded(true).with_alive(true))
        .expect("generic loaded fixture spawns");
    let fixture = engine.find_object_index(fixture).expect("fixture exists");
    assert_eq!(engine.objects[fixture].state.category, CATEGORY_STATIC_BACK);
    assert_eq!(engine.objects[fixture].state.energy, 50_000);
    assert_eq!(engine.objects[fixture].state.breath, 30_000);
    assert_eq!(engine.objects[fixture].compiled_mass, None);
}

#[test]
fn compiled_contents_keep_saved_order_and_cpp_duplicate_repair() {
    let mut engine = Engine::new();
    for id in ["CONT", "ITEM"] {
        engine.register_script_definition(id, id, "").expect("fixture definition registers");
    }
    let parent = engine
        .spawn_object(SpawnConfig::new("CONT").with_id(ObjectId::new(1)))
        .expect("container spawns");
    let first = engine
        .spawn_object(
            SpawnConfig::new("ITEM")
                .with_id(ObjectId::new(2))
                .with_container(parent),
        )
        .expect("first content spawns");
    let second = engine
        .spawn_object(
            SpawnConfig::new("ITEM")
                .with_id(ObjectId::new(3))
                .with_container(parent),
        )
        .expect("second content spawns");

    engine.restore_legacy_contents_order(&[(parent, vec![second, first])]);
    let parent_index = engine.find_object_index(parent).expect("container exists");
    assert_eq!(engine.objects[parent_index].state.contents, [second, first]);

    // C4GameObjects::Load removes the earlier link when it encounters a
    // duplicate, leaving the final occurrence in its saved position.
    engine.restore_legacy_contents_order(&[(parent, vec![second, first, second])]);
    assert_eq!(engine.objects[parent_index].state.contents, [first, second]);
}

#[test]
fn deferred_legacy_containment_preserves_mutual_cycles() {
    let mut engine = Engine::new();
    engine.register_script_definition("CYCL", "Cycle", "").expect("fixture definition registers");
    let first = engine
        .spawn_object(SpawnConfig::new("CYCL").with_id(ObjectId::new(1)))
        .expect("first object spawns");
    let second = engine
        .spawn_object(SpawnConfig::new("CYCL").with_id(ObjectId::new(2)))
        .expect("second object spawns");

    engine.restore_legacy_object_links(&[(first, second), (second, first)], &[]);

    let first_state = &engine.objects[engine.find_object_index(first).unwrap()].state;
    let second_state = &engine.objects[engine.find_object_index(second).unwrap()].state;
    assert_eq!(first_state.container, Some(second));
    assert_eq!(second_state.container, Some(first));
    assert_eq!(first_state.contents, [second]);
    assert_eq!(second_state.contents, [first]);
}

#[test]
fn loaded_mass_cache_survives_subpercent_docon_until_update_mass() {
    let mut engine = Engine::new();
    let mut definition =
        Definition::from_script("MASS", "Mass", "").expect("fixture definition compiles");
    definition.set_mass(100);
    engine
        .register_definition(definition)
        .expect("fixture definition registers");

    let construction = FULL_CON / 2 + 10;
    let mut config = SpawnConfig::new("MASS")
        .with_loaded(true)
        .with_construction(construction);
    config.compiled_mass = Some(777);
    let id = engine.spawn_object(config).expect("loaded object spawns");
    let index = engine.find_object_index(id).expect("object exists");
    assert_eq!(engine.effective_object_mass(index), 777);

    engine.do_con(index, 1).expect("subpercent DoCon succeeds");
    assert_eq!(engine.effective_object_mass(index), 777);

    engine
        .do_con(index, FULL_CON / 100)
        .expect("percent-crossing DoCon succeeds");
    let expected = (100 * engine.objects[index].state.construction / FULL_CON).max(1);
    assert_eq!(engine.effective_object_mass(index), expected);
}

#[test]
fn effective_object_mass_has_no_nesting_depth_cutoff() {
    let mut engine = Engine::new();
    let mut mass = Definition::from_script(
        "MASS",
        "Mass",
        "#strict\npublic func TryEnter(target) { return Enter(target); }\n",
    )
    .expect("mass definition compiles");
    mass.set_mass(10);
    engine
        .register_definition(mass)
        .expect("mass definition registers");

    let mut no_component = Definition::from_script("NCMP", "No component mass", "")
        .expect("NoComponentMass definition compiles");
    no_component.set_mass(7);
    no_component.set_no_component_mass(true);
    engine
        .register_definition(no_component)
        .expect("NoComponentMass definition registers");

    let mut hidden = Definition::from_script("HEAV", "Hidden cargo", "")
        .expect("hidden-cargo definition compiles");
    hidden.set_mass(1_000);
    engine
        .register_definition(hidden)
        .expect("hidden-cargo definition registers");

    let root = engine
        .spawn_object(SpawnConfig::new("MASS"))
        .expect("root object spawns");
    let mut chain = vec![root];
    let mut expected_mass = 10;
    for depth in 1..=12 {
        let construction = if depth == 5 { FULL_CON / 2 } else { FULL_CON };
        let mut config = SpawnConfig::new("MASS")
            .with_container(*chain.last().expect("chain has a parent"))
            .with_construction(construction);
        config.own_mass = Some(depth);
        let child = engine.spawn_object(config).expect("nested object spawns");
        chain.push(child);
        expected_mass += ((10 + depth) * construction / FULL_CON).max(1);
    }

    let no_component = engine
        .spawn_object(SpawnConfig::new("NCMP").with_container(root))
        .expect("NoComponentMass object spawns");
    engine
        .spawn_object(SpawnConfig::new("HEAV").with_container(no_component))
        .expect("hidden nested cargo spawns");
    expected_mass += 7;

    let no_component_index = engine
        .find_object_index(no_component)
        .expect("NoComponentMass object exists");
    assert_eq!(engine.effective_object_mass(no_component_index), 7);
    let root_index = engine.find_object_index(root).expect("root object exists");
    assert_eq!(engine.effective_object_mass(root_index), expected_mass);

    let deepest = *chain.last().expect("chain has a deepest object");
    let root_before = engine.object_snapshot(root).expect("root snapshot exists");
    let deepest_before = engine
        .object_snapshot(deepest)
        .expect("deepest snapshot exists");
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
    let root_after = engine.object_snapshot(root).expect("root remains");
    let deepest_after = engine.object_snapshot(deepest).expect("deepest remains");
    assert_eq!(root_after.container, root_before.container);
    assert_eq!(root_after.contents, root_before.contents);
    assert_eq!(deepest_after.container, deepest_before.container);
    assert_eq!(deepest_after.contents, deepest_before.contents);
    assert_eq!(engine.effective_object_mass(root_index), expected_mass);
}

#[test]
fn inactive_contents_count_while_deleted_holes_do_not_like_cpp() {
    let mut engine = Engine::new();
    let mut carrier =
        Definition::from_script("MCAR", "Mass carrier", "").expect("carrier compiles");
    carrier.set_mass(100);
    carrier.set_collection_rect(Some(DefinitionRect::new(-5, -5, 10, 10)));
    carrier.set_collection_limit(2);
    let mut item = Definition::from_script("MITM", "Mass item", "").expect("item compiles");
    item.set_mass(25);
    engine
        .register_definition(carrier)
        .expect("carrier registers");
    engine.register_definition(item).expect("item registers");

    let carrier = engine
        .spawn_object(SpawnConfig::new("MCAR"))
        .expect("carrier spawns");
    let deleted = engine
        .spawn_object(SpawnConfig::new("MITM").with_container(carrier))
        .expect("deleted item spawns");
    let inactive = engine
        .spawn_object(SpawnConfig::new("MITM").with_container(carrier))
        .expect("inactive item spawns");
    let deleted_index = engine
        .find_object_index(deleted)
        .expect("deleted item exists");
    let _ = engine.objects[deleted_index].mark_destroyed();
    let inactive_index = engine
        .find_object_index(inactive)
        .expect("inactive item exists");
    engine.objects[inactive_index].state.status = ObjectStatus::Inactive;
    let carrier_index = engine.find_object_index(carrier).expect("carrier exists");
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
    let mut definition =
        Definition::from_script("CLNK", "Clonk", "").expect("fixture definition compiles");
    definition.set_crew_member(true);
    engine
        .register_definition(definition)
        .expect("fixture definition registers");

    let object = engine
        .spawn_object(
            SpawnConfig::new("CLNK")
                .with_id(ObjectId::new(17))
                .with_owner(0)
                .with_crew_member(false)
                .with_alive(true)
                .with_loaded(true),
        )
        .expect("loaded object spawns");
    engine
        .register_player(PlayerConfig::new(0, "Player"))
        .expect("restored player registers");
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

    engine
        .initialize_scenario_script()
        .expect("InitGameFinal AssignInfo succeeds");

    let info = engine
        .crew_object_info(object)
        .expect("named roster info attaches");
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
    let object = &engine.objects[engine.find_object_index(object).unwrap()];
    assert_eq!(object.state.controller, 0);
    assert_eq!(object.state.plr_view_range, 500);
    assert_eq!(object.state.info_physical, Some(physical));
}
