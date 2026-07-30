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

use crate::ObjectId;

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

/// What changing the cursor mode publishes (`C4EditCursor::SetMode`,
/// `C4EditCursor.cpp`).
///
/// Everything here is intent: the console applies it to whatever dialogs exist,
/// and nothing requires them to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModeChange {
    /// `Console.UpdateModeCtrls(iMode)`. Runs **before** the unchanged-mode
    /// early return, so it fires even when the mode did not actually change.
    pub update_mode_controls: CursorMode,
    /// Whether anything below applies. False when the mode was already `iMode`.
    pub changed: bool,
    /// The toolbox page this mode drops: Edit and Play clear **Tools**, Draw
    /// clears **Property**.
    pub clear_page: Option<PropertyToolsPage>,
    /// `OpenPropTools()`, issued only when one of the two was already active.
    /// A mode switch never opens the toolbox from nothing.
    pub reopen_prop_tools: bool,
    /// `ShowCursor` in Play, `HideCursor` in Edit and Draw.
    pub show_mouse_cursor: bool,
    /// C++ saves the focused window before the switch and restores it after, so
    /// changing mode never steals focus from the console.
    pub restore_focus: bool,
}

/// Which page of the shared toolbox a mode change clears.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropertyToolsPage {
    Tools,
    Property,
}

/// `C4EditCursor::SetMode`. `tools_active`/`property_active` are
/// `Console.ToolsDlg.Active` / `Console.PropertyDlg.Active` as they were
/// *before* the switch.
pub fn set_mode(
    current: CursorMode,
    requested: CursorMode,
    tools_active: bool,
    property_active: bool,
) -> ModeChange {
    let reopen = tools_active || property_active;
    let changed = current != requested;
    ModeChange {
        // Unconditional, and before the early return.
        update_mode_controls: requested,
        changed,
        clear_page: changed.then_some(match requested {
            // Leaving Draw behind: the drawing tools go.
            CursorMode::Edit | CursorMode::Play => PropertyToolsPage::Tools,
            CursorMode::Draw => PropertyToolsPage::Property,
        }),
        reopen_prop_tools: changed && reopen,
        show_mouse_cursor: matches!(requested, CursorMode::Play),
        restore_focus: true,
    }
}

/// `C4EditCursor::Move`'s Edit-mode branch (`C4EditCursor.cpp:129-152`).
///
/// A held, non-frame drag moves the selection by the pointer delta and
/// re-computes the drop target; otherwise the hovered target is re-picked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditMove {
    /// `MoveSelection(xoff, yoff)` followed by `UpdateDropTarget`.
    MoveSelection { dx: i32, dy: i32 },
    /// `Target = ...` — the hovered object, or `None` over empty space.
    Retarget(Option<ObjectId>),
}

/// Picks the hovered object, with Shift continuing past the current selection
/// (`C4EditCursor.cpp:143-151`).
///
/// `find_next(after)` is `Game.FindObject(0, X, Y, 0, 0, OCF_NotContained, ...,
/// ANY_OWNER, after)`: the first hit at the cursor strictly after `after` in
/// master-list order, or `None` at the end. It is called at least once — C++
/// uses a `do`/`while` — and with Shift it keeps advancing while the hit is
/// already selected.
///
/// There is no wrap-around: once `find_next` runs out, the target is `None`.
pub fn edit_target(
    shift: bool,
    selection: &[ObjectId],
    find_next: impl Fn(Option<ObjectId>) -> Option<ObjectId>,
) -> Option<ObjectId> {
    // Shift resumes after the *last* selected object, so a repeated
    // shift-click walks the stack under the cursor.
    let mut target = shift.then(|| selection.last().copied()).flatten();
    loop {
        target = find_next(target);
        match target {
            Some(hit) if shift && selection.contains(&hit) => continue,
            _ => return target,
        }
    }
}

/// What a pointer move does in Edit mode. `drag_frame` is C++'s `DragFrame`.
pub fn edit_move(
    hold: bool,
    drag_frame: bool,
    dx: i32,
    dy: i32,
    retarget: impl FnOnce() -> Option<ObjectId>,
) -> EditMove {
    if hold && !drag_frame {
        return EditMove::MoveSelection { dx, dy };
    }
    EditMove::Retarget(retarget())
}

/// `C4EditCursor::Execute`'s Edit arm (`C4EditCursor.cpp:65-69`): while `Hold`
/// is set it re-issues `EMMO_Move` with a **zero** offset every tick, so a
/// stationary held selection still produces control traffic.
pub fn edit_tick_move(mode: CursorMode, hold: bool) -> Option<EditMove> {
    (matches!(mode, CursorMode::Edit) && hold).then_some(EditMove::MoveSelection { dx: 0, dy: 0 })
}

