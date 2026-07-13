use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use lc_engine::{ObjectId, ObjectUpdate, SpawnConfig, COM_RELEASE_OFFSET, COM_THROW};
use lc_script::Value;

fn fill_kayak(engine: &mut lc_engine::Engine, kayak: ObjectId, cargo_count: usize) -> Vec<ObjectId> {
    (0..cargo_count)
        .map(|_| {
            engine
                .spawn_object(SpawnConfig::new("BONE").with_container(kayak))
                .expect("the shipped Arctic bone cargo spawns in KAJO")
        })
        .collect()
}

#[test]
fn arctic_occupied_kayak_opens_grouped_cargo_only_at_collection_limit() {
    // Shipped KAJO::ContainedThrow falls through to the ordinary hardcoded
    // Throw below its DefCore CollectionLimit=5, but queues Activate on the
    // contained Clonk once ContentsCount reaches five
    // (FarWorlds.c4d/Arctic.c4d/Vehicles.c4d/Kajak.c4d/
    // Occupied.c4d/Script.c:123-133; Occupied.c4d/DefCore.txt:20-22;
    // C4Object.cpp:3246-3282). The driver's own slot counts toward KAJO's
    // contents, so three cargo objects remain below the limit and four hit it.
    let mut engine = load_installed_scenario("FarWorlds.c4f/Arctic.c4s", 0);
    let owner = join_local_player(&mut engine, "Arctic kayak cargo parity");
    let crew = engine
        .crew_cursor(owner)
        .expect("Arctic joins with a selected Inuit");

    let below_limit = engine
        .spawn_object(SpawnConfig::new("KAJO").with_owner(owner))
        .expect("the shipped occupied kayak spawns");
    let _below_limit_cargo = fill_kayak(&mut engine, below_limit, 3);
    engine
        .apply_object_update(crew, ObjectUpdate::new().with_container(below_limit))
        .expect("the Inuit enters the below-limit occupied kayak");
    let carried_bone = engine
        .spawn_object(SpawnConfig::new("BONE").with_container(crew))
        .expect("the Inuit carries one ordinary throwable bone");

    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("the below-limit throw control completes");

    assert_eq!(
        engine
            .object_snapshot(carried_bone)
            .expect("the ordinary cargo survives")
            .container,
        Some(below_limit),
        "below CollectionLimit hardcoded Throw puts carried cargo into KAJO"
    );
    assert_eq!(
        engine.debug_object_menu(crew.as_u64()),
        Some(None),
        "below CollectionLimit KAJO must not open its cargo menu"
    );
    engine
        .player_in_com(owner, COM_THROW + COM_RELEASE_OFFSET, 0)
        .expect("release the first Throw before probing a second press");
    {
        // These are independent route probes. Advancing 11 whole scenario
        // frames would also execute Arctic's initial crew Exit command, so
        // expire only C4Player's double-click ledger between them.
        let control = &mut engine
            .player_mut(owner)
            .expect("the Arctic player remains joined")
            .control;
        control.last_com = 0;
        control.last_com_delay = 0;
        control.last_com_down_double = 0;
        control.pressed_coms = 0;
    }

    let full = engine
        .spawn_object(SpawnConfig::new("KAJO").with_owner(owner))
        .expect("a second shipped occupied kayak spawns");
    let full_cargo = fill_kayak(&mut engine, full, 4);
    engine
        .apply_object_update(crew, ObjectUpdate::new().with_container(full))
        .expect("the Inuit enters the full occupied kayak");
    let full_contents = engine
        .object_snapshot(full)
        .expect("the full occupied kayak survives")
        .contents;
    assert_eq!(
        full_contents.len(),
        5,
        "the driver and four cargo objects fill KAJO's five direct slots"
    );
    assert!(full_contents.contains(&crew));
    assert!(full_cargo.iter().all(|cargo| full_contents.contains(cargo)));

    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("the full-kayak throw control completes");
    let commands = engine
        .object_snapshot(crew)
        .expect("the Inuit survives the cargo-menu control")
        .command_stack
        .command_names();
    assert_eq!(
        commands.first().map(String::as_str),
        Some("Activate"),
        "KAJO queues its explicit Activate command at exactly five contents"
    );

    engine
        .tick()
        .expect("the queued Activate command opens the internal cargo menu");

    assert!(
        engine.pending_menu_requests.is_empty(),
        "C4MN_Activate is owned by the engine, not deferred to the frontend"
    );
    let menu = engine
        .debug_object_menu(crew.as_u64())
        .expect("the Inuit remains live")
        .expect("the full kayak opens an Activate menu");
    assert_eq!(menu.identification, Value::Int(6));
    assert!(!menu.user_menu, "the cargo menu is an internal object menu");
    assert_eq!(menu.refill_object, Some(full));
    assert_eq!(menu.items.len(), 1, "identical cargo is grouped by definition");
    let bone = &menu.items[0];
    assert_eq!(bone.item_id, "BONE");
    assert_eq!(bone.count, 4);
    assert!(
        bone.picture_object.is_some_and(|object| full_cargo.contains(&object)),
        "the grouped row keeps a live representative from the full kayak"
    );
}
