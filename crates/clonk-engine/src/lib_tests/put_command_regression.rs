use super::*;

fn put_fixture_engine() -> Engine {
    let actor_script = r#"#strict
local tracked, nested_throw, reject_contents_seen, menu_after_nested, menu_selection_after_nested;
local put_depth, finish_nested_throw, command_after_nested, menu_during_value, menu_selection_during_value;
local mutate_menu_during_value, spawn_menu_item, reexecute_same_throw, command_after_same_reentry;
local command_during_throw_start, departure_seen;
public func NoteRejectContents()
{
  reject_contents_seen = 1;
  return(1);
}
public func NoteDeparture()
{
  departure_seen = 1;
  return(1);
}
public func RunOutsideThrow()
{
  AddCommand(this, "Throw");
  ExecuteCommand();
  return(1);
}
protected func StartThrow()
{
  command_during_throw_start = GetCommand();
  return(1);
}
protected func CalcValue(pInBase, iForPlr)
{
  menu_during_value = GetMenu();
  menu_selection_during_value = GetMenuSelection();
  if (mutate_menu_during_value) SetMenuSize(3, 4);
  if (spawn_menu_item)
  {
    spawn_menu_item = 0;
    var added = CreateObject(NITM, 0, 0, -1);
    if (added) added->Enter(pInBase);
  }
  return(7);
}
protected func Put()
{
  put_depth++;
  if (put_depth == 2 && finish_nested_throw)
  {
    FinishCommand(this, 1, 0);
    ExecuteCommand();
  }
  if (put_depth == 1 && nested_throw)
  {
    AddCommand(this, "Throw");
    ExecuteCommand();
    menu_after_nested = GetMenu();
    menu_selection_after_nested = GetMenuSelection();
    command_after_nested = GetCommand();
  }
  if (put_depth == 1 && reexecute_same_throw)
  {
    ExecuteCommand();
    command_after_same_reentry = GetCommand();
  }
  tracked->Mark(5);
  put_depth--;
  return(1);
}
"#;
    let target_script = r#"#strict
local reject, reject_contents_actor, nested_put_actor, nested_put_item, nested_put_depth;
protected func RejectContents()
{
  if (reject_contents_actor) reject_contents_actor->NoteRejectContents();
  return(0);
}
protected func RejectCollect(idItem, pItem)
{
  pItem->Mark(2);
  if (nested_put_actor && !nested_put_depth)
  {
    nested_put_depth = 1;
    SetCommand(nested_put_actor, "Put", this(), 0, 0, nested_put_item);
    ExecuteCommand(nested_put_actor);
    nested_put_depth = 0;
    return(1);
  }
  return(reject);
}
protected func Collection2(pItem)
{
  pItem->Mark(3);
  return(1);
}
protected func Collection(pItem, fPut)
{
  pItem->Mark(6);
  return(1);
}
"#;
    let item_script = r#"#strict
local callback_order;
public func Mark(iStep)
{
  callback_order = callback_order * 10 + iStep;
  return(1);
}
protected func RejectEntrance(pTarget)
{
  Mark(1);
  return(0);
}
protected func Entrance(pTarget)
{
  Mark(4);
  return(1);
}
protected func Departure(pTarget)
{
  pTarget->NoteDeparture();
  return(1);
}
"#;

    let mut actor =
        Definition::from_script("ACTR", "Actor", actor_script).expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                    "Push".to_string(),
                ActionSpec::default().with_procedure("PUSH"),
            ),
            (
                    "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                    "Throw".to_string(),
                ActionSpec::default()
                    .with_procedure("THROW")
                    .with_start_call("StartThrow"),
            ),
        ]),
    );
    let mut target =
        Definition::from_script("TARG", "Target", target_script).expect("target compiles");
    target.set_c4_callback_convention(true);
    target.set_grab_put_get(GRAB_PUT_GET_PUT);
    let mut item = Definition::from_script("ITEM", "Item", item_script).expect("item compiles");
    item.set_c4_callback_convention(true);
    item.set_collectible(true);

    let mut engine = Engine::with_seed(118);
    engine.register_definition(actor).expect("actor registers");
    engine
        .register_definition(target)
        .expect("target registers");
    engine.register_definition(item).expect("item registers");
    engine
}

fn spawn_push_put_triplet(engine: &mut Engine, reject: bool) -> (ObjectId, ObjectId, ObjectId) {
    let target = engine
        .spawn_object(
            SpawnConfig::new("TARG")
                .with_position(Vector2::new(80, 40))
                .with_velocity(Vector2::new(3, -2))
                .with_local_vars(HashMap::from([(
                        "reject".to_string(),
                    Value::Int(i32::from(reject)),
                )])),
        )
        .expect("target spawns");
    let mut push = ActionState::new("Push");
    push.target = Some(target);
    let actor = engine
        .spawn_object(
            SpawnConfig::new("ACTR")
                .with_position(Vector2::new(20, 40))
                .with_action(push),
        )
        .expect("actor spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("tracked".to_string(), object_reference_value(item));
    engine.objects[actor_index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::Put)
                .with_target(Some(target))
                .with_target2(Some(item))
                .with_ty(Some(1)),
        )
        .expect("Put queues");
    (actor, item, target)
}

