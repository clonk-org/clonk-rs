use super::*;
use crate::classic_gui::IntRect;
use crate::clonk_fonts::ClonkFontSet;
use crate::rename_edit::{RenameEdit, RenameEditCursorOperation};
use crate::startup_options_dlg::{BookFonts, BookLayout, OptionsBook};
use crate::{GuiPoint, KeyCode};

mod choices;
mod microphone;
mod render;
const ROW_HEIGHT: i32 = 36;
/// The height of a control set's picture from the classic control sheets,
/// which is 80x36.
const SET_PICTURE_HEIGHT: i32 = 36;
/// What a control set's page gains by keeping one line of text in its
/// footer: the grid needs the height more than a description of it.
const GRID_FOOTER_LIFT: i32 = 46;
/// The tallest row of the binding grid; taller pages leave the room below.
const GRID_ROW_MAX: i32 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsFocus {
    #[default]
    Search,
    Category(SettingsCategory),
    AudioPage(AudioPage),
    ControlsPage(ControlsPage),
    /// A control set's picture, by the set's index on its device.
    ControlSet(usize),
    Row(usize),
    Modified,
    Advanced,
    ResetCategory,
    TestMicrophone,
    RecordMicrophone,
    CloseMicrophoneTest,
    RefreshDevices,
    Close,
}

/// How strongly a list row is marked: the keyboard focus, the row the footer
/// actions and description apply to, or the row under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowEmphasis {
    Hovered,
    Selected,
    Focused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    Change(usize, AdvancedConfigValue),
    Close,
    CaptureBinding(usize),
    TestMicrophone,
    CancelMicrophoneTest,
    RefreshDevices,
    ConfirmDisplay(bool),
}

pub struct SettingsViewState {
    pub focus: SettingsFocus,
    pub selected: Option<usize>,
    pub scroll: usize,
    pub message: String,
    pub context: String,
    pub width: i32,
    pub height: i32,
    pub microphone_status: String,
    pub microphone_level: f32,
    pub microphone_test_open: bool,
    pub microphone_testing: bool,
    pub audio_device_status: String,
    pub display_confirmation: Option<u64>,
    pub reset_confirmation: bool,
    /// The binding waiting for its new key or button.
    pub capturing: Option<usize>,
    /// The binding that already holds the key or button just refused.
    pub conflict: Option<usize>,
    pub(crate) search_edit: RenameEdit<()>,
    pub(crate) edit: Option<(usize, RenameEdit<()>)>,
    pub(crate) pressed: Option<SettingsFocus>,
    pub(crate) choice: Option<choices::ChoicePicker>,
    pub(crate) dragging: Option<usize>,
    pub(crate) scroll_drag: Option<i32>,
    pub(crate) hover: Option<SettingsFocus>,
    /// The popup window the book opens in over another screen; `None` when
    /// it covers the screen.
    pub(crate) window: Option<IntRect>,
    layout: Option<SettingsLayout>,
}

