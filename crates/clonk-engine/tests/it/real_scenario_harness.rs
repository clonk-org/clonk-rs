#![allow(dead_code)]

use crate::object_visibility::{
    shipped_invisibility_recast_carries_remaining_time_into_reset_timer,
    shipped_invisibility_spell_hides_and_restores_its_mage,
};
use crate::support::real_scenario::{
    join_local_player, join_local_player_on_team, load_installed_scenario, load_tutorial,
    prepare_installed_scenario, PreparedInstalledScenario,
};
use crate::support::EngineTestExt;
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
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "SCVP",
            "Scenario value probe",
            r#"#strict
        public func Read(string entry, string section, int index)
        {
            return GetScenarioVal(entry, section, index);
        }
        "#,
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("SCVP"));
    let probe_index = engine.test_object_index(probe);
    let mut read = |entry: &str, section: &str, index: i32| {
        engine.call_test_object_function(
            probe_index,
            "Read",
            vec![
                Value::String(entry.to_string().into()),
                Value::String(section.to_string().into()),
                Value::Int(index),
            ],
        )
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
    let caster = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let caster_state = engine.test_object_snapshot(caster);
    let caster_vertex = crate::support::TestValueExt::test_value(caster_state.vertices.first());
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
    let spell = engine.spawn_test_object(
        SpawnConfig::new("LGT2")
            .with_owner(owner)
            .with_layer(caster),
    );
    let rng_before = engine.debug_rng_clone().count;
    let spell_index = engine.test_object_index(spell);

    assert_eq!(
        engine.call_test_object_function(
            spell_index,
            "Activate",
            vec![Value::Object(caster.as_u64()), Value::Nil, Value::Nil],
        ),
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
    engine.register_test_definition(crate::support::TestValueExt::test_value(
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
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("TCFG"));
    let probe_index = engine.test_object_index(probe);
    assert_eq!(
        engine.call_test_object_function(probe_index, "Read", Vec::new()),
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
    let race = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "RACE")
            .map(|object| object.id),
    );
    let race_index = engine.test_object_index(race);
    assert_eq!(
        engine.call_test_object_function(
            race_index,
            "InitializePlayer",
            vec![
                Value::Int(player),
                Value::Int(0),
                Value::Int(0),
                Value::Nil,
                Value::Int(1),
            ],
        ),
        Value::Bool(true)
    );

    let scoreboard = engine.snapshot().hud.scoreboard;
    let team_row = crate::support::TestValueExt::test_value(
        (1..scoreboard.row_count())
            .find(|row| scoreboard.cell(*row, 0).map(|cell| cell.value()) == Some(1)),
    );
    let caption = crate::support::TestValueExt::test_value(
        scoreboard.cell(team_row, 0).and_then(|cell| cell.text()),
    );
    assert!(
        caption.contains(player_name),
        "autogenerated Race team caption should name its member: {caption:?}"
    );
}

#[test]
fn sky_race_death_announces_before_the_shipped_relaunch_path() {
    let mut engine = load_installed_scenario("Races.c4f/Skyrace.c4s", 0);
    let owner = join_local_player(&mut engine, "Sky Race death parity");
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
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
    engine.register_test_definition(crate::support::TestValueExt::test_value(
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
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("DTHP"));
    let probe_index = engine.test_object_index(probe);
    assert_eq!(
        engine.call_test_object_function(
            probe_index,
            "Trigger",
            vec![Value::Object(clonk.as_u64())],
        ),
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
    let original = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // CLNK::Death reaches this real scenario callback after a BottomOpen
    // fall. Invoke that shipped callback synchronously so the assertions
    // observe its exact SetPosition before another physics frame. C++ adds
    // the replacement to the live C4Player::Crew inside MakeCrewMember;
    // SelectCrew and JoinPlayer(GetCrew(owner)) immediately see it
    // (C4Player.cpp:1194-1209; Skyrace.c4s/Script.c:75-91).
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "RLHP",
            "Relaunch probe",
            r#"#strict
        public func Trigger(int owner)
        {
            return GameCallEx("RelaunchPlayer", owner);
        }
        "#,
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("RLHP"));
    let probe_index = engine.test_object_index(probe);
    assert_eq!(
        engine.call_test_object_function(probe_index, "Trigger", vec![Value::Int(owner)]),
        Value::Int(1)
    );
    let replacement = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.id != original && object.definition_id == "CLNK" && object.status.is_active()
            })
            .map(|object| object.id),
    );

    let replacement_snapshot = engine.test_object_snapshot(replacement);
    let start_y =
        crate::support::TestValueExt::test_value(engine.landscape()).estimated_height() / 2 - 15;
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
    let loser = crate::support::TestValueExt::test_value(engine.join_player(JoinPlayerConfig {
        player_info_id: 2,
        color_dw: 0x00_00_ff,
        pref_color: 1,
        pref_position: 1,
        startup_player_count: 2,
        ..crate::support::join_player_config("Sky Race loser")
    }))
    .number();
    let winner_clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(winner));
    let race = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "RACE")
            .map(|object| object.id),
    );
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
    let landscape_width =
        crate::support::TestValueExt::test_value(engine.landscape()).width() as i32;
    let y = engine.test_object_snapshot(winner_clonk).position.y;
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            winner_clonk,
            ObjectUpdate::new()
                .with_position(Vector2::new(landscape_width - 99, y))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk"),
        ),
    );

    let race_index = engine.test_object_index(race);
    assert_eq!(
        engine.call_test_object_function(race_index, "GetWayPercent", vec![Value::Int(winner)]),
        Value::Int(100)
    );
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    let after_finish = engine.snapshot();
    let winner_info_id = crate::support::TestValueExt::test_value(
        after_finish
            .players
            .iter()
            .find(|player| player.id == winner)
            .map(|player| player.player_info_id),
    );
    let scoreboard = &after_finish.hud.scoreboard;
    let race_column =
        crate::support::TestValueExt::test_value((0..scoreboard.column_count()).find(|column| {
            scoreboard.cell(0, *column).map(|cell| cell.value())
                == Some(i32::from_le_bytes(*b"RACE"))
        }));
    let winner_row = crate::support::TestValueExt::test_value(
        (1..scoreboard.row_count())
            .find(|row| scoreboard.cell(*row, 0).map(|cell| cell.value()) == Some(winner_info_id)),
    );
    assert_eq!(
        scoreboard
            .cell(winner_row, race_column)
            .map(|cell| (cell.text(), cell.value())),
        Some((Some("100%"), 100)),
        "UpdateScoreboard writes the C++ finish percentage before sorting"
    );
    let winner_state = crate::support::TestValueExt::test_value(
        after_finish
            .players
            .iter()
            .find(|player| player.id == winner),
    );
    let loser_state = crate::support::TestValueExt::test_value(
        after_finish
            .players
            .iter()
            .find(|player| player.id == loser),
    );
    let loser_info_id = loser_state.player_info_id;
    assert_eq!(winner_state.status, PlayerStatus::Active);
    assert_eq!(loser_state.status, PlayerStatus::Eliminated);
    assert_eq!(engine.eliminated_owners(), vec![loser]);

    for _ in 0..300 {
        if engine.snapshot().game_over {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    assert_eq!(engine.test_object_snapshot(mage).definition_id, "MAGE");

    let monster =
        crate::support::TestValueExt::test_value(engine.test_object_snapshot(mage).container);
    assert_eq!(engine.test_object_snapshot(monster).definition_id, "MONS");

    // Monster Rescue's shipped JoinPlayer gives the Magus 30 energy and then
    // caps its temporary Magic physical at the matching 30000 before putting
    // it into MONS (Script.c:55-70). This is already enough for its sole MBRG
    // spell (Scenario.txt:18-20; MBRG DefCore Value=10).
    let energy_before = engine.test_object_snapshot(mage).magic_energy;
    assert_eq!(energy_before, 30_000);
    let mage_index = engine.test_object_index(mage);
    assert_eq!(
        engine.call_test_object_function(
            mage_index,
            "CheckMagicRequirements",
            vec![Value::C4Id("MBRG".to_string()), Value::Bool(true)],
        ),
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
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    let monster_menu = crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
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
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        mage_submenu_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    let magic_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.command.contains("ContextMagic")),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        magic_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    let (_, menu) = crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner));
    assert_eq!(
        menu.items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["MBRG"],
        "OpenSpellMenu enumerates Monster Rescue's real player magic list"
    );
    let spell_command = menu.items[0].command.clone();

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    assert_eq!(
        engine.test_object_snapshot(mage).action.name,
        "Magic",
        "the real menu command `{spell_command}` starts DoMagic; menu now {:?}, locals {:?}",
        engine
            .cursor_object_menu(owner)
            .map(|(_, menu)| menu.clone()),
        engine.test_object_snapshot(mage).local_vars
    );

    // Magic's Delay=1 PhaseCall invokes CheckMagic after each phase advance;
    // phase five creates MBRG. Its shipped Activate creates FBRG; FBRG's own
    // Initialize immediately expands into four persistent FBRS segments and
    // removes both temporary bridge/spell objects.
    for _ in 0..8 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
            "fishskin_picks_its_revaluation_target_by_magic_physical",
            alchemy_fishskin_picks_its_revaluation_target_by_magic_physical,
        ),
        (
            "firelump_collects_its_same_call_fireball_into_the_mage",
            alchemy_firelump_collects_its_same_call_fireball_into_the_mage,
        ),
        (
            "learned_warp_builds_a_connected_hole_pair_and_consumes_its_gold",
            alchemy_learned_warp_builds_a_connected_hole_pair_and_consumes_its_gold,
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
            "guarding_zaps_turns_carried_gold_into_a_nest_instead_of_zaps",
            alchemy_guarding_zaps_turns_carried_gold_into_a_nest_instead_of_zaps,
        ),
        (
            "learned_heal_cast_sustains_magic_and_restores_the_casters_energy",
            alchemy_learned_heal_cast_sustains_magic_and_restores_the_casters_energy,
        ),
        (
            "firefist_flame_consumes_inflammable_landscape",
            alchemy_firefist_flame_consumes_inflammable_landscape,
        ),
        (
            "learned_small_force_field_binds_its_field_to_the_caster",
            alchemy_learned_small_force_field_binds_its_field_to_the_caster,
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
            "learned_extinguish_puts_out_a_nearby_fire_and_spends_its_ashes",
            alchemy_learned_extinguish_puts_out_a_nearby_fire_and_spends_its_ashes,
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
            "raise_undead_selects_a_dead_clonk_and_animates_it",
            alchemy_raise_undead_selects_a_dead_clonk_and_animates_it,
        ),
        (
            "dragon_call_commands_a_grown_riderless_dragon_to_follow",
            alchemy_dragon_call_commands_a_grown_riderless_dragon_to_follow,
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
        (
            "learned_firebreath_aims_and_attaches_its_breath_to_the_caster",
            alchemy_learned_firebreath_aims_and_attaches_its_breath_to_the_caster,
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
            "curse_family_selects_a_foreign_target_through_the_shipped_selector",
            alchemy_curse_family_selects_a_foreign_target_through_the_shipped_selector,
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
        (
            "walk_on_liquid_installs_its_fixed_duration_on_the_real_caster",
            alchemy_walk_on_liquid_installs_its_fixed_duration_on_the_real_caster,
        ),
        (
            "learned_fireball_aims_steers_and_explodes_through_player_controls",
            alchemy_learned_fireball_aims_steers_and_explodes_through_player_controls,
        ),
        (
            "learned_eternal_flame_scatters_its_shipped_flame_cast",
            alchemy_learned_eternal_flame_scatters_its_shipped_flame_cast,
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

fn attached_alchemy_bag(engine: &Engine, mage: ObjectId) -> ObjectId {
    crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.action.name == "Belongs"
                    && object.action.target == Some(mage)
            })
            .map(|object| object.id),
    )
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
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    assert_eq!(engine.test_object_snapshot(mage).definition_id, "MCLK");

    // InitializePlayer places one seeded alchemy bag beside AHUT. Its Activate
    // callback delegates the ingredient move to the already attached MCLK
    // bag's Transfer callback (Bag.c4d/Script.c:5-14,148-160). Invoke that
    // shipped delegation target directly so this test isolates spell-system
    // parity from loose-item collection/activation.
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_" && object.components.get("IROC") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    let attached_bag_index = engine.test_object_index(attached_bag);
    engine.call_test_object_function(
        attached_bag_index,
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC")),
        Some(3),
        "the shipped loose bag supplies the rock ingredient used by MGUP"
    );
    assert_eq!(
        engine
            .object_snapshot(seeded_bag)
            .and_then(|bag| bag.components.get("IROC")),
        Some(0),
        "TransferAlchem moves rather than duplicates the shipped ingredients"
    );

    // With the default player ExtraData, iCombo and all quick-spell slots are
    // zero. Therefore Special is only the empty quick-spell route; the full
    // spell list is opened through ContextMagic (MagiClonk.c4d/Script.c:88-111,
    // 190-200), which C4ObjectMenu exposes as a selectable context action
    // (C4ObjectMenu.cpp:670-682).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_SPECIAL, 0));
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

    let raise_gravity_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MGUP"),
    );
    for _ in 0..raise_gravity_index {
        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
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
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    assert_eq!(engine.test_object_snapshot(mage).action.name, "Magic");
    for _ in 0..8 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.physics().gravity,
        gravity_before + 20,
        "MGUP Activate raises gravity by the shipped 20-point increment"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC")),
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
    let lower_gravity_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MGDW"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        lower_gravity_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..8 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
            .and_then(|bag| bag.components.get("IROC")),
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
    let airblast_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "ABLA"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        airblast_index as i32,
    ));
    let (_, airblast_menu) =
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner));
    assert_eq!(
        airblast_menu
            .items
            .get(airblast_menu.selection as usize)
            .map(|item| item.item_id.as_str()),
        Some("ABLA"),
        "menu selection targets ABLA before casting"
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    let aimer = (0..12)
        .find_map(|_| {
            // Pin a stale command immediately before each object-execution
            // pass. On the activation pass AIMR::Create must clear the two
            // C++ latches before Players.Execute observes them.
            {
                let control = &mut crate::support::TestValueExt::test_value(engine
                    .player_mut(owner))
                    .control;
                control.last_com = i32::from(COM_RIGHT);
                control.last_com_delay = 17;
                control.last_com_down_double = 4;
            }
            crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let player = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == owner),
    );
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
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_UP, 0));
    assert_eq!(
        engine.test_object_snapshot(aimer).local_vars.get("iAngle"),
        Some(&Value::Int(70)),
        "left-facing ABLA starts at 90 degrees and Up steps toward zero"
    );
    assert_eq!(
        engine.test_object_snapshot(mage).action.name,
        "AimMagic",
        "AimingAngle switches the mage into the shipped aiming action"
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
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
    let player = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .players
            .into_iter()
            .find(|player| player.id == owner),
    );
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

