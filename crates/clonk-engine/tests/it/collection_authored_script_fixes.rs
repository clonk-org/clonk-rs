//! Authored script defects in the imported Collection that the content
//! repository corrects (clonk-org/clonk-rs#1395). Each was reproduced under the
//! pinned C++ engine too, so these are content fixes and not parity gaps: the
//! tests pin that the shipped scripts now run, against the real content.

use crate::support::real_scenario::{
    content_root, join_local_player, join_local_player_on_team, load_installed_scenario,
    load_installed_scenario_in_languages, load_installed_scenario_with_selected_definitions,
    object_contents_count, object_with_definition,
};
use crate::support::EngineTestExt;
use clonk_engine::{Engine, ObjectId, ObjectStatus, SpawnConfig};
use clonk_script::Value;

fn call(engine: &mut Engine, object: ObjectId, function: &str) -> Value {
    let index = engine.test_object_index(object);
    engine
        .call_object_function(index, function, Vec::new())
        .unwrap_or_else(|error| panic!("{function} executes: {error}"))
}

/// clonk-org/clonk-rs-content#90: `Ueberlandleitung.c4d/Script.c` declared
/// `var pObj` inside an `if` condition, which neither engine's parser accepts,
/// so the whole UBRL script failed to parse and the power-line goal never
/// evaluated. The scenario's `UBRLTarget` hands it the mine, so the answer is
/// the mine's power state; what is pinned is that the goal answers at all.
#[test]
fn the_schweids_power_line_goal_parses_and_evaluates() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/Settling.c4f/RufDerWipfeRE.c4f/Kampagne.c4f/4Schweids.c4s",
        0,
    );
    join_local_player(&mut engine, "Schweids goal");
    let goal = object_with_definition(&engine, "UBRL")
        .unwrap_or_else(|| engine.spawn_test_object(SpawnConfig::new("UBRL")));

    let fulfilled = call(&mut engine, goal, "IsFulfilled");
    assert!(
        matches!(fulfilled.as_c4_int(), Some(0 | 1)),
        "the goal must evaluate to its 0/1 answer, got {fulfilled:?}"
    );
}

/// clonk-org/clonk-rs-content#82: the Teamview pack's `#appendto CLNK` called
/// `FindObject(RTVW)->ScheduleCall(..)` on every `Departure`, but ColdSkies
/// loads the pack without enabling the `RTVW` rule, so the crew's first
/// departure failed with a zero call target. The sibling appends in the same
/// pack guard the lookup; this one does now too.
#[test]
fn a_cold_skies_clonk_departs_without_the_teamview_rule() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/BaseMelees.c4f/ClassicMelees.c4f/ColdSkies.c4s",
        0,
    );
    // A team melee: the player has to be placed on a team to initialise.
    let owner = join_local_player_on_team(&mut engine, "ColdSkies departure", 1);
    assert!(
        object_with_definition(&engine, "RTVW").is_none(),
        "ColdSkies does not enable the Teamview rule"
    );
    let clonk = engine
        .crew_cursor(owner)
        .expect("the joined player has a crew member");

    call(&mut engine, clonk, "Departure");
}

/// clonk-org/clonk-rs-content#92: m0Xeron Settlement's `InitializePlayer` puts
/// the player's skeleton into a targetless `Build` action as its arrival pose
/// (`m0XeronSettlement.c4s/Script.c:286`), and the scenario's `BACC` rule
/// dereferenced `GetActionTarget()` on every Clonk it found in `Build`. The
/// engine stops a targetless builder on its next action cycle
/// (C4Object.cpp:5010-5015), but the rule's first pass can come before that.
/// It now resolves the target once and skips a builder that has none.
#[test]
fn the_m0xeron_build_accelerator_skips_a_builder_without_a_target() {
    // The scenario names no definitions of its own, so it runs on the
    // player's startup selection.
    let mut engine = load_installed_scenario_with_selected_definitions(
        "Collection.c4f/Settling.c4f/m0XeronSettlement.c4s",
        0,
        &["Objects.c4d"],
    );
    let owner = join_local_player(&mut engine, "Arriving skeleton");
    let clonk = engine
        .crew_cursor(owner)
        .expect("the joined player has a crew member");
    let arrival = engine.test_object_snapshot(clonk).action;
    assert_eq!(arrival.name, "Build", "the scenario's arrival pose");
    assert_eq!(arrival.target, None);
    let accelerator = object_with_definition(&engine, "BACC").expect("the scenario places BACC");

    call(&mut engine, accelerator, "Accelerating");
}

/// Asks the pack's own `IsGoldAge`, a global function, from script.
const GOLDEN_AGE_PROBE: &str = r#"#strict
public func InGoldenAge(int player)
{
    if (IsGoldAge(player)) return 1;
    return -1;
}
"#;

