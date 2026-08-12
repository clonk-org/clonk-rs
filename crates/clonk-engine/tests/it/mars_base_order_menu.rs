//! ClonkMars' "Order" page is built by the bundled Menu2 helper (MS4C), whose
//! `ShowRange` shipped one product row plus an unlabelled `Increase by 1` and
//! `Decrease by 1` row per product (Menu.c4d/Script.c:108-125). The port
//! deliberately diverges: `planet/System.c4g/MenuRangeRow.c` appends to MS4C so
//! a range is one row whose primary activation adds a step and whose secondary
//! activation removes one — the same one-row-per-product shape the engine's own
//! C4MN_Buy menu uses, where Command2 carries the other quantity
//! (C4ObjectMenu.cpp:246-271; C4Menu.cpp:512-514).

use crate::support::real_scenario::{
    join_local_player, load_installed_scenario, prepare_installed_scenario, repository_root,
};
use clonk_engine::{Engine, ObjectId, SpawnConfig, Vector2};
use clonk_resources::Group;
use clonk_script::Value;

/// Row index of the `Open Order` submenu on the Base's top-level page: the
/// `Sell` enum contributes two rows before it (Base.c4d/Script.c:117-121).
const ORDER_SUBMENU_ROW: i32 = 2;

const APPEND: &str = "MenuRangeRow.c";

fn call(engine: &mut Engine, object: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = engine
        .find_object_index(object)
        .expect("object remains live");
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function} executes: {error}"))
}

fn items(engine: &Engine, clonk: ObjectId) -> Vec<clonk_engine::ObjectMenuItem> {
    engine
        .debug_object_menu(clonk.as_u64())
        .flatten()
        .expect("the Clonk carries the Menu2 page")
        .items
}

/// Walk into a Base, press up and choose Order — the route the ticket
/// describes (Base.c4d/Script.c:115-130). The ordering Clonk becomes the
/// player's cursor, because that is what player controls address
/// (C4Object::Control via the cursor, C4Player.cpp).
fn open_order_page(engine: &mut Engine) -> (i32, ObjectId) {
    let player = join_local_player(engine, "Mars order tester");
    let base = engine
        .spawn_object(
            SpawnConfig::new("BASE")
                .with_owner(player)
                .with_position(Vector2::new(300, 200)),
        )
        .expect("Mars base spawns");
    let clonk = engine
        .spawn_object(
            SpawnConfig::new("SCNK")
                .with_loaded(true)
                .with_owner(player)
                .with_controller(player)
                .with_alive(true)
                .with_crew_member(true)
                .with_container(base),
        )
        .expect("ordering Spaceclonk spawns inside the base");
    engine
        .select_crew(player, vec![clonk])
        .expect("the ordering Clonk joins the crew");
    engine
        .set_crew_cursor(player, Some(clonk))
        .expect("the ordering Clonk becomes the cursor");
    // Base.c4d/Script.c:157-160 refuses the menu without a satellite hanging
    // over it; Sat.c4d/Script.c:18-25 is what puts one there.
    let sat = engine
        .spawn_object(
            SpawnConfig::new("SATD")
                .with_owner(player)
                .with_container(base),
        )
        .expect("supply satellite spawns");
    call(engine, sat, "Entrance", vec![Value::Object(base.as_u64())]);

    call(
        engine,
        base,
        "ContainedUp",
        vec![Value::Object(clonk.as_u64())],
    );
    let index = engine.find_object_index(clonk).expect("clonk remains live");
    engine.objects[index]
        .state
        .menu
        .as_mut()
        .expect("ContainedUp opened the Menu2 page")
        .selection = ORDER_SUBMENU_ROW;
    engine
        .menu_user_enter(clonk, false)
        .expect("selecting Order opens the submenu");
    (player, clonk)
}

