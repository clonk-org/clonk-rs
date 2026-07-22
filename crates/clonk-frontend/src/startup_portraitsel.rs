//! Classic portrait-file selector used by the startup player properties dialog.
//!
//! The file scan is intentionally cheap and synchronous: it records matching
//! direct children without decoding them. The application consumes
//! [`PortraitThumbnailRequest`] values one at a time and returns the decoded
//! previews, matching `C4PortraitSelDlg::ImageLoader`'s incremental worklist.

use crate::classic_gui::{
    draw_3d_frame, draw_engine_box, ClassicButtonState, ClassicGuiSkin, IntRect,
};
use crate::{ClonkFontSet, GuiPoint, ImageData, KeyCode};
use clonk_graphics::clonk_font::TextAlign;
use clonk_graphics::{Color, GammaRamp, Rect as SurfaceRect, Surface};
use clonk_gui::Rect as GuiRect;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MIN_WIDTH: i32 = 300;
const MAX_WIDTH: i32 = 600;
const MIN_HEIGHT: i32 = 220;
const MAX_HEIGHT: i32 = 500;
const CAPTION_HEIGHT: i32 = 28;
const PREVIEW_SIZE: i32 = 100;
const TILE_HEIGHT: i32 = 122;
const CONTROL_HEIGHT: i32 = 26;
const BUTTON_WIDTH: i32 = 120;
const BUTTON_HEIGHT: i32 = 32;
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
        let label = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| filename.clone());
        Some(Self {
            full_path: path,
            filename,
            label,
        })
    }
}

/// Direct-child `C4CFN_ImageFiles` scan (`*.png|*.bmp|*.jpeg|*.jpg`).
pub fn portrait_files_in_location(root: &Path) -> io::Result<Vec<PortraitFileEntry>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || !matches_c4cfn_image_files(&path) {
            continue;
        }
        if let Some(entry) = PortraitFileEntry::from_path(path) {
            files.push(entry);
        }
    }
    Ok(files)
}

fn matches_c4cfn_image_files(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "bmp" | "jpeg" | "jpg"
            )
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
    Location,
    Grid,
    SetPicture,
    SetBigIcon,
    Ok,
    Cancel,
}