/// One object considered as a drop target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DropCandidate {
    pub id: ObjectId,
    /// `!cobj->Status` — deleted objects are skipped.
    pub deleted: bool,
    /// `cobj->Contained` — a contained object is never a drop target.
    pub contained: bool,
    /// `cobj->x + cobj->Shape.x`, `cobj->y + cobj->Shape.y`.
    pub shape_x: i32,
    pub shape_y: i32,
    /// `cobj->Shape.Wdt`, `cobj->Shape.Hgt`.
    pub shape_width: i32,
    pub shape_height: i32,
}

/// `C4EditCursor::UpdateDropTarget` (`C4EditCursor.cpp:653-670`).
///
/// Requires Ctrl **and** a non-empty selection, then takes the first object in
/// `Game.Objects` order whose shape rectangle contains the cursor, is neither
/// deleted nor contained, and is not itself selected. `candidates` must be in
/// First -> Next order — see [`crate::developer_inspection::master_list_order`].
pub fn drop_target(
    control: bool,
    selection: &[ObjectId],
    cursor: (i32, i32),
    candidates: &[DropCandidate],
) -> Option<ObjectId> {
    if !control || selection.is_empty() {
        return None;
    }
    let (x, y) = cursor;
    candidates
        .iter()
        .filter(|candidate| !candidate.deleted && !candidate.contained)
        // `Inside(X - (x + Shape.x), 0, Wdt - 1)` — an empty shape contains
        // nothing, because the upper bound goes negative.
        .filter(|candidate| {
            (0..candidate.shape_width).contains(&(x - candidate.shape_x))
                && (0..candidate.shape_height).contains(&(y - candidate.shape_y))
        })
        .find(|candidate| !selection.contains(&candidate.id))
        .map(|candidate| candidate.id)
}

/// What releasing the left button finishes (`C4EditCursor.cpp:672-702`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditRelease {
    /// `FrameSelection()` — the rubber-band selection is applied.
    FrameSelection,
    /// `PutContents()` — `EMMO_Enter` moves the selection into the drop target.
    Enter { target: ObjectId },
}