fn alchemy_learned_warp_builds_a_connected_hole_pair_and_consumes_its_gold(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy warp parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // MGWP costs IGOL=3, which is exactly what Alchemy seeds, so the shipped
    // bag alone pays for it once ALC_::Transfer moves it across
    // (Alchemy.c4s/Script.c:21-37; Warp.c4d/DefCore.txt;
    // Bag.c4d/Script.c:148-160).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("ISPH") == Some(1)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.action.name == "Belongs"
                    && object.action.target == Some(mage)
            })
            .map(|object| object.id),
    );
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );
    assert_eq!(
        engine.call_test_object_function(
            engine.test_object_index(mage),
            "CheckMagicRequirements",
            vec![Value::C4Id("MGWP".into()), Value::Bool(true)],
        ),
        Value::Int(1)
    );

    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "MGWP"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let warp_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MGWP"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        warp_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..20 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }

    // MGWP is not an aimer: Activate warps immediately, creating both holes
    // and connecting them before removing itself
    // (Warp.c4d/Script.c:5-16,18-46).
    let holes: Vec<_> = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "WARP" && object.status.is_active())
        .map(|object| (object.id, object.position))
        .collect();
    assert_eq!(
        holes.len(),
        2,
        "a warp cast opens exactly one hole pair: {holes:?}"
    );
    assert_ne!(
        holes[0].1, holes[1].1,
        "the entry and exit holes are placed apart"
    );
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "MGWP" && object.status.is_active()),
        "MGWP removes itself once the pair is connected"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IGOL")),
        Some(0),
        "a successful MGWP cast consumes all three gold"
    );
}

fn alchemy_warp_to_base_cast_builds_the_real_portal_pair_and_transfers_the_mage(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy warp parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // ExecBase runs on Tick10 and claims AHUT for this player once its FLAG
    // has settled. MWP2 deliberately fails before that claim; wait for the
    // same C++ base lifecycle rather than manufacturing a shortcut.
    let home = crate::support::TestValueExt::test_value((0..20).find_map(|_| {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "AHUT" && object.base == owner)
            .map(|object| object.id)
    }));
    for _ in 0..160 {
        if engine
            .object_snapshot(mage)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("IMUS") == Some(4)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    let harvested_gold = engine.spawn_test_object(
        SpawnConfig::new("ALC_").with_ordered_components(vec![("IGOL".to_owned(), 1)]),
    );
    let attached_bag_index = engine.test_object_index(attached_bag);
    for source in [seeded_bag, harvested_gold] {
        engine.call_test_object_function(
            attached_bag_index,
            "Transfer",
            vec![Value::Object(source.as_u64())],
        );
    }
    let bag = engine.test_object_snapshot(attached_bag);
    assert_eq!(bag.components.get("IMUS"), Some(4));
    assert_eq!(bag.components.get("IGOL"), Some(4));
    let mage_index = engine.test_object_index(mage);
    assert!(
        engine
            .call_test_object_function(
                mage_index,
                "CheckMagicRequirements",
                vec![Value::C4Id("MWP2".to_owned()), Value::Bool(true)],
            )
            .as_bool(),
        "the attached bag satisfies MWP2 before the player casts"
    );

    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let warp_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MWP2"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        warp_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..8 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }

    let bag_after_cast = engine.test_object_snapshot(attached_bag);
    assert_eq!(
        (
            bag_after_cast.components.get("IMUS"),
            bag_after_cast.components.get("IGOL"),
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
    let start_portal = crate::support::TestValueExt::test_value(
        portals.iter().find(|portal| portal.action.target.is_some()),
    );

    // Fast-forward the source aperture's purely visual 7×64-tick growth and
    // put the mage inside it. This keeps the suite fast while still exercising
    // WARP::FxWarpUSpellTimer,
    // WarpUSpellData, vertex removal/restoration, and TransferWarpObject's
    // entrance path rather than replacing them with a direct Enter call here.
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        start_portal.id,
        ObjectUpdate::new().with_construction(FULL_CON),
    ));
    let start_portal_index = engine.test_object_index(start_portal.id);
    engine.call_test_object_function(start_portal_index, "Shrink", vec![]);
    let original_vertices = engine.test_object_snapshot(mage).vertices;
    assert!(
        !original_vertices.is_empty(),
        "the real MCLK shape supplies vertices for WarpUSpellData to remove"
    );
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
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
        ),
    );

    let warp_data_observed = (0..30).any(|_| {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        let mage = engine.test_object_snapshot(mage);
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
    let live_warp_effect =
        crate::support::TestValueExt::test_value(engine.object_snapshot(mage).and_then(|mage| {
            mage.effects
                .into_iter()
                .find(|effect| effect.name == "WarpUSpellData" && effect.priority != 0)
        }));
    assert_eq!(
        live_warp_effect.vars.len(),
        16,
        "WarpUSpellData stores power, count, and seven X/Y pairs before removing the shape: {live_warp_effect:?}"
    );

    // C4Shape::CompileFunc persists the fixed vertex arrays independently
    // of VtxNum. A save while WARP has reduced VtxNum to zero must therefore
    // retain the dormant CNAT/friction slots that AddVertex restores later.
    let saved_json =
        crate::support::TestValueExt::test_value(engine.capture_state().to_json_string());
    let saved = crate::support::TestValueExt::test_value(clonk_engine::EngineState::from_json_str(
        &saved_json,
    ));
    crate::support::TestValueExt::test_value(engine.restore_state(&saved));
    let restored_warping_mage = engine.test_object_snapshot(mage);
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let warped_mage = engine.test_object_snapshot(mage);
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
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert!(engine
        .test_object_snapshot(mage)
        .effects
        .iter()
        .all(|effect| effect.name != "WarpUSpellData"));
}

fn alchemy_learned_extinguish_puts_out_a_nearby_fire_and_spends_its_ashes(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy extinguish parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // EXTG costs IASH=3, which is exactly what the shipped bag carries, so
    // ALC_::Transfer alone pays for it (Alchemy.c4s/Script.c:21-37;
    // Extinguish.c4d/DefCore.txt; Bag.c4d/Script.c:148-160).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("INEC") == Some(1)
                    && object.components.get("IASH") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.action.name == "Belongs"
                    && object.action.target == Some(mage)
            })
            .map(|object| object.id),
    );
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );

    // Something has to be alight for the spell to do anything: with nothing
    // burning it reports $NoExtinguish$ and removes itself
    // (Extinguish.c4d/Script.c:42). A shipped FLAM sets itself alight on
    // completion, and sits inside the caster-height search radius.
    let flame =
        engine.spawn_test_object(SpawnConfig::new("FLAM").with_position(Vector2::new(510, 200)));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            flame,
            ObjectUpdate::new()
                .with_position(Vector2::new(510, 200))
                .with_velocity(Vector2::ZERO),
        ),
    );
    engine.call_test_object_function(engine.test_object_index(flame), "Completion", Vec::new());
    for _ in 0..5 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        engine.test_object_snapshot(flame).on_fire,
        "the shipped FLAM lights itself, which is what EXTG is asked to undo"
    );

    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "EXTG"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let extinguish_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "EXTG"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        extinguish_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..20 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }

    // The spell walks outward from the caster and extinguishes what it finds
    // inside three caster-heights (Extinguish.c4d/Script.c:28-41).
    assert!(
        engine
            .object_snapshot(flame)
            .is_none_or(|flame| !flame.status.is_active() || !flame.on_fire),
        "EXTG puts the nearby flame out"
    );

    // EXTG is the odd one out among these spells: MFFS, MGWP and MDBT all
    // delete themselves on the way out, and EXTG does too -- but only on the
    // failure path, where nothing was burning. A cast that actually
    // extinguished something falls through to `return(true)` with no
    // RemoveObject at all, so the spell object survives its own success
    // (Extinguish.c4d/Script.c:44-47).
    assert_eq!(
        engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.definition_id == "EXTG" && object.status.is_active())
            .count(),
        1,
        "a successful EXTG survives its own cast"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IASH")),
        Some(0),
        "a successful EXTG cast consumes all three ashes"
    );
}

fn alchemy_reincarnation_spell_revives_its_mage_during_assign_death(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy reincarnation parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // Alchemy seeds INEC=1 and IASH=3, while XCRS consumes INEC=2 and
    // IASH=4. Transfer the real starter bag plus one harvested unit of each
    // through ALC_::Transfer (Alchemy.c4s/Script.c:21-37;
    // Reincarnation.c4d/DefCore.txt:7; Bag.c4d/Script.c:148-160).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("INEC") == Some(1)
                    && object.components.get("IASH") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    let extra_ingredients = engine.spawn_test_object(
        SpawnConfig::new("ALC_")
            .with_ordered_components(vec![("INEC".to_owned(), 1), ("IASH".to_owned(), 1)]),
    );
    let attached_bag_index = engine.test_object_index(attached_bag);
    for source in [seeded_bag, extra_ingredients] {
        engine.call_test_object_function(
            attached_bag_index,
            "Transfer",
            vec![Value::Object(source.as_u64())],
        );
    }

    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let reincarnation_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "XCRS"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        reincarnation_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..8 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let protected = engine.test_object_snapshot(mage);
    assert_eq!(protected.energy, 45_000, "XCRS sacrifices ten energy");
    assert!(protected
        .effects
        .iter()
        .any(|effect| effect.name == "ReincarnationPSpell"));
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("INEC")),
        Some(0),
        "a successful XCRS cast consumes its two-nectar recipe"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IASH")),
        Some(0),
        "a successful XCRS cast consumes its four-ash recipe"
    );

    // C4Object::AssignDeath sets Alive=false, clears effects with
    // C4FxCall_RemoveDeath, and aborts ordinary death if an effect revives
    // the object (C4Object.cpp:1162-1180). XCRS's Stop callback restores
    // Alive, denies removal, and installs IntReincDelay
    // (Reincarnation.c4d/Script.c:34-58).
    let mage_index = engine.test_object_index(mage);
    crate::support::TestValueExt::test_value(engine.change_object_energy(mage_index, -100, 0, -1));
    let reincarnating = engine.test_object_snapshot(mage);
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

