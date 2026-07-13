#![allow(dead_code)]

use crate::support::real_scenario::{join_local_player, load_installed_scenario, load_tutorial};
use lc_engine::{
    math, ActionState, AudioCommand, Definition, Direction, EffectVarValue, JoinPlayerConfig,
    ObjectId, ObjectUpdate, PlayerStatus, SpawnConfig, Vector2, COM_DIG, COM_DOWN,
    COM_MENU_SELECT, COM_RELEASE_OFFSET, COM_RIGHT, COM_SPECIAL, COM_THROW, COM_UP, FULL_CON,
    OWNER_NONE,
};
use lc_script::Value;

#[test]
fn tutorial_harness_boots_the_installed_cpp_global_script_layer() {
    let engine = load_tutorial(2, 0);

    // C++ loads planet/System.c4g before definitions and the scenario
    // (C4Game.cpp:2591-2607,2764-2788). Helpers.c supplies both functions
    // used by Tutorial02 and BALN; a direct Scenario::apply fixture does not.
    for function in ["Schedule", "ScheduleCall", "FxIntScheduleCallTimer"] {
        assert!(
            engine.debug_global_has_function(function),
            "virtual play must expose planet global `{function}`"
        );
    }
    assert_eq!(
        engine.materials().len(),
        21,
        "virtual play must use the installed Material.c4g library"
    );
}

#[test]
fn sky_race_death_announces_before_the_shipped_relaunch_path() {
    let mut engine = load_installed_scenario("Races.c4f/Skyrace.c4s", 0);
    let owner = join_local_player(&mut engine, "Sky Race death parity");
    let clonk = engine
        .crew_cursor(owner)
        .expect("Sky Race joins its Scenario.txt CLNK");
    let carried_loam = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "LOAM" && object.container == Some(clonk))
        .count();
    assert_eq!(
        carried_loam, 1,
        "Skyrace.c4s::JoinPlayer gives the Clonk one LOAM bridge chunk"
    );

    // The real opening route eventually drops the Clonk between islands.
    // Exercise the shipped CLNK::Death callback directly after marking that
    // same real-content object dead. C++ FnDeathAnnounce emits exactly one
    // object message and returns true (C4Script.cpp:291-319); it never aborts
    // CLNK::Death before Skyrace's RelaunchPlayer callback can run.
    engine
        .register_definition(
            Definition::from_script(
                "DTHP",
                "Death path probe",
                r#"#strict
public func Trigger(object target)
{
    target->SetAlive(false);
    return target->Death(-1);
}
"#,
            )
            .expect("death-path probe compiles"),
        )
        .expect("death-path probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("DTHP"))
        .expect("death-path probe spawns");
    let probe_index = engine.find_object_index(probe).expect("probe index");
    assert_eq!(
        engine
            .call_object_function(
                probe_index,
                "Trigger",
                vec![Value::Object(clonk.as_u64())],
            )
            .expect("the shipped Sky Race CLNK death callback completes"),
        Value::Int(1)
    );

    let death_messages = engine
        .snapshot()
        .hud
        .messages
        .into_iter()
        .filter(|message| message.target == Some(clonk))
        .collect::<Vec<_>>();
    assert_eq!(
        death_messages.len(),
        1,
        "FnDeathAnnounce creates one C++ object-targeted death message"
    );
    let death_text = death_messages[0].lines.join("|");
    assert!(
        death_text.ends_with(" is dead.")
            || death_text.ends_with(" has|deceased.")
            || death_text.ends_with("|rests in peace."),
        "DeathAnnounce selects one of IDS_OBJ_DEATH1..7: {death_text:?}"
    );
}

#[test]
fn sky_race_relaunch_selects_and_positions_the_new_loam_carrier() {
    let mut engine = load_installed_scenario("Races.c4f/Skyrace.c4s", 0);
    let owner = join_local_player(&mut engine, "Sky Race relaunch parity");
    let original = engine
        .crew_cursor(owner)
        .expect("Sky Race joins its initial CLNK");

    // CLNK::Death reaches this real scenario callback after a BottomOpen
    // fall. Invoke that shipped callback synchronously so the assertions
    // observe its exact SetPosition before another physics frame. C++ adds
    // the replacement to the live C4Player::Crew inside MakeCrewMember;
    // SelectCrew and JoinPlayer(GetCrew(owner)) immediately see it
    // (C4Player.cpp:1194-1209; Skyrace.c4s/Script.c:75-91).
    engine
        .register_definition(
            Definition::from_script(
                "RLHP",
                "Relaunch probe",
                r#"#strict
public func Trigger(int owner)
{
    return GameCallEx("RelaunchPlayer", owner);
}
"#,
            )
            .expect("relaunch probe compiles"),
        )
        .expect("relaunch probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("RLHP"))
        .expect("relaunch probe spawns");
    let probe_index = engine.find_object_index(probe).expect("probe index");
    assert_eq!(
        engine
            .call_object_function(probe_index, "Trigger", vec![Value::Int(owner)])
            .expect("the shipped Sky Race RelaunchPlayer callback completes"),
        Value::Int(1)
    );
    let replacement = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.id != original
                && object.definition_id == "CLNK"
                && object.status.is_active()
        })
        .map(|object| object.id)
        .expect("RelaunchPlayer creates a replacement CLNK");

    let replacement_snapshot = engine
        .object_snapshot(replacement)
        .expect("the replacement CLNK remains live");
    let start_y = engine
        .landscape()
        .expect("Sky Race keeps its generated landscape")
        .estimated_height()
        / 2
        - 15;
    assert!(
        (10..110).contains(&replacement_snapshot.position.x)
            && replacement_snapshot.position.y == start_y,
        "JoinPlayer(GetCrew(owner)) must place the replacement at the scripted start; \
         position={:?}, expected y={start_y}",
        replacement_snapshot.position
    );
    let replacement_loam = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| {
            object.definition_id == "LOAM" && object.container == Some(replacement)
        })
        .count();
    assert_eq!(
        replacement_loam, 1,
        "the replacement receives exactly the one LOAM from JoinPlayer"
    );
    assert_eq!(
        engine.crew_cursor(owner),
        Some(replacement),
        "SelectCrew must see the same-call MakeCrewMember insertion"
    );
}

#[test]
fn sky_race_finish_eliminates_the_loser_and_ends_the_real_round() {
    let mut engine = load_installed_scenario("Races.c4f/Skyrace.c4s", 0);
    let winner = join_local_player(&mut engine, "Sky Race winner");
    let loser = engine
        .join_player(JoinPlayerConfig {
            name: "Sky Race loser".to_string(),
            player_info_id: 2,
            score: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0x00_00_ff,
            pref_color: 1,
            pref_position: 1,
            crew: Vec::new(),
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 2,
        })
        .expect("the second real Sky Race player joins")
        .number;
    let winner_clonk = engine
        .crew_cursor(winner)
        .expect("the winning player has a selected CLNK");
    let race = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "RACE")
        .map(|object| object.id)
        .expect("Skyrace's Scenario.txt creates the RACE goal");
    assert!(
        engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "GOAL"),
        "the inherited GOAL controller drives normal round completion"
    );

    // RACE::CheckGoal uses a strict right-edge comparison. Skyrace overrides
    // the end offset to 100 pixels, so x=width-99 is the first winning pixel
    // (Objects.c4d/Goals.c4d/Race.c4d/Script.c:19-27;
    // Races.c4f/Skyrace.c4s/Script.c:61-62).
    let landscape_width = engine
        .landscape()
        .expect("Sky Race keeps its generated landscape")
        .width() as i32;
    let y = engine
        .object_snapshot(winner_clonk)
        .expect("winner CLNK remains live")
        .position
        .y;
    engine
        .apply_object_update(
            winner_clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(landscape_width - 99, y))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk"),
        )
        .expect("place the winner on the real finish pixel");

    let race_index = engine.find_object_index(race).expect("RACE remains live");
    assert_eq!(
        engine
            .call_object_function(race_index, "GetWayPercent", vec![Value::Int(winner)])
            .expect("the shipped race computes winner progress"),
        Value::Int(100)
    );
    engine
        .tick()
        .expect("the shipped one-tick RACE timer accepts the finisher");

    let after_finish = engine.snapshot();
    let scoreboard = &after_finish.hud.scoreboard;
    let race_column = (0..scoreboard.column_count())
        .find(|column| {
            scoreboard.cell(0, *column).map(|cell| cell.value())
                == Some(i32::from_le_bytes(*b"RACE"))
        })
        .expect("RACE::Initialize creates its progress column");
    let winner_row = (1..scoreboard.row_count())
        .find(|row| scoreboard.cell(*row, 0).map(|cell| cell.value()) == Some(0))
        .expect("RACE::InitializePlayer creates the winner row by player-info ID");
    assert_eq!(
        scoreboard
            .cell(winner_row, race_column)
            .map(|cell| (cell.text(), cell.value())),
        Some((Some("100%"), 100)),
        "UpdateScoreboard writes the C++ finish percentage before sorting"
    );
    let winner_state = after_finish
        .players
        .iter()
        .find(|player| player.id == winner)
        .expect("winner state remains present");
    let loser_state = after_finish
        .players
        .iter()
        .find(|player| player.id == loser)
        .expect("loser state remains present");
    let loser_info_id = loser_state.player_info_id;
    assert_eq!(winner_state.status, PlayerStatus::Active);
    assert_eq!(loser_state.status, PlayerStatus::Eliminated);
    assert_eq!(engine.eliminated_owners(), vec![loser]);

    for _ in 0..300 {
        if engine.snapshot().game_over {
            break;
        }
        engine.tick().expect("advance the normal GOAL controller");
    }
    let completed = engine.snapshot();
    assert!(
        completed.game_over,
        "GOAL::CheckTime -> Wait4End -> RoundOver must finish Sky Race"
    );
    assert!(
        completed
            .players
            .iter()
            .find(|player| player.id == winner)
            .is_some_and(|player| player.won),
        "the surviving finisher receives the C++ winner flag"
    );
    assert!(
        !completed
            .players
            .iter()
            .any(|player| player.id == loser),
        "C4PlayerList retires the eliminated rival after 60 frames"
    );
    assert!(
        completed
            .round_results
            .players
            .iter()
            .any(|player| player.player_info_id == loser_info_id && player.score_new.is_some()),
        "retirement evaluates the loser before removing the live player"
    );
}