impl Default for SettingsViewState {
    fn default() -> Self {
        Self {
            focus: SettingsFocus::Search,
            selected: None,
            scroll: 0,
            message: String::new(),
            context: String::new(),
            width: 800,
            height: 600,
            microphone_status: String::new(),
            microphone_level: 0.0,
            microphone_test_open: false,
            microphone_testing: false,
            audio_device_status: String::new(),
            display_confirmation: None,
            reset_confirmation: false,
            capturing: None,
            conflict: None,
            search_edit: RenameEdit::new("", None),
            edit: None,
            pressed: None,
            choice: None,
            dragging: None,
            scroll_drag: None,
            hover: None,
            window: None,
            layout: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsLayout {
    pub panel: IntRect,
    pub search: IntRect,
    pub list: IntRect,
    pub footer: IntRect,
    pub tabs: [IntRect; SettingsCategory::ALL.len()],
    pub back: IntRect,
}

impl SettingsLayout {
    pub fn new(width: i32, height: i32) -> Self {
        let w = (width - 16).clamp(1, 1120);
        let h = (height - 16).clamp(1, 760);
        let panel = IntRect::new((width - w) / 2, (height - h) / 2, w, h);
        Self {
            search: IntRect::new(panel.x + 172, panel.y + 52, w - 184, 30),
            list: IntRect::new(panel.x + 172, panel.y + 118, w - 184, (h - 254).max(1)),
            footer: IntRect::new(panel.x + 12, panel.y + h - 132, w - 24, 120),
            tabs: std::array::from_fn(|i| {
                IntRect::new(panel.x + 12, panel.y + 52 + i as i32 * 36, 148, 32)
            }),
            back: IntRect::new(panel.x + w - 142, panel.y + h - 44, 130, 30),
            panel,
        }
    }

    /// The page laid out on the book's sheet. A `popup` book closes from a
    /// mark inked on its paper, as it has no Back button beneath it.
    fn from_book(book: &BookLayout, popup: bool) -> Self {
        let sheet = book.sheet;
        let margin = if sheet.w < 500 { 8 } else { 32 };
        let x = (sheet.x + margin).max(book.tab_clips[0].0 + 128);
        let w = sheet.x + sheet.w - margin - x;
        let footer = IntRect::new(x, sheet.y + sheet.h - 102, w, 102);
        Self {
            panel: book.tabular,
            search: IntRect::new(x + 60, sheet.y + 2, w - 60, 26),
            list: IntRect::new(
                x,
                sheet.y + 62,
                w,
                (footer.y - sheet.y - 66).max(ROW_HEIGHT),
            ),
            footer,
            tabs: std::array::from_fn(|i| {
                IntRect::new(
                    book.tab_clips[i].0,
                    book.tab_clips[i].1,
                    85,
                    book.tab_height - 8,
                )
            }),
            back: if popup {
                // Clear of the paper's torn top edge.
                IntRect::new(x + w - 20, sheet.y - 18, 18, 18)
            } else {
                book.back_button
            },
        }
    }
}

impl SettingsController {
    /// Lays the page out in the options book, covering the screen or, as a
    /// `popup` over another screen, in a centred window.
    pub fn resize_book(
        &mut self,
        width: i32,
        height: i32,
        gui: &ClonkFontSet,
        book: &BookFonts,
        popup: bool,
    ) {
        self.resize(width, height);
        self.view.window = popup.then(|| OptionsBook::window(width, height));
        let layout = SettingsLayout::from_book(&self.book_layout(gui, book), popup);
        if self.view.layout != Some(layout) {
            self.view.layout = Some(layout);
            self.ensure_visible();
        }
    }

    pub(crate) fn book_layout(&self, gui: &ClonkFontSet, book: &BookFonts) -> BookLayout {
        match self.view.window {
            Some(frame) => {
                OptionsBook::layout(frame.w, frame.h, gui, book).offset(frame.x, frame.y)
            }
            None => OptionsBook::layout(self.view.width, self.view.height, gui, book),
        }
    }

    pub fn layout(&self) -> SettingsLayout {
        let mut layout = self
            .view
            .layout
            .unwrap_or_else(|| SettingsLayout::new(self.view.width, self.view.height));
        if self.category == SettingsCategory::Audio {
            layout.footer.y += 16;
            layout.footer.h -= 16;
            layout.list.h += 16;
        }
        if !self.has_page_row() {
            let row = layout.list.y - (layout.search.y + layout.search.h) - 6;
            layout.list.y -= row;
            layout.list.h += row;
        }
        if self.control_set_page().is_some() {
            layout.footer.y += GRID_FOOTER_LIFT;
            layout.footer.h -= GRID_FOOTER_LIFT;
            layout.list.h += GRID_FOOTER_LIFT;
            // The pictures shrink where the page is short.
            let row = if layout.list.h >= 4 * 44 + SET_PICTURE_HEIGHT + 10 {
                SET_PICTURE_HEIGHT + 10
            } else {
                36
            };
            layout.list.y += row;
            layout.list.h -= row;
        }
        layout
    }

    /// The control set the page shows, unless a search replaced the page.
    pub(super) fn control_set_page(&self) -> Option<ControlSet> {
        self.query
            .trim()
            .is_empty()
            .then(|| self.current_control_set())
            .flatten()
    }

    /// The shown control set's name, with the players who use it.
    pub fn control_set_caption(&self) -> Option<String> {
        self.control_set_page()
            .map(|set| match self.set_user_names(set) {
                Some(names) => format!("{} · used by {names}", set.label()),
                None => set.label(),
            })
    }

    /// The names of the players who use `set`, in join order.
    pub(crate) fn set_user_names(&self, set: ControlSet) -> Option<String> {
        let users: Vec<_> = self
            .set_users
            .iter()
            .filter(|(used, _)| *used == set)
            .map(|(_, name)| name.as_str())
            .collect();
        (!users.is_empty()).then(|| users.join(", "))
    }

    /// The footer's detail line: the last change's message, or where and
    /// when the selected setting applies — on a control set's page, the set
    /// and the players who use it.
    pub(crate) fn footer_detail(&self) -> String {
        if !self.view.message.is_empty() {
            return self.view.message.clone();
        }
        self.view
            .selected
            .and_then(|index| self.settings.get(index))
            .map(|setting| match self.control_set_caption() {
                Some(caption) => detail_text_in(setting, &caption),
                None => detail_text(setting),
            })
            .unwrap_or_default()
    }

    /// Focus on the picture of the control set the page shows.
    fn control_set_focus(&self) -> Option<SettingsFocus> {
        self.control_set_page()
            .map(|set| SettingsFocus::ControlSet(set.index))
    }

    /// Whether the page has a row of tabs under the search field.
    fn has_page_row(&self) -> bool {
        self.current_page_tab().is_some()
    }

    /// The current category's tabs, in order.
    fn page_tabs(&self) -> Vec<SettingsFocus> {
        match self.category {
            SettingsCategory::Audio => AudioPage::ALL.map(SettingsFocus::AudioPage).to_vec(),
            SettingsCategory::Controls => {
                ControlsPage::ALL.map(SettingsFocus::ControlsPage).to_vec()
            }
            _ => Vec::new(),
        }
    }

    /// The tab of the page on show.
    fn current_page_tab(&self) -> Option<SettingsFocus> {
        match self.category {
            SettingsCategory::Audio => Some(SettingsFocus::AudioPage(self.audio_page)),
            SettingsCategory::Controls => Some(SettingsFocus::ControlsPage(self.controls_page)),
            _ => None,
        }
    }

    /// The row the page's tabs stand in, under the search field.
    fn page_tab_row(layout: &SettingsLayout) -> IntRect {
        IntRect::new(layout.list.x, layout.search.y + 29, layout.list.w, 28)
    }

    /// The pictures of the page's control sets, side by side between the
    /// tabs and the bindings, in the classic sheets' 80x36 proportions.
    pub(crate) fn control_set_pictures(&self, layout: &SettingsLayout) -> Vec<(usize, IntRect)> {
        if self.control_set_page().is_none() {
            return Vec::new();
        }
        let tabs = Self::page_tab_row(layout);
        let top = tabs.y + tabs.h + 4;
        let height = (layout.list.y - 6 - top).min(SET_PICTURE_HEIGHT);
        let width = height * 80 / 36;
        self.control_sets()
            .into_iter()
            .enumerate()
            .map(|(position, set)| {
                (
                    set.index,
                    IntRect::new(
                        layout.list.x + position as i32 * (width + 10),
                        top,
                        width,
                        height,
                    ),
                )
            })
            .collect()
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        if self.view.width == width && self.view.height == height {
            return;
        }
        self.view.width = width;
        self.view.height = height;
        self.view.layout = None;
        self.ensure_visible();
    }

    pub fn select_category(&mut self, category: SettingsCategory) {
        self.view.microphone_test_open = false;
        self.view.choice = None;
        self.cancel_interaction();
        self.category = category;
        self.view.message.clear();
        self.query.clear();
        self.view.search_edit.set_text("");
        self.view.edit = None;
        self.view.scroll = 0;
        self.view.selected = self.visible_indices().first().copied();
        self.view.focus = SettingsFocus::Category(category);
    }

    pub fn select_audio_page(&mut self, page: AudioPage) {
        self.audio_page = page;
        self.select_category(SettingsCategory::Audio);
        self.view.focus = SettingsFocus::AudioPage(page);
    }

    /// Waits for a new key or button for the binding at `index`.
    pub fn begin_capture(&mut self, index: usize) {
        self.view.capturing = Some(index);
        self.view.conflict = None;
    }

    /// Stops waiting for a key or button.
    pub fn end_capture(&mut self) {
        self.view.capturing = None;
        self.view.conflict = None;
    }

    pub fn select_controls_page(&mut self, page: ControlsPage) {
        self.controls_page = page;
        self.select_category(SettingsCategory::Controls);
        self.view.focus = SettingsFocus::ControlsPage(page);
    }

    pub fn voice_page_selected(&self) -> bool {
        self.category == SettingsCategory::Audio && self.audio_page == AudioPage::Voice
    }

    pub fn editing(&self) -> bool {
        !self.view.microphone_test_open
            && (self.view.edit.is_some() || self.view.focus == SettingsFocus::Search)
    }

    pub fn accepts_text(&self) -> bool {
        self.editing() || self.view.choice.is_some()
    }

    pub fn focus_search(&mut self) {
        if self.view.microphone_test_open {
            return;
        }
        self.view.edit = None;
        self.view.choice = None;
        self.cancel_interaction();
        self.view.focus = SettingsFocus::Search;
        self.view.search_edit.set_text(self.query.clone());
        self.view.search_edit.select_all();
    }

    pub fn text(&mut self, text: &str) {
        if self.view.microphone_test_open {
            return;
        }
        if self.view.choice.is_some() {
            self.choice_type(text);
            return;
        }
        if let Some((_, edit)) = self.view.edit.as_mut() {
            edit.insert_text(text);
        } else if self.view.focus == SettingsFocus::Search {
            self.view.search_edit.insert_text(text);
            self.query = self.view.search_edit.text().to_owned();
            self.view.scroll = 0;
            self.view.selected = self.visible_indices().first().copied();
        }
    }

    /// A text-editing command for the search field or the value being
    /// typed; on a focused row, Delete and Backspace reset a changed setting.
    pub fn edit_command(
        &mut self,
        command: &str,
        control: bool,
        shift: bool,
    ) -> Vec<SettingsAction> {
        if self.view.microphone_test_open {
            return Vec::new();
        }
        if let (None, SettingsFocus::Row(index)) = (&self.view.edit, self.view.focus) {
            return if matches!(command, "delete" | "backspace") && self.resettable(index) {
                vec![SettingsAction::Change(
                    index,
                    self.settings[index].default.clone(),
                )]
            } else {
                Vec::new()
            };
        }
        let edit = if let Some((_, edit)) = self.view.edit.as_mut() {
            Some(edit)
        } else if self.view.focus == SettingsFocus::Search {
            Some(&mut self.view.search_edit)
        } else {
            None
        };
        if let Some(edit) = edit {
            match command {
                "backspace" => {
                    edit.backspace(control, shift);
                }
                "delete" => {
                    edit.delete(control, shift);
                }
                "all" => edit.select_all(),
                "left" => edit.move_cursor(RenameEditCursorOperation::Left, control, shift),
                "right" => edit.move_cursor(RenameEditCursorOperation::Right, control, shift),
                "home" => edit.move_cursor(RenameEditCursorOperation::Home, control, shift),
                "end" => edit.move_cursor(RenameEditCursorOperation::End, control, shift),
                _ => {}
            }
        }
        if self.view.edit.is_none() && self.view.focus == SettingsFocus::Search {
            self.query = self.view.search_edit.text().to_owned();
            self.view.scroll = 0;
            self.view.selected = self.visible_indices().first().copied();
        }
        Vec::new()
    }

    pub fn key(&mut self, key: KeyCode, shift: bool, control: bool) -> Vec<SettingsAction> {
        if self.view.microphone_test_open {
            return self.microphone_key(key, shift, control);
        }
        if self.view.choice.is_some() {
            return self.choice_key(key);
        }
        if self.view.display_confirmation.is_some() {
            return match key {
                KeyCode::Enter => vec![SettingsAction::ConfirmDisplay(true)],
                KeyCode::Escape => vec![SettingsAction::ConfirmDisplay(false)],
                _ => Vec::new(),
            };
        }
        if self.view.reset_confirmation {
            match key {
                KeyCode::Escape => self.view.reset_confirmation = false,
                KeyCode::Enter => {
                    self.view.reset_confirmation = false;
                    return self
                        .page_reset_candidates()
                        .into_iter()
                        .map(|index| {
                            SettingsAction::Change(index, self.settings[index].default.clone())
                        })
                        .collect();
                }
                _ => {}
            }
            return Vec::new();
        }
        if let Some((index, edit)) = self.view.edit.as_ref() {
            match key {
                KeyCode::Escape => self.view.edit = None,
                KeyCode::Enter | KeyCode::Tab => match self.parse_value(*index, edit.text()) {
                    Ok(value) => {
                        let index = *index;
                        self.view.edit = None;
                        let mut actions = vec![SettingsAction::Change(index, value)];
                        if key == KeyCode::Tab {
                            actions.extend(self.key(key, shift, control));
                        }
                        return actions;
                    }
                    Err(error) => self.view.message = error,
                },
                KeyCode::Left => {
                    self.edit_command("left", control, shift);
                }
                KeyCode::Right => {
                    self.edit_command("right", control, shift);
                }
                KeyCode::Home => {
                    self.edit_command("home", control, shift);
                }
                KeyCode::End => {
                    self.edit_command("end", control, shift);
                }
                _ => {}
            }
            return Vec::new();
        }
        match key {
            KeyCode::Escape => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.view.search_edit.set_text("");
                    self.view.scroll = 0;
                } else {
                    return vec![SettingsAction::Close];
                }
            }
            KeyCode::Tab if control => {
                self.select_category(self.category.step(if shift { -1 } else { 1 }));
            }
            KeyCode::Tab => {
                let order = self.focus_order();
                let index = order
                    .iter()
                    .position(|f| *f == self.view.focus)
                    .unwrap_or(0);
                self.set_focus(
                    order[(index + if shift { order.len() - 1 } else { 1 }) % order.len()],
                );
            }
            KeyCode::Down | KeyCode::Up | KeyCode::PageDown | KeyCode::PageUp => {
                let backwards = matches!(key, KeyCode::Up | KeyCode::PageUp);
                let on_tab = Some(self.view.focus) == self.current_page_tab();
                if on_tab || matches!(self.view.focus, SettingsFocus::ControlSet(_)) {
                    let above = if on_tab {
                        None
                    } else {
                        self.current_page_tab()
                    };
                    let below = on_tab
                        .then(|| self.control_set_focus())
                        .flatten()
                        .or_else(|| {
                            self.visible_indices()
                                .first()
                                .copied()
                                .map(SettingsFocus::Row)
                        });
                    if backwards {
                        self.set_focus(above.unwrap_or(SettingsFocus::Search));
                    } else if let Some(focus) = below {
                        self.set_focus(focus);
                    }
                    return Vec::new();
                }
                if let (SettingsFocus::Row(index), Some(_)) =
                    (self.view.focus, self.control_set_page())
                {
                    let next = match self.grid_neighbour(index, (0, if backwards { -1 } else { 1 }))
                    {
                        Some(target) => SettingsFocus::Row(target),
                        None if backwards => {
                            self.control_set_focus().unwrap_or(SettingsFocus::Search)
                        }
                        None => self.page_actions_start(),
                    };
                    self.set_focus(next);
                    return Vec::new();
                }
                if matches!(
                    self.view.focus,
                    SettingsFocus::ResetCategory
                        | SettingsFocus::TestMicrophone
                        | SettingsFocus::RefreshDevices
                        | SettingsFocus::Close
                        | SettingsFocus::Modified
                        | SettingsFocus::Advanced
                ) {
                    let order = self.focus_order();
                    let index = order
                        .iter()
                        .position(|focus| *focus == self.view.focus)
                        .unwrap_or(0);
                    self.set_focus(
                        order[(index + if backwards { order.len() - 1 } else { 1 }) % order.len()],
                    );
                    return Vec::new();
                }
                if let SettingsFocus::Category(category) = self.view.focus {
                    self.select_category(category.step(if backwards { -1 } else { 1 }));
                } else {
                    let visible = self.visible_indices();
                    if !visible.is_empty() {
                        let old = visible
                            .iter()
                            .position(|i| self.view.focus == SettingsFocus::Row(*i));
                        if key == KeyCode::Up && old == Some(0) {
                            self.set_focus(
                                self.control_set_focus()
                                    .or_else(|| self.current_page_tab())
                                    .unwrap_or(SettingsFocus::Category(self.category)),
                            );
                            return Vec::new();
                        }
                        if key == KeyCode::Down && old == Some(visible.len() - 1) {
                            self.set_focus(self.page_actions_start());
                            return Vec::new();
                        }
                        let step = if matches!(key, KeyCode::PageUp | KeyCode::PageDown) {
                            self.page_size()
                        } else {
                            1
                        };
                        let position = old
                            .map(|p| {
                                if backwards {
                                    p.saturating_sub(step)
                                } else {
                                    (p + step).min(visible.len() - 1)
                                }
                            })
                            .unwrap_or(0);
                        self.set_focus(SettingsFocus::Row(visible[position]));
                    }
                }
            }
            KeyCode::Left | KeyCode::Right if self.view.focus == SettingsFocus::Search => {
                self.edit_command(
                    if key == KeyCode::Left {
                        "left"
                    } else {
                        "right"
                    },
                    control,
                    shift,
                );
            }
            KeyCode::Left | KeyCode::Right => {
                let tabs = self.page_tabs();
                if let Some(position) = tabs.iter().position(|tab| *tab == self.view.focus) {
                    let step = if key == KeyCode::Left {
                        tabs.len() - 1
                    } else {
                        1
                    };
                    return self.activate(tabs[(position + step) % tabs.len()]);
                }
                if matches!(self.view.focus, SettingsFocus::ControlSet(_)) {
                    self.cycle_control_set(if key == KeyCode::Left { -1 } else { 1 });
                    self.view.focus = self.control_set_focus().unwrap_or(self.view.focus);
                    return Vec::new();
                }
                if let (SettingsFocus::Row(index), Some(_)) =
                    (self.view.focus, self.control_set_page())
                {
                    let step = if key == KeyCode::Left { -1 } else { 1 };
                    if let Some(target) = self.grid_neighbour(index, (step, 0)) {
                        self.set_focus(SettingsFocus::Row(target));
                    }
                    return Vec::new();
                }
                if let SettingsFocus::Row(index) = self.view.focus {
                    return self.adjust(index, if key == KeyCode::Left { -1 } else { 1 });
                }
                if let SettingsFocus::Category(_) = self.view.focus {
                    if let Some(tab) = self.current_page_tab() {
                        self.set_focus(tab);
                    } else if let Some(index) = self.visible_indices().first().copied() {
                        self.set_focus(SettingsFocus::Row(index));
                    }
                } else {
                    return self.key(KeyCode::Tab, key == KeyCode::Left, false);
                }
            }
            KeyCode::Enter | KeyCode::Space => return self.activate(self.view.focus),
            KeyCode::Home | KeyCode::End if self.view.focus == SettingsFocus::Search => {
                self.edit_command(
                    if key == KeyCode::Home { "home" } else { "end" },
                    control,
                    shift,
                );
            }
            _ => {}
        }
        self.ensure_visible();
        Vec::new()
    }

    fn focus_order(&self) -> Vec<SettingsFocus> {
        let mut order = SettingsCategory::ALL.map(SettingsFocus::Category).to_vec();
        order.push(SettingsFocus::Search);
        order.extend(self.current_page_tab());
        order.extend(self.control_set_focus());
        order.extend(self.visible_indices().into_iter().map(SettingsFocus::Row));
        let layout = self.layout();
        order.extend(
            self.row_actions(&layout)
                .into_iter()
                .chain(self.footer_links(&layout))
                .map(|(focus, _)| focus),
        );
        order.push(SettingsFocus::Close);
        order
    }

    pub(crate) fn row_emphasis(&self, index: usize) -> Option<RowEmphasis> {
        [
            (
                self.view.focus == SettingsFocus::Row(index),
                RowEmphasis::Focused,
            ),
            (self.view.selected == Some(index), RowEmphasis::Selected),
            (
                self.view.hover == Some(SettingsFocus::Row(index)),
                RowEmphasis::Hovered,
            ),
        ]
        .into_iter()
        .find_map(|(applies, emphasis)| applies.then_some(emphasis))
    }

    /// The muted hint an empty search field shows, focused or not.
    pub fn search_placeholder(&self) -> Option<&'static str> {
        self.query
            .is_empty()
            .then_some("Search all settings (Ctrl+F)")
    }