fn alchemy_dragon_call_commands_a_grown_riderless_dragon_to_follow(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy dragon call parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let mage_position = engine.test_object_snapshot(mage).position;

    // Alchemy ships no dragon, so the subject of the spell has to be placed.
    // DGCL only calls one that is within 750, riderless and fully grown
    // (DragonCall.c4d/Script.c:16-22).
    let dragon = engine.spawn_test_object(
        clonk_engine::SpawnConfig::new("DRGN")
            .with_position(Vector2::new(mage_position.x + 120, mage_position.y))
            .with_owner(owner),
    );
    for _ in 0..5 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let spell = engine.spawn_test_object(
        clonk_engine::SpawnConfig::new("DGCL")
            .with_position(mage_position)
            .with_owner(owner),
    );
    engine.call_test_object_function(
        engine.test_object_index(spell),
        "Activate",
        vec![Value::Object(mage.as_u64())],
    );

    // The call is a command, not a teleport: DGCL resets the dragon's control
    // and pushes Follow onto it, aimed at the caster
    // (DragonCall.c4d/Script.c:31-35).
    assert_eq!(
        engine
            .test_object_snapshot(dragon)
            .command_stack
            .command_names(),
        vec!["Follow".to_string()],
        "DGCL commands the dragon to follow its caller"
    );

    // Like EXTG, and unlike the spells that build something, a successful DGCL
    // does not delete itself -- the success path is a bare `return(true)`
    // (DragonCall.c4d/Script.c:48).
    assert_eq!(
        engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| object.definition_id == "DGCL" && object.status.is_active())
            .count(),
        1,
        "a successful DGCL survives its own cast"
    );
}

fn alchemy_learned_group_heal_cast_sustains_magic_and_heals_nearby_crew(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy group-heal parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let patient = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "CLNK" && object.owner == owner && object.status.is_active()
            })
            .map(|object| object.id),
    );
    for (object, position) in [
        (mage, Vector2::new(500, 200)),
        (patient, Vector2::new(530, 200)),
    ] {
        crate::support::TestValueExt::test_value(
            engine.apply_object_update(
                object,
                ObjectUpdate::new()
                    .with_position(position)
                    .with_velocity(Vector2::ZERO)
                    .with_action("Walk")
                    .clear_container(),
            ),
        );
    }
    crate::support::TestValueExt::test_value(engine.change_object_energy(
        engine.test_object_index(patient),
        -20,
        0,
        -1,
    ));
    let energy_before = engine.test_object_snapshot(patient).energy;
    assert_eq!(energy_before, 35_000);

    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("IMUS") == Some(4)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );

    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "GGHG"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let heal_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "GGHG"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        heal_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..50 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }

    let caster = engine.test_object_snapshot(mage);
    assert_eq!(caster.action.name, "Magic");
    assert!(caster
        .effects
        .iter()
        .any(|effect| effect.name == "GroupHealPSpell"));
    let healed = engine.test_object_snapshot(patient);
    assert!(
        healed.energy > energy_before,
        "GGHG repeatedly heals friendly crew within 80 pixels: caster={caster:?}; patient={healed:?}"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IMUS")),
        Some(1),
        "a successful GGHG cast consumes three mushrooms"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IGOL")),
        Some(2),
        "a successful GGHG cast consumes one gold"
    );
}

fn alchemy_learned_heal_cast_sustains_magic_and_restores_the_casters_energy(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy heal parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );
    // MGHL's Activate returns without adding its effect when the target is
    // already at full energy, so the caster has to be hurt for the cast to do
    // anything at all (Heal.c4d/Script.c:16).
    let full_energy = engine.test_object_snapshot(mage).energy;
    crate::support::TestValueExt::test_value(engine.change_object_energy(
        engine.test_object_index(mage),
        -20,
        0,
        -1,
    ));
    let energy_before = engine.test_object_snapshot(mage).energy;
    assert_eq!(energy_before, full_energy - 20_000);

    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("IMUS") == Some(4)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );

    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "MGHL"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let heal_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MGHL"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        heal_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..50 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }

    let caster = engine.test_object_snapshot(mage);
    assert_eq!(caster.action.name, "Magic");
    // The effect is added to the caster and remembers its target separately,
    // because the caster is not always the patient -- the wizard tower casts
    // it on someone else (Heal.c4d/Script.c:18,22-26).
    assert!(
        caster
            .effects
            .iter()
            .any(|effect| effect.name == "HealPSpell"),
        "MGHL adds HealPSpell to the caster: {caster:?}"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IMUS")),
        Some(2),
        "a successful MGHL cast consumes two mushrooms (Heal.c4d/DefCore.txt:7)"
    );
    // Healing lands in whole `DoEnergy(+2)` steps on the roughly one timer tick
    // in three that `!Random(3)` selects, so the amount restored so far is a
    // positive multiple of two energy points and nothing finer
    // (Heal.c4d/Script.c:43-45). Pinning the step rather than the total keeps
    // this on what C++ guarantees: the count of steps is drawn from the
    // synchronized stream, the size of one is not.
    let restored = caster.energy - energy_before;
    assert!(
        restored > 0 && restored % 2_000 == 0,
        "MGHL restores whole DoEnergy(+2) steps: before={energy_before}; after={}",
        caster.energy
    );

    // Left running, the effect stops itself the moment the target is full
    // rather than overshooting the physical maximum, and takes its own slot
    // with it (Heal.c4d/Script.c:50).
    for _ in 0..400 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        if !engine
            .test_object_snapshot(mage)
            .effects
            .iter()
            .any(|effect| effect.name == "HealPSpell")
        {
            break;
        }
    }
    let finished = engine.test_object_snapshot(mage);
    assert_eq!(
        finished.energy, full_energy,
        "MGHL heals up to exactly the physical maximum: {finished:?}"
    );
    assert!(
        !finished
            .effects
            .iter()
            .any(|effect| effect.name == "HealPSpell"),
        "a fully healed target ends the spell instead of leaving it running: {finished:?}"
    );
}

fn alchemy_learned_eternal_flame_scatters_its_shipped_flame_cast(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy eternal flame parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // ETFL is the only spell in this set that spends out of both seeded bags:
    // ISPH=2 against a seeded ISPH=1, and IASH=2 against a seeded IASH=3. Both
    // go through ALC_::Transfer, plus one harvested sphere
    // (Alchemy.c4s/Script.c:21-37; EternalFlame.c4d/DefCore.txt;
    // Bag.c4d/Script.c:148-160).
    let sphere_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("ISPH") == Some(1)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let ash_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("INEC") == Some(1)
                    && object.components.get("IASH") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    let extra_sphere = engine.spawn_test_object(
        SpawnConfig::new("ALC_").with_ordered_components(vec![("ISPH".to_owned(), 1)]),
    );
    let attached_bag_index = engine.test_object_index(attached_bag);
    for source in [sphere_bag, ash_bag, extra_sphere] {
        engine.call_test_object_function(
            attached_bag_index,
            "Transfer",
            vec![Value::Object(source.as_u64())],
        );
    }

    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "ETFL"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let flame_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "ETFL"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        flame_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..20 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }

    // The cast is a single CastObjects of six MFLM, thrown to the side the
    // mage faces, after which the spell deletes itself
    // (EternalFlame.c4d/Script.c:13-18).
    let flames = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "MFLM" && object.status.is_active())
        .count();
    assert_eq!(flames, 6, "ETFL casts six flames");
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "ETFL" && object.status.is_active()),
        "ETFL removes itself once the flames are away"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("ISPH")),
        Some(0),
        "a successful ETFL cast consumes both spheres"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IASH")),
        Some(1),
        "a successful ETFL cast consumes two of the three ashes"
    );
}

fn alchemy_make_artefact_cast_opens_the_real_enchantment_menu(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy artefact parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // The shipped loose bag contains three IGOL; transfer it through the
    // attached ALC_ callback so the real NMGE rule can pay MART's one-gold
    // recipe (Alchemy.c4s/Script.c:18-30; Artefact.c4d/DefCore.txt:7-9).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_" && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );

    // MART enchants Contents(0, mage); use a real carried FLNT and teach the
    // scroll-discoverable spell, as the scenario's random scrolls do during
    // normal play (Alchemy.c4s/Script.c:5-16; C4Player.cpp:1052-1058).
    let carried = engine.spawn_test_object(
        SpawnConfig::new("FLNT")
            .with_owner(owner)
            .with_container(mage),
    );
    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "MART"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens the shipped spell menu"));
    let mart_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MART"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        mart_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..8 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }

    let (_, menu) = crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner));
    assert_eq!(menu.identification, Value::C4Id("MCMS".into()));
    assert!(
        !menu.items.is_empty(),
        "MagicMenu enumerates the installed spell classes"
    );
    let artefact_spell = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "MART" && object.status.is_active())
            .cloned(),
    );
    assert_eq!(
        artefact_spell.local_vars.get("iMagicAmount"),
        Some(&Value::Int(5)),
        "GetValue() returns MART's DefCore value for cancellation accounting"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IGOL")),
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

fn alchemy_learned_small_force_field_binds_its_field_to_the_caster(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy force field parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // MFFS is the one sphere-free spell in this set: IMUS=1 plus ILOA=1, and
    // Alchemy seeds no loam at all, so the mushrooms come from the shipped bag
    // and the loam is harvested (Alchemy.c4s/Script.c:21-37;
    // ForceFieldSmall.c4d/DefCore.txt; Bag.c4d/Script.c:148-160).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("IMUS") == Some(4)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );
    // CheckMagicRequirements answers with the number of casts the mage can
    // afford -- Min(mana casts, ingredient casts) -- not a flag, so the
    // requirement is met whenever it is positive
    // (MagiClonk.c4d/Script.c:264-283). Four transferred mushrooms cover four
    // casts at IMUS=1 each, so it is mana that decides the answer here; the
    // ingredient half still gates, which is why withholding the transfer
    // fails this assertion outright.
    assert!(
        matches!(
            engine.call_test_object_function(
                engine.test_object_index(mage),
                "CheckMagicRequirements",
                vec![Value::C4Id("MFFS".into()), Value::Bool(true)],
            ),
            Value::Int(casts) if casts >= 1
        ),
        "the transferred mushrooms pay for at least one MFFS cast"
    );

    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "MFFS"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let field_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MFFS"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        field_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..20 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }

    // Activate creates the field and immediately binds it with
    // ObjectSetAction(..., "Field", pCaster), which is what its timer reads
    // back to find whom to protect (ForceFieldSmall.c4d/Script.c:16-20).
    let field = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "FRCS" && object.status.is_active())
            .map(|object| object.id),
    );
    let field_state = engine.test_object_snapshot(field);
    assert_eq!(field_state.action.name, "Field");
    assert_eq!(
        field_state.action.target,
        Some(mage),
        "the field hangs off its caster: {field_state:?}"
    );
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "MFFS" && object.status.is_active()),
        "MFFS removes itself once the field exists"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IMUS")),
        Some(3),
        "a successful MFFS cast consumes one mushroom"
    );
}

