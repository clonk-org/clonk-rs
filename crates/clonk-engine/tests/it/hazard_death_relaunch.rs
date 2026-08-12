use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::SpawnConfig;
use clonk_script::Value;

const KILL_PROBE: &str = r#"#strict
public func Kill(object target, int killer)
{
    SetKiller(killer, target);
    SetAlive(false, target);
    return target->Death(killer);
}
"#;

/// A killed HazardClonk runs the shipped `Death` chain: it announces the kill
/// through `HHKS->KTMsg` and reaches `Arena_RelaunchClonk`, which creates the
/// replacement Clonk, creates its TIM2 holder afterwards, and only then calls
/// `pClonk->Enter(tim)` (content Hazard.c4d/Crew.c4d/HazardClonk.c4d
/// Script.c:629-670; Hazard.c4d/System.c4g/Arena.c:74-97). Both objects are
/// live by the time C++ enters them (`src/C4Object.cpp:1560-1620`), so the
/// staged spawn queue must not fail the frame on the container it has not
/// materialized yet.
#[test]
fn hazard_death_relaunch_enters_the_holder_created_after_the_clonk() {
    let mut engine = load_installed_scenario("Hazard.c4f/AH_Predator.c4s", 0);
    let victim_owner = join_local_player(&mut engine, "Victim");
    let killer_owner = join_local_player(&mut engine, "Killer");
    let victim = crate::support::TestValueExt::test_value(engine.crew_cursor(victim_owner));
    crate::support::TestValueExt::test_value(engine.register_script_definition(
        "KILP",
        "Kill probe",
        KILL_PROBE,
    ));
    let probe =
        crate::support::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("KILP")));
    let probe_index = crate::support::TestValueExt::test_value(engine.find_object_index(probe));

    assert_eq!(
        engine
            .call_object_function(
                probe_index,
                "Kill",
                vec![Value::Object(victim.as_u64()), Value::Int(killer_owner)],
            )
            .expect("the shipped HazardClonk death chain completes"),
        Value::Int(1)
    );

    let snapshot = engine.snapshot();
    let holder = crate::support::TestValueExt::test_value(
        snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "TIM2"),
    );
    let relaunched = crate::support::TestValueExt::test_value(
        snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "HZCK" && object.container == Some(holder.id)),
    );
    assert!(
        holder.contents.contains(&relaunched.id),
        "the holder's contents list must carry the relaunched Clonk"
    );
    assert!(
        snapshot
            .hud
            .messages
            .iter()
            .any(|message| message.lines.iter().any(|line| line.contains("Killed"))),
        "Killstats announces the kill: {:?}",
        snapshot
            .hud
            .messages
            .iter()
            .map(|message| message.lines.clone())
            .collect::<Vec<_>>()
    );
}
