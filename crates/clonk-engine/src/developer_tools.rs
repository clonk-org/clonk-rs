//! The developer console's landscape drawing tool state.
//!
//! `C4ToolsDlg` owns the selected tool, grade and IFT flag; `C4EditCursor`
//! turns pointer gestures into `EMDT_*` draw controls at a per-tool cadence
//! (`C4EditCursor.cpp:74,159,234,301-304`).
//!
//! This is the state machine only — dialog chrome, the shared developer window
//! host and the control-queue round trip for `EMDT_SetMode` stay out
//! (M10-P4-L044's remaining criteria, and M10-P4-L081).

/// `C4TLS_*` (`C4ToolsDlg.h:33-37`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    Brush = 0,
    Line = 1,
    Rect = 2,
    Fill = 3,
    Picker = 4,
}

impl Tool {
    fn from_index(index: i32) -> Option<Self> {
        match index {
            0 => Some(Self::Brush),
            1 => Some(Self::Line),
            2 => Some(Self::Rect),
            3 => Some(Self::Fill),
            4 => Some(Self::Picker),
            _ => None,
        }
    }
}

/// `C4TLS_GradeMin`/`Max`/`Default` (`C4ToolsDlg.h:39-41`).
pub const GRADE_MIN: i32 = 1;
pub const GRADE_MAX: i32 = 50;
pub const GRADE_DEFAULT: i32 = 5;

/// The keyboard grade step. `C4ToolsDlg` binds +/- to a five-unit change.
pub const GRADE_KEY_STEP: i32 = 5;

/// One `C4ControlEMDrawTool` the cursor emits (`C4EditCursor.cpp:560-590`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrawControl {
    /// `EMDT_Brush` at the pointer; the second coordinate pair is unused.
    Brush { x: i32, y: i32 },
    /// `EMDT_Line` with both pairs, emitted once on release.
    Line { x: i32, y: i32, x2: i32, y2: i32 },
    /// `EMDT_Rect` with both pairs, emitted once on release.
    Rect { x: i32, y: i32, x2: i32, y2: i32 },
    /// `EMDT_Fill`. C++ passes `0` for `X2` and the drag's `Y2`, and forces
    /// IFT false (`:589`).
    Fill { x: i32, y: i32, y2: i32 },
}

/// The retained tool state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeveloperTools {
    tool: Tool,
    /// The tool to restore when a temporary Picker override ends.
    restore_tool: Option<Tool>,
    grade: i32,
    ift: bool,
    /// `C4EditCursor::Hold` — set while the button is down.
    hold: bool,
    /// Where the current gesture began.
    anchor: Option<(i32, i32)>,
}

impl Default for DeveloperTools {
    fn default() -> Self {
        Self {
            tool: Tool::Brush,
            restore_tool: None,
            grade: GRADE_DEFAULT,
            ift: true,
            hold: false,
            anchor: None,
        }
    }
}

impl DeveloperTools {
    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn grade(&self) -> i32 {
        self.grade
    }

    pub fn ift(&self) -> bool {
        self.ift
    }

    pub fn holding(&self) -> bool {
        self.hold
    }

    /// `C4ToolsDlg::SetTool`. A temporary selection remembers what to restore.
    pub fn set_tool(&mut self, tool: Tool, temporary: bool) {
        if temporary {
            self.restore_tool.get_or_insert(self.tool);
        } else {
            self.restore_tool = None;
        }
        self.tool = tool;
    }

    /// `ToggleTool` — `(Tool + 1) % 4`, which never lands on Picker
    /// (`C4ToolsDlg.h:148`).
    pub fn toggle_tool(&mut self) {
        let next = (self.tool as i32 + 1) % 4;
        self.tool = Tool::from_index(next).unwrap_or(Tool::Brush);
        self.restore_tool = None;
    }