/// clonk-org/clonk-rs-content#89: `2Teudoburger.c4s` greeted each player with
/// `Golden(iPlayer)`, a function that exists nowhere in the bundle, so every
/// `InitializePlayer` ended in an unknown-function error. The pack's golden
/// age is the `IntGoldAge` effect in `RufDerWipfe.c4d/System.c4g/Golden.c`,
/// started by `StartGoldenAge(owner)`; `Golden` is what is left of the object
/// based version the pack's music script still looks for (`_GLZ`, which no
/// longer ships). The mission now calls the function the pack has.
#[test]
fn a_teudoburger_player_starts_in_a_golden_age() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/Settling.c4f/RufDerWipfeRE.c4f/Kampagne.c4f/2Teudoburger.c4s",
        0,
    );
    let owner = join_local_player(&mut engine, "Teudoburger settler");
    engine
        .register_script_definition("GAPR", "Golden age probe", GOLDEN_AGE_PROBE)
        .expect("the golden age probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("GAPR"))
        .expect("the golden age probe spawns");

    let index = engine.test_object_index(probe);
    let answer = engine
        .call_object_function(index, "InGoldenAge", vec![Value::Int(owner)])
        .expect("the pack's IsGoldAge answers");
    assert_eq!(answer.as_c4_int(), Some(1), "the player's golden age runs");
}

/// clonk-org/clonk-rs-content#85: the Quiz overloads the castle tower `CPT2`
/// with a definition whose only script was `ScriptDE.c`. A definition's script
/// is `Script.c|Script{}.c|C4Script{}.c` tried for each language of the
/// player's `LanguageEx` (C4Components.h:55; C4ComponentHost.cpp:155-186), and
/// neither shipped language file names a fallback, so a US player got a tower
/// with no script at all, and the tower parts that include it then failed on
/// its helpers. The script holds no player-visible text, so it is now the
/// language-neutral `Script.c`.
#[test]
fn the_quiz_tower_has_its_script_in_every_language() {
    // The German player had it all along and must keep it.
    for languages in [["US"], ["DE"]] {
        let mut engine = load_installed_scenario_in_languages(
            "Collection.c4f/Puzzles.c4f/Das_Clonk_Quiz_3_Meister_des_Quiz.c4s",
            0,
            &languages,
        );
        let tower = engine.spawn_test_object(SpawnConfig::new("CPT2"));
        // Asked from script, as the tower parts ask: a function the tower does
        // not have is an error there, where a call from the host is fail-safe.
        engine
            .register_script_definition(
                "QZPR",
                "Quiz probe",
                "#strict\npublic func Ask(object tower) { return tower->FindDrawbridgeUp(); }\n",
            )
            .expect("the probe registers");
        let probe = engine.spawn_test_object(SpawnConfig::new("QZPR"));

        // No drawbridge is attached, so the helper looks for one and answers 0.
        let index = engine.test_object_index(probe);
        let up = engine
            .call_object_function(index, "Ask", vec![Value::Object(tower.as_u64())])
            .unwrap_or_else(|error| panic!("the {languages:?} tower has its helpers: {error}"));
        assert_eq!(up.as_c4_int().unwrap_or(0), 0);
    }
}

/// Kills `target` the way a fight does, so the shipped `Death` runs from script
/// and makes its own typed `GameCallEx`.
const KILL_PROBE: &str = r#"#strict
public func Kill(object target, int killer)
{
    SetKiller(killer, target);
    SetAlive(false, target);
    return target->Death(killer);
}
"#;

