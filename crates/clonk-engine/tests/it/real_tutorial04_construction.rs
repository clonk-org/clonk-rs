use crate::support::real_scenario::{
    join_local_player_with_preferences, load_raw_content_scenario,
};
use clonk_engine::{Engine, ObjectMenuComponent, ObjectMenuExtra, ObjectUpdate, COM_DIG};
use clonk_script::Value;

fn load_tutorial04() -> (Engine, i32) {
    let scenario = crate::support::TestValueExt::test_value(load_raw_content_scenario(
        "Tutorial.c4f/Tutorial04.c4s",
    ));
    let mut engine = Engine::with_seed(0);
    crate::support::TestValueExt::test_value(scenario.apply(&mut engine));
    let player =
        join_local_player_with_preferences(&mut engine, "Tutorial04 construction", false, false);
    (engine, player)
}

#[test]
fn tutorial04_conkit_opens_the_real_elevator_construction_menu() {
    // CNKT::Activate creates CXCN on its containing clonk and fills it from
    // GetPlrKnowledge (Objects/.../Conkit.c4d/Script.c:5-21). Classic double
    // Dig activates the first carried object (C4ObjectCom.cpp:531-539).
    let (mut engine, player) = load_tutorial04();
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(player));
    let conkit = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "CNKT"),
    )
    .id;

    for _ in 0..160 {
        if engine
            .object_snapshot(clonk)
            .is_some_and(|object| object.container.is_none() && object.action.name == "Walk")
        {
            break;
        }
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(conkit, ObjectUpdate::new().with_container(clonk)),
    );

    crate::support::TestValueExt::test_value(engine.player_in_com(player, COM_DIG, 0));
    crate::support::TestValueExt::test_value(engine.player_in_com(player, COM_DIG, 0));

    let clonk_after_activation =
        crate::support::TestValueExt::test_value(engine.object_snapshot(clonk));
    let conkit_after_activation =
        crate::support::TestValueExt::test_value(engine.object_snapshot(conkit));
    let (_, menu) = engine.cursor_object_menu(player).unwrap_or_else(|| {
        panic!(
            "CNKT activation opens its script menu; clonk={clonk_after_activation:?}; \
             conkit={conkit_after_activation:?}"
        )
    });
    assert_eq!(menu.identification, Value::C4Id("CXCN".to_string()));
    assert_eq!(menu.symbol_id, "CXCN");
    assert_eq!(menu.style, 0, "C4MN_Style_Normal");
    assert!(!menu.permanent);
    assert_eq!(menu.command_object, Some(conkit));
    assert_eq!(menu.columns, 5);
    assert_eq!(
        menu.extra,
        ObjectMenuExtra::Components,
        "CNKT passes C4MN_Extra_Components to CreateMenu (C4Script.cpp:1420-1448)"
    );
    assert_eq!(menu.items.len(), 1, "Tutorial04 knows only ELEV");
    assert_eq!(menu.items[0].item_id, "ELEV");
    assert_eq!(menu.items[0].caption, "Construction: Elevator");
    assert_eq!(
        menu.items[0].info_caption,
        "The elevator will automatically move to pick up waiting clonks, but may also be switched to permanent movement.",
        "AddMenuItem falls back to ELEV's localized description (C4Script.cpp:1590-1594)"
    );
    assert_eq!(
        menu.items[0].components,
        vec![
            ObjectMenuComponent {
                definition_id: "WOOD".to_string(),
                count: 4,
            },
            ObjectMenuComponent {
                definition_id: "METL".to_string(),
                count: 2,
            },
        ],
        "C4MenuItem caches ELEV's ordered components at AddMenuItem time (C4Menu.cpp:92-97; C4Def.cpp:1322-1355)"
    );
    assert_eq!(menu.selection, 0);
}
