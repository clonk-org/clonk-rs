//! What the console's edit cursor draws, and in what order.
//!
//! `C4EditCursor::Draw` (`C4EditCursor.cpp`) is **not** a native widget — it
//! draws through the engine's own rasterizer, so unlike the console's dialogs
//! it has an exact pixel oracle. It emits, in this order:
//!
//! 1. a selection mark per selected object, in selection order;
//! 2. the rubber-band drag frame, in white;
//! 3. the drag line, in white;
//! 4. the drop-target icon.
//!
//! Where this list is drawn is [`crate::developer_cursor`]'s neighbour in
//! `clonk-frontend::viewport_draw_order`: after the custom-GUI objects, before
//! the per-player HUD, and never in a fullscreen game.
//!
//! `DrawSelectMark` is the fiddly part. It is **twelve individual pixels**
//! forming an L at each corner — not a rectangle outline — and it draws nothing
//! at all when the shape is narrower or shorter than one pixel. Each corner is
//! the corner pixel plus one neighbour along each edge, so the marks read as
//! brackets rather than dots.

use crate::ObjectId;

/// `CWhite` / `0xFFFFFF`.
pub const OVERLAY_WHITE: u32 = 0x00FF_FFFF;

/// One draw the edit cursor issues.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsoleOverlayCommand {
    /// `DrawSelectMark` — the twelve corner pixels of this rectangle.
    SelectMark {
        object: ObjectId,
        pixels: Vec<(i32, i32)>,
    },
    /// The selected object redrawn additively while Shift is held: `ColorMod`
    /// forced to `0xffffff` and `BlitMode` to
    /// `C4GFXBLIT_CLRSFC_MOD2 | C4GFXBLIT_ADDITIVE`, then both restored. Both
    /// `Draw` and `DrawTopFace` run, with player `-1`.
    HighlightSelected { object: ObjectId },
    /// `DrawFrame(min, min, max, max, CWhite)` — the corners are normalised, so
    /// dragging up-left frames the same rectangle as dragging down-right.
    DragFrame {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        color: u32,
    },
    /// `DrawLine(X, Y, X2, Y2, CWhite)` — *not* normalised: the line keeps its
    /// direction.
    DragLine {
        from: (i32, i32),
        to: (i32, i32),
        color: u32,
    },
    /// `fctDropTarget`, centred horizontally on the target and sitting on top
    /// of its shape.
    DropTarget { object: ObjectId, x: i32, y: i32 },
}

/// One selected object, already projected into viewport space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlaySelection {
    pub object: ObjectId,
    /// `cobj->x + cobj->Shape.x + cgo.X - cotx`, where `cotx` is the target
    /// position after `TargetPos` has applied the object's parallax.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The twelve pixels `DrawSelectMark` writes (`C4EditCursor.cpp`).
///
/// Empty when the rectangle is narrower or shorter than one pixel — C++ returns
/// early on `Wdt < 1 || Hgt < 1`.
pub fn select_mark_pixels(x: i32, y: i32, width: i32, height: i32) -> Vec<(i32, i32)> {
    if width < 1 || height < 1 {
        return Vec::new();
    }
    let right = x + width - 1;
    let bottom = y + height - 1;
    vec![
        // Top left.
        (x, y),
        (x + 1, y),
        (x, y + 1),
        // Bottom left.
        (x, bottom),
        (x + 1, bottom),
        (x, bottom - 1),
        // Top right.
        (right, y),
        (right - 1, y),
        (right, y + 1),
        // Bottom right.
        (right, bottom),
        (right - 1, bottom),
        (right, bottom - 1),
    ]
}

/// The drop-target icon's position: centred on the target horizontally and
/// resting on top of its shape (`C4EditCursor.cpp`).
pub fn drop_target_position(
    target_x: i32,
    target_y: i32,
    shape_y: i32,
    icon_width: i32,
    icon_height: i32,
) -> (i32, i32) {
    (target_x - icon_width / 2, target_y + shape_y - icon_height)
}

