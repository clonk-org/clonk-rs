use lc_engine::{
    Definition, DefinitionRect, Engine, ObjectMenuExtra, ObjectMenuItem, ObjectMenuState,
    ObjectMenuSymbol, ObjectUpdate, SpawnConfig, Vector2,
};
use lc_script::Value;

use crate::support::real_scenario::{join_local_player, load_tutorial};

#[test]
fn tutorial04_enter_all_keeps_only_one_tflint_in_the_real_clonk() {
    // C4ObjectMenu's secondary Contents command requests all three TFLNs
    // (C4ObjectMenu.cpp:300-321). C4Command::GetTryEnter routes each one
    // through Enter; on CLNK::RejectCollect it puts the previous item back
    // into the enclosing HUT2 and retries without consuming the requested
    // count (C4Command.cpp:1092-1126; C4Object.cpp:1566-1591,5853-5891).
    let mut engine = load_tutorial(4, 0);
    let owner = join_local_player(&mut engine, "Get inventory parity");
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial04 joins one real CLNK");
    let hut = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "HUT2")
        .expect("Tutorial04 creates HUT2")
        .id;
    engine
        .apply_object_update(clonk, ObjectUpdate::new().with_container(hut))
        .expect("place the real CLNK inside HUT2");

    let flints = (0..3)
        .map(|_| {
            engine
                .spawn_object(SpawnConfig::new("TFLN").with_container(hut))
                .expect("spawn a real TFLN inside HUT2")
        })
        .collect::<Vec<_>>();
    let command2 = format!(
        "SetCommand(this, \"Get\", , 3,0, Object({}), TFLN) && ExecuteCommand()",
        hut.as_u64()
    );
    let menu = ObjectMenuState {
        caption: "Contents".to_owned(),
        symbol_id: "HUT2".to_owned(),
        title_symbol: ObjectMenuSymbol::Definition,
        identification: Value::Int(18),
        style: 0,
        permanent: true,
        extra: ObjectMenuExtra::None,
        extra_data: 0,
        selection: 0,
        user_menu: false,
        command_object: Some(clonk),
        items: vec![ObjectMenuItem {
            caption: "Get T-Flint".to_owned(),
            info_caption: String::new(),
            command: format!(
                "SetCommand(this, \"Get\", Object({})) && ExecuteCommand()",
                flints[0].as_u64()
            ),
            command2,
            count: 3,
            item_id: "TFLN".to_owned(),
            symbol: ObjectMenuSymbol::Definition,
            picture_object: None,
            components: Vec::new(),
            selectable: true,
            value: None,
        }],
        columns: 5,
        lines: 0,
        text_progress: None,
        decoration: None,
    };
    let mut update = ObjectUpdate::new();
    update.menu = Some(Some(menu));
    engine
        .apply_object_update(clonk, update)
        .expect("install the real Contents command");
    assert!(engine
        .menu_user_enter(clonk, true)
        .expect("COM_MenuEnterAll executes Command2"));

    for _ in 0..30 {
        engine.tick().expect("Get command frame");
    }

    let in_clonk = flints
        .iter()
        .filter(|&&flint| {
            engine
                .object_snapshot(flint)
                .is_some_and(|object| object.container == Some(clonk))
        })
        .count();
    let in_hut = flints
        .iter()
        .filter(|&&flint| {
            engine
                .object_snapshot(flint)
                .is_some_and(|object| object.container == Some(hut))
        })
        .count();
    assert_eq!(in_clonk, 1, "CLNK's MaxContentsCount is one");
    assert_eq!(in_hut, 2, "each rejected replacement returns to HUT2");
}

#[test]
fn ordinary_command_enter_does_not_query_reject_collect() {
    // C4CMD_Enter calls Enter without a pfRejectCollect pointer
    // (C4Command.cpp:600-605), so C4Object::Enter skips the collector's
    // RejectCollect gate entirely (C4Object.cpp:1582-1591).
    let mut engine = Engine::new();
    let mut actor = Definition::from_script(
        "CLNK",
        "Clonk",
        r#"#strict
public func Board(pTarget)
{
  return(SetCommand(this(), "Enter", pTarget));
}
"#,
    )
    .expect("actor definition compiles");
    actor.set_c4_callback_convention(true);
    let mut container = Definition::from_script(
        "HUT2",
        "Hut",
        r#"#strict
protected func RejectCollect(idObject, pObject) { return(1); }
"#,
    )
    .expect("container definition compiles");
    container.set_c4_callback_convention(true);
    container.set_shape_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    container.set_entrance_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    engine
        .register_definition(actor)
        .expect("actor definition registers");
    engine
        .register_definition(container)
        .expect("container definition registers");
    let hut = engine
        .spawn_object(SpawnConfig::new("HUT2").with_position(Vector2::new(100, 120)))
        .expect("HUT2 spawns");
    let mut open = ObjectUpdate::new();
    open.entrance_status = Some(true);
    engine
        .apply_object_update(hut, open)
        .expect("HUT2 entrance opens");
    let clonk = engine
        .spawn_object(SpawnConfig::new("CLNK").with_position(Vector2::new(100, 100)))
        .expect("CLNK spawns");
    let clonk_index = engine.find_object_index(clonk).expect("CLNK exists");
    engine
        .call_object_function(clonk_index, "Board", vec![Value::Object(hut.as_u64())])
        .expect("Board arms C4CMD_Enter");

    engine.tick().expect("C4CMD_Enter frame");

    assert_eq!(
        engine
            .object_snapshot(clonk)
            .expect("CLNK survives")
            .container,
        Some(hut),
        "ordinary Enter must ignore HUT2::RejectCollect"
    );
}