/// clonk-org/clonk-rs-content#84: every QuakeR scenario declared
/// `RelaunchPlayer(int iPlr, object pCrew, object pKiller, int iTeam)`, while
/// `QBot::Death` passes `GetKiller(this())`, a player number
/// (`QuakeR.c4d/QBot.c4d/Script.c:390,399`). The typed call was refused, so a
/// player whose last bot died was never relaunched. No scenario reads the
/// argument, so the callbacks take it untyped, and `RelaunchClonk` runs: a
/// replacement `QBOT` waits inside its `TIM2` holder.
#[test]
fn a_quaker_player_is_relaunched_after_their_last_bot_dies() {
    let mut engine =
        load_installed_scenario("Collection.c4f/Hazard.c4f/QuakeR.c4f/Q_DM-Medieval.c4s", 0);
    let victim_owner = join_local_player_on_team(&mut engine, "Victim", 1);
    let killer_owner = join_local_player_on_team(&mut engine, "Killer", 2);
    let victim = engine
        .crew_cursor(victim_owner)
        .expect("the victim has a bot");
    engine
        .register_script_definition("KILP", "Kill probe", KILL_PROBE)
        .expect("the kill probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("KILP"))
        .expect("the kill probe spawns");

    let index = engine.test_object_index(probe);
    engine
        .call_object_function(
            index,
            "Kill",
            vec![Value::Object(victim.as_u64()), Value::Int(killer_owner)],
        )
        .expect("the shipped QBot death chain completes");

    let snapshot = engine.snapshot();
    let relaunched = snapshot.objects.iter().any(|object| {
        object.definition_id == "QBOT"
            && object.id != victim
            && object.owner == victim_owner
            && object.container.is_some_and(|holder| {
                snapshot
                    .objects
                    .iter()
                    .any(|other| other.id == holder && other.definition_id == "TIM2")
            })
    });
    assert!(
        relaunched,
        "the victim's replacement bot must wait inside its TIM2 holder"
    );
}

/// clonk-org/clonk-rs-content#87: DieNeueWelt restores its Storeframework
/// `#55662` holding three pieces of smoked meat but with `LocalNamed=0`. The
/// rack makes its `iMeat` and `iTime` arrays in `Initialize`, which a restored
/// object never runs, so its `CheckMeat` timer indexed nil, an error under the
/// pinned engine too (C4AulExec.cpp:906-913). The restored object now carries
/// the arrays, with the meat it holds in its first three slots.
#[test]
fn die_neue_welts_meat_rack_checks_the_meat_it_holds() {
    let mut engine = load_installed_scenario("Collection.c4f/Settling.c4f/DieNeueWelt.c4s", 0);
    let rack = object_with_definition(&engine, "STFW").expect("the scenario restores a rack");

    call(&mut engine, rack, "CheckMeat");
}

/// clonk-org/clonk-rs-content#80: Adventure and the twelve playable
/// MissionsHarkon missions name `MetalMagic.c4f\Misc.c4d`, which the import
/// keeps only at `Collection.c4f/Knights.c4f/MetalMagic.c4f/Misc.c4d`.
/// C4GameResList::Load opens each definition name against the data root and
/// fails the start with IDS_PRC_DEFNOTFOUND when it is not there
/// (C4GameParameters.cpp:199-207), so none of them could start. They now name
/// the path the definitions are at.
#[test]
fn the_metal_magic_adventures_load_their_misc_definitions() {
    // The packed Adventure and one unpacked mission from each chapter; all
    // twelve missions carry the same line.
    for path in [
        "Collection.c4f/Adventures.c4f/Adventure.c4s",
        "Collection.c4f/Adventures.c4f/MissionsHarkon.c4f/Mission1A.c4s",
        "Collection.c4f/Adventures.c4f/MissionsHarkon.c4f/Mission2G.c4s",
    ] {
        let engine = load_installed_scenario(path, 0);
        // `_BRL`, the barrel, is one of Misc.c4d's definitions.
        assert!(engine.definition("_BRL").is_some(), "{path} loads Misc.c4d");
    }
}

/// Runs one step of the scenario's script counter and answers where the step
/// left the counter.
const SCRIPT_STEP_PROBE: &str = r#"#strict
public func CounterAfter(string step)
{
    GameCall(step);
    return ScriptCounter();
}
"#;

/// clonk-org/clonk-rs-content#67: the RufDerWipfeRE tutorial declared
/// `Script410` twice. C4Aul binds the one declared last (C4Aul.cpp:562-576
/// finds the function added last first), so the hut the player is told to
/// build was never checked, and the crew check's wait, `goto(418)`, ran
/// `Script420` again and let the counter run on to `Script990`, which fulfils
/// the goal unaided. The crew check is `Script430` now and waits on itself,
/// as every other check in the file does.
#[test]
fn the_rufderwipfe_tutorial_waits_for_the_hut_and_then_for_the_crew() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/Settling.c4f/RufDerWipfeRE.c4f/Tutorial.c4s",
        0,
    );
    let _pupil = join_local_player(&mut engine, "Tutorial pupil");
    engine
        .register_script_definition("STPR", "Script step probe", SCRIPT_STEP_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("STPR"));
    let index = engine.test_object_index(probe);
    let mut counter_after = |step: &str| {
        engine
            .call_object_function(index, "CounterAfter", vec![step.into()])
            .unwrap_or_else(|error| panic!("{step} runs: {error}"))
            .as_c4_int()
    };

    // No hut is built, so the hut check waits on itself.
    assert_eq!(counter_after("Script410"), Some(408));
    // A new player has fewer than three clonks, so the crew check waits too.
    assert_eq!(counter_after("Script430"), Some(428));
}

fn knife_packs(engine: &Engine) -> usize {
    engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "KNFP" && object.status != ObjectStatus::Deleted)
        .count()
}

