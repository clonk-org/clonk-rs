//! The recursive child menus a Goal or Rule row reaches through
//! `Activate(player)` (clonk-org/clonk-rs#560).
//!
//! `C4MainMenu::MenuCommand` turns a `Player:Goal:<id>` / `Player:Rule:<id>`
//! row into one queued `CID_ActivateGameGoalRule`, whose script is
//! `Activate(plr)` evaluated in the *object's* scope (C4MainMenu.cpp:886-897;
//! C4Control.h:597-607). What that call then opens is decided entirely by
//! shipped content, so the families below are taken from the 138 shipped
//! Goal/Rule definitions rather than invented:
//!
//! | definitions | `Activate` reaches |
//! |---|---|
//! | 115 | `MessageWindow(...)` — an Info-style menu on the cursor |
//! | 10 | `CreateMenu` + `AddMenuItem` — a chooser on the cursor |
//! | 3 | a host-only guard, then `PlayerMessage` or a settings menu |
//! | 1 | `AddEffect` on the cursor, no menu at all |
//! | 1 | `GameCall` + `RemoveObject`, no menu at all |
//! | 7 | no `Activate` at all |
//!
//! Nothing in CI executes content script, so the shipped globals these rely on
//! are compiled from `planet/System.c4g` here rather than trusted.

use super::*;
use crate::lib_test_support::EngineTestExt;

/// `MessageWindow` is **not** an engine function — it is a shipped global in
/// `planet/System.c4g/Helpers.c:5-16`, which is why no C++ `file:line` in the
/// engine defines it. It opens a style-2 (`C4MN_Style_Info`) menu on the
/// caller's cursor and hangs the message on a single item.
/// Shipped content is ISO-8859-1, not UTF-8, so the bytes are decoded the way
/// `bytes_as_latin1_string` does rather than `include_str!`-ed.
const HELPERS: &[u8] = include_bytes!("../../../../planet/System.c4g/Helpers.c");

fn engine_with_helpers() -> Engine {
    let helpers: String = HELPERS.iter().copied().map(char::from).collect();
    let mut engine = Engine::new();
    assert_eq!(
        engine.install_global_scripts(&[
            ("Helpers.c".to_owned(), helpers),
            // `GetCrew` reads the player's crew *list*, which only
            // `MakeCrewMember` fills — selecting a crew and setting the cursor
            // do not (compat/players.rs:2253-2276). Enlisting through the same
            // host function real content uses keeps the fixtures honest.
            (
                "Enlist.c".to_owned(),
                "#strict 2\nglobal func ClonkRsEnlist(object crew, int plr) { return(MakeCrewMember(crew, plr)); }\nglobal func ClonkRsAlive(object obj) { return(GetObjectStatus(obj)); }"
                    .to_owned(),
            ),
        ]),
        2,
        "the shipped Helpers.c and the enlist shim must both compile"
    );
    engine.resolve_appends();
    engine
}

/// A `CLNK` whose **definition** is a crew member. `MakeCrewMember` refuses an
/// object whose def lacks the flag (`Def->CrewMember`, C4Player.cpp:1170), and
/// `SpawnConfig::with_crew_member` only marks the object, so without this the
/// player's crew list stays empty and `GetCrew` answers nil.
fn crew_definition(script: &str) -> Definition {
    let mut definition = test_definition("CLNK", "Clonk", script);
    definition.set_crew_member(true);
    definition
}

/// Gives `player` a walking crew cursor, the way `GetCursor(iForPlr)` expects.
fn player_with_cursor(engine: &mut Engine, player: i32) -> ObjectId {
    crate::TestValueExt::test_value(
        engine.register_player(PlayerConfig::new(player, format!("Player {player}"))),
    );
    crate::TestValueExt::test_value(engine.player_mut(player))
        .set_at_client(PlayerAtClient::new(0));
    let crew = crate::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("CLNK")
                .with_owner(player)
                .with_crew_member(true)
                .with_action(ActionState::new("Walk")),
        ),
    );
    assert_eq!(
        crate::TestValueExt::test_value(engine.call_engine_global_function(
            "ClonkRsEnlist",
            &[Value::Object(crew.as_u64()), Value::Int(player)],
        )),
        Value::Bool(true),
        "the crew must actually join the player's crew list"
    );
    crate::TestValueExt::test_value(engine.select_crew(player, vec![crew]));
    crate::TestValueExt::test_value(engine.set_crew_cursor(player, Some(crew)));
    crew
}

