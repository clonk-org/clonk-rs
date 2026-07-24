use clonk_engine::{
    Definition, EliminatePlayerControlData, Engine, LegacyCString, MessageBoardAnswerControlData,
    ObjectId, ObjectStatus, ObjectUpdate, PlayerAtClient, PlayerConfig, PlayerStatus, SpawnConfig,
};
use clonk_script::Value;

const PLAYER: i32 = 1;

const QUERY_PROBE_SCRIPT: &str = r#"#strict 2
local callback_answer, callback_player, callback_count, callback_frame;

public func Open(bool uppercase, string prompt, int player)
{
    return CallMessageBoard(this(), uppercase, prompt, player);
}

public func OpenFor(object target, bool uppercase, string prompt, int player)
{
    return CallMessageBoard(target, uppercase, prompt, player);
}

public func AbortQuery(int player)
{
    return AbortMessageBoard(this(), player);
}

public func TestState(int player)
{
    var invalid = TestMessageBoard(999, true);
    var available_default = TestMessageBoard(player);
    var available_explicit = TestMessageBoard(player, false);
    var empty = TestMessageBoard(player, true);
    var opened = CallMessageBoard(this(), false, "state probe", player);
    var pending = TestMessageBoard(player, true);
    return [invalid, available_default, available_explicit, empty, opened, pending];
}

public func InUse(int player)
{
    return TestMessageBoard(player, true);
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
    callback_frame = FrameCounter();
    return 1;
}
"#;

const GLOBAL_QUERY_SCRIPT: &str = r#"#strict 2
func OpenGlobal(int player)
{
    CallMessageBoard(0, false, "global query", player);
}

func AnswerGlobal(string answer, int player)
{
    if (!OnMessageBoardAnswer(0, player, answer)) SetGravity(88);
}

func ClearGlobal(int player)
{
    if (!OnMessageBoardAnswer(0, player)) SetGravity(99);
}

func AnswerGlobalEmpty(int player)
{
    if (!OnMessageBoardAnswer(0, player, "")) SetGravity(77);
}

protected func InputCallback(string answer, int player)
{
    SetGravity(100 + GetLength(answer) + player);
    return 1;
}
"#;

const ENGINE_GLOBAL_CALLBACK_SCRIPT: &str = r#"#strict 2
global func InputCallback(string answer, int player)
{
    SetGravity(313);
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
            Value::String(prompt.to_string().into()),
            Value::Int(player),
        ],
    )
}

fn object_number(object: ObjectId) -> i32 {
    i32::try_from(object.as_u64()).expect("fixture object number fits the signed control field")
}