fn reopen_order_page(engine: &mut Engine, clonk: ObjectId) {
    let index = engine.find_object_index(clonk).expect("clonk remains live");
    engine.objects[index]
        .state
        .menu
        .as_mut()
        .expect("the Call Capsule page is open")
        .selection = ORDER_SUBMENU_ROW;
    engine
        .menu_user_enter(clonk, false)
        .expect("selecting Order reopens the submenu");
}

fn mars_order_page_collapses_each_product_to_a_single_row(engine: &Engine, clonk: ObjectId) {
    // Fossae sells five products (01_Fossae.c4s Scenario.txt [Player1]
    // HomeBaseMaterial=CNKT=6;PIKT=3;METL=10;PSTC=10;WIRO=3), so the shipped
    // three-rows-per-product expansion produced 16 rows for them.
    let captions: Vec<String> = items(engine, clonk)
        .into_iter()
        .map(|item| item.caption)
        .collect();
    assert_eq!(
        captions.len(),
        6,
        "five products plus the closing row, not five times three plus one: {captions:?}"
    );
}

fn mars_order_row_offers_both_steps_on_the_product_row(engine: &Engine, clonk: ObjectId) {
    // The surviving row is the product row: it keeps the definition symbol and
    // the ordered quantity the engine draws as "Nx" (C4Menu.cpp:198-207), and
    // carries both activations — Command on a left enter, Command2 on a right
    // one (C4Menu.cpp:512-514).
    let row = items(engine, clonk).swap_remove(0);
    assert_eq!(
        row.caption, "Construction kit - 10{{GOLD}} (+1/<c 888888>-1</c>)      ",
        "the row advertises both steps from the first frame, the one its limits \
         forbid greyed rather than dropped — colour markup has no width, so the \
         row cannot change size as the value moves"
    );
    assert_eq!(
        row.info_caption, "Currently 0. Click: +1.",
        "nothing is ordered yet and the row sits on its minimum, so only the increase is offered"
    );
    assert_eq!(row.command, "Adjust(CNKT,0,0)", "primary activation");
    assert_eq!(row.command2, "Adjust(CNKT,0,1)", "secondary activation");
    assert_eq!(row.item_id, "CNKT", "the product keeps its own symbol");
}

fn mars_order_row_adds_on_a_left_enter_and_takes_back_on_a_right_one(
    engine: &mut Engine,
    clonk: ObjectId,
) {
    engine
        .menu_user_enter(clonk, false)
        .expect("a left enter on the first product runs its command");
    let ordered = items(engine, clonk).swap_remove(0);
    assert_eq!(ordered.count, 1, "one construction kit is on the order");
    assert_eq!(
        ordered.caption, "Construction kit - 10{{GOLD}} (+1/-1)      ",
        "away from both limits neither step is greyed"
    );
    assert_eq!(
        ordered.info_caption,
        "Currently 1. Click: +1, right-click: -1."
    );

    engine
        .menu_user_enter(clonk, true)
        .expect("a right enter runs Command2");
    let cleared = items(engine, clonk).swap_remove(0);
    assert_eq!(cleared.count, 0, "the order is empty again");
    assert_eq!(
        cleared.caption, "Construction kit - 10{{GOLD}} (+1/<c 888888>-1</c>)      ",
        "back on the minimum the decrease greys out again"
    );
}

fn mars_order_row_steps_with_the_left_and_right_controls(
    engine: &mut Engine,
    player: i32,
    clonk: ObjectId,
) {
    // The reporter could only ever increase, because -1 lives on an
    // activation nothing on screen names. Left/Right are dead keys in a
    // one-column menu (C4Menu.cpp:433-457), so the override claims them for
    // the selected product — the horizontal-axis-adjusts-a-value convention.
    for _ in 0..3 {
        engine
            .player_in_com(player, clonk_engine::COM_MENU_RIGHT, 0)
            .expect("right steps the selected product");
    }
    engine
        .player_in_com(player, clonk_engine::COM_MENU_LEFT, 0)
        .expect("left steps it back");

    let menu = engine
        .debug_object_menu(clonk.as_u64())
        .flatten()
        .expect("the order page is rebuilt");
    assert_eq!(menu.items[0].count, 2, "three up and one down leaves two");
    assert_eq!(
        menu.selection, 0,
        "stepping a value must not also move the selection"
    );
}

