//! Shipped wagon contents access (`Objects.c4d/Vehicles.c4d/Wagon.c4d`).
//!
//! `WAGN` and `LORY` are the same mechanics class — a `Grab=1`, `C4D_Vehicle`
//! cart a Clonk pushes — and the shipped Hydroclonk's `GetAvailableObject`
//! reaches the cargo of both. It reaches them through DIFFERENT DefCore
//! properties, which is what this pins: the shipped function accepts a
//! candidate inside a submerged container when the container's `GrabPutGet`
//! carries `C4D_GrabGet` **or** the container has `OCF_Entrance`
//! (`FarWorlds.c4d/Deep.c4d/Crew.c4d/HydroClonk.c4d/Script.c:43-71`;
//! `src/C4Script.cpp:4170-4180`).
//!
//! `Lorry.c4d` declares `GrabPutGet=C4D_GrabGet|C4D_GrabPut` and no entrance —
//! that arm is pinned in `far_worlds_deep_lorry_acquire`. `Wagon.c4d` declares
//! the reverse: no `GrabPutGet` at all, and `Entrance=-11,-6,20,20`. So a port
//! that implemented only the GrabGet arm passes the lorry case and fails here.

use crate::support::real_scenario::PreparedInstalledScenario;
use crate::support::TestValueExt;
use clonk_engine::{ocf, Landscape, LiquidSegment, SpawnConfig, Vector2};
use clonk_script::Value;

pub(super) fn deep_hydroclonk_reaches_wagon_cargo_through_its_entrance(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    assert_eq!(
        engine.definition_grab_put_get("WAGN"),
        0,
        "the shipped wagon declares no GrabPutGet (Wagon.c4d/DefCore.txt)"
    );

    // The same submerged fixture the lorry case uses: deeper than eight pixels
    // so the contained coral is not OCF_Available and the shipped function has
    // to fall through to its AnyContainer search.
    let mut water = Landscape::flat(201, 120);
    for x in 0..201 {
        water.set_liquid_column(x, vec![LiquidSegment::new(20, 90)]);
    }
    engine.set_landscape(water);

    let position = Vector2::new(100, 60);
    let hydroclonk = TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("HCLK").with_position(position)),
    );
    let wagon = TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("WAGN").with_position(position)),
    );
    let coral = TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("GCOR")
                .with_position(position)
                .with_container(wagon),
        ),
    );

    let wagon_index = TestValueExt::test_value(engine.find_object_index(wagon));
    assert_ne!(
        engine.object_ocf_at_index(wagon_index) & ocf::ENTRANCE,
        0,
        "the wagon's DefCore Entrance is what makes its cargo reachable"
    );

    let coral_index = TestValueExt::test_value(engine.find_object_index(coral));
    let coral_ocf = engine.object_ocf_at_index(coral_index);
    assert_ne!(coral_ocf & ocf::FULL_CON, 0);
    assert_eq!(
        coral_ocf & (ocf::AVAILABLE | ocf::IN_LIQUID),
        0,
        "deeply submerged contained coral must reach HCLK's container fallback"
    );

    let hydroclonk_index = TestValueExt::test_value(engine.find_object_index(hydroclonk));
    let found = TestValueExt::test_value(engine.call_object_function(
        hydroclonk_index,
        "GetAvailableObject",
        vec![Value::C4Id("GCOR".into()), Value::Int(0)],
    ));

    assert_eq!(
        found,
        Value::Object(coral.as_u64()),
        "the entrance arm reaches the wagon's exact GCOR without any GrabGet bit"
    );
}
