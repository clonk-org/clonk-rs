//! A global effect killed *after* the walk's cursor has passed it.
//!
//! `C4Effect::Execute` walks from the head, and a node it steps past this frame
//! is only reconsidered on the next pass. So an effect removed by a *later*
//! effect's timer is dead behind the cursor, and C++ unlinks it at the top of
//! the following pass like any other dead node (`C4Effect.cpp:326-334`).
//!
//! This is the case the single-effect test cannot reach, and the one that
//! matches the shape the shadow diff reports — a dead node sitting alongside a
//! live one (clonk-org/clonk-rs#1087).

use crate::{EffectState, Engine};

fn two_global_effects(engine: &mut Engine) {
    assert_eq!(
        engine.install_additional_global_scripts(&[(
            "CursorProbe.c".to_string(),
            r#"#strict 2
global func FxVictimTimer(target, int number, int time)
{
  return(0);
}
global func FxKillerTimer(target, int number, int time)
{
  RemoveEffect("Victim");
  return(0);
}
"#
            .to_string(),
        )]),
        1
    );
    // Pushed in walk order: Victim executes first, so the cursor is past it by
    // the time Killer's timer removes it.
    let mut victim = EffectState::new("Victim").with_interval(1);
    victim.number = 1;
    engine.global_effects.push(victim);
    let mut killer = EffectState::new("Killer").with_interval(1);
    killer.number = 2;
    engine.global_effects.push(killer);
}

fn listing(engine: &Engine) -> Vec<(String, i32)> {
    engine
        .global_effects
        .iter()
        .map(|effect| (effect.name.clone(), effect.priority))
        .collect()
}

#[test]
fn an_effect_killed_behind_the_cursor_is_unlinked_on_the_next_pass() {
    let mut engine = Engine::with_seed(0);
    two_global_effects(&mut engine);

    crate::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        listing(&engine),
        vec![("Victim".to_string(), 0), ("Killer".to_string(), 100)],
        "the node killed behind the cursor stays linked for the frame it died in"
    );

    crate::TestValueExt::test_value(engine.tick_without_snapshot());
    assert_eq!(
        listing(&engine),
        vec![("Killer".to_string(), 100)],
        "the following pass unlinks it, leaving the live effect alone"
    );
}