fn mars_order_arrows_still_navigate_off_a_product_row(
    engine: &mut Engine,
    player: i32,
    clonk: ObjectId,
) {
    // The closing row is not a range, so the arrows must fall back to the
    // shipped selection move there rather than swallowing the input.
    let closing_row = 5;
    let index = engine.find_object_index(clonk).expect("clonk remains live");
    engine.objects[index]
        .state
        .menu
        .as_mut()
        .expect("the order page is open")
        .selection = closing_row;

    engine
        .player_in_com(player, clonk_engine::COM_MENU_RIGHT, 0)
        .expect("right runs on the closing row");

    let menu = engine
        .debug_object_menu(clonk.as_u64())
        .flatten()
        .expect("the order page stays open");
    assert_eq!(
        menu.selection, 0,
        "an unclaimed step wraps the selection like C4Menu::Control always did"
    );
    assert!(
        menu.items.iter().take(5).all(|item| item.count == 0),
        "and orders nothing"
    );
}

/// Step the first product to `quantity`, return to the Call Capsule page and
/// activate its commit row.
fn order_and_commit(engine: &mut Engine, player: i32, clonk: ObjectId, quantity: usize) {
    for _ in 0..quantity {
        engine
            .player_in_com(player, clonk_engine::COM_MENU_RIGHT, 0)
            .expect("stepping the first product");
    }
    let select = |engine: &mut Engine, row: i32| {
        let index = engine.find_object_index(clonk).expect("clonk remains live");
        engine.objects[index]
            .state
            .menu
            .as_mut()
            .expect("a page is open")
            .selection = row;
    };
    let last_row =
        |engine: &Engine| i32::try_from(items(engine, clonk).len()).expect("row count fits") - 1;
    select(engine, last_row(engine));
    engine.menu_user_enter(clonk, false).expect("Back");
    select(engine, last_row(engine));
    engine.menu_user_enter(clonk, false).expect("commit");
}

/// Fossae already has a capsule standing on the map at scenario start, so a
/// delivery is a change in the count rather than the first one to exist.
fn capsule_count(engine: &Engine) -> usize {
    engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "CPSL")
        .count()
}

#[test]
fn an_order_over_the_players_wealth_is_refused_whole() {
    // The shipped commit calls Buy without fShowErrors, so C4Player::Buy
    // suppresses IDS_PLR_NOWEALTH and its Error sound (C4Player.cpp:849-853),
    // then `return true` on the first unaffordable item abandons the rest of
    // the order (Base.c4d/Script.c:148-151). The capsule was already created,
    // so the player silently gets a part-load in hash-bucket order and has
    // spent the allowance. Fossae starts at Wealth=30 against 186 clunkers of
    // stock, so this is the ordinary outcome, not an edge case.
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let (player, clonk) = open_order_page(&mut engine);
    let wealth = engine.player(player).expect("player").wealth();
    assert_eq!(wealth, 30, "Fossae's starting clunkers");

    let capsules = capsule_count(&engine);

    // Six construction kits at 10 each is twice what the player has.
    order_and_commit(&mut engine, player, clonk, 6);

    assert_eq!(
        engine.player(player).expect("player").wealth(),
        wealth,
        "an order that cannot be paid for buys nothing at all"
    );
    assert_eq!(
        capsule_count(&engine),
        capsules,
        "and does not spend the one-capsule allowance either"
    );
}

#[test]
fn an_affordable_order_is_delivered() {
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let (player, clonk) = open_order_page(&mut engine);
    let capsules = capsule_count(&engine);

    // Two construction kits at 10 each, against 30 clunkers.
    order_and_commit(&mut engine, player, clonk, 2);

    assert_eq!(
        engine.player(player).expect("player").wealth(),
        10,
        "the order is paid for"
    );
    assert_eq!(
        capsule_count(&engine),
        capsules + 1,
        "and the capsule is on its way"
    );
}

