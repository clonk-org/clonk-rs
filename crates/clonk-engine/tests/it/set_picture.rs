use crate::support::EngineTestExt;
use clonk_engine::{
    Definition, DefinitionPicture, DefinitionSpriteImage, DrawTransform, Engine,
    GraphicsOverlayMode, ObjectBaseGraphics, ObjectGraphicsOverlay, ObjectId, ObjectUpdate,
    PlayerConfig, RgbColor, SpawnConfig, APS_COLOR, APS_GRAPHICS, APS_NAME, APS_OVERLAY,
};
use clonk_script::Value;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

fn picture_engine() -> (Engine, ObjectId, ObjectId) {
    let mut engine = Engine::new();
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "PICT",
            "Picture probe",
            r#"
                #strict

                public func SetOwnPicture()
                {
                    return SetPicture(1, -2, 30, 40);
                }

                public func SetOtherPicture(object target)
                {
                    return SetPicture(-5, 6, 70, 80, target);
                }

                public func ClearOwnPicture()
                {
                    return SetPicture(0, 0, 0, 0);
                }

                public func SetOwnModulation(int color)
                {
                    return SetClrModulation(color);
                }

                public func GetOwnModulation()
                {
                    return GetClrModulation();
                }

                public func SetOtherModulation(int color, object target, int overlay_id)
                {
                    return SetClrModulation(color, target, overlay_id);
                }

                public func GetOtherModulation(object target, int overlay_id)
                {
                    return GetClrModulation(target, overlay_id);
                }
                "#,
        ),
    ));
    let first = engine.spawn_test_object(SpawnConfig::new("PICT"));
    let second = engine.spawn_test_object(SpawnConfig::new("PICT"));
    (engine, first, second)
}

fn serialized_picture(engine: &Engine, object: ObjectId) -> serde_json::Value {
    crate::support::TestValueExt::test_value(serde_json::to_value(
        engine.test_object_snapshot(object),
    ))["picture_rect"]
        .clone()
}

#[test]
fn set_picture_updates_local_and_explicit_foreign_objects() {
    // FnSetPicture uses the explicit pObj when non-null, otherwise cthr->Obj,
    // and writes all four rect components verbatim (src/C4Script.cpp:3708-3715).
    let (mut engine, first, second) = picture_engine();
    let first_index = engine.test_object_index(first);

    assert_eq!(
        engine.call_test_object_function(first_index, "SetOwnPicture", Vec::new()),
        Value::Bool(true)
    );
    assert_eq!(
        serialized_picture(&engine, first),
        json!({"x": 1, "y": -2, "width": 30, "height": 40})
    );

    assert_eq!(
        engine.call_test_object_function(
            first_index,
            "SetOtherPicture",
            vec![Value::Object(second.as_u64())],
        ),
        Value::Bool(true)
    );
    assert_eq!(
        serialized_picture(&engine, second),
        json!({"x": -5, "y": 6, "width": 70, "height": 80})
    );
}

#[test]
fn picture_rect_round_trips_and_zero_width_remains_the_default_sentinel() {
    // C4Object::Clear defaults PictureRect to zero, CompileFunc persists it as
    // `Picture`, and Picture2Facet falls back to Def->PictureRect iff Wdt is
    // zero (src/C4Object.cpp:128,2798,3123-3127).
    let (mut engine, first, _) = picture_engine();
    let first_index = engine.test_object_index(first);
    engine.call_test_object_function(first_index, "SetOwnPicture", Vec::new());

    let state = engine.capture_state();
    let mut restored = picture_engine().0;
    crate::support::TestValueExt::test_value(restored.restore_state(&state));
    assert_eq!(
        serialized_picture(&restored, first),
        json!({"x": 1, "y": -2, "width": 30, "height": 40})
    );

    let restored_index = restored.test_object_index(first);
    restored.call_test_object_function(restored_index, "ClearOwnPicture", Vec::new());
    assert_eq!(
        serialized_picture(&restored, first),
        json!({"x": 0, "y": 0, "width": 0, "height": 0})
    );
}

