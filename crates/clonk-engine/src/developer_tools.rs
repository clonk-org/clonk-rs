//! The developer console's landscape drawing tool state.
//!
//! `C4ToolsDlg` owns the selected tool, grade and IFT flag; `C4EditCursor`
//! turns pointer gestures into `EMDT_*` draw controls at a per-tool cadence
//! (`C4EditCursor.cpp:74,159,234,301-304`).
//!
//! This is the state machine only — dialog chrome, the shared developer window
//! host and the control-queue round trip for `EMDT_SetMode` stay out; the
//! console-surface entries in `PORT_STATUS.md` track what remains.

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
    /// `EMDT_Line`, emitted once on release. The first pair is the **live
    /// cursor** and the second the press anchor — that is C++'s argument
    /// order, `C4ControlEMDrawTool(EMDT_Line, Mode, X, Y, X2, Y2, ...)`
    /// (`:558`), and the two are not interchangeable on the wire even though
    /// `ForLine` normalizes them before drawing.
    Line { x: i32, y: i32, x2: i32, y2: i32 },
    /// `EMDT_Rect`, emitted once on release, in the same order as
    /// [`Self::Line`] (`:566`).
    Rect { x: i32, y: i32, x2: i32, y2: i32 },
    /// `EMDT_Fill` at the live cursor, with `X2` forced to `0` and IFT to
    /// false (`:579`). C++ passes its retained `Y2` for the third field, which
    /// is whatever a previous Line or Rect drag left there; nothing reads it,
    /// because the `EMDT_Fill` executor uses only `iX`, `iY` and `iGrade`
    /// (`C4Control.cpp:1035-1047`).
    Fill { x: i32, y: i32, y2: i32 },
}

/// The retained tool state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeveloperTools {
    /// `C4ToolsDlg::Active`. On the reference macOS build this is the *whole*
    /// observable state of "the dialog is open" — see [`Self::open`].
    active: bool,
    tool: Tool,
    /// The tool to restore when a temporary Picker override ends.
    restore_tool: Option<Tool>,
    grade: i32,
    ift: bool,
    /// `C4EditCursor::Hold` — set while the button is down.
    hold: bool,
    /// `C4EditCursor::X`/`Y`. `Move` overwrites these on every pointer message
    /// before it looks at the mode (`C4EditCursor.cpp:119-121`), so every
    /// emitted control reads the *live* cursor, not where the gesture began.
    cursor: (i32, i32),
    /// `C4EditCursor::X2`/`Y2` — where the current gesture began.
    anchor: Option<(i32, i32)>,
    /// `C4ToolsDlg::Material`, defaulting to `"Earth"`.
    material: String,
    /// `C4ToolsDlg::Texture`, defaulting to `"Rough"`.
    texture: String,
}

/// `C4ToolsDlg::Default`'s starting material and texture (`C4ToolsDlg.cpp`).
pub const DEFAULT_MATERIAL: &str = "Earth";
pub const DEFAULT_TEXTURE: &str = "Rough";

/// The refresh `C4ToolsDlg::Open` runs after the dialog exists, in call order.
///
/// On a build with neither `_WIN32` nor `WITH_DEVELOPER_MODE` — which is the
/// **default macOS build**, and the one this port is checked against
/// (`WITH_DEVELOPER_MODE` defaults to `OFF`, and the oracle's own arm64 build
/// has `WITH_DEVELOPER_MODE:BOOL=OFF`, `USE_SDL_MAINLOOP:BOOL=ON`) — `Open`
/// creates **no window at all**: it falls straight through to this sequence and
/// sets `Active`. So on the reference build this list *is* the dialog.
pub const TOOLS_OPEN_REFRESH: [ToolsRefresh; 6] = [
    ToolsRefresh::Grade,
    ToolsRefresh::LandscapeMode,
    ToolsRefresh::Tool,
    ToolsRefresh::Ift,
    ToolsRefresh::Materials,
    ToolsRefresh::Enablement,
];

