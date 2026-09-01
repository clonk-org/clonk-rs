use crate::support::EngineTestExt;
use clonk_engine::command::{CommandId, CommandMode, CommandRequest};
use clonk_engine::{
    Definition, DefinitionRect, Engine, ObjectId, ObjectMenuExtra, ObjectMenuItem, ObjectMenuState,
    ObjectMenuSymbol, ObjectUpdate, SpawnConfig, Vector2, CATEGORY_SELECT_KNOWLEDGE,
    CATEGORY_VEHICLE, FULL_CON,
};
use clonk_script::Value;

use crate::support::real_scenario::{join_local_player, load_tutorial};

fn arm_get(engine: &mut Engine, actor: ObjectId, target: ObjectId) {
    let actor_index = engine.test_object_index(actor);
    assert_eq!(
        engine.call_test_object_function(
            actor_index,
            "StartGet",
            vec![Value::Object(target.as_u64())],
        ),
        Value::Bool(true)
    );
}

fn local_int(engine: &Engine, object: ObjectId, name: &str) -> i32 {
    match engine
        .object_snapshot(object)
        .and_then(|snapshot| snapshot.local_vars.get(name).cloned())
    {
        Some(Value::Int(value)) => value,
        _ => 0,
    }
}

#[test]
fn tutorial04_enter_all_keeps_only_one_tflint_in_the_real_clonk() {
    // C4ObjectMenu's secondary Contents command requests all three TFLNs
    // (C4ObjectMenu.cpp:300-321). C4Command::GetTryEnter routes each one
    // through Enter; on CLNK::RejectCollect it puts the previous item back
    // into the enclosing HUT2 and retries without consuming the requested
    // count (C4Command.cpp:1092-1126; C4Object.cpp:1566-1591,5853-5891).
    let mut engine = load_tutorial(4, 0);
    let owner = join_local_player(&mut engine, "Get inventory parity");
    let clonk = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let hut = crate::support::TestValueExt::test_value(
        engine
            .snapshot()
            .objects
            .into_iter()
            .find(|object| object.definition_id == "HUT2"),
    )
    .id;
    crate::support::TestValueExt::test_value(
        engine.apply_object_update(clonk, ObjectUpdate::new().with_container(hut)),
    );

    let flints = (0..3)
        .map(|_| engine.spawn_test_object(SpawnConfig::new("TFLN").with_container(hut)))
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
        equal_item_height: false,
        permanent: true,
        close_command: clonk_engine::ObjectMenuCloseCommand::None,
        location: None,
        runtime_id: 0,
        extra: ObjectMenuExtra::None,
        extra_data: 0,
        internal_refill_token: 0,
        selection: 0,
        user_menu: false,
        command_object: Some(clonk),
        scenario_callbacks: false,
        refill_object: Some(hut),
        refill_object_contents_count: 3,
        location_reset_generation: 0,
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
            image: clonk_engine::ObjectMenuImage::default(),
            presentation_definition_id: None,
            picture_snapshot: None,
            picture_object: None,
            components: Vec::new(),
            selectable: true,
            value: None,
            text_display_progress: -1,
        }],
        columns: 5,
        lines: 0,
        text_progressing: false,
        decoration: None,
    };
    let mut update = ObjectUpdate::new();
    update.menu = Some(Some(menu));
    crate::support::TestValueExt::test_value(engine.apply_object_update(clonk, update));
    assert!(engine
        .menu_user_enter(clonk, true)
        .expect("COM_MenuEnterAll executes Command2"));

    for _ in 0..30 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
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
    let mut actor = crate::support::TestValueExt::test_value(Definition::from_script(
        "CLNK",
        "Clonk",
        r#"#strict
    public func Board(pTarget)
    {
      return(SetCommand(this(), "Enter", pTarget));
    }
    "#,
    ));
    actor.set_c4_callback_convention(true);
    let mut container = crate::support::TestValueExt::test_value(Definition::from_script(
        "HUT2",
        "Hut",
        r#"#strict
    protected func RejectCollect(idObject, pObject) { return(1); }
    "#,
    ));
    container.set_c4_callback_convention(true);
    container.set_shape_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    container.set_entrance_rect(Some(DefinitionRect::new(-20, -20, 40, 40)));
    engine.register_test_definition(actor);
    engine.register_test_definition(container);
    let hut =
        engine.spawn_test_object(SpawnConfig::new("HUT2").with_position(Vector2::new(100, 120)));
    let mut open = ObjectUpdate::new();
    open.entrance_status = Some(true);
    crate::support::TestValueExt::test_value(engine.apply_object_update(hut, open));
    let clonk =
        engine.spawn_test_object(SpawnConfig::new("CLNK").with_position(Vector2::new(100, 100)));
    let clonk_index = engine.test_object_index(clonk);
    engine.call_test_object_function(clonk_index, "Board", vec![Value::Object(hut.as_u64())]);

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(
        engine.test_object_snapshot(clonk).container,
        Some(hut),
        "ordinary Enter must ignore HUT2::RejectCollect"
    );
}

