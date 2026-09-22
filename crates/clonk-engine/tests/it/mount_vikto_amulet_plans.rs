//! Mount Vikto's Guapa grants the five amulet plans in one branch of her
//! dialogue (`System.c4g/DlgGuapa.c:134-138`) and the ShamanTipi `NTIP` in
//! another (`:119-126`). The amulets are not Indian handcraft, so neither the
//! Indian nor the Mallet offers them; the ShamanTipi does, listing the owner's
//! known `C4D_Object` plans that answer `IsIndianAmulet`
//! (`Western.c4d/Structures.c4d/Camp.c4d/ShamanTipi.c4d/Script.c:12-24`).
//! clonk-org/clonk-rs-content#98 traced a report that the amulets could not be
//! made to this prerequisite; this pins that the path works once it is met.

use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use crate::support::EngineTestExt;
use clonk_engine::{Engine, ObjectId, SpawnConfig};
use clonk_script::Value;

const AMULETS: [&str; 5] = ["AMBR", "AMBI", "AMFH", "AMWI", "AMSN"];

/// Guapa's `DlgGuapa25`, the amulet branch, as script runs it.
const GRANT_PROBE: &str = r#"#strict
public func GrantAmulets(int player)
{
    SetPlrKnowledge(player, AMBR);
    SetPlrKnowledge(player, AMBI);
    SetPlrKnowledge(player, AMFH);
    SetPlrKnowledge(player, AMWI);
    SetPlrKnowledge(player, AMSN);
    return 1;
}
"#;

fn call(engine: &mut Engine, object: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = engine.test_object_index(object);
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function} executes: {error}"))
}

#[test]
fn a_shaman_tipi_offers_the_amulet_plans_guapa_grants() {
    let mut engine = load_installed_scenario("Collection.c4f/Puzzles.c4f/2_MountVikto.c4s", 0);
    let player = join_local_player(&mut engine, "Amulet maker");
    let clonk = engine
        .crew_cursor(player)
        .expect("the joined player has a crew member");
    engine
        .register_script_definition("AMPR", "Amulet grant probe", GRANT_PROBE)
        .expect("the probe registers");
    let probe = engine.spawn_test_object(SpawnConfig::new("AMPR"));
    call(&mut engine, probe, "GrantAmulets", vec![Value::Int(player)]);
    let tipi = engine.spawn_test_object(SpawnConfig::new("NTIP").with_owner(player));

    call(
        &mut engine,
        tipi,
        "MenuProduction",
        vec![Value::Object(clonk.as_u64())],
    );

    let offered = engine
        .debug_object_menu(clonk.as_u64())
        .flatten()
        .map(|menu| {
            menu.items
                .into_iter()
                .map(|item| item.item_id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for amulet in AMULETS {
        assert!(
            offered.iter().any(|id| id == amulet),
            "{amulet} is in the production menu: {offered:?}"
        );
    }
}