/// clonk-org/clonk-rs-content#59: both Faffnir missions hand the assassin a
/// knife pack, `KNFP`, and clear the previous kit on each relaunch with
/// `RemoveAll(KNPF)`. `KNPF` names no definition, which is no error in C4Aul
/// (an id is just a constant), so every other item was cleared and the knife
/// packs piled up. The cleanup names `KNFP` now.
#[test]
fn a_faffnir_relaunch_leaves_one_knife_pack() {
    // The assassin is team 1. The second mission relaunches only player 1, so
    // a Kanderianer (team 7) joins first; it also places a knife pack of its
    // own, which the cleanup takes too.
    for (path, teams) in [
        (
            "Collection.c4f/Adventures.c4f/Faffnir.c4f/faffnir_1.c4s",
            &[1][..],
        ),
        (
            "Collection.c4f/Adventures.c4f/Faffnir.c4f/Faffnir_2.c4s",
            &[7, 1][..],
        ),
    ] {
        let mut engine = load_installed_scenario(path, 0);
        let players = teams
            .iter()
            .map(|&team| join_local_player_on_team(&mut engine, format!("Team {team}"), team))
            .collect::<Vec<_>>();
        let assassin = *players.last().expect("a player joined");

        engine
            .call_scenario_script_function("RelaunchPlayer", vec![Value::Int(assassin), Value::Nil])
            .unwrap_or_else(|error| panic!("{path} relaunches: {error}"));

        assert_eq!(knife_packs(&engine), 1, "{path}: only the new kit's pack");
    }
}

/// clonk-org/clonk-rs-content#59: Der goldene Wipf 2's `RelaunchPlayer` makes
/// the replacement Wipf, then heals `obj`, a name the function never declares.
/// Under `#strict` C4Aul refuses that identifier at link time
/// (C4AulParse.cpp:2866), so every relaunch ended in an error there. It heals
/// `clnk`, the Wipf it just made, now.
#[test]
fn a_golden_wipf_relaunch_runs_to_its_end() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/Adventures.c4f/DerGoldeneWipfMutli.c4f/Der goldene Wipf2multi.c4s",
        0,
    );
    let player = join_local_player(&mut engine, "Wipf keeper");

    engine
        .call_scenario_script_function("RelaunchPlayer", vec![Value::Int(player)])
        .unwrap_or_else(|error| panic!("the relaunch completes: {error}"));
}

/// Relaunches a player the way the pack's Clonk does from `Destruction`.
const RELAUNCH_PROBE: &str = r#"#strict
public func Relaunch(int player)
{
    return GameCallEx("RelaunchPlayer", player);
}
"#;

/// clonk-org/clonk-rs-content#60: WarStars relaunches through
/// `GameCallEx("RelaunchPlayer", ..)` from its Clonk's `Destruction`, but
/// declared `RelaunchPlayer` as a `global func`, which C4Aul gives the engine
/// rather than the scenario script, so the scenario-script call did not reach
/// it (C4AulParse.cpp:1739-1746). It also stored each player's home ship in
/// `Global(player + 1000)` and read `Global(player)`, which is never set, and
/// returned there. A player whose last Clonk died got no new one. It is a
/// scenario function now and reads the slot the ship is in.
#[test]
fn a_war_stars_player_is_relaunched_at_their_ship() {
    let mut engine = load_installed_scenario("Collection.c4f/Fun.c4f/WarStars1_1.c4s", 0);
    let player = join_local_player_on_team(&mut engine, "War star", 1);
    // The ship is handed to the Clonk by schedules a few frames in.
    for _ in 0..10 {
        let _ = engine.tick_without_snapshot();
    }
    engine
        .register_script_definition("WSPR", "Relaunch probe", RELAUNCH_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("WSPR"));
    let crew_before = engine.crew_members(player).len();

    let index = engine.test_object_index(probe);
    engine
        .call_object_function(index, "Relaunch", vec![Value::Int(player)])
        .expect("the relaunch runs");

    assert_eq!(engine.crew_members(player).len(), crew_before + 1);
}

/// clonk-org/clonk-rs-content#61: IceCorpse's `Script1` closed itself with a
/// stray `}` halfway through filling the coach, so the bows, arrows, coke and
/// flints after it sat outside any function, where C4Aul refuses statements,
/// and the coach never got them. The brace is gone.
#[test]
fn the_ice_corpse_coach_gets_its_bows_and_arrows() {
    let mut engine = load_installed_scenario("Collection.c4f/Knights.c4f/(G)IceCorpse.c4s", 0);

    engine
        .call_scenario_script_function("Script1", Vec::new())
        .expect("Script1 runs");

    let coach = object_with_definition(&engine, "COAC").expect("Script1 places the coach");
    for item in ["BOW1", "ARWP"] {
        assert!(
            object_contents_count(&engine, coach, item) > 0,
            "the coach holds {item}"
        );
    }
}