#[test]
fn monster_rescue_mage_opens_and_casts_the_shipped_bridge_spell() {
    let mut engine = load_installed_scenario("Races.c4f/MonsterRescue.c4s", 0);
    let owner = join_local_player(&mut engine, "Monster Rescue magic parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Monster Rescue joins its Scenario.txt MAGE");
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("joined mage remains live")
            .definition_id,
        "MAGE"
    );

    // MAGE inherits MagiClonk::ContextMagic through SCLK -> MCLK. C++ adds
    // that annotated Context* function to the player's object menu and calls
    // ReadyToMagic(menu crew, MCMS) before exposing it
    // (MagiClonk.c4d/Script.c:190-199; C4ObjectMenu.cpp:670-682).
    let entries = engine
        .context_menu_entries(mage)
        .expect("the real mage context menu builds");
    assert!(
        entries
            .iter()
            .any(|entry| entry.function == "ContextMagic"),
        "the installed MagiClonk ContextMagic action is visible: {entries:?}"
    );

    // Monster Rescue's shipped JoinPlayer gives the Magus 30 energy and then
    // caps its temporary Magic physical at the matching 30000 before putting
    // it into MONS (Script.c:55-70). This is already enough for its sole MBRG
    // spell (Scenario.txt:18-20; MBRG DefCore Value=10).
    let energy_before = engine
        .object_snapshot(mage)
        .expect("mage snapshot after real scenario initialization")
        .magic_energy;
    assert_eq!(energy_before, 30_000);
    let mage_index = engine.find_object_index(mage).expect("mage index");
    assert_eq!(
        engine
            .call_object_function(
                mage_index,
                "CheckMagicRequirements",
                vec![Value::C4Id("MBRG".to_string()), Value::Bool(true)],
            )
            .expect("the real spell requirement check runs"),
        Value::Int(3),
        "30 energy permits exactly three Value=10 MBRG casts"
    );

    assert!(
        engine
            .execute_context_menu(mage, "ContextMagic")
            .expect("the real ContextMagic callback runs"),
        "ContextMagic reports that it opened the spell menu"
    );
    let (_, menu) = engine
        .cursor_object_menu(owner)
        .expect("ContextMagic opens the real script-created spell menu");
    assert_eq!(
        menu.items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["MBRG"],
        "OpenSpellMenu enumerates Monster Rescue's real player magic list"
    );
    let spell_command = menu.items[0].command.clone();

    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw enters the selected MBRG menu item");
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("mage begins casting")
            .action
            .name,
        "Magic",
        "the real menu command `{spell_command}` starts DoMagic; menu now {:?}, locals {:?}",
        engine.cursor_object_menu(owner).map(|(_, menu)| menu.clone()),
        engine
            .object_snapshot(mage)
            .expect("mage snapshot for failed cast diagnostics")
            .local_vars
    );

    // Magic's Delay=1 PhaseCall invokes CheckMagic after each phase advance;
    // phase five creates MBRG. Its shipped Activate creates FBRG; FBRG's own
    // Initialize immediately expands into four persistent FBRS segments and
    // removes both temporary bridge/spell objects.
    for _ in 0..8 {
        engine.tick().expect("the real magic action advances");
    }
    let snapshot = engine.snapshot();
    let magic_objects = snapshot
        .objects
        .iter()
        .filter(|object| matches!(object.definition_id.as_str(), "MBRG" | "FBRG" | "FBRS"))
        .map(|object| {
            (
                object.id,
                object.definition_id.clone(),
                object.status,
                object.owner,
                object.action.name.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        snapshot
            .objects
            .iter()
            .filter(|object| object.definition_id == "FBRS" && object.status.is_active())
            .count(),
        4,
        "the shipped MBRG -> FBRG Initialize route creates four live bridge segments; magic objects: {magic_objects:?}; mage: {:?}",
        snapshot.object(mage)
    );
    assert_eq!(
        snapshot
            .object(mage)
            .expect("mage survives the cast")
            .magic_energy,
        energy_before - 10_000,
        "ExecMagic deducts the spell's DefCore Value after successful Activate"
    );
}

#[test]
fn alchemy_mage_uses_context_magic_and_casts_the_shipped_gravity_spells() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy magic parity");
    // Scenario.txt creates CLNK followed by MCLK. C4ObjectList::Add with
    // stMain ordering puts the newest equal-rank crew first, so C4Player's
    // initial cursor is the mage (C4ObjectList.cpp:110-195;
    // C4Player.cpp:1003-1020; Alchemy.c4s/Scenario.txt:17-19).
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with a crew cursor");
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("Alchemy's selected mage remains live")
            .definition_id,
        "MCLK"
    );

    // InitializePlayer places one seeded alchemy bag beside AHUT. Its Activate
    // callback delegates the ingredient move to the already attached MCLK
    // bag's Transfer callback (Bag.c4d/Script.c:5-14,148-160). Invoke that
    // shipped delegation target directly so this test isolates spell-system
    // parity from loose-item collection/activation.
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("IROC").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy InitializePlayer creates its seeded ingredient bag");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK keeps its attached alchemy bag");
    let attached_bag_index = engine
        .find_object_index(attached_bag)
        .expect("attached bag index");
    engine
        .call_object_function(
            attached_bag_index,
            "Transfer",
            vec![Value::Object(seeded_bag.as_u64())],
        )
        .expect("the shipped attached-bag callback transfers its ingredients");
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC").copied()),
        Some(3),
        "the shipped loose bag supplies the rock ingredient used by MGUP"
    );
    assert_eq!(
        engine
            .object_snapshot(seeded_bag)
            .and_then(|bag| bag.components.get("IROC").copied()),
        Some(0),
        "TransferAlchem moves rather than duplicates the shipped ingredients"
    );

    // With the default player ExtraData, iCombo and all quick-spell slots are
    // zero. Therefore Special is only the empty quick-spell route; the full
    // spell list is opened through ContextMagic (MagiClonk.c4d/Script.c:88-111,
    // 190-200), which C4ObjectMenu exposes as a selectable context action
    // (C4ObjectMenu.cpp:670-682).
    engine
        .player_in_com(owner, COM_SPECIAL, 0)
        .expect("Special dispatches to the selected MCLK");
    assert!(
        engine.cursor_object_menu(owner).is_none(),
        "Special must not silently substitute for the full ContextMagic menu"
    );
    assert!(
        engine
            .context_menu_entries(mage)
            .expect("MCLK context entries build")
            .iter()
            .any(|entry| entry.function == "ContextMagic"),
        "the player-visible MCLK context menu exposes spell selection"
    );
    assert!(
        engine
            .execute_context_menu(mage, "ContextMagic")
            .expect("selecting ContextMagic runs its shipped callback"),
        "ContextMagic reports that it opened the full spell menu"
    );

    let raise_gravity_index = engine
        .cursor_object_menu(owner)
        .expect("ContextMagic opens Alchemy's spell menu")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "MGUP")
        .expect("Alchemy's Scenario.txt magic list contains MGUP");
    for _ in 0..raise_gravity_index {
        engine
            .player_in_com(owner, COM_RIGHT, 0)
            .expect("Right navigates the spell menu");
    }
    assert_eq!(
        engine
            .cursor_object_menu(owner)
            .expect("spell menu remains open")
            .1
            .selection,
        raise_gravity_index as i32,
        "ordinary menu navigation selects MGUP"
    );

    let gravity_before = engine.physics().gravity;
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw enters the selected spell item");
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("MCLK begins its cast")
            .action
            .name,
        "Magic"
    );
    for _ in 0..8 {
        engine.tick().expect("the shipped Magic action advances");
    }
    assert_eq!(
        engine.physics().gravity,
        gravity_before + 20,
        "MGUP Activate raises gravity by the shipped 20-point increment"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC").copied()),
        Some(2),
        "a successful MGUP cast consumes its one IROC ingredient"
    );

    // MGDW uses the same named global effect as MGUP. C4Effect::New checks
    // the existing effect's FxGravChangeUSpellEffect callback, sees -3, and
    // delegates the new spell to FxGravChangeUSpellAdd. The two changes
    // therefore cancel inside one effect instead of installing a competing
    // second timer (GravitationDown.c4d/Script.c:64-81; C4Effect.cpp).
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("reopening Alchemy's shipped magic menu succeeds"));
    let lower_gravity_index = engine
        .cursor_object_menu(owner)
        .expect("the lower-gravity spell menu opens")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "MGDW")
        .expect("Alchemy's Scenario.txt magic list contains MGDW");
    engine
        .player_in_com(owner, COM_MENU_SELECT, lower_gravity_index as i32)
        .expect("the pointer selects MGDW by its menu index");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts the selected MGDW cast");
    for _ in 0..8 {
        engine.tick().expect("the shipped Magic action advances");
    }
    assert_eq!(
        engine.physics().gravity,
        gravity_before,
        "MGDW is absorbed by MGUP's effect and cancels its 20-point change"
    );
    let gravity_effects = engine
        .global_effects()
        .iter()
        .filter(|effect| effect.name == "GravChangeUSpell")
        .collect::<Vec<_>>();
    assert_eq!(
        gravity_effects.len(),
        1,
        "opposing gravity spells share one C++ global effect"
    );
    assert_eq!(gravity_effects[0].var(1), EffectVarValue::Int(0));
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC").copied()),
        Some(1),
        "the absorbed MGDW cast still consumes its one IROC ingredient"
    );

    // ABLA is Alchemy's shipped aimed spell. Its Activate delegates to
    // MCLK::DoSpellAim, which creates AIMR; AIMR::Create then switches the
    // cursor to itself, keeps camera focus on the mage, and clears the two
    // stale command latches (Airblast.c4d/Script.c:3-10;
    // Aimer.c4d/Script.c:24-51). The seeded bag carries exactly ABLA's
    // IASH=3 component requirement.
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("reopening Alchemy's shipped magic menu succeeds"));
    let airblast_index = engine
        .cursor_object_menu(owner)
        .expect("the second spell menu opens")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "ABLA")
        .expect("Alchemy's Scenario.txt magic list contains ABLA");
    engine
        .player_in_com(owner, COM_MENU_SELECT, airblast_index as i32)
        .expect("the pointer selects ABLA by its menu index");
    let (_, airblast_menu) = engine
        .cursor_object_menu(owner)
        .expect("ABLA spell menu remains open");
    assert_eq!(
        airblast_menu
            .items
            .get(airblast_menu.selection as usize)
            .map(|item| item.item_id.as_str()),
        Some("ABLA"),
        "menu selection targets ABLA before casting"
    );
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts the selected ABLA cast");

    let aimer = (0..12)
        .find_map(|_| {
            // Pin a stale command immediately before each object-execution
            // pass. On the activation pass AIMR::Create must clear the two
            // C++ latches before Players.Execute observes them.
            {
                let control = &mut engine
                    .player_mut(owner)
                    .expect("Alchemy player remains live")
                    .control;
                control.last_com = COM_RIGHT;
                control.last_com_delay = 17;
                control.last_com_down_double = 4;
            }
            engine.tick().expect("the ABLA Magic action advances");
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| {
                    object.definition_id == "AIMR"
                        && object.status.is_active()
                        && object.action.name == "Open"
                })
                .map(|object| object.id)
        })
        .unwrap_or_else(|| {
            panic!(
                "ABLA creates the shipped active AIMR controller; mage={:?}; AIMR={:?}; player={:?}",
                engine.object_snapshot(mage),
                engine
                    .snapshot()
                    .objects
                    .iter()
                    .filter(|object| object.definition_id == "AIMR")
                    .cloned()
                    .collect::<Vec<_>>(),
                engine.player(owner).map(|player| player.to_state()),
            )
        });
    assert_eq!(
        engine.crew_cursor(owner),
        Some(aimer),
        "AIMR::Create transfers keyboard control to the aiming object"
    );
    let player = engine
        .snapshot()
        .players
        .into_iter()
        .find(|player| player.id == owner)
        .expect("Alchemy player snapshot remains present");
    assert_eq!(
        player.viewports.first().and_then(|viewport| viewport.focus),
        Some(mage),
        "SetViewCursor follows the mage while AIMR owns the input cursor"
    );
    assert_eq!(player.control.last_com, 0);
    assert_eq!(player.control.last_com_down_double, 0);
    assert_eq!(
        player.control.last_com_delay, 17,
        "ClearLastPlrCom deliberately preserves LastComDelay like C++"
    );

    // C4Player::InCom routes each raw press through C4Object::CallControl
    // (C4Player.cpp:1490-1554; C4Object.cpp:3307-3325). The shipped AIMR
    // handlers then turn Up into a 20-degree step and Throw into DoEnter;
    // DoEnter restores the mage cursor/view before MCLK::OnAimerEnter calls
    // ABLA::ActivateAngle (Aimer.c4d/Script.c:184-270;
    // Clonk.c4d/Script.c:1002-1013; Airblast.c4d/Script.c:30-48).
    engine
        .player_in_com(owner, COM_UP, 0)
        .expect("Up steers the shipped AIMR");
    assert_eq!(
        engine
            .object_snapshot(aimer)
            .expect("AIMR remains live while steering")
            .local_vars
            .get("iAngle"),
        Some(&Value::Int(70)),
        "left-facing ABLA starts at 90 degrees and Up steps toward zero"
    );
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("mage remains live while aiming")
            .action
            .name,
        "AimMagic",
        "AimingAngle switches the mage into the shipped aiming action"
    );

    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw accepts the shipped AIMR angle");
    assert_eq!(
        engine.crew_cursor(owner),
        Some(mage),
        "AIMR::Close restores the mage as keyboard cursor"
    );
    assert!(
        engine
            .object_snapshot(aimer)
            .is_none_or(|aimer| !aimer.status.is_active()),
        "accepting the aim deactivates the AIMR controller: {:?}",
        engine.object_snapshot(aimer)
    );
    let player = engine
        .snapshot()
        .players
        .into_iter()
        .find(|player| player.id == owner)
        .expect("Alchemy player remains present after the cast");
    assert_eq!(
        player.viewports.first().and_then(|viewport| viewport.focus),
        None,
        "AIMR::Close resets the temporary view cursor"
    );
    assert!(
        engine
            .global_effects()
            .iter()
            .any(|effect| effect.name == "AirblastNSpell"),
        "ABLA::ActivateAngle installs the shipped global airblast effect"
    );
    assert_eq!(
        engine.environment().wind,
        38,
        "the mage stands in tunnel background, so GetWind() is locally zero before Sin(70, 40)"
    );
}