#[test]
fn color_modulation_is_live_persistent_and_prevents_picture_stacking() {
    // FnSet/GetClrModulation read and write C4Object::ColorMod, while
    // CanConcatPictureWith rejects unequal modulation unless APS_Color is set
    // (src/C4Script.cpp:3880-3921; src/C4Object.cpp:6179-6186,2816).
    let (mut engine, first, second) = picture_engine();
    let first_index = engine.test_object_index(first);
    assert_eq!(
        engine.call_test_object_function(first_index, "GetOwnModulation", Vec::new()),
        Value::Int(0)
    );
    assert_eq!(
        engine.call_test_object_function(
            first_index,
            "SetOwnModulation",
            vec![Value::Int(0x1122_3344)],
        ),
        Value::Bool(true)
    );
    assert_eq!(
        engine.call_test_object_function(first_index, "GetOwnModulation", Vec::new()),
        Value::Int(0x1122_3344)
    );

    let first_snapshot = engine.test_object_snapshot(first);
    let second_snapshot = engine.test_object_snapshot(second);
    assert_eq!(first_snapshot.color_modulation, 0x1122_3344);
    assert!(!engine.can_concat_picture_with(&first_snapshot, &second_snapshot));

    let state = engine.capture_state();
    let mut restored = picture_engine().0;
    crate::support::TestValueExt::test_value(restored.restore_state(&state));
    assert_eq!(
        restored.test_object_snapshot(first).color_modulation,
        0x1122_3344
    );
}

#[test]
fn clr_modulation_targets_existing_overlays_without_creating_missing_ones() {
    // Overlay IDs are looked up without creation; missing overlays return
    // false/nil (src/C4Script.cpp:3885-3894,3909-3917).
    let (mut engine, first, second) = picture_engine();
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        second,
        ObjectUpdate {
            graphics_overlays: Some(vec![ObjectGraphicsOverlay::new(
                9,
                GraphicsOverlayMode::Picture,
            )]),
            ..ObjectUpdate::default()
        },
    ));
    let first_index = engine.test_object_index(first);
    assert_eq!(
        engine.call_test_object_function(
            first_index,
            "SetOtherModulation",
            vec![
                Value::Int(0x0055_6677),
                Value::Object(second.as_u64()),
                Value::Int(9)
            ],
        ),
        Value::Bool(true)
    );
    assert_eq!(
        engine.call_test_object_function(
            first_index,
            "GetOtherModulation",
            vec![Value::Object(second.as_u64()), Value::Int(9)],
        ),
        Value::Int(0x0055_6677)
    );
    assert_eq!(
        engine.call_test_object_function(
            first_index,
            "SetOtherModulation",
            vec![
                Value::Int(1),
                Value::Object(second.as_u64()),
                Value::Int(10)
            ],
        ),
        Value::Bool(false)
    );
    assert_eq!(
        engine.call_test_object_function(
            first_index,
            "GetOtherModulation",
            vec![Value::Object(second.as_u64()), Value::Int(10)],
        ),
        Value::Nil
    );
}

fn stack_engine(allow_picture_stack: i32) -> (Engine, ObjectId, ObjectId) {
    let mut engine = Engine::new();
    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "STAK",
        "Stack probe",
        "#strict",
    ));
    definition.set_color_by_owner(true);
    definition.set_allow_picture_stack(allow_picture_stack);
    definition.set_sprite_variants(HashMap::from([(
        "alternate".to_string(),
        DefinitionSpriteImage {
            width: 1,
            height: 1,
            pixels: Arc::from(vec![0_u8; 4]),
            color_mask: None,
        },
    )]));
    engine.register_test_definition(definition);
    let first = engine.spawn_test_object(SpawnConfig::new("STAK"));
    let second = engine.spawn_test_object(SpawnConfig::new("STAK"));
    (engine, first, second)
}

fn snapshots_stack(engine: &Engine, first: ObjectId, second: ObjectId) -> bool {
    engine.can_concat_picture_with(
        &engine.test_object_snapshot(first),
        &engine.test_object_snapshot(second),
    )
}

