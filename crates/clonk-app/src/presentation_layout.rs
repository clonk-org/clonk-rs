//! Strict semantic-layout traces for presentation screens whose port-authored
//! artwork makes pixel equality the wrong comparison term.

use crate::presentation_captures::{geometry, port_assets, screens, ComparisonTerm};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Versioned identity of the semantic layout artifact.
pub const LAYOUT_TRACE_SCHEMA: &str = "clonk-rs/presentation-layout/v1";

pub(crate) fn expected_port_asset_exemptions(screen: &str) -> Option<BTreeMap<String, String>> {
    let entries: &[(&str, &str)] = match screen {
        "startup-main" => &[
            ("startup/main/branding/logo", "branding"),
            ("startup/main/branding/version", "branding"),
            ("startup/main/branding/fan-project", "branding"),
        ],
        "startup-scenario-selection" => &[(
            "startup/scenario-selection/background",
            "super-resolved-startup-art",
        )],
        "startup-network-browser" => &[(
            "startup/network-browser/background",
            "super-resolved-startup-art",
        )],
        "startup-player-selection" => &[(
            "startup/player-selection/background",
            "super-resolved-startup-art",
        )],
        "startup-options" => &[("startup/options/tabs/paper", "super-resolved-startup-art")],
        "startup-about" => &[("startup/about/branding/fan-project", "branding")],
        "network-lobby" | "loader" | "hud" | "ingame-menu" | "object-menu" | "gameplay"
        | "evaluation" => &[],
        _ => return None,
    };
    Some(
        entries
            .iter()
            .map(|(path, asset)| ((*path).to_owned(), (*asset).to_owned()))
            .collect(),
    )
}

/// One capture's fixed geometry and ordered semantic layout.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayoutTrace {
    pub schema: String,
    pub screen: String,
    pub resolution: String,
    pub scale: u32,
    pub elements: Vec<LayoutElement>,
}

/// One ordered control or text node in a layout trace.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayoutElement {
    /// Stable hierarchy path used to identify the node across implementations.
    pub path: String,
    /// Semantic role, such as `button`, `label`, or `list-item`.
    pub role: String,
    pub rect: LayoutRect,
    pub visible: bool,
    /// The manifest-declared port-authored asset class whose text may differ.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_asset: Option<String>,
    pub caption: String,
    /// Lines after wrapping has been resolved by the implementation.
    pub lines: Vec<LayoutLine>,
}

/// One resolved line of text and the exact rectangle it occupies.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayoutLine {
    pub text: String,
    pub rect: LayoutRect,
}

/// Exact integer geometry of one traced element.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LayoutRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Which member of a capture pair failed validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceSide {
    Reference,
    Actual,
}

/// The first element field that differs in the declared comparison order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutField {
    Path,
    Role,
    Rect,
    Visible,
    PortAsset,
    Caption,
    LineCount,
    LineRect,
    LineText,
}

/// A typed field value carried by an element mismatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutValue {
    Text(String),
    OptionalText(Option<String>),
    Bool(bool),
    Rect(LayoutRect),
    Count(usize),
}

/// Why an ordered layout pair is not an exact match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutMismatch {
    UnknownScreen {
        screen: String,
    },
    ComparisonTerm {
        screen: String,
        expected: ComparisonTerm,
        actual: ComparisonTerm,
    },
    MalformedJson {
        side: TraceSide,
        detail: String,
    },
    Schema {
        side: TraceSide,
        expected: String,
        actual: String,
    },
    Screen {
        side: TraceSide,
        expected: String,
        actual: String,
    },
    Geometry {
        side: TraceSide,
        expected: String,
        actual: String,
    },
    Scale {
        side: TraceSide,
        expected: u32,
        actual: u32,
    },
    UnknownPortAsset {
        index: usize,
        path: String,
        asset: String,
    },
    PortAssetNotAllowed {
        screen: String,
        index: usize,
        path: String,
        asset: String,
    },
    MissingPortAssetExemption {
        side: TraceSide,
        path: String,
        asset: String,
    },
    EmptyPath {
        side: TraceSide,
        index: usize,
    },
    DuplicatePath {
        side: TraceSide,
        first: usize,
        duplicate: usize,
        path: String,
    },
    EmptyRole {
        side: TraceSide,
        index: usize,
        path: String,
    },
    EmptyTrace {
        side: TraceSide,
    },
    ElementCount {
        expected: usize,
        actual: usize,
    },
    Element {
        index: usize,
        path: String,
        line: Option<usize>,
        field: LayoutField,
        expected: LayoutValue,
        actual: LayoutValue,
    },
}