/// Gives each player the wealth the round's scoring left them.
const WEALTH_PROBE: &str = r#"#strict
public func Give(int player, int wealth)
{
    return SetWealth(player, wealth);
}
"#;

/// clonk-org/clonk-rs-content#61: Arenafight137's `Sieger` finds the richest
/// player and was meant to eliminate the others before `GameOver`, but its loop
/// was `while(i > 10 && i != iMax)` from `i = 0`, which never runs, so every
/// player won. It eliminates everyone but the richest now.
#[test]
fn the_arenafight_ends_with_only_the_richest_player_standing() {
    let mut engine = load_installed_scenario("Collection.c4f/Knights.c4f/Arenafight137.c4s", 0);
    let players = [1, 2, 1]
        .map(|team| join_local_player_on_team(&mut engine, format!("Fighter on {team}"), team));
    engine
        .register_script_definition("WLPR", "Wealth probe", WEALTH_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("WLPR"));
    let index = engine.test_object_index(probe);
    for (player, wealth) in players.iter().zip([30, 50, 10]) {
        engine
            .call_object_function(index, "Give", vec![Value::Int(*player), Value::Int(wealth)])
            .expect("the wealth is set");
    }

    engine
        .call_scenario_script_function("Sieger", Vec::new())
        .expect("Sieger runs");

    let eliminated = players.map(|player| engine.is_owner_eliminated(player));
    assert_eq!(eliminated, [true, false, true]);
}

/// clonk-org/clonk-rs-content#61: Rebellion's Bridge gives every player on
/// each side a knight, four placed and more for a larger team, but the right
/// side counted its extra players with `GetTeamPlayerCount(Position_Left)`, so
/// a right side of more than four players left some without a knight. It
/// counts the right side now.
#[test]
fn every_bridge_player_on_the_larger_side_gets_a_knight() {
    let mut engine =
        load_installed_scenario("Collection.c4f/Knights.c4f/Rebellion.c4f/Bridge.c4s", 0);
    let _left = join_local_player_on_team(&mut engine, "Left", 1);
    let right = (0..6)
        .map(|n| join_local_player_on_team(&mut engine, format!("Right {n}"), 2))
        .collect::<Vec<_>>();

    engine
        .call_scenario_script_function("Script1", Vec::new())
        .expect("Script1 runs");

    let without_a_knight = right
        .iter()
        .filter(|player| {
            !engine.crew_members(**player).iter().any(|crew| {
                engine
                    .object_snapshot(*crew)
                    .is_some_and(|crew| crew.definition_id == "KNIG")
            })
        })
        .count();
    assert_eq!(without_a_knight, 0);
}

/// Calls one scenario function from script and answers its result. `GameCall`
/// reaches private scenario functions, as the engine's own calls do.
const SCENARIO_CALL_PROBE: &str = r#"#strict
public func Ask(string function, a, b)
{
    return GameCall(function, a, b);
}
"#;

fn ask_scenario(engine: &mut Engine, function: &str, args: [Value; 2]) -> Value {
    if object_with_definition(engine, "SCPR").is_none() {
        engine
            .register_script_definition("SCPR", "Scenario call probe", SCENARIO_CALL_PROBE)
            .expect("the probe registers");
        engine.spawn_test_object(SpawnConfig::new("SCPR"));
    }
    let probe = object_with_definition(engine, "SCPR").expect("the probe exists");
    let index = engine.test_object_index(probe);
    let [a, b] = args;
    engine
        .call_object_function(index, "Ask", vec![function.into(), a, b])
        .unwrap_or_else(|error| panic!("{function} runs: {error}"))
}

/// clonk-org/clonk-rs-content#63: DepthCharge scores each sub's death to its
/// killer with `Global(iKiller)++` before it looks at who the killer was, so a
/// death by nature (`iKiller == -1`) scored too. A negative index is slot 0
/// (C4ValueList::GetItem, C4ValueList.cpp:50-52), which is the first player's
/// score, so every death by nature gave them a point and could win them the
/// round. A death by nature is only reported now.
#[test]
fn a_depth_charge_death_by_nature_scores_nobody() {
    let mut engine = load_installed_scenario("Collection.c4f/Melees.c4f/DepthCharge.c4s", 0);
    let first = join_local_player(&mut engine, "First diver");
    let second = join_local_player(&mut engine, "Second diver");
    engine
        .register_script_definition(
            "GLPR",
            "Score probe",
            "#strict\npublic func ScoreOf(int player) { return Global(player); }\n",
        )
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("GLPR"));
    let index = engine.test_object_index(probe);
    let first_score = |engine: &mut Engine| {
        engine
            .call_object_function(index, "ScoreOf", vec![Value::Int(first)])
            .expect("the score reads")
            .as_c4_int()
            .unwrap_or(0)
    };
    let before = first_score(&mut engine);

    engine
        .call_scenario_script_function("OnSubDeath", vec![Value::Int(second), Value::Int(-1)])
        .expect("a death by nature is reported");

    assert_eq!(first_score(&mut engine), before);
}