fn alchemy_make_artefact_hit_mode_casts_the_selected_spell_after_throw(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy artefact activation parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // MART consumes its own IGOL recipe before Activate. LGCN then consumes
    // IMUS+IASH while SetMagic enchants Contents(0, mage), exactly as the
    // shipped ALC_/NMGE callbacks do (Alchemy.c4s/Script.c:18-30;
    // Artefact.c4d/Script.c:211-264).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_" && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );
    let carried = engine.spawn_test_object(
        SpawnConfig::new("ROCK")
            .with_owner(owner)
            .with_container(mage),
    );
    for spell in ["MART", "LGCN"] {
        crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, spell));
    }

    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens the shipped spell menu"));
    let mart_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MART"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        mart_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..8 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let combo_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "ALC_"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        combo_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    assert!(
        engine.cursor_object_menu(owner).is_none(),
        "the target choice finishes MART's configuration menus"
    );

    let enchanted = engine.test_object_snapshot(carried);
    let artefact = crate::support::TestValueExt::test_value(
        enchanted
            .effects
            .iter()
            .find(|effect| effect.name == "ArtefactNSpell"),
    );
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
            .and_then(|bag| bag.components.get("IMUS")),
        Some(3),
        "SetMagic consumes one of the shipped bag's four mushrooms"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IASH")),
        Some(2),
        "SetMagic consumes one of the shipped bag's three ashes"
    );

    // Mode0 arms while ROCK is in flight and casts when it next contacts the
    // landscape at low speed (Artefact.c4d/Script.c:488-509). Exercise the
    // normal CLNK Throw control and simulation callback, rather than calling
    // Mode0/CastSpell directly.
    for _ in 0..20 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    for _ in 0..240 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_" && object.components.get("IROC") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    assert_eq!(
        engine.call_test_object_function(
            engine.test_object_index(mage),
            "CheckMagicRequirements",
            vec![Value::C4Id("MGUP".into()), Value::Bool(true)],
        ),
        Value::Nil,
        "the empty attached bag cannot pay MGUP's one-IROC recipe"
    );

    // C++ exits a contained crew member on Down, automatically collects a
    // carryable inside MCLK's collection rectangle on Tick3, then turns two
    // Dig presses into ObjectComDigDouble. That activates the first carried
    // object, so ALC_::Activate transfers the loose bag into the hidden bag
    // (C4Object.cpp:3267-3272; C4GameObjects.cpp:140-197;
    // C4ObjectCom.cpp:531-540; Bag.c4d/Script.c:5-25,157-169).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DOWN, 0));
    for _ in 0..20 {
        if engine
            .object_snapshot(mage)
            .is_some_and(|object| object.container.is_none())
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        engine
            .object_snapshot(mage)
            .is_some_and(|object| object.container.is_none()),
        "MCLK exits AHUT through its ordinary Down control"
    );

    let bag_position = engine.test_object_snapshot(seeded_bag).position;
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(bag_position)
                .with_velocity(Vector2::ZERO)
                .with_action("Walk"),
        ),
    );
    for _ in 0..3 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.test_object_snapshot(seeded_bag).container,
        Some(mage),
        "the loose scenario bag enters MCLK through automatic collection"
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DIG, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_DIG, 0));
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC")),
        Some(3),
        "ALC_::Activate transfers the scenario ingredients into MCLK's hidden bag"
    );
    assert_eq!(
        engine
            .object_snapshot(seeded_bag)
            .and_then(|bag| bag.components.get("IROC")),
        Some(0),
        "the player route moves rather than duplicates the seeded ingredients"
    );
    assert_eq!(
        engine.call_test_object_function(
            engine.test_object_index(mage),
            "CheckMagicRequirements",
            vec![Value::C4Id("MGUP".into()), Value::Bool(true)],
        ),
        Value::Int(3),
        "the spell system finds all three IROC in MCLK's attached bag"
    );
}

fn alchemy_raise_undead_selects_a_dead_clonk_and_animates_it(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy raise undead parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let mage_position = engine.test_object_snapshot(mage).position;

    // RUND animates the dead and nothing else, so the subject has to actually
    // be a dead clonk: SelectorTarget wants OCF_Living, IsClonk, !GetAlive and
    // an effect-free target (RaiseUndead.c4d/Script.c:33-39).
    let corpse = engine.spawn_test_object(
        clonk_engine::SpawnConfig::new("CLNK")
            .with_position(Vector2::new(mage_position.x + 30, mage_position.y))
            .with_owner(OWNER_NONE)
            .with_action(ActionState::new("Walk")),
    );
    crate::support::TestValueExt::test_value(engine.change_object_energy(
        engine.test_object_index(corpse),
        -100,
        0,
        -1,
    ));
    for _ in 0..5 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        !engine.test_object_snapshot(corpse).alive,
        "the subject has to be dead before RUND will look at it"
    );

    let spell = engine.spawn_test_object(
        clonk_engine::SpawnConfig::new("RUND")
            .with_position(mage_position)
            .with_owner(owner),
    );
    engine.call_test_object_function(
        engine.test_object_index(spell),
        "Activate",
        vec![Value::Object(mage.as_u64())],
    );

    let selector = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "SLCR" && object.status.is_active())
            .map(|object| object.id),
    );
    assert_eq!(engine.crew_cursor(owner), Some(selector));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    assert_eq!(engine.crew_cursor(owner), Some(mage));

    // ActivateTarget installs UndeadSpell and deletes the spell
    // (RaiseUndead.c4d/Script.c:25-31).
    assert!(
        engine
            .test_object_snapshot(corpse)
            .effects
            .iter()
            .any(|effect| effect.name == "UndeadSpell"),
        "RUND animates the corpse it was pointed at"
    );
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "RUND" && object.status.is_active()),
        "RUND removes itself once the corpse is animated"
    );
}

fn alchemy_possession_uses_the_shipped_selector_control(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy selector parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let mage_position = engine.test_object_snapshot(mage).position;
    let possession = engine.spawn_test_object(
        clonk_engine::SpawnConfig::new("POSE")
            .with_position(mage_position)
            .with_owner(owner),
    );
    let mage_index = engine.test_object_index(mage);

    // C4Object::Call routes the spell's DoSpellSelect into SLCR creation;
    // C4Player::InCom then sends Right/Throw to SLCR's Control* callbacks
    // (C4Object.cpp:3229-3325; C4Player.cpp:1490-1554;
    // Selector.c4d/Script.c:6-43,128-174).
    let selector_value = engine.call_test_object_function(
        mage_index,
        "DoSpellSelect",
        vec![
            Value::Object(possession.as_u64()),
            Value::Int(400),
            Value::Object(mage.as_u64()),
        ],
    );
    let selector = match selector_value {
        Value::Object(raw) => ObjectId::new(raw),
        other => panic!("DoSpellSelect returns SLCR, got {other:?}"),
    };
    assert_eq!(engine.crew_cursor(owner), Some(selector));
    let target_count = engine.call_test_object_function(
        engine.test_object_index(selector),
        "CountTargets",
        Vec::new(),
    );
    assert!(
        matches!(target_count, Value::Int(2..=8)),
        "Alchemy has multiple nearby possessible animals within SLCR's eight-target cap: {target_count:?}"
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
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

fn alchemy_curse_family_selects_a_foreign_target_through_the_shipped_selector(
    prepared: &PreparedInstalledScenario,
) {
    // The four curses are one implementation with four names: each supplies
    // `GetCurseName` and inherits the shared Activate/SelectorTarget/
    // ActivateTarget path, which builds the effect name as
    // `Format("Curse%s", GetCurseName())`
    // (Curses.c4d/CurseAntiheal.c4d/Script.c:11-37,124).
    for (curse, effect_name) in [
        ("CAHE", "CurseAntiheal"),
        ("CCNF", "CurseControlConfusion"),
        ("CFAL", "CurseFalling"),
        ("CPAN", "CursePain"),
    ] {
        let mut engine = prepared.instantiate();
        let owner = join_local_player(&mut engine, "Alchemy curse parity");
        let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
        let mage_position = engine.test_object_snapshot(mage).position;

        // SelectorTarget refuses a friendly target unless the NTMG rule is in
        // play, and single-player Alchemy has no hostile crew at all -- so this
        // subcase exists only because the scenario ships
        // `Rules=...;NTMG=1;` and legalises exactly that
        // (Alchemy.c4s/Scenario.txt; Curses.c4d/CurseAntiheal.c4d/Script.c:48).
        // What the test still has to arrange is range: the shipped second crew
        // member starts too far away for the search box, and with nobody in it
        // the spell deletes itself and no selector ever opens.
        let patient = crate::support::TestValueExt::test_value(
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| {
                    object.definition_id == "CLNK"
                        && object.owner == owner
                        && object.status.is_active()
                })
                .map(|object| object.id),
        );
        crate::support::TestValueExt::test_value(
            engine.apply_object_update(
                patient,
                ObjectUpdate::new()
                    .with_position(Vector2::new(mage_position.x + 30, mage_position.y))
                    .with_velocity(Vector2::ZERO)
                    .with_action("Walk")
                    .clear_container(),
            ),
        );

        let spell = engine.spawn_test_object(
            clonk_engine::SpawnConfig::new(curse)
                .with_position(mage_position)
                .with_owner(owner),
        );

        // Going through Activate rather than straight to DoSpellSelect is the
        // point: Activate is what records the caster in `pCasterClonk`, and
        // SelectorTarget refuses that object, so a mage can never curse
        // itself (Curses.c4d/CurseAntiheal.c4d/Script.c:17-19,40-48).
        engine.call_test_object_function(
            engine.test_object_index(spell),
            "Activate",
            vec![Value::Object(mage.as_u64())],
        );
        let selector = crate::support::TestValueExt::test_value(
            engine
                .snapshot()
                .objects
                .iter()
                .find(|object| object.definition_id == "SLCR" && object.status.is_active())
                .map(|object| object.id),
        );
        assert_eq!(
            engine.crew_cursor(owner),
            Some(selector),
            "{curse} hands the cursor to the shipped selector"
        );

        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
        crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
        assert_eq!(engine.crew_cursor(owner), Some(mage));

        let cursed: Vec<_> = engine
            .snapshot()
            .objects
            .iter()
            .filter(|object| {
                object
                    .effects
                    .iter()
                    .any(|effect| effect.name == effect_name)
            })
            .map(|object| object.id)
            .collect();
        assert_eq!(
            cursed.len(),
            1,
            "{curse} curses exactly one target with {effect_name}: {cursed:?}"
        );
        assert_ne!(
            cursed[0], mage,
            "{curse} must not curse its own caster: SelectorTarget excludes pCasterClonk"
        );
        assert!(
            !engine
                .snapshot()
                .objects
                .iter()
                .any(|object| object.definition_id == curse && object.status.is_active()),
            "{curse} removes itself once the curse is placed"
        );
    }
}

fn alchemy_combo_mode_opens_and_accepts_the_shipped_element_control(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy combo parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let mage_index = engine.test_object_index(mage);

    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_" && object.components.get("IROC") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );

    engine.call_test_object_function(
        mage_index,
        "ContextCombo",
        vec![Value::Object(mage.as_u64())],
    );
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
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_SPECIAL, 0));
    let combo = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "CBMU" && object.status.is_active())
            .map(|object| object.id),
    );
    assert_eq!(engine.crew_cursor(owner), Some(combo));

    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        clonk_engine::COM_DOWN,
        0,
    ));
    let combo_snapshot = engine.test_object_snapshot(combo);
    assert_eq!(
        combo_snapshot.local_vars.get("szCastKeys"),
        Some(&Value::String("2".into()))
    );
    assert_eq!(
        combo_snapshot.local_vars.get("iCastControlCount"),
        Some(&Value::Int(1))
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        clonk_engine::COM_DOWN,
        0,
    ));
    assert_eq!(
        engine
            .test_object_snapshot(combo)
            .local_vars
            .get("szCastKeys"),
        Some(&Value::String("22".into()))
    );

    let gravity_before = engine.physics().gravity;
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_UP, 0));
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
    assert_eq!(engine.test_object_snapshot(mage).action.name, "Magic");

    for _ in 0..8 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.physics().gravity,
        gravity_before + 20,
        "the CBMU-selected MGUP executes its shipped gravity effect"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IROC")),
        Some(2),
        "the combo cast consumes MGUP's one IROC ingredient"
    );
}

fn alchemy_guarding_zaps_turns_carried_gold_into_a_nest_instead_of_zaps(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy guarding zaps parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let mage_position = engine.test_object_snapshot(mage).position;

    // The gold combo is checked before anything else, and it short-circuits
    // the whole spell: carried GOLD is turned into a ZAPN in place and GZ9Z
    // returns without ever offering a selector or creating a single zap
    // (GuardingZaps.c4d/Script.c:12-17).
    let gold =
        engine.spawn_test_object(clonk_engine::SpawnConfig::new("GOLD").with_container(mage));
    let spell = engine.spawn_test_object(
        clonk_engine::SpawnConfig::new("GZ9Z")
            .with_position(mage_position)
            .with_owner(owner),
    );
    engine.call_test_object_function(
        engine.test_object_index(spell),
        "Activate",
        vec![Value::Object(mage.as_u64())],
    );

    // Same object, new definition -- the gold is converted, not consumed and
    // replaced.
    assert_eq!(
        engine.test_object_snapshot(gold).definition_id,
        "ZAPN",
        "carried gold becomes the zap nest itself"
    );
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "ZAP2" && object.status.is_active()),
        "the gold branch returns before CreateZaps, so no loose zaps exist"
    );
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "SLCR" && object.status.is_active()),
        "the gold branch returns before DoSpellSelect, so no selector opens"
    );
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "GZ9Z" && object.status.is_active()),
        "GZ9Z removes itself on the gold branch"
    );
}

