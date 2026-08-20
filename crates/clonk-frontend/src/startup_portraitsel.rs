//! Classic portrait-file selector used by the startup player properties dialog.
//!
//! The file scan is intentionally cheap and synchronous: it records matching
//! direct children without decoding them. The application consumes
//! [`PortraitThumbnailRequest`] values one at a time and returns the decoded
//! previews, matching `C4PortraitSelDlg::ImageLoader`'s incremental worklist.

use crate::classic_gui::{
    draw_3d_frame, draw_engine_box, draw_facet_nearest, draw_facet_stretch, with_surface_clip,
    ClassicButtonState, ClassicGuiSkin, IntRect,
};
use crate::startup_main_menu::StartupTooltip;
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode};
use clonk_graphics::clonk_font::{ClonkFont, TextAlign};
use clonk_graphics::{GammaRamp, Rect as SurfaceRect, Surface};
use clonk_gui::Rect as GuiRect;
use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MIN_WIDTH: i32 = 300;
const MAX_WIDTH: i32 = 600;
const MIN_HEIGHT: i32 = 220;
const MAX_HEIGHT: i32 = 500;
const CAPTION_HEIGHT: i32 = 23;
const TEXT_LINE_HEIGHT: i32 = 22;
const MINI_LINE_HEIGHT: i32 = 18;
const PREVIEW_SIZE: i32 = 100;
const TILE_HEIGHT: i32 = PREVIEW_SIZE + MINI_LINE_HEIGHT;
const CONTROL_HEIGHT: i32 = 26;
const BUTTON_WIDTH: i32 = 120;
const BUTTON_HEIGHT: i32 = 32;
const SCROLLBAR_WIDTH: i32 = 16;
const CONTEXT_ROW_SPACING: i32 = 1;
const LOAD_IDLE_INTERVAL: u8 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortraitLocation {
    pub label: String,
    pub path: PathBuf,
}