/// Lights a Clonk as JungleFlurry does and reads its view range back.
const LIGHT_PROBE: &str = r#"#strict
public func Light(object clonk)
{
    MakeLight(clonk);
    return GetObjectVal("PlrViewRange", 0, clonk);
}
"#;

/// clonk-org/clonk-rs-content#63: JungleFlurry's `MakeLight` walks the players
/// to open a Clonk's view for everyone but its owner, but read
/// `GetPlayerByIndex(plr)` with the unset `plr` instead of the loop's `cnt`, so
/// it looked at the first player every time and the first player's Clonks were
/// never opened up. It reads `GetPlayerByIndex(cnt)` now.
#[test]
fn a_jungle_flurry_light_opens_the_first_players_clonk_too() {
    let mut engine = load_installed_scenario("Collection.c4f/Melees.c4f/JungleFlurry.c4s", 0);
    let first = join_local_player(&mut engine, "First");
    let _second = join_local_player(&mut engine, "Second");
    let clonk = engine
        .crew_cursor(first)
        .expect("the first player has a Clonk");
    engine
        .register_script_definition("LTPR", "Light probe", LIGHT_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("LTPR"));
    let index = engine.test_object_index(probe);

    let range = engine
        .call_object_function(index, "Light", vec![Value::Object(clonk.as_u64())])
        .expect("the light is made");
    // `SetPlrViewRange(100)` without `fExact` rounds up to 128
    // (C4Script.cpp:3686); the Clonk's own range was 500.
    assert_eq!(range.as_c4_int(), Some(128));
}

/// clonk-org/clonk-rs-content#63: MoonmapRejoin picks each spawn from
/// `[STFN, SFLN, EFLN]` at `Random(1 + Min(g_iSpawnCount/5, GetLength(id)))`,
/// which reaches index 3 once fifteen rounds have spawned, and a nil id spawns
/// nothing. The bound is `GetLength(id) - 1` now.
#[test]
fn a_late_moonmap_spawn_is_always_a_flint() {
    let mut engine = load_installed_scenario("Collection.c4f/Melees.c4f/MoonmapRejoin.c4s", 0);
    engine
        .register_script_definition(
            "MMPR",
            "Spawn count probe",
            "#strict\npublic func Late() { g_iSpawnCount = 100; return 1; }\n",
        )
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("MMPR"));
    let index = engine.test_object_index(probe);
    engine
        .call_object_function(index, "Late", Vec::new())
        .expect("the count is set");

    let nil_spawns = (0..40)
        .filter(|_| {
            ask_scenario(&mut engine, "getSpawnType", [Value::Nil, Value::Nil]) == Value::Nil
        })
        .count();
    assert_eq!(nil_spawns, 0);
}

/// clonk-org/clonk-rs-content#63: TemplePushing puts team rows at their team
/// number and player rows after `MaxTeamCount` of them, but `MaxTeamCount` was
/// 3 while `Teams.txt` defines four teams, so the fourth team's row was the
/// first player's. It is 4 now.
#[test]
fn temple_pushing_keeps_the_fourth_team_row_apart_from_the_first_player() {
    let mut engine = load_installed_scenario("Collection.c4f/Melees.c4f/TemplePushing.c4s", 0);
    let player = join_local_player_on_team(&mut engine, "Pusher", 1);

    let team_row = ask_scenario(&mut engine, "TeamRow", [Value::Int(4), Value::Nil]);
    let player_row = ask_scenario(&mut engine, "PlayerRow", [Value::Int(player), Value::Nil]);
    assert_ne!(team_row, player_row);
}

/// Kills `victim` for `killer` the way a fight does, so the scenario's death
/// callback runs.
const DEATH_PROBE: &str = r#"#strict
public func KillFor(object victim, int killer)
{
    SetKiller(killer, victim);
    SetAlive(false, victim);
    GameCallEx("OnClonkDeath", victim, killer);
    return aKillsForTeam[GetPlayerTeam(killer) - 1];
}
"#;