#[test]
fn alchemy_warp_to_base_cast_builds_the_real_portal_pair_and_transfers_the_mage() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy warp parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");

    // ExecBase runs on Tick10 and claims AHUT for this player once its FLAG
    // has settled. MWP2 deliberately fails before that claim; wait for the
    // same C++ base lifecycle rather than manufacturing a shortcut.
    let home = (0..20)
        .find_map(|_| {
            engine.tick().expect("Alchemy base lifecycle advances");
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| object.definition_id == "AHUT" && object.base == owner)
                .map(|object| object.id)
        })
        .expect("Alchemy's FLAG claims its AHUT on the C++ Tick10 cadence");
    for _ in 0..160 {
        if engine
            .object_snapshot(mage)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        {
            break;
        }
        engine.tick().expect("Alchemy ready-crew Exit advances");
    }
    assert!(
        engine
            .object_snapshot(mage)
            .is_some_and(|object| { object.container.is_none() && object.action.name == "Walk" }),
        "C4Player::PlaceReadyCrew's queued Exit puts the mage on walkable ground"
    );

    // The starter bag has IMUS=4 and IGOL=3; MWP2 costs IMUS=3, IGOL=4.
    // Transfer it plus one harvested gold ingredient through the real ALC_
    // callback (Alchemy.c4s/Script.c:21-37; WarpToBase.c4d/DefCore.txt).
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("IMUS").copied() == Some(4)
                && object.components.get("IGOL").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy creates its seeded warp ingredients");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK keeps its attached alchemy bag");
    let harvested_gold = engine
        .spawn_object(
            SpawnConfig::new("ALC_").with_ordered_components(vec![("IGOL".to_owned(), 1)]),
        )
        .expect("a harvested gold ingredient bag spawns");
    let attached_bag_index = engine
        .find_object_index(attached_bag)
        .expect("attached bag index");
    for source in [seeded_bag, harvested_gold] {
        engine
            .call_object_function(
                attached_bag_index,
                "Transfer",
                vec![Value::Object(source.as_u64())],
            )
            .expect("the shipped bag callback transfers warp ingredients");
    }
    let bag = engine
        .object_snapshot(attached_bag)
        .expect("attached bag remains live");
    assert_eq!(bag.components.get("IMUS"), Some(&4));
    assert_eq!(bag.components.get("IGOL"), Some(&4));
    let mage_index = engine.find_object_index(mage).expect("mage index");
    assert!(
        engine
            .call_object_function(
                mage_index,
                "CheckMagicRequirements",
                vec![Value::C4Id("MWP2".to_owned()), Value::Bool(true)],
            )
            .expect("the shipped requirement callback runs")
            .as_bool(),
        "the attached bag satisfies MWP2 before the player casts"
    );

    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let warp_index = engine
        .cursor_object_menu(owner)
        .expect("Alchemy's spell menu opens")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "MWP2")
        .expect("Alchemy's Scenario.txt magic list contains MWP2");
    engine
        .player_in_com(owner, COM_MENU_SELECT, warp_index as i32)
        .expect("the pointer selects MWP2 by menu index");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts MWP2's Magic action");
    for _ in 0..8 {
        engine.tick().expect("MWP2's Magic action advances");
    }

    let bag_after_cast = engine
        .object_snapshot(attached_bag)
        .expect("attached bag survives the cast");
    assert_eq!(
        (
            bag_after_cast.components.get("IMUS").copied(),
            bag_after_cast.components.get("IGOL").copied(),
        ),
        (Some(1), Some(0)),
        "MWP2 must complete through ExecMagic before portal validation; mage={:?}",
        (engine.object_snapshot(mage), engine.object_snapshot(home),)
    );

    let portals = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "WARP" && object.status.is_active())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(portals.len(), 2, "MWP2 creates its connected WARP pair");
    assert!(
        portals
            .iter()
            .any(|portal| portal.action.target2 == Some(home)),
        "the destination portal retains AHUT as its entrance target: {portals:?}"
    );
    let start_portal = portals
        .iter()
        .find(|portal| portal.action.target.is_some())
        .expect("the source WARP targets its paired destination");

    // Fast-forward the source aperture's purely visual 7×64-tick growth and
    // put the mage inside it. This keeps the suite fast while still exercising
    // WARP::FxWarpUSpellTimer,
    // WarpUSpellData, vertex removal/restoration, and TransferWarpObject's
    // entrance path rather than replacing them with a direct Enter call here.
    engine
        .apply_object_update(
            start_portal.id,
            ObjectUpdate::new().with_construction(FULL_CON),
        )
        .expect("fast-forward the source portal's visual growth");
    let start_portal_index = engine
        .find_object_index(start_portal.id)
        .expect("source portal index");
    engine
        .call_object_function(start_portal_index, "Shrink", vec![])
        .expect("the source portal's real final growth step activates it");
    engine
        .apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(start_portal.position)
                .with_velocity(Vector2::ZERO)
                .clear_container(),
        )
        .expect("place the mage inside the source warp aperture");

    let transferred = (0..30).any(|_| {
        engine.tick().expect("the real WARP pair advances");
        engine
            .object_snapshot(mage)
            .is_some_and(|object| object.container == Some(home))
    });
    assert!(
        transferred,
        "WarpUSpell must pull the mage through the start portal and enter AHUT; mage={:?}; portals={:?}",
        engine.object_snapshot(mage),
        engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.definition_id == "WARP")
            .cloned()
            .collect::<Vec<_>>()
    );
    let warped_mage = engine
        .object_snapshot(mage)
        .expect("the warped mage remains live");
    assert_eq!(
        warped_mage.vertices.len(),
        7,
        "WarpUSpellData Stop restores every CLNK shape vertex"
    );
    assert!(
        warped_mage
            .effects
            .iter()
            .all(|effect| effect.name != "WarpUSpellData"),
        "the per-object warp bookkeeping effect is removed after transfer"
    );
}

#[test]
fn alchemy_reincarnation_spell_revives_its_mage_during_assign_death() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy reincarnation parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");
    engine
        .apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        )
        .expect("place MCLK safely in open sky before the death transition");

    // Alchemy seeds INEC=1 and IASH=3, while XCRS consumes INEC=2 and
    // IASH=4. Transfer the real starter bag plus one harvested unit of each
    // through ALC_::Transfer (Alchemy.c4s/Script.c:21-37;
    // Reincarnation.c4d/DefCore.txt:7; Bag.c4d/Script.c:148-160).
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("INEC").copied() == Some(1)
                && object.components.get("IASH").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy creates its seeded reincarnation ingredients");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK keeps its attached alchemy bag");
    let extra_ingredients = engine
        .spawn_object(
            SpawnConfig::new("ALC_").with_ordered_components(vec![
                ("INEC".to_owned(), 1),
                ("IASH".to_owned(), 1),
            ]),
        )
        .expect("a harvested ingredient bag spawns");
    let attached_bag_index = engine
        .find_object_index(attached_bag)
        .expect("attached bag index");
    for source in [seeded_bag, extra_ingredients] {
        engine
            .call_object_function(
                attached_bag_index,
                "Transfer",
                vec![Value::Object(source.as_u64())],
            )
            .expect("the shipped bag callback transfers XCRS's ingredients");
    }

    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let reincarnation_index = engine
        .cursor_object_menu(owner)
        .expect("ContextMagic opens Alchemy's spell menu")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "XCRS")
        .expect("Alchemy's Scenario.txt magic list contains XCRS");
    engine
        .player_in_com(owner, COM_MENU_SELECT, reincarnation_index as i32)
        .expect("the menu selects XCRS");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts XCRS's Magic action");
    for _ in 0..8 {
        engine.tick().expect("XCRS's Magic action advances");
    }
    let protected = engine
        .object_snapshot(mage)
        .expect("the protected mage remains live");
    assert_eq!(protected.energy, 45_000, "XCRS sacrifices ten energy");
    assert!(protected
        .effects
        .iter()
        .any(|effect| effect.name == "ReincarnationPSpell"));
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("INEC").copied()),
        Some(0),
        "a successful XCRS cast consumes its two-nectar recipe"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IASH").copied()),
        Some(0),
        "a successful XCRS cast consumes its four-ash recipe"
    );

    // C4Object::AssignDeath sets Alive=false, clears effects with
    // C4FxCall_RemoveDeath, and aborts ordinary death if an effect revives
    // the object (C4Object.cpp:1162-1180). XCRS's Stop callback restores
    // Alive, denies removal, and installs IntReincDelay
    // (Reincarnation.c4d/Script.c:34-58).
    let mage_index = engine.find_object_index(mage).expect("live mage index");
    engine.change_object_energy(mage_index, -100, 0, -1);
    let reincarnating = engine
        .object_snapshot(mage)
        .expect("the reincarnating mage remains present");
    assert!(reincarnating.alive, "XCRS revives MCLK during AssignDeath");
    assert_eq!(
        reincarnating.action.name, "Dead",
        "FxIntReincDelayStart puts the revived mage into its shipped death pose: {reincarnating:?}"
    );
    assert!(reincarnating
        .effects
        .iter()
        .any(|effect| effect.name == "ReincarnationPSpell"));
    assert!(reincarnating
        .effects
        .iter()
        .any(|effect| effect.name == "IntReincDelay"));
}

#[test]
fn alchemy_learned_group_heal_cast_sustains_magic_and_heals_nearby_crew() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy group-heal parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");
    let patient = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "CLNK"
                && object.owner == owner
                && object.status.is_active()
        })
        .map(|object| object.id)
        .expect("Alchemy also joins with a regular CLNK");
    for (object, position) in [
        (mage, Vector2::new(500, 200)),
        (patient, Vector2::new(530, 200)),
    ] {
        engine
            .apply_object_update(
                object,
                ObjectUpdate::new()
                    .with_position(position)
                    .with_velocity(Vector2::ZERO)
                    .with_action("Walk")
                    .clear_container(),
            )
            .expect("place both crew outdoors inside GGHG's range");
    }
    engine.change_object_energy(
        engine.find_object_index(patient).expect("patient index"),
        -20,
        0,
        -1,
    );
    let energy_before = engine
        .object_snapshot(patient)
        .expect("the injured CLNK remains live")
        .energy;
    assert_eq!(energy_before, 35_000);

    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("IMUS").copied() == Some(4)
                && object.components.get("IGOL").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy creates GGHG's seeded ingredients");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK keeps its attached alchemy bag");
    engine
        .call_object_function(
            engine
                .find_object_index(attached_bag)
                .expect("attached bag index"),
            "Transfer",
            vec![Value::Object(seeded_bag.as_u64())],
        )
        .expect("the shipped bag callback transfers GGHG's ingredients");

    engine
        .grant_player_magic(owner, "GGHG")
        .expect("the Alchemy player learns GGHG from a scroll");
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let heal_index = engine
        .cursor_object_menu(owner)
        .expect("ContextMagic opens Alchemy's spell menu")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "GGHG")
        .expect("the learned GGHG spell is selectable");
    engine
        .player_in_com(owner, COM_MENU_SELECT, heal_index as i32)
        .expect("the menu selects GGHG");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts GGHG's Magic action");
    for _ in 0..50 {
        engine.tick().expect("GGHG's healing effect advances");
    }

    let caster = engine
        .object_snapshot(mage)
        .expect("the healing mage remains live");
    assert_eq!(caster.action.name, "Magic");
    assert!(caster
        .effects
        .iter()
        .any(|effect| effect.name == "GroupHealPSpell"));
    let healed = engine
        .object_snapshot(patient)
        .expect("the patient remains live");
    assert!(
        healed.energy > energy_before,
        "GGHG repeatedly heals friendly crew within 80 pixels: caster={caster:?}; patient={healed:?}"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IMUS").copied()),
        Some(1),
        "a successful GGHG cast consumes three mushrooms"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IGOL").copied()),
        Some(2),
        "a successful GGHG cast consumes one gold"
    );
}

#[test]
fn alchemy_make_artefact_cast_opens_the_real_enchantment_menu() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy artefact parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK cursor");

    // The shipped loose bag contains three IGOL; transfer it through the
    // attached ALC_ callback so the real NMGE rule can pay MART's one-gold
    // recipe (Alchemy.c4s/Script.c:18-30; Artefact.c4d/DefCore.txt:7-9).
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("IGOL").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy creates its seeded ingredient bag");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK owns its attached ingredient bag");
    engine
        .call_object_function(
            engine
                .find_object_index(attached_bag)
                .expect("attached bag index"),
            "Transfer",
            vec![Value::Object(seeded_bag.as_u64())],
        )
        .expect("the shipped bag callback transfers MART's ingredients");

    // MART enchants Contents(0, mage); use a real carried FLNT and teach the
    // scroll-discoverable spell, as the scenario's random scrolls do during
    // normal play (Alchemy.c4s/Script.c:5-16; C4Player.cpp:1052-1058).
    let carried = engine
        .spawn_object(
            SpawnConfig::new("FLNT")
                .with_owner(owner)
                .with_container(mage),
        )
        .expect("real FLNT enters the mage inventory");
    engine
        .grant_player_magic(owner, "MART")
        .expect("the Alchemy player learns MART");
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens the shipped spell menu"));
    let mart_index = engine
        .cursor_object_menu(owner)
        .expect("Alchemy spell menu remains open")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "MART")
        .expect("the learned MART spell is selectable");
    engine
        .player_in_com(owner, COM_MENU_SELECT, mart_index as i32)
        .expect("the pointer selects MART");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts the selected MART cast");
    for _ in 0..8 {
        engine.tick().expect("MART's Magic action advances");
    }

    let (_, menu) = engine
        .cursor_object_menu(owner)
        .expect("MART::Activate opens its real enchantment-class menu");
    assert_eq!(menu.identification, Value::C4Id("MCMS".into()));
    assert!(
        !menu.items.is_empty(),
        "MagicMenu enumerates the installed spell classes"
    );
    let artefact_spell = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "MART" && object.status.is_active())
        .cloned()
        .expect("MART remains live while its menu is open");
    assert_eq!(
        artefact_spell.local_vars.get("iMagicAmount"),
        Some(&Value::Int(5)),
        "GetValue() returns MART's DefCore value for cancellation accounting"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IGOL").copied()),
        Some(2),
        "the successful MART cast consumes one shipped gold ingredient"
    );
    assert_eq!(
        engine
            .object_snapshot(carried)
            .and_then(|object| object.container),
        Some(mage),
        "opening MART's mode selector does not consume the artefact target"
    );
}