    /// The page's changed settings that Reset page returns to their
    /// defaults. Display modes and saved progress are never reset from here.
    fn page_reset_candidates(&self) -> Vec<usize> {
        self.visible_indices()
            .into_iter()
            .filter(|index| {
                let setting = &self.settings[*index];
                setting.category == self.category
                    && !setting.details.exclude_from_category_reset
                    && setting.details.policy != ApplyPolicy::DisplayPreview
                    && self.resettable(*index)
            })
            .collect()
    }

    /// What the page reset is called: after the control set it resets on a
    /// control set's page.
    pub fn page_reset_label(&self) -> String {
        self.control_set_page()
            .map_or("Reset page".into(), |set| format!("Reset {}", set.label()))
    }

    /// The question an open confirmation asks.
    pub fn confirmation_prompt(&self) -> Option<String> {
        if let Some(seconds) = self.view.display_confirmation {
            return Some(format!(
                "Keep these display settings? They revert in {seconds} s."
            ));
        }
        self.view.reset_confirmation.then(|| {
            let count = self.page_reset_candidates().len();
            let default = if count == 1 {
                "its default"
            } else {
                "their defaults"
            };
            if let Some(set) = self.control_set_page() {
                return format!(
                    "Reset {count} {} of {} to {default}?",
                    binding_noun(set.device, count),
                    set.label()
                );
            }
            let noun = if count == 1 { "setting" } else { "settings" };
            format!(
                "Reset {count} {noun} on this page to {default}? \
                 Display modes and saved progress are kept."
            )
        })
    }

    /// An open confirmation's answers, its default (Enter) first.
    pub(crate) fn confirmation_actions(
        &self,
        layout: &SettingsLayout,
    ) -> Option<[(String, IntRect); 2]> {
        let reset = self.view.reset_confirmation;
        (reset || self.view.display_confirmation.is_some()).then(|| {
            let y = layout.footer.y + layout.footer.h - 32;
            let (confirm, cancel) = if reset {
                ("Reset", "Cancel")
            } else {
                ("Keep", "Revert")
            };
            [
                (
                    confirm.into(),
                    IntRect::new(layout.footer.x + 6, y, 130, 28),
                ),
                (
                    cancel.into(),
                    IntRect::new(layout.footer.x + 146, y, 110, 28),
                ),
            ]
        })
    }

    /// Whether sub-page tabs have room for an icon beside their caption.
    pub(crate) fn page_tab_icons(&self, layout: &SettingsLayout) -> bool {
        layout.list.w >= 480
    }

    /// The keys that operate the focused control, for the footer.
    pub fn input_hint(&self) -> String {
        if self.view.choice.is_some() {
            return "Up/Down: choose · Enter: select · Esc: cancel".into();
        }
        if let Some(index) = self.view.capturing {
            let device = self
                .settings
                .get(index)
                .and_then(|setting| setting.details.control)
                .map_or(ControlDevice::Keyboard, |binding| binding.set.device);
            return format!(
                "Press the new {} · Esc: keep the old one",
                binding_noun(device, 1)
            );
        }
        let action = match self.view.focus {
            SettingsFocus::Row(index) => self.settings.get(index).map_or("", |setting| {
                if setting
                    .details
                    .control
                    .is_some_and(|binding| binding.set.device == ControlDevice::Gamepad)
                {
                    "Enter: press a new button"
                } else if setting.details.binding {
                    "Enter: press a new key"
                } else {
                    match setting.value {
                        AdvancedConfigValue::Bool(_) => "Enter: toggle",
                        AdvancedConfigValue::Integer { .. } => {
                            "Left/Right: adjust · Enter: type a value"
                        }
                        AdvancedConfigValue::Choice { .. } => "Left/Right: change · Enter: list",
                        _ => "Enter: edit",
                    }
                }
            }),
            SettingsFocus::Search => "Type to search · Down: results",
            SettingsFocus::Category(_) => "Up/Down: pages · Right: settings",
            SettingsFocus::AudioPage(_)
            | SettingsFocus::ControlsPage(_)
            | SettingsFocus::ControlSet(_) => "Left/Right: switch",
            _ => "Tab: next",
        };
        let reset = match self.view.focus {
            SettingsFocus::Row(index) if self.resettable(index) => " · Del: reset",
            _ => "",
        };
        if action.is_empty() {
            "Esc: close".into()
        } else {
            format!("{action}{reset} · Esc: close")
        }
    }

    /// Shows the next (`step` 1) or previous (-1) control set, wrapping around.
    pub fn cycle_control_set(&mut self, step: isize) {
        let sets = self.control_sets();
        if sets.is_empty() {
            return;
        }
        let current = self
            .current_control_set()
            .and_then(|shown| sets.iter().position(|set| *set == shown))
            .unwrap_or(0);
        let next = (current as isize + step).rem_euclid(sets.len() as isize) as usize;
        self.choose_set(sets[next]);
        self.view.scroll = 0;
        self.view.selected = self.visible_indices().first().copied();
    }

    pub fn set_focus(&mut self, focus: SettingsFocus) {
        self.view.focus = focus;
        if let SettingsFocus::Row(index) = focus {
            // A change's status describes the setting that changed.
            if self.view.selected != Some(index) {
                self.view.message.clear();
            }
            self.view.selected = Some(index);
        }
        self.ensure_visible();
    }

    fn page_size(&self) -> usize {
        if self.control_set_page().is_some() {
            return self.visible_indices().len().max(1);
        }
        (self.layout().list.h / self.row_height()).max(1) as usize
    }

    fn row_height(&self) -> i32 {
        if self.category == SettingsCategory::Audio {
            28
        } else {
            ROW_HEIGHT
        }
    }

    fn ensure_visible(&mut self) {
        let visible = self.visible_indices();
        let count = self.page_size();
        self.view.scroll = self.view.scroll.min(visible.len().saturating_sub(count));
        if let SettingsFocus::Row(index) = self.view.focus {
            if let Some(position) = visible.iter().position(|i| *i == index) {
                if position < self.view.scroll {
                    self.view.scroll = position;
                }
                if position >= self.view.scroll + count {
                    self.view.scroll = position + 1 - count;
                }
            }
        }
    }

    pub fn scroll(&mut self, amount: i32) {
        if self.view.microphone_test_open {
            return;
        }
        if self.view.choice.is_some() {
            self.scroll_choices(amount);
            return;
        }
        self.view.scroll = self.view.scroll.saturating_add_signed(amount as isize).min(
            self.visible_indices()
                .len()
                .saturating_sub(self.page_size()),
        );
    }

    fn editable(&mut self, index: usize) -> bool {
        self.settings.get(index).is_some_and(|setting| {
            if let Some(reason) = &setting.details.unavailable {
                self.view.message = reason.clone();
                false
            } else {
                setting.value.is_editable() && setting.details.policy != ApplyPolicy::ReadOnly
            }
        })
    }

    fn activate(&mut self, focus: SettingsFocus) -> Vec<SettingsAction> {
        match focus {
            SettingsFocus::Row(index) if self.editable(index) => {
                let setting = &self.settings[index];
                if setting.details.binding {
                    return vec![SettingsAction::CaptureBinding(index)];
                }
                match setting.value {
                    AdvancedConfigValue::Bool(_) => return self.adjust(index, 1),
                    AdvancedConfigValue::Choice { .. } => self.open_choices(index),
                    _ => {
                        let mut edit = RenameEdit::new(setting.value.serialized(), None);
                        edit.select_all();
                        self.view.edit = Some((index, edit));
                    }
                }
            }
            SettingsFocus::Category(category) => self.select_category(category),
            SettingsFocus::AudioPage(page) => self.select_audio_page(page),
            SettingsFocus::ControlsPage(page) => self.select_controls_page(page),
            SettingsFocus::ControlSet(index) => {
                if let Some(device) = self.controls_page.device() {
                    self.choose_set(ControlSet { device, index });
                    self.view.scroll = 0;
                    self.view.selected = self.visible_indices().first().copied();
                    self.view.focus = SettingsFocus::ControlSet(index);
                }
            }
            SettingsFocus::Modified => {
                self.modified_only = !self.modified_only;
                self.view.scroll = 0;
            }
            SettingsFocus::Advanced => {
                self.show_advanced = !self.show_advanced;
                self.view.scroll = 0;
            }
            SettingsFocus::ResetCategory => {
                if !self.query.is_empty() {
                    self.view.message = "Clear search before resetting a category.".into();
                    return Vec::new();
                }
                if self.page_reset_candidates().is_empty() {
                    self.view.message = match self.control_set_page() {
                        Some(set) => format!(
                            "{} already uses its default {}.",
                            set.label(),
                            binding_noun(set.device, 2)
                        ),
                        None => "Every setting on this page is already at its default.".into(),
                    };
                } else {
                    self.view.reset_confirmation = true;
                }
            }
            SettingsFocus::TestMicrophone => {
                self.cancel_interaction();
                self.view.choice = None;
                self.view.edit = None;
                self.view.microphone_test_open = true;
                self.view.focus = SettingsFocus::RecordMicrophone;
            }
            SettingsFocus::RecordMicrophone => return vec![SettingsAction::TestMicrophone],
            SettingsFocus::CloseMicrophoneTest => {
                self.cancel_interaction();
                self.view.microphone_test_open = false;
                self.view.focus = SettingsFocus::TestMicrophone;
                return vec![SettingsAction::CancelMicrophoneTest];
            }
            SettingsFocus::RefreshDevices => return vec![SettingsAction::RefreshDevices],
            SettingsFocus::Close => return vec![SettingsAction::Close],
            _ => {}
        }
        Vec::new()
    }

