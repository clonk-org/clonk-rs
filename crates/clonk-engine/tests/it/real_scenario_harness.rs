#![allow(dead_code)]

use crate::object_visibility::{
    shipped_invisibility_recast_carries_remaining_time_into_reset_timer,
    shipped_invisibility_spell_hides_and_restores_its_mage,
};
use crate::support::real_scenario::{
    join_local_player, join_local_player_on_team, load_installed_scenario, load_tutorial,
    prepare_installed_scenario, PreparedInstalledScenario,
};
use crate::support::PreparedScenarioSubcase;
use clonk_engine::{
    math, ActionState, AudioCommand, Definition, Direction, EffectVarValue, Engine, EngineError,
    JoinPlayerConfig, Landscape, ObjectId, ObjectSnapshot, ObjectStatus, ObjectUpdate,
    PlayerStatus, SpawnConfig, TeamInfo, Vector2, COM_DIG, COM_DOWN, COM_MENU_SELECT,
    COM_RELEASE_OFFSET, COM_RIGHT, COM_SPECIAL, COM_THROW, COM_UP, FULL_CON, OWNER_NONE,
};
use clonk_script::Value;

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
fn wipfrace_get_scenario_val_uses_loaded_map_zoom_and_player_id_lists() {
    let mut engine = load_installed_scenario("Races.c4f/Wipfrace.c4s", 0);

    // Wipfrace's shipped Initialize reaches GetScenMapZoom through the
    // installed System.c4g wrapper. Scenario.txt has MapZoom=9, so the snake
    // request is (132*9,84*9); SNKE's initial DoCon shifts its integer center
    // five pixels upward while preserving the requested x coordinate.
    let snakes = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "SNKE")
        .collect::<Vec<_>>();
    assert!(
        (5..=8).contains(&snakes.len()),
        "Wipfrace creates 5..=8 snakes from its synced random draw"
    );
    assert!(
        snakes
            .iter()
            .all(|object| object.position == Vector2::new(1188, 751)),
        "GetScenMapZoom must place every snake at the C++ coordinate: {:?}",
        snakes
            .iter()
            .map(|object| object.position)
            .collect::<Vec<_>>()
    );

    let mut trap_x = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "RCTP")
        .map(|object| object.position.x)
        .collect::<Vec<_>>();
    trap_x.sort_unstable();
    assert_eq!(trap_x, [963, 1053, 1098, 1170, 1197, 1224]);

    // Probe the same applied Game.C4S snapshot directly. C4ValueGetCompiler
    // flattens HomeBaseMaterial as ID,count pairs in file order.
    engine
        .register_definition(
            Definition::from_script(
                "SCVP",
                "Scenario value probe",
                r#"#strict
public func Read(string entry, string section, int index)
{
    return GetScenarioVal(entry, section, index);
}
"#,
            )
            .expect("scenario-value probe compiles"),
        )
        .expect("scenario-value probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("SCVP"))
        .expect("scenario-value probe spawns");
    let probe_index = engine.find_object_index(probe).expect("probe index");
    let mut read = |entry: &str, section: &str, index: i32| {
        engine
            .call_object_function(
                probe_index,
                "Read",
                vec![
                    Value::String(entry.to_string().into()),
                    Value::String(section.to_string().into()),
                    Value::Int(index),
                ],
            )
            .expect("GetScenarioVal probe succeeds")
    };
    assert_eq!(read("MapZoom", "Landscape", 0), Value::Int(9));
    assert_eq!(
        read("HomeBaseMaterial", "Player1", 0),
        Value::C4Id("CNKT".to_string())
    );
    assert_eq!(read("HomeBaseMaterial", "Player1", 1), Value::Int(5));
    assert_eq!(
        read("HomeBaseMaterial", "Player1", 2),
        Value::C4Id("LNKT".to_string())
    );
    assert_eq!(read("HomeBaseMaterial", "Player1", 3), Value::Int(8));
}

#[test]
fn arctic_lightning_spell_launches_three_creatorless_native_bolts() {
    // Far Worlds LGT2 calls native LaunchLightning three times with the
    // caster's global vertex and y-ranges 36/41/46, then removes itself.
    // Each shipped FXL1 is creatorless at (50,50); its synchronous Advance
    // StartCall and Activate body consume six synced draws when the branch
    // gate is nonzero (FarWorlds.../Lightning.c4d/Script.c:3-13;
    // Objects.../Effects.../Lightning.c4d/Script.c:16-57).
    let mut engine = load_installed_scenario("FarWorlds.c4f/Arctic.c4s", 0);
    let owner = join_local_player(&mut engine, "Arctic lightning parity");
    let caster = engine
        .crew_cursor(owner)
        .expect("Arctic joins with an Inuit selected");
    let caster_state = engine.object_snapshot(caster).expect("caster exists");
    let caster_vertex = caster_state
        .vertices
        .first()
        .expect("the shipped Inuit has vertex zero");
    let origin = Vector2::new(
        caster_state.position.x + caster_vertex.x,
        caster_state.position.y + caster_vertex.y,
    );
    let advance_x = -20 + 25 * caster_state.direction.to_script_value();
    let old_bolts = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "FXL1")
        .map(|object| object.id)
        .collect::<Vec<_>>();
    let spell = engine
        .spawn_object(
            SpawnConfig::new("LGT2")
                .with_owner(owner)
                .with_layer(caster),
        )
        .expect("the shipped Arctic spell spawns");
    let rng_before = engine.debug_rng_clone().count;
    let spell_index = engine.find_object_index(spell).expect("spell exists");

    assert_eq!(
        engine
            .call_object_function(
                spell_index,
                "Activate",
                vec![Value::Object(caster.as_u64()), Value::Nil, Value::Nil],
            )
            .expect("the shipped LGT2 callback completes"),
        Value::Int(1)
    );
    assert!(
        engine
            .object_snapshot(spell)
            .is_none_or(|spell| !spell.status.is_active()),
        "LGT2 removes itself after launching the three bolts"
    );

    let mut bolts = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "FXL1" && !old_bolts.contains(&object.id))
        .collect::<Vec<_>>();
    bolts.sort_by_key(|object| object.id.as_u64());
    assert_eq!(
        bolts.len(),
        3,
        "the seed-zero Arctic cast must not take FXL1's recursive branch"
    );
    assert_eq!(
        engine.debug_rng_clone().count,
        rng_before + 18,
        "three shipped FXL1 activations consume six synced draws each"
    );
    for (bolt, expected_range) in bolts.iter().zip([36, 41, 46]) {
        assert_eq!(bolt.position, Vector2::new(50, 50));
        assert_eq!(bolt.owner, OWNER_NONE);
        assert_eq!(bolt.controller, OWNER_NONE);
        assert_eq!(bolt.layer, None);
        assert_eq!(bolt.action.name, "Advance");
        assert_eq!(
            bolt.vertices.first().map(|vertex| (vertex.x, vertex.y)),
            Some((origin.x, origin.y))
        );
        assert_eq!(bolt.local_vars.get("iAdvX"), Some(&Value::Int(advance_x)));
        assert_eq!(bolt.local_vars.get("iVarX"), Some(&Value::Int(15)));
        assert_eq!(bolt.local_vars.get("iAdvY"), Some(&Value::Int(-20)));
        assert_eq!(
            bolt.local_vars.get("iVarY"),
            Some(&Value::Int(expected_range))
        );
        assert_eq!(bolt.local_vars.get("fDoGamma"), Some(&Value::Nil));
    }
}