#[test]
fn allow_picture_stack_exempts_exact_cpp_comparison_groups() {
    // DefCore AllowPictureStack exempts color/modulation/blit, graphics/rect,
    // name, and picture-overlay checks independently
    // (src/C4Def.cpp:419-429; src/C4Object.cpp:6173-6213).
    let (mut engine, first, second) = stack_engine(0);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            color: Some(0x00aa_bbcc),
            color_modulation: Some(0x4050_6070),
            blit_mode: Some(128),
            ..ObjectUpdate::default()
        },
    ));
    assert!(!snapshots_stack(&engine, first, second));

    let (mut engine, first, second) = stack_engine(APS_COLOR);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            color: Some(0x00aa_bbcc),
            color_modulation: Some(0x4050_6070),
            blit_mode: Some(128),
            ..ObjectUpdate::default()
        },
    ));
    assert!(snapshots_stack(&engine, first, second));

    let graphics = ObjectBaseGraphics {
        definition: "STAK".to_string(),
        graphics_name: Some("Alternate".to_string()),
        blit_mode: 0,
    };
    let (mut engine, first, second) = stack_engine(0);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            picture_rect: Some(clonk_engine::DefinitionRect::new(1, 2, 3, 4)),
            base_graphics: Some(Some(graphics.clone())),
            ..ObjectUpdate::default()
        },
    ));
    assert!(!snapshots_stack(&engine, first, second));

    let (mut engine, first, second) = stack_engine(APS_GRAPHICS);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            picture_rect: Some(clonk_engine::DefinitionRect::new(1, 2, 3, 4)),
            base_graphics: Some(Some(graphics)),
            ..ObjectUpdate::default()
        },
    ));
    assert!(snapshots_stack(&engine, first, second));

    let (mut engine, first, second) = stack_engine(0);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            custom_name: Some(Some("First".to_string())),
            ..ObjectUpdate::default()
        },
    ));
    assert!(!snapshots_stack(&engine, first, second));

    let (mut engine, first, second) = stack_engine(APS_NAME);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            custom_name: Some(Some("First".to_string())),
            ..ObjectUpdate::default()
        },
    ));
    assert!(snapshots_stack(&engine, first, second));

    let mut first_overlay = ObjectGraphicsOverlay::new(7, GraphicsOverlayMode::Picture)
        .with_definition(Some("STAK".to_string()));
    let mut second_overlay = first_overlay.clone();
    first_overlay.color_modulation = 0x0011_2233;
    second_overlay.color_modulation = 0x0044_5566;
    let (mut engine, first, second) = stack_engine(0);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            graphics_overlays: Some(vec![first_overlay.clone()]),
            ..ObjectUpdate::default()
        },
    ));
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        second,
        ObjectUpdate {
            graphics_overlays: Some(vec![second_overlay.clone()]),
            ..ObjectUpdate::default()
        },
    ));
    assert!(!snapshots_stack(&engine, first, second));

    let (mut engine, first, second) = stack_engine(APS_OVERLAY);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            graphics_overlays: Some(vec![first_overlay]),
            ..ObjectUpdate::default()
        },
    ));
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        second,
        ObjectUpdate {
            graphics_overlays: Some(vec![second_overlay]),
            ..ObjectUpdate::default()
        },
    ));
    assert!(snapshots_stack(&engine, first, second));
}

#[test]
fn picture_overlay_phase_does_not_split_stacks() {
    // C4GraphicsOverlay::operator== deliberately ignores the current phase
    // so animation state can be concatenated (src/C4DefGraphics.cpp:868-878).
    let (mut engine, first, second) = stack_engine(0);
    let mut first_overlay = ObjectGraphicsOverlay::new(7, GraphicsOverlayMode::Picture)
        .with_definition(Some("STAK".to_string()));
    let mut second_overlay = first_overlay.clone();
    first_overlay.phase = 1;
    second_overlay.phase = 9;
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            graphics_overlays: Some(vec![first_overlay]),
            ..ObjectUpdate::default()
        },
    ));
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        second,
        ObjectUpdate {
            graphics_overlays: Some(vec![second_overlay]),
            ..ObjectUpdate::default()
        },
    ));
    assert!(snapshots_stack(&engine, first, second));
}