fn alchemy_learned_lightning_cast_launches_the_shipped_line_object(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy lightning parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // Alchemy's InitializePlayer puts MLGT's IBON=2 recipe into the seeded
    // loose bag. Move that recipe into MCLK's attached bag through the same
    // ALC_::Transfer callback used by gameplay (Alchemy.c4s/Script.c:21-37;
    // Lightning.c4d/DefCore.txt:11; Bag.c4d/Script.c:148-160).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_" && object.components.get("IBON") == Some(2)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );

    // Alchemy omits MLGT from its initial Scenario.txt list and teaches
    // random C4D_Magic definitions through SCRL. Granting that learned entry
    // here starts at the same C4Player magic-list state reached after reading
    // an MLGT scroll (Alchemy.c4s/Script.c:5-16; C4Player.cpp:1052-1058).
    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "MLGT"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let lightning_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MLGT"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        lightning_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    let aimer = crate::support::TestValueExt::test_value((0..12).find_map(|_| {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "AIMR" && object.status.is_active())
            .map(|object| object.id)
    }));
    assert_eq!(engine.crew_cursor(owner), Some(aimer));

    // AIMR::DoEnter calls MLGT::ActivateAngle. C++ creates LGTS, calls
    // Launch, and LGTS::Activate seeds the first vertex and Advance action
    // (Aimer.c4d/Script.c:242-270; Lightning.c4d/Script.c:22-35;
    // LightningShot.c4d/Script.c:12-34).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    assert_eq!(engine.crew_cursor(owner), Some(mage));
    let lightning = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "LGTS" && object.status.is_active())
            .cloned(),
    );
    assert_eq!(lightning.action.name, "Advance");
    assert!(
        !lightning.vertices.is_empty(),
        "LGTS::Activate seeds the cast origin as its first line vertex"
    );

    let vertex_count = lightning.vertices.len();
    for _ in 0..3 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let advanced = engine.test_object_snapshot(lightning.id);
    assert!(
        advanced.vertices.len() > vertex_count,
        "LGTS::Advance extends the lightning line: before={vertex_count}, after={:?}",
        advanced.vertices
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IBON")),
        Some(0),
        "the successful MLGT cast consumes its shipped two-bone recipe"
    );
}

fn alchemy_learned_fireball_aims_steers_and_explodes_through_player_controls(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy fireball parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // MFRB costs the same ISPH=2/IGOL=1 as MICS while Alchemy seeds ISPH=1
    // and IGOL=3, so the bag needs one harvested sphere on top of the shipped
    // one, transferred through ALC_::Transfer exactly as ordinary play does
    // (Alchemy.c4s/Script.c:21-37; Fireball.c4d/DefCore.txt:7;
    // Bag.c4d/Script.c:148-160).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("ISPH") == Some(1)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    let extra_sphere = engine.spawn_test_object(
        SpawnConfig::new("ALC_").with_ordered_components(vec![("ISPH".to_owned(), 1)]),
    );
    let attached_bag_index = engine.test_object_index(attached_bag);
    for source in [seeded_bag, extra_sphere] {
        engine.call_test_object_function(
            attached_bag_index,
            "Transfer",
            vec![Value::Object(source.as_u64())],
        );
    }
    assert_eq!(
        engine.call_test_object_function(
            engine.test_object_index(mage),
            "CheckMagicRequirements",
            vec![Value::C4Id("MFRB".into()), Value::Bool(true)],
        ),
        Value::Int(1)
    );

    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "MFRB"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let fireball_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MFRB"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        fireball_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    // MFRB::Activate creates the ball first and only then offers it to
    // DoSpellAim, so both objects exist before the aimer takes the cursor
    // (Fireball.c4d/Script.c:19-24).
    let (aimer, fireball) = crate::support::TestValueExt::test_value((0..12).find_map(|_| {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        let snapshot = engine.snapshot();
        let aimer = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "AIMR" && object.status.is_active())
            .map(|object| object.id)?;
        let fireball = snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "FIRB" && object.status.is_active())
            .map(|object| object.id)?;
        Some((aimer, fireball))
    }));
    assert_eq!(engine.crew_cursor(owner), Some(aimer));

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_UP, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    // Unlike MICS, MFRB never hands the cursor to what it launched: the ball
    // has no crew-control path of its own, so releasing the aim returns
    // control to the mage and the ball flies on unattended
    // (Fireball.c4d/Script.c:59-69; Fireballf.c4d/Script.c has no SetCursor).
    assert_eq!(
        engine.crew_cursor(owner),
        Some(mage),
        "releasing MFRB's aim returns the cursor to the mage"
    );
    // FxFireballFlightSetAngle stores the released aim in EffectVar(2) and
    // frees the launch in EffectVar(3) (Fireballf.c4d/Script.c:86-92).
    let launch = crate::support::TestValueExt::test_value(
        engine.object_snapshot(fireball).and_then(|fireball| {
            fireball
                .effects
                .iter()
                .find(|effect| effect.name == "FireballFlight")
                .map(|effect| (effect.var(2), effect.var(3)))
        }),
    );
    assert_eq!(
        launch,
        (EffectVarValue::Int(70), EffectVarValue::Int(1)),
        "aiming straight up releases at 70 and stamps the launch on the first          effect tick"
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_THROW + COM_RELEASE_OFFSET,
        0,
    ));

    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("ISPH")),
        Some(0),
        "a successful MFRB cast consumes both spheres"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("IGOL")),
        Some(2),
        "a successful MFRB cast consumes one gold"
    );

    // Freeing the launch does not start the flight. The effect is added with
    // size 1 and a maximum of 100, and every timer tick spent below the
    // maximum grows the ball by one and returns early -- so the ball charges
    // in place for a hundred ticks before a single SetXDir runs
    // (Fireballf.c4d/Script.c:22,154-166,171-174).
    let launch_position = engine.test_object_snapshot(fireball).position;
    for _ in 0..20 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.test_object_snapshot(fireball).position,
        launch_position,
        "a charging FIRB holds its position"
    );

    for _ in 0..120 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        engine
            .object_snapshot(fireball)
            .is_none_or(|flying| flying.position != launch_position),
        "a fully charged FIRB flies instead of hovering at its creation point"
    );

    // And it always ends: the flight either detonates on what it hits or
    // expires, but it never becomes a permanent object
    // (Fireballf.c4d/Script.c:47-51,135-136,172).
    let expired = (0..600).any(|_| {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        engine
            .object_snapshot(fireball)
            .is_none_or(|flying| !flying.status.is_active())
    });
    assert!(expired, "FIRB removes itself instead of flying forever");
}

fn alchemy_walk_on_liquid_installs_its_fixed_duration_on_the_real_caster(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy walk on liquid parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let mage_position = engine.test_object_snapshot(mage).position;
    let spell = engine.spawn_test_object(
        clonk_engine::SpawnConfig::new("WOLI")
            .with_position(mage_position)
            .with_owner(owner),
    );

    engine.call_test_object_function(
        engine.test_object_index(spell),
        "Activate",
        vec![Value::Object(mage.as_u64())],
    );

    // WOLI hands AddEffect a literal 1000, and the effect stashes it in
    // EffectVar(0) as the countdown its timer spends
    // (WalkOnLiquid.c4d/Script.c:11,38-40). Pinning the number matters because
    // the spell has no other observable output at cast time -- the walking
    // itself only shows up once the caster is over liquid.
    let duration = crate::support::TestValueExt::test_value(
        engine
            .test_object_snapshot(mage)
            .effects
            .iter()
            .find(|effect| effect.name == "WalkOnLiquidSpell")
            .map(|effect| effect.var(0)),
    );
    assert_eq!(duration, EffectVarValue::Int(1000));

    // The spell removes itself only when the effect took: a refused AddEffect
    // returns false and leaves the object alive (`:11-13`).
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "WOLI" && object.status.is_active()),
        "WOLI removes itself once its effect is installed"
    );
}

fn alchemy_learned_firebreath_aims_and_attaches_its_breath_to_the_caster(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy firebreath parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // MDBT is the most expensive of Alchemy's sphere spells at ISPH=3 against
    // the scenario's seeded ISPH=1, so two harvested spheres go through
    // ALC_::Transfer on top of the shipped bag
    // (Alchemy.c4s/Script.c:21-37; Firebreath.c4d/DefCore.txt;
    // Bag.c4d/Script.c:148-160).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("ISPH") == Some(1)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    let attached_bag_index = engine.test_object_index(attached_bag);
    let extra_spheres = [
        engine.spawn_test_object(
            SpawnConfig::new("ALC_").with_ordered_components(vec![("ISPH".to_owned(), 1)]),
        ),
        engine.spawn_test_object(
            SpawnConfig::new("ALC_").with_ordered_components(vec![("ISPH".to_owned(), 1)]),
        ),
    ];
    for source in [seeded_bag, extra_spheres[0], extra_spheres[1]] {
        engine.call_test_object_function(
            attached_bag_index,
            "Transfer",
            vec![Value::Object(source.as_u64())],
        );
    }
    assert_eq!(
        engine.call_test_object_function(
            engine.test_object_index(mage),
            "CheckMagicRequirements",
            vec![Value::C4Id("MDBT".into()), Value::Bool(true)],
        ),
        Value::Int(1)
    );

    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "MDBT"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let firebreath_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MDBT"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        firebreath_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    // MDBT aims before it creates anything: unlike MFRB there is no object to
    // show until the angle is released (Firebreath.c4d/Script.c:18-24).
    let aimer = crate::support::TestValueExt::test_value((0..12).find_map(|_| {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "AIMR" && object.status.is_active())
            .map(|object| object.id)
    }));
    assert_eq!(engine.crew_cursor(owner), Some(aimer));
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "FBRT" && object.status.is_active()),
        "no breath exists while the mage is still aiming"
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_UP, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_THROW + COM_RELEASE_OFFSET,
        0,
    ));

    // ActivateAngle creates the breath through the shipped global
    // CreateFireBreath and then deletes the spell object itself
    // (Firebreath.c4d/Script.c:28-39; FBreath.c4d/Script.c:84-87).
    let breath = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "FBRT" && object.status.is_active())
            .map(|object| object.id),
    );
    let breath_state = engine.test_object_snapshot(breath);
    assert_eq!(
        breath_state.action.name, "Exist",
        "FBRT::Activate pseudo-attaches the breath with SetAction(\"Exist\", caster)"
    );
    assert_eq!(
        breath_state.action.target,
        Some(mage),
        "the breath hangs off the caster, which is what its timer reads back"
    );
    assert!(
        !engine
            .snapshot()
            .objects
            .iter()
            .any(|object| object.definition_id == "MDBT" && object.status.is_active()),
        "MDBT removes itself once the breath exists"
    );
    assert_eq!(
        engine
            .object_snapshot(attached_bag)
            .and_then(|bag| bag.components.get("ISPH")),
        Some(0),
        "a successful MDBT cast consumes all three spheres"
    );

    // The breath is not permanent: its timer counts the lifetime handed to
    // Activate down to removal (FBreath.c4d/Script.c:13,29-32).
    let expired = (0..400).any(|_| {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        engine
            .object_snapshot(breath)
            .is_none_or(|breath| !breath.status.is_active())
    });
    assert!(expired, "FBRT expires instead of burning forever");
}