#[test]
fn sky_race_team_generation_branch_uses_get_team_config() {
    let mut engine = load_installed_scenario("Races.c4f/Skyrace.c4s", 0);

    // Skyrace has no Teams.txt but carries RVLR. C4TeamList::Load therefore
    // derives the exact live config [custom, active, hostility, dist,
    // switch, autogen, colors] = [0,1,1,0,0,1,0].
    engine
        .register_definition(
            Definition::from_script(
                "TCFG",
                "Team configuration probe",
                r#"#strict 2
public func Read()
{
    return [GetTeamConfig(1), GetTeamConfig(2), GetTeamConfig(3),
            GetTeamConfig(4), GetTeamConfig(5), GetTeamConfig(6),
            GetTeamConfig(7)];
}
"#,
            )
            .expect("team config probe compiles"),
        )
        .expect("team config probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("TCFG"))
        .expect("team config probe spawns");
    let probe_index = engine.find_object_index(probe).expect("probe exists");
    assert_eq!(
        engine
            .call_object_function(probe_index, "Read", Vec::new())
            .expect("all seven team configuration values are callable"),
        Value::Array(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(0),
        ])
    );

    // Exercise Race.c4d:303-327 itself. Autogenerated teams synthesize the
    // scoreboard caption from the first member and query TeamColors twice.
    engine.set_teams(vec![TeamInfo::new(1, "Team 1", 0x00f4_0000)]);
    let player_name = "Sky Race generated-team member";
    let player = join_local_player_on_team(&mut engine, player_name, 1);
    let race = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| object.definition_id == "RACE")
        .map(|object| object.id)
        .expect("Skyrace creates the shipped RACE goal");
    let race_index = engine.find_object_index(race).expect("RACE remains live");
    assert_eq!(
        engine
            .call_object_function(
                race_index,
                "InitializePlayer",
                vec![
                    Value::Int(player),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Nil,
                    Value::Int(1),
                ],
            )
            .expect("the shipped Race team-generation branch completes"),
        Value::Bool(true)
    );

    let scoreboard = engine.snapshot().hud.scoreboard;
    let team_row = (1..scoreboard.row_count())
        .find(|row| scoreboard.cell(*row, 0).map(|cell| cell.value()) == Some(1))
        .expect("Race creates the team scoreboard row");
    let caption = scoreboard
        .cell(team_row, 0)
        .and_then(|cell| cell.text())
        .expect("the team row has a caption");
    assert!(
        caption.contains(player_name),
        "autogenerated Race team caption should name its member: {caption:?}"
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
            .call_object_function(probe_index, "Trigger", vec![Value::Object(clonk.as_u64())],)
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
            object.id != original && object.definition_id == "CLNK" && object.status.is_active()
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
        .filter(|object| object.definition_id == "LOAM" && object.container == Some(replacement))
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
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
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
        .number();
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
        .tick_without_snapshot()
        .expect("the shipped one-tick RACE timer accepts the finisher");

    let after_finish = engine.snapshot();
    let winner_info_id = after_finish
        .players
        .iter()
        .find(|player| player.id == winner)
        .map(|player| player.player_info_id)
        .expect("winner state remains present");
    let scoreboard = &after_finish.hud.scoreboard;
    let race_column = (0..scoreboard.column_count())
        .find(|column| {
            scoreboard.cell(0, *column).map(|cell| cell.value())
                == Some(i32::from_le_bytes(*b"RACE"))
        })
        .expect("RACE::Initialize creates its progress column");
    let winner_row = (1..scoreboard.row_count())
        .find(|row| scoreboard.cell(*row, 0).map(|cell| cell.value()) == Some(winner_info_id))
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
        engine
            .tick_without_snapshot()
            .expect("advance the normal GOAL controller");
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
        !completed.players.iter().any(|player| player.id == loser),
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

    let monster = engine
        .object_snapshot(mage)
        .expect("joined mage remains live")
        .container
        .expect("Monster Rescue puts MAGE inside its controlled monster");
    assert_eq!(
        engine
            .object_snapshot(monster)
            .expect("controlled monster remains live")
            .definition_id,
        "MONS"
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
    // A world right-click hits the visible MONS, not its contained MAGE. C++
    // adds the menu Clonk's own actions after the clicked target's actions and
    // collapses more than two of them into a MAGE submenu. Entering that row
    // opens a second C4MN_Context on the MAGE; only then can ContextMagic open
    // MBRG (C4MouseControl.cpp:1230-1263; C4ObjectMenu.cpp:687-709;
    // MagiClonk.c4d/Script.c:190-199).
    assert!(engine
        .player_context_command(owner, monster)
        .expect("right-click queues the monster context command"));
    engine
        .tick_without_snapshot()
        .expect("the mouse context command opens the monster menu");
    let monster_menu = engine
        .cursor_object_menu(owner)
        .expect("the controlled monster opens its C++ context menu")
        .1
        .clone();
    let mage_submenu_index = monster_menu
        .items
        .iter()
        .position(|item| {
            item.caption == "Mage"
                && item.command
                    == "SetCommand(this,\"Context\",,0,0,this)&&ExecuteCommand()"
        })
        .unwrap_or_else(|| {
            panic!(
                "the monster context contains the contained MAGE submenu; menu={monster_menu:?}; mage={:?}",
                engine.object_snapshot(mage)
            )
        });
    engine
        .player_in_com(owner, COM_MENU_SELECT, mage_submenu_index as i32)
        .expect("select the MAGE submenu");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("enter the MAGE submenu");

    let magic_index = engine
        .cursor_object_menu(owner)
        .expect("the submenu opens MAGE's own context")
        .1
        .items
        .iter()
        .position(|item| item.command.contains("ContextMagic"))
        .expect("MAGE's own context exposes ContextMagic");
    engine
        .player_in_com(owner, COM_MENU_SELECT, magic_index as i32)
        .expect("select ContextMagic");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("ContextMagic opens the spell menu");

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
        engine
            .cursor_object_menu(owner)
            .map(|(_, menu)| menu.clone()),
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
        engine
            .tick_without_snapshot()
            .expect("the real magic action advances");
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
fn alchemy_real_scenario_subcases_batch_1() {
    run_alchemy_batch(&[
        (
            "earthquake_cast_applies_the_shipped_view_shake",
            alchemy_earthquake_cast_applies_the_shipped_view_shake,
        ),
        (
            "small_force_field_timer_accepts_its_shipped_sound_flags",
            alchemy_small_force_field_timer_accepts_its_shipped_sound_flags,
        ),
        (
            "tunnel_spell_opens_its_first_shipped_landscape_row",
            alchemy_tunnel_spell_opens_its_first_shipped_landscape_row,
        ),
        (
            "firelump_collects_its_same_call_fireball_into_the_mage",
            alchemy_firelump_collects_its_same_call_fireball_into_the_mage,
        ),
    ]);
}

#[test]
fn alchemy_real_scenario_subcases_batch_2() {
    run_alchemy_batch(&[
        (
            "learned_group_heal_cast_sustains_magic_and_heals_nearby_crew",
            alchemy_learned_group_heal_cast_sustains_magic_and_heals_nearby_crew,
        ),
        (
            "firefist_flame_consumes_inflammable_landscape",
            alchemy_firefist_flame_consumes_inflammable_landscape,
        ),
        (
            "learned_icestrike_aims_steers_and_impacts_through_player_controls",
            alchemy_learned_icestrike_aims_steers_and_impacts_through_player_controls,
        ),
        (
            "make_artefact_cast_opens_the_real_enchantment_menu",
            alchemy_make_artefact_cast_opens_the_real_enchantment_menu,
        ),
        (
            "shipped_invisibility_spell_hides_and_restores_its_mage",
            shipped_invisibility_spell_hides_and_restores_its_mage,
        ),
    ]);
}

#[test]
fn alchemy_real_scenario_subcases_batch_3() {
    run_alchemy_batch(&[
        (
            "make_artefact_hit_mode_casts_the_selected_spell_after_throw",
            alchemy_make_artefact_hit_mode_casts_the_selected_spell_after_throw,
        ),
        (
            "force_field_wall_puts_its_mask_before_segment_initialize",
            alchemy_force_field_wall_puts_its_mask_before_segment_initialize,
        ),
        (
            "reincarnation_spell_revives_its_mage_during_assign_death",
            alchemy_reincarnation_spell_revives_its_mage_during_assign_death,
        ),
        (
            "combo_mode_opens_and_accepts_the_shipped_element_control",
            alchemy_combo_mode_opens_and_accepts_the_shipped_element_control,
        ),
        (
            "shipped_invisibility_recast_carries_remaining_time_into_reset_timer",
            shipped_invisibility_recast_carries_remaining_time_into_reset_timer,
        ),
    ]);
}

#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "recording-host material order; required macOS CI job"
)]
fn alchemy_real_scenario_subcases_batch_4() {
    run_alchemy_batch(&[
        (
            "possession_uses_the_shipped_selector_control",
            alchemy_possession_uses_the_shipped_selector_control,
        ),
        (
            "mage_uses_context_magic_and_casts_the_shipped_gravity_spells",
            alchemy_mage_uses_context_magic_and_casts_the_shipped_gravity_spells,
        ),
        (
            "warp_to_base_cast_builds_the_real_portal_pair_and_transfers_the_mage",
            alchemy_warp_to_base_cast_builds_the_real_portal_pair_and_transfers_the_mage,
        ),
        (
            "learned_lightning_cast_launches_the_shipped_line_object",
            alchemy_learned_lightning_cast_launches_the_shipped_line_object,
        ),
        (
            "seeded_bag_collects_and_activates_through_player_controls",
            alchemy_seeded_bag_collects_and_activates_through_player_controls,
        ),
    ]);
}

fn run_alchemy_batch(subcases: &[PreparedScenarioSubcase]) {
    run_prepared_scenario_batch("Alchemy", "Fantasy.c4f/Alchemy.c4s", subcases);
}