#[test]
fn get_reject_contents_precedes_the_enter_attempt() {
    let mut engine = Engine::new();
    let mut actor = crate::support::TestValueExt::test_value(Definition::from_script(
        "CLNK",
        "Clonk",
        r#"#strict
    local rejectCollectCount, getCount, dropSelectionCount, finishedCount;
    public func StartGet(pTarget) { return(SetCommand(this(), "Get", pTarget)); }
    protected func ControlCommandFinished(szCommand)
    {
      finishedCount += 1;
    }
    protected func GetObject2Drop(pTarget)
    {
      dropSelectionCount += 1;
      return(Contents(0));
    }
    protected func RejectCollect(idObject, pObject)
    {
      rejectCollectCount += 1;
      return(0);
    }
    protected func Get(pObject)
    {
      getCount += 1;
      return(1);
    }
    "#,
    ));
    actor.set_c4_callback_convention(true);
    actor.set_crew_member(true);
    actor.set_collection_limit(1);
    let mut container = crate::support::TestValueExt::test_value(Definition::from_script(
        "HUT2",
        "Hut",
        r#"#strict
    local rejectContentsCount;
    protected func RejectContents()
    {
      rejectContentsCount += 1;
      return(1);
    }
    "#,
    ));
    container.set_c4_callback_convention(true);
    let mut item = crate::support::TestValueExt::test_value(Definition::from_script(
        "ITEM",
        "Item",
        r#"#strict
    local rejectEntranceCount;
    protected func RejectEntrance(pContainer)
    {
      rejectEntranceCount += 1;
      return(0);
    }
    "#,
    ));
    item.set_c4_callback_convention(true);
    item.set_collectible(true);
    engine.register_test_definition(actor);
    engine.register_test_definition(container);
    engine.register_test_definition(item);

    let hut = engine.spawn_test_object(SpawnConfig::new("HUT2"));
    let clonk = engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_alive(true)
            .with_crew_member(true)
            .with_container(hut),
    );
    let held = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(clonk));
    let item = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut));
    arm_get(&mut engine, clonk, item);

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(local_int(&engine, hut, "rejectContentsCount"), 1);
    assert_eq!(local_int(&engine, item, "rejectEntranceCount"), 0);
    assert_eq!(local_int(&engine, clonk, "rejectCollectCount"), 0);
    assert_eq!(local_int(&engine, clonk, "getCount"), 0);
    assert_eq!(
        local_int(&engine, clonk, "finishedCount"),
        1,
        "the failed Get remains visible to ControlCommandFinished"
    );
    assert_eq!(
        local_int(&engine, clonk, "dropSelectionCount"),
        0,
        "RejectContents precedes the collection-limit PutAway gate"
    );
    assert_eq!(engine.test_object_snapshot(held).container, Some(clonk));
    assert_eq!(engine.test_object_snapshot(item).container, Some(hut));
    assert!(
        engine
            .test_object_snapshot(clonk)
            .command_stack
            .command_names()
            .is_empty(),
        "RejectContents fails the Get command"
    );
}