/// `C4EditCursor::LeftButtonUp`'s Edit arm. Both actions can fire, in this
/// order; `Hold`, `DragFrame`, `DragLine` and `DropTarget` are cleared
/// afterwards regardless.
pub fn edit_release(drag_frame: bool, drop_target: Option<ObjectId>) -> Vec<EditRelease> {
    drag_frame
        .then_some(EditRelease::FrameSelection)
        .into_iter()
        .chain(drop_target.map(|target| EditRelease::Enter { target }))
        .collect()
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

        // C4EditCursor::SetMode — the mode-change publication.
        // UpdateModeCtrls runs even when the mode does not change.
        let unchanged = set_mode(CursorMode::Edit, CursorMode::Edit, true, false);
        assert!(!unchanged.changed);
        assert_eq!(unchanged.update_mode_controls, CursorMode::Edit);
        assert_eq!(unchanged.clear_page, None);
        assert!(
            !unchanged.reopen_prop_tools,
            "an unchanged mode publishes only the control update"
        );

        // Entering Draw clears the Property page; entering Edit or Play clears
        // the Tools page.
        assert_eq!(
            set_mode(CursorMode::Edit, CursorMode::Draw, true, false).clear_page,
            Some(PropertyToolsPage::Property)
        );
        assert_eq!(
            set_mode(CursorMode::Draw, CursorMode::Edit, true, false).clear_page,
            Some(PropertyToolsPage::Tools)
        );
        assert_eq!(
            set_mode(CursorMode::Draw, CursorMode::Play, false, true).clear_page,
            Some(PropertyToolsPage::Tools)
        );

        // The toolbox reopens only when one of the two was already active — a
        // mode switch never opens it from nothing.
        assert!(set_mode(CursorMode::Edit, CursorMode::Draw, true, false).reopen_prop_tools);
        assert!(set_mode(CursorMode::Edit, CursorMode::Draw, false, true).reopen_prop_tools);
        assert!(!set_mode(CursorMode::Edit, CursorMode::Draw, false, false).reopen_prop_tools);

        // The mouse cursor is shown in Play and hidden in the editing modes.
        assert!(set_mode(CursorMode::Edit, CursorMode::Play, false, false).show_mouse_cursor);
        assert!(!set_mode(CursorMode::Play, CursorMode::Edit, false, false).show_mouse_cursor);
        assert!(!set_mode(CursorMode::Play, CursorMode::Draw, false, false).show_mouse_cursor);
        // Focus is always saved and restored, so switching never steals it.
        assert!(set_mode(CursorMode::Play, CursorMode::Draw, false, false).restore_focus);

        // C4EditCursor.cpp:143-151 — the stack under the cursor, walked by
        // Shift. `stack` stands in for Game.FindObject's resume behaviour.
        let stack = [ObjectId(7), ObjectId(4), ObjectId(9)];
        let find_next = |after: Option<ObjectId>| match after {
            None => stack.first().copied(),
            Some(previous) => stack
                .iter()
                .position(|id| *id == previous)
                .and_then(|index| stack.get(index + 1))
                .copied(),
        };

        // No Shift: always the topmost hit, however much is selected.
        assert_eq!(edit_target(false, &[], find_next), Some(ObjectId(7)));
        assert_eq!(
            edit_target(false, &[ObjectId(7)], find_next),
            Some(ObjectId(7)),
            "without Shift the selection does not shift the pick"
        );

        // Shift resumes after the LAST selected object and skips anything
        // already selected.
        assert_eq!(
            edit_target(true, &[ObjectId(7)], find_next),
            Some(ObjectId(4))
        );
        assert_eq!(
            edit_target(true, &[ObjectId(4), ObjectId(7)], find_next),
            Some(ObjectId(9)),
            "resume starts after Selection.Last, not after the first entry"
        );
        // Shift with nothing selected still calls FindObject once.
        assert_eq!(edit_target(true, &[], find_next), Some(ObjectId(7)));
        // Running out of stack yields nothing — C++ never wraps.
        assert_eq!(edit_target(true, &[ObjectId(9)], find_next), None);
        assert_eq!(
            edit_target(true, &[ObjectId(7), ObjectId(4), ObjectId(9)], find_next),
            None,
            "an all-selected stack ends at the list end rather than cycling"
        );

        // C4EditCursor.cpp:129-142 — a held non-frame drag moves; anything else
        // re-targets.
        assert_eq!(
            edit_move(true, false, 3, -5, || unreachable!(
                "a held drag never re-targets"
            )),
            EditMove::MoveSelection { dx: 3, dy: -5 }
        );
        assert_eq!(
            edit_move(true, true, 3, -5, || Some(ObjectId(4))),
            EditMove::Retarget(Some(ObjectId(4))),
            "a rubber-band drag keeps re-targeting"
        );
        assert_eq!(
            edit_move(false, false, 3, -5, || None),
            EditMove::Retarget(None)
        );

        // C4EditCursor.cpp:65-69 — Execute re-issues a ZERO-offset move every
        // tick while Hold is set, and only in Edit mode.
        assert_eq!(
            edit_tick_move(CursorMode::Edit, true),
            Some(EditMove::MoveSelection { dx: 0, dy: 0 })
        );
        assert_eq!(edit_tick_move(CursorMode::Edit, false), None);
        assert_eq!(edit_tick_move(CursorMode::Draw, true), None);
        assert_eq!(edit_tick_move(CursorMode::Play, true), None);

        // C4EditCursor.cpp:653-670 — Ctrl-hover drop targets.
        let candidate = |id: u64, x: i32| DropCandidate {
            id: ObjectId(id),
            deleted: false,
            contained: false,
            shape_x: x,
            shape_y: 0,
            shape_width: 10,
            shape_height: 10,
        };
        let candidates = [
            DropCandidate {
                deleted: true,
                ..candidate(1, 0)
            },
            DropCandidate {
                contained: true,
                ..candidate(2, 0)
            },
            candidate(3, 0),
            candidate(4, 0),
            candidate(5, 100),
        ];
        let selection = [ObjectId(3)];

        // Without Ctrl, or with nothing selected, there is no drop target.
        assert_eq!(drop_target(false, &selection, (5, 5), &candidates), None);
        assert_eq!(drop_target(true, &[], (5, 5), &candidates), None);

        // Deleted and contained objects are skipped, the selection is
        // excluded, and the first remaining hit in list order wins.
        assert_eq!(
            drop_target(true, &selection, (5, 5), &candidates),
            Some(ObjectId(4))
        );
        // Outside every shape.
        assert_eq!(drop_target(true, &selection, (50, 5), &candidates), None);
        // The rectangle is half-open: x + Wdt is already outside.
        assert_eq!(
            drop_target(true, &selection, (9, 9), &candidates),
            Some(ObjectId(4))
        );
        assert_eq!(drop_target(true, &selection, (10, 5), &candidates), None);
        // A zero-area shape contains nothing (`Inside(.., 0, Wdt - 1)`).
        let empty = [DropCandidate {
            shape_width: 0,
            shape_height: 0,
            ..candidate(6, 0)
        }];
        assert_eq!(drop_target(true, &selection, (0, 0), &empty), None);

        // C4EditCursor.cpp:672-682 — both release actions can fire, frame
        // selection first.
        assert_eq!(edit_release(false, None), Vec::new());
        assert_eq!(edit_release(true, None), vec![EditRelease::FrameSelection]);
        assert_eq!(
            edit_release(false, Some(ObjectId(4))),
            vec![EditRelease::Enter {
                target: ObjectId(4)
            }]
        );
        assert_eq!(
            edit_release(true, Some(ObjectId(4))),
            vec![
                EditRelease::FrameSelection,
                EditRelease::Enter {
                    target: ObjectId(4)
                }
            ]
        );
    }
}
