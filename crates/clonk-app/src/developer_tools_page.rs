//! The Tools page's control inventory, grouping and enablement.
//!
//! `C4ToolsDlg` builds one page for the developer toolbox
//! (`C4ToolsDlg.cpp:289-377`). Its structure is a fixed box tree, and the
//! *order and grouping* are as much part of the port as the behaviour:
//!
//! ```text
//! hbox(12)
//! ├── vbox(6)                       landscape mode
//! │   └── Dynamic, Static, Exact
//! └── vbox(12, expands)
//!     ├── hbox(6)                   tools
//!     │   └── Brush, Line, Rect, Fill, Picker
//!     └── hbox(12, expands)
//!         ├── Preview
//!         ├── Grade                 vertical scale
//!         ├── vbox(6)               IFT
//!         │   └── Ift, NoIft
//!         └── vbox(6, expands)      material / texture
//!             └── Materials, Textures
//! ```
//!
//! Enablement comes from `C4ToolsDlg::EnableControls` (`:897-940`) and is not
//! one rule. Nearly everything needs `Mode >= C4LSC_Static`, but **Fill needs
//! `>= C4LSC_Exact`** — it is the only tool that does not exist in a static
//! landscape — and the **texture list additionally requires the material not to
//! be Sky**, because Sky has no textures to choose from. The three landscape
//! mode buttons are never disabled: they are how a user gets *out* of a mode
//! where everything else is greyed out.
//!
//! Win32 swaps in a second bitmap for a disabled button using the very same
//! predicates, so the enablement answer also selects the artwork.

use clonk_engine::developer_landscape::TOOL_SKY_MATERIAL;
use clonk_engine::developer_tools::{LandscapeMode, Tool};

/// Every control on the page, in the order the box tree builds them.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ToolsControl {
    ModeDynamic,
    ModeStatic,
    ModeExact,
    Brush,
    Line,
    Rect,
    Fill,
    Picker,
    Preview,
    Grade,
    Ift,
    NoIft,
    Materials,
    Textures,
}

/// The page's control order (`C4ToolsDlg.cpp:305-372`).
pub(crate) const TOOLS_PAGE_CONTROLS: [ToolsControl; 14] = [
    ToolsControl::ModeDynamic,
    ToolsControl::ModeStatic,
    ToolsControl::ModeExact,
    ToolsControl::Brush,
    ToolsControl::Line,
    ToolsControl::Rect,
    ToolsControl::Fill,
    ToolsControl::Picker,
    ToolsControl::Preview,
    ToolsControl::Grade,
    ToolsControl::Ift,
    ToolsControl::NoIft,
    ToolsControl::Materials,
    ToolsControl::Textures,
];

impl ToolsControl {
    /// The tool a tool button selects, if it is one.
    pub(crate) fn tool(self) -> Option<Tool> {
        match self {
            Self::Brush => Some(Tool::Brush),
            Self::Line => Some(Tool::Line),
            Self::Rect => Some(Tool::Rect),
            Self::Fill => Some(Tool::Fill),
            Self::Picker => Some(Tool::Picker),
            _ => None,
        }
    }

    /// The landscape mode a mode button selects, if it is one. These are the
    /// only controls that stay live in every mode.
    pub(crate) fn landscape_mode(self) -> Option<LandscapeMode> {
        match self {
            Self::ModeDynamic => Some(LandscapeMode::Dynamic),
            Self::ModeStatic => Some(LandscapeMode::Static),
            Self::ModeExact => Some(LandscapeMode::Exact),
            _ => None,
        }
    }
}

/// `C4ToolsDlg::EnableControls` for one control (`C4ToolsDlg.cpp:912-940`).
///
/// `material` is the selected material name; Sky is the one that also disables
/// the texture list.
pub(crate) fn tools_control_enabled(
    control: ToolsControl,
    mode: LandscapeMode,
    material: &str,
) -> bool {
    // C4LSC_Dynamic = 1, Static = 2, Exact = 3, so ">= Static" excludes
    // Dynamic and Undefined.
    let at_least_static = matches!(mode, LandscapeMode::Static | LandscapeMode::Exact);
    let exact = matches!(mode, LandscapeMode::Exact);
    match control {
        // The mode buttons are never disabled — otherwise a dynamic landscape
        // would be a dead end.
        ToolsControl::ModeDynamic | ToolsControl::ModeStatic | ToolsControl::ModeExact => true,
        // Fill is the exception: it exists only in an exact landscape.
        ToolsControl::Fill => exact,
        // Sky has no textures to choose between.
        ToolsControl::Textures => at_least_static && material != TOOL_SKY_MATERIAL,
        _ => at_least_static,
    }
}