fn a_product_row_reserves_the_column_its_quantity_is_drawn_in(engine: &Engine, clonk: ObjectId) {
    // C4Menu sizes a Context row from its caption and symbol only
    // (C4Menu.cpp:648-662) but draws the count right-aligned at the row's
    // right edge anyway (C4Menu.cpp:198-207), so the widest caption on the
    // page ends exactly where its own count starts and the two overprint.
    // Only the caption is measured, so only the caption can reserve the room.
    let rows = items(engine, clonk);

    // Construction kits stock 6 — one digit — and metal 10, which is two.
    assert!(
        rows[0].caption.ends_with("(+1/<c 888888>-1</c>)      "),
        "one digit reserves six spaces: {:?}",
        rows[0].caption
    );
    assert!(
        rows[2].caption.ends_with("(+1/<c 888888>-1</c>)         "),
        "two digits reserve nine: {:?}",
        rows[2].caption
    );
}

fn the_order_page_shows_what_it_will_cost(engine: &mut Engine, player: i32, clonk: ObjectId) {
    // Menu2 passes iExtra = 0 to CreateMenu, opting out of the one footer the
    // engine has for money: C4MN_Extra_Value draws a figure beside the wealth
    // symbol and arms the player's wealth HUD for the duration
    // (C4Menu.cpp:898-906). Without it the player composes an order with no
    // idea what it costs or what they can afford — and Fossae's stock is worth
    // six times its starting clunkers.
    let menu = engine
        .debug_object_menu(clonk.as_u64())
        .flatten()
        .expect("the order page is open");
    assert_eq!(
        menu.extra,
        clonk_engine::ObjectMenuExtra::Value,
        "the page reserves the value footer"
    );
    assert!(
        menu.items.iter().all(|item| item.value == Some(0)),
        "an untouched order costs nothing, on whichever row is selected"
    );

    // Two construction kits at 10 clunkers each.
    for _ in 0..2 {
        engine
            .player_in_com(player, clonk_engine::COM_MENU_RIGHT, 0)
            .expect("stepping the first product");
    }

    let priced = engine
        .debug_object_menu(clonk.as_u64())
        .flatten()
        .expect("the order page is rebuilt");
    assert!(
        priced.items.iter().all(|item| item.value == Some(20)),
        "the running total tracks the order: {:?}",
        priced
            .items
            .iter()
            .map(|item| item.value)
            .collect::<Vec<_>>()
    );
}

fn a_menu_whose_owner_prices_nothing_keeps_the_shipped_footer(engine: &Engine, cursor: ObjectId) {
    // ShowRange is Menu2's generic widget and its other client is a viewport
    // size chooser, which has no money in it. The footer must stay off there.
    let menu = engine
        .debug_object_menu(cursor.as_u64())
        .flatten()
        .expect("the viewport chooser is open");
    assert_eq!(
        menu.extra,
        clonk_engine::ObjectMenuExtra::None,
        "no owner-supplied total, no footer"
    );
    assert!(
        menu.items.iter().all(|item| item.value.is_none()),
        "and no rows carry one"
    );
}

fn every_product_shows_its_quantity_even_at_zero(engine: &Engine, clonk: ObjectId) {
    // C4Script.cpp:1726 turns a zero count into C4MN_Item_NoCount, so an
    // untouched order page showed no quantities at all and gave the player no
    // hint that the right-hand column is one. C4MN_Add_ForceCount keeps it.
    let counts: Vec<i32> = items(engine, clonk)
        .iter()
        .take(5)
        .map(|item| item.count)
        .collect();
    assert_eq!(
        counts,
        vec![0; 5],
        "the quantity column reads 0x on every product before anything is ordered"
    );
}