#[test]
fn alchemy_make_artefact_hit_mode_casts_the_selected_spell_after_throw() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy artefact activation parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK cursor");

    // MART consumes its own IGOL recipe before Activate. LGCN then consumes
    // IMUS+IASH while SetMagic enchants Contents(0, mage), exactly as the
    // shipped ALC_/NMGE callbacks do (Alchemy.c4s/Script.c:18-30;
    // Artefact.c4d/Script.c:211-264).
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("IGOL").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy creates its seeded ingredient bag");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK owns its attached ingredient bag");
    engine
        .call_object_function(
            engine
                .find_object_index(attached_bag)
                .expect("attached bag index"),
            "Transfer",
            vec![Value::Object(seeded_bag.as_u64())],
        )
        .expect("the shipped bag callback transfers the artefact ingredients");
    let carried = engine
        .spawn_object(
            SpawnConfig::new("ROCK")
                .with_owner(owner)
                .with_container(mage),
        )
        .expect("a real ROCK enters the mage inventory");
    for spell in ["MART", "LGCN"] {
        engine
            .grant_player_magic(owner, spell)
            .expect("the Alchemy player learns the tested spell");
    }

    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens the shipped spell menu"));
    let mart_index = engine
        .cursor_object_menu(owner)
        .expect("Alchemy spell menu remains open")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "MART")
        .expect("the learned MART spell is selectable");
    engine
        .player_in_com(owner, COM_MENU_SELECT, mart_index as i32)
        .expect("the pointer selects MART");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts the selected MART cast");
    for _ in 0..8 {
        engine.tick().expect("MART's Magic action advances");
    }

    // C4Menu::Enter executes AddMenuItem's command on MART's command object
    // (C4ObjectMenu.cpp:505-527). Select AIR1, then the learned LGCN spell,
    // hit activation, no delay, and ally target through those real controls
    // (Artefact.c4d/Script.c:198-218,266-421).
    for item_id in ["AIR1", "LGCN", "FXQ1", "FXP1", "WIPF"] {
        let index = engine
            .cursor_object_menu(owner)
            .unwrap_or_else(|| panic!("MART menu for {item_id} remains open"))
            .1
            .items
            .iter()
            .position(|item| item.item_id == item_id)
            .unwrap_or_else(|| panic!("MART menu exposes {item_id}"));
        engine
            .player_in_com(owner, COM_MENU_SELECT, index as i32)
            .unwrap_or_else(|error| panic!("the pointer selects {item_id}: {error}"));
        engine
            .player_in_com(owner, COM_THROW, 0)
            .unwrap_or_else(|error| panic!("Throw enters {item_id}: {error}"));
    }
    assert!(
        engine.cursor_object_menu(owner).is_none(),
        "the target choice finishes MART's configuration menus"
    );

    let enchanted = engine
        .object_snapshot(carried)
        .expect("the configured ROCK remains live");
    let artefact = enchanted
        .effects
        .iter()
        .find(|effect| effect.name == "ArtefactNSpell")
        .expect("SetSpell installs MART's shipped object effect");
    assert_eq!(
        artefact.vars.first(),
        Some(&EffectVarValue::C4Id("LGCN".into())),
        "FxArtefactNSpellStart stores the selected spell"
    );
    assert_eq!(
        artefact.vars.get(2),
        Some(&EffectVarValue::Int(0)),
        "SetMode stores C++ hit activation"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IMUS").copied()),
        Some(3),
        "SetMagic consumes one of the shipped bag's four mushrooms"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IASH").copied()),
        Some(2),
        "SetMagic consumes one of the shipped bag's three ashes"
    );

    // Mode0 arms while ROCK is in flight and casts when it next contacts the
    // landscape at low speed (Artefact.c4d/Script.c:488-509). Exercise the
    // normal CLNK Throw control and simulation callback, rather than calling
    // Mode0/CastSpell directly.
    for _ in 0..20 {
        engine.tick().expect("the mage leaves its Magic action");
    }
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("MCLK throws the configured ROCK");
    for _ in 0..240 {
        engine.tick().expect("the artefact throw advances");
        if engine.snapshot().objects.iter().any(|object| {
            object.definition_id == "LGCN"
                && object
                    .effects
                    .iter()
                    .any(|effect| effect.name == "LightningConcealment")
        }) {
            return;
        }
    }
    panic!(
        "C++ Mode0 must cast LGCN after the thrown ROCK hits; ROCK={:?}",
        engine.object_snapshot(carried)
    );
}

#[test]
fn alchemy_seeded_bag_collects_and_activates_through_player_controls() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy ingredient pickup parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK cursor");
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("IROC").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("InitializePlayer creates the filled loose bag by AHUT");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("Recruitment attaches an empty alchemy bag to MCLK");
    assert_eq!(
        engine
            .call_object_function(
                engine.find_object_index(mage).expect("live MCLK index"),
                "CheckMagicRequirements",
                vec![Value::C4Id("MGUP".into()), Value::Bool(true)],
            )
            .expect("silent pre-pickup requirement check runs"),
        Value::Nil,
        "the empty attached bag cannot pay MGUP's one-IROC recipe"
    );

    // C++ exits a contained crew member on Down, automatically collects a
    // carryable inside MCLK's collection rectangle on Tick3, then turns two
    // Dig presses into ObjectComDigDouble. That activates the first carried
    // object, so ALC_::Activate transfers the loose bag into the hidden bag
    // (C4Object.cpp:3267-3272; C4GameObjects.cpp:140-197;
    // C4ObjectCom.cpp:531-540; Bag.c4d/Script.c:5-25,157-169).
    engine
        .player_in_com(owner, COM_DOWN, 0)
        .expect("Down queues the normal structure exit");
    for _ in 0..20 {
        if engine
            .object_snapshot(mage)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        engine.tick().expect("execute the normal exit command");
    }
    assert!(
        engine
            .object_snapshot(mage)
            .is_some_and(|object| object.container.is_none()),
        "MCLK exits AHUT through its ordinary Down control"
    );

    let bag_position = engine
        .object_snapshot(seeded_bag)
        .expect("seeded bag remains beside AHUT")
        .position;
    engine
        .apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(bag_position)
                .with_velocity(Vector2::ZERO)
                .with_action("Walk"),
        )
        .expect("put MCLK's collection rectangle over the loose bag");
    for _ in 0..3 {
        engine.tick().expect("run through the Tick3 collection pass");
    }
    assert_eq!(
        engine
            .object_snapshot(seeded_bag)
            .expect("collected bag remains live")
            .container,
        Some(mage),
        "the loose scenario bag enters MCLK through automatic collection"
    );

    engine
        .player_in_com(owner, COM_DIG, 0)
        .expect("first Dig arms the double-click buffer");
    engine
        .player_in_com(owner, COM_DIG, 0)
        .expect("second Dig activates the first inventory object");
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC").copied()),
        Some(3),
        "ALC_::Activate transfers the scenario ingredients into MCLK's hidden bag"
    );
    assert_eq!(
        engine
            .object_snapshot(seeded_bag)
            .and_then(|bag| bag.components.get("IROC").copied()),
        Some(0),
        "the player route moves rather than duplicates the seeded ingredients"
    );
    assert_eq!(
        engine
            .call_object_function(
                engine.find_object_index(mage).expect("live MCLK index"),
                "CheckMagicRequirements",
                vec![Value::C4Id("MGUP".into()), Value::Bool(true)],
            )
            .expect("silent post-transfer requirement check runs"),
        Value::Int(3),
        "the spell system finds all three IROC in MCLK's attached bag"
    );
}

#[test]
fn alchemy_possession_uses_the_shipped_selector_control() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy selector parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");
    let mage_position = engine
        .object_snapshot(mage)
        .expect("Alchemy mage remains live")
        .position;
    let possession = engine
        .spawn_object(
            lc_engine::SpawnConfig::new("POSE")
                .with_position(mage_position)
                .with_owner(owner),
        )
        .expect("shipped POSE spell spawns");
    let mage_index = engine.find_object_index(mage).expect("mage index");

    // C4Object::Call routes the spell's DoSpellSelect into SLCR creation;
    // C4Player::InCom then sends Right/Throw to SLCR's Control* callbacks
    // (C4Object.cpp:3229-3325; C4Player.cpp:1490-1554;
    // Selector.c4d/Script.c:6-43,128-174).
    let selector_value = engine
        .call_object_function(
            mage_index,
            "DoSpellSelect",
            vec![
                Value::Object(possession.as_u64()),
                Value::Int(400),
                Value::Object(mage.as_u64()),
            ],
        )
        .expect("MCLK starts the shipped selector");
    let selector = match selector_value {
        Value::Object(raw) => ObjectId::new(raw),
        other => panic!("DoSpellSelect returns SLCR, got {other:?}"),
    };
    assert_eq!(engine.crew_cursor(owner), Some(selector));
    let target_count = engine
        .call_object_function(
            engine.find_object_index(selector).expect("selector index"),
            "CountTargets",
            Vec::new(),
        )
        .expect("SLCR counts its shipped target list");
    assert!(
        matches!(target_count, Value::Int(2..=8)),
        "Alchemy has multiple nearby possessible animals within SLCR's eight-target cap: {target_count:?}"
    );

    engine
        .player_in_com(owner, COM_RIGHT, 0)
        .expect("Right cycles the shipped selector");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw accepts the shipped selector target");
    assert_eq!(engine.crew_cursor(owner), Some(mage));
    assert!(
        engine
            .object_snapshot(selector)
            .is_none_or(|selector| !selector.status.is_active()),
        "accepting a selector target deactivates SLCR"
    );
    assert!(
        engine.snapshot().objects.iter().any(|target| {
            target
                .effects
                .iter()
                .any(|effect| effect.name == "PossessionSpell")
        }),
        "POSE::ActivateTarget installs PossessionSpell on the selected animal"
    );
}

#[test]
fn alchemy_combo_mode_opens_and_accepts_the_shipped_element_control() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy combo parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");
    let mage_index = engine.find_object_index(mage).expect("mage index");

    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("IROC").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy creates its seeded ingredient bag");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK keeps its attached alchemy bag");
    engine
        .call_object_function(
            engine
                .find_object_index(attached_bag)
                .expect("attached bag index"),
            "Transfer",
            vec![Value::Object(seeded_bag.as_u64())],
        )
        .expect("the shipped bag callback transfers MGUP's IROC ingredient");

    engine
        .call_object_function(
            mage_index,
            "ContextCombo",
            vec![Value::Object(mage.as_u64())],
        )
        .expect("the shipped context action enables combo mode");
    assert_eq!(
        engine
            .object_snapshot(mage)
            .and_then(|mage| mage.local_vars.get("iCombo").cloned()),
        Some(Value::Int(1))
    );

    // MCLK::ControlSpecial creates CBMU, which becomes the cursor. MGUP is an
    // Earth spell (class key "2") with combo "255", so the full shipped code
    // is "2255". CheckSpells auto-completes its last key when "225" leaves
    // MGUP as the sole candidate (C4Player.cpp:1490-1554;
    // MagiClonk.c4d/Script.c:87-95,482-495;
    // ComboMenu.c4d/Script.c:33-50,138-174,336-390;
    // GravitationUp.c4d/Script.c:50-51).
    engine
        .player_in_com(owner, COM_SPECIAL, 0)
        .expect("Special opens the shipped combo selector");
    let combo = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "CBMU" && object.status.is_active())
        .map(|object| object.id)
        .expect("combo mode creates live CBMU");
    assert_eq!(engine.crew_cursor(owner), Some(combo));

    engine
        .player_in_com(owner, lc_engine::COM_DOWN, 0)
        .expect("Down chooses the Earth element");
    let combo_snapshot = engine
        .object_snapshot(combo)
        .expect("CBMU remains live after its first key");
    assert_eq!(
        combo_snapshot.local_vars.get("szCastKeys"),
        Some(&Value::String("2".into()))
    );
    assert_eq!(
        combo_snapshot.local_vars.get("iCastControlCount"),
        Some(&Value::Int(1))
    );

    engine
        .player_in_com(owner, lc_engine::COM_DOWN, 0)
        .expect("Down chooses MGUP's first spell key");
    assert_eq!(
        engine
            .object_snapshot(combo)
            .expect("CBMU remains live after the second key")
            .local_vars
            .get("szCastKeys"),
        Some(&Value::String("22".into()))
    );

    let gravity_before = engine.physics().gravity;
    engine
        .player_in_com(owner, COM_UP, 0)
        .expect("Up uniquely completes the shipped MGUP combo");
    assert_eq!(
        engine.crew_cursor(owner),
        Some(mage),
        "CBMU::Close restores the mage cursor before OnComboMenuEnter"
    );
    assert!(
        engine
            .object_snapshot(combo)
            .is_none_or(|combo| !combo.status.is_active()),
        "the completed combo removes CBMU"
    );
    assert_eq!(
        engine
            .object_snapshot(mage)
            .and_then(|mage| mage.local_vars.get("pComboMenu").cloned()),
        Some(Value::Nil),
        "C4Object::AssignRemoval clears the mage's live reference to CBMU"
    );
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("mage starts the combo-selected spell")
            .action
            .name,
        "Magic"
    );

    for _ in 0..8 {
        engine.tick().expect("the shipped Magic action advances");
    }
    assert_eq!(
        engine.physics().gravity,
        gravity_before + 20,
        "the CBMU-selected MGUP executes its shipped gravity effect"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC").copied()),
        Some(2),
        "the combo cast consumes MGUP's one IROC ingredient"
    );
}