    /// `C4ToolsDlg::SetGrade` — `BoundBy(iGrade, Min, Max)` (`:732-737`).
    pub fn set_grade(&mut self, grade: i32) {
        self.grade = grade.clamp(GRADE_MIN, GRADE_MAX);
    }

    /// `ChangeGrade` — the keyboard's five-unit step, clamped the same way.
    pub fn change_grade(&mut self, steps: i32) {
        self.set_grade(self.grade + steps * GRADE_KEY_STEP);
    }

    pub fn set_ift(&mut self, ift: bool) {
        self.ift = ift;
    }

    pub fn toggle_ift(&mut self) {
        self.ift = !self.ift;
    }

    /// Alt selects the Picker temporarily, but only in Draw mode
    /// (`C4EditCursor.cpp:773-792`; `C4ToolsDlg.cpp:1011-1025`).
    pub fn press_alt(&mut self, draw_mode: bool) {
        if draw_mode {
            self.set_tool(Tool::Picker, true);
        }
    }

    /// Releasing Alt restores the previous tool.
    pub fn release_alt(&mut self) {
        if let Some(previous) = self.restore_tool.take() {
            self.tool = previous;
        }
    }

    /// `C4EditCursor::LeftButtonDown`. Brush draws immediately (`:234`), Fill
    /// only arms `Hold` and is applied from `Execute` (`:74`), and Line/Rect
    /// record their anchor and emit on release (`:301-304`).
    pub fn press(&mut self, x: i32, y: i32) -> Option<DrawControl> {
        self.hold = true;
        self.anchor = Some((x, y));
        match self.tool {
            Tool::Brush => Some(DrawControl::Brush { x, y }),
            Tool::Line | Tool::Rect | Tool::Fill | Tool::Picker => None,
        }
    }

    /// `C4EditCursor::Move` — `if (Hold) ApplyToolBrush()` (`:159`). Only the
    /// brush draws while dragging.
    pub fn drag(&mut self, x: i32, y: i32) -> Option<DrawControl> {
        (self.hold && self.tool == Tool::Brush).then_some(DrawControl::Brush { x, y })
    }

    /// `C4EditCursor::LeftButtonUp` — Line and Rect emit once, with the anchor
    /// and the release point (`:301-304`).
    pub fn release(&mut self, x: i32, y: i32) -> Option<DrawControl> {
        let anchor = self.anchor.take();
        self.hold = false;
        let (ax, ay) = anchor?;
        match self.tool {
            Tool::Line => Some(DrawControl::Line {
                x: ax,
                y: ay,
                x2: x,
                y2: y,
            }),
            Tool::Rect => Some(DrawControl::Rect {
                x: ax,
                y: ay,
                x2: x,
                y2: y,
            }),
            Tool::Brush | Tool::Fill | Tool::Picker => None,
        }
    }

