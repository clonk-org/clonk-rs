use super::*;

/// `C4DefGraphics::Update` re-resolves by name in three steps
/// (oracle-src-pinned `src/C4DefGraphics.cpp:362-368`):
///
/// ```cpp
/// if (!pObj->SetGraphics(Name, pDef))            // the reloaded definition
///     if (!pObj->SetGraphics(Name, pObj->Def))   // the object's OWN definition
///     { pObj->AssignRemoval(); pObj->pGraphics = nullptr; }
/// ```
///
/// and `SetGraphics(name, def)` fails exactly when that definition has no
/// graphic of that name (`src/C4Object.cpp:5894-5903`).
///
/// The port skipped the middle step. It removed every object whose graphic did
/// not survive on the *reloaded* definition, without first offering the
/// object's own definition — so a borrowed graphic that the object's own
/// definition also supplies took the object down with it
/// (clonk-org/clonk-rs#1094).
#[test]
fn a_reload_offers_the_objects_own_definition_before_removing_it() {
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 200));

    // The reloaded definition, which will lose the "Alt" graphic.
    crate::TestValueExt::test_value(engine.register_script_definition(
        "SRCE",
        "Graphics source",
        "#strict\n",
    ));
    // The object's own definition, which supplies "Alt" itself.
    crate::TestValueExt::test_value(engine.register_script_definition(
        "OWNR",
        "Graphics owner",
        "#strict\n",
    ));
    let mut variants = std::collections::HashMap::new();
    variants.insert(
        "Alt".to_string(),
        crate::definition::DefinitionSpriteImage {
            width: 1,
            height: 1,
            pixels: std::sync::Arc::from(vec![0u8; 4]),
            color_mask: None,
        },
    );
    engine
        .definitions
        .get_mut("OWNR")
        .expect("the owner definition is registered")
        .set_sprite_variants(variants);

    let id = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("OWNR").with_position(Vector2::new(100, 100))),
    );
    // The object borrows SRCE's "Alt" graphic, which the reload drops.
    let index = engine.find_object_index(id).expect("the object is live");
    engine.objects[index].state.base_graphics = Some(crate::ObjectBaseGraphics {
        definition: "SRCE".into(),
        graphics_name: Some("Alt".into()),
        blit_mode: 0,
    });

    engine.reassign_graphics_after_reload("SRCE");

    let index = engine
        .find_object_index(id)
        .expect("the object survives: its own definition supplies `Alt`");
    let graphics = engine.objects[index]
        .state
        .base_graphics
        .as_ref()
        .expect("it still carries a graphic");
    assert_eq!(
        graphics.definition.as_str(),
        "OWNR",
        "the second SetGraphics rebinds it onto the object's own definition"
    );
    assert_eq!(graphics.graphics_name.as_deref(), Some("Alt"));
}

/// The other arm of the same three-step chain: when the object's own
/// definition *is* the reloaded one, the second `SetGraphics(Name, pObj->Def)`
/// checks the very set that just failed, so it fails too and native runs
/// `AssignRemoval()` (oracle-src-pinned `src/C4DefGraphics.cpp:362-368`).
///
/// Only a *named* variant can reach this. The default graphic is backed up
/// with an empty name and `C4DefGraphics::Get("")` returns the base graphics,
/// which always re-resolves — which is why an ordinary reload never removes
/// anything.
///
/// The port used to clear the name here and keep the object on the default
/// graphic instead, so a reload left live objects a stock peer had removed
/// (clonk-org/clonk-rs#1094).
#[test]
fn a_reload_removes_an_object_whose_own_definition_cannot_supply_the_name() {
    let mut engine = Engine::with_seed(0);
    engine.set_landscape(Landscape::flat(400, 200));
    crate::TestValueExt::test_value(engine.register_script_definition(
        "SOLO",
        "Sole definition",
        "#strict\n",
    ));

    let id = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("SOLO").with_position(Vector2::new(100, 100))),
    );
    let index = engine.find_object_index(id).expect("the object is live");
    engine.objects[index].state.base_graphics = Some(crate::ObjectBaseGraphics {
        definition: "SOLO".into(),
        graphics_name: Some("Gone".into()),
        blit_mode: 0,
    });

    engine.reassign_graphics_after_reload("SOLO");

    assert!(
        engine
            .find_object_index(id)
            .is_none_or(|index| !engine.objects[index].state.status.is_active()),
        "native AssignRemoval()s an object no definition can supply a graphic for"
    );
}