    fn adjust(&mut self, index: usize, direction: i128) -> Vec<SettingsAction> {
        if !self.editable(index) {
            return Vec::new();
        }
        let setting = &self.settings[index];
        if setting.details.binding {
            return Vec::new();
        }
        let value = match &setting.value {
            AdvancedConfigValue::Bool(value) => AdvancedConfigValue::Bool(!value),
            AdvancedConfigValue::Integer { value, min, max } => AdvancedConfigValue::Integer {
                value: value
                    .saturating_add(direction * setting.details.step.max(1))
                    .clamp(*min, *max),
                min: *min,
                max: *max,
            },
            AdvancedConfigValue::Choice { value, choices } if !choices.is_empty() => {
                let index = choices.iter().position(|c| c.value == *value).unwrap_or(0) as i128;
                let next = (index + direction).rem_euclid(choices.len() as i128) as usize;
                AdvancedConfigValue::Choice {
                    value: choices[next].value.clone(),
                    choices: choices.clone(),
                }
            }
            _ => return Vec::new(),
        };
        vec![SettingsAction::Change(index, value)]
    }

    pub fn cancel_interaction(&mut self) {
        self.view.pressed = None;
        self.view.dragging = None;
        self.view.scroll_drag = None;
        if let Some(choice) = self.view.choice.as_mut() {
            choice.pressed = None;
        }
    }

    pub fn pointer(&mut self, point: GuiPoint, down: bool) -> Vec<SettingsAction> {
        if self.view.microphone_test_open {
            return self.microphone_pointer(point, down);
        }
        // A press elsewhere keeps the old key, so the next key pressed cannot
        // land on a binding the press takes out of view.
        if down && self.view.capturing.is_some() {
            self.end_capture();
            self.view.message = "Binding unchanged".into();
        }
        if self.view.choice.is_some() {
            return self.choice_pointer(point, down);
        }
        let layout = self.layout();
        if let Some([(_, confirm), (_, cancel)]) = self.confirmation_actions(&layout) {
            if !down {
                if contains(confirm, point) {
                    return self.key(KeyCode::Enter, false, false);
                }
                if contains(cancel, point) {
                    return self.key(KeyCode::Escape, false, false);
                }
            }
            return Vec::new();
        }
        if !down && (self.view.dragging.is_some() || self.view.scroll_drag.is_some()) {
            let actions = self.pointer_move(point);
            self.cancel_interaction();
            return actions;
        }
        if down {
            if let Some((track, thumb)) = self.scrollbar() {
                if contains(track, point) {
                    self.view.scroll_drag = Some(if contains(thumb, point) {
                        point.y as i32 - thumb.y
                    } else {
                        thumb.h / 2
                    });
                    self.view.pressed = None;
                    return self.pointer_move(point);
                }
            }
            for index in self.visible_indices() {
                let Some(row) = self.row_rect(&layout, index) else {
                    continue;
                };
                let track = slider_rect(row);
                let setting = &self.settings[index];
                if contains(track, point)
                    && !setting.details.binding
                    && setting.details.policy != ApplyPolicy::DisplayPreview
                    && matches!(setting.value, AdvancedConfigValue::Integer { min, max, .. } if max > min && max - min <= 1000)
                    && self.editable(index)
                {
                    self.view.edit = None;
                    self.set_focus(SettingsFocus::Row(index));
                    if point.x < (track.x + 16) as f32 {
                        return self.adjust(index, -1);
                    }
                    if point.x >= (track.x + track.w - 16) as f32 {
                        return self.adjust(index, 1);
                    }
                    self.view.dragging = Some(index);
                    return self.pointer_move(point);
                }
            }
        }
        let target = self
            .targets(&layout)
            .into_iter()
            .find(|(_, rect)| contains(*rect, point))
            .map(|(f, _)| f);
        if down {
            if let Some((index, edit)) = self.view.edit.as_ref() {
                if target != Some(SettingsFocus::Row(*index)) {
                    match self.parse_value(*index, edit.text()) {
                        Ok(value) => {
                            let index = *index;
                            self.view.edit = None;
                            self.view.pressed = target;
                            if let Some(focus) = target {
                                self.set_focus(focus);
                            }
                            return vec![SettingsAction::Change(index, value)];
                        }
                        Err(error) => {
                            self.view.message = error;
                            return Vec::new();
                        }
                    }
                }
            }
            self.view.pressed = target;
            if let Some(focus) = target {
                self.set_focus(focus);
            }
        } else if let Some(focus) = self.view.pressed.take().filter(|f| Some(*f) == target) {
            if self
                .view
                .edit
                .as_ref()
                .is_some_and(|(index, _)| focus == SettingsFocus::Row(*index))
            {
                return Vec::new();
            }
            return self.activate(focus);
        }
        Vec::new()
    }

    pub fn pointer_move(&mut self, point: GuiPoint) -> Vec<SettingsAction> {
        if self.view.choice.is_some() {
            self.choice_hover(point);
            return Vec::new();
        }
        let layout = self.layout();
        self.view.hover = self
            .targets(&layout)
            .into_iter()
            .find(|(_, rect)| contains(*rect, point))
            .map(|(focus, _)| focus);
        if let Some(offset) = self.view.scroll_drag {
            if let Some((track, thumb)) = self.scrollbar() {
                let maximum = self
                    .visible_indices()
                    .len()
                    .saturating_sub(self.page_size());
                let travel = (track.h - thumb.h).max(1);
                self.view.scroll = (((point.y as i32 - track.y - offset).clamp(0, travel) as f64
                    / f64::from(travel))
                    * maximum as f64)
                    .round() as usize;
            }
        }
        let Some(index) = self.view.dragging else {
            return Vec::new();
        };
        let layout = self.layout();
        let Some(row) = self.row_rect(&layout, index) else {
            return Vec::new();
        };
        let setting = &self.settings[index];
        let AdvancedConfigValue::Integer { value, min, max } = setting.value else {
            return Vec::new();
        };
        let track = slider_rect(row);
        let fraction = ((point.x - (track.x + 24) as f32) / (track.w - 48) as f32).clamp(0.0, 1.0);
        let next = min + ((max - min) as f32 * fraction).round() as i128;
        if next == value {
            return Vec::new();
        }
        vec![SettingsAction::Change(
            index,
            AdvancedConfigValue::Integer {
                value: next,
                min,
                max,
            },
        )]
    }

    fn scrollbar(&self) -> Option<(IntRect, IntRect)> {
        let count = self.visible_indices().len();
        let page = self.page_size();
        if count <= page {
            return None;
        }
        let list = self.layout().list;
        let track = IntRect::new(list.x + list.w - 10, list.y, 10, list.h);
        let height = (list.h * page as i32 / count as i32).max(20).min(list.h);
        let top = (list.h - height) * self.view.scroll as i32 / (count - page) as i32;
        Some((track, IntRect::new(track.x, track.y + top, track.w, height)))
    }

    /// The binding one step `(columns, rows)` across the grid from the one
    /// at `index`; `None` past the grid's edge.
    fn grid_neighbour(&self, index: usize, (dx, dy): (i32, i32)) -> Option<usize> {
        let command = self.settings.get(index)?.details.control?.command as i32;
        let (column, row) = (command % 3 + dx, command / 3 + dy);
        if !(0..3).contains(&column) || !(0..4).contains(&row) {
            return None;
        }
        let target = (row * 3 + column) as usize;
        self.visible_indices().into_iter().find(|candidate| {
            self.settings[*candidate]
                .details
                .control
                .is_some_and(|binding| binding.command == target)
        })
    }

    /// Where focus goes below the last setting: the first row or page action.
    fn page_actions_start(&self) -> SettingsFocus {
        let layout = self.layout();
        self.row_actions(&layout)
            .into_iter()
            .chain(self.footer_links(&layout))
            .map(|(focus, _)| focus)
            .next()
            .unwrap_or(SettingsFocus::Close)
    }

    fn row_rect(&self, layout: &SettingsLayout, index: usize) -> Option<IntRect> {
        if self.control_set_page().is_some() {
            return self
                .visible_indices()
                .contains(&index)
                .then(|| self.settings[index].details.control)
                .flatten()
                .map(|binding| grid_cell(layout, binding.command));
        }
        let position = self.visible_indices().iter().position(|i| *i == index)?;
        (position >= self.view.scroll && position < self.view.scroll + self.page_size()).then(
            || {
                IntRect::new(
                    layout.list.x,
                    layout.list.y + (position - self.view.scroll) as i32 * self.row_height(),
                    layout.list.w - 14,
                    self.row_height() - 3,
                )
            },
        )
    }

    fn targets(&self, layout: &SettingsLayout) -> Vec<(SettingsFocus, IntRect)> {
        let mut targets = vec![(SettingsFocus::Search, layout.search)];
        targets.extend(self.page_tab_rects(layout));
        targets.extend(
            self.control_set_pictures(layout)
                .into_iter()
                .map(|(index, rect)| (SettingsFocus::ControlSet(index), rect)),
        );
        targets.extend(
            SettingsCategory::ALL
                .into_iter()
                .enumerate()
                .map(|(i, c)| (SettingsFocus::Category(c), layout.tabs[i])),
        );
        // The selected row's actions sit inside it, so they take the pointer
        // before the row does.
        targets.extend(self.row_actions(layout));
        targets.extend(
            self.visible_indices()
                .into_iter()
                .filter_map(|i| self.row_rect(layout, i).map(|r| (SettingsFocus::Row(i), r))),
        );
        targets.extend(self.footer_links(layout));
        targets.push((SettingsFocus::Close, layout.back));
        targets
    }

    /// The current category's tabs, side by side under the search field.
    fn page_tab_rects(&self, layout: &SettingsLayout) -> Vec<(SettingsFocus, IntRect)> {
        let icons = self.page_tab_icons(layout);
        let mut x = layout.list.x;
        self.page_tabs()
            .into_iter()
            .map(|tab| {
                let (with_icon, plain) = match tab {
                    SettingsFocus::AudioPage(AudioPage::Sound) => (100, 72),
                    SettingsFocus::AudioPage(AudioPage::Voice) => (132, 108),
                    SettingsFocus::ControlsPage(ControlsPage::Keyboard) => (112, 86),
                    SettingsFocus::ControlsPage(ControlsPage::Controller) => (124, 98),
                    _ => (104, 80),
                };
                let width = if icons { with_icon } else { plain };
                let row = Self::page_tab_row(layout);
                let rect = IntRect::new(x, row.y, width, row.h);
                x += width + 6;
                (tab, rect)
            })
            .collect()
    }

    /// Links inked into rows just left of their values: the microphone's
    /// test. Delete resets a changed setting, so rows carry no reset link.
    pub(crate) fn row_actions(&self, layout: &SettingsLayout) -> Vec<(SettingsFocus, IntRect)> {
        if self.control_set_page().is_some() {
            return Vec::new();
        }
        self.visible_indices()
            .into_iter()
            .filter_map(|index| {
                let row = self.row_rect(layout, index)?;
                let setting = &self.settings[index];
                (setting.id.section == "Voice" && setting.id.key == "InputDevice")
                    .then(|| (SettingsFocus::TestMicrophone, row_link_rect(row)))
            })
            .collect()
    }