fn alchemy_fishskin_picks_its_revaluation_target_by_magic_physical(
    prepared: &PreparedInstalledScenario,
) {
    // FHSK does not have one output, it has two: a caster with a Magic
    // physical becomes a FCLK and everyone else becomes an ACLK
    // (Fishskin.c4d/Script.c:17-19). The mage carries Magic=45000 from
    // MagiClonk.c4d/DefCore.txt, so the two branches are reachable from the
    // same scenario by choosing the target.
    for (magic_caster, expected_definition) in [(true, "FCLK"), (false, "ACLK")] {
        let mut engine = prepared.instantiate();
        let owner = join_local_player(&mut engine, "Alchemy fishskin parity");
        let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
        let mage_position = engine.test_object_snapshot(mage).position;
        let target = if magic_caster {
            mage
        } else {
            let plain = engine.spawn_test_object(
                clonk_engine::SpawnConfig::new("CLNK")
                    .with_position(Vector2::new(mage_position.x + 20, mage_position.y))
                    .with_owner(owner)
                    .with_action(ActionState::new("Walk")),
            );
            // A spawned clonk carries no crew info at all, so it has no
            // Magic physical to read -- which is exactly the condition the
            // contrast case needs.
            assert!(
                engine
                    .test_object_snapshot(plain)
                    .info_physical
                    .is_none_or(|physical| physical.magic == 0),
                "the contrast case has to be a clonk with no Magic physical"
            );
            plain
        };

        let spell = engine.spawn_test_object(
            clonk_engine::SpawnConfig::new("FHSK")
                .with_position(mage_position)
                .with_owner(owner),
        );
        engine.call_test_object_function(
            engine.test_object_index(spell),
            "Activate",
            vec![Value::Object(target.as_u64())],
        );

        // The revaluation is a ChangeDef in place: same object, new
        // definition, put back into Walk (Fishskin.c4d/Script.c:30-34).
        let revalued = engine.test_object_snapshot(target);
        assert_eq!(
            revalued.definition_id, expected_definition,
            "a caster with magic_caster={magic_caster} revalues to {expected_definition}"
        );
        assert_eq!(revalued.action.name, "Walk");
    }
}

fn alchemy_learned_icestrike_aims_steers_and_impacts_through_player_controls(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy icestrike parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(500, 200))
                .with_velocity(Vector2::ZERO)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // Alchemy seeds ISPH=1 and IGOL=3, while MICS consumes ISPH=2 and
    // IGOL=1. Transfer the shipped bag plus one harvested sphere through
    // ALC_::Transfer, the same path used by ordinary play
    // (Alchemy.c4s/Script.c:21-37; Icestrike.c4d/DefCore.txt:7;
    // Bag.c4d/Script.c:148-160).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_"
                    && object.components.get("ISPH") == Some(1)
                    && object.components.get("IGOL") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    let extra_sphere = engine.spawn_test_object(
        SpawnConfig::new("ALC_").with_ordered_components(vec![("ISPH".to_owned(), 1)]),
    );
    let attached_bag_index = engine.test_object_index(attached_bag);
    for source in [seeded_bag, extra_sphere] {
        engine.call_test_object_function(
            attached_bag_index,
            "Transfer",
            vec![Value::Object(source.as_u64())],
        );
    }
    assert_eq!(
        engine.call_test_object_function(
            engine.test_object_index(mage),
            "CheckMagicRequirements",
            vec![Value::C4Id("MICS".into()), Value::Bool(true)],
        ),
        Value::Int(1)
    );

    // Reading a shipped SCRL grants its spell to C4Player::Magic; granting
    // that same entry directly isolates MICS after the scroll has been read
    // (Alchemy.c4s/Script.c:5-16; C4Player.cpp:1052-1058).
    crate::support::TestValueExt::test_value(engine.grant_player_magic(owner, "MICS"));
    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let icestrike_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MICS"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        icestrike_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    let (aimer, iceball) = crate::support::TestValueExt::test_value((0..12).find_map(|_| {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    }));
    assert_eq!(engine.crew_cursor(owner), Some(aimer));

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_UP, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    assert_eq!(
        engine.crew_cursor(owner),
        Some(iceball),
        "MICS::ActivateAngle hands direct control to the launched ICEB"
    );
    let launched_angle = crate::support::TestValueExt::test_value(
        engine.object_snapshot(iceball).and_then(|iceball| {
            iceball
                .effects
                .iter()
                .find(|effect| effect.name == "IceStrikeFlight")
                .map(|effect| effect.var(2))
        }),
    );
    assert_eq!(launched_angle, EffectVarValue::Int(70));
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_THROW + COM_RELEASE_OFFSET,
        0,
    ));

    // C4Player::InCom forwards Right and RightReleased to the ICEB cursor;
    // its effect applies the steering speed on the following timer tick
    // (C4Player.cpp:1490-1554; C4Object.cpp:3307-3325;
    // Iceball.c4d/Script.c:47-74,94-101,166-218).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        engine.crew_cursor(owner),
        Some(iceball),
        "an active non-crew cursor survives ICEB's ordinary effect update"
    );
    let steered_angle = crate::support::TestValueExt::test_value(
        engine.object_snapshot(iceball).and_then(|iceball| {
            iceball
                .effects
                .iter()
                .find(|effect| effect.name == "IceStrikeFlight")
                .map(|effect| effect.var(2))
        }),
    );
    assert_ne!(steered_angle, launched_angle);
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_RIGHT + COM_RELEASE_OFFSET,
        0,
    ));

    let impact_position = engine.test_object_snapshot(iceball).position;
    let target = engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_owner(OWNER_NONE)
            .with_position(Vector2::new(impact_position.x + 5, impact_position.y))
            .with_action(ActionState::new("Walk")),
    );
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            target,
            ObjectUpdate::new()
                .with_position(Vector2::new(impact_position.x + 5, impact_position.y))
                .with_velocity(Vector2::ZERO),
        ),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
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
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert!(
        engine
            .test_object_snapshot(target)
            .effects
            .iter()
            .any(|effect| effect.name == "Freeze"),
        "the ICEB frostwave freezes a living target in its first radius"
    );
}

fn alchemy_earthquake_cast_applies_the_shipped_view_shake(prepared: &PreparedInstalledScenario) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy earthquake parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // Alchemy's loose starter bag contains exactly MQKE's IROC=3 recipe.
    // Transfer it through the shipped attached-bag callback, then choose
    // Earthquake from MCLK's real ContextMagic menu and let the Magic action
    // reach phase five (Alchemy.c4s/Script.c:21-37; Magic.c:65-92,132-162;
    // Earthquake.c4d/DefCore.txt:9; MagiClonk.c4d/Script.c:219-261,430-445).
    let seeded_bag = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "ALC_" && object.components.get("IROC") == Some(3)
            })
            .map(|object| object.id),
    );
    let attached_bag = attached_alchemy_bag(&engine, mage);
    engine.call_test_object_function(
        engine.test_object_index(attached_bag),
        "Transfer",
        vec![Value::Object(seeded_bag.as_u64())],
    );
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(mage, ObjectUpdate::new().with_direction(Direction::Right)),
    );
    let cast_origin = engine.test_object_snapshot(mage).position;
    let landscape_before = crate::support::TestValueExt::test_value(engine.landscape().cloned());

    assert!(engine
        .execute_context_menu(mage, "ContextMagic")
        .expect("MCLK opens its shipped magic menu"));
    let earthquake_index = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner))
            .1
            .items
            .iter()
            .position(|item| item.item_id == "MQKE"),
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(
        owner,
        COM_MENU_SELECT,
        earthquake_index as i32,
    ));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    let quake = (0..12)
        .find_map(|_| {
            crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let quake_snapshot = engine.test_object_snapshot(quake);
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
    let quake_effect = crate::support::TestValueExt::test_value(
        quake_snapshot
            .effects
            .iter()
            .find(|effect| effect.name == "QuakeEffect"),
    );
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
            .and_then(|bag| bag.components.get("IROC")),
        Some(0),
        "a successful MQKE cast consumes its three-rock recipe"
    );
    let changed_landscape_pixels = {
        let before_grid = crate::support::TestValueExt::test_value(landscape_before.pixel_grid());
        let after_grid = crate::support::TestValueExt::test_value(
            engine
                .landscape()
                .and_then(|landscape| landscape.pixel_grid()),
        );
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let protege =
        engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(Vector2::new(500, 200)));
    let rock =
        engine.spawn_test_object(SpawnConfig::new("ROCK").with_position(Vector2::new(545, 200)));
    let mut field_action = ActionState::new("Field");
    field_action.target = Some(protege);
    let field = engine.spawn_test_object(
        SpawnConfig::new("FRCS")
            .with_owner(OWNER_NONE)
            .with_position(Vector2::new(500, 200))
            .with_action(field_action),
    );

    // FRCS::Timer flings the nearby ROCK and calls
    // Sound(..., false, obj, 50, 0, false, true, 300). Its loop slot is a
    // C4ValueInt, and C++ accepts Bool->Int unchanged (ForceFieldSmall.c4d/
    // Script.c:112; C4Script.cpp:2297; C4Value.cpp:509-520).
    engine.call_test_object_function(engine.test_object_index(field), "Timer", Vec::new());
    let snapshot = crate::support::TestValueExt::test_value(engine.tick());
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

    let caster = engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_owner(owner)
            .with_position(spell_position)
            .with_direction(Direction::Right),
    );
    let victim_position = Vector2::new(wall_x, spell_position.y - 100);
    let victim = engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_owner(owner)
            .with_position(victim_position),
    );
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(victim, ObjectUpdate::new().with_position(victim_position)),
    );
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
    let spell = engine.spawn_test_object(
        SpawnConfig::new("MFFW")
            .with_owner(owner)
            .with_position(spell_position),
    );
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(spell, ObjectUpdate::new().with_position(spell_position)),
    );

    // C4Game::NewObject performs initial DoCon (and therefore puts the
    // definition SolidMask) before Initialize. FCWS::Initialize inherits
    // FRCA::Initialize -> CheckStuck, so the first segment's phase-two mask
    // already ejects this same-x CLNK. The opaque mask is x=-3..=2 relative
    // to the segment and CLNK's leftmost vertex is -4; +7 is the first free
    // center (C4Object.cpp:1428-1511; C4SolidMask.cpp:61-107).
    assert_eq!(
        engine.call_test_object_function(
            engine.test_object_index(spell),
            "Activate",
            vec![Value::Object(caster.as_u64())],
        ),
        Value::Int(1)
    );
    let ejected_victim = engine.test_object_snapshot(victim);
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
    let landscape = crate::support::TestValueExt::test_value(engine.landscape());
    assert!(landscape.is_solid_at(wall_x - 3, spell_position.y - 110));
    assert!(landscape.is_solid_at(wall_x + 2, spell_position.y + 29));
    assert!(!landscape.is_solid_at(wall_x - 4, spell_position.y - 110));
    assert!(!landscape.is_solid_at(wall_x + 3, spell_position.y + 29));

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let landscape = crate::support::TestValueExt::test_value(engine.landscape());
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        segment_ids
            .iter()
            .all(|id| engine.object_snapshot(*id).is_none()),
        "the seven expired FCWS segments are removed together"
    );
    let landscape = crate::support::TestValueExt::test_value(engine.landscape());
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
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    assert_eq!(engine.test_object_snapshot(mage).definition_id, "MCLK");

    // The shipped MFBL Activate creates FRBL and immediately calls
    // Collect(pFireball,pClonk). C++ makes that same-call object live,
    // routes it through C4Object::Collect, and leaves it in the mage's
    // inventory when the Collection gate accepts it
    // (Firelump.c4d/Script.c:20-31; C4Script.cpp:391-415;
    // C4Object.cpp:5693-5714).
    let mage_position = engine.test_object_snapshot(mage).position;
    let spell = engine.spawn_test_object(
        SpawnConfig::new("MFBL")
            .with_owner(owner)
            .with_position(mage_position),
    );
    let spell_index = engine.test_object_index(spell);
    assert_eq!(
        engine.call_test_object_function(
            spell_index,
            "Activate",
            vec![Value::Object(mage.as_u64()), Value::Object(mage.as_u64()),],
        ),
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
    run_drachenfels_batch(&[
        (
            "shadow_generators_darken_the_mountain_until_a_clonk_walks_in",
            dragon_rock_shadow_generators_darken_the_mountain_until_a_clonk_walks_in,
        ),
        (
            "enabling_fog_rebuilds_every_saved_repeller_into_the_view_list",
            dragon_rock_enabling_fog_rebuilds_every_saved_repeller_into_the_view_list,
        ),
    ]);
}