/// clonk-org/clonk-rs-content#63: Z4 Battle of Generations is won by the first
/// team to ten kills, a point per enemy killed and one off for a team kill or
/// a suicide, as its description says. Its `OnClonkDeath` announced each
/// death but never counted it, so no team ever scored. It keeps the tallies
/// now.
#[test]
fn a_battle_of_generations_kill_scores_for_the_killers_team() {
    let mut engine =
        load_installed_scenario("Collection.c4f/Melees.c4f/Z4BattleOfGenerations.c4s", 0);
    let killer = join_local_player_on_team(&mut engine, "Left", 1);
    let victim_owner = join_local_player_on_team(&mut engine, "Right", 2);
    let victim = engine
        .crew_cursor(victim_owner)
        .expect("the victim has a Clonk");
    engine
        .register_script_definition("DTPR", "Death probe", DEATH_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("DTPR"));
    let index = engine.test_object_index(probe);

    let score = engine
        .call_object_function(
            index,
            "KillFor",
            vec![Value::Object(victim.as_u64()), Value::Int(killer)],
        )
        .expect("the death is handled");
    assert_eq!(score.as_c4_int(), Some(1));
}

/// clonk-org/clonk-rs-content#64: CMC_Train's `Initialize` called
/// `CreateEquipment()` twice, so every ammunition crate and the weapon vending
/// machine stood twice on the same spot. It is called once now.
#[test]
fn the_cmc_train_places_one_vending_machine() {
    let engine = load_installed_scenario("Collection.c4f/ModernCombat.c4f/CMC_Train.c4s", 0);

    let machines = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| object.definition_id == "WPVM")
        .count();
    assert_eq!(machines, 1);
}

/// clonk-org/clonk-rs-content#64: three of CMC_Train's four animated screens
/// were placed with a comma missing before the owner, as in
/// `CreateObject (SCA1, 698, 435 -1)`, which passes `y - 1` and no owner.
/// FnCreateObject from the scenario script keeps that owner as 0
/// (C4Script.cpp:1886-1897), so the first player owned them. They are placed
/// without an owner now, as the fourth is.
#[test]
fn the_cmc_train_screens_belong_to_no_player() {
    let engine = load_installed_scenario("Collection.c4f/ModernCombat.c4f/CMC_Train.c4s", 0);

    let owners = engine
        .snapshot()
        .objects
        .iter()
        .filter(|object| matches!(object.definition_id.as_str(), "SCA1" | "SCA2"))
        .map(|object| object.owner)
        .collect::<Vec<_>>();
    assert_eq!(owners, [-1; 4]);
}

/// clonk-org/clonk-rs-content#65: the Quiz declared the label `Script340` twice.
/// C4Aul binds the later one, so the first, which announces the first question
/// and places a light between `Script327` and `Script335`, never ran. It is
/// `Script330` now.
#[test]
fn the_quiz_announces_its_first_question_at_step_330() {
    let mut engine = load_installed_scenario(
        "Collection.c4f/Puzzles.c4f/Das_Clonk_Quiz_3_Meister_des_Quiz.c4s",
        0,
    );

    let answer = ask_scenario(&mut engine, "Script330", [Value::Nil, Value::Nil]);
    assert_eq!(answer.as_c4_int(), Some(1), "the step exists and runs");
}

/// clonk-org/clonk-rs-content#65: TempleEscape's `MakeLight` is JungleFlurry's
/// and read `GetPlayerByIndex(plr)` with the unset `plr` as well, so the first
/// player's Clonk kept its own view range. It reads `GetPlayerByIndex(cnt)`.
#[test]
fn a_temple_escape_light_opens_the_first_players_clonk_too() {
    let mut engine =
        load_installed_scenario("Collection.c4f/Puzzles.c4f/TempleEscape_League.c4s", 0);
    let first = join_local_player(&mut engine, "First");
    let _second = join_local_player(&mut engine, "Second");
    let clonk = *engine
        .crew_members(first)
        .first()
        .expect("the first player has a Clonk");
    engine
        .register_script_definition("LTPR", "Light probe", LIGHT_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("LTPR"));
    let index = engine.test_object_index(probe);

    let range = engine
        .call_object_function(index, "Light", vec![Value::Object(clonk.as_u64())])
        .expect("the light is made");
    assert_eq!(range.as_c4_int(), Some(128));
}