fn activate(engine: &mut Engine, object: ObjectId, player: i32) -> bool {
    let number = crate::TestValueExt::test_value(i32::try_from(object.as_u64()));
    engine
        .execute_activate_game_goal_rule_control(&ActivateGameGoalRuleControlData {
            object: number,
            player,
            by_client: 0,
        })
        .expect("goal/rule activation executes")
}

/// The dominant family: 115 of the 138 shipped Goal/Rule definitions do
/// nothing in `Activate` but `return(MessageWindow(<text>, iPlayer))`
/// (e.g. `content/Western.c4f/DeadMansValley.c4s/Kill.c4d`).
///
/// The whole chain has to work for a goal row to show anything at all:
/// `Activate` → the shipped `MessageWindow` global → `GetCursor` →
/// `CreateMenu(idIcon, pCursor, pCursor, 0, pCaption, 0, 2)` →
/// `AddMenuItem(pCaption, "", TIM1, pCursor, 0, 0, pMsg)`.
#[test]
fn a_goal_activate_opens_the_shipped_message_window_on_the_cursor() {
    let mut engine = engine_with_helpers();
    engine.register_test_definition(crew_definition("#strict 2\n"));
    let player = 0;
    let crew = player_with_cursor(&mut engine, player);

    let mut goal = test_definition(
        "IGOL",
        "Integrated Goal",
        "#strict 2\nprotected func Activate(iPlayer) { return(MessageWindow(\"Reach the target\", iPlayer)); }",
    );
    goal.set_category(crate::CATEGORY_GOAL);
    engine.register_test_definition(goal);
    let goal_object =
        crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("IGOL")));

    assert!(activate(&mut engine, goal_object, player));

    let (target, menu) = engine
        .cursor_object_menu(player)
        .expect("MessageWindow opens a menu on the cursor");
    assert_eq!(target, crew, "the menu belongs to the cursor, not the goal");
    // Helpers.c passes style 2 = C4MN_Style_Info.
    assert_eq!(menu.style, 2, "MessageWindow is an Info-style menu");
    // `if (!idIcon) idIcon = GetID();` / `if (!pCaption) pCaption = GetName();`
    // both run in the *goal's* scope, so they name the goal.
    assert_eq!(menu.symbol_id, "IGOL");
    assert_eq!(menu.caption, "Integrated Goal");
    assert_eq!(menu.items.len(), 1, "exactly one item carries the message");
    assert_eq!(menu.items[0].info_caption, "Reach the target");
}

