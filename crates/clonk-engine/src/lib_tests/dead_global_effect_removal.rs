//! When a killed global effect leaves the list.
//!
//! `C4Effect::Execute` walks the list from the head. A node the timer kills is
//! deliberately **left linked** — `Kill` marks it dead and the walk then steps
//! past it (`C4Effect.cpp:344-360`) — and it is deleted at the *top* of the
//! next pass by the `IsDead()` branch (`:326-334`).
//!
//! So a killed effect is observable for exactly one further frame. Holding it
//! longer is synchronized state, since the effect list is compared, and the
//! shadow diff has reported it twice on real scenarios
//! (clonk-org/clonk-rs#1087).

use crate::{EffectState, Engine};

/// A global effect whose timer kills it the first time it runs.
fn suicidal_global_effect(engine: &mut Engine) {
    assert_eq!(
        engine.install_additional_global_scripts(&[(
            "DeadGlobalEffect.c".to_string(),
            r#"#strict 2
global func FxSuicideTimer(target, int number, int time)
{
  return(-1);
}
"#
            .to_string(),
        )]),
        1
    );
    let mut effect = EffectState::new("Suicide").with_interval(1);
    effect.number = 91;
    engine.global_effects.push(effect);
}

fn global_effect_names(engine: &Engine) -> Vec<(String, i32)> {
    engine
        .global_effects
        .iter()
        .map(|effect| (effect.name.clone(), effect.priority))
        .collect()
}

#[test]
fn a_killed_global_effect_is_gone_one_frame_later() {
    let mut engine = Engine::with_seed(0);
    suicidal_global_effect(&mut engine);

    // The frame the timer runs: `Kill` marks the node dead and the walk steps
    // past it, so it is still linked, at priority zero.
    crate::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        global_effect_names(&engine),
        vec![("Suicide".to_string(), 0)],
        "the killed effect stays linked for the frame it died in, as C4Effect::Kill leaves it"
    );

    // The next pass deletes it before executing anything.
    crate::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        global_effect_names(&engine),
        Vec::new(),
        "the next execution pass unlinks it, as C4Effect::Execute's IsDead branch does"
    );
}