fn run_prepared_scenario_batch(
    scenario_name: &str,
    relative_path: &str,
    subcases: &[PreparedScenarioSubcase],
) {
    let prepared = prepare_installed_scenario(relative_path, 0);
    let mut failures = Vec::new();

    for &(name, subcase) in subcases {
        eprintln!("running {scenario_name} subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| subcase(&prepared))).is_err() {
            eprintln!("{scenario_name} subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} {scenario_name} subcase(s) failed: {}",
            failures.len(),
            failures.join(", ")
        );
    }
}

fn alchemy_mage_uses_context_magic_and_casts_the_shipped_gravity_spells(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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
            object.definition_id == "ALC_" && object.components.get("IROC").copied() == Some(3)
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
        engine
            .tick_without_snapshot()
            .expect("the shipped Magic action advances");
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
        engine
            .tick_without_snapshot()
            .expect("the shipped Magic action advances");
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
                control.last_com = i32::from(COM_RIGHT);
                control.last_com_delay = 17;
                control.last_com_down_double = 4;
            }
            engine.tick_without_snapshot().expect("the ABLA Magic action advances");
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
        player.view_cursor, None,
        "AIMR::Close resets the temporary C4Player::ViewCursor"
    );
    assert_eq!(
        player.viewports.first().and_then(|viewport| viewport.focus),
        Some(mage),
        "cursor-mode presentation falls back from nil ViewCursor to Cursor"
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

fn alchemy_warp_to_base_cast_builds_the_real_portal_pair_and_transfers_the_mage(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy warp parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");

    // ExecBase runs on Tick10 and claims AHUT for this player once its FLAG
    // has settled. MWP2 deliberately fails before that claim; wait for the
    // same C++ base lifecycle rather than manufacturing a shortcut.
    let home = (0..20)
        .find_map(|_| {
            engine
                .tick_without_snapshot()
                .expect("Alchemy base lifecycle advances");
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
        engine
            .tick_without_snapshot()
            .expect("Alchemy ready-crew Exit advances");
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
        engine
            .tick_without_snapshot()
            .expect("MWP2's Magic action advances");
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
    let original_vertices = engine
        .object_snapshot(mage)
        .expect("the mage remains live before entering the warp")
        .vertices;
    assert!(
        !original_vertices.is_empty(),
        "the real MCLK shape supplies vertices for WarpUSpellData to remove"
    );
    engine
        .apply_object_update(
            mage,
            ObjectUpdate::new()
                // Keep enough distance for WarpUSpellData to remain observable
                // between its Start and Stop callbacks. At Con=100 the C++
                // portal accepts targets inside 50 px and initializes the
                // pull strength to distance*2.5 (Warp.c lines 8-11, 242-256).
                .with_position(Vector2::new(
                    start_portal.position.x.saturating_add(30),
                    start_portal.position.y,
                ))
                .with_velocity(Vector2::ZERO)
                .clear_container(),
        )
        .expect("place the mage inside the source warp aperture");

    let warp_data_observed = (0..30).any(|_| {
        engine
            .tick_without_snapshot()
            .expect("the real WARP pair advances");
        let mage = engine
            .object_snapshot(mage)
            .expect("the mage remains live while the source warp pulls it");
        let active = mage
            .effects
            .iter()
            .any(|effect| effect.name == "WarpUSpellData" && effect.priority != 0);
        active && mage.vertices.is_empty()
    });
    assert!(
        warp_data_observed,
        "the source portal must install WarpUSpellData before transferring the mage: mage={:?}; portal={start_portal:?}",
        engine.object_snapshot(mage),
    );
    let live_warp_effect = engine
        .object_snapshot(mage)
        .and_then(|mage| {
            mage.effects
                .into_iter()
                .find(|effect| effect.name == "WarpUSpellData" && effect.priority != 0)
        })
        .expect("WarpUSpellData remains live at the observed zero-vertex point");
    assert_eq!(
        live_warp_effect.vars.len(),
        16,
        "WarpUSpellData stores power, count, and seven X/Y pairs before removing the shape: {live_warp_effect:?}"
    );

    // C4Shape::CompileFunc persists the fixed vertex arrays independently
    // of VtxNum. A save while WARP has reduced VtxNum to zero must therefore
    // retain the dormant CNAT/friction slots that AddVertex restores later.
    let saved_json = engine
        .capture_state()
        .to_json_string()
        .expect("mid-warp engine state serializes");
    let saved = clonk_engine::EngineState::from_json_str(&saved_json)
        .expect("mid-warp engine state deserializes");
    engine
        .restore_state(&saved)
        .expect("mid-warp engine state restores");
    let restored_warping_mage = engine
        .object_snapshot(mage)
        .expect("the mage survives the mid-warp restore");
    assert!(
        restored_warping_mage
            .effects
            .iter()
            .any(|effect| effect.name == "WarpUSpellData" && effect.priority != 0),
        "the live WarpUSpellData effect survives EngineState restore"
    );
    assert!(
        restored_warping_mage.vertices.is_empty(),
        "the restored mid-warp shape still has VtxNum zero"
    );

    let transferred = (0..80).any(|_| {
        engine
            .tick_without_snapshot()
            .expect("the restored real WARP pair advances");
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
    // TransferWarpObject enters AHUT before the zero return removes
    // WarpUSpellData. C4Object::Enter calls UpdateFace(true), restoring the
    // seven definition vertices; FxWarpUSpellDataStop then AddVertex-appends
    // the seven saved X/Y pairs. AddVertex does not copy CNAT/friction into
    // those new slots (C4Object.cpp:1621; C4Shape.cpp:26-32).
    let mut expected_warped_vertices = original_vertices.clone();
    expected_warped_vertices.extend(
        original_vertices
            .iter()
            .map(|vertex| clonk_engine::ObjectVertex::new(vertex.x, vertex.y)),
    );
    assert_eq!(
        warped_mage.vertices, expected_warped_vertices,
        "Enter restores the CLNK shape before WarpUSpellData Stop appends its saved coordinates"
    );
    assert!(
        warped_mage
            .effects
            .iter()
            .all(|effect| effect.name != "WarpUSpellData" || effect.priority == 0),
        "the per-object warp bookkeeping effect is dead after transfer"
    );
    assert!(warped_mage
        .effects
        .iter()
        .any(|effect| effect.name == "WarpUSpellData" && effect.priority == 0));
    engine
        .tick_without_snapshot()
        .expect("the mage's next effect Execute cleans WarpUSpellData");
    assert!(engine
        .object_snapshot(mage)
        .expect("the warped mage remains live")
        .effects
        .iter()
        .all(|effect| effect.name != "WarpUSpellData"));
}

fn alchemy_reincarnation_spell_revives_its_mage_during_assign_death(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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
            SpawnConfig::new("ALC_")
                .with_ordered_components(vec![("INEC".to_owned(), 1), ("IASH".to_owned(), 1)]),
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
        engine
            .tick_without_snapshot()
            .expect("XCRS's Magic action advances");
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
    engine
        .change_object_energy(mage_index, -100, 0, -1)
        .expect("apply lethal reincarnation damage");
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

fn alchemy_learned_group_heal_cast_sustains_magic_and_heals_nearby_crew(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy group-heal parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK selected");
    let patient = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "CLNK" && object.owner == owner && object.status.is_active()
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
    engine
        .change_object_energy(
            engine.find_object_index(patient).expect("patient index"),
            -20,
            0,
            -1,
        )
        .expect("injure the group-heal patient");
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
        engine
            .tick_without_snapshot()
            .expect("GGHG's healing effect advances");
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

fn alchemy_make_artefact_cast_opens_the_real_enchantment_menu(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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
            object.definition_id == "ALC_" && object.components.get("IGOL").copied() == Some(3)
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
        engine
            .tick_without_snapshot()
            .expect("MART's Magic action advances");
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

fn alchemy_make_artefact_hit_mode_casts_the_selected_spell_after_throw(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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
            object.definition_id == "ALC_" && object.components.get("IGOL").copied() == Some(3)
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
        engine
            .tick_without_snapshot()
            .expect("MART's Magic action advances");
    }

    // C4Menu::Enter executes AddMenuItem's command on MART's command object
    // (C4ObjectMenu.cpp:505-527). Select AIR1, then the learned LGCN spell,
    // hit activation, no delay, and ally target through those real controls
    // (Artefact.c4d/Script.c:198-218,266-421). The attached ALC_ bag rejects
    // ATTACH's forced Enter while its owner is contained, so MART's final
    // NoContainer scan offers that nearby bag as a combo object.
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
    let combo_index = engine
        .cursor_object_menu(owner)
        .expect("MART offers the callback-rejected attached bag as a combo")
        .1
        .items
        .iter()
        .position(|item| item.item_id == "ALC_")
        .expect("MART's combo menu exposes the shipped alchemy bag");
    engine
        .player_in_com(owner, COM_MENU_SELECT, combo_index as i32)
        .expect("the pointer selects the alchemy bag");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("Throw commits the selected combo object");
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
        Some(&EffectVarValue::Nil),
        "SetMode's pre-strict-3 call normalizes hit activation 0 to nil"
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
        engine
            .tick_without_snapshot()
            .expect("the mage leaves its Magic action");
    }
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("MCLK throws the configured ROCK");
    for _ in 0..240 {
        engine
            .tick_without_snapshot()
            .expect("the artefact throw advances");
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

fn alchemy_seeded_bag_collects_and_activates_through_player_controls(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy ingredient pickup parity");
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK cursor");
    let seeded_bag = engine
        .snapshot()
        .objects
        .iter()
        .find(|object| {
            object.definition_id == "ALC_" && object.components.get("IROC").copied() == Some(3)
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
        engine
            .tick_without_snapshot()
            .expect("execute the normal exit command");
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
        engine
            .tick_without_snapshot()
            .expect("run through the Tick3 collection pass");
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

fn alchemy_possession_uses_the_shipped_selector_control(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
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
            clonk_engine::SpawnConfig::new("POSE")
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

fn alchemy_combo_mode_opens_and_accepts_the_shipped_element_control(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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
            object.definition_id == "ALC_" && object.components.get("IROC").copied() == Some(3)
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
        .player_in_com(owner, clonk_engine::COM_DOWN, 0)
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
        .player_in_com(owner, clonk_engine::COM_DOWN, 0)
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
        engine
            .tick_without_snapshot()
            .expect("the shipped Magic action advances");
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

fn alchemy_learned_lightning_cast_launches_the_shipped_line_object(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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
            engine
                .tick_without_snapshot()
                .expect("MLGT's Magic action advances");
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
        engine
            .tick_without_snapshot()
            .expect("LGTS advances without a script error");
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

fn alchemy_learned_icestrike_aims_steers_and_impacts_through_player_controls(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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
            SpawnConfig::new("ALC_").with_ordered_components(vec![("ISPH".to_owned(), 1)]),
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
            engine
                .tick_without_snapshot()
                .expect("MICS's Magic action advances");
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
    engine
        .tick_without_snapshot()
        .expect("ICEB applies its steering speed");
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
    engine
        .tick_without_snapshot()
        .expect("the first frostwave radius executes");
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

fn alchemy_earthquake_cast_applies_the_shipped_view_shake(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
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
            object.definition_id == "ALC_" && object.components.get("IROC").copied() == Some(3)
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
        .apply_object_update(mage, ObjectUpdate::new().with_direction(Direction::Right))
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
            engine.tick_without_snapshot().expect("MQKE's Magic action advances");
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
    assert_eq!(
        quake_effect.var(2),
        EffectVarValue::Int((100 * a) / lifetime)
    );
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
    // to the process-owned live viewport, not synchronized C4Player state
    // (C4Script.cpp:5676-5687).
    engine.set_film_viewport_available(true);
    let view_offset = (0..8).find_map(|_| {
        engine
            .tick_without_snapshot()
            .expect("FXQ1's quake effect advances");
        engine
            .take_viewport_presentation_requests()
            .into_iter()
            .find_map(|request| match request {
                clonk_engine::ViewportPresentationRequest::SetViewOffset { player, offset }
                    if player == owner && offset != Vector2::ZERO =>
                {
                    Some(offset)
                }
                _ => None,
            })
    });
    assert!(
        view_offset.is_some(),
        "FXQ1 must write a non-zero C++ SetViewOffset while quake {quake:?} is active"
    );

    // Quake's action EndCall runs every three frames. Once ActTime exceeds
    // iLifeTime, the next successful Random(3) gate removes FXQ1
    // (Earthquake effect Script.c:7-19,31-45; ActMap.txt:3-10).
    let removed = (0..lifetime as usize + 64).any(|_| {
        engine
            .tick_without_snapshot()
            .expect("FXQ1 lifecycle advances");
        engine
            .object_snapshot(quake)
            .is_none_or(|quake| !quake.status.is_active())
    });
    assert!(removed, "FXQ1 removes itself after its shipped lifetime");
}

fn alchemy_small_force_field_timer_accepts_its_shipped_sound_flags(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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

fn alchemy_force_field_wall_puts_its_mask_before_segment_initialize(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy force-field-wall parity");
    // Frozen seed-zero open sky. Keeping this fixed makes IDs/positions a
    // stable real-scenario oracle instead of searching the generated map.
    let (wall_x, spell_y) = (990, 1370);
    let spell_position = Vector2::new(wall_x - 25, spell_y);

    let caster = engine
        .spawn_object(
            SpawnConfig::new("CLNK")
                .with_owner(owner)
                .with_position(spell_position)
                .with_direction(Direction::Right),
        )
        .expect("right-facing force-field caster spawns in open sky");
    let victim_position = Vector2::new(wall_x, spell_position.y - 100);
    let victim = engine
        .spawn_object(
            SpawnConfig::new("CLNK")
                .with_owner(owner)
                .with_position(victim_position),
        )
        .expect("crew overlapping the future first wall segment spawns");
    engine
        .apply_object_update(victim, ObjectUpdate::new().with_position(victim_position))
        .expect("place the full-grown crew exactly on the future segment center");
    assert!(
        (victim_position.y - 7..=victim_position.y + 9).all(|y| {
            (wall_x - 4..=wall_x + 11).all(|x| {
                !engine
                    .landscape()
                    .expect("Alchemy keeps its landscape")
                    .is_solid_at(x, y)
            })
        }),
        "the controlled CheckStuck region starts as open sky"
    );
    assert!(
        [
            (wall_x - 4, spell_y - 110),
            (wall_x - 3, spell_y - 110),
            (wall_x - 3, spell_y - 109),
            (wall_x, spell_y - 40),
            (wall_x + 2, spell_y + 28),
            (wall_x + 2, spell_y + 29),
            (wall_x + 3, spell_y + 29),
        ]
        .into_iter()
        .all(|(x, y)| !engine
            .landscape()
            .expect("Alchemy keeps its landscape")
            .is_solid_at(x, y)),
        "the sampled mask boundaries start as open sky"
    );
    let spell = engine
        .spawn_object(
            SpawnConfig::new("MFFW")
                .with_owner(owner)
                .with_position(spell_position),
        )
        .expect("the shipped MFFW spell object spawns");
    engine
        .apply_object_update(spell, ObjectUpdate::new().with_position(spell_position))
        .expect("pin the full-grown spell object at the controlled cast origin");

    // C4Game::NewObject performs initial DoCon (and therefore puts the
    // definition SolidMask) before Initialize. FCWS::Initialize inherits
    // FRCA::Initialize -> CheckStuck, so the first segment's phase-two mask
    // already ejects this same-x CLNK. The opaque mask is x=-3..=2 relative
    // to the segment and CLNK's leftmost vertex is -4; +7 is the first free
    // center (C4Object.cpp:1428-1511; C4SolidMask.cpp:61-107).
    assert_eq!(
        engine
            .call_object_function(
                engine.find_object_index(spell).expect("MFFW index"),
                "Activate",
                vec![Value::Object(caster.as_u64())],
            )
            .expect("the shipped MFFW activation runs"),
        Value::Int(1)
    );
    let ejected_victim = engine
        .object_snapshot(victim)
        .expect("ejected crew remains live");
    assert_eq!(
        ejected_victim.position,
        Vector2::new(wall_x + 7, victim_position.y),
        "FCWS's default mask must be script-visible during Initialize; vertices={:?}",
        ejected_victim.vertices
    );

    let spell_number = spell.as_u64();
    let controller_number = spell_number + 1;
    let segment_ids = (0..7)
        .map(|index| ObjectId::new(spell_number + 2 + index))
        .collect::<Vec<_>>();
    assert!(
        engine
            .object_snapshot(spell)
            .is_none_or(|spell| !spell.status.is_active()),
        "MFFW is assigned for removal after Activate"
    );
    assert!(
        engine
            .object_snapshot(ObjectId::new(controller_number))
            .is_none(),
        "FRCW expands synchronously and removes itself"
    );

    for (index, id) in segment_ids.iter().copied().enumerate() {
        let segment = engine
            .object_snapshot(id)
            .unwrap_or_else(|| panic!("FCWS segment {index} ({id}) remains live"));
        assert_eq!(segment.definition_id, "FCWS");
        assert_eq!(
            segment.position,
            Vector2::new(wall_x, spell_position.y - 100 + index as i32 * 20)
        );
        assert_eq!(segment.owner, OWNER_NONE);
        assert_eq!(segment.controller, owner);
        assert!(!segment.alive);
        assert_eq!(segment.action.name, "Field");
        assert_eq!(segment.action.phase, 0);
        assert_eq!(segment.action.target, None);
        let expected_last = index
            .checked_sub(1)
            .map(|previous| Value::Object(segment_ids[previous].as_u64()))
            .unwrap_or(Value::Nil);
        assert_eq!(
            segment.local_vars.get("pLast"),
            Some(&expected_last),
            "segment {index} links to its predecessor"
        );
        let expected_next = segment_ids
            .get(index + 1)
            .map(|next| Value::Object(next.as_u64()))
            .unwrap_or(Value::Nil);
        assert_eq!(
            segment.local_vars.get("pNext"),
            Some(&expected_next),
            "segment {index} links to its successor"
        );

        assert_eq!(segment.effects.len(), 2);
        let schedule = &segment.effects[0];
        assert_eq!(schedule.number, 1);
        assert_eq!(schedule.name, "IntScheduleCall");
        assert_eq!(schedule.priority, 1);
        assert_eq!(schedule.interval, 1);
        assert_eq!(schedule.timer, 0);
        assert_eq!(schedule.command_target, Some(id.as_u64() as i32));
        assert_eq!(
            schedule.vars,
            vec![
                EffectVarValue::String("UpdatePhase".into()),
                EffectVarValue::Int(1),
                EffectVarValue::Object(id.as_u64()),
                EffectVarValue::Nil,
                EffectVarValue::Nil,
                EffectVarValue::Nil,
                EffectVarValue::Nil,
                EffectVarValue::Nil,
            ]
        );
        let lifetime = &segment.effects[1];
        assert_eq!(lifetime.number, 2);
        assert_eq!(lifetime.name, "ForceFieldPSpell");
        assert_eq!(lifetime.priority, 150);
        assert_eq!(lifetime.interval, 5);
        assert_eq!(lifetime.timer, 0);
        assert_eq!(lifetime.command_target, Some(id.as_u64() as i32));
        assert!(lifetime.vars.is_empty());
        assert_eq!(engine.debug_solid_mask_override(id.as_u64()), Some(None));
    }

    // FCWS Graphics.png has opaque columns 1..=6. Before UpdatePhase all
    // seven definitions use the phase-two source mask (16,0,8,20), yielding
    // one continuous 6x140 strip at x=wall-3..=wall+2.
    let landscape = engine.landscape().expect("Alchemy keeps its landscape");
    assert!(landscape.is_solid_at(wall_x - 3, spell_position.y - 110));
    assert!(landscape.is_solid_at(wall_x + 2, spell_position.y + 29));
    assert!(!landscape.is_solid_at(wall_x - 4, spell_position.y - 110));
    assert!(!landscape.is_solid_at(wall_x + 3, spell_position.y + 29));

    engine
        .tick_without_snapshot()
        .expect("the seven scheduled phase updates execute");
    let expected_phases = [1, 2, 2, 2, 2, 2, 3];
    for (index, id) in segment_ids.iter().copied().enumerate() {
        let segment = engine
            .object_snapshot(id)
            .unwrap_or_else(|| panic!("FCWS segment {index} survives its first tick"));
        assert_eq!(segment.action.phase, expected_phases[index]);
        let active_effects = segment
            .effects
            .iter()
            .filter(|effect| effect.priority != 0)
            .collect::<Vec<_>>();
        assert_eq!(active_effects.len(), 1);
        assert_eq!(active_effects[0].number, 2);
        assert_eq!(active_effects[0].name, "ForceFieldPSpell");
        assert_eq!(active_effects[0].timer, 1);
        assert_eq!(
            engine.debug_solid_mask_override(id.as_u64()),
            Some(Some((expected_phases[index] * 8, 0, 8, 20)))
        );
    }

    // Phase one drops the top alpha row and phase three drops the bottom;
    // the joined post-schedule mask is a continuous 6x138 strip.
    let landscape = engine.landscape().expect("wall masks remain baked");
    assert_eq!(
        [
            landscape.is_solid_at(wall_x - 3, spell_position.y - 110),
            landscape.is_solid_at(wall_x - 3, spell_position.y - 109),
            landscape.is_solid_at(wall_x + 2, spell_position.y + 28),
            landscape.is_solid_at(wall_x + 2, spell_position.y + 29),
        ],
        [false, true, true, false]
    );

    // Damage shortens FCWS's effect clock by (1000/50)*damage. Applying the
    // shipped maximum damage avoids a 1000-frame test while still reaching
    // the ordinary interval-five FxForceFieldPSpellTimer -> Stop -> Destroy
    // lifecycle (segment Script.c:54-62; FRCA Script.c:43-71).
    for (index, id) in segment_ids.iter().copied().enumerate() {
        engine
            .call_object_function(
                engine
                    .find_object_index(id)
                    .unwrap_or_else(|| panic!("damaged FCWS segment {index} remains live")),
                "Damage",
                vec![Value::Int(50)],
            )
            .unwrap_or_else(|error| panic!("FCWS segment {index} accepts Damage: {error}"));
    }
    for _ in 0..4 {
        engine
            .tick_without_snapshot()
            .expect("the next interval-five force-field timer approaches");
    }
    assert!(
        segment_ids
            .iter()
            .all(|id| engine.object_snapshot(*id).is_none()),
        "the seven expired FCWS segments are removed together"
    );
    let landscape = engine
        .landscape()
        .expect("expired wall restores the landscape");
    assert!(
        [
            (wall_x - 3, spell_position.y - 109),
            (wall_x, spell_position.y - 40),
            (wall_x + 2, spell_position.y + 28),
        ]
        .into_iter()
        .all(|(x, y)| !landscape.is_solid_at(x, y)),
        "FCWS expiry clears every sampled part of the wall mask"
    );
}

fn alchemy_firelump_collects_its_same_call_fireball_into_the_mage(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
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
                vec![Value::Object(mage.as_u64()), Value::Object(mage.as_u64()),],
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
fn drachenfels_real_scenario_subcases_batch_1() {
    run_drachenfels_batch(&[
        (
            "script25_casts_cpp_sparks_and_completes_intro_step",
            dragon_rock_script25_casts_cpp_sparks_and_completes_intro_step,
        ),
        (
            "objects_keep_their_multidirectional_action_rows",
            dragon_rock_objects_keep_their_multidirectional_action_rows,
        ),
        (
            "scroll_transfer_zone_callbacks_persist_cpp_names",
            dragon_rock_scroll_transfer_zone_callbacks_persist_cpp_names,
        ),
        (
            "setdir_folds_the_flipdir_mirror_into_the_draw_transform",
            dragon_rock_setdir_folds_the_flipdir_mirror_into_the_draw_transform,
        ),
    ]);
}

#[test]
fn drachenfels_real_scenario_subcases_batch_2() {
    run_drachenfels_batch(&[
        (
            "object_lookup_carries_script1_state_into_script3",
            dragon_rock_object_lookup_carries_script1_state_into_script3,
        ),
        (
            "mage_choice_redefines_the_real_knight_and_transfers_its_flag",
            dragon_rock_mage_choice_redefines_the_real_knight_and_transfers_its_flag,
        ),
        (
            "objects_restore_serialized_c4id_named_locals",
            dragon_rock_objects_restore_serialized_c4id_named_locals,
        ),
    ]);
}

#[test]
fn drachenfels_real_scenario_subcases_batch_3() {
    run_drachenfels_batch(&[
        (
            "endboss_death_kills_the_shipped_dragon",
            dragon_rock_endboss_death_kills_the_shipped_dragon,
        ),
        (
            "walk_up_enters_the_shipped_tent",
            dragon_rock_walk_up_enters_the_shipped_tent,
        ),
        (
            "initialize_player_grants_both_plan_knowledge_sets",
            dragon_rock_initialize_player_grants_both_plan_knowledge_sets,
        ),
        (
            "real_schedule_enables_and_forces_player_fog_of_war",
            dragon_rock_real_schedule_enables_and_forces_player_fog_of_war,
        ),
    ]);
}

#[test]
fn drachenfels_real_scenario_subcases_batch_4() {
    run_drachenfels_batch(&[(
        "shadow_generators_darken_the_mountain_until_a_clonk_walks_in",
        dragon_rock_shadow_generators_darken_the_mountain_until_a_clonk_walks_in,
    )]);
}

fn run_drachenfels_batch(subcases: &[PreparedScenarioSubcase]) {
    run_prepared_scenario_batch("Drachenfels", "Fantasy.c4f/Drachenfels.c4s", subcases);
}

/// The four saved `_FOW` shadow volumes, as `Objects.txt` ships them:
/// object number, position and the `PlrViewRange` its own
/// `SetPlrViewRange(Min((w+h)/-2+40, -1))` produced
/// (Drachenfels.c4s/FoWGenerator.c4d/Script.c:74,95).
const DRACHENFELS_SHADOWS: [(u64, i32, i32, i32); 4] = [
    (2779, 1472, 1303, -235),
    (2781, 2058, 1393, -356),
    (3835, 1968, 923, -257),
    (3905, 2092, 742, -247),
];

/// Dragon Rock is the only shipped content that uses a negative
/// `PlrViewRange` as a persistent, map-authored shadow volume. Four `_FOW`
/// generators darken the mountain — negative ranges are applied after every
/// repeller so they override it (`C4Player::FoWGenerators2Map`,
/// C4Player.cpp:1949-1957) — and hold 181 objects in `C4OS_INACTIVE` between
/// them. Each one's `Active` row wraps on `NextAction=Active` every 20 ticks,
/// and C++ re-issues `StartCall` for that self-chain (C4Object.cpp:5480-5485),
/// which is what polls `CheckClonk`. Walking a crew member into the authored
/// search rect is therefore the whole reveal mechanism
/// (FoWGenerator.c4d/{ActMap.txt,Script.c:104-124}).
fn dragon_rock_shadow_generators_darken_the_mountain_until_a_clonk_walks_in(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player_on_team(&mut engine, "Dragon Rock fog reveal parity", 1);

    // Choose normal difficulty and the initially selected KNIG so ordinary
    // crew control resumes (Drachenfels.c4s/Script.c:86-128,150-178).
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("choose normal difficulty");
    engine
        .player_in_com(owner, COM_THROW, 0)
        .expect("choose KNIG");
    let knight = engine
        .crew_cursor(owner)
        .expect("Dragon Rock character choice leaves a cursor");

    // InitializePlayer schedules SetFoW one tick out (Drachenfels.c4s/
    // Script.c:70; planet/System.c4g/Helpers.c:110-132).
    engine
        .tick_without_snapshot()
        .expect("the real IntSchedule callback evaluates SetFoW");
    assert!(engine
        .player(owner)
        .expect("joined player remains live")
        .fog_of_war());

    let snapshot = engine.snapshot();
    let mut shadows = snapshot
        .objects
        .iter()
        .filter(|object| object.definition_id == "_FOW" && object.status.is_active())
        .map(|object| {
            (
                object.id.as_u64(),
                object.position.x,
                object.position.y,
                object.plr_view_range,
            )
        })
        .collect::<Vec<_>>();
    shadows.sort_unstable();
    assert_eq!(
        shadows,
        DRACHENFELS_SHADOWS.to_vec(),
        "the shipped Objects.txt generators must load with their saved ranges"
    );

    // `Initialize` calls `SetOwner(-1)`, so `C4Object::PlrFoWActualize` adds
    // each generator to *every* player's FoW list (C4Object.cpp:5546-5567).
    let view_objects = snapshot
        .fow_players
        .get(&owner)
        .map(|frame| frame.view_objects.clone())
        .expect("a fog-of-war player projects its runtime FoWViewObjs");
    for (number, ..) in DRACHENFELS_SHADOWS {
        assert!(
            view_objects.contains(&ObjectId::new(number)),
            "ownerless generator #{number} must repel fog for every player"
        );
    }

    // The same four generators deactivated 181 objects before the map was
    // saved; `Status=2` restores them as `C4OS_INACTIVE`.
    assert_eq!(
        snapshot
            .objects
            .iter()
            .filter(|object| object.status == ObjectStatus::Inactive)
            .count(),
        181,
        "the shipped Objects.txt keeps the hidden mountain interior inactive"
    );

    // Generator 2779 is centred at (1472,1303) with `search=(-320,-160,600,280)`,
    // i.e. world rect [1152,1752] x [1143,1423] once `Find_InRect` adds
    // `GetX()/GetY()` (planet/System.c4g/FindObject.c:37).
    let shadow = ObjectId::new(2779);
    let hidden = drachenfels_hidden_objects(
        &engine
            .object_snapshot(shadow)
            .expect("generator #2779 loads"),
    );
    assert_eq!(hidden.len(), 24, "generator #2779 saved iHiddenObjCnt=24");

    // Just outside the authored rect — one pixel above its top edge. The
    // Clonk is already 159px inside the 235px fully-black disc here, which is
    // exactly the reported lateness. `C4FindObjectInRect::Check` is a plain
    // point-in-rect on the object centre, so the boundary is exact; the crew
    // member is re-placed every tick so gravity cannot walk it in and pass
    // this probe for the wrong reason.
    assert_eq!(
        drachenfels_ticks_until_dispelled(
            &mut engine,
            knight,
            shadow,
            Vector2::new(1472, 1142),
            40
        ),
        None,
        "a Clonk outside the authored search rect must not dispel the shadow"
    );

    // Two pixels lower is inside, and one 20-tick `Active` poll dispels it.
    // `Deactivate` then restores every object it was hiding (Script.c:96-112).
    assert!(
        drachenfels_ticks_until_dispelled(
            &mut engine,
            knight,
            shadow,
            Vector2::new(1472, 1144),
            20
        )
        .is_some(),
        "a Clonk inside the authored search rect dispels the shadow within one poll"
    );
    for object in hidden {
        assert_eq!(
            engine
                .object_snapshot(object)
                .map(|object| object.status)
                .unwrap_or(ObjectStatus::Deleted),
            ObjectStatus::Normal,
            "Deactivate must return hidden object {object} to C4OS_NORMAL"
        );
    }
    assert!(
        !engine
            .snapshot()
            .fow_players
            .get(&owner)
            .expect("the player still has fog of war")
            .view_objects
            .contains(&shadow),
        "a removed generator leaves every player's FoWViewObjs"
    );
}

/// The objects a `_FOW` generator is holding inactive, read from the numbered
/// locals `Activate` filled (`Local(iHiddenObjCnt++) = pObj`,
/// Drachenfels.c4s/FoWGenerator.c4d/Script.c:86-93). Numbered `Locals=` are
/// stored under the `__local_<n>` keys `indexed_local_index` reads back.
fn drachenfels_hidden_objects(shadow: &ObjectSnapshot) -> Vec<ObjectId> {
    let count = match shadow.local_vars.get("iHiddenObjCnt") {
        Some(Value::Int(count)) => *count,
        _ => 0,
    };
    (0..count)
        .filter_map(|index| shadow.local_vars.get(&format!("__local_{index}")))
        .filter_map(|value| match value {
            Value::Object(id) => Some(ObjectId::new(*id)),
            _ => None,
        })
        .collect()
}

/// Hold `crew` at `position` and tick, returning the poll on which `shadow`
/// removed itself. Re-placing every tick keeps gravity from carrying the probe
/// across the boundary being measured.
fn drachenfels_ticks_until_dispelled(
    engine: &mut Engine,
    crew: ObjectId,
    shadow: ObjectId,
    position: Vector2,
    ticks: u32,
) -> Option<u32> {
    (1..=ticks).find(|_| {
        engine
            .apply_object_update(
                crew,
                ObjectUpdate::new()
                    .with_position(position)
                    .with_action("Walk")
                    .clear_container(),
            )
            .expect("hold the crew member at the probe position");
        engine
            .tick_without_snapshot()
            .expect("the generator keeps polling CheckClonk");
        engine.object_snapshot(shadow).is_none()
    })
}

fn dragon_rock_mage_choice_redefines_the_real_knight_and_transfers_its_flag(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player_on_team(&mut engine, "Dragon Rock character parity", 1);
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

fn dragon_rock_walk_up_enters_the_shipped_tent(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player_on_team(&mut engine, "Dragon Rock tent-entry parity", 1);

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
        tent.ocf & clonk_engine::ocf::ENTRANCE,
        0,
        "the shipped TENT remains targetable through OCF_Entrance"
    );
    // TENT DefCore.txt:17 is Entrance=-10,4,19,20.
    let entrance_center = Vector2::new(tent.position.x - 10 + 19 / 2, tent.position.y + 4 + 20 / 2);
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
        engine
            .tick_without_snapshot()
            .expect("queued Enter command advances");
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

fn dragon_rock_initialize_player_grants_both_plan_knowledge_sets(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player_on_team(&mut engine, "Dragon Rock knowledge parity", 1);

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
        "ADM1", "ADM3", "ANVL", "ARCH", "ARMR", "ARWP", "AXE1", "BALN", "BANP", "BARL", "BAS7",
        "BED1", "BLMP", "BOW1", "BRDG", "BRED", "BWRC", "CANN", "CATA", "CHEM", "CLD1", "CNDL",
        "CNKT", "COKI", "CPAL", "CPEL", "CPH1", "CPHC", "CPKT", "CPOF", "CPR1", "CPR2", "CPT1",
        "CPT2", "CPT3", "CPT4", "CPTL", "CPTR", "CPW1", "CPW2", "CPWK", "CPWL", "CPWR", "DCO3",
        "DCO4", "DOGH", "DPOT", "DRCK", "EFLN", "ELEV", "FARP", "FBMP", "FDRS", "FLNT", "FNDR",
        "FRGE", "GUNP", "HUT1", "HUT2", "HUT3", "KSDL", "LANC", "LNKT", "LORY", "OVEN", "PAL2",
        "PALS", "PFIR", "PHEA", "POWR", "PSTO", "PUMP", "RSRC", "SAWM", "SFLN", "SHIE", "SHRC",
        "SLBT", "SPER", "SPRC", "STFN", "SWOR", "SWRC", "TABL", "TENP", "TFLN", "THRN", "TWR2",
        "WDBR", "WGTW", "WMIL", "WODC", "WRKS", "WTWR", "WZKP", "XARP", "XBOW",
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

fn dragon_rock_real_schedule_enables_and_forces_player_fog_of_war(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player_on_team(&mut engine, "Dragon Rock fog parity", 1);
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
        EffectVarValue::String(format!("SetFoW(true, {owner})").into())
    );
    assert_eq!(schedules[0].var(1), EffectVarValue::Int(1));

    engine
        .tick_without_snapshot()
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
            .all(|effect| effect.name != "IntSchedule" || effect.priority == 0),
        "successful eval reaches Helpers.c's one-shot kill return"
    );
    let player = engine.player(owner).expect("joined player remains live");
    assert!(player.fog_of_war());
    assert!(player.force_fog_of_war());
    let persisted = player.to_state();
    assert!(persisted.fog_of_war);
    assert!(persisted.force_fog_of_war);
}

fn dragon_rock_objects_keep_their_multidirectional_action_rows(
    prepared: &PreparedInstalledScenario,
) {
    let engine = prepared.instantiate();

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

fn dragon_rock_setdir_folds_the_flipdir_mirror_into_the_draw_transform(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let dragon = ObjectId::new(202);

    // Objects.txt ships DRGN #202 as Action=Sleep / Dir=1 /
    // DrawTransform=-1,0,0,0,1,0,-1, and Sleep declares FlipDir=1
    // (Dragon.c4d/ActMap.txt). C4DrawTransform folds the mirror into mat[0]
    // (C4Facet.h:79-88), and the post-load sweep re-runs UpdateFlipDir
    // (C4GameObjects.cpp:670-673): Dir 1 >= FlipDir 1 keeps SetFlipDir(-1),
    // so the saved mirror survives the load unchanged.
    let loaded = engine.object_snapshot(dragon).expect("dragon loads");
    assert_eq!(loaded.direction.to_script_value(), 1);
    let transform = loaded
        .draw_transform
        .expect("the saved FlipDir mirror survives loading");
    assert_eq!(transform.flip_dir(), -1);
    assert_eq!(transform.matrix()[0], -1.0);

    // ReverseDir (Dragon.c4d/Script.c) is exactly `SetDir(1-GetDir())`.
    // C4Object::SetDir runs UpdateFlipDir because Sleep's FlipDir is
    // non-zero (C4Object.cpp:4276-4279); Dir 0 < FlipDir 1 then takes the
    // "no flipdir necessary" branch, so SetFlipDir(1) unfolds mat[0] and the
    // now-identity transform is deleted (C4Object.cpp:431-442).
    let index = engine.find_object_index(dragon).expect("dragon index");
    engine
        .call_object_function(index, "ReverseDir", Vec::new())
        .expect("ReverseDir is callable");

    let turned = engine.object_snapshot(dragon).expect("dragon stays live");
    assert_eq!(turned.direction.to_script_value(), 0);
    assert_eq!(
        turned.draw_transform, None,
        "UpdateFlipDir resets FlipDir to +1, which leaves an identity matrix, \
         and C++ deletes it (C4Object.cpp:436-442)"
    );

    // Turning back re-enters the mirrored range with no transform to fold
    // into, which is C++'s `pDrawTransform = new C4DrawTransform(-1)`: FlipDir
    // -1 applied through C4DrawTransform::Set leaves mat[0] = -1
    // (C4Object.cpp:425-427, C4Facet.h:63-70,79-81). The renderer consumes
    // that matrix directly, so the mirror has to survive here or a
    // right-facing dragon silently draws unmirrored.
    engine
        .call_object_function(index, "ReverseDir", Vec::new())
        .expect("ReverseDir is callable");

    let returned = engine.object_snapshot(dragon).expect("dragon stays live");
    assert_eq!(returned.direction.to_script_value(), 1);
    let transform = returned
        .draw_transform
        .expect("re-entering the mirrored range re-creates the draw transform");
    assert_eq!(transform.flip_dir(), -1);
    assert_eq!(transform.matrix()[0], -1.0);
}

fn dragon_rock_objects_restore_serialized_c4id_named_locals(prepared: &PreparedInstalledScenario) {
    let engine = prepared.instantiate();

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
                "GGHG", "GZ9Z", "ABLA", "MBOT", "MBLS", "MFRB", "MFBL", "FRFS", "MBRG", "EH69",
                "CMFG",
            ][..],
        ),
        (
            4410,
            &["GZ9Z", "CMFG", "MFFW", "MBRG", "EH69", "EXTG", "ELX2"][..],
        ),
        (
            2550,
            &[
                "CMFG", "MFFW", "ABLA", "MBRG", "EXTG", "MGHL", "MLGT", "ETFL", "MFRB", "MDBT",
                "MFBL", "RUND", "MBLS", "CPAN", "CFAL", "MGBW", "MICS", "ELX1", "GZ9Z",
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

fn dragon_rock_scroll_transfer_zone_callbacks_persist_cpp_names(
    prepared: &PreparedInstalledScenario,
) {
    let engine = prepared.instantiate();

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

fn dragon_rock_object_lookup_carries_script1_state_into_script3(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    join_local_player_on_team(&mut engine, "Dragon Rock intro object parity", 1);

    // InitializePlayer starts the ordinary C4GameScriptHost counter. Its
    // every-tenth-frame Execute post-increments Counter before calling
    // Script%d (C4ScriptHost.cpp:222-230), so Script1 runs on frame 20.
    for _ in 0..20 {
        engine
            .tick_without_snapshot()
            .expect("Dragon Rock reaches shipped Script1");
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
        engine
            .tick_without_snapshot()
            .expect("Dragon Rock reaches shipped Script3");
    }
    let globals = &engine.snapshot().script_globals.named;
    assert_eq!(globals.get("DRGN_ctrl_tx"), Some(&Value::Int(400)));
    assert_eq!(globals.get("DRGN_ctrl_ty"), Some(&Value::Int(1050)));
    assert!(matches!(
        globals.get("DRGN_ctrl_stop"),
        None | Some(Value::Nil) | Some(Value::Bool(false)) | Some(Value::Int(0))
    ));
}

fn dragon_rock_endboss_death_kills_the_shipped_dragon(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
    join_local_player_on_team(&mut engine, "Dragon Rock Kill parity", 1);

    // Script1 binds the shipped object numbers to g_pEndboss/g_pDragon.
    // OnClonkDeath then calls Kill(g_pDragon) when the endboss dies
    // (Drachenfels.c4s/Script.c:438-454). C++ routes that native through
    // AssignDeath, including the Dead action and Death callback.
    for _ in 0..20 {
        engine
            .tick_without_snapshot()
            .expect("Dragon Rock reaches shipped Script1");
    }
    let dragon_id = ObjectId::new(202);
    let dragon_before = engine
        .object_snapshot(dragon_id)
        .expect("Dragon Rock ships object #202 as its dragon");
    assert!(dragon_before.alive, "the dragon starts alive");

    engine
        .call_scenario_script_function(
            "OnClonkDeath",
            vec![Value::Object(ObjectId::new(1758).as_u64())],
        )
        .expect("the real endboss-death callback completes");

    let dragon_after = engine
        .object_snapshot(dragon_id)
        .expect("AssignDeath retains the dead dragon object");
    assert!(!dragon_after.alive, "Kill marks the dragon dead");
    assert_eq!(
        dragon_after.action.name, "Dead",
        "Kill uses the full AssignDeath action transition"
    );
}

fn dragon_rock_script25_casts_cpp_sparks_and_completes_intro_step(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    join_local_player_on_team(&mut engine, "Dragon Rock CastObjects parity", 1);

    // Let the shipped counter reach Script15's pause, then resume it through
    // the real dragon-arrival callback (Drachenfels.c4s/Script.c:286-294).
    // Counter 20 is intentionally empty; Script21 runs at frame 180 and
    // Script25 naturally runs at frame 220.
    for _ in 0..160 {
        engine
            .tick_without_snapshot()
            .expect("Dragon Rock reaches Script15 pause");
    }
    engine
        .call_scenario_script_function("OnDragonReachTarget", Vec::new())
        .expect("real dragon arrival resumes the intro counter");
    for _ in 0..59 {
        engine
            .tick_without_snapshot()
            .expect("Dragon Rock approaches Script25");
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
        let shape = engine
            .object_current_shape_rect(spark.id)
            .expect("SPRK keeps its definition-derived shape");
        assert_eq!(
            spark.position.y + shape.y + shape.height,
            princess_before.position.y,
            "Oversize Completion growth preserves the C++ spawn-bottom anchor"
        );
        let fixed_velocity = spark
            .fixed_velocity
            .unwrap_or_else(|| math::FixedVec2::from_ints(spark.velocity.x, spark.velocity.y));
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

fn alchemy_tunnel_spell_opens_its_first_shipped_landscape_row(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy tunnel parity");
    let mage = engine.crew_cursor(owner).expect("Alchemy MCLK cursor");
    let earth = engine
        .materials()
        .id_of("Earth")
        .expect("Alchemy loads Earth");
    let (target_x, target_y) = {
        let landscape = engine.landscape().expect("Alchemy keeps its landscape");
        let grid = landscape
            .pixel_grid()
            .expect("Alchemy has a raster landscape");
        (20..grid.height() as i32 - 20)
            .find_map(|y| {
                (20..grid.width() as i32 - 20)
                    .find(|&x| {
                        landscape.material_at(x, y) == Some(earth) && landscape.is_solid_at(x, y)
                    })
                    .map(|x| (x, y))
            })
            .expect("Alchemy contains an interior solid Earth pixel")
    };
    let solid_pixels_before = {
        let landscape = engine.landscape().expect("landscape before tunnel");
        let grid = landscape
            .pixel_grid()
            .expect("Alchemy raster before tunnel");
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

    engine
        .tick_without_snapshot()
        .expect("the tunnel effect reaches time zero");
    engine
        .tick_without_snapshot()
        .expect("the first tunnel row timer executes");
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

fn alchemy_firefist_flame_consumes_inflammable_landscape(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
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
        let grid = landscape
            .pixel_grid()
            .expect("Alchemy has a raster landscape");
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
    let shower_index = engine.find_object_index(fire_shower).expect("FSHW index");
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

#[test]
fn goldrush_real_scenario_subcases_batch_1() {
    run_goldrush_batch(&[
        (
            "sheriff_watch_energy_stop_removes_crew_and_completes",
            gold_rush_sheriff_watch_energy_stop_removes_crew_and_completes,
        ),
        (
            "trap_arm_check_uses_the_live_cpp_shape_offset",
            gold_rush_trap_arm_check_uses_the_live_cpp_shape_offset,
        ),
        (
            "incomplete_dynamite_box_ignition_errors_before_exploding",
            gold_rush_incomplete_dynamite_box_ignition_errors_before_exploding,
        ),
    ]);
}

#[test]
fn goldrush_real_scenario_subcases_batch_2() {
    run_goldrush_batch(&[
        (
            "scorching_timer_returns_kill_before_playing_sound",
            gold_rush_scorching_timer_returns_kill_before_playing_sound,
        ),
        (
            "fade_out_retimes_its_existing_effect_through_change_effect",
            gold_rush_fade_out_retimes_its_existing_effect_through_change_effect,
        ),
        (
            "real_anvil_forges_a_wire_roll_from_its_metal_contents",
            gold_rush_real_anvil_forges_a_wire_roll_from_its_metal_contents,
        ),
        (
            "stalactite_hit_spins_same_call_created_fragments",
            gold_rush_stalactite_hit_spins_same_call_created_fragments,
        ),
    ]);
}

fn run_goldrush_batch(subcases: &[PreparedScenarioSubcase]) {
    run_prepared_scenario_batch("Goldrush", "Western.c4f/Goldrush.c4s", subcases);
}

fn gold_rush_trap_arm_check_uses_the_live_cpp_shape_offset(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
    let base_x = engine
        .snapshot()
        .objects
        .iter()
        .map(|object| object.position.x)
        .max()
        .unwrap_or(0)
        .saturating_add(1_000);
    engine.set_landscape(Landscape::flat(
        u32::try_from(base_x.saturating_add(100)).expect("isolated trap fixture width"),
        60,
    ));

    // The shipped formula in TRAP::ArmCheck is
    // (Dir*2-1)*(TGIN Width 8 + Offset.x -4 + TRPR Width 16 +
    // Offset.x -8 + 3) = +15. At y=50 over the flat y=60 surface, its
    // first upward probe leaves solid at i=1 and returns local y=9.
    let trapper = engine
        .spawn_object(
            SpawnConfig::new("TRPR")
                .with_position(Vector2::new(base_x, 50))
                .with_direction(Direction::Right)
                .with_alive(true)
                .with_loaded(true),
        )
        .expect("the real Western trapper spawns");
    let trap = engine
        .spawn_object(
            SpawnConfig::new("TGIN")
                .with_position(Vector2::new(base_x, 50))
                .with_loaded(true),
        )
        .expect("the real Western gin trap spawns");
    let trap_index = engine.find_object_index(trap).expect("gin trap index");

    assert_eq!(
        engine
            .call_object_function(
                trap_index,
                "ArmCheck",
                vec![Value::Object(trapper.as_u64())],
            )
            .expect("the shipped inherited TRAP::ArmCheck runs"),
        Value::Array(vec![Value::Int(15), Value::Int(9)])
    );
}

fn gold_rush_stalactite_hit_spins_same_call_created_fragments(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let stalactite = ObjectId::new(450);
    let old_fragments = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "_STP")
        .map(|object| object.id)
        .collect::<Vec<_>>();

    let stalactite_index = engine
        .find_object_index(stalactite)
        .expect("GoldRush Objects.txt stalactite #450 is live");
    engine
        .call_object_function(stalactite_index, "Hit", Vec::new())
        .expect("the shipped _STA::Hit callback completes");

    assert!(
        engine
            .object_snapshot(stalactite)
            .is_none_or(|object| !object.status.is_active()),
        "_STA::Hit removes its source stalactite"
    );
    let fragments = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| {
            object.definition_id == "_STP"
                && object.status.is_active()
                && !old_fragments.contains(&object.id)
        })
        .collect::<Vec<_>>();
    assert_eq!(fragments.len(), 3, "_STA::Hit creates three _STP pieces");

    // Every SetRDir targets a fragment created earlier in this same script
    // callback. C++ inserts those objects synchronously; Rust's deferred
    // spawn path must retain and fold the foreign-object angular writes.
    let rotation_velocities = fragments
        .iter()
        .map(|fragment| fragment.rotation_velocity.unwrap_or_default().val())
        .collect::<Vec<_>>();
    assert_eq!(
        rotation_velocities,
        vec![
            math::itofix_prec(0, 10).val(),
            math::itofix_prec(-6, 10).val(),
            math::itofix_prec(-5, 10).val(),
        ],
        "seed 0 preserves all three shipped Random(20)-10 SetRDir writes"
    );
}

fn gold_rush_fade_out_retimes_its_existing_effect_through_change_effect(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    assert!(
        engine.debug_global_has_function("FadeOut"),
        "GoldRush Helpers.c supplies FadeOut to the global script layer"
    );
    engine
        .register_definition(
            Definition::from_script(
                "FDRV",
                "Fade driver",
                r#"#strict
func StartFade() { return FadeOut(4, 2, this()); }
func RetargetFade() { return FadeOut(10, 5, this()); }
"#,
            )
            .expect("fade driver compiles"),
        )
        .expect("fade driver registers against the installed global table");
    let target = engine
        .spawn_object(SpawnConfig::new("FDRV").with_position(Vector2::new(320, 120)))
        .expect("the fade driver spawns");

    let index = engine
        .find_object_index(target)
        .expect("fade driver exists");
    assert_eq!(
        engine
            .call_object_function(index, "StartFade", Vec::new())
            .expect("the shipped FadeOut starts"),
        Value::Int(1)
    );
    let index = engine
        .find_object_index(target)
        .expect("fade driver remains");
    assert_eq!(
        engine
            .call_object_function(index, "RetargetFade", Vec::new())
            .expect("the shipped FxIntFadeAdd path completes"),
        Value::Int(1)
    );

    let object = engine
        .object_snapshot(target)
        .expect("fade driver remains active");
    let fades = object
        .effects
        .iter()
        .filter(|effect| effect.name == "IntFade" && effect.priority != 0)
        .collect::<Vec<_>>();
    assert_eq!(fades.len(), 1, "the second FadeOut merges into the first");
    assert!(object
        .effects
        .iter()
        .any(|effect| effect.name == "IntFade" && effect.priority == 0));
    assert_eq!(fades[0].interval, 10, "ChangeEffect installs the new timer");
    assert_eq!(fades[0].timer, 0, "ChangeEffect resets effect time");
    assert_eq!(
        fades[0].var(0),
        EffectVarValue::Int(5),
        "FxIntFadeAdd keeps the shipped fade direction write"
    );
}

fn gold_rush_scorching_timer_returns_kill_before_playing_sound(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    assert!(engine.debug_global_has_function("SetScorching"));
    assert!(engine.debug_global_has_function("FxIntScorchingTimer"));
    let mut driver = Definition::from_script(
        "SCRH",
        "Scorching driver",
        "#strict\nfunc StartScorching() { return SetScorching(this()); }",
    )
    .expect("scorching driver compiles");
    driver.set_c4_callback_convention(true);
    engine
        .register_definition(driver)
        .expect("scorching driver registers");
    let target = engine
        .spawn_object(SpawnConfig::new("SCRH").with_position(Vector2::new(320, 120)))
        .expect("scorching driver spawns");
    let index = engine.find_object_index(target).expect("driver exists");
    engine
        .call_object_function(index, "StartScorching", Vec::new())
        .expect("the shipped SetScorching helper runs");
    assert!(
        engine
            .object_snapshot(target)
            .expect("driver remains active")
            .effects
            .iter()
            .any(|effect| effect.name == "IntScorching"),
        "SetScorching installs the shipped smoke effect"
    );

    for _ in 0..10 {
        engine
            .tick_without_snapshot()
            .expect("the scorching timer approaches");
    }
    assert!(
        engine
            .object_snapshot(target)
            .expect("driver remains active")
            .effects
            .iter()
            .all(|effect| effect.name != "IntScorching" || effect.priority == 0),
        "return (FX_Execute_Kill, Sound(...)) returns the kill code after playing the sound"
    );
}

fn gold_rush_incomplete_dynamite_box_ignition_errors_before_exploding(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let dynamite_box = engine
        .spawn_object(
            SpawnConfig::new("DYNB")
                .with_position(Vector2::new(320, 120))
                .with_construction(FULL_CON / 2),
        )
        .expect("the incomplete shipped dynamite box spawns");
    let index = engine
        .find_object_index(dynamite_box)
        .expect("dynamite box index");
    let error = engine
        .call_object_function(index, "Ignition", Vec::new())
        .expect_err("assigning through `!iCount` must fail like C++");
    let EngineError::Script { source, .. } = error else {
        panic!("unexpected ignition error: {error}");
    };
    assert!(
        source.to_string().contains("operator \"=\" left side")
            && source.to_string().contains("bool"),
        "unexpected script error: {source}"
    );

    let box_after = engine
        .object_snapshot(dynamite_box)
        .expect("the failed ignition must not remove or explode the box");
    assert!(box_after.status.is_active());
    assert_eq!(box_after.construction, FULL_CON / 2);
}

fn gold_rush_sheriff_watch_energy_stop_removes_crew_and_completes(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    assert_eq!(
        engine.debug_definition_has_function("_TLK", "FxWatchEnergyStop"),
        Some(true),
        "M_5AshCity_DlgSheriff.c appends the callback to the shipped talker"
    );
    let owner = join_local_player(&mut engine, "GoldRush sheriff parity");
    let target = engine
        .crew_cursor(owner)
        .expect("GoldRush joins a selected info-bearing crew member");
    assert!(
        engine.crew_object_info(target).is_some(),
        "the removal path must exercise this player's real CrewInfoList"
    );
    engine
        .apply_object_update(target, ObjectUpdate::new().with_energy(1))
        .expect("seed the sheriff damage-stop path");

    let talker = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "_TLK" && object.status.is_active())
        .map(|object| object.id)
        .expect("GoldRush Objects.txt contains a live talker");
    let talker_index = engine.find_object_index(talker).expect("talker index");
    engine
        .call_object_function(
            talker_index,
            "FxWatchEnergyStop",
            vec![
                Value::Object(target.as_u64()),
                Value::Int(99_999),
                Value::Int(0),
                Value::Bool(false),
            ],
        )
        .expect("the shipped sheriff stop callback reaches its tail");

    let sheriff = engine
        .object_snapshot(target)
        .expect("the transformed sheriff remains live");
    assert_eq!(sheriff.definition_id, "SHRF");
    assert!(!sheriff.crew_member);
    assert!(!sheriff.selected);
    assert!(engine.crew_object_info(target).is_none());
    assert!(!engine.crew_members(owner).contains(&target));
    assert_eq!(sheriff.energy, 50_000);
    assert!(sheriff.alive);
    assert_eq!(sheriff.action.name, "Walk");
    assert_eq!(sheriff.owner, OWNER_NONE);
    let stay_there = sheriff
        .effects
        .iter()
        .find(|effect| effect.name == "StayThere")
        .expect("the callback tail installs the StayThere effect");
    assert_eq!(stay_there.priority, 1);
    assert_eq!(stay_there.interval, 35);
}

fn gold_rush_real_anvil_forges_a_wire_roll_from_its_metal_contents(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let mut forge_action = ActionState::new("Forge");
    forge_action.time = 150;
    let anvil = engine
        .spawn_object(
            SpawnConfig::new("ANVL")
                .with_action(forge_action)
                // Loaded objects restore Action/Time without replaying the
                // Forge StartCall before this fixture can add its METL.
                .with_loaded(true)
                .with_local_vars(std::collections::HashMap::from([(
                    "product".to_owned(),
                    Value::C4Id("WIRR".to_owned()),
                )])),
        )
        .expect("the real Western anvil spawns");
    let metal = engine
        .spawn_object(SpawnConfig::new("METL").with_container(anvil))
        .expect("one real METL component enters the anvil");
    let old_wire_rolls = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "WIRR")
        .map(|object| object.id)
        .collect::<Vec<_>>();

    // ANVL::Forging calls ComposeContents(WIRR), whose DefCore requires one
    // METL. C++ removes that first matching content, creates WIRR as anvil
    // contents, and returns it; Forging immediately exits the product
    // (C4Object.cpp:3764-3806; Anvil.c4d/Script.c:179-188).
    let anvil_index = engine.find_object_index(anvil).expect("anvil index");
    assert_eq!(
        engine
            .call_object_function(anvil_index, "Forging", Vec::new())
            .expect("the shipped ANVL::Forging callback completes"),
        Value::Int(1)
    );

    assert!(
        engine
            .object_snapshot(metal)
            .is_none_or(|object| !object.status.is_active()),
        "ComposeContents consumes the anvil's METL component"
    );
    let wire_rolls = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| {
            object.definition_id == "WIRR"
                && object.status.is_active()
                && !old_wire_rolls.contains(&object.id)
        })
        .collect::<Vec<_>>();
    assert_eq!(wire_rolls.len(), 1, "the anvil creates exactly one WIRR");
    assert_eq!(
        wire_rolls[0].container, None,
        "ANVL::Forging exits the composed wire roll"
    );
}

#[test]
fn knights_bow_trajectory_runs_through_shipped_arc_cos_calls() {
    // KNIG::FireBowAt computes both candidate launch angles with ArcCos
    // (Knight.c4d/Script.c:1454-1455). These coordinates reach both calls
    // and then fail the shipped +/-120 aiming gate, before bow/equipment
    // callbacks can affect this registration probe.
    let mut engine = load_installed_scenario("Knights.c4f/Camp.c4s", 0);
    let knight = engine
        .spawn_object(
            SpawnConfig::new("KNIG")
                .with_position(Vector2::new(100, 100))
                .with_loaded(true),
        )
        .expect("shipped KNIG spawns");
    let index = engine.find_object_index(knight).expect("KNIG index");

    assert_eq!(
        engine
            .call_object_function(
                index,
                "FireBowAt",
                vec![Value::Int(150), Value::Int(150), Value::Bool(false)],
            )
            .expect("shipped Knight bow trajectory completes"),
        Value::Nil
    );
}

#[test]
fn knights_lance_rank_five_target_collision_matches_cpp() {
    // The shipped attached lance reads its rider's C4ObjectInfo rank while
    // aiming, then its phase callback punches prey at vertex one
    // (Lance.c4d/Attached.c4d/Script.c:24-55). Camp loads the unmodified
    // Objects+Knights definition stack without auto-equipping the crew.
    let mut engine = load_installed_scenario("Knights.c4f/Camp.c4s", 0);
    let crew = ["Rank Five Rider", "Rank Five Victim"]
        .into_iter()
        .map(|name| clonk_engine::player_file::CrewInfo {
            id: "KNIG".to_owned(),
            name: name.to_owned(),
            rank: 5,
            rank_name: "Lieutenant Colonel".to_owned(),
            ..Default::default()
        })
        .collect();
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Lance parity".to_owned(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: Some(1),
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew,
            control_style: false,
            auto_context_menu: false,
            startup_player_count: 1,
        })
        .expect("rank-five Knights player joins")
        .initialized()
        .expect("team one initializes immediately")
        .number;

    let mut knights = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| {
            object.owner == owner && object.crew_member && object.definition_id == "KNIG"
        })
        .map(|object| object.id)
        .collect::<Vec<_>>();
    knights.sort_unstable_by_key(|id| id.as_u64());
    assert_eq!(knights.len(), 2, "Camp recruits both player-file knights");
    for knight in &knights {
        assert_eq!(
            engine.crew_object_info(*knight).map(|info| info.rank),
            Some(5),
            "each live KNIG carries its rank-five C4ObjectInfo"
        );
    }
    let rider = knights[0];
    let victim = knights[1];
    engine
        .apply_object_update(
            rider,
            ObjectUpdate::new().with_position(Vector2::new(9_000, 9_000)),
        )
        .expect("move the rider away from the collision point");
    engine
        .apply_object_update(
            victim,
            ObjectUpdate::new()
                .with_position(Vector2::new(10_000, 9_973))
                .with_action("Walk"),
        )
        .expect("place the victim at attached-lance vertex one");
    assert_eq!(
        engine
            .object_snapshot(victim)
            .expect("rank-five victim exists")
            .energy,
        55_000,
        "fair-crew promotion raises the real KNIG energy before the hit"
    );

    let mut lance_action = ActionState::new("Lance");
    lance_action.target = Some(rider);
    let lance = engine
        .spawn_object(
            SpawnConfig::new("LNCA")
                .with_position(Vector2::new(10_000, 10_000))
                .with_owner(owner)
                .with_action(lance_action)
                .with_local_vars(std::collections::HashMap::from([
                    ("high_target".to_owned(), Value::Int(0)),
                    ("last_x".to_owned(), Value::Int(9_969)),
                ]))
                .with_loaded(true),
        )
        .expect("the real attached lance spawns");

    // Lancing computes speed_x=31, draws Random(16), reads GetRank(rider)
    // as 5, and uses divisor BoundBy((5-3)/2,1,6)=1. The resulting angle
    // always clamps to SetRDir(12) at this speed.
    let mut expected_rng = engine.debug_rng_clone();
    expected_rng.random(16);
    let lance_index = engine.find_object_index(lance).expect("lance index");
    assert_eq!(
        engine
            .call_object_function(lance_index, "Lancing", Vec::new())
            .expect("the shipped Lancing callback completes"),
        Value::Int(1)
    );
    let aimed_lance = engine.object_snapshot(lance).expect("aimed lance exists");
    assert_eq!(
        aimed_lance.rotation_velocity,
        Some(math::itofix_prec(12, 10)),
        "rank-five Lancing applies the C++ angular velocity"
    );
    assert_eq!(aimed_lance.local_vars.get("speed_x"), Some(&Value::Int(31)));
    assert_eq!(engine.debug_rng_clone(), expected_rng);

    // Classic script evaluates both operands of this legacy `||`, so the
    // non-Ride victim still consumes Random(3). KNIG's QueryCatchBlow and
    // inherited CLNK CatchBlow consume the next two draws; an unshielded
    // 15% Punch subtracts 15_000 energy and tumbles.
    expected_rng.random(3);
    expected_rng.random(50_000);
    expected_rng.random(5);
    assert_eq!(
        engine
            .call_object_function(lance_index, "Targeting", Vec::new())
            .expect("the shipped Targeting callback completes"),
        Value::Int(1)
    );
    let hit_victim = engine
        .object_snapshot(victim)
        .expect("punched victim exists");
    assert_eq!(hit_victim.energy, 40_000);
    assert_eq!(hit_victim.action.name, "Tumble");
    assert_eq!(
        engine
            .object_snapshot(lance)
            .expect("lance survives its hit")
            .local_vars
            .get("speed_x"),
        Some(&Value::Nil)
    );
    assert_eq!(engine.debug_rng_clone(), expected_rng);

    // C4Object::GrabInfo moves the same live Info pointer. Verify the
    // nonzero rank, not merely the fresh-rank-zero case, is visible on the
    // recipient immediately and remains there after the callback folds.
    let rank_probe = Definition::from_script(
        "RKPR",
        "Rank transfer probe",
        "#strict 2\nfunc Take(obj) { return [GrabObjectInfo(obj), GetRank(), GetRank(obj)]; }\nfunc Read() { return GetRank(); }",
    )
    .expect("rank transfer probe compiles");
    engine
        .register_definition(rank_probe)
        .expect("rank transfer probe registers");
    let rank_probe = engine
        .spawn_object(SpawnConfig::new("RKPR").with_owner(owner))
        .expect("rank transfer probe spawns");
    let rank_probe_index = engine
        .find_object_index(rank_probe)
        .expect("rank transfer probe index");
    assert_eq!(
        engine
            .call_object_function(
                rank_probe_index,
                "Take",
                vec![Value::Object(victim.as_u64())],
            )
            .expect("rank-five info transfer completes"),
        Value::Array(vec![Value::Bool(true), Value::Int(5), Value::Nil])
    );
    assert_eq!(
        engine
            .call_object_function(rank_probe_index, "Read", Vec::new())
            .expect("transferred rank remains linked"),
        Value::Int(5)
    );
}
