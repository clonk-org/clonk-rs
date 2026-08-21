use super::*;

trait TestEngineExt {
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str);
    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId;
}

impl TestEngineExt for Engine {
    fn register_test_script_definition(&mut self, id: &str, name: &str, script: &str) {
        crate::TestValueExt::test_value(self.register_script_definition(id, name, script));
    }

    fn spawn_test_object(&mut self, config: SpawnConfig) -> ObjectId {
        crate::TestValueExt::test_value(self.spawn_object(config))
    }
}

/// The gather order requires a clear path **both ways**, and each half is
/// pinned separately (clonk-org/clonk-rs#334).
///
/// Nothing in CI executes content script, so the shipped
/// `planet/System.c4g/GatherTask.c` is compiled and driven here rather than
/// trusted. Both halves matter and a single fixture cannot show it: an item
/// beyond the wall is already excluded by the outward check, so a return-path
/// bug hides behind it. The two cases below fail independently — deleting
/// either `PathFree` in the script turns exactly one of them red.
#[test]
fn the_gather_order_requires_a_clear_path_both_ways() {
    let mut engine = Engine::new();
    let sources = vec![(
        "GatherTask.c".to_owned(),
        include_str!("../../../../planet/System.c4g/GatherTask.c").to_owned(),
    )];
    assert_eq!(engine.install_global_scripts(&sources), 1);
    engine.resolve_appends();

    // Open ground with one solid column at x = 32, floor to ceiling.
    let mut landscape =
        crate::Landscape::with_default_material(64, vec![40; 64], None).expect("test landscape");
    landscape.set_world_height(40);
    let mut bytes = vec![0_u8; 64 * 40];
    for row in 0..40 {
        bytes[row * 64 + 32] = 1;
    }
    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        64,
        40,
        bytes,
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    engine.set_landscape(landscape);

    engine.register_test_script_definition("GOLD", "Nugget", "");
    engine.register_test_script_definition("CLNK", "Clonk", "");
    engine.register_test_script_definition("BASE", "Base", "");

    // Clonk and one nugget on the left of the wall; a second nugget beyond it.
    let clonk =
        engine.spawn_test_object(SpawnConfig::new("CLNK").with_position(Vector2::new(10, 20)));
    let near =
        engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(20, 20)));
    let _beyond =
        engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(50, 20)));

    let candidates = |engine: &mut Engine, base: Value| {
        let arguments = vec![
            Value::Object(clonk.as_u64()),
            Value::C4Id("GOLD".to_owned()),
            base,
        ];
        let value = crate::TestValueExt::test_value(
            engine.call_engine_global_function("ClonkRsGatherCandidates", &arguments),
        );
        let Value::Array(items) = value else {
            panic!("candidates must be an array, got {value:?}");
        };
        items
    };

    // Outward path: with no base to return to, only the reachable nugget is
    // offered. The one beyond the wall fails `PathFree` from the Clonk.
    let reachable = candidates(&mut engine, Value::Nil);
    assert_eq!(
        reachable.len(),
        1,
        "the nugget beyond the wall is not reachable"
    );
    assert_eq!(reachable[0], Value::Object(near.as_u64()));

    // Return path: the same nugget, now with a base on the far side of the
    // wall. The Clonk can still walk to it, but could not carry it home, so it
    // drops out — this is the half the outward check cannot cover.
    let across =
        engine.spawn_test_object(SpawnConfig::new("BASE").with_position(Vector2::new(50, 20)));
    let stranded = candidates(&mut engine, Value::Object(across.as_u64()));
    assert!(
        stranded.is_empty(),
        "an item the Clonk cannot carry home must not be offered, got {stranded:?}"
    );

    // And with a base it *can* reach, the same nugget is offered again, so the
    // exclusion above is the return path rather than the base merely existing.
    let home =
        engine.spawn_test_object(SpawnConfig::new("BASE").with_position(Vector2::new(4, 20)));
    let ordered = crate::TestValueExt::test_value(engine.call_engine_global_function(
        "ClonkRsGatherOrder",
        &[
            Value::Object(clonk.as_u64()),
            Value::C4Id("GOLD".to_owned()),
            Value::Object(home.as_u64()),
        ],
    ));
    assert_eq!(
        ordered,
        Value::Int(1),
        "one order for the one fetchable nugget"
    );
}

