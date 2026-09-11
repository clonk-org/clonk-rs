#[test]
fn running_state_reads_live_player_name_without_projecting_the_world() {
    // C4PlayerList::JoinNew logs the live player's name
    // (C4PlayerList.cpp:281), without constructing presentation state.
    let mut app = new_running_sandbox_app();
    app.engine = Engine::new();
    app.engine
        .register_player(PlayerConfig::new(app.players.local_owner, "Live player"))
        .test_value();
    assert_eq!(app.engine.snapshot_timings().total, Duration::ZERO);

    app.configure_running_state("Next game".to_string(), DEFAULT_GROUND_HEIGHT);

    assert_eq!(app.engine.snapshot_timings().total, Duration::ZERO);
    assert!(app
        .chat
        .message_board
        .log_history
        .iter()
        .any(|line| line.ends_with("Player join: Live player")));
}