/// The chooser family: 10 shipped definitions build their own menu instead of
/// deferring to `MessageWindow` — `CROB` in
/// `content/Missions.c4f/Frontier.c4s/CreateObjects.c4d:16-31` is the shape,
/// repeated verbatim in SkyVillage, RedrockBay and Treasure.
///
/// Two things separate it from the `MessageWindow` family, and both are easy
/// to get wrong because the menu still *appears* on the cursor:
///
/// * `CreateMenu(GetID(), menuObject, this(), 0, GetName())` passes the goal
///   as the **command object**, so selecting a row calls back into the goal,
///   not into the cursor that owns the menu.
/// * it passes **no style argument**, so the menu is `C4MN_Style_Normal` (0)
///   and five columns wide (C4Menu.cpp:359-365) — not the one-column Info
///   sheet `MessageWindow` asks for.
#[test]
fn a_chooser_goal_builds_a_normal_menu_owned_by_the_cursor_but_commanded_by_the_goal() {
    let mut engine = engine_with_helpers();
    engine.register_test_definition(crew_definition("#strict 2\n"));
    engine.register_test_definition(test_definition("WOOD", "Wood", "#strict 2\n"));
    engine.register_test_definition(test_definition("METL", "Metal", "#strict 2\n"));
    let player = 0;
    let crew = player_with_cursor(&mut engine, player);

    let mut goal = test_definition(
        "CROB",
        "Create Objects",
        "#strict 2\n\
         protected func Activate(iPlayer) {\n\
           var menuObject = GetCursor(iPlayer);\n\
           CreateMenu(GetID(), menuObject, this(), 0, GetName());\n\
           AddMenuItem(GetName(), \"Noop\", WOOD, menuObject, 3, 0, \"Three wood\");\n\
           AddMenuItem(GetName(), \"Noop\", METL, menuObject, 1, 0, \"One metal\");\n\
           return(1);\n\
         }\n\
         public func Noop() { return(); }",
    );
    goal.set_category(crate::CATEGORY_GOAL);
    engine.register_test_definition(goal);
    let goal_object =
        crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("CROB")));

    assert!(activate(&mut engine, goal_object, player));

    let (target, menu) = engine
        .cursor_object_menu(player)
        .expect("the chooser opens on the cursor");
    assert_eq!(target, crew, "the menu is hung on the cursor");
    assert_eq!(
        menu.command_object,
        Some(goal_object),
        "rows must command the goal, not the cursor that shows them"
    );
    assert_eq!(menu.style, 0, "no style argument means C4MN_Style_Normal");
    assert_eq!(menu.columns, 5, "Normal menus open five columns wide");
    assert_eq!(menu.caption, "Create Objects");

    let rows: Vec<_> = menu
        .items
        .iter()
        .map(|item| {
            (
                item.item_id.as_str(),
                item.count,
                item.info_caption.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [("WOOD", 3, "Three wood"), ("METL", 1, "One metal")],
        "each remaining type keeps its own count and info line"
    );
}

/// The host-only family: `BALA`, `CAPT` and `FRAG`
/// (`content/Western.c4f/DeadMansValley.c4s/Fraglimit.c4d:87-104` and its two
/// siblings) open their settings menu only for the *first* player and answer
/// everyone else with `PlayerMessage`.
///
/// `GetPlayerByIndex()` with no argument is player index 0, so the guard is
/// "am I the host". The row stays selectable for everyone — the refusal is a
/// message, not a disabled item — which is exactly why it needs pinning: a
/// port that ignored the guard would silently let any client retune the
/// scenario.
#[test]
fn a_host_only_rule_answers_other_players_with_a_message_and_no_menu() {
    let mut engine = engine_with_helpers();
    engine.register_test_definition(crew_definition("#strict 2\n"));
    let host = 0;
    let guest = 1;
    let host_crew = player_with_cursor(&mut engine, host);
    let guest_crew = player_with_cursor(&mut engine, guest);

    let mut rule = test_definition(
        "FRAG",
        "Frag Limit",
        "#strict 2\n\
         protected func Activate(iByPlayer) {\n\
           if (iByPlayer != GetPlayerByIndex()) {\n\
             PlayerMessage(iByPlayer, \"Only the host may change this\");\n\
             return(1);\n\
           }\n\
           CreateMenu(GetID(), GetCursor(iByPlayer), this(), 0, GetName());\n\
           AddMenuItem(GetName(), \"Noop\", GetID(), GetCursor(iByPlayer), 1, 0, \"Raise the limit\");\n\
           return(1);\n\
         }\n\
         public func Noop() { return(); }",
    );
    // C4D_Rule (C4Def.h:69); the port exposes CATEGORY_GOAL but no rule twin.
    rule.set_category(1 << 19);
    engine.register_test_definition(rule);
    let rule_object =
        crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("FRAG")));

    // The guest is refused: a message, and no settings menu anywhere.
    assert!(activate(&mut engine, rule_object, guest));
    assert!(
        engine.cursor_object_menu(guest).is_none(),
        "a non-host must not get the settings menu"
    );
    assert!(
        engine.cursor_object_menu(host).is_none(),
        "and the refusal must not leak a menu onto the host either"
    );

    // The host gets the real thing on their own cursor.
    assert!(activate(&mut engine, rule_object, host));
    let (target, menu) = engine
        .cursor_object_menu(host)
        .expect("the host opens the settings menu");
    assert_eq!(target, host_crew);
    assert_eq!(menu.items.len(), 1);
    assert!(
        engine.cursor_object_menu(guest).is_none(),
        "the host's menu belongs to the host's cursor only"
    );
    let _ = guest_crew;
}