fn the_order_page_offers_undo_only_once_there_is_something_to_undo(
    engine: &mut Engine,
    player: i32,
    clonk: ObjectId,
) {
    // Arrows cover the keyboard, but a mouse row is a single click target, so
    // -1 is still only on the right button there. One visible row that walks
    // the last change back means no part of composing an order is reachable
    // *only* through an input nothing on screen names.
    let untouched = items(engine, clonk);
    assert!(
        !untouched
            .iter()
            .any(|item| item.caption.starts_with("Undo")),
        "nothing has been ordered, so there is nothing to undo"
    );

    engine
        .player_in_com(player, clonk_engine::COM_MENU_RIGHT, 0)
        .expect("ordering one construction kit");

    let with_history = items(engine, clonk);
    assert_eq!(
        with_history.len(),
        untouched.len() + 1,
        "the undo row appears above the closing row"
    );
    let undo_row = with_history.len() - 2;
    assert_eq!(
        with_history[undo_row].caption, "Undo: Construction kit - 10{{GOLD}}",
        "and names what it will take back"
    );

    let index = engine.find_object_index(clonk).expect("clonk remains live");
    engine.objects[index]
        .state
        .menu
        .as_mut()
        .expect("the order page is open")
        .selection = i32::try_from(undo_row).expect("row index fits");
    engine.menu_user_enter(clonk, false).expect("undo runs");

    let undone = items(engine, clonk);
    assert_eq!(
        undone[0].count, 0,
        "the construction kit is off the order again"
    );
    assert_eq!(
        undone.len(),
        untouched.len(),
        "and the row leaves with the history it described"
    );
}

fn the_undo_row_stays_on_the_page_its_change_belongs_to(
    engine: &mut Engine,
    player: i32,
    clonk: ObjectId,
) {
    // It names a product row, so it would be meaningless on the page above.
    engine
        .player_in_com(player, clonk_engine::COM_MENU_RIGHT, 0)
        .expect("ordering one construction kit");
    assert_eq!(items(engine, clonk).len(), 7, "products, undo, Back");

    let index = engine.find_object_index(clonk).expect("clonk remains live");
    engine.objects[index]
        .state
        .menu
        .as_mut()
        .expect("the order page is open")
        .selection = 6;
    engine.menu_user_enter(clonk, false).expect("Back");

    assert_eq!(
        items(engine, clonk).len(),
        4,
        "the Call Capsule page is unchanged by an order-page undo"
    );
}

fn an_unchosen_mode_row_is_blank_rather_than_crossed_out(engine: &mut Engine, clonk: ObjectId) {
    // Menu2 renders an iconless enum item with the whole 64px cell from its
    // sheet: cell 3 is a green check for the chosen one and cell 4 a red cross
    // for the rest (Menu.c4d/Script.c:255-261). They are radio markers, but a
    // red cross beside "Only Sell" reads as forbidden, not as unselected. A
    // blank symbol says the same thing without the false warning.
    let last = i32::try_from(items(engine, clonk).len()).expect("fits") - 1;
    let index = engine.find_object_index(clonk).expect("clonk remains live");
    engine.objects[index]
        .state
        .menu
        .as_mut()
        .expect("the order page is open")
        .selection = last;
    engine.menu_user_enter(clonk, false).expect("Back");

    let rows = items(engine, clonk);
    assert_eq!(rows[0].caption, "Only Sell");
    assert_eq!(
        rows[0].image,
        clonk_engine::ObjectMenuImage::None,
        "the unchosen mode carries no symbol at all"
    );
    assert_eq!(rows[1].caption, "Buy");
    assert_eq!(
        rows[1].image,
        clonk_engine::ObjectMenuImage::Indexed { index: 3 },
        "and the chosen one keeps the check that marks it"
    );
}