    fn resettable(&self, index: usize) -> bool {
        self.settings.get(index).is_some_and(|setting| {
            setting.is_modified()
                && setting.value.is_editable()
                && setting.details.unavailable.is_none()
                && setting.details.policy != ApplyPolicy::ReadOnly
        })
    }

    /// Page-wide actions, inked along the footer: the view filters and
    /// Reset page.
    pub(crate) fn footer_links(&self, layout: &SettingsLayout) -> Vec<(SettingsFocus, IntRect)> {
        let compact = layout.footer.w < 480;
        let y = layout.footer.y
            + if self.category == SettingsCategory::Audio {
                52
            } else if self.control_set_page().is_some() {
                22
            } else {
                42
            };
        let specs = if self.control_set_page().is_some() {
            vec![(SettingsFocus::ResetCategory, 150)]
        } else {
            vec![
                (SettingsFocus::Advanced, if compact { 112 } else { 150 }),
                (SettingsFocus::Modified, if compact { 84 } else { 116 }),
                (SettingsFocus::ResetCategory, if compact { 80 } else { 92 }),
            ]
        };
        let mut x = layout.footer.x;
        specs
            .into_iter()
            .map(|(focus, width)| {
                let rect = IntRect::new(x, y, width, 26);
                x += width + 10;
                (focus, rect)
            })
            .collect()
    }

    /// Advanced settings the current page would add when shown.
    pub fn advanced_count(&self) -> usize {
        self.settings
            .iter()
            .filter(|setting| {
                setting.advanced && setting.category == self.category && self.page_contains(setting)
            })
            .count()
    }
}

/// The text a row shows for its value: the application's own label when it
/// provides one (bindings, current-match rows), otherwise the value itself.
pub(crate) fn value_label(setting: &Setting) -> String {
    setting
        .details
        .display_value
        .clone()
        .unwrap_or_else(|| formatted_value(setting, &setting.value))
}

pub(crate) fn formatted_value(setting: &Setting, value: &AdvancedConfigValue) -> String {
    match value {
        AdvancedConfigValue::Choice { value, choices } => choices
            .iter()
            .find(|c| c.value == *value)
            .map(|c| c.label.clone())
            .unwrap_or_else(|| {
                if value.is_empty() {
                    "System default".into()
                } else {
                    value.clone()
                }
            }),
        AdvancedConfigValue::Bool(on) => if *on { "On" } else { "Off" }.into(),
        _ if setting.id.key.to_lowercase().contains("password") => "••••••".into(),
        AdvancedConfigValue::Integer { value, .. } => format!("{value}{}", setting.details.unit),
        _ => value.serialized(),
    }
}

/// The footer's second line: why a setting is unavailable, or where and when
/// a change takes effect.
pub(crate) fn detail_text(setting: &Setting) -> String {
    detail_text_in(setting, &setting.details.scope)
}

/// [`detail_text`] with `scope` naming where the setting applies.
fn detail_text_in(setting: &Setting, scope: &str) -> String {
    setting.details.unavailable.clone().unwrap_or_else(|| {
        let active = setting
            .details
            .active_value
            .as_ref()
            .map(|value| format!(" · Active: {value}"))
            .unwrap_or_default();
        let default = if setting.is_modified() {
            let default = setting
                .details
                .default_display
                .clone()
                .unwrap_or_else(|| formatted_value(setting, &setting.default));
            format!(" · Default: {default}")
        } else {
            String::new()
        };
        format!(
            "{scope} · {}{active}{default}",
            setting.details.policy.label()
        )
    })
}

/// Where the binding for `command` sits in the classic control sheets' 3x4
/// grid of commands.
fn grid_cell(layout: &SettingsLayout, command: usize) -> IntRect {
    let column_width = layout.list.w / 3;
    let row_height = (layout.list.h / 4).min(GRID_ROW_MAX);
    IntRect::new(
        layout.list.x + (command % 3) as i32 * column_width,
        layout.list.y + (command / 3) as i32 * row_height,
        column_width - 8,
        row_height - 4,
    )
}

/// What a control set on `device` binds, for `count` of them.
fn binding_noun(device: ControlDevice, count: usize) -> &'static str {
    match (device, count) {
        (ControlDevice::Keyboard, 1) => "key",
        (ControlDevice::Keyboard, _) => "keys",
        (ControlDevice::Gamepad, 1) => "button",
        (ControlDevice::Gamepad, _) => "buttons",
    }
}

/// Where a link inked into a row sits: just left of the row's value.
pub(crate) fn row_link_rect(row: IntRect) -> IntRect {
    IntRect::new(row.x + row.w - 174 - 56, row.y + 3, 48, row.h - 6)
}

fn value_rect(row: IntRect) -> IntRect {
    IntRect::new(row.x + row.w - 174, row.y + 2, 174, 26)
}

