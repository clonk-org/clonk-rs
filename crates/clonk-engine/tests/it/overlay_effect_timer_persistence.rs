//! An object effect timer that writes ONE graphics overlay must leave the
//! object's other overlays alone.
//!
//! C++ splices a single node into the live sorted `C4Object::pGfxOverlay` list
//! (src/C4Object.cpp:5962-5977 `GetGraphicsOverlay`) and unlinks a single node
//! on removal (:5979-5995 `RemoveGraphicsOverlay`); overlays the callback never
//! names are untouched.
//!
//! ClonkMars 01_Fossae depends on this: MHUD's `FxMovePointerTimer` (interval 1)
//! rewrites overlays 4 and 9 every frame, while overlay 1 (HUD_O2) is written
//! only every 20 frames by the Spaceclonk through `UpdateHUD` -> `HUD->UpdateO2`.
//! An effect-callback scope seeded with an empty overlay list publishes its whole
//! list (compat/contexts.rs:8892) and deletes overlay 1 on the very next frame.

use clonk_engine::{
    Definition, Engine, GraphicsOverlayMode, ObjectGraphicsOverlay, ObjectUpdate, SpawnConfig,
};

const HUD: &str = r#"
#strict

public func Arm()
{
    return AddEffect("MovePointer", this, 100, 1, this);
}

public func FxMovePointerTimer(object target, int number, int time)
{
    SetGraphics(0, this, GetID(), 9, GFXOV_MODE_Action, "Pointer");
    return 1;
}
"#;

#[test]
fn effect_timer_overlay_write_keeps_the_objects_other_overlays() {
    let mut engine = Engine::new();
    let mut hud_definition = Definition::from_script("MHUD", "HUD", HUD).expect("HUD compiles");
    hud_definition.set_c4_callback_convention(true);
    engine
        .register_definition(hud_definition)
        .expect("HUD registers");

    let hud = engine
        .spawn_object(SpawnConfig::new("MHUD"))
        .expect("HUD spawns");

    // Overlay 1 is authoritative object state, exactly as a foreign
    // `HUD->UpdateO2(...)` write leaves it.
    engine
        .apply_object_update(
            hud,
            ObjectUpdate {
                graphics_overlays: Some(vec![ObjectGraphicsOverlay::new(
                    1,
                    GraphicsOverlayMode::Action,
                )
                .with_action(Some("O20".to_string()))]),
                ..ObjectUpdate::default()
            },
        )
        .expect("O2 overlay installs");

    let hud_index = engine.find_object_index(hud).expect("HUD present");
    engine
        .call_object_function(hud_index, "Arm", vec![])
        .expect("pointer effect arms");

    for _ in 0..5 {
        engine.tick_without_snapshot().expect("frame runs");
    }

    assert_eq!(
        engine
            .object_snapshot(hud)
            .expect("HUD survives")
            .graphics_overlays
            .iter()
            .map(|overlay| overlay.id)
            .collect::<Vec<_>>(),
        vec![1, 9],
        "the effect timer wrote overlay 9; overlay 1 must survive \
         (C4Object.cpp:5962-5977 splices one node)"
    );
}
