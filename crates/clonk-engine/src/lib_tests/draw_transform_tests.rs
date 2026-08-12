use super::DrawTransform;

#[test]
fn legacy_serialized_transform_defaults_new_matrix_components() {
    let transform: DrawTransform = crate::TestValueExt::test_value(serde_json::from_str(
        r#"{"scale_x":2.0,"scale_y":3.0,"offset_x":4.0,"offset_y":5.0}"#,
    ));

    assert_eq!(
        transform.matrix(),
        [2.0, 0.0, 4.0, 0.0, 3.0, 5.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        serde_json::to_value(transform).expect("draw transform serializes"),
        serde_json::json!({
                "scale_x": 2.0,
                "scale_y": 3.0,
                "offset_x": 4.0,
                "offset_y": 5.0,
        })
    );
}

#[test]
fn full_matrix_round_trips_through_serde() {
    let transform = DrawTransform::from_matrix([1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    let serialized = crate::TestValueExt::test_value(serde_json::to_string(&transform));
    let decoded: DrawTransform = crate::TestValueExt::test_value(serde_json::from_str(&serialized));

    assert_eq!(decoded.matrix(), transform.matrix());
    assert_eq!(decoded.flip_dir(), 1);
}

#[test]
fn flip_dir_and_projective_row_round_trip() {
    let transform = DrawTransform::from_matrix_with_flip_dir(
        [-1.0, 0.25, 3.0, 0.5, 2.0, 4.0, 0.01, 0.02, 0.75],
        -1,
    );
    assert!(!transform.is_identity());
    let encoded = crate::TestValueExt::test_value(serde_json::to_string(&transform));
    let decoded: DrawTransform = crate::TestValueExt::test_value(serde_json::from_str(&encoded));
    assert_eq!(decoded, transform);
}
