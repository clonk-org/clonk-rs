use lc_engine::{
    Definition, Engine, ObjectId, ObjectStatus, ObjectUpdate, PlayerConfig, SpawnConfig,
};
use lc_script::Value;

const PLAYER: i32 = 1;

const QUERY_PROBE_SCRIPT: &str = r#"#strict 2
local callback_answer, callback_player, callback_count;

public func Open(bool uppercase, string prompt, int player)
{
    return CallMessageBoard(this(), uppercase, prompt, player);
}

public func AbortQuery(int player)
{
    return AbortMessageBoard(this(), player);
}

public func RemoveThenOpen(int player)
{
    RemoveObject();
    return CallMessageBoard(this(), false, "removed in this call", player);
}

public func Answer(object target, string answer, int player)
{
    return OnMessageBoardAnswer(target, player, answer);
}

protected func InputCallback(string answer, int player)
{
    callback_answer = answer;
    callback_player = player;
    callback_count = callback_count + 1;
    return 1;
}
"#;

fn fixture() -> (Engine, ObjectId, ObjectId) {
    let mut engine = Engine::new();
    engine
        .register_definition(
            Definition::from_script("MBQP", "Message-board query probe", QUERY_PROBE_SCRIPT)
                .expect("message-board query probe compiles"),
        )
        .expect("message-board query probe registers");

    let mut crew_definition =
        Definition::from_script("CLNK", "Message-board player crew", "#strict 2\n")
            .expect("crew definition compiles");
    crew_definition.set_crew_member(true);
    engine
        .register_definition(crew_definition)
        .expect("crew definition registers");
    engine
        .register_player(PlayerConfig::new(PLAYER, "Message-board player"))
        .expect("message-board player registers");
    engine.set_local_players([PLAYER]);

    // Keep the player active through frame 35, when C4Player::Execute opens
    // the first pending local query (src/C4Player.cpp:228-235,2202-2213).
    let crew = engine
        .spawn_object(
            SpawnConfig::new("CLNK")
                .with_owner(PLAYER)
                .with_crew_member(true)
                .with_alive(true),
        )
        .expect("player crew spawns");
    engine
        .select_crew(PLAYER, [crew])
        .expect("player crew is selected");
    engine
        .set_crew_cursor(PLAYER, Some(crew))
        .expect("player crew becomes cursor");

    let target = engine
        .spawn_object(SpawnConfig::new("MBQP"))
        .expect("message-board target spawns");
    let driver = engine
        .spawn_object(SpawnConfig::new("MBQP"))
        .expect("message-board answer driver spawns");
    (engine, target, driver)
}

fn call(engine: &mut Engine, object: ObjectId, function: &str, args: Vec<Value>) -> Value {
    let index = engine
        .find_object_index(object)
        .unwrap_or_else(|| panic!("{function} target {object} remains addressable"));
    engine
        .call_object_function(index, function, args)
        .unwrap_or_else(|error| panic!("{function} succeeds: {error}"))
}

fn open(
    engine: &mut Engine,
    target: ObjectId,
    uppercase: bool,
    prompt: &str,
    player: i32,
) -> Value {
    call(
        engine,
        target,
        "Open",
        vec![
            Value::Bool(uppercase),
            Value::String(prompt.to_string()),
            Value::Int(player),
        ],
    )
}

#[test]
fn call_message_board_rejects_invalid_players_and_status_zero_objects() {
    let (mut engine, target, _) = fixture();

    assert_eq!(
        open(&mut engine, target, false, "invalid player", 999),
        Value::Bool(false)
    );
    assert!(
        engine
            .player(PLAYER)
            .expect("valid player remains")
            .to_state()
            .message_board_queries
            .is_empty(),
        "a rejected player must not leave a query behind"
    );

    // Keep the Status==0 object in the vector until the next tick, matching
    // the C++ AssignRemoval window in which FnCallMessageBoard can still be
    // handed the object pointer and must reject its cleared Status.
    engine
        .apply_object_update(
            target,
            ObjectUpdate::new().with_status(ObjectStatus::Deleted),
        )
        .expect("target enters Status==0 before cleanup");
    assert_eq!(
        engine
            .object_snapshot(target)
            .expect("deleted target remains until cleanup")
            .status,
        ObjectStatus::Deleted
    );
    assert_eq!(
        open(&mut engine, target, false, "deleted object", PLAYER),
        Value::Bool(false)
    );
    assert!(
        engine
            .player(PLAYER)
            .expect("valid player remains")
            .to_state()
            .message_board_queries
            .is_empty(),
        "a Status==0 target must not leave a query behind"
    );
}

#[test]
fn call_message_board_rejects_a_target_removed_earlier_in_the_same_script_call() {
    let (mut engine, target, _) = fixture();

    assert_eq!(
        call(
            &mut engine,
            target,
            "RemoveThenOpen",
            vec![Value::Int(PLAYER)],
        ),
        Value::Bool(false)
    );
    assert!(
        engine
            .player(PLAYER)
            .expect("valid player remains")
            .to_state()
            .message_board_queries
            .is_empty(),
        "AssignRemoval's synchronous Status=0 must reject the callback target"
    );
}