fn the_closing_row_says_which_of_its_two_jobs_it_is_doing(engine: &mut Engine, clonk: ObjectId) {
    // Shipped Menu2 captions the last row "Finished" on every page, but it
    // steps back out of a submenu and commits the whole template at the root
    // (Menu.c4d/Script.c:54,206-228) — two very different outcomes behind one
    // word and one green check.
    let order_page = items(engine, clonk);
    assert_eq!(
        order_page[order_page.len() - 1].caption,
        "Back",
        "the Order page's last row leaves the page"
    );

    let closing_row = i32::try_from(order_page.len()).expect("row count fits") - 1;
    let index = engine.find_object_index(clonk).expect("clonk remains live");
    engine.objects[index]
        .state
        .menu
        .as_mut()
        .expect("the order page is open")
        .selection = closing_row;
    engine
        .menu_user_enter(clonk, false)
        .expect("Back returns to the Call Capsule page");
    let root_page = items(engine, clonk);
    assert_eq!(root_page.len(), 4, "the Call Capsule page is back");
    assert_eq!(
        root_page[root_page.len() - 1].caption,
        "Finished",
        "the root page's last row is the commit"
    );
}

fn escape_abandons_the_order_from_the_order_page(
    engine: &mut Engine,
    player: i32,
    clonk: ObjectId,
) {
    // Shipped MenuQueryCancel pops one path level, so closing from a submenu
    // reopened the page above and the ordering UI could not be dismissed
    // there at all (Menu.c4d/Script.c:208-212,230-235).
    engine
        .player_in_com(player, clonk_engine::COM_MENU_CLOSE, 0)
        .expect("closing the menu runs");

    assert_eq!(
        engine
            .debug_object_menu(clonk.as_u64())
            .expect("clonk lives"),
        None,
        "the whole ordering menu is gone, not just the page"
    );
}

fn mars_order_row_stops_offering_a_step_it_cannot_take(engine: &mut Engine, clonk: ObjectId) {
    // Fossae stocks six construction kits, which is the range maximum
    // AddRangeChoice was given (Base.c4d/Script.c:125). Increase clamps there
    // (Menu.c4d/Script.c:184-187), so the row must stop advertising it.
    for _ in 0..6 {
        engine
            .menu_user_enter(clonk, false)
            .expect("a left enter on the first product runs its command");
    }
    let full = items(engine, clonk).swap_remove(0);
    assert_eq!(full.count, 6, "the whole stock is on the order");
    assert_eq!(
        full.caption, "Construction kit - 10{{GOLD}} (<c 888888>+1</c>/-1)      ",
        "on the maximum the increase greys out instead of vanishing"
    );
    assert_eq!(
        full.info_caption, "Currently 6 (maximum). Right-click: -1.",
        "and says why the increase is gone"
    );

    engine
        .menu_user_enter(clonk, false)
        .expect("a left enter on a maxed row still runs its command");
    assert_eq!(
        items(engine, clonk).swap_remove(0).count,
        6,
        "the shipped BoundBy clamp is untouched"
    );
}

fn mars_order_row_moves_only_its_own_product_and_keeps_the_selection(
    engine: &mut Engine,
    clonk: ObjectId,
) {
    // Both halves of the command matter: the typed parameter picks the range
    // to change and the row number is what ShowMenu re-selects afterwards
    // (Menu.c4d/Script.c:55,178-182).
    let metal_row = 2;
    let index = engine.find_object_index(clonk).expect("clonk remains live");
    engine.objects[index]
        .state
        .menu
        .as_mut()
        .expect("the order page is open")
        .selection = metal_row;
    engine
        .menu_user_enter(clonk, false)
        .expect("a left enter on the metal row runs its command");

    let menu = engine
        .debug_object_menu(clonk.as_u64())
        .flatten()
        .expect("the order page is rebuilt");
    assert_eq!(
        menu.selection, metal_row,
        "the rebuilt page keeps pointing at the row that was activated"
    );
    // Five products, then the undo row the change just earned, then Back.
    let counts: Vec<i32> = menu.items.iter().take(5).map(|item| item.count).collect();
    assert_eq!(counts, vec![0, 0, 1, 0, 0], "only metal was ordered");
    assert_eq!(
        menu.items[metal_row as usize].command, "Adjust(METL,2,0)",
        "each row names its own product and its own row number"
    );
}

