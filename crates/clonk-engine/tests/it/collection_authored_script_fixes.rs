//! Authored script defects in the imported Collection that the content
//! repository corrects (clonk-org/clonk-rs#1395). Each was reproduced under the
//! pinned C++ engine too, so these are content fixes and not parity gaps: the
//! tests pin that the shipped scripts now run, against the real content.

use crate::support::real_scenario::{
    join_local_player, join_local_player_on_team, load_installed_scenario,
    load_installed_scenario_in_languages, load_installed_scenario_with_selected_definitions,
    object_with_definition,
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