#[test]
fn call_message_board_rejects_invalid_players_and_status_zero_objects() {
    let (mut engine, target, driver) = fixture();

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
        call(
            &mut engine,
            driver,
            "OpenFor",
            vec![
                Value::Object(target.as_u64()),
                Value::Bool(false),
                Value::String("deleted object".to_string().into()),
                Value::Int(PLAYER),
            ],
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
fn test_message_board_reports_validity_availability_and_retained_query_state() {
    let (mut engine, target, _) = fixture();
    engine
        .player_mut(PLAYER)
        .expect("message-board player remains")
        .set_at_client(PlayerAtClient::new(7));

    assert_eq!(
        call(&mut engine, target, "TestState", vec![Value::Int(PLAYER)],),
        Value::Array(vec![
            Value::Nil,
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true),
        ])
    );
    assert_eq!(
        engine
            .player(PLAYER)
            .expect("message-board player remains")
            .to_state()
            .message_board_queries
            .len(),
        1,
        "the pending-state probe sees the query registered earlier in the same call"
    );

    for frame in 1..=35 {
        engine.tick_without_snapshot().unwrap_or_else(|error| {
            panic!("state query activation tick {frame} succeeds: {error}")
        });
    }
    let control = engine
        .prepare_message_board_answer_control(LegacyCString::default(), 7)
        .expect("active input produces its synchronized answer");
    assert!(
        engine
            .player(PLAYER)
            .expect("message-board player remains")
            .message_board_queries()[0]
            .answered
    );
    assert_eq!(
        call(&mut engine, target, "InUse", vec![Value::Int(PLAYER)]),
        Value::Bool(true)
    );
    assert!(engine
        .execute_message_board_answer_control(&control)
        .expect("the queued no-answer control removes its query"));
    assert_eq!(
        call(&mut engine, target, "InUse", vec![Value::Int(PLAYER)]),
        Value::Bool(false),
        "synchronized removal of the last query clears the in-use probe"
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
            .tick_without_snapshot()
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
fn call_message_board_replacement_moves_target_to_list_tail() {
    let (mut engine, first_target, second_target) = fixture();

    assert_eq!(
        open(&mut engine, first_target, false, "first prompt", PLAYER),
        Value::Bool(true)
    );
    assert_eq!(
        open(&mut engine, second_target, false, "second prompt", PLAYER),
        Value::Bool(true)
    );
    assert_eq!(
        open(
            &mut engine,
            first_target,
            true,
            "replacement prompt",
            PLAYER,
        ),
        Value::Bool(true)
    );

    let queries = &engine
        .player(PLAYER)
        .expect("message-board player remains")
        .to_state()
        .message_board_queries;
    assert_eq!(queries.len(), 2);
    assert_eq!(queries[0].target, Some(second_target));
    assert_eq!(queries[0].prompt, "second prompt");
    assert_eq!(queries[1].target, Some(first_target));
    assert_eq!(queries[1].prompt, "replacement prompt");
    assert!(queries[1].uppercase);
}

#[test]
fn restored_query_denumerates_a_missing_callback_object() {
    let (mut engine, target, _) = fixture();

    assert_eq!(
        open(&mut engine, target, false, "saved prompt", PLAYER),
        Value::Bool(true)
    );
    let mut state = engine.capture_state();
    state.objects.retain(|object| object.snapshot.id != target);

    engine
        .restore_state(&state)
        .expect("player state with a missing callback object restores");
    let queries = engine
        .player(PLAYER)
        .expect("message-board player restores")
        .message_board_queries();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].target, None);
    assert_eq!(queries[0].prompt, "saved prompt");
}

#[test]
fn eliminated_normal_player_still_opens_a_local_query_on_tick35() {
    let (mut engine, target, _) = fixture();

    assert_eq!(
        open(&mut engine, target, false, "last prompt", PLAYER),
        Value::Bool(true)
    );
    assert!(engine
        .execute_eliminate_player_control(&EliminatePlayerControlData {
            player: PLAYER,
            by_client: 0,
        })
        .expect("host elimination control executes"));
    assert_eq!(
        engine
            .player(PLAYER)
            .expect("eliminated player remains during the retire delay")
            .status(),
        PlayerStatus::Eliminated
    );

    for frame in 1..=35 {
        engine
            .tick_without_snapshot()
            .unwrap_or_else(|error| panic!("eliminated query tick {frame} succeeds: {error}"));
    }
    assert_eq!(
        engine
            .active_message_board_input()
            .expect("C++ PS_Normal remains true after elimination")
            .target,
        Some(target)
    );
}

#[test]
fn local_script_player_does_not_open_a_message_board_query() {
    let (mut engine, target, _) = fixture();

    assert_eq!(
        open(&mut engine, target, false, "user-only prompt", PLAYER),
        Value::Bool(true)
    );
    let mut state = engine.capture_state();
    state
        .players
        .iter_mut()
        .find(|player| player.id == PLAYER)
        .expect("message-board player is saved")
        .script_player = true;
    engine
        .restore_state(&state)
        .expect("script-player state restores");

    for frame in 1..=35 {
        engine
            .tick_without_snapshot()
            .unwrap_or_else(|error| panic!("script-player query tick {frame} succeeds: {error}"));
    }
    assert!(engine.active_message_board_input().is_none());
    assert_eq!(
        engine
            .player(PLAYER)
            .expect("script player remains")
            .message_board_queries()
            .len(),
        1,
        "the non-local query stays pending instead of opening"
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
        engine.tick_without_snapshot().unwrap_or_else(|error| {
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
        .tick_without_snapshot()
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
            .tick_without_snapshot()
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
        engine.tick_without_snapshot().unwrap_or_else(|error| {
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
            Value::String("typed answer".to_string().into()),
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
        Some(&Value::String("typed answer".to_string().into()))
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

#[test]
fn ownerless_answer_dispatches_game_script_and_no_answer_only_clears_query() {
    let (mut engine, _, driver) = fixture();
    engine
        .install_scenario_script_with_convention(
            "Message-board global query",
            GLOBAL_QUERY_SCRIPT,
            true,
        )
        .expect("the global query scenario script installs");
    assert_eq!(
        engine.install_global_scripts(&[(
            "MessageBoardGlobal.c".into(),
            ENGINE_GLOBAL_CALLBACK_SCRIPT.into(),
        )]),
        1,
        "the engine-global fallback probe installs"
    );
    let call_global = |engine: &mut Engine, name: &str, args: Vec<Value>| {
        engine
            .call_scenario_script_function(name, args)
            .unwrap_or_else(|error| panic!("{name} succeeds: {error}"))
    };

    call_global(&mut engine, "OpenGlobal", vec![Value::Int(PLAYER)]);
    assert_eq!(
        engine
            .player(PLAYER)
            .expect("message-board player remains")
            .message_board_queries()
            .len(),
        1
    );
    call_global(
        &mut engine,
        "AnswerGlobal",
        vec![Value::String("global answer".into()), Value::Int(PLAYER)],
    );
    assert!(engine
        .player(PLAYER)
        .expect("message-board player remains")
        .message_board_queries()
        .is_empty());
    assert_eq!(
        engine.physics().gravity,
        114,
        "Game.Script InputCallback receives the 13-byte answer and player 1"
    );

    call_global(
        &mut engine,
        "AnswerGlobal",
        vec![Value::String("duplicate".into()), Value::Int(PLAYER)],
    );
    assert_eq!(
        engine.physics().gravity,
        88,
        "the consumed ownerless query returns false on a doubled answer"
    );

    call_global(&mut engine, "OpenGlobal", vec![Value::Int(PLAYER)]);
    call_global(&mut engine, "ClearGlobal", vec![Value::Int(PLAYER)]);
    assert!(engine
        .player(PLAYER)
        .expect("message-board player remains")
        .message_board_queries()
        .is_empty());
    assert_eq!(
        engine.physics().gravity,
        88,
        "an omitted answer clears the query without another callback"
    );

    call_global(&mut engine, "OpenGlobal", vec![Value::Int(PLAYER)]);
    call_global(&mut engine, "AnswerGlobalEmpty", vec![Value::Int(PLAYER)]);
    assert!(engine
        .player(PLAYER)
        .expect("message-board player remains")
        .message_board_queries()
        .is_empty());
    assert_eq!(
        engine.physics().gravity,
        101,
        "an explicit empty string still reaches Game.Script InputCallback"
    );

    engine
        .register_definition(
            Definition::from_script("MBQN", "Message-board no-callback probe", "#strict 2\n")
                .expect("no-callback probe compiles"),
        )
        .expect("no-callback probe registers");
    let no_callback_target = engine
        .spawn_object(SpawnConfig::new("MBQN"))
        .expect("no-callback probe spawns");
    assert_eq!(
        call(
            &mut engine,
            driver,
            "OpenFor",
            vec![
                Value::Object(no_callback_target.as_u64()),
                Value::Bool(false),
                Value::String("object-local callback only".into()),
                Value::Int(PLAYER),
            ],
        ),
        Value::Bool(true)
    );
    assert_eq!(
        call(
            &mut engine,
            driver,
            "Answer",
            vec![
                Value::Object(no_callback_target.as_u64()),
                Value::String("no global fallback".into()),
                Value::Int(PLAYER),
            ],
        ),
        Value::Bool(false),
        "an object without InputCallback does not fall back to Game.Script"
    );
    assert_eq!(
        engine.physics().gravity,
        101,
        "the engine-global callback is not considered for an object-owned query"
    );
    assert!(engine
        .player(PLAYER)
        .expect("message-board player remains")
        .message_board_queries()
        .is_empty());
}

#[test]
fn message_board_answer_control_requires_the_players_exact_client() {
    let (mut engine, target, _) = fixture();
    engine
        .player_mut(PLAYER)
        .expect("message-board player remains")
        .set_at_client(PlayerAtClient::new(7));
    assert_eq!(
        open(&mut engine, target, false, "authenticated answer", PLAYER),
        Value::Bool(true)
    );

    let forged = MessageBoardAnswerControlData {
        object: object_number(target),
        answer: LegacyCString::from_bytes(b"forged".to_vec()).expect("answer is NUL-free"),
        player: PLAYER,
        by_client: 8,
    };
    assert!(!engine
        .execute_message_board_answer_control(&forged)
        .expect("rejected control is not an engine error"));
    assert_eq!(
        engine
            .player(PLAYER)
            .expect("message-board player remains")
            .message_board_queries()
            .len(),
        1,
        "an unauthorized answer must not consume the query"
    );
    assert_ne!(
        engine
            .object_snapshot(target)
            .expect("callback target remains")
            .local_vars
            .get("callback_count"),
        Some(&Value::Int(1)),
        "an unauthorized answer must not invoke InputCallback"
    );

    let ownerless = MessageBoardAnswerControlData {
        object: object_number(target),
        answer: LegacyCString::default(),
        player: -1,
        by_client: 999,
    };
    assert!(engine
        .execute_message_board_answer_control(&ownerless)
        .expect("NO_OWNER is explicitly allowed"));
    assert_eq!(
        engine
            .player(PLAYER)
            .expect("message-board player remains")
            .message_board_queries()
            .len(),
        1,
        "the ownerless no-player call has no unrelated player query to consume"
    );
}

#[test]
fn message_board_answer_control_preserves_escaped_and_raw_text_in_the_same_frame() {
    let (mut engine, target, _) = fixture();
    engine
        .player_mut(PLAYER)
        .expect("message-board player remains")
        .set_at_client(PlayerAtClient::new(7));
    assert_eq!(
        open(&mut engine, target, false, "escaped answer", PLAYER),
        Value::Bool(true)
    );
    engine
        .tick_without_snapshot()
        .expect("advance to a nonzero control frame");
    let frame = i32::try_from(engine.frame()).expect("fixture frame fits i32");

    let gravity = engine.physics().gravity;
    let answer = b"say \" ); SetGravity(99); // \\ \x80".to_vec();
    let control = MessageBoardAnswerControlData {
        object: object_number(target),
        answer: LegacyCString::from_bytes(answer).expect("answer is NUL-free"),
        player: PLAYER,
        by_client: 7,
    };
    assert!(engine
        .execute_message_board_answer_control(&control)
        .expect("authenticated answer executes"));
    assert_eq!(
        i32::try_from(engine.frame()).expect("fixture frame fits i32"),
        frame,
        "the callback executes in the control frame without advancing simulation"
    );

    let target_state = engine
        .object_snapshot(target)
        .expect("callback target remains");
    assert_eq!(
        target_state.local_vars.get("callback_answer"),
        Some(&Value::String(
            clonk_script::c4_string_from_bytes(b"say \" ); SetGravity(99); // \\ \x80").into(),
        )),
        "quote/backslash escaping must be transparent without transcoding packet bytes"
    );
    assert_eq!(
        engine.physics().gravity,
        gravity,
        "answer text must not escape its string argument and execute injected script"
    );
    assert_eq!(
        target_state.local_vars.get("callback_player"),
        Some(&Value::Int(PLAYER))
    );
    assert_eq!(
        target_state.local_vars.get("callback_count"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        target_state.local_vars.get("callback_frame"),
        Some(&Value::Int(frame))
    );
    assert!(engine
        .player(PLAYER)
        .expect("message-board player remains")
        .message_board_queries()
        .is_empty());
}

#[test]
fn empty_message_board_answer_control_closes_input_and_only_consumes_the_query() {
    let (mut engine, target, _) = fixture();
    engine
        .player_mut(PLAYER)
        .expect("message-board player remains")
        .set_at_client(PlayerAtClient::new(7));
    assert_eq!(
        open(&mut engine, target, false, "cancel this query", PLAYER),
        Value::Bool(true)
    );
    for frame in 1..=35 {
        engine
            .tick_without_snapshot()
            .unwrap_or_else(|error| panic!("query activation tick {frame} succeeds: {error}"));
    }
    assert!(engine.active_message_board_input().is_some());

    let control = engine
        .prepare_message_board_answer_control(LegacyCString::default(), 7)
        .expect("active input produces its synchronized answer");
    assert_eq!(control.object, object_number(target));
    assert_eq!(control.player, PLAYER);
    assert_eq!(control.by_client, 7);
    assert!(
        engine.active_message_board_input().is_none(),
        "local submission closes the process-local type-in before execution"
    );
    let pending = engine
        .player(PLAYER)
        .expect("message-board player remains")
        .message_board_queries();
    assert_eq!(pending.len(), 1);
    assert!(pending[0].answered, "submission marks the query answered");
    assert_ne!(
        engine
            .object_snapshot(target)
            .expect("callback target remains")
            .local_vars
            .get("callback_count"),
        Some(&Value::Int(1)),
        "queuing the answer must not run the synchronized callback early"
    );

    assert!(engine
        .execute_message_board_answer_control(&control)
        .expect("authenticated empty answer executes"));
    assert!(engine
        .player(PLAYER)
        .expect("message-board player remains")
        .message_board_queries()
        .is_empty());
    assert_ne!(
        engine
            .object_snapshot(target)
            .expect("callback target remains")
            .local_vars
            .get("callback_count"),
        Some(&Value::Int(1)),
        "an omitted/empty third argument must not invoke InputCallback"
    );
}

#[test]
fn message_board_answer_submission_applies_cpp_uppercase_bytes_before_queueing() {
    let (mut engine, target, _) = fixture();
    engine
        .player_mut(PLAYER)
        .expect("message-board player remains")
        .set_at_client(PlayerAtClient::new(7));
    assert_eq!(
        open(&mut engine, target, true, "uppercase answer", PLAYER),
        Value::Bool(true)
    );
    for frame in 1..=35 {
        engine
            .tick_without_snapshot()
            .unwrap_or_else(|error| panic!("query activation tick {frame} succeeds: {error}"));
    }

    let control = engine
        .prepare_message_board_answer_control(
            LegacyCString::from_bytes(vec![b'a', 0xe4, 0xf6, 0xfc]).expect("answer is NUL-free"),
            7,
        )
        .expect("uppercase input produces a control");
    assert_eq!(control.answer.as_bytes(), &[b'A', 0xc4, 0xd6, 0xdc]);
}

#[test]
fn message_board_answer_control_preserves_cpp_internal_script_parse_failures() {
    for answer in [
        b"line\nbreak".to_vec(),
        b"line\rbreak".to_vec(),
        vec![b'x'; 1025],
    ] {
        let (mut engine, target, _) = fixture();
        engine
            .player_mut(PLAYER)
            .expect("message-board player remains")
            .set_at_client(PlayerAtClient::new(7));
        assert_eq!(
            open(
                &mut engine,
                target,
                false,
                "invalid internal source",
                PLAYER
            ),
            Value::Bool(true)
        );

        let control = MessageBoardAnswerControlData {
            object: object_number(target),
            answer: LegacyCString::from_bytes(answer).expect("answer is NUL-free"),
            player: PLAYER,
            by_client: 7,
        };
        assert!(engine
            .execute_message_board_answer_control(&control)
            .expect("the fail-safe DirectExec parse failure is non-fatal"));
        assert_eq!(
            engine
                .player(PLAYER)
                .expect("message-board player remains")
                .message_board_queries()
                .len(),
            1,
            "a strict-3 parse failure occurs before OnMessageBoardAnswer consumes the query"
        );
        assert_ne!(
            engine
                .object_snapshot(target)
                .expect("callback target remains")
                .local_vars
                .get("callback_count"),
            Some(&Value::Int(1)),
            "a malformed internal script must not invoke InputCallback"
        );
    }
}
