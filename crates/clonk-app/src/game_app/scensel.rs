//! `impl GameApp` — scenario & definition selectors methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

fn add_existing_portrait_location(
    locations: &mut Vec<clonk_frontend::startup_portraitsel::PortraitLocation>,
    label: &str,
    path: PathBuf,
) {
    if !path.is_dir() {
        return;
    }
    let duplicate = locations.iter().any(|location| {
        location.path == path
            || matches!(
                (fs::canonicalize(&location.path), fs::canonicalize(&path)),
                (Ok(existing), Ok(candidate)) if existing == candidate
            )
    });
    if !duplicate {
        locations.push(clonk_frontend::startup_portraitsel::PortraitLocation::new(
            label, path,
        ));
    }
}

fn add_optional_portrait_locations(
    locations: &mut Vec<clonk_frontend::startup_portraitsel::PortraitLocation>,
    platform_locations: impl IntoIterator<Item = (&'static str, Option<PathBuf>)>,
    home: Option<(&'static str, PathBuf)>,
    add_desktop_from_home: bool,
) {
    platform_locations
        .into_iter()
        .filter_map(|(label, path)| path.map(|path| (label, path)))
        .for_each(|(label, path)| add_existing_portrait_location(locations, label, path));
    if let Some((label, home)) = home {
        add_existing_portrait_location(locations, label, home.clone());
        if add_desktop_from_home {
            add_existing_portrait_location(locations, "Desktop", home.join("Desktop"));
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_special_folder(csidl: u32) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt as _;
    use windows::Win32::UI::Shell::SHGetSpecialFolderPathW;

    let mut path = [0_u16; 260];
    // SAFETY: `path` is the fixed-size writable buffer required by
    // SHGetSpecialFolderPathW, and FALSE asks Windows not to create the folder.
    let found = unsafe { SHGetSpecialFolderPathW(None, &mut path, csidl as i32, false) }.as_bool();
    found
        .then(|| {
            let len = path
                .iter()
                .position(|component| *component == 0)
                .unwrap_or(path.len());
            PathBuf::from(OsString::from_wide(&path[..len]))
        })
        .filter(|path| !path.as_os_str().is_empty())
}

fn startup_player_portrait_locations(
    paths: &AppPaths,
) -> Vec<clonk_frontend::startup_portraitsel::PortraitLocation> {
    let mut locations = vec![clonk_frontend::startup_portraitsel::PortraitLocation::new(
        "LegacyClonk User Path",
        paths.user_data_dir(),
    )];
    let mut program_path = paths.install_root().as_os_str().to_os_string();
    if !program_path
        .to_string_lossy()
        .ends_with(std::path::MAIN_SEPARATOR)
    {
        program_path.push(std::path::MAIN_SEPARATOR_STR);
    }
    add_existing_portrait_location(
        &mut locations,
        "LegacyClonk Program Directory",
        PathBuf::from(program_path),
    );

    // C4PortraitSelDlg::C4PortraitSelDlg, C4FileSelDlg.cpp:541-556: append the
    // Windows shell folders first, HOME on every platform, and HOME/Desktop
    // only outside Windows.
    #[cfg(target_os = "windows")]
    let platform_locations = {
        use windows::Win32::UI::Shell::{CSIDL_DESKTOPDIRECTORY, CSIDL_MYPICTURES, CSIDL_PERSONAL};

        [
            ("My Documents", windows_special_folder(CSIDL_PERSONAL)),
            ("My Pictures", windows_special_folder(CSIDL_MYPICTURES)),
            ("Desktop", windows_special_folder(CSIDL_DESKTOPDIRECTORY)),
        ]
    };
    #[cfg(not(target_os = "windows"))]
    let platform_locations: [(&str, Option<PathBuf>); 0] = [];
    #[cfg(target_os = "macos")]
    let home_label = "Home";
    #[cfg(not(target_os = "macos"))]
    let home_label = "Home Folder";
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| (home_label, path));
    add_optional_portrait_locations(
        &mut locations,
        platform_locations,
        home,
        !cfg!(target_os = "windows"),
    );

    locations
}

impl GameApp {
    pub(crate) fn set_scensel_dialog_focus(&mut self, focus: ScenselDialogFocus) {
        self.menu_state.set_dialog_focus(focus);
        if focus != ScenselDialogFocus::Options {
            self.scenario_game_options.set_focused_button(None);
        }
    }

    pub(crate) fn scensel_focus_snapshot(&self) -> ScenselFocusSnapshot {
        ScenselFocusSnapshot {
            dialog: self.menu_state.dialog_focus(),
            option: self.scenario_game_options.focused_button(),
        }
    }

    pub(crate) fn restore_scensel_focus(&mut self, snapshot: ScenselFocusSnapshot) {
        self.set_scensel_dialog_focus(snapshot.dialog);
        if snapshot.dialog == ScenselDialogFocus::Options {
            self.scenario_game_options
                .set_focused_button(snapshot.option);
        }
    }

    pub(crate) fn restore_scensel_rename_pointer_focus(&mut self) {
        let focus = self.scensel_rename_pointer_focus;
        if self.mode == AppMode::Menu && self.startup_view == StartupView::ScenarioBrowser {
            if let Some(focus) = focus {
                self.restore_scensel_focus(focus);
            }
        }
    }

    pub(crate) fn activate_scensel_after_gamepad_low_rename_abort(
        &mut self,
    ) -> Result<(), EngineError> {
        if !self.abort_scenario_rename() {
            return Ok(());
        }
        if self.menu_state.current_map().is_some() {
            self.start_selected_map_scenario_from_ui()
        } else {
            self.handle_menu_input(|menu| menu.menu().handle_key_down(KeyCode::Enter))?;
            self.handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Enter))
        }
    }

    fn focus_scensel_option_edge(&mut self, backwards: bool) {
        let button = if backwards {
            self.scenario_game_options
                .context()
                .buttons()
                .last()
                .copied()
        } else {
            self.scenario_game_options
                .context()
                .buttons()
                .first()
                .copied()
        };
        self.menu_state
            .set_dialog_focus(ScenselDialogFocus::Options);
        self.scenario_game_options.set_focused_button(button);
    }

    /// Recursive `C4GUI::Dialog::AdvanceFocus` around the game-option child.
    /// Search -> List -> Back -> optional Definitions -> Options -> Open.
    pub(crate) fn advance_scensel_dialog_focus(&mut self, backwards: bool) {
        let focus = if self.scenario_game_options.focused_button().is_some() {
            ScenselDialogFocus::Options
        } else {
            self.menu_state.dialog_focus()
        };
        let definitions = self.menu_state.definition_checkbox_enabled;
        match (focus, backwards) {
            (ScenselDialogFocus::Search, false) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::List)
            }
            (ScenselDialogFocus::Search, true) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Open)
            }
            (ScenselDialogFocus::List, false) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Back)
            }
            (ScenselDialogFocus::List, true) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Search)
            }
            (ScenselDialogFocus::Back, false) if definitions => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Definitions)
            }
            (ScenselDialogFocus::Back, false) => self.focus_scensel_option_edge(false),
            (ScenselDialogFocus::Back, true) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::List)
            }
            (ScenselDialogFocus::Definitions, false) => self.focus_scensel_option_edge(false),
            (ScenselDialogFocus::Definitions, true) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Back)
            }
            (ScenselDialogFocus::Options, false) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Open)
            }
            (ScenselDialogFocus::Options, true) if definitions => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Definitions)
            }
            (ScenselDialogFocus::Options, true) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Back)
            }
            (ScenselDialogFocus::Open, false) => {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Search)
            }
            (ScenselDialogFocus::Open, true) => self.focus_scensel_option_edge(true),
        }
    }

    pub(crate) fn scensel_search_char_pos(
        &self,
        point: GuiPoint,
        require_inside: bool,
    ) -> Option<usize> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::ScenarioBrowser
            || self.menu_state.current_map().is_some()
        {
            return None;
        }
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
            fonts,
        );
        let edit = layout.search_edit;
        let inside = point.x >= edit.x as f32
            && point.x < (edit.x + edit.w) as f32
            && point.y >= edit.y as f32
            && point.y < (edit.y + edit.h) as f32;
        if require_inside && !inside {
            return None;
        }
        let control_x =
            point.x as i32 - (edit.x + 4) + self.menu_state.search_edit.horizontal_scroll;
        let text = self.menu_state.search_edit.text();
        let mut last_width = 0;
        for (start, character) in text.char_indices() {
            let end = start + character.len_utf8();
            let width = fonts.text.measure(&text[..end], false).0;
            if width - (width - last_width) / 2 >= control_x {
                return Some(start);
            }
            last_width = width;
        }
        Some(text.len())
    }

    pub(crate) fn handle_scensel_search_pointer_down(&mut self, point: GuiPoint) -> bool {
        let Some(position) = self.scensel_search_char_pos(point, true) else {
            return false;
        };
        self.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        self.menu_state
            .search_edit
            .begin_pointer_selection(position);
        true
    }

    pub(crate) fn handle_scensel_search_clear_pointer_down(
        &mut self,
        point: GuiPoint,
    ) -> Result<bool, EngineError> {
        if self.menu_state.search_text().is_empty()
            || self.mode != AppMode::Menu
            || self.startup_view != StartupView::ScenarioBrowser
            || self.menu_state.current_map().is_some()
        {
            return Ok(false);
        }
        let Some(fonts) = self.assets.clonk_fonts.as_deref() else {
            return Ok(false);
        };
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
            fonts,
        );
        let clear = clonk_frontend::startup_scensel::search_clear_button_bounds(&layout);
        let inside = point.x >= clear.x as f32
            && point.x < (clear.x + clear.w) as f32
            && point.y >= clear.y as f32
            && point.y < (clear.y + clear.h) as f32;
        if !inside {
            return Ok(false);
        }
        self.set_scensel_dialog_focus(ScenselDialogFocus::Search);
        self.menu_state.set_search_text("");
        self.submit_scenario_search()?;
        Ok(true)
    }

    /// C4GUI middle-down repositions the caret on every platform, inserts
    /// the raw PRIMARY selection only where the platform supplies one, and
    /// neither focuses the edit nor starts a selection drag.
    pub(crate) fn handle_scensel_search_middle_down(
        &mut self,
        point: GuiPoint,
        primary_selection: Option<&str>,
    ) -> bool {
        let Some(position) = self.scensel_search_char_pos(point, true) else {
            return false;
        };
        let (previous, current, inserted) = {
            let edit = &mut self.menu_state.search_edit;
            let previous = edit.caret;
            edit.anchor = position;
            edit.caret = position;
            edit.drag_anchor = position;
            let inserted = primary_selection.is_some_and(|text| edit.insert_raw_text(text));
            (previous, edit.caret, inserted)
        };
        if inserted || previous != current {
            let Some(fonts) = self.assets.clonk_fonts.clone() else {
                return true;
            };
            let layout = clonk_frontend::startup_scensel::scen_sel_layout(
                self.graphics.surface().width() as i32,
                self.graphics.surface().height() as i32,
                &fonts,
            );
            let edit = &mut self.menu_state.search_edit;
            let cursor_x = fonts.text.measure(&edit.text[..edit.caret], false).0;
            let cursor_half = fonts.text.measure("\u{a6}", false).0 / 2;
            edit.scroll_cursor_in_view(cursor_x, layout.search_edit.w - 8, cursor_half);
        }
        true
    }

    pub(crate) fn handle_scensel_search_pointer_move(&mut self, point: GuiPoint) -> bool {
        if !self.menu_state.search_edit.dragging {
            return false;
        }
        if let Some(position) = self.scensel_search_char_pos(point, false) {
            self.menu_state.search_edit.drag_pointer_selection(position);
        }
        true
    }

    pub(crate) fn handle_scensel_search_pointer_up(&mut self, point: GuiPoint) -> bool {
        if !self.menu_state.search_edit.dragging {
            return false;
        }
        let position = self
            .scensel_search_char_pos(point, false)
            .unwrap_or(self.menu_state.search_edit.caret());
        self.menu_state.search_edit.end_pointer_selection(position);
        let inside = self.scensel_search_char_pos(point, true).is_some();
        let now = Instant::now();
        let double_click = inside
            && self
                .scensel_search_last_click
                .is_some_and(|last| now.duration_since(last) < Duration::from_millis(500));
        if double_click {
            self.menu_state.search_edit.select_word_at(position);
            self.scensel_search_last_click = None;
        } else {
            self.scensel_search_last_click = inside.then_some(now);
        }
        true
    }

    fn scensel_rename_char_pos(&self, point: GuiPoint, require_inside: bool) -> Option<usize> {
        if self.mode != AppMode::Menu || self.startup_view != StartupView::ScenarioBrowser {
            return None;
        }
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let book = self.assets.book_fonts.as_deref()?;
        let rename = self.menu_state.rename_edit.as_ref()?;
        let row = self
            .menu_state
            .visible_entries()
            .iter()
            .position(|entry| entry.identifier == rename.identifier)?;
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
            fonts,
        );
        let item_height = clonk_frontend::startup_scensel::scen_list_item_height(&book.text);
        let pitch = item_height + 1;
        let edit_x = layout.list.x + 3 + item_height + 2;
        let edit_y = layout.list.y + 3 - self.menu_state.scenario_list_scroll()
            + i32::try_from(row).unwrap_or(i32::MAX).saturating_mul(pitch)
            + 2;
        let edit_w = layout.list.w - 6 - 16 - item_height - 4;
        let edit_h = fonts.text.line_height;
        let inside = point.x >= edit_x as f32
            && point.x < (edit_x + edit_w) as f32
            && point.y >= edit_y as f32
            && point.y < (edit_y + edit_h) as f32;
        if require_inside && !inside {
            return None;
        }
        let control_x = point.x as i32 - (edit_x + 2) + rename.edit.horizontal_scroll();
        let text = rename.edit.text();
        let mut last_width = 0;
        for (start, character) in text.char_indices() {
            let end = start + character.len_utf8();
            let width = fonts.text.measure(&text[..end], false).0;
            if width - (width - last_width) / 2 >= control_x {
                return Some(start);
            }
            last_width = width;
        }
        Some(text.len())
    }

    pub(crate) fn handle_scensel_rename_pointer_down(&mut self, point: GuiPoint) -> bool {
        let Some(position) = self.scensel_rename_char_pos(point, true) else {
            return false;
        };
        if let Some(rename) = self.menu_state.rename_edit.as_mut() {
            rename.edit.begin_pointer_selection(position);
        }
        true
    }

    pub(crate) fn handle_scensel_rename_pointer_move(&mut self, point: GuiPoint) -> bool {
        if !self
            .menu_state
            .rename_edit
            .as_ref()
            .is_some_and(|rename| rename.edit.is_dragging())
        {
            return false;
        }
        if let Some(position) = self.scensel_rename_char_pos(point, false) {
            if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                rename.edit.drag_pointer_selection(position);
            }
        }
        true
    }

    pub(crate) fn handle_scensel_rename_pointer_up(&mut self, point: GuiPoint) -> bool {
        if !self
            .menu_state
            .rename_edit
            .as_ref()
            .is_some_and(|rename| rename.edit.is_dragging())
        {
            return false;
        }
        let position = self
            .scensel_rename_char_pos(point, false)
            .or_else(|| {
                self.menu_state
                    .rename_edit
                    .as_ref()
                    .map(|rename| rename.edit.caret())
            })
            .unwrap_or(0);
        let inside = self.scensel_rename_char_pos(point, true).is_some();
        let now = Instant::now();
        if let Some(rename) = self.menu_state.rename_edit.as_mut() {
            rename.edit.end_pointer_selection(position);
            let double_click = inside
                && rename
                    .last_click
                    .is_some_and(|last| now.duration_since(last) < Duration::from_millis(500));
            if double_click {
                rename.edit.select_word_at(position);
                rename.last_click = None;
            } else {
                rename.last_click = inside.then_some(now);
            }
        }
        true
    }

    fn scensel_scrollbar_spec(
        &self,
        target: ScenselScrollbarTarget,
    ) -> Option<ScenselScrollbarSpec> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::ScenarioBrowser
            || self.menu_state.current_map().is_some()
        {
            return None;
        }
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let book_fonts = self.assets.book_fonts.as_deref()?;
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
            fonts,
        );
        let (rect, max_scroll, offset) = match target {
            ScenselScrollbarTarget::List => {
                let item_height =
                    clonk_frontend::startup_scensel::scen_list_item_height(&book_fonts.text);
                let max_scroll = self
                    .menu_state
                    .scenario_list_max_scroll(layout.list.h - 6, item_height + 1);
                (
                    layout.list_scrollbar,
                    max_scroll,
                    self.menu_state.scenario_list_scroll(),
                )
            }
            ScenselScrollbarTarget::Description => {
                let info = scensel_selection_info(&self.menu_state);
                let metrics = clonk_frontend::startup_scensel::selection_info_scroll_metrics(
                    &layout, book_fonts, &info,
                );
                (
                    clonk_frontend::startup_scensel::selection_info_scrollbar_rect(&layout),
                    metrics.max_scroll,
                    self.menu_state.selection_info_scroll,
                )
            }
        };
        (max_scroll > 0).then_some(ScenselScrollbarSpec {
            target,
            rect,
            max_scroll,
            offset: offset.clamp(0, max_scroll),
        })
    }

    fn scensel_scrollbar_spec_at(&self, point: GuiPoint) -> Option<ScenselScrollbarSpec> {
        [
            ScenselScrollbarTarget::List,
            ScenselScrollbarTarget::Description,
        ]
        .into_iter()
        .filter_map(|target| self.scensel_scrollbar_spec(target))
        .find(|spec| {
            point.x >= spec.rect.x as f32
                && point.x < (spec.rect.x + spec.rect.w) as f32
                && point.y >= spec.rect.y as f32
                && point.y < (spec.rect.y + spec.rect.h) as f32
        })
    }

    fn set_scensel_scrollbar_offset(
        &mut self,
        target: ScenselScrollbarTarget,
        offset: i32,
        max_scroll: i32,
    ) -> bool {
        let offset = offset.clamp(0, max_scroll);
        match target {
            ScenselScrollbarTarget::List => {
                self.menu_state.list_scroll_selection = Some(self.menu_state.menu.selected_index());
                if self.menu_state.scenario_list_scroll == offset {
                    false
                } else {
                    self.menu_state.scenario_list_scroll = offset;
                    true
                }
            }
            ScenselScrollbarTarget::Description => {
                if self.menu_state.selection_info_scroll == offset {
                    false
                } else {
                    self.menu_state.selection_info_scroll = offset;
                    true
                }
            }
        }
    }

    fn set_scensel_scrollbar_pin(&mut self, spec: ScenselScrollbarSpec, pin: i32) -> bool {
        let Some(offset) = scensel_scrollbar_offset_from_pin(pin, spec.max_scroll, spec.rect.h)
        else {
            return false;
        };
        self.set_scensel_scrollbar_offset(spec.target, offset, spec.max_scroll)
    }

    pub(crate) fn handle_scensel_scrollbar_down(&mut self, point: GuiPoint) -> bool {
        if self.scenario_selector_discovery.is_some() {
            return false;
        }
        let Some(spec) = self.scensel_scrollbar_spec_at(point) else {
            return false;
        };
        let Some(current_pin) =
            scensel_scrollbar_pin_from_offset(spec.offset, spec.max_scroll, spec.rect.h)
        else {
            return true;
        };
        let local_y = point.y as i32 - spec.rect.y;
        let (kind, pin) = if local_y < SCENSEL_SCROLLBAR_PART {
            (ScenselScrollbarInteractionKind::Arrow(-1), current_pin)
        } else if local_y >= spec.rect.h - SCENSEL_SCROLLBAR_PART {
            (ScenselScrollbarInteractionKind::Arrow(1), current_pin)
        } else {
            let Some(pin) = scensel_scrollbar_jump_pin(local_y, spec.rect.h) else {
                return true;
            };
            (ScenselScrollbarInteractionKind::Dragging, pin)
        };
        self.menu_state.scrollbar_interaction = Some(ScenselScrollbarInteraction {
            target: spec.target,
            kind,
            pin,
        });
        if kind == ScenselScrollbarInteractionKind::Dragging {
            self.set_scensel_scrollbar_pin(spec, pin);
            self.play_ui_sound("Command");
        } else {
            self.play_ui_sound("ArrowHit");
        }
        true
    }

    pub(crate) fn handle_scensel_scrollbar_move(&mut self, point: GuiPoint) -> bool {
        if self.scenario_selector_discovery.is_some() {
            self.menu_state.scrollbar_interaction = None;
            return false;
        }
        let Some(mut interaction) = self.menu_state.scrollbar_interaction else {
            return false;
        };
        let Some(spec) = self.scensel_scrollbar_spec(interaction.target) else {
            self.menu_state.scrollbar_interaction = None;
            return true;
        };
        match interaction.kind {
            ScenselScrollbarInteractionKind::Dragging => {
                if let Some(pin) =
                    scensel_scrollbar_jump_pin(point.y as i32 - spec.rect.y, spec.rect.h)
                {
                    interaction.pin = pin;
                    self.menu_state.scrollbar_interaction = Some(interaction);
                    self.set_scensel_scrollbar_pin(spec, pin);
                }
            }
            ScenselScrollbarInteractionKind::Arrow(_) => {
                let inside_x =
                    point.x >= spec.rect.x as f32 && point.x < (spec.rect.x + spec.rect.w) as f32;
                let local_y = point.y as i32 - spec.rect.y;
                interaction.kind = if inside_x && (0..SCENSEL_SCROLLBAR_PART).contains(&local_y) {
                    ScenselScrollbarInteractionKind::Arrow(-1)
                } else if inside_x
                    && local_y >= spec.rect.h - SCENSEL_SCROLLBAR_PART
                    && local_y < spec.rect.h
                {
                    ScenselScrollbarInteractionKind::Arrow(1)
                } else {
                    self.menu_state.scrollbar_interaction = None;
                    return true;
                };
                self.menu_state.scrollbar_interaction = Some(interaction);
            }
        }
        true
    }

    pub(crate) fn handle_scensel_scrollbar_up(&mut self, point: GuiPoint) -> bool {
        if self.scenario_selector_discovery.is_some() {
            self.menu_state.scrollbar_interaction = None;
            return false;
        }
        let Some(interaction) = self.menu_state.scrollbar_interaction.take() else {
            return false;
        };
        if interaction.kind == ScenselScrollbarInteractionKind::Dragging {
            if let Some(spec) = self.scensel_scrollbar_spec(interaction.target) {
                if let Some(pin) =
                    scensel_scrollbar_jump_pin(point.y as i32 - spec.rect.y, spec.rect.h)
                {
                    self.set_scensel_scrollbar_pin(spec, pin);
                }
            }
        }
        true
    }

    pub(crate) fn tick_scensel_scrollbar_arrow(&mut self) -> bool {
        let Some(mut interaction) = self.menu_state.scrollbar_interaction else {
            return false;
        };
        let ScenselScrollbarInteractionKind::Arrow(direction) = interaction.kind else {
            return false;
        };
        let Some(spec) = self.scensel_scrollbar_spec(interaction.target) else {
            self.menu_state.scrollbar_interaction = None;
            return false;
        };
        let Some(travel) = scensel_scrollbar_pin_travel(spec.rect.h) else {
            return false;
        };
        let pin = interaction.pin.saturating_add(direction).clamp(0, travel);
        if pin == interaction.pin {
            return false;
        }
        interaction.pin = pin;
        self.menu_state.scrollbar_interaction = Some(interaction);
        self.set_scensel_scrollbar_pin(spec, pin)
    }

    pub(crate) fn handle_scensel_list_navigation_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::ScenarioBrowser
            || !matches!(
                key,
                VirtualKeyCode::ArrowUp
                    | VirtualKeyCode::ArrowDown
                    | VirtualKeyCode::Home
                    | VirtualKeyCode::End
                    | VirtualKeyCode::PageUp
                    | VirtualKeyCode::PageDown
            )
        {
            return Ok(false);
        }
        if state == ElementState::Released {
            return Ok(true);
        }
        if self.menu_state.current_map().is_some() {
            return Ok(true);
        }
        let Some(fonts) = self.assets.clonk_fonts.as_deref() else {
            return Ok(true);
        };
        let Some(book_fonts) = self.assets.book_fonts.as_deref() else {
            return Ok(true);
        };
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
            fonts,
        );
        let item_height = clonk_frontend::startup_scensel::scen_list_item_height(&book_fonts.text);
        let viewport_height = layout.list.h - 6;
        self.handle_menu_input(|menu| match key {
            VirtualKeyCode::ArrowUp => menu.move_list_selection_clamped(-1),
            VirtualKeyCode::ArrowDown => menu.move_list_selection_clamped(1),
            VirtualKeyCode::Home => menu.select_list_home(),
            VirtualKeyCode::End => menu.select_list_end(),
            VirtualKeyCode::PageUp => {
                menu.page_list_selection(-1, viewport_height, item_height + 1, item_height)
            }
            VirtualKeyCode::PageDown => {
                menu.page_list_selection(1, viewport_height, item_height + 1, item_height)
            }
            _ => Vec::new(),
        })?;
        Ok(true)
    }

    pub(crate) fn reload_scenario_selector(
        &mut self,
        selected_identifier: Option<&str>,
        select_first_when_missing: bool,
        apply_live_search: bool,
    ) -> Result<(), EngineError> {
        self.cancel_scenario_selector_discovery();
        self.menu_state.scrollbar_interaction = None;
        let selected_identifier = selected_identifier.map(str::to_string);
        let Some(paths) = self.app_paths.clone() else {
            let entries = self
                .menu_state
                .stack
                .first()
                .map(|layer| layer.entries.clone())
                .unwrap_or_default();
            return self.apply_scenario_selector_entries(
                entries,
                selected_identifier,
                select_first_when_missing,
                apply_live_search,
                None,
            );
        };

        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.scenario_selector_discovery = Some(ScenarioSelectorDiscoveryState {
            receiver,
            cancel: Arc::clone(&cancel),
            progress_percent: 0,
            selected_identifier,
            select_first_when_missing,
            apply_live_search,
            retained_title: None,
        });
        thread::spawn(move || {
            let mut last_percent = 0_u8;
            let entries = load_frontend_scenarios_from_paths_with_progress(&paths, |percent| {
                if cancel.load(AtomicOrdering::Relaxed) {
                    return false;
                }
                if percent != last_percent {
                    last_percent = percent;
                    if sender
                        .send(ScenarioSelectorDiscoveryEvent::Progress(percent))
                        .is_err()
                    {
                        return false;
                    }
                }
                true
            });
            if let Some(entries) = entries {
                let _ = sender.send(ScenarioSelectorDiscoveryEvent::Finished(entries));
            }
        });
        Ok(())
    }

    fn apply_scenario_selector_entries(
        &mut self,
        mut entries: Vec<FrontendScenario>,
        selected_identifier: Option<String>,
        select_first_when_missing: bool,
        apply_live_search: bool,
        retained_title: Option<(String, String)>,
    ) -> Result<(), EngineError> {
        if let Some((identifier, title)) = retained_title {
            override_frontend_scenario_title(
                &mut entries,
                &identifier,
                &title,
                load_startup_alphabetical_sorting(self.app_paths.as_ref()),
            );
        }
        self.scenario_catalog = build_scenario_catalog(&entries);
        self.handle_menu_input(move |menu| {
            menu.replace_discovered_entries(
                entries,
                selected_identifier.as_deref(),
                select_first_when_missing,
                apply_live_search,
            )
        })?;
        // Rebuilding the folder stack creates Book layers. Restore the
        // active folder's configured map style before syncing selection-
        // dependent controls so F5 does not silently leave FolderMap view.
        self.configure_current_folder_map();
        self.refresh_scenario_entry_enabled();
        self.menu_state.sync_definition_checkbox_to_selection();
        self.sync_scenario_game_option_constraint();
        self.scensel_last_click = None;
        self.scensel_rename_pointer_focus = None;
        Ok(())
    }

    pub(crate) fn cancel_scenario_selector_discovery(&mut self) {
        if let Some(state) = self.scenario_selector_discovery.take() {
            state.cancel.store(true, AtomicOrdering::Relaxed);
        }
    }

    pub(crate) fn scenario_selector_loading_label(&self) -> Option<String> {
        let progress = self
            .scenario_selector_discovery
            .as_ref()
            .map(|state| state.progress_percent)?;
        let progress = progress.to_string();
        let template = self
            .startup_tooltip_resources
            .get("IDS_MSG_SCENARIODESC_LOADING")
            .cloned()
            .unwrap_or_else(|| "Loading... (%d%%)".to_string());
        Some(format_resource_string(template, &[&progress]).replace("%%", "%"))
    }

    pub(crate) fn poll_scenario_selector_discovery(&mut self) -> Result<(), EngineError> {
        let mut finished = None;
        let mut disconnected = false;
        while let Some(event) = self
            .scenario_selector_discovery
            .as_ref()
            .map(|state| state.receiver.try_recv())
        {
            match event {
                Ok(ScenarioSelectorDiscoveryEvent::Progress(percent)) => {
                    let state = self
                        .scenario_selector_discovery
                        .as_mut()
                        .expect("scenario discovery exists while polling");
                    let percent = state.progress_percent.max(percent.min(100));
                    if state.progress_percent != percent {
                        state.progress_percent = percent;
                    }
                }
                Ok(ScenarioSelectorDiscoveryEvent::Finished(entries)) => {
                    finished = Some(entries);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if let Some(entries) = finished {
            let mut state = self
                .scenario_selector_discovery
                .take()
                .expect("finished scenario discovery retains its state");
            state.cancel.store(true, AtomicOrdering::Relaxed);
            if self.mode != AppMode::Menu || self.startup_view != StartupView::ScenarioBrowser {
                return Ok(());
            }
            let selected_identifier = state.selected_identifier.take();
            let retained_title = state.retained_title.take();
            return self.apply_scenario_selector_entries(
                entries,
                selected_identifier,
                state.select_first_when_missing,
                state.apply_live_search,
                retained_title,
            );
        }
        if disconnected {
            self.cancel_scenario_selector_discovery();
            self.status_text = "Scenario discovery interrupted".to_string();
            tracing::warn!("scenario discovery worker disconnected");
        }
        Ok(())
    }

    pub(crate) fn handle_scenario_selector_override_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::ScenarioBrowser
            || self.context_menu.is_some()
            || state != ElementState::Pressed
        {
            return Ok(false);
        }

        // These are PRIO_CtrlOverride bindings on C4StartupScenSelDlg.  In
        // particular, unmodified Delete outranks the normal-priority search
        // edit, and Alt+M outranks the Comment option mnemonic.  C4KeyCodeEx
        // compares the complete modifier mask, so modified F2/F5/Delete and
        // Ctrl+Alt+M are different keys.
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let no_modifiers = c4_modifiers.is_empty();
        let blocked_while_loading = (no_modifiers
            && (matches!(key, VirtualKeyCode::F5 | VirtualKeyCode::F2)
                || key == VirtualKeyCode::Delete && !self.menu_state.search_focused()))
            || (c4_modifiers == ModifiersState::ALT && key == VirtualKeyCode::KeyM);
        if self.scenario_selector_discovery.is_some() {
            return Ok(blocked_while_loading);
        }
        let book_selection = self.menu_state.current_map().is_none()
            && self.menu_state.selected_scenario().is_some();
        match key {
            VirtualKeyCode::F5 if no_modifiers => {
                if self.scenario_selector_discovery.is_none() {
                    let selected = self
                        .menu_state
                        .selected_scenario()
                        .map(|entry| entry.identifier.clone());
                    self.reload_scenario_selector(selected.as_deref(), true, true)?;
                }
                Ok(true)
            }
            VirtualKeyCode::F2 if no_modifiers && book_selection => {
                let previous_focus = self.scensel_focus_snapshot();
                if self.menu_state.start_renaming_selected(previous_focus) {
                    self.scenario_game_options.set_focused_button(None);
                }
                Ok(true)
            }
            VirtualKeyCode::Delete if no_modifiers && book_selection => {
                self.open_scenario_delete_dialog()?;
                Ok(true)
            }
            VirtualKeyCode::KeyM
                if c4_modifiers == ModifiersState::ALT
                    && self.menu_state.current_map().is_none() =>
            {
                self.open_scenario_mission_access_dialog()?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(crate) fn handle_definition_selector_gamepad_event(
        &mut self,
        event: GamepadEvent,
    ) -> Result<(), EngineError> {
        let layout = self.definition_selector_layout();
        let actions = self
            .definition_selector
            .as_mut()
            .map(|controller| match event {
                GamepadEvent::Direction {
                    button,
                    state: ElementState::Pressed,
                    ..
                } => layout
                    .as_ref()
                    .map(|layout| match button {
                        ControlButton::Up => controller.handle_gamepad_up(layout),
                        ControlButton::Down => controller.handle_gamepad_down(layout),
                        ControlButton::Left => controller.handle_gamepad_left(layout),
                        ControlButton::Right => controller.handle_gamepad_right(layout),
                    })
                    .unwrap_or_default(),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::Low,
                    state: ElementState::Pressed,
                    ..
                } => layout
                    .as_ref()
                    .map(|layout| controller.handle_gamepad_low_down(layout))
                    .unwrap_or_default(),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::Low,
                    state: ElementState::Released,
                    ..
                } => controller.handle_gamepad_low_up(),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                    ..
                } => controller.handle_gamepad_high_down(),
                GamepadEvent::Clear { .. } => {
                    controller.cancel_interaction();
                    Vec::new()
                }
                GamepadEvent::Axis { .. }
                | GamepadEvent::Direction { .. }
                | GamepadEvent::Button { .. }
                | GamepadEvent::Action { .. }
                | GamepadEvent::GuiButton { .. } => Vec::new(),
            })
            .unwrap_or_default();
        self.finish_definition_selector_input(actions)
    }

    pub(crate) fn open_definition_selector(
        &mut self,
        scenario: FrontendScenario,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "C4DefinitionSelDlg",
            self.assets
                .definition_sel_resources()
                .context("exact C4DefinitionSelDlg resource set is absent")
                .and_then(|resources| resources.validate()),
        )?;
        self.close_context_menu_silently();
        self.cancel_underlying_interaction();
        let (root, custom_definition_root, entries) = if let Some(paths) = self.app_paths.as_ref() {
            let definition_paths = match startup_definition_paths(paths) {
                Ok(definition_paths) => definition_paths,
                Err(error) => {
                    let fallback = paths
                        .content_dir()
                        .unwrap_or(paths.install_root())
                        .to_path_buf();
                    tracing::error!(
                        %error,
                        fallback = %fallback.display(),
                        "failed to read General.DefinitionPath; using executable data root"
                    );
                    StartupDefinitionPaths {
                        selector_root: fallback,
                        active_custom_root: None,
                    }
                }
            };
            let root = definition_paths.selector_root;
            let entries = match definition_selector_entries(&root) {
                Ok(entries) => entries,
                Err(error) => {
                    // DirectoryIterator simply yields no files when the path
                    // cannot be enumerated; retain the modal and log it.
                    tracing::error!(
                        %error,
                        path = %root.display(),
                        "failed to enumerate definition selector root"
                    );
                    Vec::new()
                }
            };
            (root, definition_paths.active_custom_root, entries)
        } else {
            tracing::error!("cannot resolve C4DefinitionSelDlg root without application paths");
            (PathBuf::new(), None, Vec::new())
        };
        let fixed_selection = scenario_fixed_definition_modules(&scenario);
        self.startup_tooltip.pointer_left();
        self.definition_selector = Some(
            clonk_frontend::definition_sel::DefinitionSelController::new(
                root.to_string_lossy().into_owned(),
                fixed_selection,
                entries,
            ),
        );
        self.pending_definition_selection = Some(PendingDefinitionSelection {
            scenario,
            selector_mode: self.scenario_selector_mode,
            root,
            custom_definition_root,
        });
        self.pending_lobby_player_selection = None;
        self.definition_selector_last_click = None;
        self.definition_selector_consumed_keys.clear();
        self.definition_selector_pointer_capture = false;
        Ok(())
    }

    pub(crate) fn open_classic_lobby_player_selector(
        &mut self,
        client_id: i32,
    ) -> Result<bool, EngineError> {
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok());
        let local_row = self.visible_classic_lobby_controller().is_some_and(|controller| {
            controller.rows().iter().any(|row| {
                matches!(row, LobbyRosterRow::Client(client) if client.id == client_id && client.local)
            })
        });
        if local_client_id != Some(client_id) || !local_row {
            return Ok(false);
        }
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "C4PlayerSelDlg",
            self.assets
                .definition_sel_resources()
                .context("exact C4PlayerSelDlg resource set is absent")
                .and_then(|resources| {
                    resources.validate_for_mode(clonk_frontend::definition_sel::FileSelMode::Player)
                }),
        )?;
        let Some(paths) = self.app_paths.clone() else {
            let detail = "cannot open player selection without application paths";
            tracing::error!("{detail}");
            self.report_classic_lobby_error(detail);
            return Ok(false);
        };
        let config = match Config::load(paths.config_file()) {
            Ok(config) => config,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
            Err(error) => {
                tracing::error!(%error, "failed to read player selector configuration");
                self.report_classic_lobby_error(format!(
                    "Unable to read player selection configuration: {error}"
                ));
                Config::new()
            }
        };
        let frozen_root = startup_player_search_paths(&paths, &config)
            .into_iter()
            .next()
            .unwrap_or_else(|| paths.install_root().to_path_buf());
        let (root, entries, candidates) = match discover_lobby_player_selector(&paths, &config) {
            Ok(selection) => selection,
            Err(error) => {
                tracing::error!(%error, "failed to enumerate lobby player files");
                self.report_classic_lobby_error(format!(
                    "Unable to enumerate player files: {error}"
                ));
                (frozen_root, Vec::new(), BTreeMap::new())
            }
        };
        let root = root.to_string_lossy().into_owned();
        self.close_context_menu_silently();
        if let Some(controller) = self.visible_classic_lobby_controller_mut() {
            controller.cancel_interaction();
        }
        self.startup_tooltip.pointer_left();
        self.definition_selector = Some(
            clonk_frontend::definition_sel::DefinitionSelController::new_player(root, entries),
        );
        self.pending_definition_selection = None;
        self.pending_lobby_player_selection = Some(PendingLobbyPlayerSelection {
            client_id,
            config,
            candidates,
        });
        self.definition_selector_last_click = None;
        self.definition_selector_consumed_keys.clear();
        self.definition_selector_pointer_capture = false;
        Ok(true)
    }

    pub(crate) fn open_startup_player_portrait_selector(&mut self) {
        let Some(paths) = self.app_paths.as_ref() else {
            if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
                pending
                    .controller
                    .set_validation_error(Some("Application paths are unavailable".to_string()));
            }
            return;
        };
        if !self.startup_user_portraits_written {
            self.startup_user_portraits_written = true;
            extract_default_startup_portraits_once(paths);
        }
        let locations = startup_player_portrait_locations(paths);
        let current_location = self
            .startup_last_portrait_folder_index
            .or_else(|| load_startup_last_portrait_folder_index(Some(paths)))
            .filter(|index| *index < locations.len())
            .unwrap_or(0);
        self.startup_last_portrait_folder_index = Some(current_location);
        let current_path = locations[current_location].path.clone();
        let entries =
            match clonk_frontend::startup_portraitsel::portrait_files_in_location(&current_path) {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::warn!(
                        path = %current_path.display(),
                        %error,
                        "failed to scan initial portrait location"
                    );
                    Vec::new()
                }
            };
        if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
            pending
                .controller
                .open_portrait_selector(locations, current_location, entries);
            pending.controller.clear_validation_error();
        }
        self.startup_tooltip.pointer_left();
    }

    pub(crate) fn scensel_do_back(&mut self) -> Result<(), EngineError> {
        if self.scenario_selector_discovery.is_some() {
            self.close_scenario_browser();
            return Ok(());
        }
        if self.menu_state.stack.len() <= 1 {
            self.close_scenario_browser();
        } else {
            self.play_ui_sound("DoorClose");
            self.menu_state.leave_folder();
            self.configure_current_folder_map();
            self.refresh_scenario_entry_enabled();
            self.set_scensel_dialog_focus(ScenselDialogFocus::List);
            self.scenario_label = self.menu_state.label_path();
            self.handle_menu_input(|menu| menu.select_default_entry())?;
        }
        Ok(())
    }

    /// Routes a click through the C++-faithful scenario book layout
    /// (Back / Open buttons + list rows, C4StartupScenSelDlg.cpp:1349-1382).
    pub(crate) fn handle_scensel_parity_click(
        &mut self,
        point: GuiPoint,
        suppress_click_focus: bool,
    ) -> Result<(), EngineError> {
        let Some(fonts) = self.assets.clonk_fonts.clone() else {
            return Ok(());
        };
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
            &fonts,
        );
        let (px, py) = (point.x as i32, point.y as i32);
        let inside =
            |x: i32, y: i32, w: i32, h: i32| px >= x && px < x + w && py >= y && py < y + h;
        if self.scenario_selector_discovery.is_some() {
            let back = layout.back_button;
            if inside(back.x, back.y, back.w, back.h) {
                if !suppress_click_focus {
                    self.set_scensel_dialog_focus(ScenselDialogFocus::Back);
                }
                self.scensel_do_back()?;
            }
            return Ok(());
        }
        if self.menu_state.current_map().is_some() {
            return self.handle_scensel_map_click(point, layout);
        }
        let (back, open, list, search, definitions) = (
            layout.back_button,
            layout.open_button,
            layout.list,
            layout.search_edit,
            layout.user_change_checkbox,
        );
        if inside(search.x, search.y, search.w, search.h) {
            if !suppress_click_focus {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Search);
            }
        } else if inside(definitions.x, definitions.y, definitions.h, definitions.h) {
            if !suppress_click_focus && self.menu_state.definition_checkbox_enabled {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Definitions);
            }
            if self.menu_state.toggle_definition_checkbox() {
                self.play_ui_sound("ArrowHit");
            }
        } else if inside(back.x, back.y, back.w, back.h) {
            if !suppress_click_focus {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Back);
            }
            self.scensel_do_back()?;
        } else if inside(open.x, open.y, open.w, open.h) {
            if !suppress_click_focus {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Open);
            }
            self.handle_menu_input(|menu| menu.menu().handle_key_down(KeyCode::Enter))?;
            self.handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Enter))?;
        } else if inside(list.x + 3, list.y + 3, list.w - 6 - 16, list.h - 6) {
            if !suppress_click_focus {
                self.set_scensel_dialog_focus(ScenselDialogFocus::List);
            }
            if let Some(book) = self.assets.book_fonts.clone() {
                let pitch = clonk_frontend::startup_scensel::scen_list_item_height(&book.text) + 1;
                let index = ((py - (list.y + 3) + self.menu_state.scenario_list_scroll()) / pitch)
                    .max(0) as usize;
                // Double-click on the selected row opens/starts it
                // (OnSelDblClick -> DoOK, C4StartupScenSelDlg.h:430).
                let now = Instant::now();
                let is_double = self.scensel_last_click.is_some_and(|(last_index, at)| {
                    last_index == index && now.duration_since(at) < Duration::from_millis(500)
                }) && self.menu_state.menu().selected_index() == Some(index);
                self.scensel_last_click = Some((index, now));
                if is_double {
                    self.handle_menu_input(|menu| menu.menu().handle_key_down(KeyCode::Enter))?;
                    self.handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Enter))?;
                } else {
                    self.handle_menu_input(|menu| {
                        menu.menu().select_entry_by_index(index).unwrap_or_default()
                    })?;
                }
            }
        }
        Ok(())
    }

    fn handle_scensel_map_click(
        &mut self,
        point: GuiPoint,
        layout: clonk_frontend::startup_scensel::ScenSelLayout,
    ) -> Result<(), EngineError> {
        let inside = |rect: clonk_frontend::classic_gui::IntRect| {
            point.x >= rect.x as f32
                && point.x < (rect.x + rect.w) as f32
                && point.y >= rect.y as f32
                && point.y < (rect.y + rect.h) as f32
        };
        if inside(layout.user_change_checkbox) {
            if self.menu_state.definition_checkbox_enabled {
                self.set_scensel_dialog_focus(ScenselDialogFocus::Definitions);
            }
            if self.menu_state.toggle_definition_checkbox() {
                self.play_ui_sound("ArrowHit");
            }
            return Ok(());
        }
        if inside(layout.back_button) {
            self.set_scensel_dialog_focus(ScenselDialogFocus::Back);
            return self.scensel_do_back();
        }
        if inside(layout.open_button) {
            self.set_scensel_dialog_focus(ScenselDialogFocus::Open);
            let action = self.menu_state.start_selected_map_scenario();
            return self.handle_menu_input(move |_| action.into_iter().collect());
        }

        let surface = self.graphics.surface();
        let button = {
            let map = self
                .menu_state
                .current_map()
                .expect("map click requires active map data");
            let transform =
                MapFolderTransform::for_map(map, &layout, surface.width(), surface.height());
            if point_in_map_rect(point, &transform.rect(map.scenario_info_area)) {
                None
            } else {
                map_folder_button_at(map, transform, point)
            }
        };
        if let Some(index) = button {
            self.set_scensel_dialog_focus(ScenselDialogFocus::List);
            let action = self.menu_state.activate_map_button(index);
            if action.is_none() {
                self.menu_state.sync_definition_checkbox_to_selection();
                self.sync_scenario_game_option_constraint();
            }
            return self.handle_menu_input(move |_| action.into_iter().collect());
        }
        Ok(())
    }

    /// `MapPic::MouseInput` invokes `DeselectAll` on left-down. Scenario
    /// buttons sit above map pictures and consume the press instead, while a
    /// fullscreen dialog background is not a `MapPic` at all.
    pub(crate) fn handle_scensel_map_pointer_down(&mut self, point: GuiPoint) -> bool {
        if self.scenario_selector_discovery.is_some() {
            return true;
        }
        let Some(fonts) = self.assets.clonk_fonts.as_deref() else {
            return false;
        };
        let Some(map) = self.menu_state.current_map() else {
            return false;
        };
        let surface = self.graphics.surface();
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            surface.width() as i32,
            surface.height() as i32,
            fonts,
        );
        let transform =
            MapFolderTransform::for_map(map, &layout, surface.width(), surface.height());
        if point_in_map_rect(point, &transform.rect(map.scenario_info_area)) {
            return true;
        }
        if map_folder_button_at(map, transform, point).is_some() {
            return true;
        }
        let map_picture = (!map.fullscreen_background
            && point_in_map_rect(point, &transform.background))
            || map
                .access_overlays
                .iter()
                .any(|overlay| point_in_map_rect(point, &transform.rect(overlay.area)));
        if !map_picture {
            return false;
        }
        if self.menu_state.deselect_map() {
            self.play_ui_sound("ArrowHit");
        }
        self.sync_scenario_game_option_constraint();
        true
    }

    pub(crate) fn definition_selector_layout(
        &self,
    ) -> Option<clonk_frontend::definition_sel::DefinitionSelLayout> {
        let controller = self.definition_selector.as_ref()?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let surface = self.graphics.surface();
        Some(controller.layout(surface.width() as i32, surface.height() as i32, &fonts.text))
    }

    pub(crate) fn play_definition_selector_sound_events(
        &mut self,
        events: Vec<clonk_frontend::definition_sel::DefinitionSelSound>,
    ) {
        for event in events {
            self.play_ui_sound(match event {
                clonk_frontend::definition_sel::DefinitionSelSound::Command => "Command",
                clonk_frontend::definition_sel::DefinitionSelSound::ArrowHit => "ArrowHit",
                clonk_frontend::definition_sel::DefinitionSelSound::Click => "Click",
            });
        }
    }

    pub(crate) fn finish_definition_selector_input(
        &mut self,
        actions: Vec<clonk_frontend::definition_sel::DefinitionSelAction>,
    ) -> Result<(), EngineError> {
        let sounds = self
            .definition_selector
            .as_mut()
            .map(|controller| controller.take_sound_events())
            .unwrap_or_default();
        self.play_definition_selector_sound_events(sounds);
        self.process_definition_selector_actions(actions)
    }

    pub(crate) fn process_definition_selector_actions(
        &mut self,
        actions: Vec<clonk_frontend::definition_sel::DefinitionSelAction>,
    ) -> Result<(), EngineError> {
        use clonk_frontend::definition_sel::DefinitionSelAction;

        for action in actions {
            match action {
                DefinitionSelAction::FocusChanged(_)
                | DefinitionSelAction::SelectionChanged(_)
                | DefinitionSelAction::CheckedChanged { .. } => {}
                DefinitionSelAction::RefreshRequested => {
                    if self.pending_lobby_player_selection.is_some() {
                        let refreshed = self.app_paths.clone().zip(
                            self.pending_lobby_player_selection
                                .as_ref()
                                .map(|pending| pending.config.clone()),
                        );
                        let refreshed = refreshed
                            .map(|(paths, config)| discover_lobby_player_selector(&paths, &config))
                            .transpose();
                        match refreshed {
                            Ok(Some((_root, entries, candidates))) => {
                                if let Some(pending) = self.pending_lobby_player_selection.as_mut()
                                {
                                    pending.candidates = candidates;
                                }
                                if let Some(controller) = self.definition_selector.as_mut() {
                                    controller.rebuild_rows_after_refresh(entries);
                                }
                            }
                            Ok(None) => {
                                tracing::error!(
                                    "player selector refresh requested without application paths"
                                );
                                self.report_classic_lobby_error(
                                    "Unable to refresh player files without application paths.",
                                );
                            }
                            Err(error) => {
                                tracing::error!(%error, "failed to refresh lobby player selector");
                                if let Some(pending) = self.pending_lobby_player_selection.as_mut()
                                {
                                    pending.candidates.clear();
                                }
                                if let Some(controller) = self.definition_selector.as_mut() {
                                    controller.rebuild_rows_after_refresh(Vec::new());
                                }
                                self.report_classic_lobby_error(format!(
                                    "Unable to refresh player files: {error}"
                                ));
                            }
                        }
                        self.definition_selector_last_click = None;
                        continue;
                    }
                    let Some(root) = self
                        .pending_definition_selection
                        .as_ref()
                        .map(|pending| pending.root.clone())
                    else {
                        tracing::error!(
                            "definition selector refresh requested without pending scenario"
                        );
                        continue;
                    };
                    let entries = match definition_selector_entries(&root) {
                        Ok(entries) => entries,
                        Err(error) => {
                            tracing::error!(
                                %error,
                                path = %root.display(),
                                "failed to refresh definition selector root"
                            );
                            Vec::new()
                        }
                    };
                    if let Some(controller) = self.definition_selector.as_mut() {
                        // C4FileSelDlg::UpdateFileList rebuilds every row and
                        // C4DefinitionSelDlg does not reapply fixed checks on F5.
                        controller.rebuild_rows_after_refresh(entries);
                    }
                    self.definition_selector_last_click = None;
                }
                DefinitionSelAction::PleaseSelectFile => {
                    self.push_message_dialog(
                        clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                            "Please select a file first!",
                            "Error",
                            clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                        ),
                        MessageDialogContinuation::None,
                    )?;
                }
                DefinitionSelAction::Accepted(modules) => {
                    if let Some(pending) = self.pending_lobby_player_selection.take() {
                        self.startup_tooltip.pointer_left();
                        self.definition_selector = None;
                        self.definition_selector_last_click = None;
                        if let [selected] = modules.as_slice() {
                            if let Some(candidate) = pending.candidates.get(selected).cloned() {
                                self.submit_selected_classic_lobby_player(
                                    pending.client_id,
                                    &candidate.source_path,
                                    &candidate.wire_filename,
                                );
                            } else {
                                tracing::warn!(
                                    path = selected,
                                    "lobby player selector accepted a stale path"
                                );
                                self.report_classic_lobby_error(
                                    "The selected player file is no longer available.",
                                );
                            }
                        }
                        break;
                    }
                    let Some(pending) = self.pending_definition_selection.take() else {
                        tracing::error!("definition selector accepted without pending scenario");
                        self.startup_tooltip.pointer_left();
                        self.definition_selector = None;
                        break;
                    };
                    self.startup_tooltip.pointer_left();
                    self.definition_selector = None;
                    self.definition_selector_last_click = None;
                    self.accept_scenario_from_selector(
                        pending.scenario,
                        pending.selector_mode,
                        Some(ScenarioDefinitionLoad::Fixed {
                            modules,
                            definition_root: pending.custom_definition_root,
                        }),
                    )?;
                    break;
                }
                DefinitionSelAction::Cancelled => {
                    self.startup_tooltip.pointer_left();
                    self.definition_selector = None;
                    self.pending_definition_selection = None;
                    self.pending_lobby_player_selection = None;
                    self.definition_selector_last_click = None;
                    break;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn handle_definition_selector_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.definition_selector.is_none() {
            return Ok(false);
        }
        match state {
            ElementState::Pressed => {
                self.definition_selector_consumed_keys.insert(key);
            }
            ElementState::Released => {
                self.definition_selector_consumed_keys.remove(&key);
            }
        }
        let backwards = self.keyboard_modifiers.shift_key();
        let alt = self.keyboard_modifiers.alt_key();
        let layout = self.definition_selector_layout();
        let actions = self
            .definition_selector
            .as_mut()
            .map(|controller| {
                if alt && state == ElementState::Pressed {
                    return context_menu_hotkey(key)
                        .map(|character| controller.handle_hotkey(character))
                        .unwrap_or_default();
                }
                let Some(key) = definition_selector_key_code(key) else {
                    return Vec::new();
                };
                match state {
                    ElementState::Pressed => layout
                        .as_ref()
                        .map(|layout| controller.handle_key_down(key, backwards, layout))
                        .unwrap_or_default(),
                    ElementState::Released => controller.handle_key_up(key),
                }
            })
            .unwrap_or_default();
        self.finish_definition_selector_input(actions)?;
        Ok(true)
    }

    pub(crate) fn render_definition_selector(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        let Some(controller) = self.definition_selector.as_ref() else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        let Some(resources) = assets.definition_sel_resources() else {
            tracing::error!(
                "refusing to render C4DefinitionSelDlg without exact classic resources"
            );
            anyhow::bail!(
                "classic definition-selector resources are unavailable; refusing generic fallback"
            );
        };
        controller.render(
            self.graphics.surface_mut(),
            resources,
            self.message_dialogs.is_empty(),
            gamma,
        )
    }

    /// `C4ScenarioListLoader::Scenario::CanOpen`: mission access is checked
    /// before replay/player rules and applies to local and network selectors.
    pub(crate) fn scenario_selector_open_error(
        &self,
        scenario: &FrontendScenario,
        selector_mode: ScenarioSelectorMode,
    ) -> std::result::Result<Option<String>, ClassicParityBoundary> {
        if !scenario.has_mission_access(&self.mission_access) {
            return Ok(Some(
                self.runtime_resource_string("IDS_PRC_NOMISSIONACCESS"),
            ));
        }
        if selector_mode == ScenarioSelectorMode::NetworkHost
            && matches!(scenario.kind, ScenarioKind::Scenario)
        {
            return self
                .network_scenario_open_decision(scenario)
                .map(|decision| match decision {
                    NetworkScenarioOpenDecision::Error { message, .. } => Some(message),
                    NetworkScenarioOpenDecision::Proceed
                    | NetworkScenarioOpenDecision::Warning { .. } => None,
                });
        }
        let Some(head) = self.scenario_loader_head_for_start(scenario)? else {
            return Ok(None);
        };
        if !head.mission_access().is_empty() && !self.mission_access.contains(head.mission_access())
        {
            return Ok(Some(
                self.runtime_resource_string("IDS_PRC_NOMISSIONACCESS"),
            ));
        }
        self.local_scenario_player_count_error_from_head(&head)
    }

    pub(crate) fn continue_scenario_from_selector(
        &mut self,
        scenario: FrontendScenario,
    ) -> Result<(), EngineError> {
        if self.startup_view == StartupView::ScenarioBrowser
            && self.menu_state.definition_checkbox_checked
        {
            self.open_definition_selector(scenario)
        } else {
            self.accept_scenario_from_selector(scenario, self.scenario_selector_mode, None)
        }
    }

    fn accept_scenario_from_selector(
        &mut self,
        scenario: FrontendScenario,
        selector_mode: ScenarioSelectorMode,
        definition_load: Option<ScenarioDefinitionLoad>,
    ) -> Result<(), EngineError> {
        let definition_load = match definition_load {
            Some(definition_load) => definition_load,
            None => self.take_scenario_seed_definition_load(),
        };
        match selector_mode {
            ScenarioSelectorMode::Local => {
                self.start_scenario_with_definition_load(scenario, definition_load)
            }
            ScenarioSelectorMode::NetworkHost => {
                self.stage_network_host_scenario(scenario, definition_load)
            }
        }
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod portrait_location_tests {
    use super::*;

    #[test]
    fn optional_portrait_locations_follow_cpp_windows_order() {
        // C4FileSelDlg.cpp:541-552 appends the three shell folders, then HOME.
        let root = tempfile::tempdir().expect("temporary portrait roots");
        let documents = root.path().join("Documents");
        let pictures = root.path().join("Pictures");
        let desktop = root.path().join("Desktop");
        let home = root.path().join("Home");
        for path in [&documents, &pictures, &desktop, &home] {
            fs::create_dir(path).expect("create portrait root");
        }
        let mut locations = Vec::new();

        add_optional_portrait_locations(
            &mut locations,
            [
                ("My Documents", Some(documents.clone())),
                ("My Pictures", Some(pictures.clone())),
                ("Desktop", Some(desktop.clone())),
            ],
            Some(("Home Folder", home.clone())),
            false,
        );

        assert_eq!(
            locations
                .iter()
                .map(|location| (location.label.as_str(), location.path.as_path()))
                .collect::<Vec<_>>(),
            vec![
                ("My Documents", documents.as_path()),
                ("My Pictures", pictures.as_path()),
                ("Desktop", desktop.as_path()),
                ("Home Folder", home.as_path()),
            ]
        );
    }

    #[test]
    fn optional_portrait_locations_append_unix_desktop_after_home() {
        // C4FileSelDlg.cpp:550-556 uses "Home Folder" off Apple and derives
        // Desktop from HOME on every non-Windows platform.
        let root = tempfile::tempdir().expect("temporary portrait roots");
        let home = root.path().join("Home");
        let desktop = home.join("Desktop");
        fs::create_dir_all(&desktop).expect("create portrait roots");
        let mut locations = Vec::new();

        add_optional_portrait_locations(
            &mut locations,
            [],
            Some(("Home Folder", home.clone())),
            true,
        );

        assert_eq!(
            locations
                .iter()
                .map(|location| (location.label.as_str(), location.path.as_path()))
                .collect::<Vec<_>>(),
            vec![
                ("Home Folder", home.as_path()),
                ("Desktop", desktop.as_path()),
            ]
        );
    }
}