fn slider_rect(row: IntRect) -> IntRect {
    IntRect::new(row.x + row.w - 174, row.y + 9, 128, 16)
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preference(key: &str, value: AdvancedConfigValue) -> Setting {
        Setting {
            id: SettingId::new("Graphics", key),
            label: key.into(),
            keywords: String::new(),
            category: SettingsCategory::Display,
            advanced: false,
            default: value.clone(),
            value,
            details: Default::default(),
        }
    }

    #[test]
    fn the_row_the_footer_acts_on_stays_highlighted_while_search_has_focus() {
        let mut controller = SettingsController::new(
            ["ShowClock", "ShowStats", "ShowPortraits"]
                .map(|key| preference(key, AdvancedConfigValue::Bool(true)))
                .to_vec(),
        );
        controller.select_category(SettingsCategory::Display);
        controller.focus_search();
        assert_eq!(controller.row_emphasis(0), Some(RowEmphasis::Selected));
        assert_eq!(controller.row_emphasis(1), None);
        controller.set_focus(SettingsFocus::Row(1));
        assert_eq!(controller.row_emphasis(0), None);
        assert_eq!(controller.row_emphasis(1), Some(RowEmphasis::Focused));
    }

    #[test]
    fn numeric_values_and_defaults_carry_their_unit() {
        let seconds = |value| AdvancedConfigValue::Integer {
            value,
            min: 3,
            max: 60,
        };
        let mut duration = preference("Duration", seconds(12));
        duration.details.unit = " s".into();
        assert_eq!(value_label(&duration), "12 s");
        duration.value = seconds(30);
        assert!(detail_text(&duration).ends_with("Default: 12 s"));
    }

    #[test]
    fn an_empty_search_shows_a_hint_that_typing_replaces() {
        let mut controller = SettingsController::new(Vec::new());
        assert_eq!(
            controller.search_placeholder(),
            Some("Search all settings (Ctrl+F)")
        );
        controller.focus_search();
        assert!(controller.search_placeholder().is_some());
        controller.text("vol");
        assert_eq!(controller.search_placeholder(), None);
    }

    #[test]
    fn the_footer_names_the_keys_that_operate_the_focused_control() {
        let volume = AdvancedConfigValue::Integer {
            value: 50,
            min: 0,
            max: 100,
        };
        let mut binding = preference("Kbd1Key1", volume.clone());
        binding.details.binding = true;
        let mut controller = SettingsController::new(vec![
            preference("ShowClock", AdvancedConfigValue::Bool(true)),
            preference("Volume", volume),
            binding,
        ]);
        controller.select_category(SettingsCategory::Display);
        for (focus, hint) in [
            (SettingsFocus::Row(0), "Enter: toggle"),
            (
                SettingsFocus::Row(1),
                "Left/Right: adjust · Enter: type a value",
            ),
            (SettingsFocus::Row(2), "Enter: press a new key"),
            (SettingsFocus::Search, "Type to search · Down: results"),
        ] {
            controller.set_focus(focus);
            assert!(
                controller.input_hint().starts_with(hint),
                "{focus:?}: {}",
                controller.input_hint()
            );
            assert!(controller.input_hint().ends_with("Esc: close"));
        }
    }

    /// A Controls setting, bound to `(device, set, command)` when given.
    fn control(key: &str, binding: Option<(ControlDevice, usize, usize)>) -> Setting {
        let mut setting = preference(key, AdvancedConfigValue::Bool(true));
        setting.category = SettingsCategory::Controls;
        setting.details.control = binding.map(|(device, index, command)| ControlBinding {
            set: ControlSet { device, index },
            command,
        });
        setting.details.binding = binding.is_some();
        setting
    }

    #[test]
    fn controls_split_into_keyboard_controller_and_general_tabs_that_keep_their_own_set() {
        use ControlDevice::{Gamepad, Keyboard};
        let mut controller = SettingsController::new(vec![
            control("GamepadEnabled", None),
            control("Kbd1Key1", Some((Keyboard, 0, 0))),
            control("Kbd1Key2", Some((Keyboard, 0, 1))),
            control("Kbd2Key1", Some((Keyboard, 1, 0))),
            control("Pad1Button1", Some((Gamepad, 0, 0))),
            control("Pad2Button1", Some((Gamepad, 1, 0))),
        ]);
        controller.select_category(SettingsCategory::Controls);
        assert_eq!(controller.controls_page, ControlsPage::Keyboard);
        assert_eq!(controller.visible_indices(), vec![1, 2]);
        controller.set_focus(SettingsFocus::ControlSet(0));
        controller.key(KeyCode::Right, false, false);
        assert_eq!(controller.visible_indices(), vec![3]);
        controller.key(KeyCode::Right, false, false);
        assert_eq!(
            controller.visible_indices(),
            vec![1, 2],
            "keyboard sets cycle among themselves"
        );
        controller.key(KeyCode::Left, false, false);
        controller.select_controls_page(ControlsPage::Controller);
        assert_eq!(controller.visible_indices(), vec![4]);
        controller.cycle_control_set(1);
        assert_eq!(controller.visible_indices(), vec![5]);
        controller.select_controls_page(ControlsPage::General);
        assert_eq!(controller.visible_indices(), vec![0]);
        assert_eq!(controller.current_control_set(), None);
        controller.select_controls_page(ControlsPage::Keyboard);
        assert_eq!(
            controller.visible_indices(),
            vec![3],
            "each tab keeps its set"
        );
        controller.query = "Kbd1".into();
        assert_eq!(
            controller.visible_indices(),
            vec![1, 2],
            "search spans every tab and set"
        );
    }

    #[test]
    fn controls_tabs_switch_with_left_and_right_like_the_audio_tabs() {
        let mut controller = SettingsController::new(vec![control("GamepadEnabled", None)]);
        controller.select_category(SettingsCategory::Controls);
        let layout = controller.layout();
        let tabs: Vec<_> = controller
            .targets(&layout)
            .into_iter()
            .filter(|(focus, _)| matches!(focus, SettingsFocus::ControlsPage(_)))
            .collect();
        assert_eq!(
            tabs.iter().map(|(focus, _)| *focus).collect::<Vec<_>>(),
            ControlsPage::ALL.map(SettingsFocus::ControlsPage).to_vec()
        );
        for pair in tabs.windows(2) {
            assert!(pair[0].1.x + pair[0].1.w < pair[1].1.x);
        }
        assert!(tabs[2].1.x + tabs[2].1.w <= layout.list.x + layout.list.w);
        controller.set_focus(SettingsFocus::Category(SettingsCategory::Controls));
        controller.key(KeyCode::Right, false, false);
        assert_eq!(
            controller.view.focus,
            SettingsFocus::ControlsPage(ControlsPage::Keyboard)
        );
        controller.key(KeyCode::Right, false, false);
        assert_eq!(controller.controls_page, ControlsPage::Controller);
        assert_eq!(
            controller.view.focus,
            SettingsFocus::ControlsPage(ControlsPage::Controller)
        );
        controller.key(KeyCode::Left, false, false);
        controller.key(KeyCode::Left, false, false);
        assert_eq!(
            controller.controls_page,
            ControlsPage::General,
            "switching wraps"
        );
    }

    #[test]
    fn a_control_set_is_chosen_by_its_picture_with_the_pointer_or_the_arrow_keys() {
        let mut controller = SettingsController::new(
            (0..4)
                .map(|set| {
                    control(
                        &format!("Kbd{}Key1", set + 1),
                        Some((ControlDevice::Keyboard, set, 0)),
                    )
                })
                .collect(),
        );
        controller.select_controls_page(ControlsPage::Keyboard);
        let layout = controller.layout();
        let targets = controller.targets(&layout);
        let tab = targets
            .iter()
            .find(|(focus, _)| *focus == SettingsFocus::ControlsPage(ControlsPage::Keyboard))
            .unwrap()
            .1;
        let pictures: Vec<_> = targets
            .iter()
            .filter(|(focus, _)| matches!(focus, SettingsFocus::ControlSet(_)))
            .copied()
            .collect();
        assert_eq!(
            pictures.iter().map(|(focus, _)| *focus).collect::<Vec<_>>(),
            (0..4).map(SettingsFocus::ControlSet).collect::<Vec<_>>()
        );
        for (_, picture) in &pictures {
            assert!(picture.y >= tab.y + tab.h && picture.y + picture.h <= layout.list.y);
            assert!(
                picture.w > picture.h * 2,
                "the set's keyboard, not a button"
            );
        }
        let third = pictures[2].1;
        let centre = GuiPoint::new(
            (third.x + third.w / 2) as f32,
            (third.y + third.h / 2) as f32,
        );
        controller.pointer(centre, true);
        controller.pointer(centre, false);
        assert_eq!(controller.visible_indices(), vec![2]);
        assert_eq!(controller.view.focus, SettingsFocus::ControlSet(2));
        controller.key(KeyCode::Right, false, false);
        assert_eq!(controller.visible_indices(), vec![3]);
        assert_eq!(controller.view.focus, SettingsFocus::ControlSet(3));
        controller.key(KeyCode::Down, false, false);
        assert_eq!(controller.view.focus, SettingsFocus::Row(3));
        controller.key(KeyCode::Up, false, false);
        assert_eq!(controller.view.focus, SettingsFocus::ControlSet(3));
        controller.key(KeyCode::Up, false, false);
        assert_eq!(
            controller.view.focus,
            SettingsFocus::ControlsPage(ControlsPage::Keyboard)
        );
    }

    /// A general setting, then keyboard set 1's twelve bindings.
    fn keyboard_set() -> SettingsController {
        SettingsController::new(
            std::iter::once(control("GamepadEnabled", None))
                .chain((0..12).map(|command| {
                    control(
                        &format!("Kbd1Key{}", command + 1),
                        Some((ControlDevice::Keyboard, 0, command)),
                    )
                }))
                .collect(),
        )
    }

    #[test]
    fn a_control_sets_bindings_sit_in_the_classic_three_by_four_grid_of_commands() {
        let mut controller = keyboard_set();
        controller.select_controls_page(ControlsPage::Keyboard);
        let layout = controller.layout();
        let cells: Vec<_> = (1..=12)
            .map(|index| {
                controller
                    .row_rect(&layout, index)
                    .expect("every key in view")
            })
            .collect();
        for (command, cell) in cells.iter().enumerate() {
            let (column, row) = (command % 3, command / 3);
            assert_eq!(cell.y, cells[row * 3].y, "command {command} shares its row");
            assert_eq!(
                cell.x, cells[column].x,
                "command {command} shares its column"
            );
            if column > 0 {
                assert!(cells[command - 1].x + cells[command - 1].w < cell.x);
            }
            if row > 0 {
                assert!(cells[command - 3].y + cells[command - 3].h <= cell.y);
            }
            assert!(cell.x + cell.w <= layout.list.x + layout.list.w);
            assert!(cell.y + cell.h <= layout.list.y + layout.list.h);
        }
        assert!(controller.scrollbar().is_none(), "the whole set is in view");
        assert!(
            controller.row_actions(&layout).is_empty(),
            "Delete and the page reset restore defaults"
        );
        let dig = cells[5];
        let centre = GuiPoint::new((dig.x + dig.w / 2) as f32, (dig.y + dig.h / 2) as f32);
        controller.pointer(centre, true);
        assert_eq!(
            controller.pointer(centre, false),
            vec![SettingsAction::CaptureBinding(6)],
            "a click waits for the new key"
        );
    }

    #[test]
    fn arrow_keys_move_across_the_binding_grid_and_enter_waits_for_a_new_key() {
        let mut controller = keyboard_set();
        controller.select_controls_page(ControlsPage::Keyboard);
        let key = |controller: &mut SettingsController, key| {
            controller.key(key, false, false);
            controller.view.focus
        };
        // Index 1 + n is command n: row n / 3, column n % 3.
        controller.set_focus(SettingsFocus::Row(5));
        assert_eq!(key(&mut controller, KeyCode::Right), SettingsFocus::Row(6));
        assert_eq!(
            key(&mut controller, KeyCode::Right),
            SettingsFocus::Row(6),
            "the right edge holds"
        );
        assert_eq!(key(&mut controller, KeyCode::Down), SettingsFocus::Row(9));
        assert_eq!(key(&mut controller, KeyCode::Left), SettingsFocus::Row(8));
        assert_eq!(key(&mut controller, KeyCode::Up), SettingsFocus::Row(5));
        assert_eq!(key(&mut controller, KeyCode::Up), SettingsFocus::Row(2));
        assert_eq!(
            key(&mut controller, KeyCode::Up),
            SettingsFocus::ControlSet(0)
        );
        controller.set_focus(SettingsFocus::Row(11));
        assert_eq!(
            key(&mut controller, KeyCode::Down),
            SettingsFocus::ResetCategory,
            "the bottom row leads to the page's actions"
        );
        controller.set_focus(SettingsFocus::Row(8));
        assert!(controller
            .input_hint()
            .starts_with("Enter: press a new key"));
        assert_eq!(
            controller.key(KeyCode::Enter, false, false),
            vec![SettingsAction::CaptureBinding(8)]
        );
        controller.begin_capture(8);
        assert_eq!(
            controller.input_hint(),
            "Press the new key · Esc: keep the old one"
        );
        controller.end_capture();
        controller.settings[8].details.control = Some(ControlBinding {
            set: ControlSet {
                device: ControlDevice::Gamepad,
                index: 0,
            },
            command: 7,
        });
        assert!(
            controller
                .input_hint()
                .starts_with("Enter: press a new button"),
            "{}",
            controller.input_hint()
        );
    }

    #[test]
    fn a_control_set_page_keeps_only_a_reset_for_the_set_and_says_what_it_resets() {
        let mut controller = keyboard_set();
        controller.select_controls_page(ControlsPage::Keyboard);
        let layout = controller.layout();
        let links: Vec<_> = controller
            .footer_links(&layout)
            .into_iter()
            .map(|(focus, _)| focus)
            .collect();
        assert_eq!(links, vec![SettingsFocus::ResetCategory]);
        assert_eq!(controller.page_reset_label(), "Reset Keyboard 1");
        controller.set_focus(SettingsFocus::ResetCategory);
        controller.key(KeyCode::Enter, false, false);
        assert!(controller
            .view
            .message
            .contains("Keyboard 1 already uses its default keys"));
        controller.settings[4].value = AdvancedConfigValue::Bool(false);
        controller.key(KeyCode::Enter, false, false);
        assert_eq!(
            controller.confirmation_prompt().as_deref(),
            Some("Reset 1 key of Keyboard 1 to its default?")
        );
    }

    #[test]
    fn a_control_set_is_named_with_the_players_who_use_it() {
        let mut controller = keyboard_set();
        controller.select_controls_page(ControlsPage::Keyboard);
        assert_eq!(
            controller.control_set_caption().as_deref(),
            Some("Keyboard 1")
        );
        let first = ControlSet {
            device: ControlDevice::Keyboard,
            index: 0,
        };
        controller.set_users = vec![(first, "Tyler".into()), (first, "Robin".into())];
        assert_eq!(
            controller.control_set_caption().as_deref(),
            Some("Keyboard 1 · used by Tyler, Robin")
        );
        controller.set_focus(SettingsFocus::Row(1));
        assert_eq!(
            controller.footer_detail(),
            "Keyboard 1 · used by Tyler, Robin · Applies now",
            "the footer names the players wherever the caption has no room"
        );
        controller.select_controls_page(ControlsPage::General);
        assert_eq!(controller.control_set_caption(), None);
    }

    #[test]
    fn a_press_elsewhere_while_a_key_is_awaited_keeps_the_old_key() {
        let mut controller = keyboard_set();
        controller.select_controls_page(ControlsPage::Keyboard);
        controller.begin_capture(1);
        let layout = controller.layout();
        let tab = controller
            .targets(&layout)
            .into_iter()
            .find(|(focus, _)| *focus == SettingsFocus::ControlsPage(ControlsPage::Controller))
            .unwrap()
            .1;
        let centre = GuiPoint::new((tab.x + tab.w / 2) as f32, (tab.y + tab.h / 2) as f32);
        controller.pointer(centre, true);
        assert_eq!(
            controller.view.capturing, None,
            "no key lands on a hidden binding"
        );
        assert_eq!(controller.view.message, "Binding unchanged");
        controller.pointer(centre, false);
        assert_eq!(
            controller.controls_page,
            ControlsPage::Controller,
            "the press still does what it targets"
        );
    }

    /// The options book as `OptionsBook::layout` places it in the popup
    /// window over an 800x600 screen.
    fn book_in_an_800x600_popup() -> BookLayout {
        BookLayout {
            title_center: (400, 64),
            back_button: IntRect::new(106, 522, 153, 32),
            tabular: IntRect::new(76, 101, 648, 426),
            paper: IntRect::new(166, 104, 558, 423),
            sheet: IntRect::new(186, 131, 508, 364),
            tab_clips: std::array::from_fn(|i| (76, 110 + i as i32 * 58)),
            tab_icons: [(0, 0); 7],
            tab_captions: [(0, 0); 7],
            focus_highlight: IntRect::new(0, 0, 0, 0),
            tab_height: 66,
            tab_icon_size: 32,
        }
    }

    #[test]
    fn a_popup_closes_from_a_mark_on_its_paper_instead_of_the_back_button() {
        let book = book_in_an_800x600_popup();
        assert_eq!(
            SettingsLayout::from_book(&book, false).back,
            book.back_button,
            "the whole-screen book keeps its Back button"
        );
        let popup = SettingsLayout::from_book(&book, true);
        let close = popup.back;
        assert!(
            close.y >= book.paper.y && close.y + close.h <= popup.search.y,
            "on the paper, above the search field: {close:?}"
        );
        assert!(
            close.x > popup.search.x && close.x + close.w <= popup.search.x + popup.search.w,
            "at the page's right: {close:?}"
        );
    }

    #[test]
    fn a_control_sets_grid_keeps_every_key_when_only_changed_is_on_elsewhere() {
        let mut controller = keyboard_set();
        controller.modified_only = true;
        controller.select_controls_page(ControlsPage::Keyboard);
        assert_eq!(controller.visible_indices(), (1..=12).collect::<Vec<_>>());
    }

    /// The Voice chat page's switch and microphone choice.
    fn voice_page() -> SettingsController {
        use crate::startup_options_advanced::AdvancedConfigChoice;
        let voice = |key: &str, value: AdvancedConfigValue| Setting {
            id: SettingId::new("Voice", key),
            label: key.into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            default: value.clone(),
            value,
            details: Default::default(),
        };
        let device = AdvancedConfigValue::Choice {
            value: String::new(),
            choices: vec![AdvancedConfigChoice {
                value: String::new(),
                label: "System default".into(),
            }],
        };
        let mut controller = SettingsController::new(vec![
            voice("Enabled", AdvancedConfigValue::Bool(true)),
            voice("InputDevice", device),
        ]);
        controller.select_audio_page(AudioPage::Voice);
        controller
    }

    #[test]
    fn the_microphone_row_offers_its_test_beside_the_device_choice() {
        let mut controller = voice_page();
        let layout = controller.layout();
        assert!(
            controller
                .footer_links(&layout)
                .iter()
                .all(|(focus, _)| *focus != SettingsFocus::TestMicrophone),
            "not parked in the footer"
        );
        let row = controller.row_rect(&layout, 1).unwrap();
        let test = |controller: &SettingsController| {
            controller
                .row_actions(&layout)
                .into_iter()
                .find(|(focus, _)| *focus == SettingsFocus::TestMicrophone)
                .map(|(_, rect)| rect)
        };
        controller.set_focus(SettingsFocus::Row(0));
        let link = test(&controller).expect("shown while another row is selected");
        assert!(link.y >= row.y && link.y + link.h <= row.y + row.h);
        assert!(
            link.x + link.w <= value_rect(row).x,
            "left of the device choice"
        );
        let centre = GuiPoint::new((link.x + link.w / 2) as f32, (link.y + link.h / 2) as f32);
        controller.pointer(centre, true);
        controller.pointer(centre, false);
        assert!(controller.view.microphone_test_open);
    }

    #[test]
    fn a_rebound_key_names_its_default_with_the_applications_label() {
        let key = |value| AdvancedConfigValue::Integer {
            value,
            min: 0,
            max: 255,
        };
        let mut binding = preference("Kbd1Key1", key(81));
        binding.value = key(123);
        binding.details.display_value = Some("F12".into());
        binding.details.default_display = Some("Q".into());
        assert_eq!(value_label(&binding), "F12");
        assert!(detail_text(&binding).ends_with("Default: Q"));
    }

    #[test]
    fn a_changed_setting_is_marked_and_names_the_default_it_would_reset_to() {
        let mut clock = preference("ShowClock", AdvancedConfigValue::Bool(true));
        clock.value = AdvancedConfigValue::Bool(false);
        let mut scale = preference(
            "Scale",
            AdvancedConfigValue::Integer {
                value: 100,
                min: 100,
                max: 400,
            },
        );
        scale.value = AdvancedConfigValue::Integer {
            value: 150,
            min: 100,
            max: 400,
        };
        let stats = preference("ShowStats", AdvancedConfigValue::Bool(false));
        let controller = SettingsController::new(vec![clock, scale, stats]);
        assert!(controller.is_modified(0));
        assert!(controller.is_modified(1));
        assert!(!controller.is_modified(2));
        assert!(detail_text(&controller.settings[0]).ends_with(" · Default: On"));
        assert!(detail_text(&controller.settings[1]).ends_with(" · Default: 100"));
        assert!(!detail_text(&controller.settings[2]).contains("Default"));
    }

    #[test]
    fn the_pointer_marks_the_row_it_is_over_until_it_leaves_the_list() {
        let mut controller = SettingsController::new(
            ["ShowClock", "ShowStats", "ShowPortraits"]
                .map(|key| preference(key, AdvancedConfigValue::Bool(true)))
                .to_vec(),
        );
        controller.select_category(SettingsCategory::Display);
        let layout = controller.layout();
        let row = controller.row_rect(&layout, 2).unwrap();
        controller.pointer_move(GuiPoint::new((row.x + 40) as f32, (row.y + 4) as f32));
        assert_eq!(controller.row_emphasis(2), Some(RowEmphasis::Hovered));
        assert_eq!(controller.row_emphasis(0), Some(RowEmphasis::Selected));
        controller.pointer_move(GuiPoint::new(layout.panel.x as f32, layout.panel.y as f32));
        assert_eq!(controller.row_emphasis(2), None);
    }

    fn text_size() -> Setting {
        let choice = AdvancedConfigValue::Choice {
            value: "1".into(),
            choices: [("0", "Small"), ("1", "Medium"), ("2", "Large")]
                .into_iter()
                .map(
                    |(value, label)| crate::startup_options_advanced::AdvancedConfigChoice {
                        value: value.into(),
                        label: label.into(),
                    },
                )
                .collect(),
        };
        preference("TextSize", choice)
    }

    #[test]
    fn a_choice_list_opens_under_its_value_no_wider_than_it_needs_and_follows_the_pointer() {
        let mut controller = SettingsController::new(vec![text_size()]);
        controller.select_category(SettingsCategory::Display);
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Enter, false, false);
        let layout = controller.layout();
        let row = controller.row_rect(&layout, 0).unwrap();
        let value = value_rect(row);
        let list = controller.choice_rect();
        assert_eq!(list.y, value.y + value.h, "opens right under the value");
        assert_eq!(list.x + list.w, value.x + value.w, "aligned to the value");
        assert!(list.w < layout.list.w / 2, "a menu, not a page-wide panel");
        let first = GuiPoint::new((list.x + 8) as f32, (list.y + 6) as f32);
        controller.pointer_move(first);
        assert_eq!(controller.view.choice.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn moving_to_another_setting_or_page_clears_the_last_change_message() {
        let mut controller = SettingsController::new(vec![
            preference("ShowClock", AdvancedConfigValue::Bool(true)),
            preference("ShowStats", AdvancedConfigValue::Bool(true)),
        ]);
        controller.select_category(SettingsCategory::Display);
        controller.view.message = "Show clock · Applies now".into();
        controller.set_focus(SettingsFocus::Row(0));
        assert!(!controller.view.message.is_empty(), "same setting keeps it");
        controller.set_focus(SettingsFocus::Row(1));
        assert!(controller.view.message.is_empty());
        controller.view.message = "Show stats · Applies now".into();
        controller.select_category(SettingsCategory::Audio);
        assert!(controller.view.message.is_empty());
    }

    #[test]
    fn a_changed_row_carries_no_reset_link_and_delete_restores_its_default() {
        let percent = |value| AdvancedConfigValue::Integer {
            value,
            min: 0,
            max: 100,
        };
        let mut controller = SettingsController::new(vec![preference("Volume", percent(50))]);
        controller.select_category(SettingsCategory::Display);
        controller.settings[0].value = percent(80);
        controller.set_focus(SettingsFocus::Row(0));
        assert!(controller.row_actions(&controller.layout()).is_empty());
        assert_eq!(
            controller.edit_command("delete", false, false),
            vec![SettingsAction::Change(0, percent(50))]
        );
    }

    #[test]
    fn a_page_reset_names_what_it_resets_and_is_not_offered_when_nothing_changed() {
        let percent = |value| AdvancedConfigValue::Integer {
            value,
            min: 0,
            max: 100,
        };
        let mut volume = preference("Volume", percent(50));
        volume.value = percent(80);
        let mut controller = SettingsController::new(vec![
            volume,
            preference("ShowClock", AdvancedConfigValue::Bool(true)),
        ]);
        controller.select_category(SettingsCategory::Display);
        controller.set_focus(SettingsFocus::ResetCategory);
        controller.key(KeyCode::Enter, false, false);
        assert!(controller
            .confirmation_prompt()
            .is_some_and(|prompt| prompt.starts_with("Reset 1 setting on this page")));
        let layout = controller.layout();
        let [(_, confirm), _] = controller.confirmation_actions(&layout).unwrap();
        let centre = GuiPoint::new(
            (confirm.x + confirm.w / 2) as f32,
            (confirm.y + confirm.h / 2) as f32,
        );
        controller.pointer(centre, true);
        assert_eq!(
            controller.pointer(centre, false),
            vec![SettingsAction::Change(0, percent(50))]
        );
        assert!(controller.confirmation_prompt().is_none());

        controller.settings[0].value = percent(50);
        controller.set_focus(SettingsFocus::ResetCategory);
        controller.key(KeyCode::Enter, false, false);
        assert!(controller.confirmation_prompt().is_none());
        assert!(controller.view.message.contains("already at its default"));
    }

    #[test]
    fn delete_resets_the_focused_setting_that_differs_from_its_default() {
        let percent = |value| AdvancedConfigValue::Integer {
            value,
            min: 0,
            max: 100,
        };
        let mut volume = preference("Volume", percent(50));
        volume.value = percent(80);
        let mut controller = SettingsController::new(vec![
            volume,
            preference("ShowClock", AdvancedConfigValue::Bool(true)),
        ]);
        controller.select_category(SettingsCategory::Display);
        controller.set_focus(SettingsFocus::Row(0));
        assert_eq!(
            controller.edit_command("delete", false, false),
            vec![SettingsAction::Change(0, percent(50))]
        );
        controller.set_focus(SettingsFocus::Row(1));
        assert!(controller
            .edit_command("backspace", false, false)
            .is_empty());
    }

    #[test]
    fn the_header_holds_only_search_and_page_tabs_and_filters_join_the_footer() {
        for category in [SettingsCategory::Interface, SettingsCategory::Audio] {
            let mut controller = SettingsController::new(Vec::new());
            controller.select_category(category);
            let layout = controller.layout();
            let targets = controller.targets(&layout);
            for filter in [SettingsFocus::Modified, SettingsFocus::Advanced] {
                let (_, rect) = targets.iter().find(|(focus, _)| *focus == filter).unwrap();
                assert!(
                    rect.y >= layout.footer.y,
                    "{category:?} {filter:?} in the footer"
                );
            }
            let below_search = layout.list.y - (layout.search.y + layout.search.h);
            if category == SettingsCategory::Audio {
                assert!(below_search >= 28, "the Audio tabs keep their row");
            } else {
                assert!(below_search < 12, "no empty row between search and list");
            }
        }
    }

    #[test]
    fn audio_page_tabs_hold_an_icon_where_the_page_has_room() {
        for (width, height, icons) in [(640, 480, false), (1280, 720, true)] {
            let mut controller = SettingsController::new(Vec::new());
            controller.resize(width, height);
            controller.select_category(SettingsCategory::Audio);
            let layout = controller.layout();
            assert_eq!(
                controller.page_tab_icons(&layout),
                icons,
                "{width}x{height}"
            );
            let targets = controller.targets(&layout);
            let rect = |focus| targets.iter().find(|(f, _)| *f == focus).unwrap().1;
            let sound = rect(SettingsFocus::AudioPage(AudioPage::Sound));
            let voice = rect(SettingsFocus::AudioPage(AudioPage::Voice));
            assert!(voice.w >= if icons { 132 } else { 108 }, "{width}x{height}");
            assert!(sound.x + sound.w < voice.x);
            assert!(voice.x + voice.w <= layout.list.x + layout.list.w);
        }
    }

    #[test]
    fn audio_subpages_are_directly_clickable_without_changing_filters() {
        let mut controller = SettingsController::new(Vec::new());
        controller.select_category(SettingsCategory::Audio);
        let layout = controller.layout();
        let (_, tab) = controller
            .targets(&layout)
            .into_iter()
            .find(|(focus, _)| *focus == SettingsFocus::AudioPage(AudioPage::Voice))
            .unwrap();
        let voice = GuiPoint::new((tab.x + tab.w / 2) as f32, (tab.y + tab.h / 2) as f32);
        controller.pointer(voice, true);
        controller.pointer(voice, false);
        assert_eq!(controller.audio_page, AudioPage::Voice);
        assert!(!controller.modified_only);
    }

    #[test]
    fn opening_microphone_test_waits_for_an_explicit_record_action() {
        let mut controller = SettingsController::new(Vec::new());
        controller.select_category(SettingsCategory::Audio);
        controller.audio_page = AudioPage::Voice;
        controller.set_focus(SettingsFocus::TestMicrophone);
        assert!(
            controller.key(KeyCode::Enter, false, false).is_empty(),
            "opening the test panel must not start recording"
        );
        assert!(
            controller
                .key(KeyCode::Escape, false, false)
                .iter()
                .all(|action| *action != SettingsAction::Close),
            "Escape returns to Voice chat before leaving settings"
        );
        controller.key(KeyCode::Enter, false, false);
        for expected in [
            SettingsFocus::RefreshDevices,
            SettingsFocus::CloseMicrophoneTest,
            SettingsFocus::RecordMicrophone,
        ] {
            controller.key(KeyCode::Tab, false, false);
            assert_eq!(controller.view.focus, expected);
        }
        controller.focus_search();
        assert_eq!(controller.view.focus, SettingsFocus::RecordMicrophone);
        assert_eq!(
            controller.key(KeyCode::Enter, false, false),
            vec![SettingsAction::TestMicrophone]
        );
        assert_eq!(
            controller.key(KeyCode::Escape, false, false),
            vec![SettingsAction::CancelMicrophoneTest]
        );
        assert!(!controller.view.microphone_test_open);
        assert_eq!(controller.view.focus, SettingsFocus::TestMicrophone);
        controller.key(KeyCode::Enter, false, false);
        assert_eq!(
            controller.key(KeyCode::Tab, false, true),
            vec![SettingsAction::CancelMicrophoneTest]
        );
        assert_eq!(controller.category, SettingsCategory::Controls);
    }

    #[test]
    fn focusing_search_finishes_the_previous_edit_interaction() {
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Network", "Comment"),
            label: "Comment".into(),
            keywords: String::new(),
            category: SettingsCategory::Interface,
            advanced: false,
            value: AdvancedConfigValue::Text("old".into()),
            default: AdvancedConfigValue::Text(String::new()),
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Interface;
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Enter, false, false);
        controller.text("unfinished");
        controller.focus_search();
        controller.text("volume");
        assert_eq!(controller.query, "volume");
        assert_eq!(controller.settings[0].value.serialized(), "old");
    }

    #[test]
    fn category_reset_does_not_reset_hidden_advanced_preferences() {
        let mut controller = SettingsController::new(
            [false, true]
                .into_iter()
                .map(|advanced| Setting {
                    id: SettingId::new("Sound", if advanced { "Internal" } else { "Music" }),
                    label: "Preference".into(),
                    keywords: String::new(),
                    category: SettingsCategory::Audio,
                    advanced,
                    value: AdvancedConfigValue::Bool(false),
                    default: AdvancedConfigValue::Bool(true),
                    details: Default::default(),
                })
                .collect(),
        );
        controller.category = SettingsCategory::Audio;
        controller.set_focus(SettingsFocus::ResetCategory);
        controller.key(KeyCode::Enter, false, false);
        let actions = controller.key(KeyCode::Enter, false, false);
        assert_eq!(
            actions,
            vec![SettingsAction::Change(0, AdvancedConfigValue::Bool(true))]
        );
    }
    #[test]
    fn directional_navigation_reaches_microphone_tools_and_returns_to_categories() {
        let mut controller = voice_page();
        controller.set_focus(SettingsFocus::Row(1));
        for _ in 0..8 {
            if controller.view.focus == SettingsFocus::TestMicrophone {
                break;
            }
            controller.key(KeyCode::Down, false, false);
        }
        assert_eq!(controller.view.focus, SettingsFocus::TestMicrophone);
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Up, false, false);
        assert_eq!(
            controller.view.focus,
            SettingsFocus::AudioPage(AudioPage::Voice)
        );
    }
    #[test]
    fn clicking_a_volume_slider_emits_the_value_at_the_pointer() {
        let value = AdvancedConfigValue::Integer {
            value: 40,
            min: 0,
            max: 100,
        };
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Sound", "MusicVolume"),
            label: "Music volume".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        let layout = SettingsLayout::new(800, 600);
        let row = controller.row_rect(&layout, 0).unwrap();
        let track = slider_rect(row);
        let actions = controller.pointer(
            GuiPoint::new((track.x + track.w - 24) as f32, (track.y + 8) as f32),
            true,
        );
        assert!(
            matches!(actions.as_slice(), [SettingsAction::Change(0, value)] if value.serialized() == "100")
        );
    }
    #[test]
    fn wheel_scroll_keeps_its_position_when_the_focused_row_leaves_the_view() {
        let mut controller = SettingsController::new(
            (0..20)
                .map(|i| Setting {
                    id: SettingId::new("Sound", i.to_string()),
                    label: format!("Setting {i}"),
                    keywords: String::new(),
                    category: SettingsCategory::Audio,
                    advanced: false,
                    value: AdvancedConfigValue::Bool(true),
                    default: AdvancedConfigValue::Bool(true),
                    details: Default::default(),
                })
                .collect(),
        );
        controller.category = SettingsCategory::Audio;
        controller.set_focus(SettingsFocus::Row(0));
        controller.scroll(5);
        controller.resize(800, 600);
        assert_eq!(controller.view.scroll, 5);
    }
    #[test]
    fn device_choices_can_be_browsed_and_cancelled_before_applying() {
        use crate::startup_options_advanced::AdvancedConfigChoice;
        let value = AdvancedConfigValue::Choice {
            value: "default".into(),
            choices: vec![
                AdvancedConfigChoice {
                    value: "default".into(),
                    label: "System default".into(),
                },
                AdvancedConfigChoice {
                    value: "usb".into(),
                    label: "USB microphone".into(),
                },
            ],
        };
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Voice", "InputDevice"),
            label: "Microphone".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.set_focus(SettingsFocus::Row(0));
        assert!(controller.key(KeyCode::Enter, false, false).is_empty());
        assert!(controller.key(KeyCode::Down, false, false).is_empty());
        assert!(controller.key(KeyCode::Escape, false, false).is_empty());
        assert_eq!(controller.settings[0].value.serialized(), "default");
        controller.key(KeyCode::Enter, false, false);
        controller.key(KeyCode::Down, false, false);
        let actions = controller.key(KeyCode::Enter, false, false);
        assert!(
            matches!(actions.as_slice(), [SettingsAction::Change(0, value)] if value.serialized() == "usb")
        );
    }
    #[test]
    fn leaving_a_setting_editor_commits_valid_input_and_retains_invalid_input() {
        let value = AdvancedConfigValue::Integer {
            value: 40,
            min: 0,
            max: 100,
        };
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Sound", "MusicVolume"),
            label: "Music volume".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Enter, false, false);
        controller.text("75");
        assert_eq!(controller.key(KeyCode::Tab, false, false).len(), 1);
        assert_eq!(controller.view.focus, SettingsFocus::Advanced);
        controller.set_focus(SettingsFocus::Row(0));
        controller.key(KeyCode::Enter, false, false);
        controller.text("999");
        assert!(controller.key(KeyCode::Tab, false, false).is_empty());
        assert!(controller.editing());
        assert!(controller.view.message.contains("between"));
    }
    #[test]
    fn editing_can_be_cancelled_and_all_footer_actions_are_keyboard_reachable() {
        let value = AdvancedConfigValue::Integer {
            value: 40,
            min: 0,
            max: 100,
        };
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Sound", "MusicVolume"),
            label: "Music volume".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.view.focus = SettingsFocus::Row(0);
        controller.key(KeyCode::Enter, false, false);
        controller.text("75");
        assert!(controller.key(KeyCode::Escape, false, false).is_empty());
        assert_eq!(controller.settings[0].value.serialized(), "40");
        controller.key(KeyCode::Enter, false, false);
        controller.text("75");
        assert_eq!(
            controller.key(KeyCode::Enter, false, false),
            vec![SettingsAction::Change(
                0,
                AdvancedConfigValue::Integer {
                    value: 75,
                    min: 0,
                    max: 100
                }
            )]
        );
        // Reset is offered once the row differs from its default.
        controller.settings[0].value = AdvancedConfigValue::Integer {
            value: 75,
            min: 0,
            max: 100,
        };
        let mut visited = Vec::new();
        for _ in 0..30 {
            controller.key(KeyCode::Tab, false, false);
            visited.push(controller.view.focus);
        }
        for focus in [
            SettingsFocus::Search,
            SettingsFocus::Modified,
            SettingsFocus::Advanced,
            SettingsFocus::ResetCategory,
            SettingsFocus::Close,
        ] {
            assert!(visited.contains(&focus), "unreachable: {focus:?}");
        }
    }
    #[test]
    fn keyboard_navigation_edits_a_setting_and_returns_without_gameplay_input() {
        let value = AdvancedConfigValue::Bool(true);
        let mut controller = SettingsController::new(vec![Setting {
            id: SettingId::new("Sound", "Music"),
            label: "Music".into(),
            keywords: String::new(),
            category: SettingsCategory::Audio,
            advanced: false,
            value: value.clone(),
            default: value,
            details: Default::default(),
        }]);
        controller.category = SettingsCategory::Audio;
        controller.view.focus = SettingsFocus::Search;
        controller.key(KeyCode::Down, false, false);
        assert_eq!(controller.view.focus, SettingsFocus::Row(0));
        assert_eq!(
            controller.key(KeyCode::Enter, false, false),
            vec![SettingsAction::Change(0, AdvancedConfigValue::Bool(false))]
        );
        assert_eq!(
            controller.key(KeyCode::Escape, false, false),
            vec![SettingsAction::Close]
        );
    }
    #[test]
    fn settings_layout_keeps_search_rows_and_footer_usable_in_compact_windows() {
        for (width, height) in [(640, 480), (800, 600), (1280, 720)] {
            let layout = SettingsLayout::new(width, height);
            assert!(layout.list.w >= 360 && layout.list.h >= 150);
            assert!(layout.search.y + layout.search.h <= layout.list.y);
            assert!(layout.list.y + layout.list.h <= layout.footer.y);
            for rect in [layout.panel, layout.search, layout.list, layout.footer] {
                assert!(
                    rect.x >= 0
                        && rect.y >= 0
                        && rect.x + rect.w <= width
                        && rect.y + rect.h <= height
                );
            }
        }
    }
}