#[test]
fn get_reject_contents_replacement_does_not_fail_the_new_get() {
    let mut engine = Engine::new();
    let mut actor = crate::support::TestValueExt::test_value(Definition::from_script(
        "CLNK",
        "Clonk",
        r#"#strict
    public func StartGet(pTarget) { return(SetCommand(this(), "Get", pTarget)); }
    "#,
    ));
    actor.set_c4_callback_convention(true);
    let mut container = crate::support::TestValueExt::test_value(Definition::from_script(
        "HUT2",
        "Hut",
        r#"#strict
    local actor, replacement, rejectContentsCount;
    public func Configure(pActor, pReplacement)
    {
      actor = pActor;
      replacement = pReplacement;
      return(1);
    }
    protected func RejectContents()
    {
      rejectContentsCount += 1;
      if (rejectContentsCount == 1)
      {
    SetCommand(actor, "Get", replacement);
    return(1);
      }
      return(0);
    }
    "#,
    ));
    container.set_c4_callback_convention(true);
    let mut item = crate::support::TestValueExt::test_value(Definition::from_script(
        "ITEM",
        "Item",
        "#strict\n",
    ));
    item.set_collectible(true);
    engine.register_test_definition(actor);
    engine.register_test_definition(container);
    engine.register_test_definition(item);

    let hut = engine.spawn_test_object(SpawnConfig::new("HUT2"));
    let clonk = engine.spawn_test_object(SpawnConfig::new("CLNK").with_container(hut));
    let old_target = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut));
    let replacement = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut));
    let hut_index = engine.test_object_index(hut);
    assert_eq!(
        engine.call_test_object_function(
            hut_index,
            "Configure",
            vec![
                Value::Object(clonk.as_u64()),
                Value::Object(replacement.as_u64()),
            ],
        ),
        Value::Int(1)
    );
    arm_get(&mut engine, clonk, old_target);

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(
        engine
            .test_object_snapshot(clonk)
            .command_stack
            .command_names(),
        vec!["Get"],
        "the callback-installed Get must not receive the old attempt's failure"
    );
    assert_eq!(engine.test_object_snapshot(old_target).container, Some(hut));

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        engine.test_object_snapshot(replacement).container,
        Some(clonk),
        "the replacement command remains live and collects its own target"
    );
}

#[test]
fn get_reject_contents_removal_passes_nil_to_put_away() {
    let mut engine = Engine::new();
    let mut actor = crate::support::TestValueExt::test_value(Definition::from_script(
        "CLNK",
        "Clonk",
        r#"#strict
    local nilDropTargetCount, staleDropTargetCount;
    public func StartGet(pTarget) { return(SetCommand(this(), "Get", pTarget)); }
    protected func GetObject2Drop(pTarget)
    {
      if (pTarget) staleDropTargetCount += 1;
      else nilDropTargetCount += 1;
      return(Contents(0));
    }
    "#,
    ));
    actor.set_c4_callback_convention(true);
    actor.set_collection_limit(1);
    let mut container = crate::support::TestValueExt::test_value(Definition::from_script(
        "HUT2",
        "Hut",
        r#"#strict
    local target, rejectContentsCount;
    public func Configure(pTarget)
    {
      target = pTarget;
      return(1);
    }
    protected func RejectContents()
    {
      rejectContentsCount += 1;
      RemoveObject(target);
      return(0);
    }
    "#,
    ));
    container.set_c4_callback_convention(true);
    let mut item = crate::support::TestValueExt::test_value(Definition::from_script(
        "ITEM",
        "Item",
        "#strict\n",
    ));
    item.set_collectible(true);
    engine.register_test_definition(actor);
    engine.register_test_definition(container);
    engine.register_test_definition(item);

    let hut = engine.spawn_test_object(SpawnConfig::new("HUT2"));
    let clonk = engine.spawn_test_object(SpawnConfig::new("CLNK").with_container(hut));
    let held = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(clonk));
    let target = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut));
    let hut_index = engine.test_object_index(hut);
    assert_eq!(
        engine.call_test_object_function(
            hut_index,
            "Configure",
            vec![Value::Object(target.as_u64())],
        ),
        Value::Int(1)
    );
    arm_get(&mut engine, clonk, target);

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(local_int(&engine, hut, "rejectContentsCount"), 1);
    assert_eq!(local_int(&engine, clonk, "nilDropTargetCount"), 1);
    assert_eq!(local_int(&engine, clonk, "staleDropTargetCount"), 0);
    assert!(
        engine.object_snapshot(held).is_some(),
        "selected item survives"
    );
}