/// The menu offers one row per item *type*, carrying how many of it the Clonk
/// would fetch (clonk-org/clonk-rs#334).
///
/// The row list is what the player actually chooses from, so it is built by a
/// function of its own rather than inside the context callback: a menu built
/// straight from `FindObjects` would show one row per nugget, which is a list
/// of litter rather than a list of orders. Grouping is also where the count
/// comes from, and the count is the only thing on the row that tells the
/// player the order is worth giving.
///
/// The same wall as above separates reachable from unreachable, so this also
/// pins that the grouping runs *after* the reachability filter — counting
/// first and filtering second would advertise nuggets the Clonk cannot fetch.
#[test]
fn the_gather_menu_lists_one_row_per_reachable_type_with_its_count() {
    let mut engine = Engine::new();
    let sources = vec![(
        "GatherTask.c".to_owned(),
        include_str!("../../../../planet/System.c4g/GatherTask.c").to_owned(),
    )];
    assert_eq!(engine.install_global_scripts(&sources), 1);
    engine.resolve_appends();

    let mut landscape =
        crate::Landscape::with_default_material(64, vec![40; 64], None).expect("test landscape");
    landscape.set_world_height(40);
    let mut bytes = vec![0_u8; 64 * 40];
    for row in 0..40 {
        bytes[row * 64 + 32] = 1;
    }
    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        64,
        40,
        bytes,
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    engine.set_landscape(landscape);

    // Categories are the filter the menu uses to tell an item from its
    // surroundings, so the fixture has to carry real ones: `Find_Category`
    // would match nothing against the `C4D_StaticBack` a bare script
    // definition defaults to.
    let register = |engine: &mut Engine, id: &str, name: &str, category: i32| {
        let mut definition =
            crate::TestValueExt::test_value(crate::Definition::from_script(id, name, ""));
        definition.set_category(category);
        crate::TestValueExt::test_value(engine.register_definition(definition));
    };
    register(&mut engine, "GOLD", "Nugget", crate::CATEGORY_OBJECT);
    register(&mut engine, "ROCK", "Rock", crate::CATEGORY_OBJECT);
    register(&mut engine, "CLNK", "Clonk", crate::CATEGORY_LIVING);
    register(&mut engine, "BASE", "Base", crate::CATEGORY_STRUCTURE);

    let clonk =
        engine.spawn_test_object(SpawnConfig::new("CLNK").with_position(Vector2::new(10, 20)));
    let home =
        engine.spawn_test_object(SpawnConfig::new("BASE").with_position(Vector2::new(4, 20)));
    // Two nuggets and one rock this side of the wall...
    engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(20, 20)));
    engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(24, 20)));
    engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(Vector2::new(18, 20)));
    // ...and one of each beyond it, which must not reach the menu at all.
    engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(50, 20)));
    engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(Vector2::new(52, 20)));

    let value = crate::TestValueExt::test_value(engine.call_engine_global_function(
        "ClonkRsGatherTypes",
        &[Value::Object(clonk.as_u64()), Value::Object(home.as_u64())],
    ));
    let Value::Array(rows) = value else {
        panic!("gather types must be an array, got {value:?}");
    };

    // One row per type, each carrying only what is reachable: 2 nuggets and 1
    // rock, never the pair stranded beyond the wall.
    let mut seen: Vec<(String, i32)> = rows
        .iter()
        .map(|row| {
            let Value::Array(pair) = row else {
                panic!("each row is [id, count], got {row:?}");
            };
            let (Value::C4Id(id), Value::Int(count)) = (&pair[0], &pair[1]) else {
                panic!("each row is [id, count], got {pair:?}");
            };
            (id.clone(), *count)
        })
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        vec![("GOLD".to_owned(), 2), ("ROCK".to_owned(), 1)],
        "one row per reachable type, counting only the reachable ones"
    );
    // The Clonk standing there and the base it would carry to are both
    // uncontained and both trivially reachable, so only the category filter
    // keeps them off a menu of things to go and fetch.
    assert!(
        !seen.iter().any(|(id, _)| id == "CLNK" || id == "BASE"),
        "a crew member and a building are not litter, got {seen:?}"
    );
}