#[test]
fn alchemy_learned_lightning_cast_launches_the_shipped_line_object() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy lightning parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");

    // Alchemy's InitializePlayer puts MLGT's IBON=2 recipe into the seeded
    // loose bag. Move that recipe into MCLK's attached bag through the same
    // ALC_::Transfer callback used by gameplay (Alchemy.c4s/Script.c:21-37;
    // Lightning.c4d/DefCore.txt:11; Bag.c4d/Script.c:148-160).
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_" && object.components.get("IBON").copied() == Some(2)
        })
        .map(|object| object.id)
        .expect("Alchemy creates its seeded IBON bag");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK keeps its attached alchemy bag");
    engine
        .call_object_function(
            engine
                .find_object_index(attached_bag)
                .expect("attached bag index"),
            "Transfer",
            vec![Value::Object(seeded_bag.as_u64())],
        )
        .expect("the shipped bag callback transfers MLGT's bones");

    // Alchemy omits MLGT from its initial Scenario.txt list and teaches
    // random C4D_Magic definitions through SCRL. Granting that learned entry
    // here starts at the same C4Player magic-list state reached after reading
    // an MLGT scroll (Alchemy.c4s/Script.c:5-16; C4Player.cpp:1052-1058).
    engine
        .grant_player_magic(owner, "MLGT")
        .expect("the Alchemy player learns MLGT");
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let lightning_index = engine
        .cursor_object_menu(owner)
        .expect("ContextMagic opens Alchemy's spell menu")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "MLGT")
        .expect("the learned lightning spell is selectable");
    engine
        .player_in_com(owner, COM_MENU_SELECT, lightning_index as i32)
        .expect("the menu selects MLGT");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts MLGT's Magic action");

    let aimer = (0..12)
        .find_map(|_| {
            engine.tick().expect("MLGT's Magic action advances");
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| object.definition_id == "AIMR" && object.status.is_active())
                .map(|object| object.id)
        })
        .expect("MLGT::Activate delegates to MCLK::DoSpellAim");
    assert_eq!(engine.crew_cursor(owner), Some(aimer));

    // AIMR::DoEnter calls MLGT::ActivateAngle. C++ creates LGTS, calls
    // Launch, and LGTS::Activate seeds the first vertex and Advance action
    // (Aimer.c4d/Script.c:242-270; Lightning.c4d/Script.c:22-35;
    // LightningShot.c4d/Script.c:12-34).
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw accepts MLGT's aimed angle");
    assert_eq!(engine.crew_cursor(owner), Some(mage));
    let lightning = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "LGTS" && object.status.is_active())
        .cloned()
        .expect("MLGT launches a live LGTS line object");
    assert_eq!(lightning.action.name, "Advance");
    assert!(
        !lightning.vertices.is_empty(),
        "LGTS::Activate seeds the cast origin as its first line vertex"
    );

    let vertex_count = lightning.vertices.len();
    for _ in 0..3 {
        engine.tick().expect("LGTS advances without a script error");
    }
    let advanced = engine
        .object_snapshot(lightning.id)
        .expect("LGTS remains live in open space while advancing");
    assert!(
        advanced.vertices.len() > vertex_count,
        "LGTS::Advance extends the lightning line: before={vertex_count}, after={:?}",
        advanced.vertices
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IBON").copied()),
        Some(0),
        "the successful MLGT cast consumes its shipped two-bone recipe"
    );
}

#[test]
fn alchemy_learned_icestrike_aims_steers_and_impacts_through_player_controls() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy icestrike parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");
    engine
        .apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        )
        .expect("place MCLK in open sky for the aimed flight");

    // Alchemy seeds ISPH=1 and IGOL=3, while MICS consumes ISPH=2 and
    // IGOL=1. Transfer the shipped bag plus one harvested sphere through
    // ALC_::Transfer, the same path used by ordinary play
    // (Alchemy.c4s/Script.c:21-37; Icestrike.c4d/DefCore.txt:7;
    // Bag.c4d/Script.c:148-160).
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("ISPH").copied() == Some(1)
                && object.components.get("IGOL").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy creates its seeded ingredient bag");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK keeps its attached alchemy bag");
    let extra_sphere = engine
        .spawn_object(
            SpawnConfig::new("ALC_")
                .with_ordered_components(vec![("ISPH".to_owned(), 1)]),
        )
        .expect("a harvested sphere bag spawns");
    let attached_bag_index = engine
        .find_object_index(attached_bag)
        .expect("attached bag index");
    for source in [seeded_bag, extra_sphere] {
        engine
            .call_object_function(
                attached_bag_index,
                "Transfer",
                vec![Value::Object(source.as_u64())],
            )
            .expect("the shipped bag callback transfers MICS ingredients");
    }
    assert_eq!(
        engine
            .call_object_function(
                engine.find_object_index(mage).expect("live MCLK index"),
                "CheckMagicRequirements",
                vec![Value::C4Id("MICS".into()), Value::Bool(true)],
            )
            .expect("MICS ingredient requirements run"),
        Value::Int(1)
    );

    // Reading a shipped SCRL grants its spell to C4Player::Magic; granting
    // that same entry directly isolates MICS after the scroll has been read
    // (Alchemy.c4s/Script.c:5-16; C4Player.cpp:1052-1058).
    engine
        .grant_player_magic(owner, "MICS")
        .expect("the Alchemy player learns MICS");
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let icestrike_index = engine
        .cursor_object_menu(owner)
        .expect("ContextMagic opens Alchemy's spell menu")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "MICS")
        .expect("the learned icestrike is selectable");
    engine
        .player_in_com(owner, COM_MENU_SELECT, icestrike_index as i32)
        .expect("the menu selects MICS");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts MICS's Magic action");

    let (aimer, iceball) = (0..12)
        .find_map(|_| {
            engine.tick().expect("MICS's Magic action advances");
            let snapshot = engine.snapshot();
            let aimer = snapshot
                .objects
                .iter()
                .find(|object| object.definition_id == "AIMR" && object.status.is_active())
                .map(|object| object.id)?;
            let iceball = snapshot
                .objects
                .iter()
                .find(|object| object.definition_id == "ICEB" && object.status.is_active())
                .map(|object| object.id)?;
            Some((aimer, iceball))
        })
        .expect("MICS creates both its shipped aimer and ICEB");
    assert_eq!(engine.crew_cursor(owner), Some(aimer));

    engine
        .player_in_com(owner, COM_UP, 0)
        .expect("Up changes the shipped AIMR angle");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw releases the aimed ICEB");
    assert_eq!(
        engine.crew_cursor(owner),
        Some(iceball),
        "MICS::ActivateAngle hands direct control to the launched ICEB"
    );
    let launched_angle = engine
        .object_snapshot(iceball)
        .and_then(|iceball| {
            iceball
                .effects
                .iter()
                .find(|effect| effect.name == "IceStrikeFlight")
                .map(|effect| effect.var(2))
        })
        .expect("ICEB keeps its flight effect angle");
    assert_eq!(launched_angle, EffectVarValue::Int(70));
    engine
        .player_in_com(owner, COM_THROW + COM_RELEASE_OFFSET, 0)
        .expect("release the aim-accept key on the ICEB cursor");

    // C4Player::InCom forwards Right and RightReleased to the ICEB cursor;
    // its effect applies the steering speed on the following timer tick
    // (C4Player.cpp:1490-1554; C4Object.cpp:3307-3325;
    // Iceball.c4d/Script.c:47-74,94-101,166-218).
    engine
        .player_in_com(owner, COM_RIGHT, 0)
        .expect("Right steers the launched ICEB");
    engine.tick().expect("ICEB applies its steering speed");
    assert_eq!(
        engine.crew_cursor(owner),
        Some(iceball),
        "an active non-crew cursor survives ICEB's ordinary effect update"
    );
    let steered_angle = engine
        .object_snapshot(iceball)
        .and_then(|iceball| {
            iceball
                .effects
                .iter()
                .find(|effect| effect.name == "IceStrikeFlight")
                .map(|effect| effect.var(2))
        })
        .expect("steered ICEB keeps its flight effect");
    assert_ne!(steered_angle, launched_angle);
    engine
        .player_in_com(owner, COM_RIGHT + COM_RELEASE_OFFSET, 0)
        .expect("Right release stops ICEB steering");

    let impact_position = engine
        .object_snapshot(iceball)
        .expect("ICEB remains live before manual impact")
        .position;
    let target = engine
        .spawn_object(
            SpawnConfig::new("CLNK")
                .with_owner(OWNER_NONE)
                .with_position(Vector2::new(impact_position.x + 5, impact_position.y))
                .with_action(ActionState::new("Walk")),
        )
        .expect("a living frostwave target spawns");
    engine
        .apply_object_update(
            target,
            ObjectUpdate::new()
                .with_position(Vector2::new(impact_position.x + 5, impact_position.y))
                .with_velocity(Vector2::ZERO),
        )
        .expect("place the frostwave target on the first radius");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw triggers ICEB's shipped impact");
    assert!(
        engine
            .object_snapshot(iceball)
            .is_none_or(|iceball| !iceball.status.is_active()),
        "ICEB removes itself on impact"
    );
    assert!(
        engine
            .global_effects()
            .iter()
            .any(|effect| effect.name == "FrostwaveNSpell"),
        "ICEB impact installs the shipped global frostwave"
    );
    engine.tick().expect("the first frostwave radius executes");
    assert!(
        engine
            .object_snapshot(target)
            .expect("frostwave target remains live")
            .effects
            .iter()
            .any(|effect| effect.name == "Freeze"),
        "the ICEB frostwave freezes a living target in its first radius"
    );
}