impl PortraitLocation {
    pub fn new(label: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            label: label.into(),
            path: path.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortraitFileEntry {
    pub full_path: PathBuf,
    pub filename: String,
    pub label: String,
}

impl PortraitFileEntry {
    pub fn from_path(path: PathBuf) -> Option<Self> {
        let filename = path.file_name()?.to_string_lossy().into_owned();
        let label = filename
            .rfind('.')
            .map_or_else(|| filename.clone(), |dot| filename[..dot].to_string());
        Some(Self {
            full_path: path,
            filename,
            label,
        })
    }
}

/// Raw direct-child `C4CFN_ImageFiles` scan (`*.png|*.bmp|*.jpeg|*.jpg`).
pub fn portrait_files_in_location(root: &Path) -> io::Result<Vec<PortraitFileEntry>> {
    Ok(portrait_files_from_paths(
        fs::read_dir(root)?.map(|entry| entry.map(|entry| entry.path())),
    ))
}

fn portrait_files_from_paths(
    entries: impl IntoIterator<Item = io::Result<PathBuf>>,
) -> Vec<PortraitFileEntry> {
    let mut files = Vec::new();
    for path in entries.into_iter().map_while(Result::ok) {
        // C4Group::FindEntry applies the wildcard to raw entries and does not
        // filter matching directories (`C4FileSelDlg.cpp:251-266`,
        // `StdFile.cpp:824-836`).
        if !matches_c4cfn_image_files(&path) {
            continue;
        }
        if let Some(entry) = PortraitFileEntry::from_path(path) {
            files.push(entry);
        }
    }
    files
}

fn matches_c4cfn_image_files(path: &Path) -> bool {
    path.file_name().is_some_and(|filename| {
        let filename = filename.to_string_lossy().to_ascii_lowercase();
        [".png", ".bmp", ".jpeg", ".jpg"]
            .iter()
            .any(|suffix| filename.ends_with(suffix))
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortraitChoice {
    None,
    File(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PortraitThumbnail {
    None,
    Pending,
    Loading,
    Ready(ImageData),
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortraitItem {
    choice: PortraitChoice,
    filename: Option<String>,
    label: String,
    thumbnail: PortraitThumbnail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PortraitGridItemLayout {
    rect: IntRect,
    wrapped_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PortraitGridContentLayout {
    columns: usize,
    items: Vec<PortraitGridItemLayout>,
    content_height: i32,
}

impl PortraitGridContentLayout {
    fn reflow(&self, columns: usize) -> Self {
        let columns = columns.max(1);
        let mut items = self.items.clone();
        let mut row_y = 0_i32;
        for row in items.chunks_mut(columns) {
            let row_height = row.iter().map(|item| item.rect.h).max().unwrap_or(0);
            for (column, item) in row.iter_mut().enumerate() {
                item.rect.x = i32::try_from(column)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(PREVIEW_SIZE);
                item.rect.y = row_y;
                item.rect.w = PREVIEW_SIZE;
            }
            row_y = row_y.saturating_add(row_height);
        }
        Self {
            columns,
            items,
            content_height: row_y,
        }
    }
}

impl PortraitItem {
    fn none() -> Self {
        Self {
            choice: PortraitChoice::None,
            filename: None,
            label: "No Portrait".to_string(),
            thumbnail: PortraitThumbnail::None,
        }
    }

    fn file(entry: PortraitFileEntry) -> Self {
        Self {
            choice: PortraitChoice::File(entry.full_path),
            filename: Some(entry.filename),
            label: entry.label,
            thumbnail: PortraitThumbnail::Pending,
        }
    }

    pub const fn choice(&self) -> &PortraitChoice {
        &self.choice
    }

    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub const fn thumbnail(&self) -> &PortraitThumbnail {
        &self.thumbnail
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortraitSelCommit {
    pub choice: PortraitChoice,
    pub set_picture: bool,
    pub set_big_icon: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortraitSelAction {
    ChangeLocation { index: usize, path: PathBuf },
    Accept(PortraitSelCommit),
    SelectionRequired,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortraitThumbnailRequest {
    pub generation: u64,
    pub index: usize,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortraitSelControl {
    NoControl,
    Close,
    Location,
    Grid,
    SelectedItem,
    SetPicture,
    SetBigIcon,
    Ok,
    Cancel,
}

impl PortraitSelControl {
    const ORDER: [Self; 8] = [
        Self::Close,
        Self::Location,
        Self::Grid,
        Self::SelectedItem,
        Self::SetPicture,
        Self::SetBigIcon,
        Self::Ok,
        Self::Cancel,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitTarget {
    None,
    Close,
    Location,
    LocationPopup,
    LocationOption(usize),
    Tile(usize),
    SetPicture,
    SetBigIcon,
    Ok,
    Cancel,
}

#[derive(Clone, Debug)]
pub struct PortraitSelController {
    width: i32,
    height: i32,
    locations: Vec<PortraitLocation>,
    current_location: usize,
    items: Vec<PortraitItem>,
    selected: Option<usize>,
    focus: PortraitSelControl,
    focused_item: Option<usize>,
    scroll_y: i32,
    scrollbar_pin: i32,
    grid_layout: RefCell<Option<PortraitGridContentLayout>>,
    set_picture: bool,
    set_big_icon: bool,
    combo_open: bool,
    combo_highlight: Option<usize>,
    pointer: Option<GuiPoint>,
    pressed: HitTarget,
    key_pressed: Option<PortraitSelControl>,
    scrollbar_dragging: bool,
    scrollbar_arrow_captured: bool,
    scrollbar_arrow: i8,
    idle_tick: u8,
    generation: u64,
    validation_error: Option<String>,
}

impl PortraitSelController {
    pub fn new(
        locations: Vec<PortraitLocation>,
        current_location: usize,
        entries: Vec<PortraitFileEntry>,
        set_picture: bool,
        set_big_icon: bool,
    ) -> Self {
        let current_location = current_location.min(locations.len().saturating_sub(1));
        let mut controller = Self {
            width: 600,
            height: 500,
            locations,
            current_location,
            items: Vec::new(),
            selected: None,
            focus: PortraitSelControl::Grid,
            focused_item: None,
            scroll_y: 0,
            scrollbar_pin: 0,
            grid_layout: RefCell::new(None),
            set_picture,
            set_big_icon,
            combo_open: false,
            combo_highlight: None,
            pointer: None,
            pressed: HitTarget::None,
            key_pressed: None,
            scrollbar_dragging: false,
            scrollbar_arrow_captured: false,
            scrollbar_arrow: 0,
            idle_tick: 0,
            generation: 0,
            validation_error: None,
        };
        controller.install_entries(entries);
        controller
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        let columns = self.layout().columns;
        if let Some(layout) = self.grid_layout.get_mut().as_mut() {
            *layout = layout.reflow(columns);
        }
        self.scroll_y = self.scroll_y.clamp(0, self.max_scroll_y());
        self.ensure_selected_visible();
        self.sync_scrollbar_pin();
    }

    pub fn locations(&self) -> &[PortraitLocation] {
        &self.locations
    }

    pub const fn current_location_index(&self) -> usize {
        self.current_location
    }

    pub fn current_location(&self) -> Option<&PortraitLocation> {
        self.locations.get(self.current_location)
    }

    pub fn items(&self) -> &[PortraitItem] {
        &self.items
    }

    pub const fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&PortraitItem> {
        self.selected.and_then(|index| self.items.get(index))
    }

    pub const fn set_picture(&self) -> bool {
        self.set_picture
    }

    pub const fn set_big_icon(&self) -> bool {
        self.set_big_icon
    }

    pub const fn focus(&self) -> PortraitSelControl {
        self.focus
    }

    pub const fn is_location_popup_open(&self) -> bool {
        self.combo_open
    }

    pub fn validation_error(&self) -> Option<&str> {
        self.validation_error.as_deref()
    }

    pub fn set_validation_error(&mut self, error: impl Into<String>) {
        self.validation_error = Some(error.into());
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer
    }

    pub fn tooltip_at(&self, point: GuiPoint) -> Option<StartupTooltip> {
        let layout = self.layout();
        if self.combo_open {
            if let Some(index) = layout
                .location_options
                .iter()
                .position(|rect| contains(*rect, point))
            {
                return self.locations.get(index).map(|location| {
                    StartupTooltip::formatted_resource("IDS_MSG_SELECT", [location.label.clone()])
                });
            }
            if contains(layout.location_popup, point) {
                return None;
            }
        }
        if contains(layout.close, point) {
            return Some(StartupTooltip::resource("IDS_MNU_CLOSE"));
        }
        if contains(layout.caption, point) {
            return Some(StartupTooltip::text(self.caption()));
        }
        if contains(layout.set_picture, point) {
            return Some(StartupTooltip::resource(
                "IDS_DESC_CHANGESTHEIMAGEYOUSEEINTH",
            ));
        }
        contains(layout.set_big_icon, point)
            .then(|| StartupTooltip::resource("IDS_DESC_CHANGESTHEIMAGEYOUSEEINTH2"))
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.pressed = HitTarget::None;
        self.scrollbar_dragging = false;
        self.scrollbar_arrow_captured = false;
        self.scrollbar_arrow = 0;
        self.combo_highlight = None;
    }

    pub fn replace_location_entries(
        &mut self,
        location_index: usize,
        entries: Vec<PortraitFileEntry>,
    ) -> bool {
        if location_index != self.current_location {
            return false;
        }
        self.install_entries(entries);
        true
    }

    pub fn fail_location_entries(&mut self, location_index: usize, _error: impl Into<String>) {
        if location_index == self.current_location {
            self.install_entries(Vec::new());
        }
    }

    fn install_entries(&mut self, entries: Vec<PortraitFileEntry>) {
        self.generation = self.generation.wrapping_add(1);
        if self.focus == PortraitSelControl::SelectedItem {
            // Deleting the focused ListItem clears Dialog::pActiveCtrl
            // (`C4GuiDialogs.cpp:475-481`).
            self.focus = PortraitSelControl::NoControl;
        }
        self.focused_item = None;
        self.items.clear();
        self.items
            .extend(entries.into_iter().map(PortraitItem::file));
        // `C4FileSelDlg::UpdateFileList` appends the null entry after every
        // matching file (`C4FileSelDlg.cpp:251-266`).
        self.items.push(PortraitItem::none());
        // Pinned C++ then loses this null filename in GetSelection and makes
        // the visible "No Portrait" choice ineffective
        // (`C4FileSelDlg.cpp:305-315,627-642`). Keep the intended UI action:
        // profile artwork is presentation-only and cannot affect lockstep.
        self.selected = None;
        self.scroll_y = 0;
        self.scrollbar_pin = 0;
        self.scrollbar_dragging = false;
        self.scrollbar_arrow_captured = false;
        self.scrollbar_arrow = 0;
        self.grid_layout.get_mut().take();
        self.idle_tick = 0;
        self.validation_error = None;
    }

    /// Simulates one `OnIdle` call. At most one request is released on every
    /// tenth call, with the first item released immediately like `i++ % 10`.
    pub fn advance_idle(&mut self) -> Option<PortraitThumbnailRequest> {
        let release = self.idle_tick == 0;
        self.idle_tick = (self.idle_tick + 1) % LOAD_IDLE_INTERVAL;
        if !release {
            return None;
        }
        let (index, item) = self
            .items
            .iter_mut()
            .enumerate()
            .find(|(_, item)| item.thumbnail == PortraitThumbnail::Pending)?;
        let PortraitChoice::File(path) = &item.choice else {
            return None;
        };
        let request = PortraitThumbnailRequest {
            generation: self.generation,
            index,
            path: path.clone(),
        };
        item.thumbnail = PortraitThumbnail::Loading;
        Some(request)
    }

    pub fn complete_thumbnail(
        &mut self,
        request: &PortraitThumbnailRequest,
        thumbnail: Result<ImageData, String>,
    ) -> bool {
        if request.generation != self.generation {
            return false;
        }
        let Some(item) = self.items.get_mut(request.index) else {
            return false;
        };
        if !matches!(&item.choice, PortraitChoice::File(path) if path == &request.path) {
            return false;
        }
        item.thumbnail = thumbnail
            .map(PortraitThumbnail::Ready)
            .unwrap_or(PortraitThumbnail::Failed);
        true
    }

    pub fn has_pending_thumbnails(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(
                item.thumbnail,
                PortraitThumbnail::Pending | PortraitThumbnail::Loading
            )
        })
    }

    pub fn handle_pointer_move(&mut self, point: GuiPoint) -> Vec<PortraitSelAction> {
        self.pointer = Some(point);
        if self.scrollbar_dragging {
            self.set_scroll_from_scrollbar_pointer(point);
            return Vec::new();
        }
        if self.scrollbar_arrow_captured {
            let arrow = self.scrollbar_arrow_at(point);
            if arrow != 0 {
                self.scrollbar_arrow = arrow;
            } else if contains(self.layout().grid_scrollbar, point) && self.max_scrollbar_pin() > 0
            {
                self.scrollbar_arrow_captured = false;
                self.scrollbar_arrow = 0;
                self.set_scroll_from_scrollbar_pointer(point);
                self.scrollbar_dragging = true;
            } else {
                self.scrollbar_arrow = 0;
            }
            return Vec::new();
        }
        if self.combo_open {
            self.combo_highlight = match self.hit_target(point) {
                HitTarget::LocationOption(index) => Some(index),
                _ => None,
            };
        }
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, point: GuiPoint) -> Vec<PortraitSelAction> {
        self.pointer = Some(point);
        if self.combo_open {
            match self.hit_target(point) {
                HitTarget::LocationOption(index) => {
                    self.pressed = HitTarget::None;
                    return self.choose_location(index);
                }
                HitTarget::LocationPopup => {
                    self.pressed = HitTarget::LocationPopup;
                    self.combo_highlight = None;
                    return Vec::new();
                }
                HitTarget::Location => {
                    // Screen aborts the old context before ComboBox sees this
                    // click. The combo recognizes its just-closed menu and
                    // deliberately does not reopen it.
                    self.combo_open = false;
                    self.combo_highlight = None;
                    self.pressed = HitTarget::Location;
                    return Vec::new();
                }
                _ => {
                    self.combo_open = false;
                    self.combo_highlight = None;
                }
            }
        }
        let layout = self.layout();
        if contains(layout.grid_scrollbar, point) {
            self.pressed = HitTarget::None;
            if self.focus != PortraitSelControl::SelectedItem {
                self.focus = PortraitSelControl::Grid;
                self.focused_item = None;
            }
            if self.max_scroll_y() > 0 {
                self.begin_scrollbar_pointer(point);
            }
            return Vec::new();
        }
        let target = self.hit_target(point);
        self.pressed = target;
        if contains(layout.grid, point) && self.focus != PortraitSelControl::SelectedItem {
            self.focus = PortraitSelControl::Grid;
            self.focused_item = None;
        }
        if contains(list_selection_hitbox(&layout), point) {
            self.combo_open = false;
            let previous = self.selected;
            self.selected = match target {
                HitTarget::Tile(index) => Some(index),
                _ => None,
            };
            self.validation_error = None;
            if self.selected.is_some() && self.selected != previous {
                self.ensure_selected_visible();
            }
            return Vec::new();
        }
        match target {
            HitTarget::Tile(_) => {}
            HitTarget::Location => {
                self.combo_open = true;
                self.combo_highlight = None;
            }
            HitTarget::SetPicture
            | HitTarget::SetBigIcon
            | HitTarget::Ok
            | HitTarget::Cancel
            | HitTarget::Close
            | HitTarget::LocationPopup
            | HitTarget::LocationOption(_) => {}
            HitTarget::None => self.combo_open = false,
        }
        Vec::new()
    }

    pub fn handle_pointer_right_down(&mut self, point: GuiPoint) -> Vec<PortraitSelAction> {
        self.pointer = Some(point);
        if self.combo_open {
            if contains(self.layout().location_popup, point) {
                self.combo_highlight = match self.hit_target(point) {
                    HitTarget::LocationOption(index) => Some(index),
                    _ => None,
                };
            } else {
                self.combo_open = false;
                self.combo_highlight = None;
                self.pressed = HitTarget::None;
                self.key_pressed = None;
            }
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, point: GuiPoint) -> Vec<PortraitSelAction> {
        self.pointer = Some(point);
        if self.scrollbar_dragging {
            self.set_scroll_from_scrollbar_pointer(point);
            self.scrollbar_dragging = false;
            self.pressed = HitTarget::None;
        }
        if self.scrollbar_arrow_captured {
            self.scrollbar_arrow_captured = false;
            self.scrollbar_arrow = 0;
            self.pressed = HitTarget::None;
        }
        let released = self.hit_target(point);
        let pressed = std::mem::replace(&mut self.pressed, HitTarget::None);
        match released {
            HitTarget::SetPicture => {
                self.set_picture = !self.set_picture;
                return Vec::new();
            }
            HitTarget::SetBigIcon => {
                self.set_big_icon = !self.set_big_icon;
                return Vec::new();
            }
            _ => {}
        }
        if pressed != released {
            return Vec::new();
        }
        match released {
            HitTarget::Close | HitTarget::Cancel => vec![PortraitSelAction::Cancel],
            HitTarget::Location => Vec::new(),
            HitTarget::LocationPopup
            | HitTarget::LocationOption(_)
            | HitTarget::SetPicture
            | HitTarget::SetBigIcon => Vec::new(),
            HitTarget::Ok => self.try_accept(),
            HitTarget::Tile(_) | HitTarget::None => Vec::new(),
        }
    }

    pub fn handle_pointer_double_click(&mut self, point: GuiPoint) -> Vec<PortraitSelAction> {
        self.pointer = Some(point);
        let layout = self.layout();
        if self.combo_open && contains(layout.location_popup, point) {
            self.combo_highlight = match self.hit_target(point) {
                HitTarget::LocationOption(index) => Some(index),
                _ => None,
            };
            return Vec::new();
        }
        if !contains(layout.grid, point) {
            return Vec::new();
        }
        self.pressed = HitTarget::None;
        if !contains(list_selection_hitbox(&layout), point) {
            return Vec::new();
        }
        self.selected = match self.hit_target(point) {
            HitTarget::Tile(index) => Some(index),
            _ => None,
        };
        self.validation_error = None;
        if self.selected.is_some() {
            self.ensure_selected_visible();
            return self.try_accept();
        }
        Vec::new()
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<PortraitSelAction> {
        self.handle_key_down_with_tab_direction(key, false)
    }

    pub fn handle_key_down_with_tab_direction(
        &mut self,
        key: KeyCode,
        backwards: bool,
    ) -> Vec<PortraitSelAction> {
        if self.combo_open {
            match key {
                KeyCode::Escape => {
                    self.combo_open = false;
                    self.combo_highlight = None;
                    return Vec::new();
                }
                KeyCode::Up => {
                    if !self.locations.is_empty() {
                        self.combo_highlight = Some(self.combo_highlight.map_or_else(
                            || self.locations.len() - 1,
                            |index| (index + self.locations.len() - 1) % self.locations.len(),
                        ));
                    }
                    return Vec::new();
                }
                KeyCode::Down => {
                    if !self.locations.is_empty() {
                        self.combo_highlight = Some(
                            self.combo_highlight
                                .map_or(0, |index| (index + 1) % self.locations.len()),
                        );
                    }
                    return Vec::new();
                }
                KeyCode::Enter => {
                    return self
                        .combo_highlight
                        .map_or_else(Vec::new, |index| self.choose_location(index));
                }
                KeyCode::Tab
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Space => return Vec::new(),
            }
        }

        match key {
            KeyCode::Escape => vec![PortraitSelAction::Cancel],
            KeyCode::Enter => self.try_accept(),
            KeyCode::Tab => {
                self.move_focus(backwards);
                Vec::new()
            }
            KeyCode::Up if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::Up);
                Vec::new()
            }
            KeyCode::Down if self.focus == PortraitSelControl::Location => {
                self.combo_open = true;
                self.combo_highlight = None;
                Vec::new()
            }
            KeyCode::Down if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::Down);
                Vec::new()
            }
            KeyCode::Left if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::Left);
                Vec::new()
            }
            KeyCode::Right if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::Right);
                Vec::new()
            }
            KeyCode::Home if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::Home);
                Vec::new()
            }
            KeyCode::End if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::End);
                Vec::new()
            }
            KeyCode::PageUp if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::PageUp);
                Vec::new()
            }
            KeyCode::PageDown if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::PageDown);
                Vec::new()
            }
            KeyCode::Space => self.activate_focus(),
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, key: KeyCode) -> Vec<PortraitSelAction> {
        if self.combo_open {
            self.key_pressed = None;
            return Vec::new();
        }
        if key != KeyCode::Space {
            return Vec::new();
        }
        let Some(pressed) = self.key_pressed.take() else {
            return Vec::new();
        };
        if pressed != self.focus {
            return Vec::new();
        }
        match pressed {
            PortraitSelControl::Close | PortraitSelControl::Cancel => {
                vec![PortraitSelAction::Cancel]
            }
            PortraitSelControl::Ok => self.try_accept(),
            PortraitSelControl::NoControl
            | PortraitSelControl::Location
            | PortraitSelControl::Grid
            | PortraitSelControl::SelectedItem
            | PortraitSelControl::SetPicture
            | PortraitSelControl::SetBigIcon => Vec::new(),
        }
    }

    pub fn handle_gamepad_low_down(&mut self) -> Vec<PortraitSelAction> {
        if self.combo_open {
            return self.handle_key_down(KeyCode::Enter);
        }
        match self.focus {
            PortraitSelControl::NoControl
            | PortraitSelControl::Grid
            | PortraitSelControl::SelectedItem => self.try_accept(),
            PortraitSelControl::Close
            | PortraitSelControl::Location
            | PortraitSelControl::SetPicture
            | PortraitSelControl::SetBigIcon
            | PortraitSelControl::Ok
            | PortraitSelControl::Cancel => self.activate_focus(),
        }
    }

    pub fn handle_gamepad_low_up(&mut self) -> Vec<PortraitSelAction> {
        self.handle_key_up(KeyCode::Space)
    }

    pub fn handle_gamepad_high_down(&mut self) -> Vec<PortraitSelAction> {
        if self.combo_open {
            self.combo_open = false;
            self.combo_highlight = None;
            Vec::new()
        } else {
            vec![PortraitSelAction::Cancel]
        }
    }

    pub fn handle_gamepad_direction(&mut self, key: KeyCode) -> Vec<PortraitSelAction> {
        if !matches!(
            key,
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
        ) {
            return Vec::new();
        }
        if self.combo_open {
            return match key {
                KeyCode::Up | KeyCode::Down => self.handle_key_down(key),
                KeyCode::Left | KeyCode::Right => Vec::new(),
                _ => unreachable!("direction key checked above"),
            };
        }
        match (self.focus, key) {
            (PortraitSelControl::Grid, _) => self.handle_key_down(key),
            (PortraitSelControl::Location, KeyCode::Down) => self.handle_key_down(key),
            (_, KeyCode::Left) => {
                self.move_focus(true);
                Vec::new()
            }
            (_, KeyCode::Right) => {
                self.move_focus(false);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub fn handle_wheel(&mut self, native_delta: i32) -> bool {
        let layout = self.layout();
        if native_delta == 0
            || self
                .pointer
                .is_some_and(|point| self.combo_open && contains(layout.location_popup, point))
            || !self
                .pointer
                .is_some_and(|point| contains(layout.grid_viewport, point))
        {
            return false;
        }
        let old = self.scroll_y;
        self.scroll_y = self
            .scroll_y
            .saturating_add(native_delta.saturating_neg())
            .clamp(0, self.max_scroll_y());
        self.sync_scrollbar_pin();
        old != self.scroll_y
    }

    /// Advances a held classic scrollbar arrow by one thumb pixel. Pinned
    /// `C4GUI::ScrollBar::DrawElement` performs this once per presentation
    /// (`C4GuiContainers.cpp:446-457`).
    pub fn tick_scrollbar(&mut self) -> bool {
        if self.scrollbar_arrow == 0 {
            return false;
        }
        let max_scroll = self.max_scroll_y();
        if max_scroll == 0 {
            return false;
        }
        let max_pin = self.max_scrollbar_pin();
        let previous_pin = self.scrollbar_pin;
        self.scrollbar_pin =
            (self.scrollbar_pin + i32::from(self.scrollbar_arrow)).clamp(0, max_pin);
        self.scroll_y = max_scroll.saturating_mul(self.scrollbar_pin) / max_pin.max(1);
        self.scrollbar_pin != previous_pin
    }

    fn activate_focus(&mut self) -> Vec<PortraitSelAction> {
        match self.focus {
            PortraitSelControl::Close | PortraitSelControl::Ok | PortraitSelControl::Cancel => {
                self.key_pressed = Some(self.focus);
                Vec::new()
            }
            PortraitSelControl::Location => {
                self.combo_open = !self.combo_open;
                self.combo_highlight = None;
                Vec::new()
            }
            PortraitSelControl::NoControl
            | PortraitSelControl::Grid
            | PortraitSelControl::SelectedItem => Vec::new(),
            PortraitSelControl::SetPicture => {
                self.set_picture = !self.set_picture;
                Vec::new()
            }
            PortraitSelControl::SetBigIcon => {
                self.set_big_icon = !self.set_big_icon;
                Vec::new()
            }
        }
    }

    fn try_accept(&mut self) -> Vec<PortraitSelAction> {
        let Some(choice) = self.selected_item().map(|item| item.choice.clone()) else {
            return vec![PortraitSelAction::SelectionRequired];
        };
        vec![PortraitSelAction::Accept(PortraitSelCommit {
            choice,
            set_picture: self.set_picture,
            set_big_icon: self.set_big_icon,
        })]
    }

    fn choose_location(&mut self, index: usize) -> Vec<PortraitSelAction> {
        self.combo_open = false;
        self.combo_highlight = None;
        let Some(location) = self.locations.get(index).cloned() else {
            return Vec::new();
        };
        self.current_location = index;
        self.install_entries(Vec::new());
        vec![PortraitSelAction::ChangeLocation {
            index,
            path: location.path,
        }]
    }

    fn move_focus(&mut self, backwards: bool) {
        if self.focus == PortraitSelControl::NoControl {
            self.focus = if backwards {
                PortraitSelControl::Cancel
            } else {
                PortraitSelControl::Close
            };
            self.focused_item = None;
            return;
        }
        if self.focus == PortraitSelControl::SelectedItem {
            let selected_is_next =
                self.focused_item
                    .zip(self.selected)
                    .is_some_and(|(focused, selected)| {
                        if backwards {
                            selected < focused
                        } else {
                            selected > focused
                        }
                    });
            if selected_is_next {
                self.focused_item = self.selected;
                return;
            }
        }
        let mut position = PortraitSelControl::ORDER
            .iter()
            .position(|control| *control == self.focus)
            .unwrap_or(0);
        let count = PortraitSelControl::ORDER.len();
        loop {
            position = if backwards {
                (position + count - 1) % count
            } else {
                (position + 1) % count
            };
            let candidate = PortraitSelControl::ORDER[position];
            if candidate != PortraitSelControl::SelectedItem || self.selected.is_some() {
                self.focus = candidate;
                self.focused_item = (candidate == PortraitSelControl::SelectedItem)
                    .then_some(self.selected)
                    .flatten();
                break;
            }
        }
        if self.focus == PortraitSelControl::Grid
            && self.selected.is_none()
            && !self.items.is_empty()
        {
            self.selected = Some(0);
            self.validation_error = None;
            self.ensure_selected_visible();
        }
    }

    fn move_grid(&mut self, movement: GridMove) {
        if self.items.is_empty() {
            self.selected = None;
            return;
        }
        let layout = self.layout();
        let columns = layout.columns.max(1);
        let last = self.items.len() - 1;
        if matches!(movement, GridMove::PageUp | GridMove::PageDown) {
            let next = self.move_grid_page(matches!(movement, GridMove::PageDown));
            if next != self.selected {
                self.selected = next;
                self.validation_error = None;
                self.ensure_selected_visible();
            }
            return;
        }
        let Some(current) = self.selected else {
            self.selected = Some(
                if matches!(movement, GridMove::Up | GridMove::Left | GridMove::End) {
                    last
                } else {
                    0
                },
            );
            self.validation_error = None;
            self.ensure_selected_visible();
            return;
        };
        let next = match movement {
            GridMove::Left => current.saturating_sub(1),
            GridMove::Right => (current + 1).min(last),
            GridMove::Up => current.checked_sub(columns).unwrap_or(current),
            GridMove::Down => current
                .checked_add(columns)
                .filter(|index| *index <= last)
                .unwrap_or(current),
            GridMove::Home => 0,
            GridMove::End => last,
            GridMove::PageUp | GridMove::PageDown => unreachable!("handled above"),
        };
        if next == current {
            return;
        }
        self.selected = Some(next);
        self.validation_error = None;
        self.ensure_selected_visible();
    }

    fn move_grid_page(&mut self, down: bool) -> Option<usize> {
        let layout = self.layout();
        let grid = self.grid_content_layout(layout.columns);
        let last = grid.items.len().checked_sub(1)?;
        let mut next = self.selected.unwrap_or(if down { 0 } else { last });
        if down {
            let Some(adjacent) = next.checked_add(1).filter(|index| *index <= last) else {
                return Some(next);
            };
            next = adjacent;
            if grid_item_fully_visible(&grid, next, self.scroll_y, layout.grid_viewport.h) {
                while next < last
                    && grid_item_fully_visible(
                        &grid,
                        next + 1,
                        self.scroll_y,
                        layout.grid_viewport.h,
                    )
                {
                    next += 1;
                }
            } else {
                self.scroll_y = self
                    .scroll_y
                    .saturating_add(layout.grid_viewport.h)
                    .clamp(0, self.max_scroll_y());
                self.sync_scrollbar_pin();
                next = last;
                while next > 0
                    && !grid_item_fully_visible(&grid, next, self.scroll_y, layout.grid_viewport.h)
                {
                    next -= 1;
                }
            }
        } else if next > 0 {
            next -= 1;
            if grid_item_fully_visible(&grid, next, self.scroll_y, layout.grid_viewport.h) {
                while next > 0
                    && grid_item_fully_visible(
                        &grid,
                        next - 1,
                        self.scroll_y,
                        layout.grid_viewport.h,
                    )
                {
                    next -= 1;
                }
            } else {
                self.scroll_y = self
                    .scroll_y
                    .saturating_sub(layout.grid_viewport.h)
                    .clamp(0, self.max_scroll_y());
                self.sync_scrollbar_pin();
                next = 0;
                while next < last
                    && !grid_item_fully_visible(&grid, next, self.scroll_y, layout.grid_viewport.h)
                {
                    next += 1;
                }
            }
        }
        Some(next)
    }

    fn ensure_selected_visible(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let layout = self.layout();
        let grid = self.grid_content_layout(layout.columns);
        let Some(item) = grid.items.get(selected) else {
            return;
        };
        let top = item.rect.y;
        let bottom = top.saturating_add(item.rect.h);
        if self.scroll_y > top {
            self.scroll_y = top;
        } else if self.scroll_y.saturating_add(layout.grid_viewport.h) < bottom {
            self.scroll_y = bottom.saturating_sub(layout.grid_viewport.h);
        }
        self.scroll_y = self.scroll_y.clamp(0, self.max_scroll_y());
        self.sync_scrollbar_pin();
    }

    fn max_scroll_y(&self) -> i32 {
        let layout = self.layout();
        self.grid_content_layout(layout.columns)
            .content_height
            .saturating_sub(layout.grid_viewport.h)
            .max(0)
    }

    fn set_scroll_from_scrollbar_pointer(&mut self, point: GuiPoint) {
        let layout = self.layout();
        let pin_range = self.max_scrollbar_pin();
        if pin_range == 0 {
            return;
        }
        self.scrollbar_pin =
            (point.y.floor() as i32 - layout.grid_scrollbar.y - 16 - 8).clamp(0, pin_range);
        self.scroll_y = self.max_scroll_y().saturating_mul(self.scrollbar_pin) / pin_range;
    }

    fn max_scrollbar_pin(&self) -> i32 {
        (self.layout().grid_scrollbar.h - 48).max(0)
    }

    fn sync_scrollbar_pin(&mut self) {
        let max_scroll = self.max_scroll_y();
        self.scrollbar_pin = if max_scroll == 0 {
            0
        } else {
            self.max_scrollbar_pin().saturating_mul(self.scroll_y) / max_scroll
        };
    }

    fn scrollbar_arrow_at(&self, point: GuiPoint) -> i8 {
        let bar = self.layout().grid_scrollbar;
        if !contains(bar, point) {
            return 0;
        }
        let relative_y = point.y.floor() as i32 - bar.y;
        if relative_y < 16 {
            -1
        } else if relative_y >= bar.h - 16 {
            1
        } else {
            0
        }
    }

    fn begin_scrollbar_pointer(&mut self, point: GuiPoint) {
        let arrow = self.scrollbar_arrow_at(point);
        if arrow != 0 {
            self.scrollbar_arrow_captured = true;
            self.scrollbar_arrow = arrow;
        } else if self.max_scrollbar_pin() > 0 {
            self.scrollbar_arrow_captured = false;
            self.set_scroll_from_scrollbar_pointer(point);
            self.scrollbar_dragging = true;
        }
    }

    fn grid_content_layout(&self, columns: usize) -> PortraitGridContentLayout {
        self.grid_layout
            .borrow()
            .as_ref()
            .filter(|layout| {
                layout.columns == columns.max(1) && layout.items.len() == self.items.len()
            })
            .cloned()
            .unwrap_or_else(|| portrait_grid_content_layout(&self.items, columns, None))
    }

    fn update_grid_content_layout(
        &mut self,
        columns: usize,
        font: &ClonkFont,
    ) -> PortraitGridContentLayout {
        let layout = portrait_grid_content_layout(&self.items, columns, Some(font));
        let changed = self.grid_layout.borrow().as_ref() != Some(&layout);
        if changed {
            self.grid_layout.replace(Some(layout.clone()));
            self.scroll_y = self.scroll_y.clamp(0, self.max_scroll_y());
            self.ensure_selected_visible();
            self.sync_scrollbar_pin();
        }
        layout
    }

    fn layout(&self) -> PortraitSelLayout {
        portrait_sel_layout(self.width, self.height, self.locations.len())
    }

    fn hit_target(&self, point: GuiPoint) -> HitTarget {
        let layout = self.layout();
        if self.combo_open {
            for (index, rect) in layout.location_options.iter().enumerate() {
                if contains(*rect, point) {
                    return HitTarget::LocationOption(index);
                }
            }
            if contains(layout.location_popup, point) {
                return HitTarget::LocationPopup;
            }
        }
        if contains(layout.close, point) {
            return HitTarget::Close;
        }
        if contains(layout.location_combo, point) {
            return HitTarget::Location;
        }
        if contains_inclusive(checkbox_square(layout.set_picture), point) {
            return HitTarget::SetPicture;
        }
        if contains_inclusive(checkbox_square(layout.set_big_icon), point) {
            return HitTarget::SetBigIcon;
        }
        if contains(layout.ok, point) {
            return HitTarget::Ok;
        }
        if contains(layout.cancel, point) {
            return HitTarget::Cancel;
        }
        if contains(list_selection_hitbox(&layout), point) {
            let relative_x = point.x.floor() as i32 - layout.grid_viewport.x;
            let grid = self.grid_content_layout(layout.columns);
            let max_scroll = grid
                .content_height
                .saturating_sub(layout.grid_viewport.h)
                .max(0);
            let relative_y = point.y.floor() as i32 - layout.grid_viewport.y
                + self.scroll_y.clamp(0, max_scroll);
            if let Some(index) = grid.items.iter().position(|item| {
                relative_x >= item.rect.x
                    && relative_y >= item.rect.y
                    && relative_x < item.rect.x + item.rect.w
                    && relative_y < item.rect.y + item.rect.h
            }) {
                return HitTarget::Tile(index);
            }
        }
        HitTarget::None
    }

    pub fn render(
        &mut self,
        surface: &mut Surface,
        resources: PortraitSelResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        self.render_dialog(surface, resources, gamma);
        self.render_location_popup(surface, resources, gamma);
    }

    pub fn render_dialog(
        &mut self,
        surface: &mut Surface,
        resources: PortraitSelResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        let layout = portrait_sel_layout(
            surface.width() as i32,
            surface.height() as i32,
            self.locations.len(),
        );
        resources.skin.draw_dialog(surface, layout.bounds, gamma);
        resources.skin.draw_caption_with_right_indent(
            surface,
            layout.caption,
            &self.caption(),
            &resources.fonts.text,
            [255, 255, 255, 255],
            TextAlign::Left,
            layout.close.w + 4,
            gamma,
        );
        let close_highlighted = self
            .pointer
            .is_some_and(|point| contains(layout.close, point))
            || (!self.combo_open && self.focus == PortraitSelControl::Close);
        if close_highlighted {
            draw_highlight(surface, layout.close, resources.button_highlight, gamma);
        }
        draw_icon_phase(surface, layout.close, resources.icons, 34, gamma);
        if (self.pressed == HitTarget::Close && close_highlighted)
            || self.key_pressed == Some(PortraitSelControl::Close)
        {
            draw_highlight(surface, layout.close, resources.button_highlight, gamma);
        }

        resources.fonts.text.draw_with_gamma(
            surface,
            layout.location_label.x,
            layout.location_label.y,
            "Location:",
            [255, 255, 255, 255],
            TextAlign::Left,
            false,
            gamma,
        );
        let location_text = self
            .current_location()
            .map(|location| location.label.as_str())
            .unwrap_or("No locations");
        draw_combo_box(
            surface,
            layout.location_combo,
            location_text,
            self.combo_open,
            self.focus == PortraitSelControl::Location
                || self
                    .pointer
                    .is_some_and(|point| contains(layout.location_combo, point)),
            resources,
            gamma,
        );

        draw_engine_box(
            surface,
            layout.grid.x,
            layout.grid.y,
            layout.grid.x + layout.grid.w - 1,
            layout.grid.y + layout.grid.h - 1,
            0x7f00_0000,
            gamma,
        );
        let grid_content = self.update_grid_content_layout(layout.columns, &resources.fonts.mini);
        let max_scroll_y = grid_content
            .content_height
            .saturating_sub(layout.grid_viewport.h)
            .max(0);
        let scroll_y = self.scroll_y.clamp(0, max_scroll_y);
        with_surface_clip(surface, layout.grid_viewport, |surface| {
            for (index, item_layout) in grid_content.items.iter().enumerate() {
                let tile = IntRect::new(
                    layout.grid_viewport.x + item_layout.rect.x,
                    layout.grid_viewport.y + item_layout.rect.y - scroll_y,
                    item_layout.rect.w,
                    item_layout.rect.h,
                );
                if tile.y >= layout.grid_viewport.y + layout.grid_viewport.h
                    || tile.y + tile.h <= layout.grid_viewport.y
                {
                    continue;
                }
                self.draw_item(
                    surface,
                    &self.items[index],
                    index,
                    &item_layout.wrapped_label,
                    tile,
                    resources,
                    gamma,
                );
            }
        });
        draw_scrollbar(
            surface,
            layout.grid_scrollbar,
            resources.scroll,
            self.scrollbar_pin,
            max_scroll_y,
            self.scrollbar_arrow,
            gamma,
        );

        resources.fonts.text.draw_with_gamma(
            surface,
            layout.import_label.x,
            layout.import_label.y,
            "Import image as:",
            [255, 255, 255, 255],
            TextAlign::Left,
            false,
            gamma,
        );
        for (control, rect, selected, label) in [
            (
                PortraitSelControl::SetPicture,
                layout.set_picture,
                self.set_picture,
                "Player image",
            ),
            (
                PortraitSelControl::SetBigIcon,
                layout.set_big_icon,
                self.set_big_icon,
                "Lobby-Icon",
            ),
        ] {
            draw_checkbox(
                surface,
                rect,
                selected,
                label,
                (!self.combo_open && self.focus == control)
                    || self
                        .pointer
                        .is_some_and(|point| contains_inclusive(checkbox_square(rect), point)),
                resources,
                gamma,
            );
        }
        for (control, target, rect, label) in [
            (PortraitSelControl::Ok, HitTarget::Ok, layout.ok, "OK"),
            (
                PortraitSelControl::Cancel,
                HitTarget::Cancel,
                layout.cancel,
                "Cancel",
            ),
        ] {
            let pointer_over = self.pointer.is_some_and(|point| contains(rect, point));
            resources.skin.draw_button(
                surface,
                rect,
                label,
                resources.fonts,
                ClassicButtonState {
                    pressed: (self.pressed == target && pointer_over)
                        || self.key_pressed == Some(control),
                    highlighted: (!self.combo_open && self.focus == control) || pointer_over,
                },
                gamma,
            );
        }
        if let Some(error) = self.validation_error.as_deref() {
            resources.fonts.mini.draw_with_gamma(
                surface,
                layout.bounds.x + layout.bounds.w / 2,
                layout.ok.y - resources.fonts.mini.line_height - 2,
                error,
                [255, 80, 60, 255],
                TextAlign::Center,
                false,
                gamma,
            );
        }
    }

    pub fn render_location_popup(
        &self,
        surface: &mut Surface,
        resources: PortraitSelResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        if !self.combo_open {
            return;
        }
        let layout = portrait_sel_layout(
            surface.width() as i32,
            surface.height() as i32,
            self.locations.len(),
        );
        draw_engine_box(
            surface,
            layout.location_popup.x,
            layout.location_popup.y,
            layout.location_popup.x + layout.location_popup.w - 1,
            layout.location_popup.y + layout.location_popup.h - 1,
            0x4f3f_1a00,
            gamma,
        );
        if let Some(rect) = self
            .combo_highlight
            .and_then(|index| layout.location_options.get(index))
        {
            draw_engine_box(
                surface,
                rect.x,
                rect.y,
                rect.x + rect.w - 1,
                rect.y + rect.h - 1,
                0xafaf_0000,
                gamma,
            );
        }
        draw_3d_frame(surface, layout.location_popup, gamma);
        for (location, rect) in self.locations.iter().zip(layout.location_options.iter()) {
            resources.fonts.text.draw_with_gamma(
                surface,
                rect.x + rect.h + 2,
                rect.y,
                &location.label,
                [255, 255, 255, 255],
                TextAlign::Left,
                true,
                gamma,
            );
        }
    }

    fn caption(&self) -> String {
        self.current_location().map_or_else(
            || "Select Portrait".to_string(),
            |location| format!("Select Portrait [{}]", location.path.display()),
        )
    }

    fn draw_item(
        &self,
        surface: &mut Surface,
        item: &PortraitItem,
        index: usize,
        label: &str,
        tile: IntRect,
        resources: PortraitSelResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        let picture = tile.with_height(PREVIEW_SIZE.min(tile.h));
        if self.selected == Some(index) {
            draw_engine_box(
                surface,
                tile.x,
                tile.y,
                tile.x + tile.w - 1,
                tile.y + tile.h - 1,
                if !self.combo_open && self.focus == PortraitSelControl::Grid {
                    0xafaf_0000
                } else {
                    0xaf7f_7f7f
                },
                gamma,
            );
        }
        match &item.thumbnail {
            PortraitThumbnail::Ready(image) => {
                let fitted = aspect_fit(image, picture);
                crate::draw_image_bilinear(surface, &gui_rect(fitted), image, gamma);
            }
            PortraitThumbnail::Pending | PortraitThumbnail::Loading => {
                resources.fonts.mini.draw_with_gamma(
                    surface,
                    picture.x + picture.w / 2,
                    picture.y + (picture.h - resources.fonts.mini.line_height) / 2,
                    "Loading...",
                    [255, 255, 255, 255],
                    TextAlign::Center,
                    false,
                    gamma,
                );
            }
            PortraitThumbnail::None | PortraitThumbnail::Failed => {
                let size = 32.min(picture.w).min(picture.h);
                let marker = IntRect::new(
                    picture.x + (picture.w - size) / 2,
                    picture.y + (picture.h - size) / 2,
                    size,
                    size,
                );
                draw_facet_nearest(
                    surface,
                    resources.control,
                    SurfaceRect::new(160, 100, 32, 32),
                    SurfaceRect::new(marker.x, marker.y, marker.w as u32, marker.h as u32),
                    gamma,
                );
            }
        }
        if tile.h > PREVIEW_SIZE {
            resources.fonts.mini.draw_with_gamma(
                surface,
                tile.x + tile.w / 2,
                tile.y + PREVIEW_SIZE,
                label,
                [255, 255, 255, 255],
                TextAlign::Center,
                false,
                gamma,
            );
        }
    }
}

fn portrait_grid_content_layout(
    items: &[PortraitItem],
    columns: usize,
    font: Option<&ClonkFont>,
) -> PortraitGridContentLayout {
    let columns = columns.max(1);
    let mut layouts = Vec::with_capacity(items.len());
    let mut row_y = 0_i32;
    let mut row_height = 0_i32;
    for (index, item) in items.iter().enumerate() {
        let column = index % columns;
        if column == 0 && index > 0 {
            row_y = row_y.saturating_add(row_height);
            row_height = 0;
        }
        let wrapped_label = font.map_or_else(
            || item.label.clone(),
            |font| {
                crate::message_dialog::break_message_with_options(
                    font,
                    &item.label,
                    PREVIEW_SIZE.saturating_sub(6),
                    crate::message_dialog::BreakMessageOptions {
                        markup: false,
                        ..crate::message_dialog::BreakMessageOptions::default()
                    },
                )
            },
        );
        let label_height = font.map_or(MINI_LINE_HEIGHT, |font| {
            font.measure(&wrapped_label, false).1.max(1)
        });
        let item_height = PREVIEW_SIZE.saturating_add(label_height);
        row_height = row_height.max(item_height);
        layouts.push(PortraitGridItemLayout {
            rect: IntRect::new(
                i32::try_from(column)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(PREVIEW_SIZE),
                row_y,
                PREVIEW_SIZE,
                item_height,
            ),
            wrapped_label,
        });
    }
    let content_height = if layouts.is_empty() {
        0
    } else {
        row_y.saturating_add(row_height)
    };
    PortraitGridContentLayout {
        columns,
        items: layouts,
        content_height,
    }
}

fn grid_item_fully_visible(
    layout: &PortraitGridContentLayout,
    index: usize,
    scroll_y: i32,
    viewport_height: i32,
) -> bool {
    layout.items.get(index).is_some_and(|item| {
        scroll_y <= item.rect.y
            && scroll_y.saturating_add(viewport_height) >= item.rect.y.saturating_add(item.rect.h)
    })
}

#[derive(Clone, Copy)]
pub struct PortraitSelResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
    pub icons: &'a ImageData,
    pub context: &'a ImageData,
    pub checkbox: &'a ImageData,
    pub scroll: &'a ImageData,
    pub control: &'a ImageData,
    pub button_highlight: &'a ImageData,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortraitSelLayout {
    pub bounds: IntRect,
    pub caption: IntRect,
    pub close: IntRect,
    pub location_label: IntRect,
    pub location_combo: IntRect,
    pub location_popup: IntRect,
    pub location_options: Vec<IntRect>,
    pub grid: IntRect,
    pub grid_client: IntRect,
    pub grid_viewport: IntRect,
    pub grid_scrollbar: IntRect,
    pub import_label: IntRect,
    pub set_picture: IntRect,
    pub set_big_icon: IntRect,
    pub ok: IntRect,
    pub cancel: IntRect,
    pub columns: usize,
    pub visible_rows: usize,
}

pub fn portrait_sel_layout(
    screen_width: i32,
    screen_height: i32,
    location_count: usize,
) -> PortraitSelLayout {
    let width = (i64::from(screen_width) * 2 / 3 + 10)
        .clamp(i64::from(MIN_WIDTH), i64::from(MAX_WIDTH)) as i32;
    let height = (i64::from(screen_height) * 2 / 3 + 10)
        .clamp(i64::from(MIN_HEIGHT), i64::from(MAX_HEIGHT)) as i32;
    let bounds = IntRect::new(
        ((i64::from(screen_width) - i64::from(width)) / 2) as i32,
        ((i64::from(screen_height) - i64::from(height)) / 2) as i32,
        width,
        height,
    );
    let caption = IntRect::new(bounds.x, bounds.y, bounds.w, CAPTION_HEIGHT);
    let close = IntRect::new(bounds.x + bounds.w - 20, bounds.y + 4, 16, 16);
    let location_y = bounds.y + CAPTION_HEIGHT + 14;
    let location_label = IntRect::new(bounds.x + 20, location_y, 57, TEXT_LINE_HEIGHT);
    let location_combo = IntRect::new(
        location_label.x + location_label.w + 20,
        location_y,
        bounds.x + bounds.w - 20 - (location_label.x + location_label.w + 20),
        CONTROL_HEIGHT,
    );
    let grid = IntRect::new(
        bounds.x + 10,
        bounds.y + CAPTION_HEIGHT + TEXT_LINE_HEIGHT + 42,
        bounds.w - 20,
        bounds.h - CAPTION_HEIGHT - 2 * TEXT_LINE_HEIGHT - 100,
    );
    let grid_client = IntRect::new(
        grid.x + 3,
        grid.y + 3,
        (grid.w - 6).max(0),
        (grid.h - 6).max(0),
    );
    let grid_viewport = grid_client.with_width((grid_client.w - SCROLLBAR_WIDTH).max(0));
    let grid_scrollbar = IntRect::new(
        grid_viewport.x + grid_viewport.w,
        grid_client.y,
        SCROLLBAR_WIDTH,
        grid_client.h,
    );
    let columns = (grid_viewport.w / PREVIEW_SIZE).max(1) as usize;
    let visible_rows = ((grid_viewport.h + TILE_HEIGHT - 1) / TILE_HEIGHT).max(1) as usize;
    let options_y = bounds.y + bounds.h - TEXT_LINE_HEIGHT - 44;
    let import_label = IntRect::new(bounds.x + 10, options_y, 106, TEXT_LINE_HEIGHT);
    let option_width = ((bounds.w - 10) / 3 - 10).max(1);
    let set_picture = IntRect::new(
        bounds.x + option_width + 20,
        options_y,
        option_width,
        TEXT_LINE_HEIGHT,
    );
    let set_big_icon = IntRect::new(
        bounds.x + option_width * 2 + 30,
        options_y,
        option_width,
        TEXT_LINE_HEIGHT,
    );
    let buttons_y = bounds.y + bounds.h - 36;
    let ok = IntRect::new(
        bounds.x + (bounds.w - 260) / 2,
        buttons_y,
        BUTTON_WIDTH,
        BUTTON_HEIGHT,
    );
    let cancel = IntRect::new(
        bounds.x + bounds.w / 2 + 10,
        buttons_y,
        BUTTON_WIDTH,
        BUTTON_HEIGHT,
    );
    let location_count_i32 = i32::try_from(location_count).unwrap_or(i32::MAX);
    let popup_content_height = location_count_i32
        .saturating_mul(TEXT_LINE_HEIGHT)
        .saturating_add(
            location_count_i32
                .saturating_sub(1)
                .saturating_mul(CONTEXT_ROW_SPACING),
        )
        .max(8);
    let popup_height = popup_content_height.saturating_add(10);
    let location_popup = IntRect::new(
        location_combo.x,
        location_combo.y + location_combo.h,
        location_combo.w,
        popup_height,
    );
    let location_options = (0..location_count)
        .map(|index| {
            IntRect::new(
                location_popup.x + 5,
                location_popup.y.saturating_add(5).saturating_add(
                    (TEXT_LINE_HEIGHT + CONTEXT_ROW_SPACING)
                        .saturating_mul(i32::try_from(index).unwrap_or(i32::MAX)),
                ),
                location_popup.w - 10,
                TEXT_LINE_HEIGHT,
            )
        })
        .collect();
    PortraitSelLayout {
        bounds,
        caption,
        close,
        location_label,
        location_combo,
        location_popup,
        location_options,
        grid,
        grid_client,
        grid_viewport,
        grid_scrollbar,
        import_label,
        set_picture,
        set_big_icon,
        ok,
        cancel,
        columns,
        visible_rows,
    }
}

#[derive(Clone, Copy)]
enum GridMove {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

fn list_selection_hitbox(layout: &PortraitSelLayout) -> IntRect {
    // ListBox checks its zero-origin ScrollWindow bounds before subtracting
    // the three-pixel client margins (`C4GuiListBox.cpp:149-162`).
    IntRect::new(
        layout.grid.x,
        layout.grid.y,
        layout.grid_viewport.w,
        layout.grid_client.h,
    )
}

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn contains_inclusive(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x <= (rect.x + rect.w) as f32
        && point.y <= (rect.y + rect.h) as f32
}

fn draw_highlight(
    surface: &mut Surface,
    rect: IntRect,
    highlight: &ImageData,
    gamma: Option<&GammaRamp>,
) {
    crate::draw_image_bilinear_additive(
        surface,
        &GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        highlight,
        gamma,
    );
}

fn draw_combo_box(
    surface: &mut Surface,
    rect: IntRect,
    text: &str,
    open: bool,
    highlighted: bool,
    resources: PortraitSelResources<'_>,
    gamma: Option<&GammaRamp>,
) {
    draw_engine_box(
        surface,
        rect.x,
        rect.y,
        rect.x + rect.w - 1,
        rect.y + rect.h - 1,
        crate::classic_gui::STANDARD_BACKGROUND_COLOR,
        gamma,
    );
    draw_3d_frame(surface, rect, gamma);
    let arrow_width = resources
        .context
        .height()
        .min(resources.context.width() / 2) as i32;
    let arrow_x = rect.x + rect.w - arrow_width - 1;
    crate::draw_image_strip(
        surface,
        arrow_x,
        rect.y + (rect.h - arrow_width) / 2,
        resources.context,
        u32::from(open) * arrow_width as u32,
        0,
        arrow_width as u32,
        arrow_width as u32,
        gamma,
    );
    with_surface_clip(
        surface,
        IntRect::new(rect.x, rect.y, (arrow_x - rect.x).max(0), rect.h),
        |surface| {
            resources.fonts.text.draw_with_gamma(
                surface,
                rect.x + arrow_width + 2,
                rect.y + (rect.h - resources.fonts.text.line_height) / 2,
                text,
                [255, 255, 255, 255],
                TextAlign::Left,
                false,
                gamma,
            );
        },
    );
    if open || highlighted {
        draw_highlight(surface, rect, resources.button_highlight, gamma);
    }
}

fn draw_checkbox(
    surface: &mut Surface,
    rect: IntRect,
    checked: bool,
    label: &str,
    highlighted: bool,
    resources: PortraitSelResources<'_>,
    gamma: Option<&GammaRamp>,
) {
    let square = checkbox_square(rect);
    let cell = resources.checkbox.height();
    draw_facet_stretch(
        surface,
        resources.checkbox,
        (
            (u32::from(checked) * cell) as f32,
            0.0,
            cell as f32,
            cell as f32,
        ),
        (
            square.x as f32,
            square.y as f32,
            square.w as f32,
            square.h as f32,
        ),
        gamma,
    );
    resources.fonts.text.draw_with_gamma(
        surface,
        rect.x + rect.h + 4,
        rect.y + (rect.h - resources.fonts.text.line_height).max(0) / 2,
        label,
        [255, 255, 255, 255],
        TextAlign::Left,
        true,
        gamma,
    );
    if highlighted {
        let size = rect.h / 2;
        draw_highlight(
            surface,
            IntRect::new(rect.x + rect.h / 4, rect.y + rect.h / 4, size, size),
            resources.button_highlight,
            gamma,
        );
    }
}

fn draw_scrollbar(
    surface: &mut Surface,
    bar: IntRect,
    scroll: &ImageData,
    scrollbar_pin: i32,
    max_scroll_y: i32,
    scrollbar_arrow: i8,
    gamma: Option<&GammaRamp>,
) {
    let top_x = if scrollbar_arrow < 0 { 16 } else { 0 };
    let bottom_x = if scrollbar_arrow > 0 { 16 } else { 0 };
    crate::draw_image_strip(surface, bar.x, bar.y, scroll, top_x, 0, 16, 16, gamma);
    let mut y = 16;
    while y < bar.h - 5 {
        let tile_height = 16.min(bar.h - 5 - y).max(0) as u32;
        if tile_height == 0 {
            break;
        }
        crate::draw_image_strip(
            surface,
            bar.x,
            bar.y + y,
            scroll,
            0,
            16,
            16,
            tile_height,
            gamma,
        );
        y += 16;
    }
    crate::draw_image_strip(
        surface,
        bar.x,
        bar.y + bar.h - 16,
        scroll,
        bottom_x,
        32,
        16,
        16,
        gamma,
    );
    if max_scroll_y > 0 && bar.h > 48 {
        crate::draw_image_strip(
            surface,
            bar.x,
            bar.y + 16 + scrollbar_pin.clamp(0, bar.h - 48),
            scroll,
            16,
            16,
            16,
            16,
            gamma,
        );
    }
}

fn checkbox_square(rect: IntRect) -> IntRect {
    rect.with_width(rect.h)
}

fn draw_icon_phase(
    surface: &mut Surface,
    rect: IntRect,
    icons: &ImageData,
    phase: u32,
    gamma: Option<&GammaRamp>,
) {
    let columns = icons.width() / 40;
    if columns == 0 || icons.height() < 40 {
        return;
    }
    let source_x = phase % columns * 40;
    let source_y = phase / columns * 40;
    if source_y.saturating_add(40) > icons.height() {
        return;
    }
    draw_facet_stretch(
        surface,
        icons,
        (source_x as f32, source_y as f32, 40.0, 40.0),
        (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32),
        gamma,
    );
}

fn gui_rect(rect: IntRect) -> GuiRect {
    GuiRect::new(rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32)
}

fn aspect_fit(image: &ImageData, rect: IntRect) -> IntRect {
    let width = image.width().max(1) as i64;
    let height = image.height().max(1) as i64;
    let (fitted_w, fitted_h) = if width * i64::from(rect.h) > height * i64::from(rect.w) {
        (rect.w, (i64::from(rect.w) * height / width).max(1) as i32)
    } else {
        ((i64::from(rect.h) * width / height).max(1) as i32, rect.h)
    };
    IntRect::new(
        rect.x + (rect.w - fitted_w) / 2,
        rect.y + (rect.h - fitted_h) / 2,
        fitted_w,
        fitted_h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn test_controller() -> PortraitSelController {
        PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            Vec::new(),
            true,
            true,
        )
    }

    fn render_test_controller(controller: &mut PortraitSelController) -> Surface {
        let caption = crate::test_support::load_graphics_png("GUICaption.png");
        let button = crate::test_support::load_graphics_png("GUIButton.png");
        let button_down = crate::test_support::load_graphics_png("GUIButtonDown.png");
        let highlight = crate::test_support::load_graphics_png("GUIButtonHighlight.png");
        let icons = crate::test_support::load_graphics_png("GUIIcons.png");
        let context = crate::test_support::load_graphics_png("GUIContext.png");
        let checkbox = crate::test_support::load_graphics_png("GUICheckbox.png");
        let scroll = crate::test_support::load_graphics_png("GUIScroll.png");
        let control = crate::test_support::load_graphics_png("Control.png");
        let fonts = crate::test_support::endeavour_font_set();
        let mut surface = Surface::new(640, 480, clonk_graphics::PixelFormat::Rgba8888);
        controller.render(
            &mut surface,
            PortraitSelResources {
                skin: ClassicGuiSkin::new(&caption, &button, &button_down, Some(&highlight)),
                fonts: &fonts,
                icons: &icons,
                context: &context,
                checkbox: &checkbox,
                scroll: &scroll,
                control: &control,
                button_highlight: &highlight,
            },
            None,
        );
        surface
    }

    #[test]
    fn portrait_selector_layout_matches_cpp_at_1152x723() {
        // Pinned C++ `C4FileSelDlg.cpp:113-169` lays out a 600x492 dialog
        // through `ComponentAligner`; `C4GuiDialogs.cpp:399-421` supplies the
        // 23px caption/16px close button, and `C4GuiListBox.h:119-123` plus
        // `C4GuiContainers.cpp:477-490` reserve the list margins/scrollbar.
        let layout = portrait_sel_layout(1152, 723, 4);
        let rect = |x, y, w, h| IntRect::new(x, y, w, h);

        assert_eq!(layout.bounds, rect(276, 115, 600, 492));
        assert_eq!(layout.caption, rect(276, 115, 600, 23));
        assert_eq!(layout.close, rect(856, 119, 16, 16));
        assert_eq!(layout.location_label, rect(296, 152, 57, 22));
        assert_eq!(layout.location_combo, rect(373, 152, 483, 26));
        assert_eq!(layout.grid, rect(286, 202, 580, 325));
        assert_eq!(layout.import_label, rect(286, 541, 106, 22));
        assert_eq!(layout.set_picture, rect(482, 541, 186, 22));
        assert_eq!(layout.set_big_icon, rect(678, 541, 186, 22));
        assert_eq!(layout.ok, rect(446, 571, 120, 32));
        assert_eq!(layout.cancel, rect(586, 571, 120, 32));
        assert_eq!(layout.columns, 5);
        assert_eq!(layout.visible_rows, 3);
    }

    #[test]
    fn portrait_selector_layout_handles_extreme_screen_extents() {
        // FileSel computes two-thirds of the screen before BoundBy clamps the
        // dialog (`C4FileSelDlg.cpp:113-115`). Widening the intermediate keeps
        // the same ordinary integer result without overflowing public resize
        // inputs.
        let layout = portrait_sel_layout(i32::MAX, i32::MAX, 0);

        assert_eq!(layout.bounds.w, MAX_WIDTH);
        assert_eq!(layout.bounds.h, MAX_HEIGHT);
        assert_eq!(layout.bounds.x, 1_073_741_523);
        assert_eq!(layout.bounds.y, 1_073_741_573);
    }

    #[test]
    fn portrait_selector_appends_no_portrait_after_file_items() {
        // Pinned C++ `C4FileSelDlg::UpdateFileList` enumerates and appends
        // every matching file first, then appends the null "No Portrait" item
        // (`C4FileSelDlg.cpp:251-266`).
        let entries = ["King.png", "Mage.png"]
            .into_iter()
            .map(|name| {
                PortraitFileEntry::from_path(PathBuf::from("/portraits").join(name))
                    .expect("portrait entry")
            })
            .collect();
        let controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );

        assert_eq!(
            controller
                .items()
                .iter()
                .map(PortraitItem::label)
                .collect::<Vec<_>>(),
            ["King", "Mage", "No Portrait"]
        );
    }

    #[test]
    fn portrait_selector_uses_the_standard_close_icon() {
        // `C4GUI::Dialog::UpdateOwnPos` installs `Ico_Close`, and
        // `IconButton::DrawElement` draws that GUIIcons phase without a
        // wooden button background (pinned `C4GuiDialogs.cpp:399-421`,
        // `C4GuiButton.cpp:205-225`).
        let mut controller = test_controller();
        let surface = render_test_controller(&mut controller);

        let close = portrait_sel_layout(640, 480, 1).close;
        let center = surface
            .get_pixel(
                (close.x + close.w / 2) as u32,
                (close.y + close.h / 2) as u32,
            )
            .expect("close center");
        assert!(
            center.r > 2 * center.g && center.r > 2 * center.b,
            "Ico_Close center must be red-dominant, got {center:?}"
        );
    }

    #[test]
    fn portrait_selector_uses_standard_combo_chrome() {
        // `C4GUI::ComboBox::DrawElement` uses StandardBGColor plus a 3D
        // frame and GUIContext; it is not a wooden Button (pinned
        // `C4GuiComboBox.cpp:138-183`).
        let mut controller = test_controller();
        let surface = render_test_controller(&mut controller);

        let combo = portrait_sel_layout(640, 480, 1).location_combo;
        let interior = surface
            .get_pixel(
                (combo.x + combo.w - 40) as u32,
                (combo.y + combo.h / 2) as u32,
            )
            .expect("combo interior");
        assert_eq!(
            [interior.r, interior.g, interior.b],
            [0, 0, 0],
            "standard combo interior is translucent black, not wooden"
        );
    }

    #[test]
    fn portrait_selector_uses_standard_checkbox_chrome() {
        // `C4GUI::CheckBox::DrawElement` draws a square GUICheckbox phase
        // followed by plain TextFont; the full option cell is not a wooden
        // Button (pinned `C4GuiCheckBox.cpp:110-136`).
        let mut controller = test_controller();
        let surface = render_test_controller(&mut controller);

        let checkbox = portrait_sel_layout(640, 480, 1).set_picture;
        let center = surface
            .get_pixel(
                (checkbox.x + checkbox.h / 2) as u32,
                (checkbox.y + checkbox.h / 2) as u32,
            )
            .expect("checked checkbox center");
        assert!(
            center.r > 2 * center.g && center.r > 2 * center.b,
            "checked GUICheckbox center must be red-dominant, got {center:?}"
        );
    }

    #[test]
    fn portrait_selector_draws_the_permanent_standard_scrollbar() {
        // `C4GUI::ScrollWindow` always reserves and draws its 16px
        // `GUIScroll` bar; portrait lists do not auto-hide it (pinned
        // `C4GuiContainers.cpp:481-497`, `C4GuiListBox.h:119-123`).
        let mut controller = test_controller();
        let surface = render_test_controller(&mut controller);

        let bar = portrait_sel_layout(640, 480, 1).grid_scrollbar;
        let arrow = surface
            .get_pixel((bar.x + bar.w / 2) as u32, (bar.y + 8) as u32)
            .expect("scrollbar top arrow");
        assert!(
            u16::from(arrow.r) + u16::from(arrow.g) + u16::from(arrow.b) > 30,
            "GUIScroll top arrow must replace the black list background, got {arrow:?}"
        );
    }

    #[test]
    fn portrait_selector_wheel_preserves_pixel_delta_and_scrolled_hit_testing() {
        // Pinned C++ `ScrollWindow::MouseInput` forwards the negated native
        // wheel magnitude to pixel-based `ScrollBy`; ListBox adds that exact
        // `GetScrollY()` to pointer hit tests (`C4GuiContainers.cpp:618-625`,
        // `C4GuiListBox.cpp:142-162`).
        let entries = (0..15)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.resize(1152, 723);
        let layout = portrait_sel_layout(1152, 723, 1);
        let viewport = layout.grid_viewport;
        let top_left = GuiPoint::new((viewport.x + 1) as f32, (viewport.y + 1) as f32);

        controller.combo_open = true;
        let popup_overlap = GuiPoint::new(
            (layout.location_popup.x.max(viewport.x) + 1) as f32,
            (layout.location_popup.y.max(viewport.y) + 1) as f32,
        );
        controller.handle_pointer_move(popup_overlap);
        assert!(
            !controller.handle_wheel(-60),
            "the open ContextMenu consumes wheel input across its bounds"
        );
        controller.combo_open = false;
        controller.handle_pointer_move(GuiPoint::new(0.0, 0.0));
        assert!(
            !controller.handle_wheel(-60),
            "C++ routes wheel input only through the hovered ScrollWindow"
        );
        controller.handle_pointer_move(top_left);
        assert!(controller.handle_wheel(-60));
        assert_eq!(
            controller.hit_target(top_left),
            HitTarget::Tile(0),
            "60px of scrolling must leave the lower 58px of row zero hittable"
        );

        assert!(controller.handle_wheel(-10_000));
        let last_row = GuiPoint::new(
            (viewport.x + 1) as f32,
            (viewport.y + 3 * TILE_HEIGHT - (4 * TILE_HEIGHT - viewport.h) + 1) as f32,
        );
        assert_eq!(
            controller.hit_target(last_row),
            HitTarget::Tile(15),
            "clamping to the pixel bottom must reveal the clipped fourth row"
        );
    }

    #[test]
    fn portrait_selector_can_reveal_a_clipped_third_row() {
        // C++ derives ScrollWindow's range from pixel content height, not the
        // number of wholly visible rows (`C4GuiListBox.cpp:486-527`,
        // `C4GuiContainers.cpp:499-511,540-553`).
        let entries = (0..10)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.resize(1152, 723);
        let viewport = portrait_sel_layout(1152, 723, 1).grid_viewport;
        controller.handle_pointer_move(GuiPoint::new(
            (viewport.x + 1) as f32,
            (viewport.y + 1) as f32,
        ));

        assert!(controller.handle_wheel(-10_000));
        assert_eq!(controller.scroll_y, 3 * TILE_HEIGHT - viewport.h);
        let trailing_item = GuiPoint::new(
            (viewport.x + 1) as f32,
            (viewport.y + 2 * TILE_HEIGHT - controller.scroll_y + 1) as f32,
        );
        assert_eq!(controller.hit_target(trailing_item), HitTarget::Tile(10));
    }

    #[test]
    fn portrait_selector_wraps_labels_and_uses_the_tallest_item_for_the_next_row() {
        // Pinned C++ portrait items run MiniFont::BreakMessage at a 94px width,
        // add the resulting pixel height to the 100px preview, and ListBox
        // starts the next multi-column row below the tallest preceding item
        // (`C4FileSelDlg.cpp:426-445`, `C4GuiListBox.cpp:486-527`).
        let entries = [
            "WWWWWWWWWWWW.png",
            "Short1.png",
            "Short2.png",
            "SecondRow1.png",
        ]
        .into_iter()
        .map(|name| {
            PortraitFileEntry::from_path(PathBuf::from("/portraits").join(name))
                .expect("portrait entry")
        })
        .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        let fonts = crate::test_support::endeavour_font_set();
        let wrapped = crate::message_dialog::break_message(
            &fonts.mini,
            controller.items()[0].label(),
            PREVIEW_SIZE - 6,
        );
        let wrapped_height = fonts.mini.measure(&wrapped, false).1;
        assert!(wrapped_height > fonts.mini.line_height);

        let _surface = render_test_controller(&mut controller);
        let layout = controller.layout();
        let uniform_second_row = GuiPoint::new(
            (layout.grid_viewport.x + PREVIEW_SIZE + 1) as f32,
            (layout.grid_viewport.y + TILE_HEIGHT + 1) as f32,
        );
        assert_eq!(
            controller.hit_target(uniform_second_row),
            HitTarget::None,
            "the short item leaves a gap below it while the wrapped peer keeps row zero open"
        );

        let dynamic_second_row = GuiPoint::new(
            uniform_second_row.x,
            (layout.grid_viewport.y + PREVIEW_SIZE + wrapped_height + 1) as f32,
        );
        assert_eq!(
            controller.hit_target(dynamic_second_row),
            HitTarget::Tile(4)
        );
    }

    #[test]
    fn portrait_selector_wraps_markup_like_filename_bytes_as_literal_text() {
        // PortraitItem explicitly disables markup for both BreakMessage and
        // TextOut, so markup/image syntax and `|` are ordinary filename glyphs
        // (`C4FileSelDlg.cpp:426-444,470-502`).
        let entry = PortraitFileEntry::from_path(PathBuf::from(
            "/portraits/<c ff0000><i>{{LONGIMAGE}}|ABCDE.png",
        ))
        .expect("portrait entry");
        let controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            vec![entry],
            true,
            true,
        );
        let fonts = crate::test_support::endeavour_font_set();
        let max_width = PREVIEW_SIZE - 6;
        let label = controller.items()[0].label();
        assert!(fonts.mini.measure(label, false).0 > max_width);

        let content = portrait_grid_content_layout(controller.items(), 1, Some(&fonts.mini));
        let wrapped = &content.items[0].wrapped_label;
        assert!(
            wrapped.contains('\n'),
            "literal filename glyphs must contribute to automatic wrapping"
        );
        assert!(!wrapped.contains("</i><i>"));
        assert_eq!(wrapped.replace('\n', ""), label.replace(' ', ""));
    }

    #[test]
    fn portrait_selector_resize_reflows_cached_font_heights_before_clamping_scroll() {
        // C++ retains every MiniFont-wrapped ListItem height while reflowing
        // columns, then clamps pixel scroll and derives the scrollbar pin
        // from that exact client height (`C4FileSelDlg.cpp:426-445`,
        // `C4GuiListBox.cpp:405-459`, `C4GuiContainers.cpp:343-360,493-505`).
        let entries = (0..12)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/WWWWWWWWWWWW{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        let fonts = crate::test_support::endeavour_font_set();
        controller.resize(640, 480);
        let old_columns = controller.layout().columns;
        controller.update_grid_content_layout(old_columns, &fonts.mini);
        controller.handle_key_down(KeyCode::End);

        controller.resize(400, 360);

        let layout = controller.layout();
        let expected =
            portrait_grid_content_layout(controller.items(), layout.columns, Some(&fonts.mini));
        assert!(expected.items[0].rect.h > TILE_HEIGHT);
        assert_eq!(controller.grid_content_layout(layout.columns), expected);
        let exact_max = expected
            .content_height
            .saturating_sub(layout.grid_viewport.h)
            .max(0);
        assert_eq!(controller.scroll_y, exact_max);
        assert_eq!(
            controller.scrollbar_pin,
            controller.max_scrollbar_pin(),
            "the selected trailing row and scrollbar thumb both reach the exact bottom"
        );
    }

    #[test]
    fn portrait_selector_refresh_reconciles_pre_render_navigation_with_measured_rows() {
        // Native ListItems have their MiniFont-wrapped height before keyboard
        // navigation can run. Installing Rust's measured layout after a
        // refresh must therefore reconcile the selected item and scrollbar
        // position chosen using the temporary geometry (`C4FileSelDlg.cpp:426-445`,
        // `C4GuiListBox.cpp:405-459`, `C4GuiContainers.cpp:493-505`).
        let mut controller = test_controller();
        controller.resize(400, 360);
        let entries = (0..12)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/WWWWWWWWWWWWWWWWWWWWWWWW{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        assert!(controller.replace_location_entries(0, entries));
        controller.handle_key_down(KeyCode::End);

        let layout = controller.layout();
        let fonts = crate::test_support::endeavour_font_set();
        let measured =
            portrait_grid_content_layout(controller.items(), layout.columns, Some(&fonts.mini));
        let exact_max = measured
            .content_height
            .saturating_sub(layout.grid_viewport.h)
            .max(0);
        assert!(
            controller.scroll_y < exact_max,
            "pre-render navigation demonstrates the temporary fixed-height geometry"
        );

        controller.update_grid_content_layout(layout.columns, &fonts.mini);

        assert_eq!(controller.scroll_y, exact_max);
        assert_eq!(controller.scrollbar_pin, controller.max_scrollbar_pin());
    }

    #[test]
    fn portrait_selector_scrollbar_track_click_and_drag_map_to_pixel_scroll() {
        // Pinned C++ ScrollBar centers its fixed 16px thumb on a track click,
        // maps thumb pixels into the ScrollWindow's pixel range, and continues
        // that mapping while captured for dragging
        // (`C4GuiContainers.cpp:343-388,391-435`).
        let entries = (0..15)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.resize(1152, 723);
        let bar = portrait_sel_layout(1152, 723, 1).grid_scrollbar;
        let middle = GuiPoint::new((bar.x + bar.w / 2) as f32, (bar.y + bar.h / 2) as f32);

        controller.handle_pointer_down(middle);
        assert_eq!(controller.scroll_y, 76);

        let bottom = GuiPoint::new(middle.x, (bar.y + bar.h - 17) as f32);
        controller.handle_pointer_move(bottom);
        assert_eq!(controller.scroll_y, controller.max_scroll_y());
        controller.handle_pointer_up(bottom);

        controller.handle_pointer_move(middle);
        assert_eq!(
            controller.scroll_y,
            controller.max_scroll_y(),
            "releasing the captured thumb must end dragging"
        );
    }

    #[test]
    fn portrait_selector_scrollbar_release_continues_to_the_release_target() {
        // Screen stops the captured ScrollBar drag, clears pDragElement, and
        // then routes the same LeftUp normally. CheckBox toggles from any
        // release inside its leading square (`C4Gui.cpp:850-875`,
        // `C4GuiCheckBox.cpp:82-96`).
        let entries = (0..15)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.resize(1152, 723);
        let layout = controller.layout();
        let track = GuiPoint::new(
            (layout.grid_scrollbar.x + layout.grid_scrollbar.w / 2) as f32,
            (layout.grid_scrollbar.y + layout.grid_scrollbar.h / 2) as f32,
        );
        controller.handle_pointer_down(track);
        assert!(controller.scrollbar_dragging);

        let checkbox = GuiPoint::new(
            (layout.set_picture.x + 2) as f32,
            (layout.set_picture.y + 2) as f32,
        );
        controller.handle_pointer_up(checkbox);

        assert!(!controller.scrollbar_dragging);
        assert!(!controller.set_picture());
    }

    #[test]
    fn portrait_selector_held_scrollbar_arrows_repeat_by_thumb_pixel() {
        // Pinned C++ ScrollBar retains an arrow press, advances its thumb by
        // one pixel from every DrawElement call, and releases capture on
        // left-up (`C4GuiContainers.cpp:391-456`).
        let entries = (0..15)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.resize(1152, 723);
        let bar = portrait_sel_layout(1152, 723, 1).grid_scrollbar;
        let bottom_arrow = GuiPoint::new((bar.x + bar.w / 2) as f32, (bar.y + bar.h - 1) as f32);

        controller.focus = PortraitSelControl::SetPicture;
        controller.handle_pointer_down(bottom_arrow);
        assert_eq!(controller.focus(), PortraitSelControl::Grid);
        assert_eq!(controller.scroll_y, 0);
        assert!(controller.tick_scrollbar());
        assert_eq!(controller.scrollbar_pin, 1);
        assert!(
            controller.tick_scrollbar(),
            "holding the arrow must advance on the next presentation too"
        );
        assert_eq!(controller.scrollbar_pin, 2);

        let middle_track = GuiPoint::new(bottom_arrow.x, (bar.y + bar.h / 2) as f32);
        controller.handle_pointer_move(middle_track);
        assert!(
            controller.scrollbar_dragging,
            "held arrow movement into the track must transition to thumb dragging"
        );
        assert!(!controller.scrollbar_arrow_captured);
        controller.handle_pointer_up(middle_track);
        assert!(!controller.tick_scrollbar());
    }

    #[test]
    fn portrait_selector_content_height_includes_the_tallest_item_in_the_final_row() {
        // Pinned C++'s incremental AddElement path accidentally derives final
        // client height from the trailing short null item, clipping a taller
        // peer in that row (`C4GuiListBox.cpp:486-527`). Preserve the intended
        // per-row maximum; this presentation-only correction cannot desync.
        let entries = [
            "One.png",
            "Two.png",
            "Three.png",
            "Four.png",
            "Five.png",
            "Six.png",
            "WWWWWWWWWWWW.png",
        ]
        .into_iter()
        .map(|name| {
            PortraitFileEntry::from_path(PathBuf::from("/portraits").join(name))
                .expect("portrait entry")
        })
        .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        let fonts = crate::test_support::endeavour_font_set();
        let wrapped = crate::message_dialog::break_message(
            &fonts.mini,
            controller.items()[6].label(),
            PREVIEW_SIZE - 6,
        );
        let final_row_height = PREVIEW_SIZE + fonts.mini.measure(&wrapped, false).1;
        let _surface = render_test_controller(&mut controller);
        let viewport = controller.layout().grid_viewport;
        let viewport_height = viewport.h;
        controller.handle_pointer_move(GuiPoint::new(
            (viewport.x + 1) as f32,
            (viewport.y + 1) as f32,
        ));

        controller.handle_wheel(-10_000);

        assert_eq!(
            controller.scroll_y,
            2 * TILE_HEIGHT + final_row_height - viewport_height
        );
    }

    #[test]
    fn portrait_selector_location_popup_uses_context_menu_chrome() {
        // ComboBox::DoDropdown fills a C4GUI::ContextMenu with `Ico_Empty`
        // rows; the popup is a translucent brown context panel, not stacked
        // wooden Buttons (pinned `C4GuiComboBox.cpp:102-120`,
        // `C4GuiMenu.cpp:330-360`).
        let mut controller = PortraitSelController::new(
            vec![
                PortraitLocation::new("User Path", "/portraits"),
                PortraitLocation::new("Home", "/home"),
            ],
            0,
            Vec::new(),
            true,
            true,
        );
        controller.resize(640, 480);
        let layout = portrait_sel_layout(640, 480, 2);
        let combo_center = GuiPoint::new(
            (layout.location_combo.x + layout.location_combo.w / 2) as f32,
            (layout.location_combo.y + layout.location_combo.h / 2) as f32,
        );
        controller.handle_pointer_down(combo_center);
        controller.handle_pointer_up(combo_center);
        let surface = render_test_controller(&mut controller);

        let row = layout.location_options[0];
        let interior = surface
            .get_pixel((row.x + row.w - 40) as u32, (row.y + row.h / 2) as u32)
            .expect("context-menu row interior");
        assert!(
            interior.r < 100 && interior.g < 60,
            "context popup must not use bright wooden button chrome, got {interior:?}"
        );
    }

    #[test]
    fn portrait_selector_checkbox_label_does_not_toggle() {
        // C4GUI::CheckBox only toggles when the left-button release is inside
        // the leading h×h square (`C4GuiCheckBox.cpp:82-96`).
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            Vec::new(),
            true,
            true,
        );
        let checkbox = portrait_sel_layout(600, 500, 1).set_picture;
        let label = GuiPoint::new(
            (checkbox.x + checkbox.h + 8) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );

        controller.handle_pointer_down(label);
        controller.handle_pointer_up(label);

        assert!(controller.set_picture());
    }

    #[test]
    fn portrait_selector_combo_opens_on_pointer_down_without_stealing_focus() {
        // ComboBox opens its ContextMenu from left-down and controls opt out
        // of click-to-focus (`C4GuiComboBox.cpp:187-203`,
        // `C4GuiComboBox.h:65-66`).
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            Vec::new(),
            true,
            true,
        );
        let combo = portrait_sel_layout(600, 500, 1).location_combo;
        let point = GuiPoint::new(
            (combo.x + combo.w / 2) as f32,
            (combo.y + combo.h / 2) as f32,
        );

        controller.handle_pointer_down(point);

        assert!(controller.combo_open);
        assert_eq!(controller.focus(), PortraitSelControl::Grid);
        controller.handle_pointer_up(point);
        assert!(controller.combo_open);
    }

    #[test]
    fn portrait_selector_checkbox_and_button_clicks_preserve_keyboard_focus() {
        // CheckBox and Button both opt out of click-to-focus; only a clicked
        // ListBox changes keyboard focus in this dialog (`C4Gui.h:1084-1088,
        // 1260-1266`, `C4GuiContainers.cpp:695-712`).
        let mut controller = test_controller();
        let layout = portrait_sel_layout(600, 500, 1);
        let set_picture = checkbox_square(layout.set_picture);
        let checkbox_point = GuiPoint::new(
            (set_picture.x + set_picture.w / 2) as f32,
            (set_picture.y + set_picture.h / 2) as f32,
        );
        controller.handle_pointer_down(checkbox_point);
        assert_eq!(controller.focus(), PortraitSelControl::Grid);

        let ok_point = GuiPoint::new(
            (layout.ok.x + layout.ok.w / 2) as f32,
            (layout.ok.y + layout.ok.h / 2) as f32,
        );
        controller.handle_pointer_down(ok_point);
        assert_eq!(controller.focus(), PortraitSelControl::Grid);
    }

    #[test]
    fn portrait_selector_checkbox_toggles_from_release_inside_square() {
        // CheckBox handles left-up inside its inclusive square; it does not
        // require the matching left-down to have begun there
        // (`C4GuiCheckBox.cpp:82-96`, `C4Math.h:22`).
        let mut controller = test_controller();
        let checkbox = portrait_sel_layout(600, 500, 1).set_picture;
        let label_point = GuiPoint::new(
            (checkbox.x + checkbox.h + 8) as f32,
            (checkbox.y + checkbox.h / 2) as f32,
        );
        let square = checkbox_square(checkbox);
        let square_point = GuiPoint::new(
            (square.x + square.w / 2) as f32,
            (square.y + square.h / 2) as f32,
        );

        controller.handle_pointer_down(label_point);
        controller.handle_pointer_up(square_point);

        assert!(!controller.set_picture());

        let inclusive_bottom_right =
            GuiPoint::new((square.x + square.w) as f32, (square.y + square.h) as f32);
        controller.handle_pointer_up(inclusive_bottom_right);

        assert!(controller.set_picture());
    }

    #[test]
    fn portrait_selector_second_combo_click_closes_the_context_menu() {
        // Screen aborts an existing context before forwarding the next
        // left-down; ComboBox recognizes that just-closed menu and does not
        // reopen it (`C4Gui.cpp:774-776`, `C4GuiComboBox.cpp:187-200`).
        let mut controller = test_controller();
        let combo = portrait_sel_layout(600, 500, 1).location_combo;
        let point = GuiPoint::new(
            (combo.x + combo.w / 2) as f32,
            (combo.y + combo.h / 2) as f32,
        );
        controller.handle_pointer_down(point);
        controller.handle_pointer_up(point);
        assert!(controller.combo_open);

        controller.handle_pointer_down(point);

        assert!(!controller.combo_open);
    }

    #[test]
    fn portrait_selector_open_context_cancels_pending_button_key_release() {
        // Opening a context removes draw focus from every underlying control,
        // so a pending Space-up cannot activate its Button callback. C++
        // accidentally leaves Button::fDown latched; Rust intentionally
        // clears the presentation state to avoid a stuck button
        // (`C4GuiButton.cpp:112-128`, `C4Gui.h:1622-1635`).
        let mut controller = test_controller();
        controller.selected = Some(0);
        controller.focus = PortraitSelControl::Ok;
        assert!(controller.handle_key_down(KeyCode::Space).is_empty());
        let combo = controller.layout().location_combo;
        let combo_point = GuiPoint::new(
            (combo.x + combo.w / 2) as f32,
            (combo.y + combo.h / 2) as f32,
        );

        assert!(controller.handle_pointer_down(combo_point).is_empty());
        assert!(controller.combo_open);
        assert!(controller.handle_key_up(KeyCode::Space).is_empty());
        assert!(controller.combo_open);
        assert_eq!(controller.key_pressed, None);
    }

    #[test]
    fn portrait_selector_reselecting_current_location_requests_a_refresh() {
        // ComboBox always invokes the row callback. SetCurrentLocation has no
        // equality shortcut and refreshes the current path again
        // (`C4FileSelDlg.cpp:189-200,373-382`).
        let mut controller = test_controller();

        assert_eq!(
            controller.choose_location(0),
            vec![PortraitSelAction::ChangeLocation {
                index: 0,
                path: PathBuf::from("/portraits"),
            }]
        );
        assert_eq!(
            controller.items().len(),
            1,
            "refresh clears stale files while the caller rescans the location"
        );
        assert_eq!(controller.items()[0].choice(), &PortraitChoice::None);
    }

    #[test]
    fn portrait_selector_grid_arrows_match_cpp_none_and_boundary_selection() {
        // Multi-column ListBox chooses last for Up/Left from no selection,
        // first for Down/Right, and refuses an incomplete column stride at
        // the top/bottom boundary (`C4GuiListBox.cpp:218-292`).
        let entries = (0..7)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        let columns = controller.layout().columns;
        let last = controller.items().len() - 1;

        controller.move_grid(GridMove::Up);
        assert_eq!(controller.selected_index(), Some(last));
        controller.selected = None;
        controller.move_grid(GridMove::Left);
        assert_eq!(controller.selected_index(), Some(last));
        controller.selected = None;
        controller.move_grid(GridMove::Down);
        assert_eq!(controller.selected_index(), Some(0));
        controller.selected = None;
        controller.move_grid(GridMove::Right);
        assert_eq!(controller.selected_index(), Some(0));

        controller.selected = Some(columns - 1);
        controller.move_grid(GridMove::Up);
        assert_eq!(controller.selected_index(), Some(columns - 1));
        controller.selected = Some(last - 1);
        controller.move_grid(GridMove::Down);
        assert_eq!(controller.selected_index(), Some(last - 1));
    }

    #[test]
    fn portrait_selector_keyboard_focus_entering_empty_grid_selects_first_item() {
        // ListBox::OnGetFocus chooses its first row only for keyboard focus;
        // mouse blank-space focus intentionally leaves no selection
        // (`C4GuiListBox.cpp:199-208`).
        let mut controller = test_controller();
        controller.focus = PortraitSelControl::Location;
        controller.selected = None;

        controller.handle_key_down(KeyCode::Tab);

        assert_eq!(controller.focus(), PortraitSelControl::Grid);
        assert_eq!(controller.selected_index(), Some(0));
    }

    #[test]
    fn portrait_selector_tab_order_includes_only_the_selected_list_item() {
        // Recursive dialog traversal may enter only the selected ListBox
        // child. Portrait ListItem is a focusable Control, so it sits between
        // the list and Player image whenever a selection exists
        // (`C4GuiDialogs.cpp:618-648`, `C4GuiListBox.h:138`,
        // `C4FileSelDlg.h:61-67`).
        let mut controller = test_controller();
        controller.focus = PortraitSelControl::Grid;
        controller.selected = None;
        controller.handle_key_down_with_tab_direction(KeyCode::Tab, false);
        assert_eq!(controller.focus(), PortraitSelControl::SetPicture);

        controller.focus = PortraitSelControl::Grid;
        controller.selected = Some(0);
        controller.handle_key_down_with_tab_direction(KeyCode::Tab, false);
        assert_eq!(controller.focus(), PortraitSelControl::SelectedItem);
        controller.handle_key_down_with_tab_direction(KeyCode::Tab, false);
        assert_eq!(controller.focus(), PortraitSelControl::SetPicture);
        controller.handle_key_down_with_tab_direction(KeyCode::Tab, true);
        assert_eq!(controller.focus(), PortraitSelControl::SelectedItem);
        controller.handle_key_down_with_tab_direction(KeyCode::Tab, true);
        assert_eq!(controller.focus(), PortraitSelControl::Grid);
    }

    #[test]
    fn portrait_selector_refresh_deletes_the_focused_selected_child() {
        // UpdateFileList deletes every old ListItem. Deleting the active
        // child clears Dialog::pActiveCtrl; the next forward Tab starts at
        // the first registered control (`C4GuiDialogs.cpp:475-481,618-645`).
        let mut controller = test_controller();
        controller.selected = Some(0);
        controller.focus = PortraitSelControl::SelectedItem;
        controller.focused_item = Some(0);

        assert!(controller.replace_location_entries(0, Vec::new()));
        assert_eq!(controller.selected_index(), None);
        assert_eq!(controller.focus(), PortraitSelControl::NoControl);

        controller.handle_key_down_with_tab_direction(KeyCode::Tab, false);
        assert_eq!(controller.focus(), PortraitSelControl::Close);
    }

    #[test]
    fn portrait_selector_gamepad_low_is_control_specific() {
        // AnyLow is first offered to the focused control before Dialog's
        // default OK binding (`C4GuiComboBox.cpp:66-78`,
        // `C4GuiCheckBox.cpp:43-51`, `C4GuiButton.cpp:36-43`,
        // `C4GuiListBox.cpp:72-81`).
        let mut controller = test_controller();
        controller.focus = PortraitSelControl::Location;
        assert!(controller.handle_gamepad_low_down().is_empty());
        assert!(controller.combo_open);
        assert!(controller.handle_gamepad_low_up().is_empty());

        controller.combo_open = false;
        controller.focus = PortraitSelControl::SetPicture;
        let old_picture = controller.set_picture();
        assert!(controller.handle_gamepad_low_down().is_empty());
        assert_eq!(controller.set_picture(), !old_picture);
        assert!(controller.handle_gamepad_low_up().is_empty());

        controller.focus = PortraitSelControl::Grid;
        controller.selected = Some(0);
        assert!(matches!(
            controller.handle_gamepad_low_down().as_slice(),
            [PortraitSelAction::Accept(_)]
        ));

        controller.focus = PortraitSelControl::Cancel;
        assert!(controller.handle_gamepad_low_down().is_empty());
        assert_eq!(
            controller.handle_gamepad_low_up(),
            vec![PortraitSelAction::Cancel]
        );
    }

    #[test]
    fn portrait_selector_gamepad_directions_follow_control_priority() {
        // Grid directions belong to ListBox; Combo owns Down; otherwise
        // gamepad Left/Right traverse dialog controls. An open ContextMenu
        // owns the complete direction cluster (`C4GuiDialogs.cpp:341-362`,
        // `C4GuiComboBox.cpp:66-78`, `C4GuiListBox.cpp:43-71`).
        let mut controller = test_controller();
        controller.focus = PortraitSelControl::Location;

        assert!(controller
            .handle_gamepad_direction(KeyCode::Left)
            .is_empty());
        assert_eq!(controller.focus(), PortraitSelControl::Close);
        assert!(controller
            .handle_gamepad_direction(KeyCode::Right)
            .is_empty());
        assert_eq!(controller.focus(), PortraitSelControl::Location);
        assert!(controller
            .handle_gamepad_direction(KeyCode::Down)
            .is_empty());
        assert!(controller.combo_open);

        assert!(controller
            .handle_gamepad_direction(KeyCode::Left)
            .is_empty());
        assert_eq!(controller.focus(), PortraitSelControl::Location);
        assert_eq!(controller.combo_highlight, None);
        assert!(controller.handle_gamepad_direction(KeyCode::Up).is_empty());
        assert_eq!(
            controller.combo_highlight,
            Some(controller.locations().len() - 1)
        );

        assert!(controller.handle_gamepad_high_down().is_empty());
        assert!(!controller.combo_open);
        assert_eq!(
            controller.handle_gamepad_high_down(),
            vec![PortraitSelAction::Cancel]
        );
    }

    #[test]
    fn portrait_selector_close_participates_in_tab_order_and_activates_on_space_up() {
        // Dialog creates the focusable title CloseIconButton before the file
        // controls, and Button activates Space on key-up, not key-down
        // (`C4GuiDialogs.cpp:386-421`, `C4GuiButton.cpp:112-128`).
        let mut controller = test_controller();
        controller.focus = PortraitSelControl::Cancel;

        assert!(controller.handle_key_down(KeyCode::Tab).is_empty());
        assert_eq!(controller.focus(), PortraitSelControl::Close);
        assert!(controller.handle_key_down(KeyCode::Space).is_empty());
        assert_eq!(
            controller.handle_key_up(KeyCode::Space),
            vec![PortraitSelAction::Cancel]
        );
    }

    #[test]
    fn portrait_selector_page_keys_walk_pixel_visible_items() {
        // C++ PageDown/PageUp walk to the last/first fully visible item, and
        // scroll exactly one viewport only when the adjacent item is outside
        // (`C4GuiListBox.cpp:294-364`, `C4GuiContainers.cpp:534-581`).
        let entries = (0..15)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.resize(1152, 723);

        controller.move_grid(GridMove::PageDown);
        assert_eq!(controller.selected_index(), Some(9));
        assert_eq!(controller.scroll_y, 0);

        controller.move_grid(GridMove::PageDown);
        assert_eq!(controller.selected_index(), Some(15));
        assert_eq!(controller.scroll_y, controller.max_scroll_y());

        controller.move_grid(GridMove::PageUp);
        assert_eq!(controller.selected_index(), Some(10));
        controller.move_grid(GridMove::PageUp);
        assert_eq!(controller.selected_index(), Some(0));
        assert_eq!(controller.scroll_y, 0);
    }

    #[test]
    fn portrait_selector_blank_grid_click_focuses_and_clears_selection() {
        // ListBox handles left-down across its complete ScrollWindow: blank
        // space still focuses the list and clears the selected item
        // (`C4GuiListBox.cpp:142-168`, `C4GuiContainers.cpp:695-712`).
        let mut controller = test_controller();
        controller.selected = Some(0);
        controller.focus = PortraitSelControl::SetPicture;
        let viewport = controller.layout().grid_viewport;
        let blank = GuiPoint::new(
            (viewport.x + 2 * PREVIEW_SIZE + 1) as f32,
            (viewport.y + 1) as f32,
        );

        controller.handle_pointer_down(blank);

        assert_eq!(controller.focus(), PortraitSelControl::Grid);
        assert_eq!(controller.selected_index(), None);
    }

    #[test]
    fn portrait_selector_reclicking_selected_clipped_item_does_not_scroll() {
        // SelectionChanged scrolls an item into view only when the selected
        // pointer actually changes. Re-clicking the already-selected clipped
        // item leaves the ScrollWindow offset untouched
        // (`C4GuiListBox.cpp:154-167,571-585`).
        let entries = (0..12)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.resize(1152, 723);
        let layout = controller.layout();
        let grid = controller.grid_content_layout(layout.columns);
        let selected = layout.columns * 2;
        let item = &grid.items[selected];
        controller.selected = Some(selected);
        controller.scroll_y = 0;
        controller.sync_scrollbar_pin();
        let point = GuiPoint::new(
            (layout.grid_viewport.x + item.rect.x + 1) as f32,
            (layout.grid_viewport.y + item.rect.y + 1) as f32,
        );
        let old_scroll = controller.scroll_y;

        controller.handle_pointer_down(point);

        assert_eq!(controller.selected_index(), Some(selected));
        assert_eq!(controller.scroll_y, old_scroll);
    }

    #[test]
    fn portrait_selector_selected_child_focus_survives_list_pointer_input() {
        // ListBox refuses mouse focus while its active control is a nested
        // child, even though it still performs pointer selection
        // (`C4GuiContainers.cpp:695-712`, `C4GuiListBox.cpp:142-167`).
        let mut controller = test_controller();
        controller.selected = Some(0);
        controller.focus = PortraitSelControl::SelectedItem;
        controller.focused_item = Some(0);
        let viewport = controller.layout().grid_viewport;
        let first_item = GuiPoint::new((viewport.x + 1) as f32, (viewport.y + 1) as f32);

        controller.handle_pointer_down(first_item);

        assert_eq!(controller.focus(), PortraitSelControl::SelectedItem);
        assert_eq!(controller.selected_index(), Some(0));
    }

    #[test]
    fn portrait_selector_tabs_through_new_selection_after_focused_item_changes() {
        // Pointer selection does not replace the ListBox's active child.
        // Forward dialog traversal first visits a newly selected later item,
        // then continues to the next control (`C4GuiContainers.cpp:695-717`,
        // `C4GuiListBox.cpp:142-167`, `C4GuiDialogs.cpp:618-645`).
        let entries = ["King.png", "Mage.png"]
            .into_iter()
            .map(|name| {
                PortraitFileEntry::from_path(PathBuf::from(format!("/portraits/{name}")))
                    .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.selected = Some(0);
        controller.focus = PortraitSelControl::SelectedItem;
        controller.focused_item = Some(0);
        let layout = controller.layout();
        let grid = controller.grid_content_layout(layout.columns);
        let second = &grid.items[1];
        let point = GuiPoint::new(
            (layout.grid_viewport.x + second.rect.x + 1) as f32,
            (layout.grid_viewport.y + second.rect.y + 1) as f32,
        );

        controller.handle_pointer_down(point);

        assert_eq!(controller.selected_index(), Some(1));
        assert_eq!(controller.focus(), PortraitSelControl::SelectedItem);
        controller.handle_key_down(KeyCode::Tab);
        assert_eq!(controller.focus(), PortraitSelControl::SelectedItem);
        controller.handle_key_down(KeyCode::Tab);
        assert_eq!(controller.focus(), PortraitSelControl::SetPicture);
    }

    #[test]
    fn portrait_selector_list_frame_clicks_match_cpp_selection_bounds() {
        // Control focuses across the full ListBox. Its zero-origin
        // ScrollWindow hitbox includes the top-left frame before subtracting
        // the three-pixel margin, but excludes the bottom-right frame
        // (`C4GuiListBox.cpp:142-168`, `C4GuiContainers.cpp:483-490`).
        let mut controller = test_controller();
        controller.selected = Some(0);
        controller.focus = PortraitSelControl::SetPicture;
        let grid = controller.layout().grid;
        let top_left = GuiPoint::new(grid.x as f32, grid.y as f32);

        controller.handle_pointer_down(top_left);

        assert_eq!(controller.focus(), PortraitSelControl::Grid);
        assert_eq!(controller.selected_index(), None);

        controller.selected = Some(0);
        controller.focus = PortraitSelControl::SetPicture;
        let bottom_right =
            GuiPoint::new((grid.x + grid.w - 1) as f32, (grid.y + grid.h - 1) as f32);

        controller.handle_pointer_down(bottom_right);

        assert_eq!(controller.focus(), PortraitSelControl::Grid);
        assert_eq!(controller.selected_index(), Some(0));
    }

    #[test]
    fn portrait_selector_trailing_viewport_strip_is_outside_list_selection() {
        // The ListBox first checks the zero-origin ScrollWindow width, then
        // subtracts its three-pixel client margin. The shifted client viewport
        // must not extend that pointer hitbox (`C4GuiListBox.cpp:142-168`,
        // `C4GuiContainers.cpp:477-490`).
        let entries = (0..5)
            .map(|index| {
                PortraitFileEntry::from_path(PathBuf::from(format!(
                    "/portraits/Portrait{index}.png"
                )))
                .expect("portrait entry")
            })
            .collect();
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            entries,
            true,
            true,
        );
        controller.resize(800, 600);
        controller.selected = Some(0);
        controller.focus = PortraitSelControl::SetPicture;
        let layout = controller.layout();
        let point = GuiPoint::new(
            (layout.grid.x + layout.grid_viewport.w) as f32,
            (layout.grid_viewport.y + 1) as f32,
        );

        controller.handle_pointer_down(point);

        assert_eq!(controller.focus(), PortraitSelControl::Grid);
        assert_eq!(controller.selected_index(), Some(0));
    }

    #[test]
    fn portrait_selector_double_click_selects_and_accepts_file() {
        // ListBox selects on LeftDouble and invokes the portrait dialog's
        // single-selection callback, but only exact LeftDown takes focus
        // (`C4GuiContainers.cpp:701-717`, `C4GuiListBox.cpp:142-168`,
        // `C4FileSelDlg.cpp:230-239`).
        let path = PathBuf::from("/portraits/King.png");
        let entry = PortraitFileEntry::from_path(path.clone()).expect("portrait entry");
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            vec![entry],
            true,
            true,
        );
        controller.focus = PortraitSelControl::SetPicture;
        let viewport = controller.layout().grid_viewport;
        let first_item = GuiPoint::new((viewport.x + 1) as f32, (viewport.y + 1) as f32);

        assert_eq!(
            controller.handle_pointer_double_click(first_item),
            vec![PortraitSelAction::Accept(PortraitSelCommit {
                choice: PortraitChoice::File(path),
                set_picture: true,
                set_big_icon: true,
            })]
        );
        assert_eq!(controller.selected_index(), Some(0));
        assert_eq!(controller.focus(), PortraitSelControl::SetPicture);
    }

    #[test]
    fn portrait_selector_open_context_owns_double_clicks_inside_its_bounds() {
        // ContextMenu consumes every mouse event in its framed bounds, but
        // only exact LeftDown confirms a row. LeftDouble merely updates the
        // row highlight (`C4Gui.cpp:766-776`, `C4GuiMenu.cpp:200-227,430-439`).
        let mut controller = test_controller();
        let layout = controller.layout();
        controller.combo_open = true;
        controller.combo_highlight = None;
        let row = layout.location_options[0];
        let row_point = GuiPoint::new((row.x + row.w / 2) as f32, (row.y + row.h / 2) as f32);

        assert!(controller.handle_pointer_double_click(row_point).is_empty());
        assert!(controller.combo_open);
        assert_eq!(controller.combo_highlight, Some(0));
        assert_eq!(controller.selected_index(), None);
    }

    #[test]
    fn portrait_selector_outside_right_down_aborts_the_open_context() {
        // Screen aborts its ContextMenu on either LeftDown or RightDown
        // outside the menu bounds before routing the underlying event
        // (`C4Gui.cpp:766-776`).
        let mut controller = PortraitSelController::new(
            vec![
                PortraitLocation::new("User Path", "/portraits"),
                PortraitLocation::new("Home Folder", "/home/user"),
            ],
            0,
            Vec::new(),
            true,
            true,
        );
        let layout = controller.layout();
        controller.combo_open = true;
        controller.combo_highlight = Some(0);
        let outside = GuiPoint::new(layout.grid.x as f32, layout.grid.y as f32);

        assert!(controller.handle_pointer_right_down(outside).is_empty());
        assert!(!controller.combo_open);
        assert_eq!(controller.combo_highlight, None);

        controller.combo_open = true;
        controller.combo_highlight = Some(0);
        let second = layout.location_options[1];
        let inside = GuiPoint::new(
            (second.x + second.w / 2) as f32,
            (second.y + second.h / 2) as f32,
        );
        assert!(controller.handle_pointer_right_down(inside).is_empty());
        assert!(controller.combo_open);
        assert_eq!(controller.combo_highlight, Some(1));
    }

    #[test]
    fn portrait_selector_tooltip_targets_match_cpp_assignments() {
        // Dialog assigns tips to its WoodenLabel title and Close control;
        // PortraitSel adds tips to both checkboxes, while ComboBox supplies
        // IDS_MSG_SELECT descriptions to its popup entries
        // (`C4GuiDialogs.cpp:386-419`, `C4FileSelDlg.cpp:564-572`,
        // `C4GuiComboBox.cpp:37-44`).
        let mut controller = test_controller();
        let layout = controller.layout();
        let center = |rect: IntRect| {
            GuiPoint::new((rect.x + rect.w / 2) as f32, (rect.y + rect.h / 2) as f32)
        };
        assert_eq!(
            controller.tooltip_at(center(layout.close)),
            Some(StartupTooltip::resource("IDS_MNU_CLOSE"))
        );
        assert_eq!(
            controller.tooltip_at(GuiPoint::new(
                (layout.caption.x + 2) as f32,
                (layout.caption.y + 2) as f32,
            )),
            Some(StartupTooltip::text(controller.caption()))
        );
        assert_eq!(
            controller.tooltip_at(center(layout.set_picture)),
            Some(StartupTooltip::resource(
                "IDS_DESC_CHANGESTHEIMAGEYOUSEEINTH"
            ))
        );
        assert_eq!(
            controller.tooltip_at(center(layout.set_big_icon)),
            Some(StartupTooltip::resource(
                "IDS_DESC_CHANGESTHEIMAGEYOUSEEINTH2"
            ))
        );
        assert_eq!(controller.tooltip_at(center(layout.location_combo)), None);

        controller.combo_open = true;
        assert_eq!(
            controller.tooltip_at(center(layout.location_options[0])),
            Some(StartupTooltip::formatted_resource(
                "IDS_MSG_SELECT",
                [controller.locations()[0].label.clone()],
            ))
        );
        let popup_margin = GuiPoint::new(
            (layout.location_popup.x + 1) as f32,
            (layout.location_popup.y + 1) as f32,
        );
        assert_eq!(controller.tooltip_at(popup_margin), None);
    }

    #[test]
    fn portrait_selector_context_margin_consumes_pointer_input() {
        // A ContextMenu owns its complete framed bounds. Clicking the margin
        // keeps the menu open and cannot fall through to the ListBox beneath
        // it (`C4GuiMenu.cpp:200-227`, `C4Gui.cpp:766-776`).
        let mut controller = PortraitSelController::new(
            vec![
                PortraitLocation::new("User Path", "/portraits"),
                PortraitLocation::new("Home", "/home"),
            ],
            0,
            Vec::new(),
            true,
            true,
        );
        let layout = portrait_sel_layout(600, 500, 2);
        let combo_point = GuiPoint::new(
            (layout.location_combo.x + layout.location_combo.w / 2) as f32,
            (layout.location_combo.y + layout.location_combo.h / 2) as f32,
        );
        controller.handle_pointer_down(combo_point);
        assert_eq!(controller.combo_highlight, None);
        controller.handle_pointer_up(combo_point);
        let margin_point = GuiPoint::new(
            (layout.location_popup.x + 2) as f32,
            (layout.location_popup.y + 2) as f32,
        );

        controller.handle_pointer_down(margin_point);
        controller.handle_pointer_up(margin_point);

        assert!(controller.combo_open);
        assert_eq!(controller.selected_index(), None);
    }

    #[test]
    fn portrait_selector_context_starts_and_leaves_with_no_highlight() {
        // ContextMenu starts with no selected entry and clears a mouse-picked
        // entry when the pointer leaves it (`C4GuiMenu.cpp:86,200-237`).
        let mut controller = PortraitSelController::new(
            vec![
                PortraitLocation::new("User Path", "/portraits"),
                PortraitLocation::new("Home", "/home"),
            ],
            0,
            Vec::new(),
            true,
            true,
        );
        controller.resize(640, 480);
        let layout = portrait_sel_layout(640, 480, 2);
        let combo_point = GuiPoint::new(
            (layout.location_combo.x + layout.location_combo.w / 2) as f32,
            (layout.location_combo.y + layout.location_combo.h / 2) as f32,
        );
        controller.handle_pointer_down(combo_point);

        let row = layout.location_options[0];
        let margin_point = GuiPoint::new(
            (layout.location_popup.x + 4) as f32,
            (row.y + row.h / 2) as f32,
        );
        let row_point = GuiPoint::new((row.x + 2) as f32, margin_point.y);
        let surface = render_test_controller(&mut controller);
        assert_eq!(
            surface.get_pixel(margin_point.x as u32, margin_point.y as u32),
            surface.get_pixel(row_point.x as u32, row_point.y as u32),
            "a newly opened context must not pre-highlight the current location"
        );

        controller.handle_pointer_move(row_point);
        assert_eq!(controller.combo_highlight, Some(0));
        controller.handle_pointer_move(margin_point);
        assert_eq!(
            controller.combo_highlight, None,
            "leaving a context row must clear its mouse highlight"
        );
    }

    #[test]
    fn portrait_selector_combo_keyboard_matches_context_ownership() {
        // Focused ComboBox opens on Down. While its root ContextMenu is open,
        // unbound dialog keys cannot fall through, and Home/End are not menu
        // bindings (`C4GuiComboBox.cpp:66-86`, `C4GuiMenu.cpp:92-147`,
        // `C4GuiDialogs.cpp:731-740`).
        let mut controller = PortraitSelController::new(
            vec![
                PortraitLocation::new("User Path", "/portraits"),
                PortraitLocation::new("Home", "/home"),
            ],
            0,
            Vec::new(),
            true,
            true,
        );
        controller.focus = PortraitSelControl::Location;

        assert!(controller.handle_key_down(KeyCode::Down).is_empty());
        assert!(controller.combo_open);
        assert_eq!(controller.combo_highlight, None);
        for key in [
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Tab,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Space,
        ] {
            assert!(controller.handle_key_down(key).is_empty());
            assert!(controller.combo_open, "{key:?} must not close the context");
            assert_eq!(
                controller.combo_highlight, None,
                "{key:?} is not a row-selection binding"
            );
        }
    }

    #[test]
    fn portrait_selector_unhandled_keys_preserve_focus_without_a_default_control() {
        // Dialog::KeyFocusDefault asks GetDefaultControl, but C4FileSelDlg
        // inherits the base null result. An unhandled arrow therefore leaves
        // both focus and selection unchanged (`C4GuiDialogs.cpp:385-388,
        // 585-592`, `C4GuiDialogs.h:83-84`).
        let mut controller = PortraitSelController::new(
            vec![
                PortraitLocation::new("User Path", "/portraits"),
                PortraitLocation::new("Home", "/home"),
            ],
            0,
            Vec::new(),
            true,
            true,
        );

        assert!(controller.handle_key_down(KeyCode::Space).is_empty());
        assert_eq!(controller.selected_index(), None);

        controller.focus = PortraitSelControl::Location;
        assert!(controller.handle_key_down(KeyCode::Right).is_empty());
        assert_eq!(controller.current_location_index(), 0);
        assert_eq!(controller.focus(), PortraitSelControl::Location);
        assert_eq!(controller.selected_index(), None);
    }

    #[test]
    fn portrait_selector_missing_selection_requests_a_screen_error_dialog() {
        // FileSel leaves itself open and asks Screen::ShowErrorMessage rather
        // than painting validation text inside the selector
        // (`C4FileSelDlg.cpp:209-219`).
        let mut controller = test_controller();

        assert_eq!(
            controller.handle_key_down(KeyCode::Enter),
            vec![PortraitSelAction::SelectionRequired]
        );
        assert_eq!(controller.validation_error(), None);
    }

    #[test]
    fn portrait_selector_context_rows_keep_cpp_spacing() {
        // ContextMenu inserts C4GUI_DefaultListSpacing (one pixel) before
        // every row after the first (`C4GuiMenu.cpp:327-360`,
        // `C4Gui.h:129`).
        let layout = portrait_sel_layout(640, 480, 2);
        assert_eq!(
            layout.location_options[1].y,
            layout.location_options[0].y + layout.location_options[0].h + 1
        );
        assert_eq!(layout.location_popup.h, 2 * TEXT_LINE_HEIGHT + 1 + 10);
    }

    #[test]
    fn portrait_selector_malformed_icon_override_does_not_panic() {
        // Scenario GUI sheet overrides are input data. A short GUIIcons sheet
        // must not turn close-button drawing into division by zero
        // (`C4Gui.cpp:1085-1112`, `C4GuiButton.cpp:205-225`).
        let mut surface = Surface::new(32, 32, clonk_graphics::PixelFormat::Rgba8888);
        let malformed = ImageData::new(0, 0, Vec::new());
        draw_icon_phase(
            &mut surface,
            IntRect::new(0, 0, 16, 16),
            &malformed,
            34,
            None,
        );
    }

    #[test]
    fn portrait_sel_dialog_lists_images_matching_c4cfn_image_files_in_selected_location() {
        let temp = tempfile::tempdir().expect("portrait directory");
        // WildcardListMatch allows `*` to consume zero bytes, so a raw child
        // named exactly like the extension still matches (`StdFile.cpp:322-367`).
        for name in [
            "one.png",
            "two.BMP",
            "three.Jpeg",
            "four.JPG",
            ".png",
            ".JPG",
        ] {
            fs::write(temp.path().join(name), b"not decoded during enumeration")
                .expect("write matching image name");
        }
        for name in ["five.gif", "notes.txt", "six.png.bak"] {
            fs::write(temp.path().join(name), b"ignored").expect("write rejected name");
        }
        fs::create_dir(temp.path().join("folder.png")).expect("matching directory entry");

        let entries = portrait_files_in_location(temp.path()).expect("scan portrait directory");
        let names = entries
            .iter()
            .map(|entry| entry.filename.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            names,
            BTreeSet::from([
                ".JPG",
                ".png",
                "folder.png",
                "four.JPG",
                "one.png",
                "three.Jpeg",
                "two.BMP",
            ])
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.filename == ".png")
                .map(|entry| entry.label.as_str()),
            Some(""),
            "RemoveExtension treats the leading dot as the final extension \
             (`StdFile.cpp:99-106,279-290`)"
        );

        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", temp.path())],
            0,
            entries,
            true,
            false,
        );
        assert_eq!(controller.items().len(), 8);
        assert_eq!(controller.items()[7].choice(), &PortraitChoice::None);
        assert!(controller.items()[..7]
            .iter()
            .all(|item| item.thumbnail() == &PortraitThumbnail::Pending));

        let first = controller
            .advance_idle()
            .expect("first idle releases one image");
        assert!(controller.advance_idle().is_none());
        controller.complete_thumbnail(&first, Ok(ImageData::new(1, 1, vec![1, 2, 3, 255])));
        assert_eq!(
            controller
                .items()
                .iter()
                .filter(|item| matches!(item.thumbnail(), PortraitThumbnail::Ready(_)))
                .count(),
            1
        );
        assert_eq!(
            controller
                .items()
                .iter()
                .filter(|item| item.thumbnail() == &PortraitThumbnail::Pending)
                .count(),
            6,
            "only one matching entry is decoded per loader quantum"
        );

        for _ in 0..8 {
            assert!(controller.advance_idle().is_none());
        }
        let second = controller
            .advance_idle()
            .expect("the next loader quantum releases a second image");
        assert_ne!(second.index, first.index);
    }

    #[test]
    fn portrait_directory_iteration_error_keeps_the_yielded_prefix() {
        // DirectoryIterator turns the first read error into end-of-iteration;
        // UpdateFileList retains every entry already returned before appending
        // its null tile (`StdFile.cpp:824-836`, `C4FileSelDlg.cpp:251-274`).
        let paths = [
            Ok(PathBuf::from("/portraits/First.png")),
            Err(io::Error::other("read failed")),
            Ok(PathBuf::from("/portraits/Unreachable.png")),
        ];

        let entries = portrait_files_from_paths(paths);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, "First.png");
    }