#[test]
fn serialized_picture_defaults_resolve_to_cpp_stack_equivalence() {
    // C++ compares resolved C4DefGraphics pointers, its graphics lookup is
    // case-insensitive, every overlay owns an identity transform, and an
    // empty CustomName falls back through GetName. Rust snapshots omit those
    // defaults, so normalize the representation before comparing
    // (src/C4DefGraphics.cpp:221-229,868-878; src/C4Object.cpp:6173-6213).
    let (mut engine, first, second) = stack_engine(0);
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        first,
        ObjectUpdate {
            custom_name: Some(Some(String::new())),
            base_graphics: Some(Some(ObjectBaseGraphics {
                definition: "STAK".to_string(),
                graphics_name: Some("Alternate".to_string()),
                blit_mode: 0,
            })),
            graphics_overlays: Some(vec![ObjectGraphicsOverlay::new(
                    7,
                    GraphicsOverlayMode::Picture,
                )
                .with_definition(Some("STAK".to_string()))
                .with_graphics_name(Some("Alternate".to_string()))]),
            ..ObjectUpdate::default()
        },
    ));
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        second,
        ObjectUpdate {
            base_graphics: Some(Some(ObjectBaseGraphics {
                definition: "STAK".to_string(),
                graphics_name: Some("alternate".to_string()),
                blit_mode: 0,
            })),
            graphics_overlays: Some(vec![ObjectGraphicsOverlay::new(
                    7,
                    GraphicsOverlayMode::Picture,
                )
                .with_definition(Some("STAK".to_string()))
                .with_graphics_name(Some("alternate".to_string()))
                .with_action(Some(String::new()))
                .with_transform(Some(DrawTransform::identity()))]),
            ..ObjectUpdate::default()
        },
    ));

    assert!(
        snapshots_stack(&engine, first, second),
        "case-only graphics names, omitted defaults, and an empty custom name resolve identically"
    );

    let mut changed_overlay = ObjectGraphicsOverlay::new(7, GraphicsOverlayMode::Picture)
        .with_definition(Some("STAK".to_string()))
        .with_graphics_name(Some("alternate".to_string()));
    changed_overlay.transform = Some(DrawTransform::from_components(2.0, 1.0, 0.0, 0.0));
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        second,
        ObjectUpdate {
            graphics_overlays: Some(vec![changed_overlay]),
            ..ObjectUpdate::default()
        },
    ));
    assert!(
        !snapshots_stack(&engine, first, second),
        "a genuinely different native transform still splits the stack"
    );
}

#[test]
fn fresh_color_by_owner_objects_copy_the_live_player_color() {
    // C4Object::Init copies C4Player::ColorDw for ColorByOwner definitions;
    // savegame-loaded objects compile ColorDw verbatim instead
    // (src/C4Object.cpp:201-204,2733-2787).
    let mut engine = Engine::new();
    engine.register_test_player(
        PlayerConfig::new(0, "Red").with_color(Some(RgbColor::new(0xaa, 0x22, 0x44))),
    );
    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "COLR",
        "Color probe",
        "#strict",
    ));
    definition.set_color_by_owner(true);
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(SpawnConfig::new("COLR").with_owner(0));
    assert_eq!(engine.test_object_snapshot(object).color, 0x00aa_2244);
}

#[test]
fn object_picture_rect_uses_the_definition_graphics_scale() {
    // Picture2Facet scales all four source-rect components by C4Def::Scale
    // before selecting pixels (src/C4Object.cpp:3123-3129;
    // src/C4Rect.cpp:37-45).
    let mut engine = Engine::new();
    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "SCAL",
        "Scale probe",
        "#strict",
    ));
    let pixels: Vec<u8> = (0..8)
        .flat_map(|y| (0..8).flat_map(move |x| [x as u8, y as u8, (x + y) as u8, 0xff]))
        .collect();
    definition.set_sprite_image(Some(DefinitionSpriteImage {
        width: 8,
        height: 8,
        pixels: Arc::from(pixels.into_boxed_slice()),
        color_mask: None,
    }));
    definition.set_picture(Some(DefinitionPicture {
        x: 1,
        y: 1,
        width: 2,
        height: 2,
    }));
    definition.set_graphics_scale(2.0);
    engine.register_test_definition(definition);
    let object = engine.spawn_test_object(SpawnConfig::new("SCAL"));
    let snapshot = engine.test_object_snapshot(object);
    let picture = crate::support::TestValueExt::test_value(engine.object_picture_image(&snapshot));
    assert_eq!((picture.width(), picture.height()), (4, 4));
    assert_eq!(&picture.pixels()[..4], &[2, 2, 4, 0xff]);
}