    /// `C4EditCursor::Execute` — `if (Hold) if (!Game.HaltCount) if (Console.Editing)
    /// ApplyToolFill()` (`:74`). Fill repeats every frame while the button is
    /// held and the game is running; a halted game refuses to arm it.
    pub fn execute_frame(&self, halted: bool, editing: bool) -> Option<DrawControl> {
        if !self.hold || self.tool != Tool::Fill || halted || !editing {
            return None;
        }
        let (x, y) = self.anchor?;
        Some(DrawControl::Fill { x, y, y2: y })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C4ToolsDlg.h:33-41,148; C4ToolsDlg.cpp:732-737; C4EditCursor.cpp:74,159,
    // 234,301-304,773-792 — tool cycling, grade clamping, the per-tool gesture
    // cadence and the Alt picker override.
    #[test]
    fn console_draw_mode_routes_pointer_gestures_through_tools_state() {
        let mut tools = DeveloperTools::default();
        assert_eq!(tools.tool(), Tool::Brush);
        assert_eq!(tools.grade(), GRADE_DEFAULT);

        // ToggleTool cycles Brush->Line->Rect->Fill->Brush, never Picker.
        for expected in [Tool::Line, Tool::Rect, Tool::Fill, Tool::Brush] {
            tools.toggle_tool();
            assert_eq!(tools.tool(), expected);
        }
        tools.set_tool(Tool::Picker, false);
        tools.toggle_tool();
        assert_eq!(
            tools.tool(),
            Tool::Line,
            "(Picker + 1) % 4 lands on Line, so cycling never sticks on Picker"
        );

        // Grade clamps at both ends and steps five per key press.
        tools.set_grade(0);
        assert_eq!(tools.grade(), GRADE_MIN);
        tools.set_grade(999);
        assert_eq!(tools.grade(), GRADE_MAX);
        tools.set_grade(20);
        tools.change_grade(1);
        assert_eq!(tools.grade(), 25);
        tools.change_grade(-2);
        assert_eq!(tools.grade(), 15);
        tools.change_grade(-100);
        assert_eq!(tools.grade(), GRADE_MIN, "a large step still clamps");

        // Brush draws on click and on every drag step (:159,:234).
        tools.set_tool(Tool::Brush, false);
        assert_eq!(
            tools.press(10, 20),
            Some(DrawControl::Brush { x: 10, y: 20 })
        );
        assert_eq!(
            tools.drag(11, 21),
            Some(DrawControl::Brush { x: 11, y: 21 })
        );
        assert_eq!(
            tools.release(12, 22),
            None,
            "the brush emits nothing on release"
        );
        assert_eq!(tools.drag(13, 23), None, "no drag emission once released");

        // Line and Rect emit once on release, carrying both coordinate pairs.
        tools.set_tool(Tool::Line, false);
        assert_eq!(tools.press(1, 2), None);
        assert_eq!(tools.drag(5, 6), None, "line does not draw while dragging");
        assert_eq!(
            tools.release(7, 8),
            Some(DrawControl::Line {
                x: 1,
                y: 2,
                x2: 7,
                y2: 8
            })
        );
        tools.set_tool(Tool::Rect, false);
        tools.press(1, 2);
        assert_eq!(
            tools.release(7, 8),
            Some(DrawControl::Rect {
                x: 1,
                y: 2,
                x2: 7,
                y2: 8
            })
        );

        // Fill arms Hold on click and repeats from Execute while running.
        tools.set_tool(Tool::Fill, false);
        assert_eq!(
            tools.press(4, 9),
            None,
            "fill emits nothing on the click itself"
        );
        assert!(tools.holding());
        let fill = Some(DrawControl::Fill { x: 4, y: 9, y2: 9 });
        assert_eq!(tools.execute_frame(false, true), fill);
        assert_eq!(
            tools.execute_frame(false, true),
            fill,
            "fill repeats every frame while held"
        );
        // A halted game refuses it, as does a non-editing console (:74).
        assert_eq!(tools.execute_frame(true, true), None);
        assert_eq!(tools.execute_frame(false, false), None);
        tools.release(4, 9);
        assert_eq!(
            tools.execute_frame(false, true),
            None,
            "releasing stops the repeat"
        );

        // Alt selects Picker temporarily, in Draw mode only.
        tools.set_tool(Tool::Rect, false);
        tools.press_alt(false);
        assert_eq!(
            tools.tool(),
            Tool::Rect,
            "Alt does nothing outside Draw mode"
        );
        tools.press_alt(true);
        assert_eq!(tools.tool(), Tool::Picker);
        // Holding Alt does not stack overrides.
        tools.press_alt(true);
        tools.release_alt();
        assert_eq!(tools.tool(), Tool::Rect, "releasing Alt restores the tool");
        tools.release_alt();
        assert_eq!(tools.tool(), Tool::Rect, "a second release is inert");

        // IFT toggles independently of the tool.
        assert!(tools.ift());
        tools.toggle_ift();
        assert!(!tools.ift());
        tools.set_ift(true);
        assert!(tools.ift());
    }
}
