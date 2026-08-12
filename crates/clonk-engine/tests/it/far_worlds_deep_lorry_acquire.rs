use crate::support::real_scenario::PreparedInstalledScenario;
use clonk_engine::{ocf, Landscape, LiquidSegment, SpawnConfig, Vector2};
use clonk_script::Value;

pub(super) fn deep_hydroclonk_finds_coral_inside_a_submerged_lorry(
    prepared: &PreparedInstalledScenario,
) {
    // HCLK::GetAvailableObject has a second search specifically for objects
    // hidden inside submerged containers. C++ accepts the candidate only when
    // its container's DefCore GrabPutGet contains C4D_GrabGet (or the
    // container has OCF_Entrance)
    // (FarWorlds.c4d/Deep.c4d/Crew.c4d/HydroClonk.c4d/Script.c:43-71;
    // src/C4Script.cpp:4170-4180). LORY is GrabGet|GrabPut and has no entrance.
    let mut engine = prepared.instantiate();
    assert_eq!(
        engine.debug_definition_has_function("HCLK", "GetAvailableObject"),
        Some(true),
        "the shipped Hydroclonk acquisition override is loaded"
    );
    assert_eq!(
        engine.definition_grab_put_get("LORY"),
        3,
        "the shipped lorry carries C4D_GrabPut|C4D_GrabGet"
    );

    // Keep the objects more than eight pixels below the water surface so the
    // contained coral is not OCF_Available. This forces the shipped function's
    // AnyContainer/OCF_FullCon fallback instead of its ordinary first search.
    let mut water = Landscape::flat(201, 120);
    for x in 0..201 {
        water.set_liquid_column(x, vec![LiquidSegment::new(20, 90)]);
    }
    engine.set_landscape(water);

    let position = Vector2::new(100, 60);
    let hydroclonk = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("HCLK").with_position(position)),
    );
    let lorry = crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("LORY").with_position(position)),
    );
    let coral = crate::support::TestValueExt::test_value(
        engine.spawn_object(
            SpawnConfig::new("GCOR")
                .with_position(position)
                .with_container(lorry),
        ),
    );

    let coral_index = crate::support::TestValueExt::test_value(engine.find_object_index(coral));
    let coral_ocf = engine.object_ocf_at_index(coral_index);
    assert_ne!(coral_ocf & ocf::FULL_CON, 0);
    assert_eq!(
        coral_ocf & (ocf::AVAILABLE | ocf::IN_LIQUID),
        0,
        "deeply submerged contained coral must reach HCLK's container fallback"
    );

    let hydroclonk_index =
        crate::support::TestValueExt::test_value(engine.find_object_index(hydroclonk));
    let found = crate::support::TestValueExt::test_value(engine.call_object_function(
        hydroclonk_index,
        "GetAvailableObject",
        vec![Value::C4Id("GCOR".into()), Value::Int(0)],
    ));

    assert_eq!(
        found,
        Value::Object(coral.as_u64()),
        "LORY's reflected C4D_GrabGet bit makes its exact GCOR accessible"
    );
}
