//! Developer edit-cursor mode and its context-menu enablement.
//!
//! `C4EditCursor::ToggleMode` (`C4EditCursor.cpp:540-556`) steps
//! Play -> Edit -> Draw -> Play, but only when `EditingOK()` passes — which is
//! just `Console.Editing` (`:683-692`). A refused toggle also clears `Hold` and
//! shows the `IDS_CNS_NONETEDIT` message.
//!
//! `:594-626` decides which context entries are enabled, and switches the
//! Properties item's caption by mode.
//!
//! Selection ordering lives in [`crate::developer_selection`]; the drawing
//! tools in [`crate::developer_tools`].

/// `C4CNS_Mode*` (`C4Console.h`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorMode {
    Play,
    Edit,
    Draw,
}

/// What a refused or accepted mode toggle produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModeToggle {
    /// The mode after the toggle — unchanged when it was refused.
    pub mode: CursorMode,
    /// Whether the toggle was allowed (`EditingOK`).
    pub accepted: bool,
    /// `EditingOK` clears `Hold` on refusal (`C4EditCursor.cpp:687`).
    pub clears_hold: bool,
}

/// `C4EditCursor::ToggleMode`. `editing` is `Console.Editing`; a network client
/// without edit rights fails it and gets `IDS_CNS_NONETEDIT`.
pub fn toggle_mode(mode: CursorMode, editing: bool) -> ModeToggle {
    if !editing {
        return ModeToggle {
            mode,
            accepted: false,
            clears_hold: true,
        };
    }
    let next = match mode {
        CursorMode::Play => CursorMode::Edit,
        CursorMode::Edit => CursorMode::Draw,
        CursorMode::Draw => CursorMode::Play,
    };
    ModeToggle {
        mode: next,
        accepted: true,
        clears_hold: false,
    }
}

/// Which viewport context entries are enabled (`C4EditCursor.cpp:598-601`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorContextMenu {
    pub delete: bool,
    pub duplicate: bool,
    pub contents: bool,
    pub properties: bool,
    /// `LoadResStrChoice(Mode == C4CNS_ModeEdit, IDS_CNS_PROPERTIES, IDS_CNS_TOOLS)`
    /// (`:605`) — outside Edit mode the entry reads "Tools", not "Properties".
    pub properties_caption: PropertiesCaption,
}

/// Which resource string the Properties entry shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertiesCaption {
    /// `IDS_CNS_PROPERTIES`, in Edit mode.
    Properties,
    /// `IDS_CNS_TOOLS`, in Play or Draw mode.
    Tools,
}

/// `C4EditCursor::DoContextMenu`'s enablement (`C4EditCursor.cpp:594-605`).
///
/// `first_selection_contents` is `Selection.GetObject()->Contents.ObjectCount()`
/// for the *first* selected object — C++ asks only that one.
pub fn context_menu(
    mode: CursorMode,
    editing: bool,
    object_selected: bool,
    first_selection_contents: usize,
) -> CursorContextMenu {
    let selected_and_editing = object_selected && editing;
    CursorContextMenu {
        delete: selected_and_editing,
        duplicate: selected_and_editing,
        contents: selected_and_editing && first_selection_contents > 0,
        // Properties is gated on mode alone — not on selection or editing.
        properties: mode != CursorMode::Play,
        properties_caption: if mode == CursorMode::Edit {
            PropertiesCaption::Properties
        } else {
            PropertiesCaption::Tools
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4EditCursor.cpp:540-556,594-605,683-692 — the mode cycle, its editing
    // gate, and the context entries each mode and selection enables.
    #[test]
    fn console_edit_cursor_selects_cycles_drags_and_emits_cpp_ordered_controls() {
        // Play -> Edit -> Draw -> Play while editing is allowed.
        let mut mode = CursorMode::Play;
        for expected in [CursorMode::Edit, CursorMode::Draw, CursorMode::Play] {
            let toggled = toggle_mode(mode, true);
            assert!(toggled.accepted);
            assert!(!toggled.clears_hold);
            assert_eq!(toggled.mode, expected);
            mode = toggled.mode;
        }

        // Without `Console.Editing` the mode is unchanged and Hold is cleared.
        let refused = toggle_mode(CursorMode::Edit, false);
        assert!(!refused.accepted);
        assert_eq!(refused.mode, CursorMode::Edit, "a refused toggle stays put");
        assert!(
            refused.clears_hold,
            "EditingOK clears Hold on refusal (:687)"
        );

        // Delete/Duplicate need a selection *and* editing rights.
        let editing_with_selection = context_menu(CursorMode::Edit, true, true, 0);
        assert!(editing_with_selection.delete);
        assert!(editing_with_selection.duplicate);
        // Contents additionally needs the first selected object to hold some.
        assert!(!editing_with_selection.contents);
        assert!(context_menu(CursorMode::Edit, true, true, 3).contents);

        // No selection, or no editing rights, disables all three.
        let no_selection = context_menu(CursorMode::Edit, true, false, 5);
        assert!(!no_selection.delete && !no_selection.duplicate && !no_selection.contents);
        let not_editing = context_menu(CursorMode::Edit, false, true, 5);
        assert!(!not_editing.delete && !not_editing.duplicate && !not_editing.contents);

        // Properties is gated on mode alone — it stays enabled with no
        // selection and without editing rights (:601).
        assert!(context_menu(CursorMode::Edit, false, false, 0).properties);
        assert!(context_menu(CursorMode::Draw, false, false, 0).properties);
        assert!(!context_menu(CursorMode::Play, true, true, 9).properties);

        // ...and its caption is "Tools" outside Edit mode (:605).
        assert_eq!(
            context_menu(CursorMode::Edit, true, true, 1).properties_caption,
            PropertiesCaption::Properties
        );
        assert_eq!(
            context_menu(CursorMode::Draw, true, true, 1).properties_caption,
            PropertiesCaption::Tools
        );
        assert_eq!(
            context_menu(CursorMode::Play, true, true, 1).properties_caption,
            PropertiesCaption::Tools
        );
    }
}