#[test]
fn get_reject_contents_status_clear_mode_controls_put_away_target() {
    #[derive(Clone, Copy)]
    enum CallbackOrder {
        Attached,
        ClearThenDetach,
        DetachThenClear,
    }

    fn run(clear_pointers: bool, order: CallbackOrder) -> (i32, i32) {
        let mut engine = Engine::new();
        let mut actor = crate::support::TestValueExt::test_value(Definition::from_script(
            "CLNK",
            "Clonk",
            r#"#strict
        local nilDropTargetCount, retainedDropTargetCount;
        public func StartGet(pTarget) { return(SetCommand(this(), "Get", pTarget)); }
        protected func GetObject2Drop(pTarget)
        {
          if (pTarget) retainedDropTargetCount += 1;
          else nilDropTargetCount += 1;
          return(Contents(0));
        }
        "#,
        ));
        actor.set_c4_callback_convention(true);
        actor.set_collection_limit(1);
        let clear_pointers = if clear_pointers { "true" } else { "false" };
        let callback_body = match order {
            CallbackOrder::Attached => {
                format!("SetObjectStatus(2, target, {clear_pointers});")
            }
            CallbackOrder::ClearThenDetach => format!(
                "SetObjectStatus(2, target, {clear_pointers});\n  SetCommand(actor, \"Wait\");"
            ),
            CallbackOrder::DetachThenClear => format!(
                "SetCommand(actor, \"Wait\");\n  SetObjectStatus(2, target, {clear_pointers});"
            ),
        };
        let container_script = format!(
            r#"#strict
local actor, target;
public func Configure(pActor, pTarget)
{{
  actor = pActor;
  target = pTarget;
  return(1);
}}
protected func RejectContents()
{{
  {callback_body}
  return(0);
}}
"#
        );
        let mut container = crate::support::TestValueExt::test_value(Definition::from_script(
            "HUT2",
            "Hut",
            &container_script,
        ));
        container.set_c4_callback_convention(true);
        let mut item = crate::support::TestValueExt::test_value(Definition::from_script(
            "ITEM",
            "Item",
            "#strict\n",
        ));
        item.set_collectible(true);
        engine.register_test_definition(actor);
        engine.register_test_definition(container);
        engine.register_test_definition(item);

        let hut = engine.spawn_test_object(SpawnConfig::new("HUT2"));
        let clonk = engine.spawn_test_object(SpawnConfig::new("CLNK").with_container(hut));
        let _held = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(clonk));
        let target = engine.spawn_test_object(SpawnConfig::new("ITEM").with_container(hut));
        let hut_index = engine.test_object_index(hut);
        engine.call_test_object_function(
            hut_index,
            "Configure",
            vec![
                Value::Object(clonk.as_u64()),
                Value::Object(target.as_u64()),
            ],
        );
        arm_get(&mut engine, clonk, target);
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

        (
            local_int(&engine, clonk, "nilDropTargetCount"),
            local_int(&engine, clonk, "retainedDropTargetCount"),
        )
    }

    assert_eq!(
        run(false, CallbackOrder::Attached),
        (0, 1),
        "StatusDeactivate(false) retains the executing Get's Target"
    );
    assert_eq!(
        run(true, CallbackOrder::Attached),
        (1, 0),
        "StatusDeactivate(true) clears the linked executing Get's Target"
    );
    assert_eq!(
        run(true, CallbackOrder::ClearThenDetach),
        (1, 0),
        "a clear followed by SetCommand freezes the executing Get's null Target"
    );
    assert_eq!(
        run(true, CallbackOrder::DetachThenClear),
        (0, 1),
        "SetCommand unlinks the executing Get before the later ClearPointers walk"
    );
}

#[test]
fn get_collection_limit_puts_away_before_reject_collect() {
    let mut engine = Engine::new();
    let mut actor = crate::support::TestValueExt::test_value(Definition::from_script(
        "CLNK",
        "Clonk",
        r#"#strict
    local dropSelectionCount, rejectCollectCount;
    public func StartGet(pTarget) { return(SetCommand(this(), "Get", pTarget)); }
    protected func GetObject2Drop(pTarget)
    {
      dropSelectionCount += 1;
      return(Contents(0));
    }
    protected func RejectCollect(idObject, pObject)
    {
      rejectCollectCount += 1;
      return(1);
    }
    "#,
    ));
    actor.set_c4_callback_convention(true);
    actor.set_collection_limit(1);
    let mut held = crate::support::TestValueExt::test_value(Definition::from_script(
        "HELD",
        "Held item",
        "#strict\n",
    ));
    held.set_collectible(true);
    let mut incoming = crate::support::TestValueExt::test_value(Definition::from_script(
        "INCM",
        "Incoming item",
        r#"#strict
    local rejectEntranceCount;
    protected func RejectEntrance(pContainer)
    {
      rejectEntranceCount += 1;
      return(0);
    }
    "#,
    ));
    incoming.set_c4_callback_convention(true);
    incoming.set_collectible(true);
    engine.register_test_definition(actor);
    engine.register_test_definition(held);
    engine.register_test_definition(incoming);

    let clonk = engine.spawn_test_object(SpawnConfig::new("CLNK"));
    let held = engine.spawn_test_object(SpawnConfig::new("HELD").with_container(clonk));
    let incoming = engine.spawn_test_object(SpawnConfig::new("INCM"));
    arm_get(&mut engine, clonk, incoming);

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(local_int(&engine, clonk, "dropSelectionCount"), 1);
    assert_eq!(local_int(&engine, clonk, "rejectCollectCount"), 0);
    assert_eq!(local_int(&engine, incoming, "rejectEntranceCount"), 0);
    assert!(
        engine.object_snapshot(held).is_some(),
        "selected item survives"
    );
    assert_eq!(
        engine.test_object_snapshot(incoming).container,
        None,
        "the desired item is not offered to RejectCollect on the full frame"
    );
    assert!(
        engine
            .test_object_snapshot(clonk)
            .command_stack
            .command_names()
            .iter()
            .any(|command| command == "Get"),
        "successful PutAway leaves the Get command pending"
    );
}