#[test]
fn mars_order_initial_page_exposes_compact_priced_quantities_and_undo() {
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let (player, clonk) = open_order_page(&mut engine);

    mars_order_page_collapses_each_product_to_a_single_row(&engine, clonk);
    mars_order_row_offers_both_steps_on_the_product_row(&engine, clonk);
    a_product_row_reserves_the_column_its_quantity_is_drawn_in(&engine, clonk);
    every_product_shows_its_quantity_even_at_zero(&engine, clonk);
    the_order_page_offers_undo_only_once_there_is_something_to_undo(&mut engine, player, clonk);
}

#[test]
fn mars_order_adjustment_controls_preserve_counts_costs_and_limits() {
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let (player, clonk) = open_order_page(&mut engine);

    mars_order_row_adds_on_a_left_enter_and_takes_back_on_a_right_one(&mut engine, clonk);
    the_order_page_shows_what_it_will_cost(&mut engine, player, clonk);
    for _ in 0..2 {
        engine
            .player_in_com(player, clonk_engine::COM_MENU_LEFT, 0)
            .expect("resetting the first product after checking its price");
    }
    mars_order_row_steps_with_the_left_and_right_controls(&mut engine, player, clonk);
    for _ in 0..2 {
        engine
            .player_in_com(player, clonk_engine::COM_MENU_LEFT, 0)
            .expect("resetting the first product after checking arrow controls");
    }
    mars_order_row_stops_offering_a_step_it_cannot_take(&mut engine, clonk);
}

#[test]
fn mars_order_selection_and_navigation_keep_the_active_product() {
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let (player, clonk) = open_order_page(&mut engine);

    mars_order_arrows_still_navigate_off_a_product_row(&mut engine, player, clonk);
    mars_order_row_moves_only_its_own_product_and_keeps_the_selection(&mut engine, clonk);
}

#[test]
fn mars_order_exit_routes_keep_root_labels_and_escape_reachable() {
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let (player, clonk) = open_order_page(&mut engine);

    the_undo_row_stays_on_the_page_its_change_belongs_to(&mut engine, player, clonk);
    reopen_order_page(&mut engine, clonk);
    an_unchosen_mode_row_is_blank_rather_than_crossed_out(&mut engine, clonk);
    reopen_order_page(&mut engine, clonk);
    the_closing_row_says_which_of_its_two_jobs_it_is_doing(&mut engine, clonk);
    reopen_order_page(&mut engine, clonk);
    escape_abandons_the_order_from_the_order_page(&mut engine, player, clonk);
}

/// ClonkMars' other Menu2 client: a resolution chooser whose two ranges use a
/// string key, carry no item id, and are switched off by picking a preset
/// (Viewport.c4d/Script.c:50-66).
fn open_viewport_menu(engine: &mut Engine) -> ObjectId {
    let player = join_local_player(engine, "Mars viewport tester");
    let cursor = engine
        .player(player)
        .and_then(|player| player.cursor())
        .expect("the joined player has a crew cursor");
    let viewport = engine
        .spawn_object(SpawnConfig::new("VWPT").with_owner(player))
        .expect("the viewport helper spawns");
    call(engine, viewport, "SizeSelection", Vec::new());
    cursor
}

fn menu2_range_rows_collapse_for_a_string_key_without_a_symbol_too(
    engine: &Engine,
    cursor: ObjectId,
) {
    // The case whose composed command has to survive quoting.
    let rows = items(engine, cursor);
    let captions: Vec<&str> = rows.iter().map(|item| item.caption.as_str()).collect();
    assert_eq!(
        captions.len(),
        11,
        "seven preset resolutions, Custom, the two axes and the closing row: {captions:?}"
    );
    let axis = &rows[8];
    assert!(
        axis.caption.starts_with("X resolution ("),
        "the axis row states its steps: {}",
        axis.caption
    );
    assert_eq!(
        axis.command, "Adjust(\"X\",8,0)",
        "a string key reaches the dispatcher quoted"
    );
    assert_eq!(axis.command2, "Adjust(\"X\",8,1)");
}