#[test]
fn alchemy_earthquake_cast_applies_the_shipped_view_shake() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy earthquake parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");

    // Alchemy's loose starter bag contains exactly MQKE's IROC=3 recipe.
    // Transfer it through the shipped attached-bag callback, then choose
    // Earthquake from MCLK's real ContextMagic menu and let the Magic action
    // reach phase five (Alchemy.c4s/Script.c:21-37; Magic.c:65-92,132-162;
    // Earthquake.c4d/DefCore.txt:9; MagiClonk.c4d/Script.c:219-261,430-445).
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.components.get("IROC").copied() == Some(3)
        })
        .map(|object| object.id)
        .expect("Alchemy creates its seeded rock bag");
    let attached_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_"
                && object.action.name == "Belongs"
                && object.action.target == Some(mage)
        })
        .map(|object| object.id)
        .expect("MCLK keeps its attached alchemy bag");
    engine
        .call_object_function(
            engine
                .find_object_index(attached_bag)
                .expect("attached bag index"),
            "Transfer",
            vec![Value::Object(seeded_bag.as_u64())],
        )
        .expect("the shipped bag callback transfers MQKE's rocks");
    engine
        .apply_object_update(
            mage,
            ObjectUpdate::new().with_direction(Direction::Right),
        )
        .expect("face the contained mage right without disturbing its cast-ready action");
    let cast_origin = engine
        .object_snapshot(mage)
        .expect("MCLK remains live before MQKE")
        .position;
    let landscape_before = engine
        .landscape()
        .cloned()
        .expect("Alchemy keeps its generated landscape");

    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let earthquake_index = engine
        .cursor_object_menu(owner)
        .expect("ContextMagic opens Alchemy's spell menu")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "MQKE")
        .expect("Alchemy's Scenario.txt magic list contains MQKE");
    engine
        .player_in_com(owner, COM_MENU_SELECT, earthquake_index as i32)
        .expect("the menu selects MQKE");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw starts MQKE's Magic action");

    let quake = (0..12)
        .find_map(|_| {
            engine.tick().expect("MQKE's Magic action advances");
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| object.definition_id == "FXQ1" && object.status.is_active())
                .map(|object| object.id)
        })
        .unwrap_or_else(|| {
            let snapshot = engine.snapshot();
            panic!(
                "MQKE launches the shipped FXQ1 earthquake; mage={:?}; magic objects={:?}; bag={:?}; player={:?}",
                snapshot.object(mage),
                snapshot
                    .objects
                    .iter()
                    .filter(|object| {
                        matches!(object.definition_id.as_str(), "MQKE" | "FXQ1")
                    })
                    .collect::<Vec<_>>(),
                snapshot.object(attached_bag),
                snapshot.players.iter().find(|player| player.id == owner),
            )
        });
    let quake_snapshot = engine
        .object_snapshot(quake)
        .expect("FXQ1 remains live immediately after activation");
    assert_eq!(quake_snapshot.action.name, "Quake");
    assert!(
        (100..=120).contains(&(quake_snapshot.position.x - cast_origin.x)),
        "right-facing MQKE uses RandomX(100,120) for its ground search: origin={cast_origin:?}, quake={:?}",
        quake_snapshot.position
    );
    let level = match quake_snapshot.local_vars.get("iLevel") {
        Some(Value::Int(level)) => *level,
        other => panic!("FXQ1 Activate stores its randomized iLevel: {other:?}"),
    };
    let lifetime = match quake_snapshot.local_vars.get("iLifeTime") {
        Some(Value::Int(lifetime)) => *lifetime,
        other => panic!("FXQ1 Activate stores iLifeTime: {other:?}"),
    };
    assert!((100..=200).contains(&level));
    assert_eq!(lifetime, level / 2);
    let quake_effect = quake_snapshot
        .effects
        .iter()
        .find(|effect| effect.name == "QuakeEffect")
        .expect("FXQ1 installs its interval-one view effect");
    assert_eq!(quake_effect.priority, 200);
    assert_eq!(quake_effect.interval, 1);
    assert_eq!(quake_effect.var(0), EffectVarValue::Int(level));
    let a = (4 * 10 * level) / (10 * lifetime);
    assert_eq!(quake_effect.var(1), EffectVarValue::Int(a));
    assert_eq!(quake_effect.var(2), EffectVarValue::Int((100 * a) / lifetime));
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC").copied()),
        Some(0),
        "a successful MQKE cast consumes its three-rock recipe"
    );
    let changed_landscape_pixels = {
        let before_grid = landscape_before
            .pixel_grid()
            .expect("pre-cast Alchemy raster");
        let after_grid = engine
            .landscape()
            .and_then(|landscape| landscape.pixel_grid())
            .expect("post-cast Alchemy raster");
        let center = quake_snapshot.position;
        (center.y - 50..=center.y + 50)
            .flat_map(|y| (center.x - 50..=center.x + 50).map(move |x| (x, y)))
            .filter(|&(x, y)| before_grid.byte_at(x, y) != after_grid.byte_at(x, y))
            .count()
    };
    assert!(
        changed_landscape_pixels > 0,
        "MQKE's three immediate randomized ShakeFree circles alter DigFree pixels around FXQ1"
    );

    // FXQ1 installs QuakeEffect at interval one. Its first timer computes a
    // non-zero Sin/Cos camera displacement and calls SetViewOffset for every
    // player (Earthquake effect Script.c:31-59). C++ writes that displacement
    // to the player's live viewport immediately (C4Script.cpp:5676-5687).
    let view_offset = (0..8).find_map(|_| {
        engine.tick().expect("FXQ1's quake effect advances");
        engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == owner)
            .map(|player| player.view_offset)
            .filter(|offset| *offset != Vector2::ZERO)
    });
    assert!(
        view_offset.is_some(),
        "FXQ1 must write a non-zero C++ SetViewOffset while quake {quake:?} is active"
    );

    // Quake's action EndCall runs every three frames. Once ActTime exceeds
    // iLifeTime, the next successful Random(3) gate removes FXQ1
    // (Earthquake effect Script.c:7-19,31-45; ActMap.txt:3-10).
    let removed = (0..lifetime as usize + 64).any(|_| {
        engine.tick().expect("FXQ1 lifecycle advances");
        engine
            .object_snapshot(quake)
            .is_none_or(|quake| !quake.status.is_active())
    });
    assert!(removed, "FXQ1 removes itself after its shipped lifetime");
}

#[test]
fn alchemy_small_force_field_timer_accepts_its_shipped_sound_flags() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let protege = engine
        .spawn_object(SpawnConfig::new("ROCK").with_position(Vector2::new(500, 200)))
        .expect("force-field protege spawns in open sky");
    let rock = engine
        .spawn_object(SpawnConfig::new("ROCK").with_position(Vector2::new(545, 200)))
        .expect("test rock spawns beside the protege");
    let mut field_action = ActionState::new("Field");
    field_action.target = Some(protege);
    let field = engine
        .spawn_object(
            SpawnConfig::new("FRCS")
                .with_owner(OWNER_NONE)
                .with_position(Vector2::new(500, 200))
                .with_action(field_action),
        )
        .expect("shipped small force field spawns");

    // FRCS::Timer flings the nearby ROCK and calls
    // Sound(..., false, obj, 50, 0, false, true, 300). Its loop slot is a
    // C4ValueInt, and C++ accepts Bool->Int unchanged (ForceFieldSmall.c4d/
    // Script.c:112; C4Script.cpp:2297; C4Value.cpp:509-520).
    engine
        .call_object_function(
            engine.find_object_index(field).expect("field index"),
            "Timer",
            Vec::new(),
        )
        .expect("the shipped FRCS timer accepts its boolean loop flag");
    let snapshot = engine.tick().expect("audio events drain on the next frame");
    assert!(snapshot.audio.contains(&AudioCommand::PlaySound {
        name: "MgWind*".into(),
        target: Some(rock),
        volume: 50,
        looped: false,
        multiple: true,
        custom_falloff: Some(300),
    }));
}

#[test]
fn alchemy_firelump_collects_its_same_call_fireball_into_the_mage() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy firelump parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK cursor");
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("Alchemy mage remains live")
            .definition_id,
        "MCLK"
    );

    // The shipped MFBL Activate creates FRBL and immediately calls
    // Collect(pFireball,pClonk). C++ makes that same-call object live,
    // routes it through C4Object::Collect, and leaves it in the mage's
    // inventory when the Collection gate accepts it
    // (Firelump.c4d/Script.c:20-31; C4Script.cpp:391-415;
    // C4Object.cpp:5693-5714).
    let mage_position = engine
        .object_snapshot(mage)
        .expect("mage snapshot")
        .position;
    let spell = engine
        .spawn_object(
            SpawnConfig::new("MFBL")
                .with_owner(owner)
                .with_position(mage_position),
        )
        .expect("the shipped MFBL spell object spawns");
    let spell_index = engine.find_object_index(spell).expect("MFBL index");
    assert_eq!(
        engine
            .call_object_function(
                spell_index,
                "Activate",
                vec![
                    Value::Object(mage.as_u64()),
                    Value::Object(mage.as_u64()),
                ],
            )
            .expect("the shipped MFBL Activate callback runs"),
        Value::Int(1)
    );

    let fireballs = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "FRBL" && object.status.is_active())
        .collect::<Vec<_>>();
    assert_eq!(fireballs.len(), 1, "MFBL creates exactly one live FRBL");
    assert_eq!(
        fireballs[0].container,
        Some(mage),
        "Collect places the same-call FRBL in MCLK instead of flinging it"
    );
}

#[test]
fn dragon_rock_mage_choice_redefines_the_real_knight_and_transfers_its_flag() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    let owner = join_local_player(&mut engine, "Dragon Rock character parity");
    let knight = engine
        .crew_cursor(owner)
        .expect("Dragon Rock joins the Scenario.txt KNIG");

    // Choose normal difficulty through the real KNIG object menu. The shipped
    // InitializePlayer2 then creates FLAG in that KNIG and opens the shipped
    // KNIG/MAGE selection menu (Drachenfels.c4s/Script.c:86-103,112-128).
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("choose normal difficulty");
    let flag = engine
        .object_snapshot(knight)
        .and_then(|knight| {
            knight.contents.into_iter().find(|item| {
                engine
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "FLAG")
            })
        })
        .expect("normal difficulty gives the real KNIG a FLAG");
    let (_, choice) = engine
        .cursor_object_menu(owner)
        .expect("normal difficulty opens the real character menu");
    assert_eq!(
        choice
            .items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["KNIG", "MAGE"]
    );

    engine
        .player_in_com(owner, COM_RIGHT, 0)
        .expect("select MAGE");
    assert_eq!(
        engine
            .cursor_object_menu(owner)
            .expect("character menu remains open")
            .1
            .selection,
        1,
        "the physical Right control selects MAGE"
    );
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("execute Redefine3(MAGE)");

    // Redefine3 creates MAGE, immediately calls pNew->GrabContents(this()),
    // copies the live state, installs it as crew/cursor, then removes KNIG
    // (Drachenfels.c4s/Script.c:150-178). FnGrabContents is an engine-global
    // function found after MAGE's own script and transfers a copied contents
    // list through ordinary Enter calls (C4Aul.cpp:130-148;
    // C4Script.cpp:320-327; C4Object.cpp:6162-6171).
    let mage = engine
        .crew_cursor(owner)
        .expect("Redefine3 leaves a live crew cursor");
    assert_eq!(
        engine
            .object_snapshot(mage)
            .expect("replacement crew remains live")
            .definition_id,
        "MAGE"
    );
    assert!(
        !engine
            .object_snapshot(knight)
            .expect("the removal stays observable until cleanup")
            .status
            .is_active(),
        "Redefine3 marks the old KNIG deleted immediately"
    );
    assert_eq!(
        engine
            .object_snapshot(flag)
            .expect("FLAG survives the character replacement")
            .container,
        Some(mage),
        "MAGE receives KNIG's contents through the real GrabContents call"
    );
}

#[test]
fn dragon_rock_walk_up_enters_the_shipped_tent() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    let owner = join_local_player(&mut engine, "Dragon Rock tent-entry parity");

    // Choose normal difficulty and the initially selected KNIG. Both choices
    // use the real shipped menus before ordinary crew control resumes
    // (Drachenfels.c4s/Script.c:86-128,150-178).
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("choose normal difficulty");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("choose KNIG");
    let knight = engine
        .crew_cursor(owner)
        .expect("Dragon Rock character choice leaves a cursor");
    assert_eq!(
        engine
            .object_snapshot(knight)
            .expect("chosen crew remains live")
            .definition_id,
        "KNIG"
    );

    let tent = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "TENT"
                && object.status.is_active()
                && object.position.x == 222
                && object.position.y == 1240
        })
        .cloned()
        .expect("Dragon Rock ships its first camp tent");
    assert_ne!(
        tent.ocf & lc_engine::ocf::ENTRANCE,
        0,
        "the shipped TENT remains targetable through OCF_Entrance"
    );
    // TENT DefCore.txt:17 is Entrance=-10,4,19,20.
    let entrance_center = Vector2::new(
        tent.position.x - 10 + 19 / 2,
        tent.position.y + 4 + 20 / 2,
    );
    engine
        .apply_object_update(
            knight,
            ObjectUpdate::new()
                .with_position(entrance_center)
                .with_action("Walk")
                .clear_container(),
        )
        .expect("place KNIG at the real TENT entrance");

    // C++ WALK+Up first probes AtObject with OCF_Entrance and queues Enter;
    // C4Command::Enter then checks Target->At with the same entrance OCF and
    // calls C4Object::Enter when EntranceStatus is open
    // (C4ObjectCom.cpp:335-350; C4Command.cpp:545-615).
    engine
        .player_in_com(owner, COM_UP, 0)
        .expect("Up dispatches through the real KNIG control path");
    for _ in 0..3 {
        engine.tick().expect("queued Enter command advances");
    }
    assert_eq!(
        engine
            .object_snapshot(knight)
            .expect("KNIG remains live")
            .container,
        Some(tent.id),
        "pressing Up at an open TENT entrance enters it like C++"
    );
}

