//! Recursive classic `C4GUI::ContextMenu` chassis.
//!
//! Geometry, input routing, placement and sounds follow `src/C4GuiMenu.cpp`
//! and `C4GUI::Screen::{DoContext,MouseInput}` in `src/C4Gui.cpp`.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Surface};

use crate::classic_gui::{
    draw_3d_frame, draw_engine_box, draw_engine_frame, draw_facet_stretch, IntRect,
};
use crate::clonk_fonts::expand_hotkey_markup;
use crate::{GuiPoint, ImageData, KeyCode};

const MARGIN: i32 = 5;
const ROW_SPACING: i32 = 1;
const MIN_INTERIOR_WIDTH: i32 = 30;
const MIN_INTERIOR_HEIGHT: i32 = 8;
const EMPTY_MENU_WIDTH: i32 = 40;
const EMPTY_MENU_HEIGHT: i32 = 7;
const ICON_CELL: u32 = 40;
/// `C4GUI_ToolTipShowTime`: the cursor must be still for this long before a
/// tooltip may be drawn.
pub const CLASSIC_TOOLTIP_DELAY: Duration = Duration::from_millis(500);
const TOOLTIP_DELAY: Duration = CLASSIC_TOOLTIP_DELAY;
const TOOLTIP_MAX_WIDTH: i32 = 500;

const CONTEXT_BACKGROUND: u32 = 0x4f3f_1a00;
const CONTEXT_SELECTION: u32 = 0xafaf_0000;
const TOOLTIP_BACKGROUND: u32 = 0x00f1_ea78;
const TOOLTIP_FRAME: u32 = 0x7f00_0000;
const CONTEXT_TEXT: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const TOOLTIP_TEXT: [u8; 4] = [0x48, 0x32, 0x22, 0xff];

/// Process-level mouse-input state used to gate classic delayed tooltips.
///
/// This mirrors the relevant `C4GUI::CMouse` fields instead of attaching a
/// separate timer to every control. Pointer coordinates are compared after
/// integer-pixel conversion because native GUI input receives integer screen
/// positions. A key or gamepad event clears the active-input flag without
/// changing the retained pointer or timer; only a different pointer pixel, a
/// pointer button, or a wheel event makes pointer input active again.
#[derive(Clone, Debug)]
pub struct ClassicTooltipTracker {
    pointer: Option<GuiPoint>,
    last_pointer_activity: Instant,
    pointer_active: bool,
}

impl Default for ClassicTooltipTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ClassicTooltipTracker {
    /// Creates an inactive tracker using the current monotonic clock value.
    pub fn new() -> Self {
        Self::new_at(Instant::now())
    }

    /// Deterministic constructor for hosts and tests that own the clock.
    pub fn new_at(now: Instant) -> Self {
        Self {
            pointer: None,
            last_pointer_activity: now,
            pointer_active: false,
        }
    }

    pub const fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer
    }

    pub const fn pointer_active(&self) -> bool {
        self.pointer_active
    }

    /// Records pointer movement, returning whether the native integer pixel
    /// changed. Subpixel-only motion updates the retained draw position but
    /// neither restarts the delay nor reactivates pointer input after a key.
    pub fn note_pointer_move(&mut self, point: GuiPoint) -> bool {
        self.note_pointer_move_at(point, Instant::now())
    }

    pub fn note_pointer_move_at(&mut self, point: GuiPoint, now: Instant) -> bool {
        let moved = self.pointer.is_none_or(|previous| {
            previous.x as i32 != point.x as i32 || previous.y as i32 != point.y as i32
        });
        self.pointer = Some(point);
        if moved {
            self.note_pointer_activity_at(now);
        }
        moved
    }

    /// A pointer button is active mouse input even if the pointer did not
    /// move, matching `CMouse::Input(iButton != 0, ...)`.
    pub fn note_pointer_button(&mut self) {
        self.note_pointer_button_at(Instant::now());
    }

    pub fn note_pointer_button_at(&mut self, now: Instant) {
        self.note_pointer_activity_at(now);
    }

    /// Wheel input has the same tooltip-reset semantics as a pointer button.
    pub fn note_pointer_wheel(&mut self) {
        self.note_pointer_wheel_at(Instant::now());
    }

    pub fn note_pointer_wheel_at(&mut self, now: Instant) {
        self.note_pointer_activity_at(now);
    }

    /// Mirrors `CMouse::ResetActiveInput` for keyboard and gamepad input.
    pub fn note_non_pointer_input(&mut self) {
        self.pointer_active = false;
    }

    /// Suppresses tooltips and forgets the pointer when the cursor leaves the
    /// application. The next real pointer event starts a fresh delay.
    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.pointer_active = false;
    }

    /// Returns the retained cursor whenever pointer input is active, both
    /// before and after the delay. Hosts use this to keep a frame cache live
    /// whenever the control under this point owns a tooltip.
    pub const fn pending_pointer(&self) -> Option<GuiPoint> {
        if self.pointer_active {
            self.pointer
        } else {
            None
        }
    }

    pub fn eligible_pointer(&self) -> Option<GuiPoint> {
        self.eligible_pointer_at(Instant::now())
    }

    /// Returns the cursor only when pointer input is active and the exact
    /// classic delay has elapsed. A clock value before the last activity is
    /// treated as not ready.
    pub fn eligible_pointer_at(&self, now: Instant) -> Option<GuiPoint> {
        let pointer = self.pointer?;
        (self.pointer_active
            && now
                .checked_duration_since(self.last_pointer_activity)
                .is_some_and(|elapsed| elapsed >= CLASSIC_TOOLTIP_DELAY))
        .then_some(pointer)
    }

    fn note_pointer_activity_at(&mut self, now: Instant) {
        self.last_pointer_activity = now;
        self.pointer_active = true;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuIcon {
    /// `Ico_None`: no picture and no indentation.
    None,
    /// `Ico_Empty`: reserve the icon column without drawing a phase.
    Empty,
    /// A standard 40×40 `GUIIcons.png` phase.
    Phase(u16),
}

type LazySubmenu<A> = Arc<dyn Fn() -> Vec<ContextMenuEntry<A>> + Send + Sync>;

enum ContextSubmenu<A: Clone> {
    Entries(Vec<ContextMenuEntry<A>>),
    Lazy(LazySubmenu<A>),
    /// Child entries the widget cannot compute itself. Opening emits
    /// `ContextMenuEvent::SubmenuRequested` with this request so the host
    /// answers with live state via [`ClassicContextMenu::fill_requested_submenu`],
    /// mirroring the C4GUI submenu-open `ContextHandler::OnSubcontext`
    /// callback (src/C4GuiMenu.cpp:469-506).
    Deferred(A),
}

impl<A: Clone> Clone for ContextSubmenu<A> {
    fn clone(&self) -> Self {
        match self {
            Self::Entries(entries) => Self::Entries(entries.clone()),
            Self::Lazy(provider) => Self::Lazy(Arc::clone(provider)),
            Self::Deferred(request) => Self::Deferred(request.clone()),
        }
    }
}

/// One context-menu entry. Builders deliberately allow both an action and a
/// submenu because the C++ entry stores the two handlers independently.
pub struct ContextMenuEntry<A: Clone> {
    pub text: String,
    pub tooltip: Option<String>,
    pub icon: ContextMenuIcon,
    pub hotkey: Option<char>,
    pub action: Option<A>,
    submenu: Option<ContextSubmenu<A>>,
}

impl<A: Clone> Clone for ContextMenuEntry<A> {
    fn clone(&self) -> Self {
        Self {
            text: self.text.clone(),
            tooltip: self.tooltip.clone(),
            icon: self.icon,
            hotkey: self.hotkey,
            action: self.action.clone(),
            submenu: self.submenu.clone(),
        }
    }
}

impl<A: Clone + fmt::Debug> fmt::Debug for ContextMenuEntry<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextMenuEntry")
            .field("text", &self.text)
            .field("tooltip", &self.tooltip)
            .field("icon", &self.icon)
            .field("hotkey", &self.hotkey)
            .field("action", &self.action)
            .field("has_submenu", &self.submenu.is_some())
            .finish()
    }
}

impl<A: Clone> ContextMenuEntry<A> {
    pub fn new(text: impl Into<String>) -> Self {
        let (text, hotkey) = expand_hotkey_markup(&text.into());
        Self {
            text,
            tooltip: None,
            icon: ContextMenuIcon::None,
            hotkey,
            action: None,
            submenu: None,
        }
    }

    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub const fn with_icon(mut self, icon: ContextMenuIcon) -> Self {
        self.icon = icon;
        self
    }