/// One step of `C4ToolsDlg::Open`'s trailing refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolsRefresh {
    /// `InitGradeCtrl`.
    Grade,
    /// `UpdateLandscapeModeCtrls`.
    LandscapeMode,
    /// `UpdateToolCtrls`.
    Tool,
    /// `UpdateIFTControls`.
    Ift,
    /// `InitMaterialCtrls`.
    Materials,
    /// `EnableControls`, which itself ends in `UpdatePreview`.
    Enablement,
}

impl Default for DeveloperTools {
    fn default() -> Self {
        // `C4ToolsDlg::Default` (`C4ToolsDlg.cpp`).
        Self {
            active: false,
            tool: Tool::Brush,
            restore_tool: None,
            grade: GRADE_DEFAULT,
            ift: true,
            hold: false,
            cursor: (0, 0),
            anchor: None,
            material: DEFAULT_MATERIAL.to_owned(),
            texture: DEFAULT_TEXTURE.to_owned(),
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

    /// `C4EditCursor::X`/`Y` — where the last pointer message left the cursor.
    pub fn cursor(&self) -> (i32, i32) {
        self.cursor
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn material(&self) -> &str {
        &self.material
    }

    pub fn texture(&self) -> &str {
        &self.texture
    }

    pub fn set_material(&mut self, material: impl Into<String>) {
        self.material = material.into();
    }

    pub fn set_texture(&mut self, texture: impl Into<String>) {
        self.texture = texture.into();
    }

    /// `C4ToolsDlg::Open`. Marks the dialog active and returns the ordered
    /// refresh it runs; re-opening an already-open dialog repeats the refresh,
    /// as C++ does on every call that reaches the tail.
    pub fn open(&mut self) -> [ToolsRefresh; 6] {
        self.active = true;
        TOOLS_OPEN_REFRESH
    }

    /// `C4ToolsDlg::Clear`. Away from Win32 this only clears `Active` — the
    /// retained tool, grade, IFT, material and texture all survive, which is
    /// why re-opening restores the previous selection rather than the defaults.
    pub fn clear(&mut self) {
        self.active = false;
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
        self.cursor = (x, y);
        self.anchor = Some((x, y));
        match self.tool {
            Tool::Brush => Some(DrawControl::Brush { x, y }),
            Tool::Line | Tool::Rect | Tool::Fill | Tool::Picker => None,
        }
    }

    /// `C4EditCursor::Move` — `if (Hold) ApplyToolBrush()` (`:159`). Only the
    /// brush draws while dragging, but the cursor moves for every tool: `Move`
    /// assigns X/Y before it dispatches on the mode at all (`:119-121`).
    pub fn drag(&mut self, x: i32, y: i32) -> Option<DrawControl> {
        self.cursor = (x, y);
        (self.hold && self.tool == Tool::Brush).then_some(DrawControl::Brush { x, y })
    }

    /// `C4EditCursor::LeftButtonUp` — Line and Rect emit once (`:301-304`).
    ///
    /// The release point leads and the anchor follows, because that is the
    /// order `ApplyToolLine`/`ApplyToolRect` pass them: X/Y are the live
    /// cursor the window's release message already moved (`:558`, `:566`).
    pub fn release(&mut self, x: i32, y: i32) -> Option<DrawControl> {
        self.cursor = (x, y);
        let anchor = self.anchor.take();
        self.hold = false;
        let (ax, ay) = anchor?;
        match self.tool {
            Tool::Line => Some(DrawControl::Line {
                x,
                y,
                x2: ax,
                y2: ay,
            }),
            Tool::Rect => Some(DrawControl::Rect {
                x,
                y,
                x2: ax,
                y2: ay,
            }),
            Tool::Brush | Tool::Fill | Tool::Picker => None,
        }
    }

    /// `C4EditCursor::Execute` — `if (Hold) if (!Game.HaltCount) if (Console.Editing)
    /// ApplyToolFill()` (`:74`). Fill repeats every frame while the button is
    /// held and the game is running; a halted game refuses to arm it.
    ///
    /// It fills wherever the cursor *now* is, not where the drag began —
    /// `ApplyToolFill` reads the same live X/Y every other tool does (`:579`).
    pub fn execute_frame(&self, halted: bool, editing: bool) -> Option<DrawControl> {
        if !self.hold || self.tool != Tool::Fill || halted || !editing {
            return None;
        }
        let (x, y) = self.cursor;
        Some(DrawControl::Fill { x, y, y2: y })
    }

    /// The `fThroughControl == true` arm of `C4ToolsDlg::SetLandscapeMode`
    /// (`C4ToolsDlg.cpp:880-894`), applied to this dialog state.
    ///
    /// Call this only when the queued `EMDT_SetMode` control *executes*; the
    /// local request path must change nothing — see
    /// [`landscape_mode_needs_confirmation`].
    pub fn apply_landscape_mode(
        &mut self,
        current: LandscapeMode,
        target: LandscapeMode,
    ) -> LandscapeModeChange {
        let change = landscape_mode_change(current, target, self.tool);
        self.set_tool(change.tool, false);
        change
    }
}

/// `C4LSC_*` (`C4Landscape.h:38-41`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LandscapeMode {
    Undefined,
    Dynamic,
    Static,
    Exact,
}

/// Whether the local `SetLandscapeMode` must show `IDS_CNS_EXACTTOSTATIC`
/// before enqueueing anything (`C4ToolsDlg.cpp:869-874`).
///
/// Only Exact -> Static loses data, and only the *local* path asks — a mode
/// change arriving through the control queue is never re-confirmed. Declining
/// aborts: nothing is enqueued and nothing changes.
pub fn landscape_mode_needs_confirmation(current: LandscapeMode, target: LandscapeMode) -> bool {
    current == LandscapeMode::Exact && target == LandscapeMode::Static
}

/// What executing an `EMDT_SetMode` control does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LandscapeModeChange {
    pub mode: LandscapeMode,
    /// `Game.Landscape.MapToLandscape()` — Exact -> Static redraws the
    /// landscape from the map (`C4ToolsDlg.cpp:884-886`).
    pub redraw_from_map: bool,
    /// The tool after `SetTool(C4TLS_Brush, false)`'s correction: Fill exists
    /// only in Exact mode, so any other mode falls back to Brush
    /// (`:888-890`). Every other tool is left alone.
    pub tool: Tool,
}

/// `C4ToolsDlg::SetLandscapeMode(iMode, true)` (`C4ToolsDlg.cpp:880-894`).
pub fn landscape_mode_change(
    current: LandscapeMode,
    target: LandscapeMode,
    tool: Tool,
) -> LandscapeModeChange {
    LandscapeModeChange {
        mode: target,
        redraw_from_map: current == LandscapeMode::Exact && target == LandscapeMode::Static,
        tool: match (target, tool) {
            (mode, Tool::Fill) if mode != LandscapeMode::Exact => Tool::Brush,
            (_, tool) => tool,
        },
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
        // C4ToolsDlg::Default — the exact starting state.
        assert!(!tools.active());
        assert_eq!(tools.material(), "Earth");
        assert_eq!(tools.texture(), "Rough");
        assert!(tools.ift());

        // C4ToolsDlg::Open — on a build with neither _WIN32 nor
        // WITH_DEVELOPER_MODE (the default macOS build, and the one the oracle
        // itself is compiled as) no window is created at all: Open sets Active
        // and runs this refresh, in this order.
        assert_eq!(
            tools.open(),
            [
                ToolsRefresh::Grade,
                ToolsRefresh::LandscapeMode,
                ToolsRefresh::Tool,
                ToolsRefresh::Ift,
                ToolsRefresh::Materials,
                ToolsRefresh::Enablement,
            ]
        );
        assert!(tools.active());

        // C4ToolsDlg::Clear drops Active and *nothing else*, which is why
        // re-opening restores the previous selection rather than the defaults.
        tools.set_grade(31);
        tools.set_material("Granite");
        tools.clear();
        assert!(!tools.active());
        assert_eq!(tools.grade(), 31);
        assert_eq!(tools.material(), "Granite");
        tools.open();
        assert_eq!(tools.grade(), 31);
        assert_eq!(tools.material(), "Granite");
        tools.set_grade(GRADE_DEFAULT);
        tools.set_material(DEFAULT_MATERIAL);

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

        // Line and Rect emit once on release, carrying both coordinate pairs
        // in C++'s argument order: `ApplyToolLine` passes the *live* cursor
        // first and the press anchor second (`C4EditCursor.cpp:558,566`),
        // because `Move` overwrites X/Y on every motion (`:119-121`) while
        // `LeftButtonDown` freezes X2/Y2 at the press (`:225-226`).
        tools.set_tool(Tool::Line, false);
        assert_eq!(tools.press(1, 2), None);
        assert_eq!(tools.drag(5, 6), None, "line does not draw while dragging");
        assert_eq!(
            tools.release(7, 8),
            Some(DrawControl::Line {
                x: 7,
                y: 8,
                x2: 1,
                y2: 2
            })
        );
        tools.set_tool(Tool::Rect, false);
        tools.press(1, 2);
        assert_eq!(
            tools.release(7, 8),
            Some(DrawControl::Rect {
                x: 7,
                y: 8,
                x2: 1,
                y2: 2
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
        // `ApplyToolFill` reads the live X/Y too, so a held fill follows the
        // cursor rather than staying at the press point (`:579`).
        assert_eq!(tools.drag(50, 60), None, "fill emits nothing on the drag");
        assert_eq!(
            tools.execute_frame(false, true),
            Some(DrawControl::Fill {
                x: 50,
                y: 60,
                y2: 60
            }),
            "a held fill tracks the cursor"
        );
        // A halted game refuses it, as does a non-editing console (:74).
        assert_eq!(tools.execute_frame(true, true), None);
        assert_eq!(tools.execute_frame(false, false), None);
        tools.release(50, 60);
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

        // C4ToolsDlg.cpp:865-894 — the landscape-mode change is a control, not
        // a local edit. Only Exact -> Static warns, and only from the local
        // path; the control-executed arm never re-asks.
        assert!(landscape_mode_needs_confirmation(
            LandscapeMode::Exact,
            LandscapeMode::Static
        ));
        assert!(!landscape_mode_needs_confirmation(
            LandscapeMode::Static,
            LandscapeMode::Exact
        ));
        assert!(!landscape_mode_needs_confirmation(
            LandscapeMode::Exact,
            LandscapeMode::Dynamic
        ));
        assert!(!landscape_mode_needs_confirmation(
            LandscapeMode::Exact,
            LandscapeMode::Exact
        ));

        // Exact -> Static also redraws the landscape from the map.
        assert_eq!(
            landscape_mode_change(LandscapeMode::Exact, LandscapeMode::Static, Tool::Brush),
            LandscapeModeChange {
                mode: LandscapeMode::Static,
                redraw_from_map: true,
                tool: Tool::Brush,
            }
        );
        assert!(
            !landscape_mode_change(LandscapeMode::Static, LandscapeMode::Exact, Tool::Brush)
                .redraw_from_map
        );

        // Fill exists only in Exact mode, so leaving Exact forces Brush...
        assert_eq!(
            landscape_mode_change(LandscapeMode::Exact, LandscapeMode::Static, Tool::Fill).tool,
            Tool::Brush
        );
        assert_eq!(
            landscape_mode_change(LandscapeMode::Exact, LandscapeMode::Dynamic, Tool::Fill).tool,
            Tool::Brush
        );
        // ...while staying in Exact keeps it, and no other tool is corrected.
        assert_eq!(
            landscape_mode_change(LandscapeMode::Static, LandscapeMode::Exact, Tool::Fill).tool,
            Tool::Fill
        );
        assert_eq!(
            landscape_mode_change(LandscapeMode::Exact, LandscapeMode::Static, Tool::Line).tool,
            Tool::Line
        );

        // Applying it to live dialog state moves the tool with it.
        let mut exact = DeveloperTools::default();
        exact.set_tool(Tool::Fill, false);
        let applied = exact.apply_landscape_mode(LandscapeMode::Exact, LandscapeMode::Static);
        assert!(applied.redraw_from_map);
        assert_eq!(exact.tool(), Tool::Brush);
    }
}
