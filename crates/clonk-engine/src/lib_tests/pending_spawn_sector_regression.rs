use super::*;

#[test]
fn construction_adds_pending_object_without_reordering_unrelated_sector_links() {
    // C4Game::NewObject adds the fresh object to Game.Objects and its sectors
    // before Construction (oracle-src-pinned src/C4Game.cpp:1121-1142;
    // src/C4GameObjects.cpp:54-70; src/C4Sector.cpp:88-101). That targeted
    // Add leaves unrelated physical sector lists untouched, even when their
    // order differs from stMain, and bounded FindObjects walks those lists
    // verbatim (src/C4FindObject.cpp:310-355).
    let pending_script = r#"#strict
local pending_count, ordered;
func Construction()
{
    pending_count = GetLength(FindObjects(
        [C4FO_InRect, 100, 0, 50, 100],
        [C4FO_ID, PEND]));
    ordered = FindObjects(
        [C4FO_InRect, 0, 0, 50, 100],
        [C4FO_ID, ORDR]);
    return true;
}
"#;
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 100));
    engine
        .register_script_definition("ORDR", "Ordered candidate", "#strict\n")
        .expect("candidate definition registers");
    engine
        .register_script_definition("PEND", "Pending object", pending_script)
        .expect("pending definition registers");
    let older = engine
        .spawn_object(SpawnConfig::new("ORDR").with_position(Vector2::new(10, 10)))
        .expect("older candidate spawns");
    let newer = engine
        .spawn_object(SpawnConfig::new("ORDR").with_position(Vector2::new(20, 10)))
        .expect("newer candidate spawns");

    let older_index = engine.find_object_index(older).expect("older exists");
    let newer_index = engine.find_object_index(newer).expect("newer exists");
    engine.objects[older_index].state.category = CATEGORY_OBJECT;
    engine.objects[newer_index].state.category = CATEGORY_STRUCTURE;
    engine
        .pending_object_order_commands
        .push(ObjectOrderCommand::SortByCategory);
    engine.execute_object_order_commands();

    let pending_id = engine
        .spawn_object(SpawnConfig::new("PEND").with_position(Vector2::new(110, 10)))
        .expect("pending object spawns");
    let pending_index = engine
        .find_object_index(pending_id)
        .expect("pending object materializes");
    let pending = &engine.objects[pending_index];
    assert_eq!(
        pending.state.local_vars.get("pending_count"),
        Some(&Value::Int(1)),
        "Construction must find the fresh object in its newly added sector"
    );
    assert_eq!(
        pending.state.local_vars.get("ordered"),
        Some(&Value::Array(vec![
            object_reference_value(newer),
            object_reference_value(older),
        ])),
        "adding the fresh object must not rebuild unrelated physical sector lists"
    );
}

#[test]
fn initial_growth_updates_pending_object_sectors_before_completion() {
    // C4Game::NewObject adds the object to Game.Objects.Sectors before
    // Construction, then initial DoCon updates its post-growth position and
    // sector links before Completion/Initialize return to the creator
    // (oracle-src-pinned src/C4Game.cpp:1121-1142;
    // src/C4GameObjects.cpp:54-70; src/C4Object.cpp:1428-1511). Bounded
    // FindObjects therefore observes that position both in Completion and in
    // the creator's next statement (src/C4FindObject.cpp:310-355).
    let grown_script = r#"#strict
local completion_count;
func Completion()
{
    completion_count = GetLength(FindObjects(
        [C4FO_InRect, -5, 75, 10, 10],
        [C4FO_ID, GROW]));
    return true;
}
"#;
    let creator_script = r#"#strict
local grown, creator_count;
func Trigger()
{
    grown = CreateObject(GROW, 0, 100, -1);
    creator_count = GetLength(FindObjects(
        [C4FO_InRect, -5, 75, 10, 10],
        [C4FO_ID, GROW]));
    return true;
}
"#;

    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(200, 200));
    let mut grown =
        Definition::from_script("GROW", "Growing object", grown_script).expect("grown compiles");
    grown.set_shape_rect(Some(DefinitionRect::new(-10, -20, 20, 40)));
    engine.register_definition(grown).expect("grown registers");
    engine
        .register_script_definition("CALL", "Creator", creator_script)
        .expect("creator registers");
    let creator = engine
        .spawn_object(SpawnConfig::new("CALL").with_category(CATEGORY_OBJECT))
        .expect("creator spawns");

    let creator_index = engine.find_object_index(creator).expect("creator exists");
    engine
        .call_object_function(creator_index, "Trigger", Vec::new())
        .expect("creation succeeds");

    let creator = engine.object_snapshot(creator).expect("creator remains");
    let grown = engine
        .objects
        .iter()
        .find(|object| object.definition_id == "GROW")
        .expect("grown object materializes");
    assert_eq!(grown.state.position.y, 80, "initial DoCon keeps the bottom");
    assert_eq!(
        grown.state.local_vars.get("completion_count"),
        Some(&Value::Int(1)),
        "Completion sees the post-growth sector"
    );
    assert_eq!(
        creator.local_vars.get("creator_count"),
        Some(&Value::Int(1)),
        "the creator's next statement sees the post-growth sector"
    );
}
