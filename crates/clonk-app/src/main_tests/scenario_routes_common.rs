// Spliced into `mod tests` (src/main_tests.rs) via include!: a bare item
// sequence, not a child module, so test ids stay `tests::<fn>`.

fn advance_app_until(
    app: &mut GameApp,
    milestone: &str,
    max_ticks: u32,
    mut reached: impl FnMut(&GameApp) -> bool,
) {
    advance_app_until_erased(app, milestone, max_ticks, &mut reached);
}

#[inline(never)]
fn advance_app_until_erased(
    app: &mut GameApp,
    milestone: &str,
    max_ticks: u32,
    reached: &mut dyn FnMut(&GameApp) -> bool,
) {
    if reached(app) {
        return;
    }
    for _ in 0..max_ticks {
        app.update()
            .unwrap_or_else(|error| panic!("{milestone}: {error}"));
        if reached(app) {
            return;
        }
    }
    panic!(
        "timed out after {max_ticks} app ticks waiting for {milestone} at frame {}; cursor={:?}",
        app.engine.frame(),
        app.engine
            .crew_cursor(app.players.local_owner)
            .and_then(|cursor| app.engine.object_snapshot(cursor))
    );
}

fn hold_app_key_until(
    app: &mut GameApp,
    key: VirtualKeyCode,
    milestone: &str,
    max_ticks: u32,
    mut reached: impl FnMut(&GameApp) -> bool,
) {
    hold_app_key_until_erased(app, key, milestone, max_ticks, &mut reached);
}

#[inline(never)]
fn hold_app_key_until_erased(
    app: &mut GameApp,
    key: VirtualKeyCode,
    milestone: &str,
    max_ticks: u32,
    reached: &mut dyn FnMut(&GameApp) -> bool,
) {
    AppVirtualKeyboard::new(app).press(key);
    advance_app_until_erased(app, milestone, max_ticks, reached);
    AppVirtualKeyboard::new(app).release(key);
}

fn app_tutorial_message_contains(app: &GameApp, needle: &str) -> bool {
    app.snapshot
        .hud
        .messages
        .iter()
        .any(|message| message.lines.iter().any(|line| line.contains(needle)))
}

fn app_object_with_definition(app: &GameApp, definition: &str) -> Option<ObjectId> {
    app.engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == definition)
        .map(|object| object.id)
}

fn app_object_with_definition_near_x(
    app: &GameApp,
    definition: &str,
    expected_x: i32,
) -> Option<ObjectId> {
    app.engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == definition)
        .min_by_key(|object| (object.position.x - expected_x).abs())
        .map(|object| object.id)
}

fn app_clonk_carries(app: &GameApp, clonk: ObjectId, definition: &str) -> bool {
    app.engine.object_snapshot(clonk).is_some_and(|clonk| {
        clonk.contents.iter().any(|item| {
            app.engine
                .object_snapshot(*item)
                .is_some_and(|item| item.definition_id == definition)
        })
    })
}

fn app_object_contents_count(app: &GameApp, container: ObjectId, definition: &str) -> usize {
    app.engine
        .object_snapshot(container)
        .map_or(0, |container| {
            container
                .contents
                .iter()
                .filter(|object_id| {
                    app.engine
                        .object_snapshot(**object_id)
                        .is_some_and(|object| object.definition_id == definition)
                })
                .count()
        })
}

fn app_cursor_inventory_contains(app: &mut GameApp, clonk: ObjectId, definition: &str) -> bool {
    let mut overlays = collect_player_overlays(
        &mut app.engine,
        &app.snapshot,
        Some(clonk),
        &app.bindings,
        &app.gamepad_bindings,
    );
    populate_crew_inventories(
        &app.engine,
        &app.snapshot,
        &mut overlays,
        clonk_frontend::AdvancedRendererConfig::DEFAULT,
    );
    overlays
        .iter()
        .flat_map(|player| &player.crew)
        .find(|crew| crew.object_id == clonk)
        .is_some_and(|crew| {
            crew.inventory
                .iter()
                .any(|item| item.definition_id == definition && item.picture.is_some())
        })
}

fn real_tutorial_app(tutorial: u8, player_name: &str) -> RealTutorialApp {
    real_installed_scenario_app(
        &format!("Tutorial.c4f/Tutorial{tutorial:02}.c4s"),
        player_name,
    )
}

fn app_tutorial09_system_names_preserve_cpp_ready_conkit_route(
    prepared: &PreparedRealInstalledScenario,
) {
    // C4Game::InitScriptEngine loads System.c4g/Names.txt before players
    // join. C4ObjectInfoCore::Default consumes its synchronized name draw
    // before PlaceReadyCrew's position draw, leaving the seed-zero CLNK
    // just left of CNKT so the shipped rightward lesson route collects it
    // (C4Game.cpp:2767-2792; C4InfoCore.cpp:34-55;
    // C4Player.cpp:481-520).
    let mut app = prepared.instantiate("Tutorial 9 app name parity", false);
    let clonk = app.engine.test_crew_cursor(app.players.local_owner);
    main_assert_eq!(
        app.engine
            .object_snapshot(clonk)
            .expect("Tutorial09 CLNK survives startup")
            .position =>
        Vector2::new(278, 130),
        "System names keep the C++ seed-zero ready-crew placement"
    );
    advance_app_until(&mut app, "Tutorial09 asks for an igloo", 240, |app| {
        app_tutorial_message_contains(app, "build an igloo")
    });
    hold_app_key_until(
        &mut app,
        VirtualKeyCode::KeyC,
        "physical C collects Tutorial09 CNKT",
        30,
        |app| app_clonk_carries(app, clonk, "CNKT"),
    );
}

fn run_real_tutorial01_app_subcase(
    name: &'static str,
    failures: &mut Vec<&'static str>,
    subcase: impl FnOnce(),
) {
    eprintln!("running Tutorial01 app subcase `{name}`");
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(subcase)).is_err() {
        eprintln!("Tutorial01 app subcase `{name}` failed; continuing batch");
        failures.push(name);
    }
}

fn assert_no_real_tutorial01_app_subcase_failures(failures: Vec<&str>) {
    main_assert!(failures.is_empty(), "Tutorial01 app subcase(s) failed: {}", failures.join(", "));
}