fn spawn_contained_put_take_triplet(
    engine: &mut Engine,
    reject: bool,
    command: CommandId,
) -> (ObjectId, ObjectId, ObjectId) {
    let target = engine
        .spawn_object(SpawnConfig::new("TARG").with_local_vars(HashMap::from([(
                "reject".to_string(),
            Value::Int(i32::from(reject)),
        )])))
        .expect("target spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_container(target))
        .expect("contained actor spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("carried item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("tracked".to_string(), object_reference_value(item));
    engine.objects[actor_index]
        .commands
        .push_front(CommandRequest::new(command).with_target(Some(item)))
        .expect("put/take command queues");
    (actor, item, target)
}

#[test]
fn object_com_put_accepts_an_explicit_non_content_object() {
    let mut engine = put_fixture_engine();
    let target = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("target spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR"))
        .expect("actor spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM"))
        .expect("uncontained item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("tracked".to_string(), object_reference_value(item));

    assert!(engine
        .try_object_com_put(actor, target, item)
        .expect("ObjectComPut executes"));
    let item_state =
        &engine.objects[engine.find_object_index(item).expect("item remains")].state;
    assert_eq!(item_state.container, Some(target));
    assert_eq!(
        item_state.local_vars.get("callback_order"),
        Some(&Value::Int(123_456))
    );
}

#[test]
fn empty_put_take_opens_menu_on_a_retained_status_zero_target() {
    let mut engine = put_fixture_engine();
    let target = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("target spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_container(target))
        .expect("contained actor spawns");
    let target_index = engine.find_object_index(target).expect("target exists");
    let _ = engine.objects[target_index].mark_destroyed();

    assert_eq!(
        engine
            .try_object_com_put_take(actor, target, None)
            .expect("ObjectComPutTake executes"),
        ObjectComPutTakeOutcome::Finished
    );
    let menu = engine.objects[engine.find_object_index(actor).expect("actor remains")]
        .state
        .menu
        .as_ref()
        .expect("activate menu opens");
    assert_eq!(menu.identification, Value::Int(6));
    assert_eq!(menu.refill_object, Some(target));
}

#[test]
fn exit_unlinks_a_retained_status_zero_object() {
    let mut engine = put_fixture_engine();
    let container = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("container spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(container))
        .expect("contained item spawns");
    let item_index = engine.find_object_index(item).expect("item exists");
    let position = engine.objects[item_index].state.position;
    let rotation = engine.objects[item_index].state.rotation;
    let _ = engine.objects[item_index].mark_destroyed();

    assert!(engine
        .exit_object_at_position_with_zero_motion(item, container, position, rotation)
        .expect("Exit executes"));
    assert_eq!(
        engine.objects[engine.find_object_index(item).expect("item remains")]
            .state
            .container,
        None
    );
    assert!(!engine.objects[engine
        .find_object_index(container)
        .expect("container remains")]
    .state
    .contents
    .contains(&item));
}

#[test]
fn assign_removal_exit_contents_continues_after_a_child_reenters() {
    let mut engine = put_fixture_engine();
    let reentering_script = r#"#strict
local reenter_target;
protected func Departure(pTarget)
{
  if (reenter_target) Enter(reenter_target);
  return(1);
}
"#;
    let mut reentering = Definition::from_script("RITM", "Reentering item", reentering_script)
        .expect("reentering item compiles");
    reentering.set_c4_callback_convention(true);
    engine
        .register_definition(reentering)
        .expect("reentering item registers");
    let destination = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("destination spawns");
    let parent = engine
        .spawn_object(SpawnConfig::new("ACTR"))
        .expect("parent spawns");
    let first = engine
        .spawn_object(SpawnConfig::new("RITM").with_container(parent))
        .expect("first child spawns");
    let second = engine
        .spawn_object(SpawnConfig::new("RITM").with_container(parent))
        .expect("second child spawns");
    let parent_index = engine.find_object_index(parent).expect("parent exists");
    let first_link = engine.objects[parent_index].state.contents[0];
    let sibling = if first_link == first { second } else { first };
    let first_link_index = engine
        .find_object_index(first_link)
        .expect("first linked child exists");
    engine.objects[first_link_index].state.local_vars.insert(
            "reenter_target".to_string(),
        object_reference_value(destination),
    );

    assert!(engine
        .assign_object_removal_with_contents(parent, true)
        .expect("AssignRemoval executes"));
    assert_eq!(
        engine.objects[engine
            .find_object_index(first_link)
            .expect("reentered child remains")]
        .state
        .container,
        Some(destination)
    );
    assert_eq!(
        engine.objects[engine
            .find_object_index(sibling)
            .expect("exited sibling remains")]
        .state
        .container,
        None,
            "the loop re-reads Contents.First even when the prior Exit returned false"
    );
}

#[test]
fn put_event_runs_object_com_put_callbacks_and_ty_ungrab_only_on_success() {
    let mut engine = put_fixture_engine();
    let (actor, item, target) = spawn_push_put_triplet(&mut engine, false);

    engine
        .execute_object_command_now(actor)
        .expect("successful Put executes");

    let item_index = engine.find_object_index(item).expect("item remains");
    let target_index = engine.find_object_index(target).expect("target remains");
    assert_eq!(engine.objects[item_index].state.container, Some(target));
    assert_eq!(
        engine.objects[item_index]
            .state
            .local_vars
            .get("callback_order"),
        Some(&Value::Int(123_456)),
            "RejectEntrance -> RejectCollect -> Collection2 -> Entrance -> Put -> Collection"
    );
    assert_eq!(
        engine.objects[item_index].state.position,
        engine.objects[target_index].state.position
    );
    assert_eq!(
        engine.objects[item_index].fixed_velocity,
        engine.objects[target_index].fixed_velocity
    );
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_names(),
        ["UnGrab", "Put"],
            "Ty queues interval-zero UnGrab above the still-running Put"
    );
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_views()[1]
            .ty,
        Some(1)
    );

    let (rejected_actor, rejected_item, _) = spawn_push_put_triplet(&mut engine, true);
    engine
        .execute_object_command_now(rejected_actor)
        .expect("rejected Put resolves");
    let rejected_item_index = engine
        .find_object_index(rejected_item)
        .expect("rejected item remains");
    assert_eq!(
        engine.objects[rejected_item_index].state.container,
        Some(rejected_actor)
    );
    assert_eq!(
        engine.objects[rejected_item_index]
            .state
            .local_vars
            .get("callback_order"),
        Some(&Value::Int(12))
    );
    let rejected_actor_index = engine
        .find_object_index(rejected_actor)
        .expect("rejected actor remains");
    assert!(
        engine.objects[rejected_actor_index]
            .commands
            .snapshot()
            .is_empty(),
            "helper failure finishes Put and must not queue Ty-UnGrab"
    );
}

#[test]
fn contained_throw_and_drop_run_object_com_put_callbacks_before_finishing() {
    for command in [CommandId::Throw, CommandId::Drop] {
        let mut accepted = put_fixture_engine();
        let (actor, item, target) =
            spawn_contained_put_take_triplet(&mut accepted, false, command);
        accepted
            .execute_object_command_now(actor)
            .expect("accepted ObjectComPutTake executes");

        let item_state = &accepted.objects[accepted
            .find_object_index(item)
            .expect("accepted item remains")]
        .state;
        assert_eq!(item_state.container, Some(target), "{command:?} transfers");
        assert_eq!(
            item_state.local_vars.get("callback_order"),
            Some(&Value::Int(123_456)),
                "{command:?}: RejectEntrance -> RejectCollect -> Collection2 -> Entrance -> Put -> Collection"
        );
        assert!(
            accepted.objects[accepted
                .find_object_index(actor)
                .expect("accepted actor remains")]
            .commands
            .snapshot()
            .is_empty(),
                "{command:?} finishes after the callback tail"
        );

        let mut rejected = put_fixture_engine();
        let (actor, item, _) = spawn_contained_put_take_triplet(&mut rejected, true, command);
        rejected
            .execute_object_command_now(actor)
            .expect("rejected ObjectComPutTake executes");
        let item_state = &rejected.objects[rejected
            .find_object_index(item)
            .expect("rejected item remains")]
        .state;
        assert_eq!(
            item_state.container,
            Some(actor),
                "{command:?} honors RejectCollect"
        );
        assert_eq!(
            item_state.local_vars.get("callback_order"),
            Some(&Value::Int(12)),
                "Put/Collection must not run after RejectCollect"
        );
        assert!(
            rejected.objects[rejected
                .find_object_index(actor)
                .expect("rejected actor remains")]
            .commands
            .snapshot()
            .is_empty(),
                "{command:?} ignores the helper result and still finishes"
        );
    }
}

#[test]
fn command_references_accept_inactive_live_put_take_helpers_like_cpp() {
    for command in [CommandId::Throw, CommandId::Drop] {
        let mut engine = put_fixture_engine();
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_container(target))
            .expect("contained actor spawns");
        let deleted = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
            .expect("tombstone item spawns");
        let inactive = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
            .expect("inactive item spawns");

        let deleted_index = engine.find_object_index(deleted).expect("tombstone exists");
        let _ = engine.objects[deleted_index].mark_destroyed();
        let inactive_index = engine
            .find_object_index(inactive)
            .expect("inactive item exists");
        engine.objects[inactive_index].state.status = ObjectStatus::Inactive;
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_index].state.contents = vec![deleted, inactive];
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(command))
            .expect("put/take command queues");

        engine
            .execute_object_command_now(actor)
            .expect("ObjectComPutTake executes");

        assert_eq!(
            engine
                .object_snapshot(inactive)
                .expect("inactive item remains")
                .container,
            Some(target),
                "{command:?} skips Status==0 and retains C4OS_INACTIVE"
        );
        assert_eq!(
            engine
                .object_snapshot(deleted)
                .expect("tombstone remains until end-of-tick cleanup")
                .container,
            Some(actor),
                "the deleted contents hole is never selected"
        );
    }
}

