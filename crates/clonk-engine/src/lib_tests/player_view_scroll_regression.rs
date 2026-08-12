use super::*;

#[test]
fn engine_scroll_player_view_uses_landscape_dimensions() {
    let mut engine = Engine::new();
    engine.set_landscape(Landscape::flat(1_000, 1_000));
    crate::TestValueExt::test_value(engine.register_player(PlayerConfig::new(7, "Player")));
    crate::TestValueExt::test_value(
        engine.replace_player_viewports(7, vec![PlayerViewport::new(Vector2::new(15, 995))]),
    );

    crate::TestValueExt::test_value(engine.scroll_player_view(
        7,
        Vector2::new(-10, 10),
        100,
        80,
        true,
    ));

    let state = crate::TestValueExt::test_value(engine.player(7)).to_state();
    assert_eq!(state.view_mode, PLAYER_VIEW_MODE_SCROLLING);
    assert_eq!(state.viewports[0].center, Vector2::new(10, 1_000));
}

#[test]
fn engine_scroll_player_view_rejects_an_unknown_player() {
    let mut engine = Engine::new();
    let error = engine
        .scroll_player_view(42, Vector2::new(10, 0), 100, 80, false)
        .expect_err("unknown player is rejected");
    assert!(matches!(error, EngineError::UnknownPlayer(42)));
}