/// `C4Player::SetFoW` runs `Game.Objects.AssignPlrViewRange()` the first time
/// fog is enabled (C4Player.cpp:817-818), which re-actualizes every live
/// object carrying a nonzero `PlrViewRange`. That matters for saved content:
/// `Objects.txt` restores `Owner=` for players that never join, and
/// `C4Object::PlrFoWActualize` puts an object whose owner is not a live player
/// into *every* player's list (C4Object.cpp:5546-5567). Nothing else re-walks
/// those — `C4Player::Init`'s own pass covers `NO_OWNER` objects only — so
/// without the rebuild they light nobody's map.
///
/// Dragon Rock ships exactly two of them live: MAGE #1758 and KING #5129, both
/// `PlrViewRange=500` and both `Owner=10`.
fn dragon_rock_enabling_fog_rebuilds_every_saved_repeller_into_the_view_list(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player_on_team(&mut engine, "Dragon Rock fog rebuild parity", 1);

    // InitializePlayer schedules SetFoW one tick out (Drachenfels.c4s/
    // Script.c:70; planet/System.c4g/Helpers.c:110-132).
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    let view_objects = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .fow_players
            .get(&owner)
            .map(|frame| frame.view_objects.clone()),
    );

    for (number, definition) in [(1758, "MAGE"), (5129, "KING")] {
        let object = ObjectId::new(number);
        let loaded = engine
            .object_snapshot(object)
            .unwrap_or_else(|| panic!("Dragon Rock object #{number} loads"));
        assert_eq!(loaded.definition_id, definition);
        assert!(loaded.status.is_active(), "#{number} loads live");
        assert_eq!(
            loaded.plr_view_range, 500,
            "#{number} saved PlrViewRange=500"
        );
        assert!(
            engine.player(loaded.owner).is_none(),
            "#{number}'s saved owner must not be a live player for this to bite"
        );
        assert!(
            view_objects.contains(&object),
            "enabling fog must rebuild saved repeller #{number} into every player's FoWViewObjs"
        );
    }

    // The rebuild re-actualizes, it does not append blindly: the four
    // ownerless generators the join-time pass already added stay exactly once.
    for (number, ..) in DRACHENFELS_SHADOWS {
        assert_eq!(
            view_objects
                .iter()
                .filter(|object| **object == ObjectId::new(number))
                .count(),
            1,
            "generator #{number} must not be duplicated by the rebuild"
        );
    }
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
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    let knight = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // InitializePlayer schedules SetFoW one tick out (Drachenfels.c4s/
    // Script.c:70; planet/System.c4g/Helpers.c:110-132).
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let view_objects = crate::support::TestValueExt::test_value(
        snapshot
            .fow_players
            .get(&owner)
            .map(|frame| frame.view_objects.clone()),
    );
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
    let hidden = drachenfels_hidden_objects(&engine.test_object_snapshot(shadow));
    assert_eq!(hidden.len(), 24, "generator #2779 saved iHiddenObjCnt=24");

    // 343px above the centre: outside the authored rect and outside the
    // widened circle (235 dark radius + 100 margin = 335), and far enough
    // from the other three generators that none of them can answer for it.
    // The crew member is re-placed every tick so gravity cannot walk it into
    // range and pass this probe for the wrong reason.
    assert_eq!(
        drachenfels_ticks_until_dispelled(&mut engine, knight, shadow, Vector2::new(1472, 960), 40),
        None,
        "a Clonk outside both the authored rect and the reveal circle must not dispel the shadow"
    );

    // One pixel above the rect's top edge — 161px up, which C++ answers `no`
    // to (`C4FindObjectInRect::Check` is a plain point-in-rect on the object
    // centre) while already 74px inside the fully-black disc. The reveal
    // circle is what dispels it here, within one 20-tick poll, and
    // `Deactivate` restores every object it was hiding (Script.c:96-112).
    assert!(
        drachenfels_ticks_until_dispelled(
            &mut engine,
            knight,
            shadow,
            Vector2::new(1472, 1142),
            20
        )
        .is_some(),
        "the reveal circle dispels a shadow the Clonk has reached the edge of"
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

    // The authored rect is still an independent arm of the union, not a
    // subset of the circle: #2781's top-left corner sits 488px from its
    // centre, outside its own 456px reveal circle, and the C++ rect answers
    // for it there exactly as it always did.
    assert!(
        drachenfels_ticks_until_dispelled(
            &mut engine,
            knight,
            ObjectId::new(2781),
            Vector2::new(1658, 1113),
            20
        )
        .is_some(),
        "the authored search rect still dispels at the corners it reaches past the circle"
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
        crate::support::TestValueExt::test_value(
            engine.apply_object_update(
                crew,
                ObjectUpdate::new()
                    .with_position(position)
                    .with_action("Walk")
                    .clear_container(),
            ),
        );
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
        engine.object_snapshot(shadow).is_none()
    })
}

fn dragon_rock_mage_choice_redefines_the_real_knight_and_transfers_its_flag(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player_on_team(&mut engine, "Dragon Rock character parity", 1);
    let knight = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    // Choose normal difficulty through the real KNIG object menu. The shipped
    // InitializePlayer2 then creates FLAG in that KNIG and opens the shipped
    // KNIG/MAGE selection menu (Drachenfels.c4s/Script.c:86-103,112-128).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    let flag = crate::support::TestValueExt::test_value(engine.object_snapshot(knight).and_then(
        |knight| {
            knight.contents.into_iter().find(|item| {
                engine
                    .object_snapshot(*item)
                    .is_some_and(|item| item.definition_id == "FLAG")
            })
        },
    ));
    let (_, choice) = crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner));
    assert_eq!(
        choice
            .items
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["KNIG", "MAGE"]
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_RIGHT, 0));
    assert_eq!(
        engine
            .cursor_object_menu(owner)
            .expect("character menu remains open")
            .1
            .selection,
        1,
        "the physical Right control selects MAGE"
    );
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));

    // Redefine3 creates MAGE, immediately calls pNew->GrabContents(this()),
    // copies the live state, installs it as crew/cursor, then removes KNIG
    // (Drachenfels.c4s/Script.c:150-178). FnGrabContents is an engine-global
    // function found after MAGE's own script and transfers a copied contents
    // list through ordinary Enter calls (C4Aul.cpp:130-148;
    // C4Script.cpp:320-327; C4Object.cpp:6162-6171).
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    assert_eq!(engine.test_object_snapshot(mage).definition_id, "MAGE");
    assert!(
        !engine.test_object_snapshot(knight).status.is_active(),
        "Redefine3 marks the old KNIG deleted immediately"
    );
    assert_eq!(
        engine.test_object_snapshot(flag).container,
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
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_THROW, 0));
    let knight = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    assert_eq!(engine.test_object_snapshot(knight).definition_id, "KNIG");

    let tent = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "TENT"
                    && object.status.is_active()
                    && object.position.x == 222
                    && object.position.y == 1240
            })
            .cloned(),
    );
    assert_ne!(
        tent.ocf & clonk_engine::ocf::ENTRANCE,
        0,
        "the shipped TENT remains targetable through OCF_Entrance"
    );
    // TENT DefCore.txt:17 is Entrance=-10,4,19,20.
    let entrance_center = Vector2::new(tent.position.x - 10 + 19 / 2, tent.position.y + 4 + 20 / 2);
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            knight,
            ObjectUpdate::new()
                .with_position(entrance_center)
                .with_action("Walk")
                .clear_container(),
        ),
    );

    // C++ WALK+Up first probes AtObject with OCF_Entrance and queues Enter;
    // C4Command::Enter then checks Target->At with the same entrance OCF and
    // calls C4Object::Enter when EntranceStatus is open
    // (C4ObjectCom.cpp:335-350; C4Command.cpp:545-615).
    crate::support::TestValueExt::test_value(engine.player_in_com(owner, COM_UP, 0));
    for _ in 0..3 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        engine.test_object_snapshot(knight).container,
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
    let player = crate::support::TestValueExt::test_value(engine.player(owner));
    let mut actual = player
        .knowledge()
        .map(|definition| definition.as_str())
        .collect::<Vec<_>>();
    actual.sort_unstable();
    assert_eq!(actual, expected, "both shipped plan sets persist exactly");

    // The difficulty menu is created after both definition calls. Its
    // presence proves InitializePlayer ran past every omitted remove flag
    // instead of aborting at the original argument-count warning.
    let (_, menu) = crate::support::TestValueExt::test_value(engine.cursor_object_menu(owner));
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
    let player = crate::support::TestValueExt::test_value(engine.player(owner));
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

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

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
    let player = crate::support::TestValueExt::test_value(engine.player(owner));
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
    let loaded = engine.test_object_snapshot(dragon);
    assert_eq!(loaded.direction.to_script_value(), 1);
    let transform = crate::support::TestValueExt::test_value(loaded.draw_transform);
    assert_eq!(transform.flip_dir(), -1);
    assert_eq!(transform.matrix()[0], -1.0);

    // ReverseDir (Dragon.c4d/Script.c) is exactly `SetDir(1-GetDir())`.
    // C4Object::SetDir runs UpdateFlipDir because Sleep's FlipDir is
    // non-zero (C4Object.cpp:4276-4279); Dir 0 < FlipDir 1 then takes the
    // "no flipdir necessary" branch, so SetFlipDir(1) unfolds mat[0] and the
    // now-identity transform is deleted (C4Object.cpp:431-442).
    let index = engine.test_object_index(dragon);
    engine.call_test_object_function(index, "ReverseDir", Vec::new());

    let turned = engine.test_object_snapshot(dragon);
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
    engine.call_test_object_function(index, "ReverseDir", Vec::new());

    let returned = engine.test_object_snapshot(dragon);
    assert_eq!(returned.direction.to_script_value(), 1);
    let transform = crate::support::TestValueExt::test_value(returned.draw_transform);
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let dragon_id = ObjectId::new(202);
    let dragon_before = engine.test_object_snapshot(dragon_id);
    assert!(dragon_before.alive, "the dragon starts alive");

    engine.call_test_scenario_script_function(
        "OnClonkDeath",
        vec![Value::Object(ObjectId::new(1758).as_u64())],
    );

    let dragon_after = engine.test_object_snapshot(dragon_id);
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
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    engine.call_test_scenario_script_function("OnDragonReachTarget", Vec::new());
    for _ in 0..59 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(engine.snapshot().frame, 219);

    let princess_before = engine.test_object_snapshot(ObjectId::new(1777));
    let old_sparks = engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == "SPRK")
        .map(|object| object.id)
        .collect::<Vec<_>>();

    let frame = crate::support::TestValueExt::test_value(engine.tick());
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
        let shape =
            crate::support::TestValueExt::test_value(engine.object_current_shape_rect(spark.id));
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
    let princess = engine.test_object_snapshot(ObjectId::new(1777));
    assert_eq!((princess.position.x, princess.position.y), (2145, 485));
    assert_eq!(princess.action.name, "Walk");
    assert_eq!(princess.direction.to_script_value(), 0);
    let endboss = engine.test_object_snapshot(ObjectId::new(1758));
    assert_eq!(endboss.action.name, "RideMagic");
    assert_eq!(endboss.action.target, Some(ObjectId::new(202)));
}