    pub fn with_hotkey(mut self, hotkey: char) -> Self {
        self.hotkey = Some(hotkey.to_ascii_uppercase());
        self
    }

    pub fn with_action(mut self, action: A) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_submenu(mut self, entries: Vec<Self>) -> Self {
        self.submenu = Some(ContextSubmenu::Entries(entries));
        self
    }

    pub fn with_lazy_submenu<F>(mut self, provider: F) -> Self
    where
        F: Fn() -> Vec<Self> + Send + Sync + 'static,
    {
        self.submenu = Some(ContextSubmenu::Lazy(Arc::new(provider)));
        self
    }

    /// Marks a submenu whose children only the host can compute. Opening
    /// emits [`ContextMenuEvent::SubmenuRequested`] carrying `request`; the
    /// host must answer with [`ClassicContextMenu::fill_requested_submenu`].
    pub fn with_deferred_submenu(mut self, request: A) -> Self {
        self.submenu = Some(ContextSubmenu::Deferred(request));
        self
    }

    pub const fn has_submenu(&self) -> bool {
        self.submenu.is_some()
    }

    fn submenu_entries(&self) -> Option<Vec<Self>> {
        match self.submenu.as_ref()? {
            ContextSubmenu::Entries(entries) => Some(entries.clone()),
            ContextSubmenu::Lazy(provider) => Some(provider()),
            ContextSubmenu::Deferred(_) => None,
        }
    }

    fn deferred_submenu_request(&self) -> Option<A> {
        match self.submenu.as_ref()? {
            ContextSubmenu::Deferred(request) => Some(request.clone()),
            ContextSubmenu::Entries(_) | ContextSubmenu::Lazy(_) => None,
        }
    }
}

/// Validated, cheaply cloned classic resources owned by an open menu. Owning
/// these avoids a self-referential borrow when a `GameApp` stores the popup
/// alongside its asset cache.
#[derive(Clone)]
pub struct ContextMenuResources {
    font: ClonkFont,
    tooltip_font: ClonkFont,
    icons: ImageData,
    submenu_arrow: ImageData,
}