fn parse_layout_trace(side: TraceSide, json: &str) -> Result<LayoutTrace, LayoutMismatch> {
    serde_json::from_str(json).map_err(|error| LayoutMismatch::MalformedJson {
        side,
        detail: error.to_string(),
    })
}

/// Compares two serialized layout traces on the named screen's manifest term.
pub fn compare_layout_traces(
    screen: &str,
    reference_json: &str,
    actual_json: &str,
) -> Result<(), LayoutMismatch> {
    let terms = screens()
        .iter()
        .find(|entry| entry.id == screen)
        .ok_or_else(|| LayoutMismatch::UnknownScreen {
            screen: screen.to_owned(),
        })?;
    if terms.comparison != ComparisonTerm::Layout {
        return Err(LayoutMismatch::ComparisonTerm {
            screen: screen.to_owned(),
            expected: terms.comparison,
            actual: ComparisonTerm::Layout,
        });
    }
    let exemptions =
        expected_port_asset_exemptions(screen).ok_or_else(|| LayoutMismatch::UnknownScreen {
            screen: screen.to_owned(),
        })?;
    let reference = parse_layout_trace(TraceSide::Reference, reference_json)?;
    let actual = parse_layout_trace(TraceSide::Actual, actual_json)?;
    for (side, trace) in [
        (TraceSide::Reference, &reference),
        (TraceSide::Actual, &actual),
    ] {
        if trace.schema != LAYOUT_TRACE_SCHEMA {
            return Err(LayoutMismatch::Schema {
                side,
                expected: LAYOUT_TRACE_SCHEMA.to_owned(),
                actual: trace.schema.clone(),
            });
        }
        if trace.screen != screen {
            return Err(LayoutMismatch::Screen {
                side,
                expected: screen.to_owned(),
                actual: trace.screen.clone(),
            });
        }
        if trace.resolution != geometry().resolution {
            return Err(LayoutMismatch::Geometry {
                side,
                expected: geometry().resolution.clone(),
                actual: trace.resolution.clone(),
            });
        }
        if trace.scale != geometry().scale {
            return Err(LayoutMismatch::Scale {
                side,
                expected: geometry().scale,
                actual: trace.scale,
            });
        }
        if trace.elements.is_empty() {
            return Err(LayoutMismatch::EmptyTrace { side });
        }
        if let Some((index, _)) = trace
            .elements
            .iter()
            .enumerate()
            .find(|(_, element)| element.path.trim().is_empty())
        {
            return Err(LayoutMismatch::EmptyPath { side, index });
        }
        if let Some((index, element)) = trace
            .elements
            .iter()
            .enumerate()
            .find(|(_, element)| element.role.trim().is_empty())
        {
            return Err(LayoutMismatch::EmptyRole {
                side,
                index,
                path: element.path.clone(),
            });
        }
        for (duplicate, element) in trace.elements.iter().enumerate() {
            if let Some(first) = trace.elements[..duplicate]
                .iter()
                .position(|prior| prior.path == element.path)
            {
                return Err(LayoutMismatch::DuplicatePath {
                    side,
                    first,
                    duplicate,
                    path: element.path.clone(),
                });
            }
        }
        for (index, element) in trace.elements.iter().enumerate() {
            if let Some(asset) = element.port_asset.as_ref() {
                if port_assets().iter().all(|declared| declared.id != *asset) {
                    return Err(LayoutMismatch::UnknownPortAsset {
                        index,
                        path: element.path.clone(),
                        asset: asset.clone(),
                    });
                }
                if exemptions.get(&element.path) != Some(asset) {
                    return Err(LayoutMismatch::PortAssetNotAllowed {
                        screen: screen.to_owned(),
                        index,
                        path: element.path.clone(),
                        asset: asset.clone(),
                    });
                }
            }
        }
    }
    if reference.elements.len() != actual.elements.len() {
        return Err(LayoutMismatch::ElementCount {
            expected: reference.elements.len(),
            actual: actual.elements.len(),
        });
    }
    for (index, (expected, actual)) in reference.elements.iter().zip(&actual.elements).enumerate() {
        if expected.path != actual.path {
            return Err(LayoutMismatch::Element {
                index,
                path: expected.path.clone(),
                line: None,
                field: LayoutField::Path,
                expected: LayoutValue::Text(expected.path.clone()),
                actual: LayoutValue::Text(actual.path.clone()),
            });
        }
        if expected.role != actual.role {
            return Err(LayoutMismatch::Element {
                index,
                path: expected.path.clone(),
                line: None,
                field: LayoutField::Role,
                expected: LayoutValue::Text(expected.role.clone()),
                actual: LayoutValue::Text(actual.role.clone()),
            });
        }
        if expected.rect != actual.rect {
            return Err(LayoutMismatch::Element {
                index,
                path: expected.path.clone(),
                line: None,
                field: LayoutField::Rect,
                expected: LayoutValue::Rect(expected.rect),
                actual: LayoutValue::Rect(actual.rect),
            });
        }
        if expected.visible != actual.visible {
            return Err(LayoutMismatch::Element {
                index,
                path: expected.path.clone(),
                line: None,
                field: LayoutField::Visible,
                expected: LayoutValue::Bool(expected.visible),
                actual: LayoutValue::Bool(actual.visible),
            });
        }
        if expected.port_asset != actual.port_asset {
            return Err(LayoutMismatch::Element {
                index,
                path: expected.path.clone(),
                line: None,
                field: LayoutField::PortAsset,
                expected: LayoutValue::OptionalText(expected.port_asset.clone()),
                actual: LayoutValue::OptionalText(actual.port_asset.clone()),
            });
        }
        if expected.port_asset.is_none() && expected.caption != actual.caption {
            return Err(LayoutMismatch::Element {
                index,
                path: expected.path.clone(),
                line: None,
                field: LayoutField::Caption,
                expected: LayoutValue::Text(expected.caption.clone()),
                actual: LayoutValue::Text(actual.caption.clone()),
            });
        }
        if expected.lines.len() != actual.lines.len() {
            return Err(LayoutMismatch::Element {
                index,
                path: expected.path.clone(),
                line: None,
                field: LayoutField::LineCount,
                expected: LayoutValue::Count(expected.lines.len()),
                actual: LayoutValue::Count(actual.lines.len()),
            });
        }
        for (line, (expected_line, actual_line)) in
            expected.lines.iter().zip(&actual.lines).enumerate()
        {
            if expected_line.rect != actual_line.rect {
                return Err(LayoutMismatch::Element {
                    index,
                    path: expected.path.clone(),
                    line: Some(line),
                    field: LayoutField::LineRect,
                    expected: LayoutValue::Rect(expected_line.rect),
                    actual: LayoutValue::Rect(actual_line.rect),
                });
            }
            if expected.port_asset.is_none() && expected_line.text != actual_line.text {
                return Err(LayoutMismatch::Element {
                    index,
                    path: expected.path.clone(),
                    line: Some(line),
                    field: LayoutField::LineText,
                    expected: LayoutValue::Text(expected_line.text.clone()),
                    actual: LayoutValue::Text(actual_line.text.clone()),
                });
            }
        }
    }
    for (side, trace) in [
        (TraceSide::Reference, &reference),
        (TraceSide::Actual, &actual),
    ] {
        if let Some((path, asset)) = exemptions.iter().find(|(path, asset)| {
            trace.elements.iter().all(|element| {
                element.path != path.as_str() || element.port_asset.as_ref() != Some(*asset)
            })
        }) {
            return Err(LayoutMismatch::MissingPortAssetExemption {
                side,
                path: path.clone(),
                asset: asset.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    #[test]
    fn an_absent_port_asset_is_omitted_from_layout_json() {
        let element = LayoutElement {
            path: "startup/main/buttons/start-game".to_owned(),
            role: "button".to_owned(),
            rect: LayoutRect {
                x: 40,
                y: 80,
                width: 240,
                height: 32,
            },
            visible: true,
            port_asset: None,
            caption: "Local Game".to_owned(),
            lines: Vec::new(),
        };

        let json = serde_json::to_value(element).expect("serialize layout element");

        assert!(!json
            .as_object()
            .expect("layout element is an object")
            .contains_key("port_asset"));
    }

    fn element(path: &str) -> String {
        format!(
            r#"{{"path":"{path}","role":"button","rect":{{"x":40,"y":80,"width":240,"height":32}},"visible":true,"caption":"Local Game","lines":[{{"text":"Local Game","rect":{{"x":48,"y":86,"width":96,"height":16}}}}]}}"#
        )
    }

    fn trace(elements: &str) -> String {
        let mut elements = vec![elements.to_owned()];
        for path in [
            "startup/main/branding/logo",
            "startup/main/branding/version",
            "startup/main/branding/fan-project",
        ] {
            if !elements[0].contains(&format!(r#""path":"{path}""#)) {
                elements.push(element(path).replacen(
                    r#""caption":"#,
                    r#""port_asset":"branding","caption":"#,
                    1,
                ));
            }
        }
        identified_trace(
            "startup-main",
            &elements
                .into_iter()
                .filter(|element| !element.is_empty())
                .collect::<Vec<_>>()
                .join(","),
        )
    }

    fn identified_trace(screen: &str, elements: &str) -> String {
        format!(
            r#"{{"schema":"clonk-rs/presentation-layout/v1","screen":"{screen}","resolution":"1280x720","scale":100,"elements":[{elements}]}}"#
        )
    }

    #[test]
    fn a_trace_with_the_current_schema_and_requested_screen_is_accepted() {
        let trace = trace(&element("startup/main/local-game"));

        assert_eq!(
            compare_layout_traces("startup-main", &trace, &trace),
            Ok(())
        );
    }

    #[test]
    fn a_trace_with_a_different_schema_is_rejected() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen(LAYOUT_TRACE_SCHEMA, "other/schema/v9", 1);

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Schema {
                side: TraceSide::Actual,
                expected: LAYOUT_TRACE_SCHEMA.to_owned(),
                actual: "other/schema/v9".to_owned(),
            })
        );
    }

    #[test]
    fn a_trace_for_a_different_screen_is_rejected() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = identified_trace("startup-about", &element("startup/main/local-game"));

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Screen {
                side: TraceSide::Actual,
                expected: "startup-main".to_owned(),
                actual: "startup-about".to_owned(),
            })
        );
    }

    #[test]
    fn an_empty_element_path_is_rejected() {
        let trace = trace(&element(""));

        assert_eq!(
            compare_layout_traces("startup-main", &trace, &trace),
            Err(LayoutMismatch::EmptyPath {
                side: TraceSide::Reference,
                index: 0,
            })
        );
    }

    #[test]
    fn duplicate_element_paths_are_rejected() {
        let duplicate = element("startup/main/local-game");
        let trace = trace(&format!("{duplicate},{duplicate}"));

        assert_eq!(
            compare_layout_traces("startup-main", &trace, &trace),
            Err(LayoutMismatch::DuplicatePath {
                side: TraceSide::Reference,
                first: 0,
                duplicate: 1,
                path: "startup/main/local-game".to_owned(),
            })
        );
    }

    #[test]
    fn an_empty_semantic_role_is_rejected() {
        let trace = trace(&element("startup/main/local-game").replacen(
            r#""role":"button""#,
            r#""role":"""#,
            1,
        ));

        assert_eq!(
            compare_layout_traces("startup-main", &trace, &trace),
            Err(LayoutMismatch::EmptyRole {
                side: TraceSide::Reference,
                index: 0,
                path: "startup/main/local-game".to_owned(),
            })
        );
    }

    #[test]
    fn an_empty_layout_trace_is_rejected() {
        let trace = identified_trace("startup-main", "");

        assert_eq!(
            compare_layout_traces("startup-main", &trace, &trace),
            Err(LayoutMismatch::EmptyTrace {
                side: TraceSide::Reference,
            })
        );
    }

    #[test]
    fn identical_layout_traces_match() {
        let trace = trace(&element("startup/main/local-game"));

        assert_eq!(
            compare_layout_traces("startup-main", &trace, &trace),
            Ok(())
        );
    }

    #[test]
    fn an_unknown_screen_has_no_layout_terms() {
        let trace = trace(&element("startup/main/local-game"));

        assert_eq!(
            compare_layout_traces("not-a-screen", &trace, &trace),
            Err(LayoutMismatch::UnknownScreen {
                screen: "not-a-screen".to_owned(),
            })
        );
    }

    #[test]
    fn a_pixel_screen_cannot_be_verified_by_the_layout_comparator() {
        let trace = trace(&element("gameplay/world"));

        assert_eq!(
            compare_layout_traces("gameplay", &trace, &trace),
            Err(LayoutMismatch::ComparisonTerm {
                screen: "gameplay".to_owned(),
                expected: ComparisonTerm::Pixel,
                actual: ComparisonTerm::Layout,
            })
        );
    }

    #[test]
    fn malformed_json_is_rejected_on_the_side_that_cannot_be_decoded() {
        let actual = trace(&element("startup/main/local-game"));

        assert!(matches!(
            compare_layout_traces("startup-main", "{", &actual),
            Err(LayoutMismatch::MalformedJson {
                side: TraceSide::Reference,
                ..
            })
        ));
    }

    #[test]
    fn unknown_fields_are_rejected_at_every_trace_level() {
        let valid = trace(&element("startup/main/local-game"));
        let candidates = [
            valid.replacen(r#""elements":"#, r#""unexpected":true,"elements":"#, 1),
            valid.replacen(r#""role":"#, r#""unexpected":true,"role":"#, 1),
            valid.replacen(r#""x":40"#, r#""unexpected":true,"x":40"#, 1),
            valid.replacen(r#""text":"#, r#""unexpected":true,"text":"#, 1),
        ];

        for actual in candidates {
            assert!(matches!(
                compare_layout_traces("startup-main", &valid, &actual),
                Err(LayoutMismatch::MalformedJson {
                    side: TraceSide::Actual,
                    ..
                })
            ));
        }
    }

    #[test]
    fn a_trace_at_a_different_resolution_is_rejected() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen("1280x720", "640x480", 1);

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Geometry {
                side: TraceSide::Actual,
                expected: "1280x720".to_owned(),
                actual: "640x480".to_owned(),
            })
        );
    }

    #[test]
    fn a_trace_at_a_different_scale_is_rejected() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen(r#""scale":100"#, r#""scale":125"#, 1);

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Scale {
                side: TraceSide::Actual,
                expected: 100,
                actual: 125,
            })
        );
    }

    #[test]
    fn differing_element_counts_are_rejected_before_field_comparison() {
        let first = element("startup/main/local-game");
        let reference = trace(&first);
        let actual = trace(&format!("{first},{}", element("startup/main/network-game")));

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::ElementCount {
                expected: 4,
                actual: 5,
            })
        );
    }

    #[test]
    fn element_order_is_part_of_the_layout_contract() {
        let local = element("startup/main/local-game");
        let network = element("startup/main/network-game");
        let reference = trace(&format!("{local},{network}"));
        let actual = trace(&format!("{network},{local}"));

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                path: "startup/main/local-game".to_owned(),
                line: None,
                field: LayoutField::Path,
                expected: LayoutValue::Text("startup/main/local-game".to_owned()),
                actual: LayoutValue::Text("startup/main/network-game".to_owned()),
            })
        );
    }

    #[test]
    fn the_first_differing_element_rect_is_reported_exactly() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen(r#""width":240"#, r#""width":241"#, 1);

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                path: "startup/main/local-game".to_owned(),
                line: None,
                field: LayoutField::Rect,
                expected: LayoutValue::Rect(LayoutRect {
                    x: 40,
                    y: 80,
                    width: 240,
                    height: 32,
                }),
                actual: LayoutValue::Rect(LayoutRect {
                    x: 40,
                    y: 80,
                    width: 241,
                    height: 32,
                }),
            })
        );
    }

    #[test]
    fn semantic_roles_are_exact() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen(r#""role":"button""#, r#""role":"label""#, 1);

        assert!(matches!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                line: None,
                field: LayoutField::Role,
                ..
            })
        ));
    }

    #[test]
    fn element_visibility_is_exact() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen(r#""visible":true"#, r#""visible":false"#, 1);

        assert!(matches!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                line: None,
                field: LayoutField::Visible,
                ..
            })
        ));
    }

    #[test]
    fn an_untagged_caption_difference_is_rejected() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen("Local Game", "Network Game", 1);

        assert!(matches!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                line: None,
                field: LayoutField::Caption,
                ..
            })
        ));
    }

    #[test]
    fn an_untagged_resolved_line_difference_is_rejected() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen(r#""text":"Local Game""#, r#""text":"Local game""#, 1);

        assert!(matches!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                line: Some(0),
                field: LayoutField::LineText,
                ..
            })
        ));
    }

    #[test]
    fn differing_resolved_line_counts_are_rejected() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen(
            r#""lines":[{"text":"Local Game","rect":{"x":48,"y":86,"width":96,"height":16}}]"#,
            r#""lines":[]"#,
            1,
        );

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                path: "startup/main/local-game".to_owned(),
                line: None,
                field: LayoutField::LineCount,
                expected: LayoutValue::Count(1),
                actual: LayoutValue::Count(0),
            })
        );
    }

    #[test]
    fn resolved_line_rects_are_exact() {
        let reference = trace(&element("startup/main/local-game"));
        let actual = reference.replacen(r#""width":96"#, r#""width":97"#, 1);

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                path: "startup/main/local-game".to_owned(),
                line: Some(0),
                field: LayoutField::LineRect,
                expected: LayoutValue::Rect(LayoutRect {
                    x: 48,
                    y: 86,
                    width: 96,
                    height: 16,
                }),
                actual: LayoutValue::Rect(LayoutRect {
                    x: 48,
                    y: 86,
                    width: 97,
                    height: 16,
                }),
            })
        );
    }

    #[test]
    fn a_port_asset_tag_must_be_symmetric() {
        let untagged = element("startup/main/branding/version");
        let tagged = untagged.replacen(r#""caption":"#, r#""port_asset":"branding","caption":"#, 1);
        let reference = trace(&tagged);
        let actual = trace(&untagged);

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Err(LayoutMismatch::Element {
                index: 0,
                path: "startup/main/branding/version".to_owned(),
                line: None,
                field: LayoutField::PortAsset,
                expected: LayoutValue::OptionalText(Some("branding".to_owned())),
                actual: LayoutValue::OptionalText(None),
            })
        );
    }

    #[test]
    fn an_undeclared_port_asset_tag_is_rejected() {
        let tagged = element("startup/main/branding").replacen(
            r#""caption":"#,
            r#""port_asset":"not-declared","caption":"#,
            1,
        );
        let trace = trace(&tagged);

        assert_eq!(
            compare_layout_traces("startup-main", &trace, &trace),
            Err(LayoutMismatch::UnknownPortAsset {
                index: 0,
                path: "startup/main/branding".to_owned(),
                asset: "not-declared".to_owned(),
            })
        );
    }

    #[test]
    fn a_port_asset_tag_must_be_allowed_for_the_requested_screen() {
        let tagged = element("startup/main/background").replacen(
            r#""caption":"#,
            r#""port_asset":"super-resolved-startup-art","caption":"#,
            1,
        );
        let trace = trace(&tagged);

        assert_eq!(
            compare_layout_traces("startup-main", &trace, &trace),
            Err(LayoutMismatch::PortAssetNotAllowed {
                screen: "startup-main".to_owned(),
                index: 0,
                path: "startup/main/background".to_owned(),
                asset: "super-resolved-startup-art".to_owned(),
            })
        );
    }

    #[test]
    fn a_screen_allowed_port_asset_class_is_rejected_at_an_undeclared_path() {
        let tagged = element("startup/main/buttons/start-game").replacen(
            r#""caption":"#,
            r#""port_asset":"branding","caption":"#,
            1,
        );
        let trace = trace(&tagged);

        assert!(matches!(
            compare_layout_traces("startup-main", &trace, &trace),
            Err(LayoutMismatch::PortAssetNotAllowed {
                screen,
                index: 0,
                path,
                asset,
            }) if screen == "startup-main"
                && path == "startup/main/buttons/start-game"
                && asset == "branding"
        ));
    }

    #[test]
    fn an_allowed_port_asset_tag_may_have_a_different_caption() {
        let tagged = element("startup/main/branding/version").replacen(
            r#""caption":"#,
            r#""port_asset":"branding","caption":"#,
            1,
        );
        let reference = trace(&tagged);
        let actual = trace(&tagged.replacen("Local Game", "clonk-rs", 1));

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Ok(())
        );
    }

    #[test]
    fn an_allowed_port_asset_tag_may_have_different_resolved_line_text() {
        let tagged = element("startup/main/branding/version").replacen(
            r#""caption":"#,
            r#""port_asset":"branding","caption":"#,
            1,
        );
        let reference = trace(&tagged);
        let actual = trace(&tagged.replacen(r#""text":"Local Game""#, r#""text":"clonk-rs""#, 1));

        assert_eq!(
            compare_layout_traces("startup-main", &reference, &actual),
            Ok(())
        );
    }

    #[test]
    fn an_allowed_port_asset_tag_does_not_relax_structure_or_geometry() {
        let tagged = element("startup/main/branding/version").replacen(
            r#""caption":"#,
            r#""port_asset":"branding","caption":"#,
            1,
        );
        let reference = trace(&tagged);
        let candidates = [
            (
                tagged.replacen(r#""role":"button""#, r#""role":"label""#, 1),
                LayoutField::Role,
                None,
            ),
            (
                tagged.replacen(r#""width":240"#, r#""width":241"#, 1),
                LayoutField::Rect,
                None,
            ),
            (
                tagged.replacen(r#""visible":true"#, r#""visible":false"#, 1),
                LayoutField::Visible,
                None,
            ),
            (
                tagged.replacen(
                    r#""lines":[{"text":"Local Game","rect":{"x":48,"y":86,"width":96,"height":16}}]"#,
                    r#""lines":[]"#,
                    1,
                ),
                LayoutField::LineCount,
                None,
            ),
            (
                tagged.replacen(r#""width":96"#, r#""width":97"#, 1),
                LayoutField::LineRect,
                Some(0),
            ),
        ];

        for (actual_element, expected_field, expected_line) in candidates {
            let actual = trace(&actual_element);
            let Err(LayoutMismatch::Element { field, line, .. }) =
                compare_layout_traces("startup-main", &reference, &actual)
            else {
                panic!("tagged structural difference was not rejected");
            };
            assert_eq!((field, line), (expected_field, expected_line));
        }
    }
}