fn alchemy_tunnel_spell_opens_its_first_shipped_landscape_row(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Alchemy tunnel parity");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let earth = crate::support::TestValueExt::test_value(engine.materials().id_of("Earth"));
    let (target_x, target_y) = {
        let landscape = crate::support::TestValueExt::test_value(engine.landscape());
        let grid = crate::support::TestValueExt::test_value(landscape.pixel_grid());
        crate::support::TestValueExt::test_value((20..grid.height() as i32 - 20).find_map(|y| {
            (20..grid.width() as i32 - 20)
                .find(|&x| {
                    landscape.material_at(x, y) == Some(earth) && landscape.is_solid_at(x, y)
                })
                .map(|x| (x, y))
        }))
    };
    let solid_pixels_before = {
        let landscape = crate::support::TestValueExt::test_value(engine.landscape());
        let grid = crate::support::TestValueExt::test_value(landscape.pixel_grid());
        (target_y - 2..=target_y + 2)
            .flat_map(|y| (target_x - 17..=target_x + 17).map(move |x| (x, y)))
            .filter(|&(x, y)| landscape.is_solid_at(x, y))
            .filter_map(|(x, y)| grid.byte_at(x, y).map(|byte| (x, y, byte)))
            .collect::<Vec<_>>()
    };
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            mage,
            ObjectUpdate::new()
                .with_position(Vector2::new(target_x, target_y - 10))
                .clear_container(),
        ),
    );
    let spell = engine.spawn_test_object(
        SpawnConfig::new("MTNL")
            .with_owner(owner)
            .with_position(Vector2::new(target_x, target_y - 10)),
    );
    let spell_index = engine.test_object_index(spell);

    assert_eq!(
        engine.call_test_object_function(
            spell_index,
            "ActivateAngle",
            vec![Value::Object(mage.as_u64()), Value::Int(0)],
        ),
        Value::Int(1)
    );
    assert_eq!(
        engine
            .landscape()
            .expect("landscape before tunnel timer")
            .material_at(target_x, target_y),
        Some(earth)
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    let opened_pixels = {
        let landscape = crate::support::TestValueExt::test_value(engine.landscape());
        let grid = crate::support::TestValueExt::test_value(landscape.pixel_grid());
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
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let fuel = crate::support::TestValueExt::test_value(engine.materials().id_of("Oil"));
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
        let landscape = crate::support::TestValueExt::test_value(engine.landscape());
        let grid = crate::support::TestValueExt::test_value(landscape.pixel_grid());
        let air_position = crate::support::TestValueExt::test_value(
            (30..grid.height() as i32 - 30).find_map(|y| {
                (30..grid.width() as i32 - 30)
                    .find(|&x| {
                        landscape.material_at(x, y).is_none()
                            && landscape.material_at(x - 20, y).is_none()
                            && landscape.material_at(x + 20, y).is_none()
                            && landscape.material_at(x, y - 10).is_none()
                            && landscape.material_at(x, y + 10).is_none()
                    })
                    .map(|x| (x, y))
            }),
        );
        (
            (grid.width() as i32 / 2, grid.height() as i32 / 2),
            air_position,
        )
    };

    // The fixed seed has no naturally placed inflammable pixel. Draw a small
    // Oil-Smooth patch through the shipped engine API so the flame path has a
    // controlled C++-valid precondition.
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "FUEL",
            "Fuel painter",
            r#"#strict
        public func Paint(int x, int y)
        {
            return DrawMaterialQuad("Oil-Smooth", x-1,y-1, x+1,y-1, x+1,y+1, x-1,y+1, false);
        }
        "#,
        ),
    ));
    let painter = engine.spawn_test_object(SpawnConfig::new("FUEL"));
    let painter_index = engine.test_object_index(painter);
    assert_eq!(
        engine.call_test_object_function(
            painter_index,
            "Paint",
            vec![Value::Int(fuel_x), Value::Int(fuel_y)],
        ),
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
    let spell = engine.spawn_test_object(
        SpawnConfig::new("FRFS")
            .with_owner(owner)
            .with_position(Vector2::new(air_x + 15, air_y)),
    );
    let spell_index = engine.test_object_index(spell);
    assert_eq!(
        engine.call_test_object_function(
            spell_index,
            "Activate",
            vec![Value::Object(mage.as_u64()), Value::Object(mage.as_u64())],
        ),
        Value::Bool(true)
    );
    let fire_shower = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| {
                object.definition_id == "FSHW"
                    && object.status.is_active()
                    && object.action.name == "Left"
            })
            .map(|object| object.id),
    );
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        fire_shower,
        ObjectUpdate::new().with_position(Vector2::new(air_x, air_y)),
    ));
    let shower_index = engine.test_object_index(fire_shower);
    engine.call_test_object_function(shower_index, "Hit", Vec::new());
    let flame = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .iter()
            .find(|object| object.definition_id == "FLAM" && object.status.is_active())
            .map(|object| object.id),
    );
    assert!(
        engine.test_object_snapshot(flame).on_fire,
        "FLAM Completion incinerates the flame before BurnProcess"
    );
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        flame,
        ObjectUpdate::new().with_position(Vector2::new(fuel_x, fuel_y)),
    ));
    let fuel_before = crate::support::TestValueExt::test_value(engine.landscape())
        .material_pixel_count(fuel, None);

    let flame_index = engine.test_object_index(flame);
    assert_eq!(
        engine.call_test_object_function(flame_index, "BurnProcess", Vec::new()),
        Value::Int(1)
    );
    let fuel_after = crate::support::TestValueExt::test_value(engine.landscape())
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
        crate::support::TestValueExt::test_value(u32::try_from(base_x.saturating_add(100))),
        60,
    ));

    // The shipped formula in TRAP::ArmCheck is
    // (Dir*2-1)*(TGIN Width 8 + Offset.x -4 + TRPR Width 16 +
    // Offset.x -8 + 3) = +15. At y=50 over the flat y=60 surface, its
    // first upward probe leaves solid at i=1 and returns local y=9.
    let trapper = engine.spawn_test_object(
        SpawnConfig::new("TRPR")
            .with_position(Vector2::new(base_x, 50))
            .with_direction(Direction::Right)
            .with_alive(true)
            .with_loaded(true),
    );
    let trap = engine.spawn_test_object(
        SpawnConfig::new("TGIN")
            .with_position(Vector2::new(base_x, 50))
            .with_loaded(true),
    );
    let trap_index = engine.test_object_index(trap);

    assert_eq!(
        engine.call_test_object_function(
            trap_index,
            "ArmCheck",
            vec![Value::Object(trapper.as_u64())],
        ),
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

    let stalactite_index = engine.test_object_index(stalactite);
    engine.call_test_object_function(stalactite_index, "Hit", Vec::new());

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
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "FDRV",
            "Fade driver",
            r#"#strict
        func StartFade() { return FadeOut(4, 2, this()); }
        func RetargetFade() { return FadeOut(10, 5, this()); }
        "#,
        ),
    ));
    let target =
        engine.spawn_test_object(SpawnConfig::new("FDRV").with_position(Vector2::new(320, 120)));

    let index = engine.test_object_index(target);
    assert_eq!(
        engine.call_test_object_function(index, "StartFade", Vec::new()),
        Value::Int(1)
    );
    let index = engine.test_object_index(target);
    assert_eq!(
        engine.call_test_object_function(index, "RetargetFade", Vec::new()),
        Value::Int(1)
    );

    let object = engine.test_object_snapshot(target);
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
    let mut driver = crate::support::TestValueExt::test_value(Definition::from_script(
        "SCRH",
        "Scorching driver",
        "#strict\nfunc StartScorching() { return SetScorching(this()); }",
    ));
    driver.set_c4_callback_convention(true);
    engine.register_test_definition(driver);
    let target =
        engine.spawn_test_object(SpawnConfig::new("SCRH").with_position(Vector2::new(320, 120)));
    let index = engine.test_object_index(target);
    engine.call_test_object_function(index, "StartScorching", Vec::new());
    assert!(
        engine
            .test_object_snapshot(target)
            .effects
            .iter()
            .any(|effect| effect.name == "IntScorching"),
        "SetScorching installs the shipped smoke effect"
    );

    for _ in 0..10 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        engine
            .test_object_snapshot(target)
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
    let dynamite_box = engine.spawn_test_object(
        SpawnConfig::new("DYNB")
            .with_position(Vector2::new(320, 120))
            .with_construction(FULL_CON / 2),
    );
    let index = engine.test_object_index(dynamite_box);
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

    let box_after = engine.test_object_snapshot(dynamite_box);
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
    let target = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    assert!(
        engine.crew_object_info(target).is_some(),
        "the removal path must exercise this player's real CrewInfoList"
    );
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(target, ObjectUpdate::new().with_energy(1)),
    );

    let talker = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "_TLK" && object.status.is_active())
            .map(|object| object.id),
    );
    let talker_index = engine.test_object_index(talker);
    engine.call_test_object_function(
        talker_index,
        "FxWatchEnergyStop",
        vec![
            Value::Object(target.as_u64()),
            Value::Int(99_999),
            Value::Int(0),
            Value::Bool(false),
        ],
    );

    let sheriff = engine.test_object_snapshot(target);
    assert_eq!(sheriff.definition_id, "SHRF");
    assert!(!sheriff.crew_member);
    assert!(!sheriff.selected);
    assert!(engine.crew_object_info(target).is_none());
    assert!(!engine.crew_members(owner).contains(&target));
    assert_eq!(sheriff.energy, 50_000);
    assert!(sheriff.alive);
    assert_eq!(sheriff.action.name, "Walk");
    assert_eq!(sheriff.owner, OWNER_NONE);
    let stay_there = crate::support::TestValueExt::test_value(
        sheriff
            .effects
            .iter()
            .find(|effect| effect.name == "StayThere"),
    );
    assert_eq!(stay_there.priority, 1);
    assert_eq!(stay_there.interval, 35);
}

fn gold_rush_real_anvil_forges_a_wire_roll_from_its_metal_contents(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let mut forge_action = ActionState::new("Forge");
    forge_action.time = 150;
    let anvil = engine.spawn_test_object(
        SpawnConfig::new("ANVL")
            .with_action(forge_action)
            // Loaded objects restore Action/Time without replaying the
            // Forge StartCall before this fixture can add its METL.
            .with_loaded(true)
            .with_local_vars(std::collections::HashMap::from([(
                "product".to_owned(),
                Value::C4Id("WIRR".to_owned()),
            )])),
    );
    let metal = engine.spawn_test_object(SpawnConfig::new("METL").with_container(anvil));
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
    let anvil_index = engine.test_object_index(anvil);
    assert_eq!(
        engine.call_test_object_function(anvil_index, "Forging", Vec::new()),
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
    let knight = engine.spawn_test_object(
        SpawnConfig::new("KNIG")
            .with_position(Vector2::new(100, 100))
            .with_loaded(true),
    );
    let index = engine.test_object_index(knight);

    assert_eq!(
        engine.call_test_object_function(
            index,
            "FireBowAt",
            vec![Value::Int(150), Value::Int(150), Value::Bool(false)],
        ),
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
    let owner = crate::support::TestValueExt::test_value(
        crate::support::TestValueExt::test_value(engine.join_player(JoinPlayerConfig {
            team: Some(1),
            crew,
            ..crate::support::join_player_config("Lance parity")
        }))
        .initialized(),
    )
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
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        rider,
        ObjectUpdate::new().with_position(Vector2::new(9_000, 9_000)),
    ));
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(
            victim,
            ObjectUpdate::new()
                .with_position(Vector2::new(10_000, 9_973))
                .with_action("Walk"),
        ),
    );
    assert_eq!(
        engine.test_object_snapshot(victim).energy,
        55_000,
        "fair-crew promotion raises the real KNIG energy before the hit"
    );

    let mut lance_action = ActionState::new("Lance");
    lance_action.target = Some(rider);
    let lance = engine.spawn_test_object(
        SpawnConfig::new("LNCA")
            .with_position(Vector2::new(10_000, 10_000))
            .with_owner(owner)
            .with_action(lance_action)
            .with_local_vars(std::collections::HashMap::from([
                ("high_target".to_owned(), Value::Int(0)),
                ("last_x".to_owned(), Value::Int(9_969)),
            ]))
            .with_loaded(true),
    );

    // Lancing computes speed_x=31, draws Random(16), reads GetRank(rider)
    // as 5, and uses divisor BoundBy((5-3)/2,1,6)=1. The resulting angle
    // always clamps to SetRDir(12) at this speed.
    let mut expected_rng = engine.debug_rng_clone();
    expected_rng.random(16);
    let lance_index = engine.test_object_index(lance);
    assert_eq!(
        engine.call_test_object_function(lance_index, "Lancing", Vec::new()),
        Value::Int(1)
    );
    let aimed_lance = engine.test_object_snapshot(lance);
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
        engine.call_test_object_function(lance_index, "Targeting", Vec::new()),
        Value::Int(1)
    );
    let hit_victim = engine.test_object_snapshot(victim);
    assert_eq!(hit_victim.energy, 40_000);
    assert_eq!(hit_victim.action.name, "Tumble");
    assert_eq!(
        engine.test_object_snapshot(lance).local_vars.get("speed_x"),
        Some(&Value::Nil)
    );
    assert_eq!(engine.debug_rng_clone(), expected_rng);

    // C4Object::GrabInfo moves the same live Info pointer. Verify the
    // nonzero rank, not merely the fresh-rank-zero case, is visible on the
    // recipient immediately and remains there after the callback folds.
    let rank_probe = crate::support::TestValueExt::test_value(Definition::from_script(
        "RKPR",
        "Rank transfer probe",
        "#strict 2\nfunc Take(obj) { return [GrabObjectInfo(obj), GetRank(), GetRank(obj)]; }\nfunc Read() { return GetRank(); }",
    ));
    engine.register_test_definition(rank_probe);
    let rank_probe = engine.spawn_test_object(SpawnConfig::new("RKPR").with_owner(owner));
    let rank_probe_index = engine.test_object_index(rank_probe);
    assert_eq!(
        engine.call_test_object_function(
            rank_probe_index,
            "Take",
            vec![Value::Object(victim.as_u64())],
        ),
        Value::Array(vec![Value::Bool(true), Value::Int(5), Value::Nil])
    );
    assert_eq!(
        engine.call_test_object_function(rank_probe_index, "Read", Vec::new()),
        Value::Int(5)
    );
}
