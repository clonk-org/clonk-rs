use super::*;
use crate::lib_test_support::{register_fixture, spawn_fixture, EngineTestExt};

/// A denied effect still consumes its number for the rest of the frame.
///
/// `C4Effect`'s constructor inserts the new effect into the list and allocates
/// its number *before* asking higher-priority effects whether it may exist
/// (`C4Effect.cpp:74-89`). When `Check` answers `C4Fx_Effect_Deny` the
/// constructor returns early (`:107-114`) — it does **not** unlink what it
/// inserted. The effect simply never reaches `iPriority = iPrio`
/// (`:127`), so it stays at the constructor's initial `iPriority = 0`, and
/// `IsDead()` is exactly `!iPriority` (`C4Effects.h:110`).
///
/// Dead effects are reaped by `C4Effect::Execute`, whose own comment on
/// `SetDead` says "mark effect to be removed in **next execution cycle**"
/// (`C4Effects.h:109`; the reap is `C4Effect.cpp:326-334`). So for the
/// remainder of the current frame the denied effect is still a list member —
/// and number allocation scans the whole list, dead entries included
/// (`C4Effect.cpp:76-78`).
///
/// The consequence is script-visible: a second `AddEffect` in the same frame
/// must not reuse the denied effect's number. `AddEffect` hands that number
/// back, and scripts key `GetEffect`/`EffectVar` off it, so reusing it would
/// silently alias two different effects across a frame boundary.
#[test]
fn a_denied_effect_still_reserves_its_number_for_the_frame() {
    let mut engine = Engine::with_seed(0);
    register_fixture!(
        engine,
        "FXDN",
        "Effect deny numbering fixture",
        r#"#strict 3
    local blocker, denied, later;

    public func Arm()
    {
        blocker = AddEffect("Block", this(), 100, 0, this());
        denied = AddEffect("Denied", this(), 10, 0, this());
        later = AddEffect("Later", this(), 10, 0, this());
        return 0;
    }

    public func Blocker() { return blocker; }
    public func Denied() { return denied; }
    public func Later() { return later; }

    func FxBlockEffect(string new_name, object target, int number)
    {
        // Deny only the middle effect; the third must be admitted so its
        // number is observable.
        if (new_name == "Denied") return -1;
        return 0;
    }
    "#,
        set_c4_callback_convention(true)
    );

    let object = spawn_fixture!(engine, "FXDN");
    let index = engine.test_object_index(object);
    crate::TestValueExt::test_value(engine.call_object_function(index, "Arm", Vec::new()));

    let number = |engine: &mut Engine, name: &str| {
        crate::compat::value_as_i32(&crate::TestValueExt::test_value(
            engine.call_object_function(index, name, Vec::new()),
        ))
    };

    assert_eq!(
        number(&mut engine, "Blocker"),
        1,
        "the first effect takes number 1"
    );
    assert_eq!(
        number(&mut engine, "Denied"),
        0,
        "a denied AddEffect reports 0: riStoredAsNumber is only assigned when \
         Check did not answer Deny (C4Effect.cpp:110-113)"
    );
    // The invariant the numbering rests on, asserted directly: C++ does not
    // unlink what its constructor inserted, so the denied effect is still a
    // list member — dead, awaiting the next execution cycle.
    let effects = &engine.objects[index].state.effects;
    let denied_entry = effects
        .iter()
        .find(|effect| effect.number == 2)
        .expect("the denied effect stays in the list holding number 2");
    assert_eq!(denied_entry.name, "Denied");
    assert_eq!(
        denied_entry.priority, 0,
        "a denied effect never reaches iPriority = iPrio (C4Effect.cpp:127),          so it reads as dead: IsDead() is !iPriority (C4Effects.h:110)"
    );

    assert_eq!(
        number(&mut engine, "Later"),
        3,
        "the denied effect is still in the list holding number 2, so the next \
         effect takes 3 — reusing 2 would alias it with an entry that is not \
         reaped until the next execution cycle"
    );
}