/// Which controls are enabled, in page order.
pub(crate) fn tools_page_enablement(
    mode: LandscapeMode,
    material: &str,
) -> Vec<(ToolsControl, bool)> {
    TOOLS_PAGE_CONTROLS
        .into_iter()
        .map(|control| (control, tools_control_enabled(control, mode, material)))
        .collect()
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    fn enabled(mode: LandscapeMode, material: &str) -> Vec<ToolsControl> {
        tools_page_enablement(mode, material)
            .into_iter()
            .filter(|(_, enabled)| *enabled)
            .map(|(control, _)| control)
            .collect()
    }

    // C4ToolsDlg.cpp:289-377 (the box tree) and :912-940 (EnableControls).
    #[test]
    fn tools_page_enables_fill_only_in_exact_and_textures_only_off_sky() {
        // The page's control order is the order the box tree builds it.
        assert_eq!(
            TOOLS_PAGE_CONTROLS.first(),
            Some(&ToolsControl::ModeDynamic)
        );
        assert_eq!(TOOLS_PAGE_CONTROLS.last(), Some(&ToolsControl::Textures));
        assert_eq!(ToolsControl::Fill.tool(), Some(Tool::Fill));
        assert_eq!(ToolsControl::Preview.tool(), None);
        assert_eq!(
            ToolsControl::ModeExact.landscape_mode(),
            Some(LandscapeMode::Exact)
        );
        assert_eq!(ToolsControl::Brush.landscape_mode(), None);

        // A dynamic landscape leaves *only* the three mode buttons live, so the
        // user can still switch out of it.
        assert_eq!(
            enabled(LandscapeMode::Dynamic, "Earth"),
            vec![
                ToolsControl::ModeDynamic,
                ToolsControl::ModeStatic,
                ToolsControl::ModeExact
            ]
        );
        assert_eq!(
            enabled(LandscapeMode::Undefined, "Earth"),
            enabled(LandscapeMode::Dynamic, "Earth"),
            "Undefined is below Static too"
        );

        // Static enables everything except Fill.
        let static_mode = enabled(LandscapeMode::Static, "Earth");
        assert!(!static_mode.contains(&ToolsControl::Fill));
        for control in [
            ToolsControl::Brush,
            ToolsControl::Line,
            ToolsControl::Rect,
            ToolsControl::Picker,
            ToolsControl::Preview,
            ToolsControl::Grade,
            ToolsControl::Ift,
            ToolsControl::NoIft,
            ToolsControl::Materials,
            ToolsControl::Textures,
        ] {
            assert!(
                static_mode.contains(&control),
                "{control:?} is live in a static landscape"
            );
        }

        // Exact adds Fill and nothing else.
        let exact_mode = enabled(LandscapeMode::Exact, "Earth");
        assert!(exact_mode.contains(&ToolsControl::Fill));
        assert_eq!(
            exact_mode.len(),
            static_mode.len() + 1,
            "Fill is the only control Exact adds"
        );

        // Sky disables the texture list on its own, without touching the
        // material list beside it.
        let sky = enabled(LandscapeMode::Exact, TOOL_SKY_MATERIAL);
        assert!(!sky.contains(&ToolsControl::Textures));
        assert!(sky.contains(&ToolsControl::Materials));
        assert!(
            tools_control_enabled(ToolsControl::Textures, LandscapeMode::Static, "Earth"),
            "a real material keeps its textures"
        );
        assert!(
            !tools_control_enabled(ToolsControl::Textures, LandscapeMode::Dynamic, "Earth"),
            "the mode gate still applies to a real material"
        );
    }
}