#[test]
fn replacement_query_opens_once_and_abort_matches_active_and_pending_paths() {
    let (mut engine, target, _) = fixture();

    assert_eq!(
        open(&mut engine, target, false, "old prompt", PLAYER),
        Value::Bool(true)
    );
    assert_eq!(
        open(&mut engine, target, true, "replacement prompt", PLAYER,),
        Value::Bool(true)
    );

    let state = engine
        .player(PLAYER)
        .expect("message-board player remains")
        .to_state();
    assert_eq!(state.message_board_queries.len(), 1);
    let query = &state.message_board_queries[0];
    assert_eq!(query.target, Some(target));
    assert_eq!(query.prompt, "replacement prompt");
    assert!(query.uppercase);
    assert!(!query.answered);

    for frame in 1..=35 {
        engine
            .tick()
            .unwrap_or_else(|error| panic!("query activation tick {frame} succeeds: {error}"));
    }
    let active = engine
        .active_message_board_input()
        .expect("the local player's replacement query opens on Tick35");
    assert_eq!(active.player, PLAYER);
    assert_eq!(active.target, Some(target));
    assert_eq!(active.prompt, "replacement prompt");
    assert!(active.uppercase);

    // Closing a local active C4ChatInputDialog synchronously submits its
    // no-answer control. That consumes the query reentrantly before
    // FnAbortMessageBoard reaches RemoveMessageBoardQuery, so the outer
    // builtin returns false even though it did close the matching input.
    assert_eq!(
        call(&mut engine, target, "AbortQuery", vec![Value::Int(PLAYER)],),
        Value::Bool(false)
    );
    assert!(
        engine.active_message_board_input().is_none(),
        "AbortMessageBoard closes a matching active type-in immediately"
    );
    assert!(engine
        .player(PLAYER)
        .expect("message-board player remains")
        .to_state()
        .message_board_queries
        .is_empty());

    // With no active TypeIn, AbortMsgBoardQuery is a no-op and the explicit
    // query removal supplies the ordinary true-then-false return contract.
    assert_eq!(
        open(&mut engine, target, false, "pending abort", PLAYER),
        Value::Bool(true)
    );
    assert!(engine.active_message_board_input().is_none());
    assert_eq!(
        call(&mut engine, target, "AbortQuery", vec![Value::Int(PLAYER)],),
        Value::Bool(true)
    );
    assert_eq!(
        call(&mut engine, target, "AbortQuery", vec![Value::Int(PLAYER)],),
        Value::Bool(false),
        "AbortMessageBoard reports false after the one matching query is gone"
    );
}

#[test]
fn deleting_an_active_query_target_closes_the_input_and_unblocks_the_next_prompt() {
    let (mut engine, target, next_target) = fixture();

    assert_eq!(
        open(&mut engine, target, false, "first prompt", PLAYER),
        Value::Bool(true)
    );
    for frame in 1..=35 {
        engine.tick().unwrap_or_else(|error| {
            panic!("first query activation tick {frame} succeeds: {error}")
        });
    }
    assert_eq!(
        engine
            .active_message_board_input()
            .expect("the first prompt opens")
            .target,
        Some(target)
    );

    engine
        .apply_object_update(
            target,
            ObjectUpdate::new().with_status(ObjectStatus::Deleted),
        )
        .expect("active query target enters Status==0");
    engine
        .tick()
        .expect("ordinary destroyed-object cleanup succeeds");
    assert!(
        engine.active_message_board_input().is_none(),
        "C4MessageInput::ClearPointers closes the deleted target's type-in"
    );

    assert_eq!(
        open(&mut engine, next_target, false, "unblocked prompt", PLAYER,),
        Value::Bool(true)
    );
    for frame in 1..=35 {
        engine
            .tick()
            .unwrap_or_else(|error| panic!("next query activation tick {frame} succeeds: {error}"));
    }
    let active = engine
        .active_message_board_input()
        .expect("a later prompt is no longer blocked by the deleted target");
    assert_eq!(active.target, Some(next_target));
    assert_eq!(active.prompt, "unblocked prompt");
}

#[test]
fn message_board_answer_reaches_the_target_input_callback_exactly_once() {
    let (mut engine, target, driver) = fixture();

    assert_eq!(
        open(&mut engine, target, false, "type an answer", PLAYER),
        Value::Bool(true)
    );
    for frame in 1..=35 {
        engine.tick().unwrap_or_else(|error| {
            panic!("answer query activation tick {frame} succeeds: {error}")
        });
    }
    assert_eq!(
        engine
            .active_message_board_input()
            .expect("the answer query opens")
            .target,
        Some(target)
    );
    let answer_args = || {
        vec![
            Value::Object(target.as_u64()),
            Value::String("typed answer".to_string()),
            Value::Int(PLAYER),
        ]
    };
    assert_eq!(
        call(&mut engine, driver, "Answer", answer_args()),
        Value::Bool(true)
    );

    let target_state = engine
        .object_snapshot(target)
        .expect("callback target remains active");
    assert_eq!(
        target_state.local_vars.get("callback_answer"),
        Some(&Value::String("typed answer".to_string()))
    );
    assert_eq!(
        target_state.local_vars.get("callback_player"),
        Some(&Value::Int(PLAYER))
    );
    assert_eq!(
        target_state.local_vars.get("callback_count"),
        Some(&Value::Int(1))
    );
    assert!(
        engine
            .player(PLAYER)
            .expect("message-board player remains")
            .to_state()
            .message_board_queries
            .is_empty(),
        "OnMessageBoardAnswer consumes the query before dispatching InputCallback"
    );
    assert_eq!(
        engine
            .active_message_board_input()
            .expect("the builtin itself does not own dialog closure")
            .target,
        Some(target),
        "the UI/CID submission path, not FnOnMessageBoardAnswer, closes the type-in"
    );

    assert_eq!(
        call(&mut engine, driver, "Answer", answer_args()),
        Value::Bool(false),
        "a doubled answer is rejected after the query was consumed"
    );
    assert_eq!(
        engine
            .object_snapshot(target)
            .expect("callback target remains active")
            .local_vars
            .get("callback_count"),
        Some(&Value::Int(1)),
        "the rejected doubled answer must not invoke InputCallback again"
    );
}
