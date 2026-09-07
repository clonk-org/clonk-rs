use std::cell::Cell;

use crate::{Definition, Engine, SpawnConfig, DEFINITION_DEEP_CLONES};

/// `C4Object::Call` resolves the callback on the definition's script in
/// place and never copies a `C4Def` (C4Object.cpp:2224-2240). The port's
/// callback gate used to clone every definition it touched — the whole
/// script tree, DefCore tables and graphics handles — once per callback, and
/// again per continuation pass. On Seven Keys (621 objects) that copying was
/// about a quarter of every simulation frame.
#[test]
fn object_timer_callbacks_do_not_deep_copy_their_definition() {
    let mut engine = Engine::new();
    let mut ticker = Definition::from_script(
        "TICK",
        "Ticker",
        "#strict 2\nlocal count;\nfunc Timer() { count++; }\n",
    )
    .expect("ticker definition compiles");
    ticker.set_timer(1);
    ticker.set_timer_call(Some("Timer".to_owned()));
    engine
        .register_definition(ticker)
        .expect("ticker definition registers");
    engine
        .spawn_object(SpawnConfig::new("TICK"))
        .expect("ticker object spawns");

    DEFINITION_DEEP_CLONES.with(|count| count.set(0));
    for _ in 0..10 {
        engine.tick_without_snapshot().expect("frame advances");
    }
    assert_eq!(
        DEFINITION_DEEP_CLONES.with(Cell::get),
        0,
        "object callbacks must share the definition, not copy it"
    );
}