/// The side-effect family: two shipped rules open no menu at all.
/// `REAC` (`content/Objects.c4d/Rules.c4d/ReleaseClonk.c4d`) hangs an effect
/// on the cursor, and `RSTR`
/// (`content/Objects.c4d/Goals.c4d/Race.c4d/Restart.c4d`) removes the crew
/// after a `GameCall`.
///
/// "Nothing opens" is the whole contract, and it is the one a port breaks by
/// helpfully falling back to a menu when `Activate` returns without creating
/// one.
#[test]
fn a_side_effect_rule_runs_its_effect_and_opens_no_menu() {
    let mut engine = engine_with_helpers();
    engine.register_test_definition(crew_definition(
        "#strict 2\npublic func HasFade() { return(GetEffect(\"ReleaseClonkFadeOut\", this())); }",
    ));
    let player = 0;
    let crew = player_with_cursor(&mut engine, player);

    let mut rule = test_definition(
        "REAC",
        "Release Clonk",
        "#strict 2\n\
         protected func Activate(plr) {\n\
           AddEffect(\"ReleaseClonkFadeOut\", GetCursor(plr), 320, 1, 0, GetID(), plr);\n\
           return(1);\n\
         }\n\
         public func FxReleaseClonkFadeOutStart() { return(1); }",
    );
    // C4D_Rule (C4Def.h:69); the port exposes CATEGORY_GOAL but no rule twin.
    rule.set_category(1 << 19);
    engine.register_test_definition(rule);
    let rule_object =
        crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("REAC")));

    assert!(activate(&mut engine, rule_object, player));

    let crew_index = crate::TestValueExt::test_value(engine.find_object_index(crew));
    let fade = engine
        .call_object_function(crew_index, "HasFade", Vec::new())
        .expect("effect query runs");
    assert_ne!(
        fade,
        Value::Nil,
        "the rule's effect must be attached to the cursor"
    );
    assert!(
        engine.cursor_object_menu(player).is_none(),
        "a side-effect rule opens no menu"
    );
}

/// Seven shipped definitions define no `Activate` at all — `GCTF`, `GDOM` and
/// `GTDM` in `content/Hazard.c4d/Goals.c4d`, `BNKR`/`ORDR` in BankRobbery,
/// `CHIF` in DeadMansValley and the `_ETG` extinguisher rule in CoFuT.
///
/// Their rows are still listed and still selectable, because `ActivateGoals`
/// builds the list from the goal *ID list* and never asks whether the
/// definition implements anything (C4MainMenu.cpp:351-374). Selecting one
/// therefore queues a control whose script call resolves to nothing: the
/// engine must absorb that quietly rather than fail the control or open an
/// empty menu.
#[test]
fn a_definition_without_activate_is_selectable_and_does_nothing() {
    let mut engine = engine_with_helpers();
    engine.register_test_definition(crew_definition("#strict 2\n"));
    let player = 0;
    let _crew = player_with_cursor(&mut engine, player);

    let mut goal = test_definition(
        "GTDM",
        "Deathmatch",
        "#strict 2\npublic func IsFulfilled() { return(false); }",
    );
    goal.set_category(crate::CATEGORY_GOAL);
    engine.register_test_definition(goal);
    let goal_object =
        crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("GTDM")));

    assert!(
        activate(&mut engine, goal_object, player),
        "the control is still authorized and still executes"
    );
    assert!(
        engine.cursor_object_menu(player).is_none(),
        "a definition with no Activate opens nothing"
    );
}