#[test]
fn command_references_accept_inactive_sell_candidates_like_cpp() {
    let mut engine = put_fixture_engine();
    engine
        .register_player(PlayerConfig::new(1, "Seller"))
        .expect("seller registers");
    let base = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("base spawns");
    let base_index = engine.find_object_index(base).expect("base exists");
    engine.objects[base_index].state.base = 1;
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_owner(1))
        .expect("seller spawns");
    let deleted = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(base))
        .expect("tombstone item spawns");
    let inactive = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(base))
        .expect("inactive item spawns");

    let deleted_index = engine.find_object_index(deleted).expect("tombstone exists");
    let _ = engine.objects[deleted_index].mark_destroyed();
    let inactive_index = engine
        .find_object_index(inactive)
        .expect("inactive item exists");
    engine.objects[inactive_index].state.status = ObjectStatus::Inactive;
    engine.objects[base_index].state.contents = vec![deleted, inactive];

    assert_eq!(
        engine.command_sell_candidate(actor, base, "ITEM", Some(inactive)),
        Some((1, inactive)),
            "an explicit inactive Target2 remains preferred"
    );
    assert_eq!(
        engine.command_sell_candidate(actor, base, "ITEM", Some(deleted)),
        Some((1, inactive)),
            "a deleted Target2 falls back to Contents.Find's inactive entry"
    );
}

#[test]
fn nested_put_take_does_not_consume_the_outer_put_result_marker() {
    let mut engine = put_fixture_engine();
    engine
        .definitions
        .get_mut(&DefinitionId::from("TARG"))
        .expect("target definition exists")
        .set_collection_limit(1);
    let (actor, first_item, target) = spawn_push_put_triplet(&mut engine, false);
    let second_item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("second carried item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("nested_throw".to_string(), Value::Int(1));

    engine
        .execute_object_command_now(actor)
        .expect("outer Put and nested Throw execute");

    assert_eq!(
        engine
            .object_snapshot(first_item)
            .expect("first item remains")
            .container,
        Some(target),
            "the outer Put succeeds before its Put callback"
    );
    assert_eq!(
        engine
            .object_snapshot(second_item)
            .expect("second item remains")
            .container,
        Some(actor),
            "the nested Throw's put is rejected by the now-full target"
    );
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_names(),
        ["UnGrab", "Put"],
            "the nested false PutTake result must not fail the outer successful Put"
    );

    engine
        .execute_object_command_now(actor)
        .expect("Ty cleanup UnGrab executes");
    engine
        .execute_object_command_now(actor)
        .expect("outer Put observes the transferred item and completes");
    assert!(
        engine.objects[engine.find_object_index(actor).expect("actor remains")]
            .commands
            .snapshot()
            .is_empty(),
            "the outer Put retained and consumed its own success result"
    );
}

#[test]
fn recursive_same_kind_put_keeps_each_callback_result_with_its_emitter() {
    let mut engine = put_fixture_engine();
    let target = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("target spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_container(target))
        .expect("contained actor spawns");
    let outer_item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("outer item spawns");
    let replacement_item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("replacement item spawns");
    let target_index = engine.find_object_index(target).expect("target exists");
    engine.objects[target_index].state.local_vars.extend([
        (
                "nested_put_actor".to_string(),
            object_reference_value(actor),
        ),
        (
                "nested_put_item".to_string(),
            object_reference_value(replacement_item),
        ),
    ]);
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index].state.local_vars.insert(
            "tracked".to_string(),
        object_reference_value(replacement_item),
    );
    engine.objects[actor_index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::Put)
                .with_target(Some(target))
                .with_target2(Some(outer_item)),
        )
        .expect("outer Put queues");

    engine
        .execute_object_command_now(actor)
        .expect("outer and replacement Put execute");

    assert_eq!(
        engine
            .object_snapshot(outer_item)
            .expect("outer item remains")
            .container,
        Some(actor),
            "the outer RejectCollect result stays with the detached outer Put"
    );
    assert_eq!(
        engine
            .object_snapshot(replacement_item)
            .expect("replacement item remains")
            .container,
        Some(target),
            "recursive ExecuteCommand completes the replacement helper synchronously"
    );
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let commands = engine.objects[actor_index].commands.command_views();
    let [replacement] = commands.as_slice() else {
        panic!("replacement Put must remain for its next native Execute: {commands:?}");
    };
    assert_eq!(replacement.name, "Put");
    assert_eq!(replacement.target, Some(target));
    assert_eq!(replacement.target2, Some(replacement_item));
    assert!(
        !replacement.finished,
            "the outer failure must not consume the successful replacement Put"
    );
}