fn a_range_whose_condition_fails_collapses_to_one_inert_row(engine: &mut Engine, cursor: ObjectId) {
    // Picking a preset resolution turns the two custom axes off
    // (Viewport.c4d/Script.c:61-62 guards them with MenuCond_Chosen). The
    // greyed branch keeps the shipped inert ShowMenu command and its greyed
    // symbol, but costs one row instead of three.
    engine
        .menu_user_enter(cursor, false)
        .expect("choosing the first preset resolution runs its command");

    let rows = items(engine, cursor);
    assert_eq!(rows.len(), 11, "still one row per range, now inert");
    let axis = &rows[8];
    assert_eq!(
        axis.caption, "<c 888888>X resolution</c>",
        "an unavailable range advertises no step at all"
    );
    assert_eq!(
        axis.command, "ShowMenu(8)",
        "and keeps the shipped do-nothing command"
    );
}

#[test]
fn mars_viewport_ranges_keep_compact_rows_conditions_and_footer() {
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let cursor = open_viewport_menu(&mut engine);

    a_menu_whose_owner_prices_nothing_keeps_the_shipped_footer(&engine, cursor);
    menu2_range_rows_collapse_for_a_string_key_without_a_symbol_too(&engine, cursor);
    a_range_whose_condition_fails_collapses_to_one_inert_row(&mut engine, cursor);
}

#[test]
fn collapsing_the_rows_spends_no_synchronized_draw() {
    // A/B against the shipped three-row expansion. The rows differ, so the
    // object-number counter does — the greyed branch creates one dummy where
    // Menu2 created three — but neither branch reaches the synchronized RNG,
    // which is what a scenario's own assertions and every replay depend on.
    let prepared = prepare_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    let mut collapsed = prepared.instantiate();
    let mut shipped = prepared.instantiate_without_system_script(APPEND);

    let cursors = [&mut collapsed, &mut shipped].map(|engine| {
        let cursor = open_viewport_menu(engine);
        // Picking a preset resolution greys both ranges out, which is the
        // branch whose dummy objects differ.
        engine
            .menu_user_enter(cursor, false)
            .expect("choosing the first preset resolution runs its command");
        cursor
    });
    assert_eq!(
        items(&collapsed, cursors[0]).len() + 4,
        items(&shipped, cursors[1]).len(),
        "the A/B must actually exercise two different pages"
    );

    assert_eq!(
        collapsed.debug_rng_clone().count,
        shipped.debug_rng_clone().count,
        "building either page must spend the same synchronized draws"
    );
    let spawned = [&mut collapsed, &mut shipped].map(|engine| {
        engine
            .spawn_object(SpawnConfig::new("METL"))
            .expect("a probe object spawns")
    });
    assert_ne!(
        spawned[0], spawned[1],
        "the accepted cost: four fewer dummy objects leaves the object-number \
         counter somewhere else"
    );
}

#[test]
fn the_row_hint_is_localized_from_the_system_group() {
    // The hint text is the first `$...$` in planet/System.c4g, so its string
    // table is new to that group: pin that both languages resolve, since a
    // missing key would leave the literal key in the source and fail to parse
    // (C4LangStringTable / clonk-resources script_strings.rs:130-186).
    let system =
        Group::open(repository_root().join("planet/System.c4g")).expect("planet System.c4g opens");
    let source = clonk_script::c4_string_from_bytes(
        &system
            .read_file(APPEND)
            .expect("the Menu2 range override ships"),
    );

    for (language, expected) in [
        ("US", "Currently %d. Click: +%d, right-click: -%d."),
        ("DE", "Aktuell %d. Klick: +%d, Rechtsklick: -%d."),
    ] {
        let localized = clonk_resources::localize_script_source(&system, &source, &[language])
            .expect("the override localizes");
        assert!(
            localized.contains(expected),
            "{language} resolves the range hint"
        );
        assert!(
            !localized.contains("$Menu"),
            "{language} leaves no unresolved key behind"
        );
    }
}