impl ContextMenuResources {
    pub fn new(
        font: &ClonkFont,
        tooltip_font: &ClonkFont,
        icons: &ImageData,
        submenu_arrow: &ImageData,
    ) -> Result<Self> {
        ensure!(
            icons.width() >= ICON_CELL
                && icons.height() >= ICON_CELL
                && icons.width().is_multiple_of(ICON_CELL)
                && icons.height().is_multiple_of(ICON_CELL),
            "GUIIcons.png cannot form the classic 40px icon grid: got {}x{}",
            icons.width(),
            icons.height()
        );
        ensure!(
            submenu_arrow.width() == 8 && submenu_arrow.height() == 16,
            "GUISubmenu.png must be the exact 8x16 classic facet: got {}x{}",
            submenu_arrow.width(),
            submenu_arrow.height()
        );
        ensure!(
            font.line_height > 0,
            "classic context-menu font has no line height"
        );
        ensure!(
            tooltip_font.line_height > 0,
            "classic tooltip font has no line height"
        );
        Ok(Self {
            font: font.clone(),
            tooltip_font: tooltip_font.clone(),
            icons: icons.clone(),
            submenu_arrow: submenu_arrow.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuPointerButton {
    Left,
    Right,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuSound {
    DoorOpen,
    DoorClose,
    Command,
    Click,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextMenuEvent<A> {
    Sound(ContextMenuSound),
    Closed,
    Activated(A),
    /// A deferred submenu wants its children. C4GUI fills a submenu at open
    /// time through the entry's `ContextHandler::OnSubcontext` callback
    /// (src/C4GuiMenu.cpp:478-482); the host answers this event in the same
    /// dispatch via [`ClassicContextMenu::fill_requested_submenu`].
    SubmenuRequested(A),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMenuOutcome<A> {
    pub captured: bool,
    pub pass_through: bool,
    pub focus_suppressed: bool,
    pub events: Vec<ContextMenuEvent<A>>,
}

impl<A> ContextMenuOutcome<A> {
    fn new(open: bool) -> Self {
        Self {
            captured: false,
            pass_through: false,
            focus_suppressed: open,
            events: Vec::new(),
        }
    }

    fn captured(open: bool) -> Self {
        Self {
            captured: true,
            ..Self::new(open)
        }
    }

    fn passed(open: bool) -> Self {
        Self {
            pass_through: true,
            ..Self::new(open)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMenuRowLayout {
    pub index: usize,
    pub rect: IntRect,
    pub text_x: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextMenuPanelLayout {
    pub bounds: IntRect,
    pub client: IntRect,
    pub rows: Vec<ContextMenuRowLayout>,
    pub selected: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextMenuLayout {
    pub panels: Vec<ContextMenuPanelLayout>,
}

struct ContextPanel<A: Clone> {
    entries: Vec<ContextMenuEntry<A>>,
    layout: ContextMenuPanelLayout,
    selected: Option<usize>,
    hovered: Option<usize>,
    hover_started: Option<Instant>,
    submenu: Option<Box<ContextPanel<A>>>,
}

impl<A: Clone> ContextPanel<A> {
    fn new_root(
        entries: Vec<ContextMenuEntry<A>>,
        anchor: GuiPoint,
        screen: IntRect,
        resources: &ContextMenuResources,
        minimum_width: i32,
    ) -> Self {
        let (width, height, rows) = panel_dimensions(&entries, resources);
        let width = width.max(minimum_width);
        let (x, y) = flip_root(anchor.x as i32, anchor.y as i32, width, height, screen);
        Self::at(entries, x, y, width, height, rows)
    }

    fn new_child(
        entries: Vec<ContextMenuEntry<A>>,
        parent: &ContextMenuPanelLayout,
        selected_row: IntRect,
        screen: IntRect,
        resources: &ContextMenuResources,
    ) -> Self {
        let (width, height, rows) = panel_dimensions(&entries, resources);
        let anchor_x = parent.client.x + parent.client.w;
        let anchor_y = selected_row.y + selected_row.h / 2;
        let (x, y) = flip_child(anchor_x, anchor_y, width, height, parent, screen);
        Self::at(entries, x, y, width, height, rows)
    }

    fn at(
        entries: Vec<ContextMenuEntry<A>>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        local_rows: Vec<(i32, i32, i32)>,
    ) -> Self {
        let client = IntRect::new(
            x + MARGIN,
            y + MARGIN,
            (width - 2 * MARGIN).max(0),
            (height - 2 * MARGIN).max(0),
        );
        let rows = local_rows
            .into_iter()
            .enumerate()
            .map(
                |(index, (row_y, row_height, text_indent))| ContextMenuRowLayout {
                    index,
                    rect: IntRect::new(client.x, client.y + row_y, client.w, row_height),
                    text_x: client.x + text_indent,
                },
            )
            .collect();
        Self {
            entries,
            layout: ContextMenuPanelLayout {
                bounds: IntRect::new(x, y, width, height),
                client,
                rows,
                selected: None,
            },
            selected: None,
            hovered: None,
            hover_started: None,
            submenu: None,
        }
    }

    fn contains(&self, point: GuiPoint) -> bool {
        contains(self.layout.bounds, point)
    }

    fn row_at(&self, point: GuiPoint) -> Option<usize> {
        self.layout
            .rows
            .iter()
            .find(|row| contains(row.rect, point))
            .map(|row| row.index)
    }

    fn collect_layout(&self, panels: &mut Vec<ContextMenuPanelLayout>) {
        let mut layout = self.layout.clone();
        layout.selected = self.selected;
        panels.push(layout);
        if let Some(submenu) = self.submenu.as_deref() {
            submenu.collect_layout(panels);
        }
    }

    fn count(&self) -> usize {
        1 + self.submenu.as_deref().map_or(0, Self::count)
    }

    fn at_depth(&self, depth: usize) -> Option<&Self> {
        if depth == 0 {
            Some(self)
        } else {
            self.submenu.as_deref()?.at_depth(depth - 1)
        }
    }

    fn deepest(&self) -> &Self {
        self.submenu.as_deref().map_or(self, Self::deepest)
    }

    fn deepest_mut(&mut self) -> &mut Self {
        if self.submenu.is_some() {
            self.submenu
                .as_deref_mut()
                .expect("submenu checked above")
                .deepest_mut()
        } else {
            self
        }
    }
}

/// One open root context menu and its recursively owned submenu chain.
pub struct ClassicContextMenu<A: Clone> {
    resources: ContextMenuResources,
    screen: IntRect,
    root: ContextPanel<A>,
    pointer_position: GuiPoint,
    last_pointer_activity: Instant,
    pointer_active: bool,
    open: bool,
}

impl<A: Clone> ClassicContextMenu<A> {
    pub fn open(
        entries: Vec<ContextMenuEntry<A>>,
        anchor: GuiPoint,
        screen: IntRect,
        resources: ContextMenuResources,
    ) -> (Self, ContextMenuOutcome<A>) {
        Self::open_with_minimum_width(entries, anchor, screen, resources, 0)
    }

    /// Open a root menu whose outer bounds are at least `minimum_width`.
    /// C4GUI::ComboBox uses its control width here before Screen::DoContext
    /// applies the ordinary edge-flip placement.
    pub fn open_with_minimum_width(
        entries: Vec<ContextMenuEntry<A>>,
        anchor: GuiPoint,
        screen: IntRect,
        resources: ContextMenuResources,
        minimum_width: i32,
    ) -> (Self, ContextMenuOutcome<A>) {
        let root = ContextPanel::new_root(entries, anchor, screen, &resources, minimum_width);
        let mut outcome = ContextMenuOutcome::new(true);
        outcome
            .events
            .push(ContextMenuEvent::Sound(ContextMenuSound::DoorOpen));
        (
            Self {
                resources,
                screen,
                root,
                pointer_position: anchor,
                last_pointer_activity: Instant::now(),
                pointer_active: true,
                open: true,
            },
            outcome,
        )
    }

    pub const fn is_open(&self) -> bool {
        self.open
    }

    pub const fn suppresses_focus(&self) -> bool {
        self.open
    }

    pub const fn pointer_position(&self) -> GuiPoint {
        self.pointer_position
    }

    pub fn note_non_pointer_input(&mut self) {
        self.pointer_active = false;
    }

    pub fn layout(&self) -> ContextMenuLayout {
        let mut panels = Vec::new();
        if self.open {
            self.root.collect_layout(&mut panels);
        }
        ContextMenuLayout { panels }
    }

    pub fn captures_point(&self, point: GuiPoint) -> bool {
        self.open && panel_chain_contains(&self.root, point)
    }

    pub fn hovered_tooltip(&self) -> Option<&str> {
        self.hovered_tooltip_at(Instant::now())
    }

    pub fn hovered_tooltip_at(&self, now: Instant) -> Option<&str> {
        if !self.open
            || !self.pointer_active
            || now.saturating_duration_since(self.last_pointer_activity) < TOOLTIP_DELAY
        {
            return None;
        }
        hovered_entry_at(&self.root, self.pointer_position)?
            .tooltip
            .as_deref()
            .filter(|tooltip| !tooltip.is_empty())
    }

    pub fn handle_pointer_move(&mut self, point: GuiPoint) -> ContextMenuOutcome<A> {
        if point.x as i32 != self.pointer_position.x as i32
            || point.y as i32 != self.pointer_position.y as i32
        {
            self.last_pointer_activity = Instant::now();
            self.pointer_active = true;
        }
        self.pointer_position = point;
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        let mut outcome = ContextMenuOutcome::new(true);
        if pointer_move_panel(
            &mut self.root,
            point,
            self.screen,
            &self.resources,
            &mut outcome.events,
        ) {
            outcome.captured = true;
        } else {
            clear_deepest_leaf_hover(&mut self.root);
            outcome.pass_through = true;
        }
        outcome
    }

    pub fn handle_pointer_down(
        &mut self,
        point: GuiPoint,
        button: ContextMenuPointerButton,
    ) -> ContextMenuOutcome<A> {
        self.pointer_position = point;
        self.last_pointer_activity = Instant::now();
        self.pointer_active = true;
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        if !panel_chain_contains(&self.root, point) {
            if matches!(
                button,
                ContextMenuPointerButton::Left | ContextMenuPointerButton::Right
            ) {
                let mut outcome = ContextMenuOutcome::passed(false);
                self.close_root(true, &mut outcome.events);
                outcome.focus_suppressed = false;
                return outcome;
            }
            clear_deepest_leaf_hover(&mut self.root);
            return ContextMenuOutcome::passed(true);
        }

        let mut outcome = ContextMenuOutcome::captured(true);
        // Resolve against the tree that received this event. Selecting the
        // row below may synchronously open an overlapping child panel, but
        // C4GUI does not redispatch the same button-down into that new child.
        let clicked_action = (button == ContextMenuPointerButton::Left)
            .then(|| action_at_point(&self.root, point))
            .flatten();
        pointer_move_panel(
            &mut self.root,
            point,
            self.screen,
            &self.resources,
            &mut outcome.events,
        );
        if let Some(action) = clicked_action {
            self.activate(action, &mut outcome.events);
            outcome.focus_suppressed = false;
        }
        outcome
    }

    pub fn handle_pointer_up(
        &mut self,
        point: GuiPoint,
        _button: ContextMenuPointerButton,
    ) -> ContextMenuOutcome<A> {
        self.last_pointer_activity = Instant::now();
        self.pointer_active = true;
        self.handle_pointer_move(point)
    }

    /// Clears the deepest hover/selection when the OS cursor leaves the
    /// application without dismissing the popup tree.
    pub fn handle_pointer_left(&mut self) -> ContextMenuOutcome<A> {
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        self.pointer_active = false;
        clear_deepest_leaf_hover(&mut self.root);
        ContextMenuOutcome::passed(true)
    }

    pub fn handle_key(&mut self, key: KeyCode) -> ContextMenuOutcome<A> {
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        self.pointer_active = false;
        match key {
            KeyCode::Up => self.handle_direction(ContextMenuDirection::Up),
            KeyCode::Down => self.handle_direction(ContextMenuDirection::Down),
            KeyCode::Left => self.handle_direction(ContextMenuDirection::Left),
            KeyCode::Right => self.handle_direction(ContextMenuDirection::Right),
            KeyCode::Escape => self.handle_gamepad_high(),
            KeyCode::Enter => self.handle_gamepad_low(),
            KeyCode::Space
            | KeyCode::Tab
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => ContextMenuOutcome::passed(true),
        }
    }

    pub fn handle_hotkey(&mut self, hotkey: char) -> ContextMenuOutcome<A> {
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        self.pointer_active = false;
        let hotkey = hotkey.to_ascii_uppercase();
        if !hotkey.is_ascii_alphanumeric() {
            return ContextMenuOutcome::passed(true);
        }
        let panel = self.root.deepest_mut();
        let Some(index) = panel
            .entries
            .iter()
            .position(|entry| entry.hotkey == Some(hotkey))
        else {
            return ContextMenuOutcome::passed(true);
        };
        let mut outcome = ContextMenuOutcome::captured(true);
        select_index(
            panel,
            Some(index),
            false,
            self.screen,
            &self.resources,
            &mut outcome.events,
        );
        self.confirm_selected(&mut outcome);
        outcome
    }

    /// Read-only counterpart to the PRIO_Context key callbacks. This is used
    /// by lower-priority global bindings to decide whether the deepest menu
    /// would consume the chord before mutating either owner.
    pub fn owns_key(&self, key: KeyCode) -> bool {
        if !self.open {
            return false;
        }
        match key {
            KeyCode::Up | KeyCode::Down | KeyCode::Right | KeyCode::Escape | KeyCode::Enter => true,
            KeyCode::Left => self.root.submenu.is_some(),
            KeyCode::Space
            | KeyCode::Tab
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => false,
        }
    }

    pub fn owns_hotkey(&self, hotkey: char) -> bool {
        self.open
            && self
                .root
                .deepest()
                .entries
                .iter()
                .any(|entry| entry.hotkey == Some(hotkey.to_ascii_uppercase()))
    }

    pub fn handle_gamepad_direction(
        &mut self,
        direction: ContextMenuDirection,
    ) -> ContextMenuOutcome<A> {
        self.handle_direction(direction)
    }

    pub fn handle_gamepad_low(&mut self) -> ContextMenuOutcome<A> {
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        self.pointer_active = false;
        let mut outcome = ContextMenuOutcome::captured(true);
        self.confirm_selected(&mut outcome);
        outcome
    }

    pub fn handle_gamepad_high(&mut self) -> ContextMenuOutcome<A> {
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        self.pointer_active = false;
        let mut outcome = ContextMenuOutcome::captured(true);
        if close_deepest_submenu(&mut self.root, true, &mut outcome.events) {
            return outcome;
        }
        self.close_root(true, &mut outcome.events);
        outcome.focus_suppressed = false;
        outcome
    }

    pub fn dismiss(&mut self, by_user: bool) -> ContextMenuOutcome<A> {
        let mut outcome = ContextMenuOutcome::captured(self.open);
        self.close_root(by_user, &mut outcome.events);
        outcome.focus_suppressed = false;
        outcome
    }

    /// Answers [`ContextMenuEvent::SubmenuRequested`]: opens the deferred
    /// child panel for the deepest panel's selected entry with host-computed
    /// entries. C4GUI opens the callback-filled submenu immediately after
    /// `OnSubcontext` returns, playing the door sound on that open
    /// (src/C4GuiMenu.cpp:480-505); an empty child menu still opens as the
    /// minimum-size box. Ignored when the selection moved off a deferred
    /// entry or its panel already opened.
    pub fn fill_requested_submenu(
        &mut self,
        entries: Vec<ContextMenuEntry<A>>,
    ) -> ContextMenuOutcome<A> {
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        let mut outcome = ContextMenuOutcome::captured(true);
        let screen = self.screen;
        let resources = self.resources.clone();
        let panel = self.root.deepest_mut();
        let deferred_index = panel.selected.filter(|index| {
            panel
                .entries
                .get(*index)
                .is_some_and(|entry| entry.deferred_submenu_request().is_some())
        });
        if let Some(index) = deferred_index.filter(|_| panel.submenu.is_none()) {
            open_child_panel(
                panel,
                index,
                entries,
                screen,
                &resources,
                &mut outcome.events,
            );
        }
        outcome
    }

    /// Number of currently visible panels, ordered from root to deepest
    /// submenu. Each panel is a distinct C++ ownership layer.
    pub fn panel_count(&self) -> usize {
        if self.open {
            self.root.count()
        } else {
            0
        }
    }

    /// Draw exactly one visible panel by root-to-leaf index, without drawing
    /// child panels or the final tooltip. Ordered native presentation commits
    /// after each call so a child panel's chrome can cover its parent text.
    pub fn render_panel(
        &self,
        surface: &mut Surface,
        index: usize,
        gamma: Option<&GammaRamp>,
    ) -> Result<()> {
        ensure!(
            self.open,
            "cannot render a panel from a closed context menu"
        );
        let panel = self
            .root
            .at_depth(index)
            .ok_or_else(|| anyhow::anyhow!("context-menu panel index {index} is out of range"))?;
        render_panel_only(surface, panel, &self.resources, gamma)
    }

    /// Draw only the delayed tooltip belonging to the current pointer hover.
    /// This is the final context-menu layer and intentionally excludes panels.
    pub fn render_tooltip_at(
        &self,
        surface: &mut Surface,
        gamma: Option<&GammaRamp>,
        now: Instant,
    ) -> bool {
        if let Some(tooltip) = self.hovered_tooltip_at(now) {
            draw_classic_tooltip(
                surface,
                &self.resources.tooltip_font,
                self.pointer_position,
                tooltip,
                gamma,
            );
            true
        } else {
            false
        }
    }

    pub fn render_tooltip(&self, surface: &mut Surface, gamma: Option<&GammaRamp>) -> bool {
        self.render_tooltip_at(surface, gamma, Instant::now())
    }

    /// Draw the recursively owned menu panels without the delayed tooltip.
    /// C4GUI draws CMouse after dialog elements and before screen-global
    /// tooltips, so hosts that draw the classic cursor need the two passes.
    pub fn render_panels(&self, surface: &mut Surface, gamma: Option<&GammaRamp>) -> Result<()> {
        if !self.open {
            return Ok(());
        }
        for index in 0..self.panel_count() {
            self.render_panel(surface, index, gamma)?;
        }
        Ok(())
    }

    pub fn render(&self, surface: &mut Surface, gamma: Option<&GammaRamp>) -> Result<()> {
        self.render_panels(surface, gamma)?;
        self.render_tooltip(surface, gamma);
        Ok(())
    }

    fn handle_direction(&mut self, direction: ContextMenuDirection) -> ContextMenuOutcome<A> {
        if !self.open {
            return ContextMenuOutcome::passed(false);
        }
        self.pointer_active = false;
        let mut outcome = ContextMenuOutcome::captured(true);
        match direction {
            ContextMenuDirection::Up | ContextMenuDirection::Down => {
                let panel = self.root.deepest_mut();
                if panel.entries.is_empty() {
                    return outcome;
                }
                let next = match (panel.selected, direction) {
                    (None, ContextMenuDirection::Up) => panel.entries.len() - 1,
                    (None, _) => 0,
                    (Some(0), ContextMenuDirection::Up) => panel.entries.len() - 1,
                    (Some(index), ContextMenuDirection::Up) => index - 1,
                    (Some(index), _) => (index + 1) % panel.entries.len(),
                };
                select_index(
                    panel,
                    Some(next),
                    false,
                    self.screen,
                    &self.resources,
                    &mut outcome.events,
                );
            }
            ContextMenuDirection::Right => {
                let panel = self.root.deepest_mut();
                open_selected_submenu(panel, self.screen, &self.resources, &mut outcome.events);
            }
            ContextMenuDirection::Left => {
                if !close_deepest_submenu(&mut self.root, true, &mut outcome.events) {
                    return ContextMenuOutcome::passed(true);
                }
            }
        }
        outcome
    }

    fn confirm_selected(&mut self, outcome: &mut ContextMenuOutcome<A>) {
        let panel = self.root.deepest_mut();
        open_selected_submenu(panel, self.screen, &self.resources, &mut outcome.events);
        let action = panel
            .selected
            .and_then(|index| panel.entries.get(index))
            .and_then(|entry| entry.action.clone());
        if let Some(action) = action {
            self.activate(action, &mut outcome.events);
            outcome.focus_suppressed = false;
        }
    }

    fn activate(&mut self, action: A, events: &mut Vec<ContextMenuEvent<A>>) {
        self.open = false;
        self.root.submenu = None;
        events.push(ContextMenuEvent::Closed);
        events.push(ContextMenuEvent::Sound(ContextMenuSound::Click));
        events.push(ContextMenuEvent::Activated(action));
    }

    fn close_root(&mut self, by_user: bool, events: &mut Vec<ContextMenuEvent<A>>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.root.submenu = None;
        if by_user {
            events.push(ContextMenuEvent::Sound(ContextMenuSound::DoorClose));
        }
        events.push(ContextMenuEvent::Closed);
    }
}

fn panel_dimensions<A: Clone>(
    entries: &[ContextMenuEntry<A>],
    resources: &ContextMenuResources,
) -> (i32, i32, Vec<(i32, i32, i32)>) {
    if entries.is_empty() {
        return (EMPTY_MENU_WIDTH, EMPTY_MENU_HEIGHT, Vec::new());
    }
    let mut natural = Vec::with_capacity(entries.len());
    let mut width = MIN_INTERIOR_WIDTH;
    let mut overall_height = 0_i32;
    for (index, entry) in entries.iter().enumerate() {
        let (text_width, measured_height) = resources.font.measure(&entry.text, true);
        let row_height = measured_height.max(resources.font.line_height).max(1);
        let icon_indent = match entry.icon {
            ContextMenuIcon::None => 0,
            ContextMenuIcon::Empty | ContextMenuIcon::Phase(_) => row_height + 2,
        };
        let submenu_width = if entry.has_submenu() {
            resources.submenu_arrow.width() as i32 + 2
        } else {
            0
        };
        let row_width = text_width
            .saturating_add(icon_indent)
            .saturating_add(submenu_width);
        width = width.max(row_width);
        if index != 0 {
            overall_height = overall_height.saturating_add(ROW_SPACING);
        }
        natural.push((overall_height, row_height, icon_indent));
        overall_height = overall_height.saturating_add(row_height);
    }
    (
        width + 2 * MARGIN,
        overall_height.max(MIN_INTERIOR_HEIGHT) + 2 * MARGIN,
        natural,
    )
}

fn flip_root(mut x: i32, mut y: i32, width: i32, height: i32, screen: IntRect) -> (i32, i32) {
    let right = screen.x + screen.w;
    let bottom = screen.y + screen.h;
    if y + height >= bottom {
        if y - screen.y < height {
            y = bottom;
        }
        y -= height;
    }
    if x + width >= right {
        if x - screen.x < width {
            x = right;
        }
        x -= width;
    }
    (x, y)
}

fn flip_child(
    mut x: i32,
    mut y: i32,
    width: i32,
    height: i32,
    parent: &ContextMenuPanelLayout,
    screen: IntRect,
) -> (i32, i32) {
    let right = screen.x + screen.w;
    let bottom = screen.y + screen.h;
    if y + height >= bottom {
        if y - screen.y < height {
            y = bottom;
        }
        y -= height;
    }
    if x + width >= right {
        if parent.client.x - screen.x < width {
            x = right;
        } else {
            x = parent.client.x;
        }
        x -= width;
    }
    (x, y)
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn panel_chain_contains<A: Clone>(panel: &ContextPanel<A>, point: GuiPoint) -> bool {
    panel
        .submenu
        .as_deref()
        .is_some_and(|submenu| panel_chain_contains(submenu, point))
        || panel.contains(point)
}

fn hovered_entry_at<A: Clone>(
    panel: &ContextPanel<A>,
    point: GuiPoint,
) -> Option<&ContextMenuEntry<A>> {
    if let Some(submenu) = panel.submenu.as_deref() {
        if panel_chain_contains(submenu, point) {
            return hovered_entry_at(submenu, point);
        }
    }
    if !panel.contains(point) {
        return None;
    }
    panel.entries.get(panel.row_at(point)?)
}

fn pointer_move_panel<A: Clone>(
    panel: &mut ContextPanel<A>,
    point: GuiPoint,
    screen: IntRect,
    resources: &ContextMenuResources,
    events: &mut Vec<ContextMenuEvent<A>>,
) -> bool {
    if let Some(submenu) = panel.submenu.as_deref_mut() {
        if pointer_move_panel(submenu, point, screen, resources, events) {
            return true;
        }
        // The global C4GUI mouse-over transition sends MouseLeave to the
        // deepest entry even when an ancestor panel receives this move.
        clear_deepest_leaf_hover(submenu);
    }
    if !panel.contains(point) {
        return false;
    }
    if contains(panel.layout.client, point) {
        let selected = panel.row_at(point);
        select_index(panel, selected, true, screen, resources, events);
    } else if panel.submenu.is_none() {
        clear_deepest_leaf_hover(panel);
    }
    true
}

fn clear_deepest_leaf_hover<A: Clone>(panel: &mut ContextPanel<A>) {
    if let Some(submenu) = panel.submenu.as_deref_mut() {
        clear_deepest_leaf_hover(submenu);
    } else {
        panel.hovered = None;
        panel.hover_started = None;
        panel.selected = None;
        panel.layout.selected = None;
    }
}

fn select_index<A: Clone>(
    panel: &mut ContextPanel<A>,
    selected: Option<usize>,
    open_submenu: bool,
    screen: IntRect,
    resources: &ContextMenuResources,
    events: &mut Vec<ContextMenuEvent<A>>,
) {
    let selected = selected.filter(|index| *index < panel.entries.len());
    let changed = panel.selected != selected;
    if changed {
        panel.selected = selected;
        panel.layout.selected = selected;
        panel.hovered = if open_submenu { selected } else { None };
        panel.hover_started = panel.hovered.map(|_| Instant::now());
        if selected.is_some() {
            events.push(ContextMenuEvent::Sound(ContextMenuSound::Command));
        }
        if panel.submenu.take().is_some() {
            events.push(ContextMenuEvent::Sound(ContextMenuSound::DoorClose));
        }
    } else if open_submenu {
        panel.hovered = selected;
        panel.hover_started.get_or_insert_with(Instant::now);
    }
    if open_submenu {
        open_selected_submenu(panel, screen, resources, events);
    }
}

fn open_selected_submenu<A: Clone>(
    panel: &mut ContextPanel<A>,
    screen: IntRect,
    resources: &ContextMenuResources,
    events: &mut Vec<ContextMenuEvent<A>>,
) -> bool {
    if panel.submenu.is_some() {
        return true;
    }
    let Some(index) = panel.selected else {
        return false;
    };
    let Some(entry) = panel.entries.get(index) else {
        return false;
    };
    // C4GUI::ContextMenu::CheckOpenSubmenu resolves the child menu through
    // the entry's OnSubcontext callback at open time (src/C4GuiMenu.cpp:
    // 469-506). A deferred entry hands that callback to the host, which
    // answers within the same dispatch via `fill_requested_submenu`.
    if let Some(request) = entry.deferred_submenu_request() {
        events.push(ContextMenuEvent::SubmenuRequested(request));
        return false;
    }
    let Some(entries) = entry.submenu_entries() else {
        return false;
    };
    open_child_panel(panel, index, entries, screen, resources, events)
}

fn open_child_panel<A: Clone>(
    panel: &mut ContextPanel<A>,
    index: usize,
    entries: Vec<ContextMenuEntry<A>>,
    screen: IntRect,
    resources: &ContextMenuResources,
    events: &mut Vec<ContextMenuEvent<A>>,
) -> bool {
    let Some(row) = panel.layout.rows.get(index).map(|row| row.rect) else {
        return false;
    };
    let submenu = ContextPanel::new_child(entries, &panel.layout, row, screen, resources);
    panel.submenu = Some(Box::new(submenu));
    events.push(ContextMenuEvent::Sound(ContextMenuSound::DoorOpen));
    true
}

fn close_deepest_submenu<A: Clone>(
    panel: &mut ContextPanel<A>,
    by_user: bool,
    events: &mut Vec<ContextMenuEvent<A>>,
) -> bool {
    let Some(submenu) = panel.submenu.as_deref_mut() else {
        return false;
    };
    if close_deepest_submenu(submenu, by_user, events) {
        return true;
    }
    panel.submenu = None;
    if by_user {
        events.push(ContextMenuEvent::Sound(ContextMenuSound::DoorClose));
    }
    true
}

fn action_at_point<A: Clone>(panel: &ContextPanel<A>, point: GuiPoint) -> Option<A> {
    if let Some(submenu) = panel.submenu.as_deref() {
        if panel_chain_contains(submenu, point) {
            return action_at_point(submenu, point);
        }
    }
    let index = panel.row_at(point)?;
    panel.entries.get(index)?.action.clone()
}

fn render_panel_only<A: Clone>(
    surface: &mut Surface,
    panel: &ContextPanel<A>,
    resources: &ContextMenuResources,
    gamma: Option<&GammaRamp>,
) -> Result<()> {
    let bounds = panel.layout.bounds;
    draw_engine_box(
        surface,
        bounds.x,
        bounds.y,
        bounds.x + bounds.w - 1,
        bounds.y + bounds.h - 1,
        CONTEXT_BACKGROUND,
        gamma,
    );
    if let Some(index) = panel.selected {
        if let Some(row) = panel.layout.rows.get(index) {
            draw_engine_box(
                surface,
                row.rect.x,
                row.rect.y,
                row.rect.x + row.rect.w - 1,
                row.rect.y + row.rect.h - 1,
                CONTEXT_SELECTION,
                gamma,
            );
        }
    }
    draw_3d_frame(surface, bounds, gamma);

    let icon_columns = resources.icons.width() / ICON_CELL;
    for row in &panel.layout.rows {
        let Some(entry) = panel.entries.get(row.index) else {
            continue;
        };
        if let ContextMenuIcon::Phase(phase) = entry.icon {
            let phase = u32::from(phase);
            let source_x = (phase % icon_columns) * ICON_CELL;
            let source_y = (phase / icon_columns) * ICON_CELL;
            ensure!(
                source_x + ICON_CELL <= resources.icons.width()
                    && source_y + ICON_CELL <= resources.icons.height(),
                "classic context-menu icon phase {phase} is outside GUIIcons.png"
            );
            draw_facet_stretch(
                surface,
                &resources.icons,
                (
                    source_x as f32,
                    source_y as f32,
                    ICON_CELL as f32,
                    ICON_CELL as f32,
                ),
                (
                    row.rect.x as f32,
                    row.rect.y as f32,
                    row.rect.h as f32,
                    row.rect.h as f32,
                ),
                gamma,
            );
        }
        resources.font.draw_with_gamma(
            surface,
            row.text_x,
            row.rect.y,
            &entry.text,
            CONTEXT_TEXT,
            TextAlign::Left,
            true,
            gamma,
        );
        if entry.has_submenu() {
            draw_facet_stretch(
                surface,
                &resources.submenu_arrow,
                (
                    0.0,
                    0.0,
                    resources.submenu_arrow.width() as f32,
                    resources.submenu_arrow.height() as f32,
                ),
                (
                    (row.rect.x + row.rect.w - resources.submenu_arrow.width() as i32) as f32,
                    (row.rect.y + (row.rect.h - resources.submenu_arrow.height() as i32) / 2)
                        as f32,
                    resources.submenu_arrow.width() as f32,
                    resources.submenu_arrow.height() as f32,
                ),
                gamma,
            );
        }
    }
    Ok(())
}

pub fn draw_classic_tooltip(
    surface: &mut Surface,
    font: &ClonkFont,
    pointer: GuiPoint,
    text: &str,
    gamma: Option<&GammaRamp>,
) {
    let max_width = TOOLTIP_MAX_WIDTH.min((surface.width() as i32).max(50));
    let broken = crate::message_dialog::break_message(font, text, max_width);
    let (text_width, text_height) = font.measure(&broken, true);
    let width = text_width + 6;
    let height = text_height + 4;
    let pointer_x = pointer.x as i32;
    let pointer_y = pointer.y as i32;
    let y = if pointer_y < height + 5 {
        (pointer_y + 5).min(surface.height() as i32 - height)
    } else {
        pointer_y - height - 5
    };
    let candidate_x = pointer_x - width / 2;
    let max_x = surface.width() as i32 - width;
    let x = if candidate_x < 0 {
        0
    } else if candidate_x > max_x {
        max_x
    } else {
        candidate_x
    };
    draw_engine_box(
        surface,
        x,
        y,
        x + width - 1,
        y + height - 2,
        TOOLTIP_BACKGROUND,
        gamma,
    );
    draw_engine_frame(
        surface,
        x,
        y,
        x + width - 1,
        y + height - 1,
        TOOLTIP_FRAME,
        gamma,
    );
    font.draw_with_gamma(
        surface,
        x + 3,
        y + 1,
        &broken,
        TOOLTIP_TEXT,
        TextAlign::Left,
        true,
        gamma,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_graphics::PixelFormat;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Action {
        One,
        Two,
        Child,
    }

    fn resources() -> ContextMenuResources {
        let font = ClonkFont::new(22);
        let icons = ImageData::new(240, 40, vec![255; 240 * 40 * 4]);
        let arrow = ImageData::new(8, 16, vec![255; 8 * 16 * 4]);
        ContextMenuResources::new(&font, &font, &icons, &arrow).expect("resources")
    }

    fn capturing_resources() -> ContextMenuResources {
        let fonts = crate::test_support::endeavour_font_set();
        let mut tooltip = fonts.text.clone();
        tooltip.set_role(Some(clonk_graphics::clonk_font::ClonkFontRole::GuiTooltip));
        let icons = ImageData::new(240, 40, vec![255; 240 * 40 * 4]);
        let arrow = ImageData::new(8, 16, vec![255; 8 * 16 * 4]);
        ContextMenuResources::new(&fonts.text, &tooltip, &icons, &arrow).expect("resources")
    }

    #[test]
    fn classic_tooltip_tracker_starts_inactive_and_uses_the_exact_delay() {
        let start = Instant::now();
        let mut tracker = ClassicTooltipTracker::new_at(start);
        assert_eq!(tracker.pointer_position(), None);
        assert!(!tracker.pointer_active());
        assert_eq!(tracker.pending_pointer(), None);
        assert_eq!(
            tracker.eligible_pointer_at(start + CLASSIC_TOOLTIP_DELAY),
            None
        );

        let first = GuiPoint::new(10.1, 20.9);
        assert!(tracker.note_pointer_move_at(first, start));
        assert!(tracker.pointer_active());
        assert_eq!(tracker.pending_pointer(), Some(first));
        assert_eq!(
            tracker.eligible_pointer_at(start + CLASSIC_TOOLTIP_DELAY - Duration::from_millis(1)),
            None
        );
        assert_eq!(
            tracker.eligible_pointer_at(start + CLASSIC_TOOLTIP_DELAY),
            Some(first),
            "the native 500ms boundary is inclusive"
        );

        let next_pixel = GuiPoint::new(11.0, 20.0);
        let moved_at = start + Duration::from_millis(750);
        assert!(tracker.note_pointer_move_at(next_pixel, moved_at));
        assert_eq!(
            tracker
                .eligible_pointer_at(moved_at + CLASSIC_TOOLTIP_DELAY - Duration::from_millis(1)),
            None
        );
        assert_eq!(
            tracker.eligible_pointer_at(moved_at + CLASSIC_TOOLTIP_DELAY),
            Some(next_pixel)
        );
    }

    #[test]
    fn classic_tooltip_tracker_ignores_subpixel_motion_and_keys_require_real_mouse_input() {
        let start = Instant::now();
        let mut tracker = ClassicTooltipTracker::new_at(start);
        let first = GuiPoint::new(10.1, 20.1);
        tracker.note_pointer_move_at(first, start);

        let subpixel = GuiPoint::new(10.9, 20.8);
        assert!(!tracker.note_pointer_move_at(subpixel, start + Duration::from_millis(400)));
        assert_eq!(
            tracker.eligible_pointer_at(start + CLASSIC_TOOLTIP_DELAY),
            Some(subpixel),
            "motion within one native integer pixel must not restart the timer"
        );

        tracker.note_non_pointer_input();
        assert!(!tracker.pointer_active());
        assert_eq!(tracker.pending_pointer(), None);
        assert_eq!(
            tracker.eligible_pointer_at(start + Duration::from_secs(2)),
            None
        );
        assert!(!tracker
            .note_pointer_move_at(GuiPoint::new(10.2, 20.2), start + Duration::from_secs(2)));
        assert!(
            !tracker.pointer_active(),
            "same-pixel motion is not new input"
        );

        let reactivated_at = start + Duration::from_secs(3);
        let changed = GuiPoint::new(9.9, 20.2);
        assert!(tracker.note_pointer_move_at(changed, reactivated_at));
        assert!(tracker.pointer_active());
        assert_eq!(
            tracker.eligible_pointer_at(
                reactivated_at + CLASSIC_TOOLTIP_DELAY - Duration::from_millis(1)
            ),
            None
        );
        assert_eq!(
            tracker.eligible_pointer_at(reactivated_at + CLASSIC_TOOLTIP_DELAY),
            Some(changed)
        );
    }

    #[test]
    fn classic_tooltip_tracker_buttons_wheel_and_leave_match_cmouse_activity() {
        let start = Instant::now();
        let mut tracker = ClassicTooltipTracker::new_at(start);
        let point = GuiPoint::new(5.0, 7.0);
        tracker.note_pointer_move_at(point, start);
        tracker.note_non_pointer_input();

        let button_at = start + Duration::from_secs(1);
        tracker.note_pointer_button_at(button_at);
        assert!(tracker.pointer_active());
        assert_eq!(
            tracker.eligible_pointer_at(button_at + CLASSIC_TOOLTIP_DELAY),
            Some(point)
        );

        tracker.note_non_pointer_input();
        let wheel_at = start + Duration::from_secs(2);
        tracker.note_pointer_wheel_at(wheel_at);
        assert_eq!(
            tracker
                .eligible_pointer_at(wheel_at + CLASSIC_TOOLTIP_DELAY - Duration::from_millis(1)),
            None
        );
        assert_eq!(
            tracker.eligible_pointer_at(wheel_at + CLASSIC_TOOLTIP_DELAY),
            Some(point)
        );

        tracker.pointer_left();
        assert_eq!(tracker.pointer_position(), None);
        assert!(!tracker.pointer_active());
        assert_eq!(tracker.pending_pointer(), None);
        assert_eq!(
            tracker.eligible_pointer_at(start + Duration::from_secs(10)),
            None
        );

        let returned_at = start + Duration::from_secs(11);
        assert!(tracker.note_pointer_move_at(point, returned_at));
        assert_eq!(
            tracker.eligible_pointer_at(returned_at - Duration::from_secs(1)),
            None
        );
        assert_eq!(
            tracker.eligible_pointer_at(returned_at + CLASSIC_TOOLTIP_DELAY),
            Some(point)
        );
    }

    fn screen() -> IntRect {
        IntRect::new(0, 0, 320, 200)
    }

    #[test]
    fn two_text_rows_match_cpp_geometry_and_start_unselected() {
        let entries = vec![
            ContextMenuEntry::new("Properties").with_action(Action::One),
            ContextMenuEntry::new("Delete").with_action(Action::Two),
        ];
        let (menu, outcome) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        assert_eq!(
            outcome.events,
            vec![ContextMenuEvent::Sound(ContextMenuSound::DoorOpen)]
        );
        let layout = menu.layout();
        assert_eq!(layout.panels.len(), 1);
        assert_eq!(layout.panels[0].bounds.y, 30);
        assert_eq!(layout.panels[0].bounds.h, 55);
        assert_eq!(layout.panels[0].rows[0].rect.h, 22);
        assert_eq!(layout.panels[0].rows[1].rect.y, 58);
        assert_eq!(layout.panels[0].selected, None);
    }

    #[test]
    fn root_placement_flips_instead_of_clamping() {
        let entries = vec![ContextMenuEntry::new("Delete").with_action(Action::One)];
        let (menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(319.0, 199.0), screen(), resources());
        let bounds = menu.layout().panels[0].bounds;
        assert_eq!(bounds.x + bounds.w, 319);
        assert_eq!(bounds.y + bounds.h, 199);
    }

    #[test]
    fn combo_root_honors_control_width_before_edge_flip() {
        let entries = vec![ContextMenuEntry::new("A").with_action(Action::One)];
        let (menu, _) = ClassicContextMenu::open_with_minimum_width(
            entries,
            GuiPoint::new(280.0, 30.0),
            screen(),
            resources(),
            180,
        );
        let bounds = menu.layout().panels[0].bounds;
        assert_eq!(bounds.w, 180);
        assert_eq!(bounds.x + bounds.w, 280);
        assert_eq!(bounds.y, 30);
        assert_eq!(menu.layout().panels[0].rows[0].rect.w, 170);
    }

    #[test]
    fn pointer_activation_closes_before_callback_and_outside_down_passes() {
        let entries = vec![ContextMenuEntry::new("Delete").with_action(Action::Two)];
        let (mut menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        let row = menu.layout().panels[0].rows[0].rect;
        let result = menu.handle_pointer_down(
            GuiPoint::new((row.x + 1) as f32, (row.y + 1) as f32),
            ContextMenuPointerButton::Left,
        );
        assert!(result.captured);
        assert_eq!(
            result.events[result.events.len() - 3..],
            [
                ContextMenuEvent::Closed,
                ContextMenuEvent::Sound(ContextMenuSound::Click),
                ContextMenuEvent::Activated(Action::Two),
            ]
        );

        let (mut menu, _) = ClassicContextMenu::open(
            vec![ContextMenuEntry::new("Delete").with_action(Action::Two)],
            GuiPoint::new(20.0, 30.0),
            screen(),
            resources(),
        );
        let result =
            menu.handle_pointer_down(GuiPoint::new(300.0, 180.0), ContextMenuPointerButton::Right);
        assert!(result.pass_through);
        assert_eq!(
            result.events,
            vec![
                ContextMenuEvent::Sound(ContextMenuSound::DoorClose),
                ContextMenuEvent::Closed,
            ]
        );
    }

    #[test]
    fn keyboard_wraps_and_escape_closes_only_the_deepest_level() {
        let child = vec![ContextMenuEntry::new("Child").with_action(Action::Child)];
        let entries = vec![
            ContextMenuEntry::new("Submenu").with_submenu(child),
            ContextMenuEntry::new("Two").with_action(Action::Two),
        ];
        let (mut menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        menu.handle_key(KeyCode::Up);
        assert_eq!(menu.layout().panels[0].selected, Some(1));
        menu.handle_key(KeyCode::Down);
        assert_eq!(menu.layout().panels[0].selected, Some(0));
        menu.handle_key(KeyCode::Right);
        assert_eq!(menu.layout().panels.len(), 2);
        let result = menu.handle_key(KeyCode::Escape);
        assert!(result.captured);
        assert!(menu.is_open());
        assert_eq!(menu.layout().panels.len(), 1);
        let result = menu.handle_key(KeyCode::Escape);
        assert!(!menu.is_open());
        assert!(result.events.contains(&ContextMenuEvent::Closed));
    }

    #[test]
    fn hotkeys_and_gamepad_follow_context_bindings() {
        let entries = vec![
            ContextMenuEntry::new("&One").with_action(Action::One),
            ContextMenuEntry::new("Two").with_action(Action::Two),
        ];
        let (mut menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        let result = menu.handle_hotkey('o');
        assert!(result
            .events
            .contains(&ContextMenuEvent::Activated(Action::One)));

        let (mut menu, _) = ClassicContextMenu::open(
            vec![ContextMenuEntry::new("Two").with_action(Action::Two)],
            GuiPoint::new(20.0, 30.0),
            screen(),
            resources(),
        );
        menu.handle_gamepad_direction(ContextMenuDirection::Down);
        let result = menu.handle_gamepad_low();
        assert!(result
            .events
            .contains(&ContextMenuEvent::Activated(Action::Two)));
    }

    #[test]
    fn deferred_submenu_requests_fill_at_open_and_refills_on_reopen() {
        let entries = vec![
            ContextMenuEntry::new("Take over").with_deferred_submenu(Action::One),
            ContextMenuEntry::new("Other").with_action(Action::Two),
        ];
        let (mut menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        let first = menu.layout().panels[0].rows[0].rect;
        let outcome =
            menu.handle_pointer_move(GuiPoint::new((first.x + 1) as f32, (first.y + 1) as f32));
        assert_eq!(
            outcome.events,
            vec![
                ContextMenuEvent::Sound(ContextMenuSound::Command),
                ContextMenuEvent::SubmenuRequested(Action::One),
            ],
            "selecting a deferred entry runs the C4GUI OnSubcontext request"
        );
        assert_eq!(
            menu.layout().panels.len(),
            1,
            "the child panel waits for the host answer"
        );

        let outcome = menu.fill_requested_submenu(vec![
            ContextMenuEntry::new("Using A").with_action(Action::Child)
        ]);
        assert_eq!(
            outcome.events,
            vec![ContextMenuEvent::Sound(ContextMenuSound::DoorOpen)]
        );
        assert_eq!(menu.layout().panels.len(), 2);
        assert_eq!(menu.layout().panels[1].rows.len(), 1);

        let outcome =
            menu.handle_pointer_move(GuiPoint::new((first.x + 2) as f32, (first.y + 2) as f32));
        assert!(
            outcome.events.is_empty(),
            "hovering the already-open parent neither re-requests nor re-opens"
        );

        let second = menu.layout().panels[0].rows[1].rect;
        menu.handle_pointer_move(GuiPoint::new((second.x + 1) as f32, (second.y + 1) as f32));
        let outcome =
            menu.handle_pointer_move(GuiPoint::new((first.x + 1) as f32, (first.y + 1) as f32));
        assert_eq!(
            outcome.events,
            vec![
                ContextMenuEvent::Sound(ContextMenuSound::Command),
                ContextMenuEvent::SubmenuRequested(Action::One),
            ],
            "re-selecting the parent re-runs the fill callback like C4GUI"
        );
        let outcome = menu.fill_requested_submenu(Vec::new());
        assert_eq!(
            outcome.events,
            vec![ContextMenuEvent::Sound(ContextMenuSound::DoorOpen)],
            "an empty live answer still opens the minimum-size C4GUI box"
        );
        assert_eq!(menu.layout().panels.len(), 2);
        assert!(menu.layout().panels[1].rows.is_empty());

        menu.handle_pointer_move(GuiPoint::new((second.x + 1) as f32, (second.y + 1) as f32));
        let outcome = menu.fill_requested_submenu(vec![ContextMenuEntry::new("Stale")]);
        assert!(
            outcome.events.is_empty(),
            "an answer after the selection moved away is dropped"
        );
        assert_eq!(menu.layout().panels.len(), 1);
    }

    #[test]
    fn recursive_pointer_dispatch_reaches_grandchildren() {
        let entries =
            vec![ContextMenuEntry::new("Root")
                .with_submenu(vec![ContextMenuEntry::new("Child").with_submenu(vec![
                    ContextMenuEntry::new("Grandchild").with_action(Action::Child),
                ])])];
        let (mut menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        for depth in 0..2 {
            let row = menu.layout().panels[depth].rows[0].rect;
            menu.handle_pointer_move(GuiPoint::new((row.x + 1) as f32, (row.y + 1) as f32));
        }
        assert_eq!(menu.layout().panels.len(), 3);
        let row = menu.layout().panels[2].rows[0].rect;
        let outcome = menu.handle_pointer_down(
            GuiPoint::new((row.x + 1) as f32, (row.y + 1) as f32),
            ContextMenuPointerButton::Left,
        );
        assert!(outcome
            .events
            .contains(&ContextMenuEvent::Activated(Action::Child)));
    }

    #[test]
    fn changing_an_ancestor_selection_commands_before_closing_its_child() {
        let entries = vec![
            ContextMenuEntry::new("Submenu").with_submenu(vec![
                ContextMenuEntry::new("Child").with_action(Action::Child)
            ]),
            ContextMenuEntry::new("Other").with_action(Action::Two),
        ];
        let (mut menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        let first = menu.layout().panels[0].rows[0].rect;
        menu.handle_pointer_move(GuiPoint::new((first.x + 1) as f32, (first.y + 1) as f32));
        let second = menu.layout().panels[0].rows[1].rect;
        let outcome =
            menu.handle_pointer_move(GuiPoint::new((second.x + 1) as f32, (second.y + 1) as f32));
        assert_eq!(
            outcome.events,
            vec![
                ContextMenuEvent::Sound(ContextMenuSound::Command),
                ContextMenuEvent::Sound(ContextMenuSound::DoorClose),
            ]
        );
        assert_eq!(menu.layout().panels.len(), 1);
    }

    #[test]
    fn pointer_up_refreshes_selection_and_frame_margin_clears_a_leaf() {
        let entries = vec![
            ContextMenuEntry::new("One").with_action(Action::One),
            ContextMenuEntry::new("Two").with_action(Action::Two),
        ];
        let (mut menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        let second = menu.layout().panels[0].rows[1].rect;
        let outcome = menu.handle_pointer_up(
            GuiPoint::new((second.x + 1) as f32, (second.y + 1) as f32),
            ContextMenuPointerButton::Left,
        );
        assert_eq!(menu.layout().panels[0].selected, Some(1));
        assert_eq!(
            outcome.events,
            vec![ContextMenuEvent::Sound(ContextMenuSound::Command)]
        );

        let bounds = menu.layout().panels[0].bounds;
        let outcome =
            menu.handle_pointer_move(GuiPoint::new((bounds.x + 1) as f32, (bounds.y + 1) as f32));
        assert_eq!(menu.layout().panels[0].selected, None);
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn tooltip_tracks_the_actual_panel_and_keyboard_hides_mouse_input() {
        let entries = vec![ContextMenuEntry::new("Parent")
            .with_tooltip("Parent tip")
            .with_submenu(vec![ContextMenuEntry::new("Child")
                .with_tooltip("Child tip")
                .with_action(Action::Child)])];
        let (mut menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        let parent = menu.layout().panels[0].rows[0].rect;
        menu.handle_pointer_move(GuiPoint::new((parent.x + 1) as f32, (parent.y + 1) as f32));
        menu.last_pointer_activity = Instant::now() - Duration::from_secs(1);
        assert_eq!(menu.hovered_tooltip(), Some("Parent tip"));

        let child = menu.layout().panels[1].rows[0].rect;
        menu.handle_pointer_move(GuiPoint::new((child.x + 1) as f32, (child.y + 1) as f32));
        menu.last_pointer_activity = Instant::now() - Duration::from_secs(1);
        assert_eq!(menu.hovered_tooltip(), Some("Child tip"));

        menu.handle_pointer_move(GuiPoint::new((parent.x + 1) as f32, (parent.y + 1) as f32));
        menu.last_pointer_activity = Instant::now() - Duration::from_secs(1);
        assert_eq!(menu.hovered_tooltip(), Some("Parent tip"));
        menu.handle_gamepad_direction(ContextMenuDirection::Down);
        assert_eq!(menu.hovered_tooltip(), None);
    }

    #[test]
    fn context_tooltip_draw_and_layer_signal_share_one_timestamp() {
        let entries = vec![ContextMenuEntry::new("Entry").with_tooltip("Delayed tip")];
        let (mut menu, _) = ClassicContextMenu::<()>::open(
            entries,
            GuiPoint::new(20.0, 30.0),
            screen(),
            capturing_resources(),
        );
        let row = menu.layout().panels[0].rows[0].rect;
        menu.handle_pointer_move(GuiPoint::new((row.x + 1) as f32, (row.y + 1) as f32));
        let hovered_at = menu.last_pointer_activity;
        let mut surface = Surface::new(320, 200, PixelFormat::Rgba8888);

        surface.begin_clonk_text_capture();
        assert!(!menu.render_tooltip_at(
            &mut surface,
            None,
            hovered_at + TOOLTIP_DELAY - Duration::from_millis(1),
        ));
        assert!(surface.take_clonk_text_capture().is_empty());

        surface.begin_clonk_text_capture();
        assert!(menu.render_tooltip_at(&mut surface, None, hovered_at + TOOLTIP_DELAY,));
        let commands = surface.take_clonk_text_capture();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].text, "Delayed tip");
    }

    #[test]
    fn staged_panels_and_tooltip_keep_distinct_capture_and_z_order() {
        let entries = vec![ContextMenuEntry::new("Parent")
            .with_tooltip("Parent tip")
            .with_submenu(vec![ContextMenuEntry::new("Child")
                .with_tooltip("Child tip")
                .with_action(Action::Child)])];
        let (mut menu, _) = ClassicContextMenu::open(
            entries,
            GuiPoint::new(20.0, 30.0),
            screen(),
            capturing_resources(),
        );
        let parent_row = menu.layout().panels[0].rows[0].rect;
        menu.handle_pointer_move(GuiPoint::new(
            (parent_row.x + 1) as f32,
            (parent_row.y + 1) as f32,
        ));
        let child_row = menu.layout().panels[1].rows[0].rect;
        menu.handle_pointer_move(GuiPoint::new(
            (child_row.x + 1) as f32,
            (child_row.y + 1) as f32,
        ));
        menu.last_pointer_activity = Instant::now() - Duration::from_secs(1);
        assert_eq!(menu.panel_count(), 2);

        let mut surface = Surface::new(320, 200, PixelFormat::Rgba8888);
        surface.begin_clonk_text_capture();
        menu.render_panel(&mut surface, 0, None)
            .expect("root panel");
        let parent_commands = surface.take_clonk_text_capture();
        assert_eq!(parent_commands.len(), 1);
        assert_eq!(parent_commands[0].text, "Parent");

        let layout = menu.layout();
        let parent = layout.panels[0].bounds;
        let child = layout.panels[1].bounds;
        let overlap_left = parent.x.max(child.x);
        let overlap_top = parent.y.max(child.y);
        let overlap_right = (parent.x + parent.w).min(child.x + child.w);
        let overlap_bottom = (parent.y + parent.h).min(child.y + child.h);
        assert!(overlap_left < overlap_right && overlap_top < overlap_bottom);
        let parent_overlap = (overlap_top..overlap_bottom)
            .flat_map(|y| (overlap_left..overlap_right).map(move |x| (x, y)))
            .map(|(x, y)| surface.get_pixel(x as u32, y as u32).unwrap_or_default())
            .collect::<Vec<_>>();

        surface.begin_clonk_text_capture();
        menu.render_panel(&mut surface, 1, None)
            .expect("child panel");
        let child_commands = surface.take_clonk_text_capture();
        assert_eq!(child_commands.len(), 1);
        assert_eq!(child_commands[0].text, "Child");
        let child_overlap = (overlap_top..overlap_bottom)
            .flat_map(|y| (overlap_left..overlap_right).map(move |x| (x, y)))
            .map(|(x, y)| surface.get_pixel(x as u32, y as u32).unwrap_or_default())
            .collect::<Vec<_>>();
        assert_ne!(
            parent_overlap, child_overlap,
            "child chrome is a later layer"
        );

        surface.begin_clonk_text_capture();
        menu.render_tooltip(&mut surface, None);
        let tooltip_commands = surface.take_clonk_text_capture();
        assert_eq!(tooltip_commands.len(), 1);
        assert_eq!(tooltip_commands[0].text, "Child tip");
        assert_eq!(
            tooltip_commands[0].role,
            clonk_graphics::clonk_font::ClonkFontRole::GuiTooltip
        );

        let mut combined = Surface::new(320, 200, PixelFormat::Rgba8888);
        let mut staged = Surface::new(320, 200, PixelFormat::Rgba8888);
        menu.render(&mut combined, None).expect("combined render");
        for index in 0..menu.panel_count() {
            menu.render_panel(&mut staged, index, None)
                .expect("staged panel");
        }
        menu.render_tooltip(&mut staged, None);
        assert_eq!(combined.pixels(), staged.pixels());
    }

    #[test]
    fn renderer_uses_exact_assets_and_rejects_bad_icon_phases() {
        let entries = vec![ContextMenuEntry::new("Icon")
            .with_icon(ContextMenuIcon::Phase(99))
            .with_action(Action::One)];
        let (menu, _) =
            ClassicContextMenu::open(entries, GuiPoint::new(20.0, 30.0), screen(), resources());
        let mut surface = Surface::new(320, 200, PixelFormat::Rgba8888);
        assert!(menu.render(&mut surface, None).is_err());
    }
}