/// clonk-org/clonk-rs-content#65: three Puzzles scenarios were saved against an
/// older Metal & Magic and place its sainthood, the sainthood's helper and the
/// Cra-Kla-Zoth staff as `HLTM`, `HTMH` and `SM8W`. Metal & Magic 3.1b ships
/// them as `SNHD`, `SNHP` and `SCKZ`, and an object whose id names no
/// definition is skipped on load (C4Object::CompileFunc), so the scenarios had
/// none of them. They name the current ids now.
#[test]
fn the_puzzles_sainthoods_and_staffs_load() {
    for (path, expected) in [
        (
            "Collection.c4f/Puzzles.c4f/1_S2Curious.c4s",
            &[("SNHD", 1), ("SNHP", 2)][..],
        ),
        (
            "Collection.c4f/Puzzles.c4f/2_MountVikto.c4s",
            &[("SNHD", 1), ("SNHP", 1)][..],
        ),
        (
            "Collection.c4f/Puzzles.c4f/4_TowerOfMagic.c4s",
            &[("SCKZ", 3)][..],
        ),
    ] {
        let engine = load_installed_scenario(path, 0);
        let snapshot = engine.snapshot();
        for (id, count) in expected {
            let found = snapshot
                .objects
                .iter()
                .filter(|object| object.definition_id == *id)
                .count();
            assert_eq!(found, *count, "{path}: {id}");
        }
    }
}

/// clonk-org/clonk-rs-content#66: tschudispecial declared `InitializePlayer`
/// twice. C4Aul binds the one declared last (C4Aul.cpp:562-576), which keeps
/// the player's Clonk on its side, so the first, which starts the scenario
/// script with `ScriptGo(1)`, never ran and the story in `Script20` to
/// `Script50` was never told. The one `InitializePlayer` does both now.
#[test]
fn the_last_will_tells_its_story_once_a_player_joins() {
    let mut engine = load_installed_scenario("Collection.c4f/Races.c4f/tschudispecial.c4s", 0);
    let player = join_local_player_on_team(&mut engine, "Heir", 1);
    // C4GameScriptHost::Execute runs one step every tenth frame while
    // `ScriptGo` holds (C4ScriptHost.cpp:222-232).
    for _ in 0..30 {
        let _ = engine.tick_without_snapshot();
    }

    assert!(engine.scenario_script_counter() > 0, "the script runs");
    let clonk = *engine
        .crew_members(player)
        .first()
        .expect("the player has a Clonk");
    let clonk = engine.object_snapshot(clonk).expect("the Clonk exists");
    assert!(
        clonk
            .effects
            .iter()
            .any(|effect| effect.name == "StayOnSide"),
        "the Clonk is still kept on its side"
    );
}

/// clonk-org/clonk-rs-content#66: RopeRace offers to run again through
/// `SetNextMission("RopepackRemake.c4f\\RopeRace.c4s", ..)`, the path it had in
/// its own pack. C4Application::QuitGame starts the next mission by that path
/// from the data root (C4Application.cpp:389-398), and the import keeps the
/// race at `Collection.c4f\Races.c4f\RopeRace.c4s`, so there was nothing to
/// run again. It names that path now.
#[test]
fn the_rope_race_runs_again_from_where_it_is_installed() {
    let engine = load_installed_scenario("Collection.c4f/Races.c4f/RopeRace.c4s", 0);

    let next = engine.next_mission().path.replace('\\', "/");
    assert!(content_root().join(&next).is_file(), "{next} is installed");
}

/// clonk-org/clonk-rs-content#66, reported as clonk-org/clonk-rs#1471: both
/// Abwärts races make each Clonk an Aquaclonk, for the contact calls their
/// fall damage needs, and then drew it with the Clonk's sheet,
/// `SetGraphics(0, this(), CLNK)`, or with the scenario's own `_INE` sheet for
/// a player whose name ends in "ine". C4Object::UpdateActionFace cuts the live
/// action's facet out of whichever sheet is selected (C4Object.cpp:4202-4217),
/// and the Aquaclonk walks in cells 20 wide where the Clonk sheet's are 16 wide
/// and `_INE`'s whole sheet is laid out in 16 by 20 cells, so a walking Clonk
/// slid out of its cell and vanished. Each keeps the Aquaclonk's own sheet now.
#[test]
fn an_abwaerts_clonk_is_drawn_from_the_sheet_its_actions_are_cut_for() {
    for path in [
        "Collection.c4f/Races.c4f/AbwaertsExtremTeam2.c4s",
        "Collection.c4f/Races.c4f/AbwaertsFalschrum.c4s",
    ] {
        let mut engine = load_installed_scenario(path, 0);
        // The races generate a team for each player.
        for (team, name) in [(1, "Faller"), (2, "Clonkine")] {
            let player = join_local_player_on_team(&mut engine, name, team);
            let clonk = *engine
                .crew_members(player)
                .first()
                .expect("the player has a Clonk");
            let clonk = engine.object_snapshot(clonk).expect("the Clonk exists");
            assert_eq!(
                clonk.definition_id, "ACLK",
                "{path}: {name} keeps its contact calls"
            );
            assert_eq!(
                clonk.base_graphics, None,
                "{path}: {name} is drawn as an Aquaclonk"
            );
        }
    }
}