/// The command family: `CM5B`
/// (`content/EkeReloaded.c4d/GoalsAndRules.c4d/ContextMenu.c4d:1-28`) opens no
/// menu of its own. It picks a target — the building the cursor stands in, the
/// vehicle it pushes, its first contents, else the Clonk itself — and issues
/// `SetCommand(clonk, "Context", 0, 0, 0, stuff)`, letting the ordinary
/// command layer raise that object's context menu.
///
/// So the rule's whole observable effect is *a queued command*, and the menu
/// arrives later through command execution. Asserting the command rather than
/// a menu is the point: this family is the one that looks broken if you only
/// ever look for a menu after `Activate`.
#[test]
fn a_command_rule_queues_a_context_command_instead_of_building_a_menu() {
    let mut engine = engine_with_helpers();
    engine.register_test_definition(crew_definition(
        "#strict 2\n\
         public func ReadCommand() { return(GetCommand(this(), 0)); }\n\
         public func ReadTarget() { return(GetCommand(this(), 1)); }\n\
         public func ReadTarget2() { return(GetCommand(this(), 4)); }",
    ));
    let player = 0;
    let crew = player_with_cursor(&mut engine, player);

    let mut rule = test_definition(
        "CM5B",
        "Context Menu",
        "#strict 2\n\
         protected func Activate(player) {\n\
           var clonk = GetCursor(player);\n\
           OpenContextMenu(clonk, clonk);\n\
         }\n\
         private func OpenContextMenu(clonk, stuff) {\n\
           SetCommand(clonk, \"Context\", 0, 0, 0, stuff);\n\
         }",
    );
    // C4D_Rule (C4Def.h:69); the port exposes CATEGORY_GOAL but no rule twin.
    rule.set_category(1 << 19);
    engine.register_test_definition(rule);
    let rule_object =
        crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("CM5B")));

    assert!(activate(&mut engine, rule_object, player));

    let crew_index = crate::TestValueExt::test_value(engine.find_object_index(crew));
    assert_eq!(
        engine
            .call_object_function(crew_index, "ReadCommand", Vec::new())
            .expect("command name reads"),
        Value::String("Context".into()),
        "activation queues a Context command on the cursor"
    );
    // `SetCommand(clonk, "Context", 0, 0, 0, stuff)` puts `stuff` in the
    // *sixth* parameter, which is Target2 — Target stays null. Elements are
    // 0 Name, 1 Target, 2 Tx, 3 Ty, 4 Target2 (C4Script.cpp:929-943).
    assert_eq!(
        engine
            .call_object_function(crew_index, "ReadTarget", Vec::new())
            .expect("command target reads"),
        Value::Nil,
        "the context object is not the command's Target"
    );
    assert_eq!(
        engine
            .call_object_function(crew_index, "ReadTarget2", Vec::new())
            .expect("command target2 reads"),
        Value::Object(crew.as_u64()),
        "it rides in Target2, which is where the context menu reads it from"
    );
    assert!(
        engine.cursor_object_menu(player).is_none(),
        "the rule itself builds no menu — the command layer does that later"
    );
}

/// The vetoable family: `RSTR`
/// (`content/Objects.c4d/Goals.c4d/Race.c4d/Restart.c4d:5-12`) asks the
/// scenario first — `if (GameCall("OnRestart", iPlr)) return();` — and only
/// removes the player's Clonk when the scenario declines to handle it.
///
/// Both halves matter and one fixture cannot show it: a port that ignored the
/// return value would still look right in the no-handler case, because the
/// removal is what happens then anyway. So the veto is driven twice, and only
/// the handled run must leave the crew alive.
#[test]
fn a_vetoable_rule_removes_the_crew_only_when_the_scenario_declines() {
    let removal_survives = |handled: bool| {
        let mut engine = engine_with_helpers();
        engine.register_test_definition(crew_definition("#strict 2\n"));
        let player = 0;
        let crew = player_with_cursor(&mut engine, player);

        // `GameCall` dispatches into the *scenario* script, not into globals
        // (compat/contexts.rs:1251-1269), so the handler has to be installed
        // as one.
        crate::TestValueExt::test_value(engine.install_scenario_script_with_convention(
            "Scenario.c",
            &format!(
                "#strict 2\npublic func OnRestart(plr) {{ return({}); }}",
                if handled { "1" } else { "0" }
            ),
            true,
        ));

        let mut rule = test_definition(
            "RSTR",
            "Restart",
            "#strict 2\n\
             protected func Activate(iPlr) {\n\
               if (GameCall(\"OnRestart\", iPlr)) return();\n\
               RemoveObject(GetCrew(iPlr), 1);\n\
             }",
        );
        // C4D_Rule (C4Def.h:69); the port exposes CATEGORY_GOAL but no rule twin.
        rule.set_category(1 << 19);
        engine.register_test_definition(rule);
        let rule_object =
            crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("RSTR")));

        assert!(activate(&mut engine, rule_object, player));
        // Removing a player's last crew member eliminates the player, which is
        // why crew liveness is read as "is this object still an object" rather
        // than through the player.
        // A removed object reports status 0, not nil, so liveness is
        // `Status == C4OS_NORMAL` rather than "the query answered".
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ClonkRsAlive", &[Value::Object(crew.as_u64())]),
        ) == Value::Int(1)
    };

    assert!(
        removal_survives(true),
        "a scenario that handles OnRestart keeps its own crew"
    );
    assert!(
        !removal_survives(false),
        "an unhandled OnRestart falls through to RemoveObject"
    );
}