/// The context row opens a menu carrying one entry per reachable type, with
/// the type's id and its count on the entry (clonk-org/clonk-rs#334).
///
/// This is the half `GatherTask.c` deliberately left out. It is a separate
/// file because `#appendto` applies to a whole script: `GatherTask.c` stays a
/// set of global helpers usable from anywhere, and only the menu binds to the
/// crew definition.
///
/// The entry's id and count are asserted rather than its caption. The caption
/// runs through `$...$` string-table lookup that a bare engine fixture has no
/// table for, and the id/count pair is what the following `MenuSelection`
/// actually dispatches on.
#[test]
fn the_gather_context_row_opens_a_menu_of_reachable_types() {
    let mut engine = Engine::new();

    // Categories as in the row-list test: the menu's own filter needs them.
    let register = |engine: &mut Engine, id: &str, name: &str, category: i32| {
        let mut definition =
            crate::TestValueExt::test_value(crate::Definition::from_script(id, name, ""));
        definition.set_category(category);
        crate::TestValueExt::test_value(engine.register_definition(definition));
    };
    register(&mut engine, "GOLD", "Nugget", crate::CATEGORY_OBJECT);
    register(&mut engine, "ROCK", "Rock", crate::CATEGORY_OBJECT);
    register(&mut engine, "CLNK", "Clonk", crate::CATEGORY_LIVING);
    register(&mut engine, "BASE", "Base", crate::CATEGORY_STRUCTURE);

    // The definitions have to exist before the append resolves onto CLNK.
    let sources = vec![
        (
            "GatherTask.c".to_owned(),
            include_str!("../../../../planet/System.c4g/GatherTask.c").to_owned(),
        ),
        (
            "GatherMenu.c".to_owned(),
            include_str!("../../../../planet/System.c4g/GatherMenu.c").to_owned(),
        ),
    ];
    assert_eq!(engine.install_global_scripts(&sources), 2);
    engine.resolve_appends();

    let mut landscape =
        crate::Landscape::with_default_material(64, vec![40; 64], None).expect("test landscape");
    landscape.set_world_height(40);
    let mut bytes = vec![0_u8; 64 * 40];
    for row in 0..40 {
        bytes[row * 64 + 32] = 1;
    }
    landscape.set_pixel_grid(crate::landscape::PixelGrid::new(
        64,
        40,
        bytes,
        vec![0, 100],
        vec![None, Some("Earth".to_owned())],
        vec![None; 2],
    ));
    engine.set_landscape(landscape);

    let clonk =
        engine.spawn_test_object(SpawnConfig::new("CLNK").with_position(Vector2::new(10, 20)));
    engine.spawn_test_object(SpawnConfig::new("BASE").with_position(Vector2::new(4, 20)));
    engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(20, 20)));
    engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(24, 20)));
    engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(Vector2::new(18, 20)));
    // Beyond the wall, so neither reaches the menu.
    engine.spawn_test_object(SpawnConfig::new("GOLD").with_position(Vector2::new(50, 20)));
    engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(Vector2::new(52, 20)));

    // The engine reaches a context function through ProtectedCall on the
    // target with the calling Clonk as the argument (C4ObjectMenu.cpp:678).
    let index = engine.find_object_index(clonk).expect("the clonk exists");
    crate::TestValueExt::test_value(engine.call_object_function(
        index,
        "ContextGather",
        vec![Value::Object(clonk.as_u64())],
    ));

    let menu = engine
        .debug_object_menu(clonk.as_u64())
        .expect("the clonk exists")
        .expect("the context row opens a menu");
    let mut rows: Vec<(String, i32)> = menu
        .items
        .iter()
        .map(|item| (item.item_id.clone(), item.count))
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![("GOLD".to_owned(), 2), ("ROCK".to_owned(), 1)],
        "one entry per reachable type, carrying what the order would fetch"
    );
}
