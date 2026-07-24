use super::*;

#[test]
fn explicit_owner_overlay_alpha_participates_in_solid_mask_pixels() {
    let image = DefinitionSpriteImage {
        width: 2,
        height: 1,
        pixels: Arc::from([10, 20, 30, 40, 50, 60, 70, 200]),
        color_mask: Some(Arc::from([128, 128, 128, 90, 255, 255, 255, 0])),
    };
    let pixels = image.solid_mask_source_pixels();
    assert_eq!(pixels[3], 130, "base and overlay opacity saturating-add");
    assert_eq!(pixels[7], 200, "transparent overlay leaves base alpha");

    let solid = solid_mask_pixels_for_checked_bitmap(
        DefinitionTargetRect::new(0, 0, 2, 1, 0, 0),
        2,
        1,
        &pixels,
    )
    .expect("solid mask pixels");
    assert_eq!(solid.as_slice(), &[1, 1]);
}