/// The overlay's draw list. `shift` mirrors `Application.IsShiftDown()`.
#[allow(clippy::too_many_arguments)]
pub fn console_overlay_commands(
    fullscreen: bool,
    shift: bool,
    selection: &[OverlaySelection],
    drag_frame: Option<((i32, i32), (i32, i32))>,
    drag_line: Option<((i32, i32), (i32, i32))>,
    drop_target: Option<(ObjectId, i32, i32)>,
) -> Vec<ConsoleOverlayCommand> {
    // `C4Viewport::Draw` only calls the cursor when windowed, so a fullscreen
    // game draws none of this however the console is set up.
    if fullscreen {
        return Vec::new();
    }
    let mut commands = Vec::new();
    for entry in selection {
        commands.push(ConsoleOverlayCommand::SelectMark {
            object: entry.object,
            pixels: select_mark_pixels(entry.x, entry.y, entry.width, entry.height),
        });
        if shift {
            commands.push(ConsoleOverlayCommand::HighlightSelected {
                object: entry.object,
            });
        }
    }
    if let Some(((x, y), (x2, y2))) = drag_frame {
        commands.push(ConsoleOverlayCommand::DragFrame {
            left: x.min(x2),
            top: y.min(y2),
            right: x.max(x2),
            bottom: y.max(y2),
            color: OVERLAY_WHITE,
        });
    }
    if let Some((from, to)) = drag_line {
        commands.push(ConsoleOverlayCommand::DragLine {
            from,
            to,
            color: OVERLAY_WHITE,
        });
    }
    if let Some((object, x, y)) = drop_target {
        commands.push(ConsoleOverlayCommand::DropTarget { object, x, y });
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected(id: u64, x: i32, y: i32) -> OverlaySelection {
        OverlaySelection {
            object: ObjectId(id),
            x,
            y,
            width: 10,
            height: 8,
        }
    }

    // C4EditCursor::Draw / DrawSelectMark — the emission order, the twelve
    // corner pixels, and the fullscreen gate.
    #[test]
    fn console_overlay_emits_marks_frame_line_and_drop_target_in_cpp_order() {
        // DrawSelectMark writes twelve pixels: each corner plus one neighbour
        // along each edge, so the marks read as brackets, not dots.
        let pixels = select_mark_pixels(100, 50, 10, 8);
        assert_eq!(pixels.len(), 12);
        assert_eq!(
            pixels,
            vec![
                (100, 50),
                (101, 50),
                (100, 51),
                (100, 57),
                (101, 57),
                (100, 56),
                (109, 50),
                (108, 50),
                (109, 51),
                (109, 57),
                (108, 57),
                (109, 56),
            ]
        );
        // A rectangle narrower or shorter than a pixel draws nothing at all.
        assert!(select_mark_pixels(0, 0, 0, 8).is_empty());
        assert!(select_mark_pixels(0, 0, 10, 0).is_empty());
        // A one-pixel shape still emits twelve writes; C++ does not deduplicate
        // them, they just land on the same pixels.
        assert_eq!(select_mark_pixels(0, 0, 1, 1).len(), 12);

        // The emission order is marks, frame, line, drop target.
        let commands = console_overlay_commands(
            false,
            false,
            &[selected(1, 0, 0), selected(2, 20, 20)],
            Some(((30, 40), (10, 20))),
            Some(((1, 2), (3, 4))),
            Some((ObjectId(9), 70, 80)),
        );
        assert!(matches!(
            commands.as_slice(),
            [
                ConsoleOverlayCommand::SelectMark { .. },
                ConsoleOverlayCommand::SelectMark { .. },
                ConsoleOverlayCommand::DragFrame { .. },
                ConsoleOverlayCommand::DragLine { .. },
                ConsoleOverlayCommand::DropTarget { .. },
            ]
        ));

        // The frame normalises its corners; the line does not.
        assert_eq!(
            commands[2],
            ConsoleOverlayCommand::DragFrame {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
                color: OVERLAY_WHITE,
            },
            "dragging up-left frames the same rectangle as down-right"
        );
        assert_eq!(
            commands[3],
            ConsoleOverlayCommand::DragLine {
                from: (1, 2),
                to: (3, 4),
                color: OVERLAY_WHITE,
            }
        );

        // Shift adds an additive highlight after each mark, interleaved rather
        // than appended as a second pass.
        let highlighted = console_overlay_commands(
            false,
            true,
            &[selected(1, 0, 0), selected(2, 20, 20)],
            None,
            None,
            None,
        );
        assert!(matches!(
            highlighted.as_slice(),
            [
                ConsoleOverlayCommand::SelectMark { object: a, .. },
                ConsoleOverlayCommand::HighlightSelected { object: b },
                ConsoleOverlayCommand::SelectMark { .. },
                ConsoleOverlayCommand::HighlightSelected { .. },
            ] if *a == ObjectId(1) && *b == ObjectId(1)
        ));

        // Nothing at all in a fullscreen game.
        assert!(console_overlay_commands(
            true,
            true,
            &[selected(1, 0, 0)],
            Some(((0, 0), (1, 1))),
            Some(((0, 0), (1, 1))),
            Some((ObjectId(9), 0, 0)),
        )
        .is_empty());

        // An idle windowed cursor emits nothing either.
        assert!(console_overlay_commands(false, false, &[], None, None, None).is_empty());

        // The drop-target icon is centred horizontally and sits on top of the
        // target's shape.
        assert_eq!(drop_target_position(100, 200, -6, 20, 14), (90, 180));
    }
}