#[test]
fn dragon_rock_initialize_player_grants_both_plan_knowledge_sets() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    let owner = join_local_player(&mut engine, "Dragon Rock knowledge parity");

    // Dragon Rock calls WPPL->SetKnowledge(iPlr) and
    // CPPL->SetKnowledge(iPlr) before continuing player initialization
    // (Drachenfels.c4s/Script.c:63-103). Both shipped plan scripts use the
    // two-argument SetPlrKnowledge(player, id) form throughout
    // (Weapons/Plans/Script.c:10-65; Castle/Plans/Script.c:22-70).
    // C4Aul pads the omitted third slot with nil, which converts to false,
    // so FnSetPlrKnowledge validates the definition and grants count one
    // (C4AulParse.cpp:2339-2345; C4Value.h:161,325-330;
    // C4Script.cpp:2636-2650; C4IDList.cpp:85-103).
    // The persisted result is the union with Scenario.txt's positive
    // Player1 Knowledge entries. C4Player first ConsolidateValids that list
    // (C4Player.cpp:697-706; C4IDList.cpp:175-184), and each plan grant also
    // rejects an unloaded definition (C4Script.cpp:2646-2649). Thus PNON
    // from Scenario.txt and CODH from WPPL are deliberately absent.
    let expected = [
        "ADM1", "ADM3", "ANVL", "ARCH", "ARMR", "ARWP", "AXE1", "BALN", "BANP",
        "BARL", "BAS7", "BED1", "BLMP", "BOW1", "BRDG", "BRED", "BWRC", "CANN",
        "CATA", "CHEM", "CLD1", "CNDL", "CNKT", "COKI", "CPAL", "CPEL", "CPH1",
        "CPHC", "CPKT", "CPOF", "CPR1", "CPR2", "CPT1", "CPT2", "CPT3", "CPT4",
        "CPTL", "CPTR", "CPW1", "CPW2", "CPWK", "CPWL", "CPWR", "DCO3", "DCO4",
        "DOGH", "DPOT", "DRCK", "EFLN", "ELEV", "FARP", "FBMP", "FDRS", "FLNT",
        "FNDR", "FRGE", "GUNP", "HUT1", "HUT2", "HUT3", "KSDL", "LANC", "LNKT",
        "LORY", "OVEN", "PAL2", "PALS", "PFIR", "PHEA", "POWR", "PSTO", "PUMP",
        "RSRC", "SAWM", "SFLN", "SHIE", "SHRC", "SLBT", "SPER", "SPRC", "STFN",
        "SWOR", "SWRC", "TABL", "TENP", "TFLN", "THRN", "TWR2", "WDBR", "WGTW",
        "WMIL", "WODC", "WRKS", "WTWR", "WZKP", "XARP", "XBOW",
    ];
    let player = engine.player(owner).expect("joined player remains live");
    let mut actual = player
        .knowledge()
        .map(|definition| definition.as_str())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    assert_eq!(actual, expected, "both shipped plan sets persist exactly");

    // The difficulty menu is created after both definition calls. Its
    // presence proves InitializePlayer ran past every omitted remove flag
    // instead of aborting at the original argument-count warning.
    let (_, menu) = engine
        .cursor_object_menu(owner)
        .expect("InitializePlayer continues into the difficulty menu");
    assert_eq!(
        menu.items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["WIPF", "MONS"]
    );
}

#[test]
fn dragon_rock_real_schedule_enables_and_forces_player_fog_of_war() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    let owner = join_local_player(&mut engine, "Dragon Rock fog parity");
    let player = engine.player(owner).expect("joined player remains live");
    assert!(!player.fog_of_war());
    assert!(!player.force_fog_of_war());

    // The shipped InitializePlayer schedules SetFoW through the installed
    // Helpers.c IntSchedule effect (Drachenfels.c4s/Script.c:56-71;
    // planet/System.c4g/Helpers.c:110-132). A failed eval aborts before the
    // callback's -1 return, so the one-shot effect staying alive would expose
    // the original unknown-function warning.
    let schedules = engine
        .global_effects()
        .iter()
        .filter(|effect| effect.name == "IntSchedule")
        .collect::<Vec<_>>();
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules[0].interval, 1);
    assert_eq!(
        schedules[0].var(0),
        EffectVarValue::String(format!("SetFoW(true, {owner})"))
    );
    assert_eq!(schedules[0].var(1), EffectVarValue::Int(1));

    engine
        .tick()
        .expect("the real IntSchedule callback evaluates SetFoW");

    // FnSetFoW accepts the live player and calls C4Player::SetFoW
    // (C4Script.cpp:3671-3678), which enables both the active FoW flag and
    // its initialized state and forces the script choice
    // (C4Player.cpp:815-824). The Rust save state exposes the two persistent
    // fields FogOfWar and ForceFogOfWar (C4Player.cpp:1580-1581).
    assert!(
        engine
            .global_effects()
            .iter()
            .all(|effect| effect.name != "IntSchedule"),
        "successful eval reaches Helpers.c's one-shot kill return"
    );
    let player = engine.player(owner).expect("joined player remains live");
    assert!(player.fog_of_war());
    assert!(player.force_fog_of_war());
    let persisted = player.to_state();
    assert!(persisted.fog_of_war);
    assert!(persisted.force_fog_of_war);
}

#[test]
fn dragon_rock_objects_keep_their_multidirectional_action_rows() {
    let engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);

    // C4Action::CompileFunc reads Dir as an unrestricted int32
    // (C4Action.cpp:45-54). Loading resolves the action name without replacing
    // that field (C4Object.cpp:2867-2876), then UpdateFlipDir derives DrawDir
    // from it (C4GameObjects.cpp:665-674; C4Object.cpp:404-430).
    for (number, definition, action, direction) in [
        (294, "BANR", "FlyBack", 13),
        (293, "BANR", "FlyBack", 13),
        (292, "BANR", "Fly", 13),
        (290, "BANR", "Fly", 13),
        (1159, "FLAG", "FlyBase", 4),
        (4447, "MUSH", "Exist", 3),
        (548, "BANR", "Fly", 7),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Dragon Rock object #{number} loads"));
        assert_eq!(object.definition_id, definition, "object #{number}");
        assert_eq!(object.action.name, action, "object #{number}");
        assert_eq!(
            object.direction.to_script_value(),
            direction,
            "object #{number} must retain its Objects.txt Dir"
        );
    }

    // These are valid graphic rows, not malformed two-way facing values.
    // BANR's FlipDir=7 maps raw 13 to row 0 mirrored and raw 7 to row 6
    // mirrored; FLAG and MUSH draw their raw rows directly.
    for (definition, action, directions, flip_dir) in [
        ("BANR", "Fly", 14, Some(7)),
        ("BANR", "FlyBack", 14, Some(7)),
        ("FLAG", "FlyBase", 9, None),
        ("MUSH", "Exist", 4, None),
    ] {
        let graphics = engine
            .definition_action_graphics(definition)
            .unwrap_or_else(|| panic!("{definition} action graphics load"));
        let graphics = graphics
            .get(action)
            .unwrap_or_else(|| panic!("{definition}::{action} action loads"));
        assert_eq!(graphics.directions, directions);
        assert_eq!(graphics.flip_dir, flip_dir);
    }
}

#[test]
fn dragon_rock_objects_restore_serialized_c4id_named_locals() {
    let engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);

    // GetC4VID assigns uppercase `I` to C4V_C4ID (C4Value.cpp:368-410).
    // C4Value::CompileFunc persists the ID's signed 32-bit payload verbatim
    // (C4Value.cpp:717-766), and C4ID converts that payload to its four
    // little-endian text bytes (C4Id.cpp:26-52). These are IDs—not integer or
    // object-reference locals—so definition lookup must not gate restoration.
    for (number, definition) in [
        (1758, "MAGE"),
        (1781, "SCRL"),
        (3714, "SCRL"),
        (5064, "SCRL"),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Dragon Rock object #{number} loads"));
        assert_eq!(object.definition_id, definition);
        assert!(object.status.is_active(), "object #{number} is live");
    }

    for (number, local, id) in [
        (1758, "idLastSpell", "EH69"),
        (1781, "idSpell", "MFBL"),
        (3714, "idSpell", "ELX2"),
        (5064, "idSpell", "MFRB"),
        (4410, "idLastSpell", "_MWP"),
        (3886, "idShield", "SHIE"),
        (3883, "idShield", "SHIE"),
        (3818, "idShield", "SHIE"),
        (2541, "idShield", "SHIE"),
        (2555, "idShield", "SHIE"),
        (1128, "idShield", "SHIE"),
        (1128, "ai_idFirstEncounterCB", "BAND"),
        (1779, "idSpell", "XCRS"),
        (1780, "idSpell", "XCRS"),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Dragon Rock object #{number} loads"));
        assert_eq!(
            object.local_vars.get(local),
            Some(&Value::C4Id(id.to_string())),
            "object #{number} local {local}"
        );
    }

    // C4Value arrays recurse through the same compiler and retain their
    // declared order/size (C4Value.cpp:801-805). Cover every Dragon Rock
    // ai_aSpells array that generated the original per-element warnings.
    for (number, ids) in [
        (
            3893,
            &[
                "GGHG", "GZ9Z", "ABLA", "MBOT", "MBLS", "MFRB", "MFBL", "FRFS",
                "MBRG", "EH69", "CMFG",
            ][..],
        ),
        (
            4410,
            &["GZ9Z", "CMFG", "MFFW", "MBRG", "EH69", "EXTG", "ELX2"][..],
        ),
        (
            2550,
            &[
                "CMFG", "MFFW", "ABLA", "MBRG", "EXTG", "MGHL", "MLGT", "ETFL",
                "MFRB", "MDBT", "MFBL", "RUND", "MBLS", "CPAN", "CFAL", "MGBW",
                "MICS", "ELX1", "GZ9Z",
            ][..],
        ),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Dragon Rock spellcaster #{number} loads"));
        let expected = Value::Array(
            ids.iter()
                .map(|id| Value::C4Id((*id).to_string()))
                .collect(),
        );
        assert_eq!(
            object.local_vars.get("ai_aSpells"),
            Some(&expected),
            "object #{number} spell order"
        );
    }
}

#[test]
fn dragon_rock_scroll_transfer_zone_callbacks_persist_cpp_names() {
    let engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);

    // C4Game::Synchronize reaches UpdateTransferZone through
    // TransferZones::Synchronize and Game.Objects.UpdateTransferZones
    // (C4Game.cpp:3727-3729; C4TransferZone.cpp:110-114;
    // C4ObjectList.cpp:734-739). Game.Objects excludes Status=2 objects
    // (C4GameObjects.cpp:54-58), so only Dragon Rock's three active SCRL
    // objects execute the shipped UpdateName call (Scroll.c4d/Script.c:
    // 141-153); the two inactive scrolls retain their serialized names.
    //
    // SetName resolves to the engine-global function after the definition
    // scope (C4Aul.cpp:130-148). With no explicit target it writes the
    // calling object, and GetName then observes CustomName before the
    // definition fallback (C4Script.cpp:993-1005,1008-1060;
    // C4Object.cpp:2103-2115). Because SetName is UpdateName's final call,
    // these changed names prove every relevant shipped callback completed.
    for (number, expected_name) in [
        (1781, "Scroll: Fiery lump"),
        (3714, "Scroll: Recovery"),
        (5064, "Scroll: Fireball"),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("active Dragon Rock scroll #{number} loads"));
        assert!(object.status.is_active(), "scroll #{number} is active");
        assert_eq!(
            object.custom_name.as_deref(),
            Some(expected_name),
            "scroll #{number} persists UpdateName's SetName result"
        );
    }

    for (number, saved_name) in [
        (1779, "Schriftrolle: Reinkarnation"),
        (1780, "Schriftrolle: Reinkarnation"),
    ] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("inactive Dragon Rock scroll #{number} loads"));
        assert!(!object.status.is_active(), "scroll #{number} is inactive");
        assert_eq!(
            object.custom_name.as_deref(),
            Some(saved_name),
            "inactive scroll #{number} is outside C++ Game.Objects broadcast"
        );
    }
}

#[test]
fn dragon_rock_object_lookup_carries_script1_state_into_script3() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    join_local_player(&mut engine, "Dragon Rock intro object parity");

    // InitializePlayer starts the ordinary C4GameScriptHost counter. Its
    // every-tenth-frame Execute post-increments Counter before calling
    // Script%d (C4ScriptHost.cpp:222-230), so Script1 runs on frame 20.
    for _ in 0..20 {
        engine.tick().expect("Dragon Rock reaches shipped Script1");
    }

    // GetEndboss calls Object(EVIL_MAGE_OBJ), where EVIL_MAGE_OBJ is the
    // Objects.txt Number 1758 (Drachenfels.c4s/Script.c:10,194-198,243-279).
    // FnObject resolves that exact number through SafeObjectPointer
    // (C4Script.cpp:3327-3330), whose Game.Objects override searches both
    // active and inactive lists and whose final guard rejects only Status=0
    // (C4GameObjects.cpp:270-276; C4ObjectList.cpp:544-557).
    let globals = &engine.snapshot().script_globals.named;
    for (name, number) in [
        ("g_pEndboss", 1758),
        ("g_pDragon", 202),
        ("g_pKing", 5129),
        ("g_pPrincess", 1777),
    ] {
        assert_eq!(
            globals.get(name),
            Some(&Value::Object(number)),
            "Script1 persists {name} for later callbacks"
        );
    }
    for (number, definition) in [(1758, "MAGE"), (202, "DRGN")] {
        let object = engine
            .object_snapshot(ObjectId::new(number))
            .unwrap_or_else(|| panic!("Script1 target #{number} remains live"));
        assert_eq!(object.definition_id, definition);
        assert_eq!(object.position.x, 1000, "Script1 moved #{number}");
        assert_eq!(object.position.y, 800, "Script1 moved #{number}");
    }

    // Script3 runs on frame 40 and dereferences the Script1 result through
    // g_pDragon->IntroControl(400, 1050) (Drachenfels.c4s/Script.c:281-284).
    // The shipped DRGN append writes all three globals before returning true
    // (Drachenfels.c4s/System.c4g/Dragon.c:17,26-32). If Script1 aborted at
    // Object(), this is the original "target is zero" failure instead.
    for _ in 0..20 {
        engine.tick().expect("Dragon Rock reaches shipped Script3");
    }
    let globals = &engine.snapshot().script_globals.named;
    assert_eq!(globals.get("DRGN_ctrl_tx"), Some(&Value::Int(400)));
    assert_eq!(globals.get("DRGN_ctrl_ty"), Some(&Value::Int(1050)));
    assert!(matches!(
        globals.get("DRGN_ctrl_stop"),
        None | Some(Value::Nil) | Some(Value::Bool(false)) | Some(Value::Int(0))
    ));
}

