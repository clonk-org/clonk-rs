use crate::support::real_scenario::{
    join_local_player, prepare_installed_scenario, PreparedInstalledScenario,
};
use crate::support::EngineTestExt;
use crate::support::PreparedScenarioSubcase;
use clonk_engine::{
    CommandDirection, Direction, ObjectId, ObjectUpdate, SpawnConfig, COM_LEFT, COM_RELEASE_OFFSET,
    COM_THROW,
};
use clonk_script::Value;

fn fill_kayak(
    engine: &mut clonk_engine::Engine,
    kayak: ObjectId,
    cargo_count: usize,
) -> Vec<ObjectId> {
    (0..cargo_count)
        .map(|_| engine.spawn_test_object(SpawnConfig::new("BONE").with_container(kayak)))
        .collect()
}

#[test]
fn arctic_occupied_kayak_rows_with_jump_and_run_direction_updates() {
    let prepared = prepare_installed_scenario("FarWorlds.c4f/Arctic.c4s", 0);
    let subcases: &[PreparedScenarioSubcase] = &[
        (
            "rows_with_jump_and_run_direction_updates",
            arctic_occupied_kayak_rows_with_jump_and_run_direction_updates_subcase,
        ),
        (
            "opens_grouped_cargo_only_at_collection_limit",
            arctic_occupied_kayak_opens_grouped_cargo_only_at_collection_limit,
        ),
    ];
    let mut failures = Vec::new();

    for &(name, subcase) in subcases {
        eprintln!("running Arctic kayak subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| subcase(&prepared))).is_err() {
            eprintln!("Arctic kayak subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} Arctic kayak subcase(s) failed: {}",
            failures.len(),
            failures.join(", ")
        );
    }
}

fn arctic_occupied_kayak_rows_with_jump_and_run_direction_updates_subcase(
    prepared: &PreparedInstalledScenario,
) {
    // C4Object::ContainedControl dispatches PSF_ContainedControlUpdate, whose
    // script name is `~ContainedUpdate` (C4Script.h:74; C4Object.cpp:3253-3263).
    // Shipped KAJO deliberately defers Jump'n'Run direction handling from
    // ContainedLeft to ContainedUpdate, which starts/stops its Paddle action
    // (Occupied.c4d/Script.c:25-52,54-100).
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Arctic kayak rowing parity");
    crate::support::TestValueExt::test_value(engine.player_mut(owner))
        .control
        .control_style = true;
    let crew = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let kayak = engine.spawn_test_object(
        SpawnConfig::new("KAJO")
            .with_owner(owner)
            .with_in_liquid(true),
    );
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(crew, ObjectUpdate::new().with_container(kayak)),
    );
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            kayak,
            ObjectUpdate::new()
                .with_action("Stop")
                .with_direction(Direction::Left)
                .with_command_direction(CommandDirection::Stop),
        ),
    );
    engine.debug_set_in_liquid(kayak, true);

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_LEFT, 0));
    let rowing = engine.test_object_snapshot(kayak);
    assert_eq!(rowing.action.name, "Paddle");
    assert_eq!(rowing.command_direction, CommandDirection::Left);
    assert_eq!(rowing.direction, Direction::Left);

    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_LEFT + COM_RELEASE_OFFSET,
        0,
    ));
    let stopped = engine.test_object_snapshot(kayak);
    assert_eq!(stopped.action.name, "Stop");
    assert_eq!(stopped.command_direction, CommandDirection::Stop);
}

fn arctic_occupied_kayak_opens_grouped_cargo_only_at_collection_limit(
    prepared: &PreparedInstalledScenario,
) {
    // Shipped KAJO::ContainedThrow falls through to the ordinary hardcoded
    // Throw below its DefCore CollectionLimit=5, but queues Activate on the
    // contained Clonk once ContentsCount reaches five
    // (FarWorlds.c4d/Arctic.c4d/Vehicles.c4d/Kajak.c4d/
    // Occupied.c4d/Script.c:123-133; Occupied.c4d/DefCore.txt:20-22;
    // C4Object.cpp:3246-3282). The driver's own slot counts toward KAJO's
    // contents, so three cargo objects remain below the limit and four hit it.
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Arctic kayak cargo parity");
    let crew = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    let below_limit = engine.spawn_test_object(SpawnConfig::new("KAJO").with_owner(owner));
    let _below_limit_cargo = fill_kayak(&mut engine, below_limit, 3);
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(crew, ObjectUpdate::new().with_container(below_limit)),
    );
    let carried_bone = engine.spawn_test_object(SpawnConfig::new("BONE").with_container(crew));

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    assert_eq!(
        engine.test_object_snapshot(carried_bone).container,
        Some(below_limit),
        "below CollectionLimit hardcoded Throw puts carried cargo into KAJO"
    );
    assert_eq!(
        engine.debug_object_menu(crew.as_u64()),
        Some(None),
        "below CollectionLimit KAJO must not open its cargo menu"
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_THROW + COM_RELEASE_OFFSET,
        0,
    ));
    {
        // These are independent route probes. Advancing 11 whole scenario
        // frames would also execute Arctic's initial crew Exit command, so
        // expire only C4Player's double-click ledger between them.
        let control =
            &mut crate::support::TestValueExt::test_value(engine.player_mut(owner)).control;
        control.last_com = 0;
        control.last_com_delay = 0;
        control.last_com_down_double = 0;
        control.pressed_coms = 0;
    }

    let full = engine.spawn_test_object(SpawnConfig::new("KAJO").with_owner(owner));
    let full_cargo = fill_kayak(&mut engine, full, 4);
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(crew, ObjectUpdate::new().with_container(full)),
    );
    let full_contents = engine.test_object_snapshot(full).contents;
    assert_eq!(
        full_contents.len(),
        5,
        "the driver and four cargo objects fill KAJO's five direct slots"
    );
    assert!(full_contents.contains(&crew));
    assert!(full_cargo.iter().all(|cargo| full_contents.contains(cargo)));

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    let commands = engine
        .test_object_snapshot(crew)
        .command_stack
        .command_names();
    assert_eq!(
        commands.first().map(String::as_str),
        Some("Activate"),
        "KAJO queues its explicit Activate command at exactly five contents"
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert!(
        engine.pending_menu_requests.is_empty(),
        "C4MN_Activate is owned by the engine, not deferred to the frontend"
    );
    let menu = crate::support::TestValueExt::test_value(crate::support::TestValueExt::test_value(
        engine.debug_object_menu(crew.as_u64()),
    ));
    assert_eq!(menu.identification, Value::Int(6));
    assert!(!menu.user_menu, "the cargo menu is an internal object menu");
    assert_eq!(menu.refill_object, Some(full));
    assert_eq!(
        menu.items.len(),
        1,
        "identical cargo is grouped by definition"
    );
    let bone = &menu.items[0];
    assert_eq!(bone.item_id, "BONE");
    assert_eq!(bone.count, 4);
    assert!(
        bone.picture_object
            .is_some_and(|object| full_cargo.contains(&object)),
        "the grouped row keeps a live representative from the full kayak"
    );
}