impl PortraitSelControl {
    const ORDER: [Self; 6] = [
        Self::Location,
        Self::Grid,
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
    scroll_row: usize,
    set_picture: bool,
    set_big_icon: bool,
    combo_open: bool,
    combo_highlight: usize,
    pointer: Option<GuiPoint>,
    pressed: HitTarget,
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
            scroll_row: 0,
            set_picture,
            set_big_icon,
            combo_open: false,
            combo_highlight: current_location,
            pointer: None,
            pressed: HitTarget::None,
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
        self.ensure_selected_visible();
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

    pub fn validation_error(&self) -> Option<&str> {
        self.validation_error.as_deref()
    }

    pub fn set_validation_error(&mut self, error: impl Into<String>) {
        self.validation_error = Some(error.into());
    }

    pub fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.pressed = HitTarget::None;
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

    pub fn fail_location_entries(&mut self, location_index: usize, error: impl Into<String>) {
        if location_index == self.current_location {
            self.install_entries(Vec::new());
            self.validation_error = Some(error.into());
        }
    }

    fn install_entries(&mut self, entries: Vec<PortraitFileEntry>) {
        self.generation = self.generation.wrapping_add(1);
        self.items.clear();
        // The issue intentionally fixes C++'s ineffective trailing null item:
        // a distinct leading tile can be selected and committed as a clear.
        self.items.push(PortraitItem::none());
        self.items
            .extend(entries.into_iter().map(PortraitItem::file));
        self.selected = None;
        self.scroll_row = 0;
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
        if let HitTarget::LocationOption(index) = self.hit_target(point) {
            self.combo_highlight = index;
        }
        Vec::new()
    }

    pub fn handle_pointer_down(&mut self, point: GuiPoint) -> Vec<PortraitSelAction> {
        self.pointer = Some(point);
        let target = self.hit_target(point);
        self.pressed = target;
        match target {
            HitTarget::Tile(index) => {
                self.combo_open = false;
                self.focus = PortraitSelControl::Grid;
                self.selected = Some(index);
                self.validation_error = None;
                self.ensure_selected_visible();
            }
            HitTarget::Location | HitTarget::LocationOption(_) => {
                self.focus = PortraitSelControl::Location;
            }
            HitTarget::SetPicture => self.focus = PortraitSelControl::SetPicture,
            HitTarget::SetBigIcon => self.focus = PortraitSelControl::SetBigIcon,
            HitTarget::Ok => self.focus = PortraitSelControl::Ok,
            HitTarget::Cancel | HitTarget::Close => self.focus = PortraitSelControl::Cancel,
            HitTarget::None => self.combo_open = false,
        }
        Vec::new()
    }

    pub fn handle_pointer_up(&mut self, point: GuiPoint) -> Vec<PortraitSelAction> {
        self.pointer = Some(point);
        let released = self.hit_target(point);
        let pressed = std::mem::replace(&mut self.pressed, HitTarget::None);
        if pressed != released {
            return Vec::new();
        }
        match released {
            HitTarget::Close | HitTarget::Cancel => vec![PortraitSelAction::Cancel],
            HitTarget::Location => {
                self.combo_open = !self.combo_open;
                self.combo_highlight = self.current_location;
                Vec::new()
            }
            HitTarget::LocationOption(index) => self.choose_location(index),
            HitTarget::SetPicture => {
                self.set_picture = !self.set_picture;
                Vec::new()
            }
            HitTarget::SetBigIcon => {
                self.set_big_icon = !self.set_big_icon;
                Vec::new()
            }
            HitTarget::Ok => self.try_accept(),
            HitTarget::Tile(_) | HitTarget::None => Vec::new(),
        }
    }

    pub fn handle_key_down(&mut self, key: KeyCode) -> Vec<PortraitSelAction> {
        if self.combo_open {
            match key {
                KeyCode::Escape => {
                    self.combo_open = false;
                    return Vec::new();
                }
                KeyCode::Up => {
                    if !self.locations.is_empty() {
                        self.combo_highlight = (self.combo_highlight + self.locations.len() - 1)
                            % self.locations.len();
                    }
                    return Vec::new();
                }
                KeyCode::Down => {
                    if !self.locations.is_empty() {
                        self.combo_highlight = (self.combo_highlight + 1) % self.locations.len();
                    }
                    return Vec::new();
                }
                KeyCode::Home => {
                    self.combo_highlight = 0;
                    return Vec::new();
                }
                KeyCode::End => {
                    self.combo_highlight = self.locations.len().saturating_sub(1);
                    return Vec::new();
                }
                KeyCode::Enter | KeyCode::Space => {
                    return self.choose_location(self.combo_highlight);
                }
                KeyCode::Tab
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::PageUp
                | KeyCode::PageDown => self.combo_open = false,
            }
        }

        match key {
            KeyCode::Escape => vec![PortraitSelAction::Cancel],
            KeyCode::Enter => self.try_accept(),
            KeyCode::Tab => {
                self.move_focus(false);
                Vec::new()
            }
            KeyCode::Up if self.focus == PortraitSelControl::Grid => {
                self.move_grid(GridMove::Up);
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
            KeyCode::Left if self.focus == PortraitSelControl::Location => {
                let index = if self.locations.is_empty() {
                    0
                } else {
                    (self.current_location + self.locations.len() - 1) % self.locations.len()
                };
                self.choose_location(index)
            }
            KeyCode::Right if self.focus == PortraitSelControl::Location => {
                let index = if self.locations.is_empty() {
                    0
                } else {
                    (self.current_location + 1) % self.locations.len()
                };
                self.choose_location(index)
            }
            KeyCode::Space => self.activate_focus(),
            KeyCode::Up => {
                self.move_focus(true);
                Vec::new()
            }
            KeyCode::Down => {
                self.move_focus(false);
                Vec::new()
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown => Vec::new(),
        }
    }

    pub fn handle_key_up(&mut self, _key: KeyCode) -> Vec<PortraitSelAction> {
        Vec::new()
    }

    pub fn handle_wheel(&mut self, native_delta: i32) -> bool {
        if native_delta == 0 {
            return false;
        }
        let old = self.scroll_row;
        if native_delta > 0 {
            self.scroll_row = self.scroll_row.saturating_sub(1);
        } else {
            self.scroll_row = (self.scroll_row + 1).min(self.max_scroll_row());
        }
        old != self.scroll_row
    }

    fn activate_focus(&mut self) -> Vec<PortraitSelAction> {
        match self.focus {
            PortraitSelControl::Location => {
                self.combo_open = !self.combo_open;
                self.combo_highlight = self.current_location;
                Vec::new()
            }
            PortraitSelControl::Grid => {
                if self.selected.is_none() && !self.items.is_empty() {
                    self.selected = Some(0);
                }
                Vec::new()
            }
            PortraitSelControl::SetPicture => {
                self.set_picture = !self.set_picture;
                Vec::new()
            }
            PortraitSelControl::SetBigIcon => {
                self.set_big_icon = !self.set_big_icon;
                Vec::new()
            }
            PortraitSelControl::Ok => self.try_accept(),
            PortraitSelControl::Cancel => vec![PortraitSelAction::Cancel],
        }
    }

    fn try_accept(&mut self) -> Vec<PortraitSelAction> {
        let Some(choice) = self.selected_item().map(|item| item.choice.clone()) else {
            self.validation_error = Some("Please select a file first".to_string());
            return Vec::new();
        };
        vec![PortraitSelAction::Accept(PortraitSelCommit {
            choice,
            set_picture: self.set_picture,
            set_big_icon: self.set_big_icon,
        })]
    }

    fn choose_location(&mut self, index: usize) -> Vec<PortraitSelAction> {
        self.combo_open = false;
        let Some(location) = self.locations.get(index).cloned() else {
            return Vec::new();
        };
        if index == self.current_location {
            return Vec::new();
        }
        self.current_location = index;
        self.combo_highlight = index;
        self.install_entries(Vec::new());
        vec![PortraitSelAction::ChangeLocation {
            index,
            path: location.path,
        }]
    }

    fn move_focus(&mut self, backwards: bool) {
        let position = PortraitSelControl::ORDER
            .iter()
            .position(|control| *control == self.focus)
            .unwrap_or(0);
        let count = PortraitSelControl::ORDER.len();
        self.focus = PortraitSelControl::ORDER[if backwards {
            (position + count - 1) % count
        } else {
            (position + 1) % count
        }];
    }

    fn move_grid(&mut self, movement: GridMove) {
        if self.items.is_empty() {
            self.selected = None;
            return;
        }
        let layout = self.layout();
        let columns = layout.columns.max(1);
        let visible = columns * layout.visible_rows.max(1);
        let last = self.items.len() - 1;
        let Some(current) = self.selected else {
            self.selected = Some(if matches!(movement, GridMove::End) {
                last
            } else {
                0
            });
            self.validation_error = None;
            self.ensure_selected_visible();
            return;
        };
        let next = match movement {
            GridMove::Left => current.saturating_sub(1),
            GridMove::Right => (current + 1).min(last),
            GridMove::Up => current.saturating_sub(columns),
            GridMove::Down => (current + columns).min(last),
            GridMove::Home => 0,
            GridMove::End => last,
            GridMove::PageUp => current.saturating_sub(visible),
            GridMove::PageDown => (current + visible).min(last),
        };
        self.selected = Some(next);
        self.validation_error = None;
        self.ensure_selected_visible();
    }

    fn ensure_selected_visible(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let layout = self.layout();
        let row = selected / layout.columns.max(1);
        if row < self.scroll_row {
            self.scroll_row = row;
        } else if row >= self.scroll_row + layout.visible_rows.max(1) {
            self.scroll_row = row + 1 - layout.visible_rows.max(1);
        }
        self.scroll_row = self.scroll_row.min(self.max_scroll_row());
    }

    fn max_scroll_row(&self) -> usize {
        let layout = self.layout();
        let columns = layout.columns.max(1);
        let rows = self.items.len().div_ceil(columns);
        rows.saturating_sub(layout.visible_rows.max(1))
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
        }
        if contains(layout.close, point) {
            return HitTarget::Close;
        }
        if contains(layout.location_combo, point) {
            return HitTarget::Location;
        }
        if contains(layout.set_picture, point) {
            return HitTarget::SetPicture;
        }
        if contains(layout.set_big_icon, point) {
            return HitTarget::SetBigIcon;
        }
        if contains(layout.ok, point) {
            return HitTarget::Ok;
        }
        if contains(layout.cancel, point) {
            return HitTarget::Cancel;
        }
        if contains(layout.grid, point) {
            let relative_x = point.x.floor() as i32 - layout.grid.x;
            let relative_y = point.y.floor() as i32 - layout.grid.y;
            let column = (relative_x / PREVIEW_SIZE).max(0) as usize;
            let row = (relative_y / TILE_HEIGHT).max(0) as usize + self.scroll_row;
            let index = row * layout.columns.max(1) + column;
            if column < layout.columns && index < self.items.len() {
                return HitTarget::Tile(index);
            }
        }
        HitTarget::None
    }

    pub fn render(
        &self,
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
        resources.skin.draw_button(
            surface,
            layout.close,
            "X",
            resources.fonts,
            ClassicButtonState {
                pressed: self.pressed == HitTarget::Close,
                highlighted: self
                    .pointer
                    .is_some_and(|point| contains(layout.close, point)),
            },
            gamma,
        );

        resources.fonts.text.draw_with_gamma(
            surface,
            layout.location_label.x,
            layout.location_label.y + 3,
            "Location:",
            [255, 255, 255, 255],
            TextAlign::Left,
            false,
            gamma,
        );
        let location_text = self
            .current_location()
            .map(|location| format!("{}  v", location.label))
            .unwrap_or_else(|| "No locations".to_string());
        resources.skin.draw_button(
            surface,
            layout.location_combo,
            &location_text,
            resources.fonts,
            ClassicButtonState {
                pressed: self.pressed == HitTarget::Location,
                highlighted: self.focus == PortraitSelControl::Location,
            },
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
        draw_3d_frame(surface, layout.grid, gamma);
        for (index, item) in self.items.iter().enumerate() {
            let row = index / layout.columns.max(1);
            if row < self.scroll_row || row >= self.scroll_row + layout.visible_rows.max(1) {
                continue;
            }
            let column = index % layout.columns.max(1);
            let tile = IntRect {
                x: layout.grid.x + column as i32 * PREVIEW_SIZE,
                y: layout.grid.y + (row - self.scroll_row) as i32 * TILE_HEIGHT,
                w: PREVIEW_SIZE,
                h: TILE_HEIGHT.min(
                    layout.grid.y + layout.grid.h
                        - (layout.grid.y + (row - self.scroll_row) as i32 * TILE_HEIGHT),
                ),
            };
            if tile.h <= 0 {
                continue;
            }
            self.draw_item(surface, item, index, tile, resources, gamma);
        }

        resources.fonts.text.draw_with_gamma(
            surface,
            layout.import_label.x,
            layout.import_label.y + 3,
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
            resources.skin.draw_button(
                surface,
                rect,
                &format!("[{}] {label}", if selected { 'x' } else { ' ' }),
                resources.fonts,
                ClassicButtonState {
                    pressed: self.pressed
                        == if control == PortraitSelControl::SetPicture {
                            HitTarget::SetPicture
                        } else {
                            HitTarget::SetBigIcon
                        },
                    highlighted: self.focus == control,
                },
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
            resources.skin.draw_button(
                surface,
                rect,
                label,
                resources.fonts,
                ClassicButtonState {
                    pressed: self.pressed == target,
                    highlighted: self.focus == control,
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

        if self.combo_open {
            for (index, (location, rect)) in self
                .locations
                .iter()
                .zip(layout.location_options.iter())
                .enumerate()
            {
                resources.skin.draw_button(
                    surface,
                    *rect,
                    &location.label,
                    resources.fonts,
                    ClassicButtonState {
                        pressed: self.pressed == HitTarget::LocationOption(index),
                        highlighted: self.combo_highlight == index,
                    },
                    gamma,
                );
            }
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
        tile: IntRect,
        resources: PortraitSelResources<'_>,
        gamma: Option<&GammaRamp>,
    ) {
        let picture = IntRect {
            h: PREVIEW_SIZE.min(tile.h),
            ..tile
        };
        if self.selected == Some(index) {
            draw_engine_box(
                surface,
                tile.x,
                tile.y,
                tile.x + tile.w - 1,
                tile.y + tile.h - 1,
                if self.focus == PortraitSelControl::Grid {
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
                let marker = IntRect {
                    x: picture.x + (picture.w - size) / 2,
                    y: picture.y + (picture.h - size) / 2,
                    w: size,
                    h: size,
                };
                fill(surface, marker, Color::new(80, 20, 10, 220));
                resources.fonts.text.draw_with_gamma(
                    surface,
                    marker.x + marker.w / 2,
                    marker.y + (marker.h - resources.fonts.text.line_height) / 2,
                    "X",
                    [255, 255, 255, 255],
                    TextAlign::Center,
                    false,
                    gamma,
                );
            }
        }
        if tile.h > PREVIEW_SIZE {
            resources.fonts.mini.draw_with_gamma(
                surface,
                tile.x + tile.w / 2,
                tile.y + PREVIEW_SIZE,
                &item.label,
                [255, 255, 255, 255],
                TextAlign::Center,
                false,
                gamma,
            );
        }
        draw_3d_frame(surface, tile, gamma);
    }
}

#[derive(Clone, Copy)]
pub struct PortraitSelResources<'a> {
    pub skin: ClassicGuiSkin<'a>,
    pub fonts: &'a ClonkFontSet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortraitSelLayout {
    pub bounds: IntRect,
    pub caption: IntRect,
    pub close: IntRect,
    pub location_label: IntRect,
    pub location_combo: IntRect,
    pub location_options: Vec<IntRect>,
    pub grid: IntRect,
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
    let width = (screen_width * 2 / 3 + 10).clamp(MIN_WIDTH, MAX_WIDTH);
    let height = (screen_height * 2 / 3 + 10).clamp(MIN_HEIGHT, MAX_HEIGHT);
    let bounds = IntRect {
        x: (screen_width - width) / 2,
        y: (screen_height - height) / 2,
        w: width,
        h: height,
    };
    let caption = IntRect {
        x: bounds.x,
        y: bounds.y,
        w: bounds.w,
        h: CAPTION_HEIGHT,
    };
    let close = IntRect {
        x: bounds.x + bounds.w - CAPTION_HEIGHT,
        y: bounds.y,
        w: CAPTION_HEIGHT,
        h: CAPTION_HEIGHT,
    };
    let location_y = caption.y + caption.h + 8;
    let location_label = IntRect {
        x: bounds.x + 10,
        y: location_y,
        w: 75,
        h: CONTROL_HEIGHT,
    };
    let location_combo = IntRect {
        x: location_label.x + location_label.w,
        y: location_y,
        w: (bounds.x + bounds.w - 10) - (location_label.x + location_label.w),
        h: CONTROL_HEIGHT,
    };
    let options_y = bounds.y + bounds.h - 80;
    let buttons_y = bounds.y + bounds.h - BUTTON_HEIGHT - 8;
    let grid = IntRect {
        x: bounds.x + 10,
        y: location_y + CONTROL_HEIGHT + 8,
        w: bounds.w - 20,
        h: (options_y - 8) - (location_y + CONTROL_HEIGHT + 8),
    };
    let columns = (grid.w / PREVIEW_SIZE).max(1) as usize;
    let visible_rows = ((grid.h + TILE_HEIGHT - 1) / TILE_HEIGHT).max(1) as usize;
    let import_label = IntRect {
        x: bounds.x + 10,
        y: options_y,
        w: 120,
        h: CONTROL_HEIGHT,
    };
    let option_width = ((bounds.w - import_label.w - 30) / 2).max(1);
    let set_picture = IntRect {
        x: import_label.x + import_label.w,
        y: options_y,
        w: option_width,
        h: CONTROL_HEIGHT,
    };
    let set_big_icon = IntRect {
        x: set_picture.x + set_picture.w + 5,
        y: options_y,
        w: option_width,
        h: CONTROL_HEIGHT,
    };
    let cancel = IntRect {
        x: bounds.x + bounds.w - 10 - BUTTON_WIDTH,
        y: buttons_y,
        w: BUTTON_WIDTH,
        h: BUTTON_HEIGHT,
    };
    let ok = IntRect {
        x: cancel.x - 5 - BUTTON_WIDTH,
        y: buttons_y,
        w: BUTTON_WIDTH,
        h: BUTTON_HEIGHT,
    };
    let location_options = (0..location_count)
        .map(|index| IntRect {
            x: location_combo.x,
            y: location_combo.y + location_combo.h * (index as i32 + 1),
            w: location_combo.w,
            h: location_combo.h,
        })
        .collect();
    PortraitSelLayout {
        bounds,
        caption,
        close,
        location_label,
        location_combo,
        location_options,
        grid,
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

fn contains(rect: IntRect, point: GuiPoint) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < (rect.x + rect.w) as f32
        && point.y < (rect.y + rect.h) as f32
}

fn fill(surface: &mut Surface, rect: IntRect, color: Color) {
    if rect.w <= 0 || rect.h <= 0 {
        return;
    }
    surface.fill_rect(
        SurfaceRect::new(rect.x, rect.y, rect.w as u32, rect.h as u32),
        color,
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
    IntRect {
        x: rect.x + (rect.w - fitted_w) / 2,
        y: rect.y + (rect.h - fitted_h) / 2,
        w: fitted_w,
        h: fitted_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn portrait_sel_dialog_lists_images_matching_c4cfn_image_files_in_selected_location() {
        let temp = tempfile::tempdir().expect("portrait directory");
        for name in ["one.png", "two.BMP", "three.Jpeg", "four.JPG"] {
            fs::write(temp.path().join(name), b"not decoded during enumeration")
                .expect("write matching image name");
        }
        for name in ["five.gif", "notes.txt", "six.png.bak"] {
            fs::write(temp.path().join(name), b"ignored").expect("write rejected name");
        }
        fs::create_dir(temp.path().join("folder.png")).expect("matching directory is not a file");

        let entries = portrait_files_in_location(temp.path()).expect("scan portrait directory");
        let names = entries
            .iter()
            .map(|entry| entry.filename.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            names,
            BTreeSet::from(["four.JPG", "one.png", "three.Jpeg", "two.BMP"])
        );

        let mut controller = PortraitSelController::new(
            vec![PortraitLocation::new("User Path", temp.path())],
            0,
            entries,
            true,
            false,
        );
        assert_eq!(controller.items().len(), 5);
        assert_eq!(controller.items()[0].choice(), &PortraitChoice::None);
        assert!(controller.items()[1..]
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
            3,
            "only one file is decoded per loader quantum"
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
        controller.handle_key_down(KeyCode::Right);
        assert_eq!(
            controller.handle_key_down(KeyCode::Enter),
            vec![PortraitSelAction::Accept(PortraitSelCommit {
                choice: PortraitChoice::File(path),
                set_picture: false,
                set_big_icon: true,
            })]
        );

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