    #[test]
    fn portrait_selector_directory_scan_failure_rebuilds_a_silent_empty_list() {
        // DirectoryIterator failure simply leaves UpdateFileList with no file
        // entries before it appends the null tile; FileSel shows no inline
        // error (`C4FileSelDlg.cpp:251-274`, `StdFile.cpp:712-847`).
        let entry = PortraitFileEntry::from_path(PathBuf::from("/portraits/King.png"))
            .expect("portrait entry");
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/portraits")],
            0,
            vec![entry],
            true,
            true,
        );

        controller.fail_location_entries(0, "permission denied");

        assert_eq!(controller.items().len(), 1);
        assert_eq!(controller.items()[0].choice(), &PortraitChoice::None);
        assert_eq!(controller.validation_error(), None);
    }

    #[test]
    fn portrait_sel_dialog_none_item_clears_portrait() {
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", "/tmp")],
            0,
            Vec::new(),
            true,
            false,
        );
        assert!(controller.handle_key_down(KeyCode::Down).is_empty());
        assert_eq!(controller.selected_index(), Some(0));
        assert_eq!(
            controller.handle_key_down(KeyCode::Enter),
            vec![PortraitSelAction::Accept(PortraitSelCommit {
                choice: PortraitChoice::None,
                set_picture: true,
                set_big_icon: false,
            })]
        );
    }

    #[test]
    fn portrait_selector_emits_independent_checkbox_defaults_for_a_file() {
        let temp = tempfile::tempdir().expect("portrait directory");
        let path = temp.path().join("icon-only.png");
        fs::write(&path, b"enumeration does not decode this file").expect("write portrait name");
        let entry = PortraitFileEntry::from_path(path.clone()).expect("portrait entry");
        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", temp.path())],
            0,
            vec![entry],
            false,
            true,
        );
        assert!(!controller.set_picture());
        assert!(controller.set_big_icon());
        controller.handle_key_down(KeyCode::Down);
        assert_eq!(
            controller.handle_key_down(KeyCode::Enter),
            vec![PortraitSelAction::Accept(PortraitSelCommit {
                choice: PortraitChoice::File(path),
                set_picture: false,
                set_big_icon: true,
            })]
        );

        controller.handle_key_down(KeyCode::Tab);
        assert_eq!(controller.focus(), PortraitSelControl::SelectedItem);
        controller.handle_key_down(KeyCode::Tab);
        assert_eq!(controller.focus(), PortraitSelControl::SetPicture);
        controller.handle_key_down(KeyCode::Space);
        controller.handle_key_down(KeyCode::Tab);
        assert_eq!(controller.focus(), PortraitSelControl::SetBigIcon);
        controller.handle_key_down(KeyCode::Space);
        assert!(controller.set_picture());
        assert!(!controller.set_big_icon());
    }

    #[test]
    fn location_change_invalidates_an_old_thumbnail_completion() {
        let temp = tempfile::tempdir().expect("portrait directory");
        let first_path = temp.path().join("first.png");
        fs::write(&first_path, b"pending").expect("write first path");
        let entry = PortraitFileEntry::from_path(first_path).expect("file entry");
        let mut controller = PortraitSelController::new(
            vec![
                PortraitLocation::new("First", temp.path()),
                PortraitLocation::new("Second", temp.path()),
            ],
            0,
            vec![entry],
            true,
            true,
        );
        let stale = controller.advance_idle().expect("old load request");
        assert_eq!(
            controller.choose_location(1),
            vec![PortraitSelAction::ChangeLocation {
                index: 1,
                path: temp.path().to_path_buf(),
            }]
        );
        assert!(
            !controller.complete_thumbnail(&stale, Ok(ImageData::new(1, 1, vec![1, 2, 3, 255])),)
        );
    }
}