#[test]
fn removed_nested_throw_does_not_finish_the_outer_throw_instance() {
    let mut engine = put_fixture_engine();
    let target = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("target spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_container(target))
        .expect("actor spawns contained");
    let first_item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("first item spawns");
    let second_item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("second item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index].state.local_vars.extend([
        ("tracked".to_string(), object_reference_value(first_item)),
        ("nested_throw".to_string(), Value::Int(1)),
        ("finish_nested_throw".to_string(), Value::Int(1)),
    ]);
    engine.objects[actor_index]
        .commands
        .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(first_item)))
        .expect("outer Throw queues");

    engine
        .execute_object_command_now(actor)
        .expect("outer and nested Throw execute");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("command_after_nested"),
        Some(&Value::String("Throw".to_string().into())),
            "removing the inner Throw while its helper returns leaves the outer instance live"
    );
    assert!(
        engine.objects[actor_index].commands.snapshot().is_empty(),
            "the outer Throw finishes only when its own helper returns"
    );
    for item in [first_item, second_item] {
        assert_eq!(
            engine
                .object_snapshot(item)
                .expect("item remains")
                .container,
            Some(target)
        );
    }
}

#[test]
fn callback_execute_command_reenters_the_same_in_flight_throw() {
    let mut engine = put_fixture_engine();
    let target = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("target spawns");
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_container(target))
        .expect("actor spawns contained");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index].state.local_vars.extend([
        ("tracked".to_string(), object_reference_value(item)),
        ("reexecute_same_throw".to_string(), Value::Int(1)),
    ]);
    engine.objects[actor_index]
        .commands
        .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(item)))
        .expect("Throw queues");

    engine
        .execute_object_command_now(actor)
        .expect("Throw and callback reentry execute");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("command_after_same_reentry"),
        Some(&Value::String("Get".to_string().into())),
            "the in-flight Throw reexecutes and queues Get after its requested item moved"
    );
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_names(),
        ["Get", "Throw"],
            "the reentrant child remains above the exact finished outer Throw"
    );
}