#[test]
fn get_incomplete_select_knowledge_reports_no_con_activate() {
    let mut engine = Engine::new();
    let mut actor = crate::support::TestValueExt::test_value(Definition::from_script(
        "CLNK",
        "Clonk",
        r#"#strict
    local rejectCollectCount;
    public func StartGet(pTarget) { return(SetCommand(this(), "Get", pTarget)); }
    protected func RejectCollect(idObject, pObject)
    {
      rejectCollectCount += 1;
      return(0);
    }
    "#,
    ));
    actor.set_c4_callback_convention(true);
    actor.set_crew_member(true);
    let mut container = crate::support::TestValueExt::test_value(Definition::from_script(
        "HUT2",
        "Hut",
        r#"#strict
    local rejectContentsCount;
    protected func RejectContents()
    {
      rejectContentsCount += 1;
      return(0);
    }
    "#,
    ));
    container.set_c4_callback_convention(true);
    let mut cart = crate::support::TestValueExt::test_value(Definition::from_script(
        "CART",
        "Cart",
        r#"#strict
    local rejectEntranceCount;
    protected func RejectEntrance(pContainer)
    {
      rejectEntranceCount += 1;
      return(0);
    }
    "#,
    ));
    cart.set_c4_callback_convention(true);
    cart.set_collectible(true);
    engine.register_test_definition(actor);
    engine.register_test_definition(container);
    engine.register_test_definition(cart);

    let hut = engine.spawn_test_object(SpawnConfig::new("HUT2"));
    let clonk = engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_alive(true)
            .with_crew_member(true)
            .with_container(hut),
    );
    let cart = engine.spawn_test_object(
        SpawnConfig::new("CART")
            .with_category(CATEGORY_VEHICLE | CATEGORY_SELECT_KNOWLEDGE)
            .with_construction(FULL_CON - 1)
            .with_container(hut),
    );
    arm_get(&mut engine, clonk, cart);

    let frame = crate::support::TestValueExt::test_value(engine.tick());

    assert_eq!(local_int(&engine, hut, "rejectContentsCount"), 0);
    assert_eq!(local_int(&engine, cart, "rejectEntranceCount"), 0);
    assert_eq!(local_int(&engine, clonk, "rejectCollectCount"), 0);
    assert_eq!(engine.test_object_snapshot(cart).container, Some(hut));
    let message = crate::support::TestValueExt::test_value(
        frame
            .hud
            .messages
            .iter()
            .find(|message| message.target == Some(clonk)),
    );
    assert_eq!(
        message.lines,
        vec!["Cart not completed.", "Activation denied."]
    );
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert!(engine
        .test_object_snapshot(clonk)
        .command_stack
        .command_names()
        .is_empty());

    let silent_clonk = engine.spawn_test_object(
        SpawnConfig::new("CLNK")
            .with_alive(true)
            .with_crew_member(true)
            .with_container(hut),
    );
    let silent_cart = engine.spawn_test_object(
        SpawnConfig::new("CART")
            .with_category(CATEGORY_VEHICLE | CATEGORY_SELECT_KNOWLEDGE)
            .with_construction(FULL_CON - 1)
            .with_container(hut),
    );
    let silent_index = engine.test_object_index(silent_clonk);
    crate::support::TestValueExt::test_value(
        engine.objects[silent_index].commands.push_front(
            CommandRequest::new(CommandId::Get)
                .with_target(Some(silent_cart))
                .with_mode(CommandMode::SilentBase),
        ),
    );
    let silent_frame = crate::support::TestValueExt::test_value(engine.tick());
    assert!(
        silent_frame
            .hud
            .messages
            .iter()
            .all(|message| message.target != Some(silent_clonk)),
        "SilentBase suppresses the no-construction failure message"
    );
}