#[test]
fn dragon_rock_script25_casts_cpp_sparks_and_completes_intro_step() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Drachenfels.c4s", 0);
    join_local_player(&mut engine, "Dragon Rock CastObjects parity");

    // Let the shipped counter reach Script15's pause, then resume it through
    // the real dragon-arrival callback (Drachenfels.c4s/Script.c:286-294).
    // Counter 20 is intentionally empty; Script21 runs at frame 180 and
    // Script25 naturally runs at frame 220.
    for _ in 0..160 {
        engine.tick().expect("Dragon Rock reaches Script15 pause");
    }
    engine
        .call_scenario_script_function("OnDragonReachTarget", Vec::new())
        .expect("real dragon arrival resumes the intro counter");
    for _ in 0..59 {
        engine.tick().expect("Dragon Rock approaches Script25");
    }
    assert_eq!(engine.snapshot().frame, 219);

    let princess_before = engine
        .object_snapshot(ObjectId::new(1777))
        .expect("Dragon Rock princess remains live before Script25");
    let old_sparks = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "SPRK")
        .map(|object| object.id)
        .collect::<Vec<_>>();

    let frame = engine.tick().expect("natural Script25 callback succeeds");
    assert_eq!(frame.frame, 220);
    let sparks = frame
        .objects
        .iter()
        .filter(|object| object.definition_id == "SPRK" && !old_sparks.contains(&object.id))
        .collect::<Vec<_>>();

    // Script25 calls the shipped Sparkle(5, fx, fy), which casts
    // 5/3+2 == three SPRK objects (Script.c:307-320;
    // Objects.c4d/Effects.c4d/Spark.c4d/Script.c:25-28). FnCastObjects
    // applies scenario-global coordinates and NO_OWNER/NO_OWNER, while
    // C4Game::CastObjects samples rdir, ydir, xdir, rotation in that exact
    // order for every object (C4Script.cpp:2476-2480;
    // C4Game.cpp:1727-1739). SPRK is not rotateable, so Init clears the
    // sampled rotation/rdir but preserves both FIXED10 velocity components
    // (C4Object.cpp:153-187).
    assert_eq!(sparks.len(), 3, "Sparkle(5) casts exactly three sparks");
    let allowed_velocity = (-5..=5).map(math::fixed10).collect::<Vec<_>>();
    for spark in sparks {
        assert_eq!(spark.owner, OWNER_NONE);
        assert_eq!(spark.controller, OWNER_NONE);
        assert_eq!(spark.position.x, princess_before.position.x);
        assert_eq!(spark.position.y, princess_before.position.y - 3);
        let fixed_velocity = spark.fixed_velocity.unwrap_or_else(|| {
            math::FixedVec2::from_ints(spark.velocity.x, spark.velocity.y)
        });
        assert!(allowed_velocity.contains(&fixed_velocity.x));
        assert!(allowed_velocity.contains(&fixed_velocity.y));
        assert_eq!(spark.rotation, 0);
        assert_eq!(spark.rotation_velocity, None);
        assert_eq!(spark.action.name, "Sparkle");
        assert!(
            (FULL_CON..FULL_CON * 2).contains(&spark.construction),
            "SPRK Completion applies DoCon(Random(100)) after initial FullCon; got {}",
            spark.construction
        );
    }

    // These statements follow Sparkle in Script25. They prove CastObjects
    // returned normally instead of aborting the callback at the old unknown
    // function warning.
    let princess = engine
        .object_snapshot(ObjectId::new(1777))
        .expect("princess survives Script25");
    assert_eq!((princess.position.x, princess.position.y), (2145, 485));
    assert_eq!(princess.action.name, "Walk");
    assert_eq!(princess.direction.to_script_value(), 0);
    let endboss = engine
        .object_snapshot(ObjectId::new(1758))
        .expect("endboss survives Script25");
    assert_eq!(endboss.action.name, "RideMagic");
    assert_eq!(endboss.action.target, Some(ObjectId::new(202)));
}

#[test]
fn alchemy_tunnel_spell_opens_its_first_shipped_landscape_row() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy tunnel parity");
    let mage = engine.crew_cursor(owner).expect("Alchemy MCLK cursor");
    let earth = engine
        .materials()
        .id_of("Earth")
        .expect("Alchemy loads Earth");
    let (target_x, target_y) = {
        let landscape = engine.landscape().expect("Alchemy keeps its landscape");
        let grid = landscape.pixel_grid().expect("Alchemy has a raster landscape");
        (20..grid.height() as i32 - 20)
            .find_map(|y| {
                (20..grid.width() as i32 - 20)
                    .find(|&x| {
                        landscape.material_at(x, y) == Some(earth)
                            && landscape.is_solid_at(x, y)
                    })
                    .map(|x| (x, y))
            })
            .expect("Alchemy contains an interior solid Earth pixel")
    };
    let solid_pixels_before = {
        let landscape = engine.landscape().expect("landscape before tunnel");
        let grid = landscape.pixel_grid().expect("Alchemy raster before tunnel");
        (target_y - 2..=target_y + 2)
            .flat_map(|y| (target_x - 17..=target_x + 17).map(move |x| (x, y)))
            .filter(|&(x, y)| landscape.is_solid_at(x, y))
            .filter_map(|(x, y)| grid.byte_at(x, y).map(|byte| (x, y, byte)))
            .collect::<Vec<_>>()
    };
    engine
        .apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(target_x, target_y - 10))
                .clear_container(),
        )
        .expect("place MCLK ten pixels above the tunnel origin");
    let spell = engine
        .spawn_object(
            SpawnConfig::new("MTNL")
                .with_owner(owner)
                .with_position(Vector2::new(target_x, target_y - 10)),
        )
        .expect("the shipped MTNL spell spawns");
    let spell_index = engine.find_object_index(spell).expect("MTNL index");

    assert_eq!(
        engine
            .call_object_function(
                spell_index,
                "ActivateAngle",
                vec![Value::Object(mage.as_u64()), Value::Int(0)],
            )
            .expect("the shipped aimed activation starts its global effect"),
        Value::Int(1)
    );
    assert_eq!(
        engine
            .landscape()
            .expect("landscape before tunnel timer")
            .material_at(target_x, target_y),
        Some(earth)
    );

    engine.tick().expect("the tunnel effect reaches time zero");
    engine.tick().expect("the first tunnel row timer executes");
    let opened_pixels = {
        let landscape = engine.landscape().expect("landscape after tunnel timer");
        let grid = landscape.pixel_grid().expect("Alchemy raster after tunnel");
        solid_pixels_before
            .iter()
            .filter(|&&(x, y, before)| {
                grid.byte_at(x, y).is_some_and(|after| after != before)
                    && !landscape.is_solid_at(x, y)
            })
            .copied()
            .collect::<Vec<_>>()
    };
    assert!(
        !opened_pixels.is_empty(),
        "C++ FnEffectVar returns a reference whose indexed array element is assignable; \
         MTNL records and frees solid pixels in its first landscape row"
    );
}

#[test]
fn alchemy_firefist_flame_consumes_inflammable_landscape() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Alchemy.c4s", 0);
    let owner = join_local_player(&mut engine, "Alchemy firefist parity");
    let mage = engine.crew_cursor(owner).expect("Alchemy MCLK cursor");
    let fuel = engine.materials().id_of("Oil").expect("Alchemy loads Oil");
    assert_ne!(
        engine
            .materials()
            .get_by_id(fuel)
            .expect("Oil material metadata")
            .inflammable(),
        0,
        "Oil is C++-inflammable"
    );
    let ((fuel_x, fuel_y), (air_x, air_y)) = {
        let landscape = engine.landscape().expect("Alchemy keeps its landscape");
        let grid = landscape.pixel_grid().expect("Alchemy has a raster landscape");
        let air_position = (30..grid.height() as i32 - 30)
            .find_map(|y| {
                (30..grid.width() as i32 - 30)
                    .find(|&x| {
                        landscape.material_at(x, y).is_none()
                            && landscape.material_at(x - 20, y).is_none()
                            && landscape.material_at(x + 20, y).is_none()
                            && landscape.material_at(x, y - 10).is_none()
                            && landscape.material_at(x, y + 10).is_none()
                    })
                    .map(|x| (x, y))
            })
            .expect("Alchemy contains open air for the fire shower");
        (
            (grid.width() as i32 / 2, grid.height() as i32 / 2),
            air_position,
        )
    };

    // The fixed seed has no naturally placed inflammable pixel. Draw a small
    // Oil-Smooth patch through the shipped engine API so the flame path has a
    // controlled C++-valid precondition.
    engine
        .register_definition(
            Definition::from_script(
                "FUEL",
                "Fuel painter",
                r#"#strict
public func Paint(int x, int y)
{
    return DrawMaterialQuad("Oil-Smooth", x-1,y-1, x+1,y-1, x+1,y+1, x-1,y+1, false);
}
"#,
            )
            .expect("fuel painter compiles"),
        )
        .expect("fuel painter registers");
    let painter = engine
        .spawn_object(SpawnConfig::new("FUEL"))
        .expect("fuel painter spawns");
    let painter_index = engine.find_object_index(painter).expect("FUEL index");
    assert_eq!(
        engine
            .call_object_function(
                painter_index,
                "Paint",
                vec![Value::Int(fuel_x), Value::Int(fuel_y)],
            )
            .expect("DrawMaterialQuad paints the controlled fuel patch"),
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .landscape()
            .expect("landscape after fuel setup")
            .material_at(fuel_x, fuel_y),
        Some(fuel),
        "the controlled patch is Oil"
    );

    // FRFS Activate creates the two shipped FSHW fire showers. Force the
    // left one through its ordinary Hit -> Jumpada path in open air so it
    // creates the same FLAM child as live gameplay
    // (Firefist.c4d/Script.c:15-55; Firetail.c4d/Script.c:19-40).
    let spell = engine
        .spawn_object(
            SpawnConfig::new("FRFS")
                .with_owner(owner)
                .with_position(Vector2::new(air_x + 15, air_y)),
        )
        .expect("the shipped FRFS spell spawns");
    let spell_index = engine.find_object_index(spell).expect("FRFS index");
    assert_eq!(
        engine
            .call_object_function(
                spell_index,
                "Activate",
                vec![Value::Object(mage.as_u64()), Value::Object(mage.as_u64())],
            )
            .expect("the shipped Firefist activates"),
        Value::Bool(true)
    );
    let fire_shower = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "FSHW"
                && object.status.is_active()
                && object.action.name == "Left"
        })
        .map(|object| object.id)
        .expect("Firefist creates its left fire shower");
    engine
        .apply_object_update(
            fire_shower,
            ObjectUpdate::new().with_position(Vector2::new(air_x, air_y)),
        )
        .expect("place the fire shower in open air");
    let shower_index = engine
        .find_object_index(fire_shower)
        .expect("FSHW index");
    engine
        .call_object_function(shower_index, "Hit", Vec::new())
        .expect("the shipped Hit callback creates FLAM");
    let flame = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "FLAM" && object.status.is_active())
        .map(|object| object.id)
        .expect("FSHW::Jumpada creates a live FLAM child");
    assert!(
        engine
            .object_snapshot(flame)
            .expect("FLAM snapshot")
            .on_fire,
        "FLAM Completion incinerates the flame before BurnProcess"
    );
    engine
        .apply_object_update(
            flame,
            ObjectUpdate::new().with_position(Vector2::new(fuel_x, fuel_y)),
        )
        .expect("place FLAM on the inflammable material pixel");
    let fuel_before = engine
        .landscape()
        .expect("landscape before flame consumption")
        .material_pixel_count(fuel, None);

    let flame_index = engine.find_object_index(flame).expect("FLAM index");
    assert_eq!(
        engine
            .call_object_function(flame_index, "BurnProcess", Vec::new())
            .expect("the shipped FLAM BurnProcess executes"),
        Value::Int(1)
    );
    let fuel_after = engine
        .landscape()
        .expect("landscape after flame consumption")
        .material_pixel_count(fuel, None);
    assert!(
        fuel_after < fuel_before,
        "FnFlameConsumeMaterial extracts at least one inflammable material pixel like C++"
    );
}