#[test]
fn script_execute_command_runs_outside_throw_callbacks_before_returning() {
    let mut engine = put_fixture_engine();
    let actor = engine
        .spawn_object(SpawnConfig::new("ACTR").with_action(ActionState::new("Walk")))
        .expect("walking actor spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("carried item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .state
        .local_vars
        .insert("tracked".to_string(), object_reference_value(item));

    engine
        .call_object_function(actor_index, "RunOutsideThrow", Vec::new())
        .expect("script ExecuteCommand returns");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let actor_state = &engine.objects[actor_index].state;
    assert_eq!(
        actor_state.local_vars.get("command_during_throw_start"),
        Some(&Value::String("Throw".to_string().into())),
            "StartCall sees the exact executing Throw before Finish(true)"
    );
    assert_eq!(
        actor_state.local_vars.get("departure_seen"),
        Some(&Value::Int(1)),
            "Departure runs before script ExecuteCommand returns"
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
    assert_eq!(
        engine
            .object_snapshot(item)
            .expect("item remains")
            .container,
        None
    );
}

#[test]
fn script_throw_from_no_other_action_dig_uses_callbackful_object_com_stop() {
    let script = r#"#strict
public func RunExecute()
{
  ExecuteCommand();
  return(1);
}
"#;
    let mut actor = Definition::from_script("NDIG", "NoOtherAction digger", script)
        .expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                    "Dig".to_string(),
                ActionSpec::default()
                    .with_procedure("DIG")
                    .with_no_other_action(true),
            ),
            (
                    "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                    "Throw".to_string(),
                ActionSpec::default().with_procedure("THROW"),
            ),
        ]),
    );
    let mut item = Definition::from_script("NDIT", "NoOtherAction item", "#strict")
        .expect("item compiles");
    item.set_collectible(true);

    let mut engine = Engine::with_seed(119);
    engine.register_definition(actor).expect("actor registers");
    engine.register_definition(item).expect("item registers");
    let actor = engine
        .spawn_object(
            SpawnConfig::new("NDIG")
                .with_action(ActionState::new("Dig"))
                .with_velocity(Vector2::new(7, -3)),
        )
        .expect("digger spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("NDIT").with_container(actor))
        .expect("item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .commands
        .push_front(CommandRequest::new(CommandId::Throw).with_target(Some(item)))
        .expect("Throw queues");

    engine
        .call_object_function(actor_index, "RunExecute", Vec::new())
        .expect("script ExecuteCommand returns");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(engine.objects[actor_index].state.action.name, "Dig");
    assert_eq!(
        engine.objects[actor_index].fixed_velocity,
        FixedVec2::from_ints(7, -3),
            "failed Idle/Walk transitions do not zero the digging velocity"
    );
    assert_eq!(
        engine.objects[actor_index].state.command_direction,
        CommandDirection::Stop,
            "ObjectActionStand writes ComDir before its rejected Walk"
    );
    assert_eq!(
        engine
            .object_snapshot(item)
            .expect("item remains")
            .container,
        Some(actor),
            "ObjectComThrow rejects the still-Dig procedure"
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
}

#[test]
fn script_drop_from_dig_runs_object_com_stop_callbacks_before_dropping() {
    let script = r#"#strict
local callback_order;
public func RunExecute()
{
  ExecuteCommand();
  return(1);
}
protected func AbortDig()
{
  callback_order = callback_order * 10 + 1;
  return(1);
}
protected func StartWalk()
{
  callback_order = callback_order * 10 + 2;
  return(1);
}
"#;
    let mut actor =
        Definition::from_script("DDIG", "Digging dropper", script).expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                    "Dig".to_string(),
                ActionSpec::default()
                    .with_procedure("DIG")
                    .with_abort_call("AbortDig"),
            ),
            (
                    "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_start_call("StartWalk"),
            ),
        ]),
    );
    let mut item =
        Definition::from_script("DDIT", "Dropped item", "#strict").expect("item compiles");
    item.set_collectible(true);

    let mut engine = Engine::with_seed(121);
    engine.register_definition(actor).expect("actor registers");
    engine.register_definition(item).expect("item registers");
    let actor = engine
        .spawn_object(
            SpawnConfig::new("DDIG")
                .with_action(ActionState::new("Dig"))
                .with_velocity(Vector2::new(7, -3)),
        )
        .expect("digger spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("DDIT").with_container(actor))
        .expect("item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .commands
        .push_front(CommandRequest::new(CommandId::Drop).with_target(Some(item)))
        .expect("Drop queues");

    engine
        .call_object_function(actor_index, "RunExecute", Vec::new())
        .expect("script ExecuteCommand returns");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(engine.objects[actor_index].state.action.name, "Walk");
    assert_eq!(engine.objects[actor_index].fixed_velocity, FixedVec2::ZERO);
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("callback_order"),
        Some(&Value::Int(12)),
            "Dig Abort and Walk Start finish before ObjectComDrop"
    );
    assert_eq!(
        engine
            .object_snapshot(item)
            .expect("item remains")
            .container,
        None
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
}

#[test]
fn ungrab_command_respects_no_other_action_and_suppresses_release_callbacks() {
    let actor_script = r#"#strict
local grab_calls;
protected func Grab(object target, bool grab)
{
  grab_calls++;
  return(1);
}
"#;
    let target_script = r#"#strict
local grabbed_calls;
protected func Grabbed(object actor, bool grab)
{
  grabbed_calls++;
  return(1);
}
"#;
    let mut actor =
        Definition::from_script("UNGA", "Locked pusher", actor_script).expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                    "Push".to_string(),
                ActionSpec::default()
                    .with_procedure("PUSH")
                    .with_no_other_action(true),
            ),
            (
                    "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
        ]),
    );
    let mut target =
        Definition::from_script("UNGT", "Push target", target_script).expect("target compiles");
    target.set_c4_callback_convention(true);

    let mut engine = Engine::new();
    engine.register_definition(actor).expect("actor registers");
    engine
        .register_definition(target)
        .expect("target registers");
    let target = engine
        .spawn_object(SpawnConfig::new("UNGT"))
        .expect("target spawns");
    let mut push = ActionState::new("Push");
    push.target = Some(target);
    let actor = engine
        .spawn_object(
            SpawnConfig::new("UNGA")
                .with_action(push)
                .with_velocity(Vector2::new(4, -2)),
        )
        .expect("actor spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index].state.command_direction = CommandDirection::Left;
    engine.objects[actor_index]
        .commands
        .push_front(CommandRequest::new(CommandId::UnGrab))
        .expect("UnGrab queues");

    engine
        .execute_object_command_now(actor)
        .expect("UnGrab executes");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(engine.objects[actor_index].state.action.name, "Push");
    assert_eq!(
        engine.objects[actor_index].state.action.target,
        Some(target)
    );
    assert_eq!(
        engine.objects[actor_index].fixed_velocity,
        FixedVec2::from_ints(4, -2)
    );
    assert_eq!(
        engine.objects[actor_index].state.command_direction,
        CommandDirection::Stop,
            "C4Command::UnGrab writes Stop even when ObjectComUnGrab fails"
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("grab_calls"),
        None
    );
    let target_index = engine.find_object_index(target).expect("target remains");
    assert_eq!(
        engine.objects[target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        None
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
}

#[test]
fn object_com_ungrab_soft_closes_menu_before_release_callbacks() {
    let actor_script = r#"#strict
local deny, query_calls, grab_calls, menu_during_grab;
public func OpenMenu() { return CreateMenu(WIPF, this(), this(), 0, "Choose"); }
public func SetDeny(value) { deny = value; return(1); }
public func ReadMenu() { return GetMenu(); }
protected func MenuQueryCancel()
{
  query_calls++;
  return deny;
}
protected func Grab(object target, bool grab)
{
  grab_calls++;
  menu_during_grab = GetMenu();
  return(1);
}
"#;
    let target_script = r#"#strict
local grabbed_calls, menu_during_grabbed;
protected func Grabbed(object actor, bool grab)
{
  grabbed_calls++;
  menu_during_grabbed = actor->ReadMenu();
  return(1);
}
"#;
    let mut actor =
        Definition::from_script("UGMA", "Menu pusher", actor_script).expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                    "Push".to_string(),
                ActionSpec::default().with_procedure("PUSH"),
            ),
            (
                    "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
        ]),
    );
    let mut target = Definition::from_script("UGMT", "Menu push target", target_script)
        .expect("target compiles");
    target.set_c4_callback_convention(true);

    let mut engine = Engine::new();
    engine.register_definition(actor).expect("actor registers");
    engine
        .register_definition(target)
        .expect("target registers");
    let target = engine
        .spawn_object(SpawnConfig::new("UGMT"))
        .expect("target spawns");
    let mut push = ActionState::new("Push");
    push.target = Some(target);
    let actor = engine
        .spawn_object(
            SpawnConfig::new("UGMA")
                .with_action(push)
                .with_velocity(Vector2::new(4, -2)),
        )
        .expect("actor spawns");
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    engine.objects[actor_index].state.command_direction = CommandDirection::Left;

    let call_actor = |engine: &mut Engine, name: &str, args: Vec<Value>| {
        let actor_index = engine.find_object_index(actor).expect("actor remains");
        engine
            .call_object_function(actor_index, name, args)
            .expect("actor call succeeds")
    };
    assert_eq!(
        call_actor(&mut engine, "OpenMenu", Vec::new()),
        Value::Bool(true)
    );
    assert_eq!(
        call_actor(&mut engine, "SetDeny", vec![Value::Int(1)]),
        Value::Int(1)
    );

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert!(!engine
        .object_com_ungrab(actor_index)
        .expect("denied ungrab returns"));
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(engine.objects[actor_index].state.action.name, "Walk");
    assert_eq!(engine.objects[actor_index].fixed_velocity, FixedVec2::ZERO);
    assert_eq!(
        engine.objects[actor_index].state.command_direction,
        CommandDirection::Stop
    );
    assert!(engine.objects[actor_index].state.menu.is_some());
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("query_calls"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("grab_calls"),
        Some(&Value::Nil),
            "a denied close stops before Grab(false)"
    );
    let target_index = engine.find_object_index(target).expect("target remains");
    assert_eq!(
        engine.objects[target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        None,
            "a denied close stops before Grabbed(false)"
    );

    assert_eq!(
        call_actor(&mut engine, "SetDeny", vec![Value::Int(0)]),
        Value::Int(1)
    );
    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let mut push = ActionState::new("Push");
    push.target = Some(target);
    engine.objects[actor_index].state.action = push;
    assert!(engine
        .object_com_ungrab(actor_index)
        .expect("allowed ungrab succeeds"));

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(engine.objects[actor_index].state.action.name, "Walk");
    assert!(engine.objects[actor_index].state.menu.is_none());
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("query_calls"),
        Some(&Value::Int(2))
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("grab_calls"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("menu_during_grab"),
        Some(&Value::Int(0)),
            "Grab(false) observes the menu already closed"
    );
    let target_index = engine.find_object_index(target).expect("target remains");
    assert_eq!(
        engine.objects[target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        engine.objects[target_index]
            .state
            .local_vars
            .get("menu_during_grabbed"),
        Some(&Value::Int(0)),
            "Grabbed(false) observes the menu already closed"
    );
}

#[test]
fn object_com_ungrab_uses_raw_status_gates_after_grab_callback() {
    let actor_script = r#"#strict
local grab_calls, remove_self;
protected func Grab(object target, bool grab)
{
  grab_calls++;
  if (!grab && remove_self) RemoveObject();
  return(1);
}
"#;
    let target_script = r#"#strict
local grabbed_calls;
protected func Grabbed(object actor, bool grab)
{
  grabbed_calls++;
  return(1);
}
"#;
    let mut actor =
        Definition::from_script("UGRA", "Pusher", actor_script).expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        None,
        HashMap::from([
            (
                    "Push".to_string(),
                ActionSpec::default().with_procedure("PUSH"),
            ),
            (
                    "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
        ]),
    );
    let mut target =
        Definition::from_script("UGRT", "Target", target_script).expect("target compiles");
    target.set_c4_callback_convention(true);
    let mut engine = Engine::new();
    engine.register_definition(actor).expect("actor registers");
    engine
        .register_definition(target)
        .expect("target registers");

    let inactive_target = engine
        .spawn_object(SpawnConfig::new("UGRT").with_status(ObjectStatus::Inactive))
        .expect("inactive target spawns");
    let mut push = ActionState::new("Push");
    push.target = Some(inactive_target);
    let first_actor = engine
        .spawn_object(SpawnConfig::new("UGRA").with_action(push))
        .expect("first actor spawns");
    let first_actor_index = engine
        .find_object_index(first_actor)
        .expect("first actor exists");
    assert!(engine
        .object_com_ungrab(first_actor_index)
        .expect("inactive target ungrab succeeds"));
    let first_actor_index = engine
        .find_object_index(first_actor)
        .expect("first actor remains");
    assert_eq!(engine.objects[first_actor_index].state.action.name, "Walk");
    assert_eq!(
        engine.objects[first_actor_index]
            .state
            .local_vars
            .get("grab_calls"),
        Some(&Value::Int(1))
    );
    let inactive_target_index = engine
        .find_object_index(inactive_target)
        .expect("inactive target remains");
    assert_eq!(
        engine.objects[inactive_target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        Some(&Value::Int(1)),
            "inactive Status is nonzero and still receives Grabbed(false)"
    );

    let live_target = engine
        .spawn_object(SpawnConfig::new("UGRT"))
        .expect("live target spawns");
    let mut push = ActionState::new("Push");
    push.target = Some(live_target);
    let removing_actor = engine
        .spawn_object(SpawnConfig::new("UGRA").with_action(push))
        .expect("removing actor spawns");
    let removing_actor_index = engine
        .find_object_index(removing_actor)
        .expect("removing actor exists");
    engine.objects[removing_actor_index]
        .state
        .local_vars
        .insert("remove_self".to_string(), Value::Int(1));
    assert!(engine
        .object_com_ungrab(removing_actor_index)
        .expect("callback-removing ungrab returns"));
    assert_eq!(
        engine
            .object_snapshot(removing_actor)
            .expect("removed actor storage remains")
            .status,
        ObjectStatus::Deleted
    );
    let live_target_index = engine
        .find_object_index(live_target)
        .expect("live target remains");
    assert_eq!(
        engine.objects[live_target_index]
            .state
            .local_vars
            .get("grabbed_calls"),
        None,
            "actor Status=0 after Grab(false) suppresses target Grabbed(false)"
    );
}

#[test]
fn script_targeted_throw_runs_turn_action_before_object_com_throw() {
    let script = r#"#strict
local turn_started, throw_saw_turn;
public func RunExecute()
{
  ExecuteCommand();
  return(1);
}
protected func StartTurn()
{
  turn_started = 1;
  return(1);
}
protected func StartThrow()
{
  throw_saw_turn = turn_started;
  return(1);
}
"#;
    let mut actor =
        Definition::from_script("TTRN", "Turning thrower", script).expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
    actor.set_physical(PhysicalInfo {
        throw: 50_000,
        ..PhysicalInfo::default()
    });
    actor.configure_actions(
        None,
        HashMap::from([
            (
                    "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_directions(2)
                    .with_turn_action("Turn"),
            ),
            (
                    "Turn".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_directions(2)
                    .with_start_call("StartTurn"),
            ),
            (
                    "Throw".to_string(),
                ActionSpec::default()
                    .with_procedure("THROW")
                    .with_start_call("StartThrow"),
            ),
        ]),
    );
    let mut item = Definition::from_script("TIT2", "Turning throw item", "#strict")
        .expect("item compiles");
    item.set_collectible(true);

    let mut engine = Engine::with_seed(120);
    let mut landscape = Landscape::flat(200, 100);
    landscape.set_world_height(150);
    engine.set_landscape(landscape);
    engine.register_definition(actor).expect("actor registers");
    engine.register_definition(item).expect("item registers");
    let actor = engine
        .spawn_object(
            SpawnConfig::new("TTRN")
                .with_position(Vector2::new(99, 99))
                .with_direction(Direction::Left)
                .with_action(ActionState::new("Walk")),
        )
        .expect("thrower spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("TIT2").with_container(actor))
        .expect("item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::Throw)
                .with_target(Some(item))
                .with_tx(Some(100))
                .with_ty(Some(70)),
        )
        .expect("targeted Throw queues");

    engine
        .call_object_function(actor_index, "RunExecute", Vec::new())
        .expect("script ExecuteCommand returns");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    assert_eq!(
        engine.objects[actor_index].state.direction,
        Direction::Right
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("turn_started"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        engine.objects[actor_index]
            .state
            .local_vars
            .get("throw_saw_turn"),
        Some(&Value::Int(1)),
            "SetDir's TurnAction completes before Throw's StartCall"
    );
    assert_eq!(
        engine
            .object_snapshot(item)
            .expect("item remains")
            .container,
        None
    );
    assert!(engine.objects[actor_index].commands.snapshot().is_empty());
}

#[test]
fn object_action_throw_runs_full_exit_bounds_motion_and_callbacks() {
    let actor_script = r#"#strict
protected func Ejection(pObject)
{
  pObject->RecordThrowStep(2);
  return(1);
}
"#;
    let item_script = r#"#strict
local throw_order;
public func RecordThrowStep(iStep)
{
  throw_order = throw_order * 10 + iStep;
  return(1);
}
protected func ContactTop()
{
  RecordThrowStep(1);
  return(0);
}
protected func Departure(pContainer)
{
  RecordThrowStep(3);
  return(1);
}
"#;

    let mut engine = Engine::with_seed(118);
    let mut actor =
        Definition::from_script("TACT", "Throw actor", actor_script).expect("actor compiles");
    actor.set_c4_callback_convention(true);
    actor.configure_actions(
        Some("Walk".to_string()),
        HashMap::from([
            (
                    "Walk".to_string(),
                ActionSpec::default().with_procedure("WALK"),
            ),
            (
                    "Throw".to_string(),
                ActionSpec::default().with_procedure("THROW"),
            ),
        ]),
    );
    actor.set_physical(PhysicalInfo {
        throw: 50_000,
        ..PhysicalInfo::default()
    });
    engine.register_definition(actor).expect("actor registers");

    let mut item =
        Definition::from_script("TITM", "Thrown item", item_script).expect("item compiles");
    item.set_c4_callback_convention(true);
    item.set_contact_function_calls(true);
    item.set_border_bound(C4D_BORDER_TOP);
    item.set_shape_rect(Some(DefinitionRect::new(0, 0, 4, 4)));
    engine.register_definition(item).expect("item registers");

    let actor = engine
        .spawn_object(
            SpawnConfig::new("TACT")
                .with_position(Vector2::new(20, 0))
                .with_direction(Direction::Right)
                .with_action(ActionState::new("Walk")),
        )
        .expect("actor spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("TITM").with_container(actor))
        .expect("item spawns");
    let item_index = engine.find_object_index(item).expect("item exists");
    engine.objects[item_index].state.shape_override = Some(DefinitionRect::new(0, 0, 6, 6));
    engine.objects[item_index].shape_rect = Some(DefinitionRect::new(0, 0, 6, 6));
    engine.objects[item_index].fixed_velocity = FixedVec2::new(itofix(7), itofix(-9));

    assert!(engine
        .try_object_action_throw(actor, item)
        .expect("ObjectActionThrow succeeds"));

    let force = math::val_by_physical(400, 50_000);
    let item_index = engine.find_object_index(item).expect("item remains");
    let item = &engine.objects[item_index];
    assert_eq!(item.state.container, None);
    assert_eq!(item.state.position, Vector2::new(20, 0));
    assert_eq!(item.fixed_velocity, FixedVec2::new(force, -force));
    assert_eq!(item.rotation_velocity, force);
    assert_eq!(item.state.shape_override, None);
    assert_eq!(
        item.state.local_vars.get("throw_order"),
        Some(&Value::Int(123)),
            "BoundsCheck ContactTop runs before Ejection and Departure"
    );
}

#[test]
fn nested_empty_put_take_runs_reject_contents_and_opens_menu_before_return() {
    let mut engine = put_fixture_engine();
    let mut new_item =
        Definition::from_script("NITM", "New menu item", "#strict").expect("new item compiles");
    new_item.set_category(CATEGORY_STRUCTURE);
    engine
        .register_definition(new_item)
        .expect("new item registers");
    let target = engine
        .spawn_object(SpawnConfig::new("TARG"))
        .expect("target spawns");
    let actor = engine
        .spawn_object(
            SpawnConfig::new("ACTR")
                .with_container(target)
                .with_category(CATEGORY_OBJECT),
        )
        .expect("contained actor spawns");
    let item = engine
        .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
        .expect("carried item spawns");
    let actor_index = engine.find_object_index(actor).expect("actor exists");
    engine.objects[actor_index].state.local_vars.extend([
        ("tracked".to_string(), object_reference_value(item)),
        ("nested_throw".to_string(), Value::Int(1)),
        ("mutate_menu_during_value".to_string(), Value::Int(1)),
        ("spawn_menu_item".to_string(), Value::Int(1)),
    ]);
    let target_index = engine.find_object_index(target).expect("target exists");
    engine.objects[target_index].state.local_vars.insert(
            "reject_contents_actor".to_string(),
        object_reference_value(actor),
    );
    engine.objects[actor_index]
        .commands
        .push_front(
            CommandRequest::new(CommandId::Put)
                .with_target(Some(target))
                .with_target2(Some(item)),
        )
        .expect("outer Put queues");

    engine
        .execute_object_command_now(actor)
        .expect("outer Put callback executes nested empty Throw");

    let actor_index = engine.find_object_index(actor).expect("actor remains");
    let actor_state = &engine.objects[actor_index].state;
    assert_eq!(
        actor_state.local_vars.get("reject_contents_seen"),
        Some(&Value::Int(1)),
            "RejectContents runs inside nested ExecuteCommand"
    );
    assert_eq!(
        actor_state.local_vars.get("menu_after_nested"),
        Some(&Value::Int(6)),
            "GetMenu observes C4MN_Activate before ExecuteCommand returns"
    );
    assert_eq!(
        actor_state.local_vars.get("menu_selection_after_nested"),
        Some(&Value::Int(0)),
            "the activate menu has already refilled and selected its first row"
    );
    assert_eq!(
        actor_state.local_vars.get("menu_during_value"),
        Some(&Value::Int(6)),
            "CalcValue sees the already-installed Activate menu during refill"
    );
    assert_eq!(
        actor_state.local_vars.get("menu_selection_during_value"),
        Some(&Value::Int(-1)),
            "selection adjustment stays frozen until the full refill completes"
    );
    let menu = actor_state
        .menu
        .as_ref()
        .expect("activate menu remains open");
    assert_eq!(menu.identification, Value::Int(6));
    assert_eq!(menu.refill_object, Some(target));
    assert_eq!((menu.columns, menu.lines), (3, 4));
    assert_eq!(
        menu.items
            .iter()
            .find(|entry| entry.item_id == "ACTR")
            .and_then(|entry| entry.value),
        Some(7)
    );
    assert!(
        menu.items.iter().any(|entry| entry.item_id == "NITM"),
            "an item entered after the iterator's current row is included in the same refill"
    );
    assert_eq!(
        engine.objects[actor_index]
            .commands
            .snapshot()
            .command_names(),
        ["Put"],
            "the nested Throw finished without disturbing the outer Put"
    );
}

#[test]
fn object_com_put_without_grab_put_drops_with_full_physics_only_when_down_double_is_armed() {
    for (down_double, should_drop) in [(0, false), (-3, true)] {
        let mut engine = put_fixture_engine();
        let actor_definition = engine
            .definitions
            .get_mut("ACTR")
            .expect("actor definition remains");
        actor_definition.set_shape_rect(Some(DefinitionRect::new(-8, -10, 16, 20)));
        actor_definition.set_physical(PhysicalInfo {
            throw: 50_000,
            ..PhysicalInfo::default()
        });
        engine
            .definitions
            .get_mut("ITEM")
            .expect("item definition remains")
            .set_shape_rect(Some(DefinitionRect::new(0, 0, 4, 4)));
        engine
            .register_definition(
                Definition::from_script("NOPU", "No put", "#strict")
                    .expect("no-put target compiles"),
            )
            .expect("no-put target registers");
        engine
            .register_player(PlayerConfig::new(1, "PutTake owner"))
            .expect("player registers");
        engine
            .player_mut(1)
            .expect("player remains")
            .control
            .last_com_down_double = down_double;
        let target = engine
            .spawn_object(SpawnConfig::new("NOPU"))
            .expect("no-put target spawns");
        let mut push = ActionState::new("Push");
        push.target = Some(target);
        let actor = engine
            .spawn_object(
                SpawnConfig::new("ACTR")
                    .with_owner(1)
                    .with_position(Vector2::new(20, 40))
                    .with_velocity(Vector2::new(-2, 0))
                    .with_command_direction(CommandDirection::Right)
                    .with_action(push),
            )
            .expect("pushing actor spawns");
        let item = engine
            .spawn_object(SpawnConfig::new("ITEM").with_container(actor))
            .expect("carried item spawns");
        assert_eq!(
            engine
                .try_object_com_put(actor, target, item)
                .expect("ObjectComPut attempt executes"),
            should_drop,
                "LastComDownDouble={down_double}"
        );

        let actor_index = engine.find_object_index(actor).expect("actor remains");
        let item_index = engine.find_object_index(item).expect("item remains");
        if should_drop {
            let force = math::val_by_physical(400, 50_000);
            assert_eq!(engine.objects[item_index].state.container, None);
            assert_eq!(
                engine.objects[item_index].state.position,
                Vector2::new(
                    engine.objects[actor_index].state.position.x + 8,
                    engine.objects[actor_index].state.position.y + 6,
                ),
                    "drop uses the live actor/item shapes for its exit position"
            );
            assert_eq!(
                engine.objects[item_index].fixed_velocity,
                FixedVec2::new(force, C4Fixed::ZERO),
                    "rightward drop applies the physical throw force"
            );
            assert_eq!(engine.objects[actor_index].state.no_collect_delay, 2);
            assert_eq!(engine.objects[actor_index].state.action.name, "Walk");
            assert_eq!(
                engine.objects[actor_index]
                    .state
                    .local_vars
                    .get("departure_seen"),
                Some(&Value::Int(1)),
                    "the drop runs the item's Departure callback"
            );
        } else {
            assert_eq!(engine.objects[item_index].state.container, Some(actor));
            assert_eq!(engine.objects[actor_index].state.no_collect_delay, 0);
            assert_eq!(engine.objects[actor_index].state.action.name, "Push");
            assert_eq!(
                engine.objects[actor_index]
                    .state
                    .local_vars
                    .get("departure_seen"),
                None,
                    "a disarmed failed put must not begin the drop callback sequence"
            );
        }
    }
}

#[test]
fn empty_contained_throw_and_drop_open_the_activate_menu_before_finishing() {
    for command in [CommandId::Throw, CommandId::Drop] {
        let mut engine = put_fixture_engine();
        let target = engine
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("target spawns");
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_container(target))
            .expect("empty contained actor spawns");
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(command))
            .expect("empty put/take command queues");

        engine
            .execute_object_command_now(actor)
            .expect("empty ObjectComPutTake executes");

        let actor_index = engine.find_object_index(actor).expect("actor remains");
        let actor_state = &engine.objects[actor_index].state;
        let menu = actor_state.menu.as_ref().expect("activate menu opens");
        assert_eq!(menu.identification, Value::Int(6));
        assert_eq!(menu.refill_object, Some(target));
        assert!(
            engine.objects[actor_index].commands.snapshot().is_empty(),
                "{command:?} finishes after opening the menu"
        );
    }
}

#[test]
fn empty_pushing_throw_and_drop_open_get_only_for_grab_get_targets() {
    for command in [CommandId::Throw, CommandId::Drop] {
        let mut engine = put_fixture_engine();
        let mut target_definition = Definition::from_script("GETT", "Get target", "#strict")
            .expect("get target compiles");
        target_definition.set_grab_put_get(GRAB_PUT_GET_GET);
        engine
            .register_definition(target_definition)
            .expect("get target registers");
        let target = engine
            .spawn_object(SpawnConfig::new("GETT"))
            .expect("get target spawns");
        let mut push = ActionState::new("Push");
        push.target = Some(target);
        let actor = engine
            .spawn_object(SpawnConfig::new("ACTR").with_action(push))
            .expect("empty pusher spawns");
        let actor_index = engine.find_object_index(actor).expect("actor exists");
        engine.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(command))
            .expect("empty pushing command queues");

        engine
            .execute_object_command_now(actor)
            .expect("empty pushing ObjectComPutTake executes");

        let actor_index = engine.find_object_index(actor).expect("actor remains");
        let menu = engine.objects[actor_index]
            .state
            .menu
            .as_ref()
            .expect("get menu opens");
        assert_eq!(menu.identification, Value::Int(13));
        assert_eq!(menu.refill_object, Some(target));
        assert!(engine.objects[actor_index].commands.snapshot().is_empty());

        let mut denied = put_fixture_engine();
        let target = denied
            .spawn_object(SpawnConfig::new("TARG"))
            .expect("non-GrabGet target spawns");
        let mut push = ActionState::new("Push");
        push.target = Some(target);
        let actor = denied
            .spawn_object(SpawnConfig::new("ACTR").with_action(push))
            .expect("empty denied pusher spawns");
        let actor_index = denied.find_object_index(actor).expect("actor exists");
        denied.objects[actor_index]
            .commands
            .push_front(CommandRequest::new(command))
            .expect("denied pushing command queues");
        denied
            .execute_object_command_now(actor)
            .expect("denied empty PutTake executes");
        let actor_index = denied.find_object_index(actor).expect("actor remains");
        assert!(denied.objects[actor_index].state.menu.is_none());
        assert!(denied.objects[actor_index].commands.snapshot().is_empty());
    }
}
